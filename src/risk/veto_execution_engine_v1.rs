//! Private bounded observation engine. Never grants permission to trade.
use super::veto_chain::{VetoChain, VetoContext, VetoOutcome};
use super::veto_execution_report_v1::*;
use InputStatusV1 as I;
use RuleExecutionStatusV1 as R;
use VetoInputFieldV1 as F;

fn float(value: Option<f64>) -> I {
    match value {
        None => I::Missing,
        Some(v) if !v.is_finite() => I::NonFinite,
        Some(_) => I::Present,
    }
}
fn input(c: &VetoContext, field: F) -> I {
    let last = c.money_flow_days.as_ref().and_then(|d| d.last());
    match field {
        F::IsBuySignal | F::IsBearish => I::Present,
        F::SignalScore => {
            if (0..=100).contains(&c.signal_score) {
                I::Present
            } else {
                I::OutOfRange
            }
        }
        F::BiasMa5 => float(Some(c.bias_ma5)),
        F::MoneyFlowLastRecord => match &c.money_flow_days {
            None => I::Missing,
            Some(d) if d.is_empty() => I::EmptyCollection,
            Some(_) => I::Present,
        },
        F::MoneyFlowLastMainNet => float(last.map(|d| d.main_net)),
        F::MoneyFlowLastPctChg => float(last.and_then(|d| d.pct_chg)),
        F::PeRatio => {
            if c.pe_ratio == Some(0.0) {
                I::ZeroSentinel
            } else {
                float(c.pe_ratio)
            }
        }
        F::NetProfitYoy => float(c.net_profit_yoy),
    }
}
fn state(states: impl Iterator<Item = I>) -> R {
    let mut missing = false;
    for s in states {
        match s {
            I::NonFinite | I::OutOfRange => return R::InputInvalid,
            I::Missing | I::EmptyCollection | I::ZeroSentinel => missing = true,
            _ => {}
        }
    }
    if missing {
        R::InputMissing
    } else {
        R::EvaluatedClear
    }
}
/// `triggered` comes from the same predicate helpers that produce the old verdict.
pub(super) fn observe_builtin(
    c: &VetoContext,
    config: ConfiguredBuiltinRuleV1,
    triggered: &[bool],
) -> RuleInputObservationV1 {
    let inputs: Vec<_> = config
        .required_inputs
        .iter()
        .map(|d| RequiredInputObservationV1 {
            field: d.field,
            required: d.required,
            status: if d.required {
                input(c, d.field)
            } else {
                I::NotRequired
            },
        })
        .collect();
    let applicable = c.is_buy_signal && c.signal_score >= 60;
    let mut status = if !applicable {
        R::NotApplicable
    } else if !config.thresholds.finite() {
        R::InputInvalid
    } else {
        state(inputs.iter().filter(|i| i.required).map(|i| i.status))
    };
    let descriptor = builtin_rule_catalog_v1()
        .iter()
        .find(|d| d.id == config.id)
        .expect("builtin descriptor");
    let subconditions = config
        .subconditions
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut status = if !s.enabled {
                R::RuleDisabled
            } else if !applicable {
                R::NotApplicable
            } else if !config.thresholds.finite() {
                R::InputInvalid
            } else {
                state(
                    descriptor.subconditions[i]
                        .required_inputs
                        .iter()
                        .map(|f| input(c, *f))
                        .chain([input(c, F::SignalScore)]),
                )
            };
            if status == R::EvaluatedClear && triggered.get(i) == Some(&true) {
                status = R::EvaluatedVeto;
            }
            SubconditionObservationV1 {
                id: s.id,
                enabled: s.enabled,
                status,
            }
        })
        .collect::<Vec<_>>();
    if status == R::EvaluatedClear && subconditions.iter().any(|s| s.status == R::EvaluatedVeto) {
        status = R::EvaluatedVeto;
    }
    RuleInputObservationV1 {
        threshold_bits: config.thresholds.bits(),
        builtin: Some(config),
        status,
        inputs,
        subconditions,
    }
}
fn panic_observation(payload: &(dyn std::any::Any + Send)) -> PanicObservationV1 {
    let text = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("unknown panic payload");
    let mut end = text.len().min(512);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    PanicObservationV1 {
        payload: text[..end].to_owned(),
        truncated: end < text.len(),
    }
}
fn disable(contract: &mut RuleInputObservationV1, status: R) {
    contract.status = status;
    for s in &mut contract.subconditions {
        s.status = status;
    }
}
fn add_failure(report: &mut VetoExecutionReportV1, f: VetoReportFailureV1) {
    if !report.failures.contains(&f) {
        report.failures.push(f);
    }
}

pub(super) fn evaluate(
    chain: &VetoChain,
    ctx: &VetoContext,
    config: Option<&VetoConfigurationSnapshotV1>,
) -> VetoExecutionReportV1 {
    let mut report = VetoExecutionReportV1 {
        schema_version: VETO_REPORT_SCHEMA_VERSION_V1,
        semantics: VetoObservationSemanticsV1::ObservedOnly,
        configuration: config.cloned(),
        context: None,
        rules: Vec::new(),
        aggregate: VetoOutcome::default(),
        failures: Vec::new(),
        blocked_by_report_failure: false,
    };
    match ObservedVetoContextV1::capture(ctx) {
        Ok(c) => report.context = Some(c),
        Err(e) => add_failure(&mut report, e),
    }
    if chain.rules.len() > 64 {
        add_failure(
            &mut report,
            VetoReportFailureV1::ResourceLimit { resource: "rules" },
        );
    }
    let mut flags_bytes = 0usize;
    for (index, rule) in chain.rules.iter().take(64).enumerate() {
        let name = rule.name();
        if name.len() > 128 {
            add_failure(
                &mut report,
                VetoReportFailureV1::ResourceLimit {
                    resource: "rule_name",
                },
            );
        }
        let disabled = config.and_then(|c| {
            if !c.enabled() {
                Some(R::GloballyDisabled)
            } else if !c.rules()[index].enabled {
                Some(R::RuleDisabled)
            } else {
                None
            }
        });
        let evaluated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut contract = rule.observe_inputs(ctx);
            if let Some(c) = config {
                let frozen = &c.rules()[index];
                for field in &mut contract.inputs {
                    field.required = frozen
                        .required_inputs
                        .iter()
                        .any(|i| i.field == field.field && i.required);
                    if !field.required {
                        field.status = I::NotRequired;
                    }
                }
                for sub in &mut contract.subconditions {
                    sub.enabled = frozen
                        .subconditions
                        .iter()
                        .any(|s| s.id == sub.id && s.enabled);
                }
                contract.builtin = Some(frozen.clone());
            }
            if let Some(status) = disabled {
                disable(&mut contract, status);
                return (contract, None);
            }
            (
                contract,
                Some(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || rule.evaluate(ctx),
                ))),
            )
        }));
        let mut row = RuleExecutionObservationV1 {
            rule_name: if name.len() <= 128 {
                name.to_owned()
            } else {
                "<name exceeds bound>".into()
            },
            version: None,
            priority: rule.priority(),
            status: R::LegacyInputContractUnknown,
            contract: RuleInputObservationV1::legacy_unknown(),
            original_verdict: None,
            panic: None,
            failure: None,
        };
        match evaluated {
            Err(payload) => {
                row.status = R::Panicked;
                row.contract.status = R::Panicked;
                row.panic = Some(panic_observation(payload.as_ref()));
                // Builtin metadata is pure; retain configured identity even if evaluate panics.
                if let Some(c) = config {
                    row.version = Some(c.rules()[index].version);
                    row.contract.builtin = Some(c.rules()[index].clone());
                }
            }
            Ok((contract, verdict)) => {
                row.status = contract.status;
                row.version = contract.builtin.as_ref().map(|b| b.version);
                if contract.inputs.len() > 16
                    || contract.subconditions.len() > 8
                    || contract.threshold_bits.len() > 3
                    || contract
                        .builtin
                        .as_ref()
                        .is_some_and(|b| b.required_inputs.len() > 16 || b.subconditions.len() > 8)
                {
                    add_failure(
                        &mut report,
                        VetoReportFailureV1::ResourceLimit {
                            resource: "input_contract",
                        },
                    );
                    row.status = R::ReportFailed;
                    row.failure = Some(VetoReportFailureV1::ResourceLimit {
                        resource: "input_contract",
                    });
                } else {
                    row.contract = contract;
                }
                if let Some(verdict) = verdict {
                    let v = match verdict {
                        Ok(v) => v,
                        Err(payload) => {
                            row.status = R::Panicked;
                            row.contract.status = R::Panicked;
                            for s in &mut row.contract.subconditions {
                                if s.enabled {
                                    s.status = R::Panicked;
                                }
                            }
                            row.panic = Some(panic_observation(payload.as_ref()));
                            report.rules.push(row);
                            continue;
                        }
                    };
                    let bytes = v
                        .risk_flags
                        .iter()
                        .try_fold(0usize, |n, s| n.checked_add(s.len()));
                    let bounded = v.risk_flags.len() <= 8
                        && v.risk_flags.iter().all(|s| s.len() <= 2048)
                        && bytes.is_some_and(|n| n <= 65536usize.saturating_sub(flags_bytes));
                    if bounded {
                        flags_bytes += bytes.unwrap_or(0);
                        super::veto_chain::aggregate_verdict(&mut report.aggregate, &v);
                        if matches!(row.status, R::EvaluatedClear | R::EvaluatedVeto)
                            && (!v.risk_flags.is_empty() || v.force_hold || v.score_penalty != 0)
                        {
                            row.status = R::EvaluatedVeto;
                        }
                        row.original_verdict = Some(v);
                    } else {
                        add_failure(
                            &mut report,
                            VetoReportFailureV1::ResourceLimit {
                                resource: "verdict",
                            },
                        );
                        row.status = R::ReportFailed;
                        row.failure = Some(VetoReportFailureV1::ResourceLimit {
                            resource: "verdict",
                        });
                    }
                }
            }
        }
        if name.len() > 128 {
            row.status = R::ReportFailed;
            row.failure = Some(VetoReportFailureV1::ResourceLimit {
                resource: "rule_name",
            });
        }
        report.rules.push(row);
    }
    if let Some(e) = report.aggregate.aggregation_failure.clone() {
        add_failure(&mut report, e);
    }
    if let Err(e) = report.encode_bounded() {
        add_failure(&mut report, e);
    }
    report
}
