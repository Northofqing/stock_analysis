use super::*;
use crate::capital_flow::MoneyFlowDay;
use crate::risk::veto_chain::{VetoChain, VetoRule};
use crate::risk::veto_rules_live::{
    build_chain, BiasRateRule, FundamentalDeteriorationRule, MainFlowRule,
};
use InputStatusV1 as I;
use RuleExecutionStatusV1 as R;

fn context() -> VetoContext {
    VetoContext {
        code: "TEST_CODE_RISK".into(),
        current_price: 10.0,
        signal_score: 65,
        is_buy_signal: true,
        bias_ma5: 2.0,
        is_bearish: false,
        money_flow_days: Some(vec![MoneyFlowDay {
            date: "2026-10-03".into(),
            main_net: 1.0,
            xl_net: 0.0,
            big_net: 0.0,
            main_pct: 0.0,
            pct_chg: Some(1.0),
        }]),
        pct_chg: Some(1.0),
        pe_ratio: Some(15.0),
        net_profit_yoy: Some(10.0),
    }
}
fn evaluate(c: &VetoContext, config: VetoChainConfig) -> VetoExecutionReportV1 {
    ConfiguredVetoChainV1::new(VetoConfigurationSnapshotV1::from_config(&config)).evaluate(c)
}
fn default_report(c: &VetoContext) -> VetoExecutionReportV1 {
    evaluate(c, VetoChainConfig::default())
}
fn status(report: &VetoExecutionReportV1, rule: usize, field: VetoInputFieldV1) -> I {
    report.rules[rule]
        .contract
        .inputs
        .iter()
        .find(|i| i.field == field)
        .unwrap()
        .status
}

#[test]
fn catalog_config_and_actual_rules_share_frozen_defaults() {
    let raw = crate::config::LiveVetoConfig {
        mode: "Live".into(),
        bias_rate_enabled: false,
        ..Default::default()
    };
    let s = VetoConfigurationSnapshotV1::from_live_config(&raw).unwrap();
    assert_eq!(s.raw_mode(), "Live");
    assert_eq!(s.effective_mode(), VetoMode::DryRun);
    assert_eq!(s.rules().len(), 3);
    assert!(!s.rules()[0].subconditions[0].enabled);
    assert!(s.rules()[0].subconditions[1].enabled);
    assert!(
        !s.rules()[0]
            .required_inputs
            .iter()
            .find(|i| i.field == F::BiasMa5)
            .unwrap()
            .required
    );
    for (rule, descriptor) in s.rules().iter().zip(builtin_rule_catalog_v1()) {
        assert_eq!(rule.id, descriptor.id);
        assert_eq!(rule.priority, descriptor.priority);
        assert_eq!(rule.version, descriptor.version);
    }
    let json = serde_json::to_value(&s).unwrap();
    assert_eq!(json["model_version"], VETO_MODEL_VERSION_V1);
    assert_eq!(
        json["rules"][0]["thresholds"]["BiasRate"]["bias_threshold"],
        BiasRateRule::default().bias_threshold
    );
}
#[test]
fn full_clear_and_each_numerical_veto_preserve_legacy_aggregate() {
    let clear = default_report(&context());
    assert_eq!(clear.rules.len(), 3);
    assert!(clear.rules.iter().all(|r| r.status == R::EvaluatedClear));
    assert!(!clear.has_report_failure());
    for (rule, descriptor) in clear.rules.iter().zip(builtin_rule_catalog_v1()) {
        assert_eq!(rule.priority, descriptor.priority);
        assert_eq!(rule.version, Some(descriptor.version));
    }
    for index in 0..3 {
        let mut ctx = context();
        match index {
            0 => ctx.bias_ma5 = 6.0,
            1 => ctx.money_flow_days.as_mut().unwrap()[0].main_net = -60_000_000.0,
            _ => {
                ctx.pe_ratio = Some(500.0);
                ctx.net_profit_yoy = Some(-35.0);
            }
        }
        let report = default_report(&ctx);
        let old = build_chain(&VetoChainConfig::default())
            .unwrap()
            .evaluate_all(&ctx);
        assert_eq!(report.rules[index].status, R::EvaluatedVeto);
        assert_eq!(report.aggregate.flags, old.flags);
        assert_eq!(report.aggregate.force_hold, old.force_hold);
        assert_eq!(report.aggregate.total_penalty, old.total_penalty);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&report.encode_bounded().unwrap()).unwrap()
                ["semantics"],
            "observed_only"
        );
    }
}
#[test]
fn missing_flow_is_never_clear_and_context_quote_does_not_fill_last_record() {
    let mut c = context();
    c.money_flow_days = None;
    let r = default_report(&c);
    assert_eq!(r.rules[1].status, R::InputMissing);
    assert_eq!(status(&r, 1, F::MoneyFlowLastRecord), I::Missing);
    assert!(!r.has_report_failure());
    c.money_flow_days = Some(vec![]);
    let r = default_report(&c);
    assert_eq!(status(&r, 1, F::MoneyFlowLastRecord), I::EmptyCollection);
    c = context();
    c.money_flow_days.as_mut().unwrap()[0].pct_chg = None;
    c.pct_chg = Some(9.0);
    c.money_flow_days.as_mut().unwrap()[0].main_net = -60_000_000.0;
    let r = default_report(&c);
    assert_eq!(r.rules[1].status, R::InputMissing);
    assert_eq!(status(&r, 1, F::MoneyFlowLastPctChg), I::Missing);
    assert_eq!(
        r.rules[1].contract.subconditions[0].status,
        R::EvaluatedVeto
    );
    assert_eq!(r.rules[1].contract.subconditions[1].status, R::InputMissing);
    assert!(r.aggregate.force_hold);
    assert_eq!(r.aggregate.flags.len(), 1);
}
#[test]
fn fundamental_zero_absence_and_each_missing_field_remain_distinct() {
    for pe in [None, Some(0.0)] {
        let mut c = context();
        c.pe_ratio = pe;
        c.net_profit_yoy = None;
        let r = default_report(&c);
        assert_eq!(r.rules[2].status, R::InputMissing);
        assert_eq!(
            status(&r, 2, F::PeRatio),
            if pe.is_none() {
                I::Missing
            } else {
                I::ZeroSentinel
            }
        );
        assert_eq!(status(&r, 2, F::NetProfitYoy), I::Missing);
        assert!(!r.aggregate.force_hold);
    }
    let mut c = context();
    c.net_profit_yoy = None;
    assert_eq!(default_report(&c).rules[2].status, R::InputMissing);
}
#[test]
fn nonfinite_consumed_inputs_are_invalid_and_last_record_is_preserved() {
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut c = context();
        c.bias_ma5 = value;
        c.pe_ratio = Some(value);
        c.net_profit_yoy = Some(value);
        let day = &mut c.money_flow_days.as_mut().unwrap()[0];
        day.main_net = value;
        day.pct_chg = Some(value);
        let r = default_report(&c);
        assert!(r.rules.iter().all(|r| r.status == R::InputInvalid));
        assert_eq!(r.context.as_ref().unwrap().bias_ma5_bits, value.to_bits());
        assert!(r.encode_bounded().is_ok());
    }
    let mut c = context();
    let mut last = c.money_flow_days.as_ref().unwrap()[0].clone();
    last.date = "actual-last".into();
    last.main_net = -60_000_000.0;
    c.money_flow_days.as_mut().unwrap().push(last);
    let r = default_report(&c);
    assert_eq!(r.context.as_ref().unwrap().money_flow_count, Some(2));
    assert_eq!(
        r.context
            .as_ref()
            .unwrap()
            .last_money_flow
            .as_ref()
            .unwrap()
            .date,
        "actual-last"
    );
    assert_eq!(r.rules[1].status, R::EvaluatedVeto);
    c.current_price = f64::NAN;
    c.pct_chg = Some(f64::INFINITY);
    assert_ne!(default_report(&c).rules[1].status, R::InputInvalid);
}
#[test]
fn disabled_and_nonapplicable_rows_still_cover_the_full_catalog() {
    let c = context();
    let r = evaluate(
        &c,
        VetoChainConfig {
            enabled: false,
            ..Default::default()
        },
    );
    assert_eq!(r.rules.len(), 3);
    assert!(r
        .rules
        .iter()
        .all(|r| r.status == R::GloballyDisabled && r.original_verdict.is_none()));
    let r = evaluate(
        &c,
        VetoChainConfig {
            bias_rate_enabled: false,
            bearish_alignment_enabled: false,
            main_flow_enabled: false,
            fundamental_enabled: false,
            ..Default::default()
        },
    );
    assert!(r.rules.iter().all(|r| r.status == R::RuleDisabled));
    assert!(r
        .rules
        .iter()
        .all(|r| !r.contract.builtin.as_ref().unwrap().enabled));
    for index in 0..3 {
        let mut conf = VetoChainConfig::default();
        match index {
            0 => {
                conf.bias_rate_enabled = false;
                conf.bearish_alignment_enabled = false
            }
            1 => conf.main_flow_enabled = false,
            _ => conf.fundamental_enabled = false,
        };
        let r = evaluate(&c, conf);
        assert_eq!(r.rules[index].status, R::RuleDisabled);
    }
    for (buy, score) in [(false, 65), (true, 55)] {
        let mut c = context();
        c.is_buy_signal = buy;
        c.signal_score = score;
        c.money_flow_days = None;
        let r = default_report(&c);
        assert!(r.rules.iter().all(|r| r.status == R::NotApplicable));
    }
}
#[test]
fn all_four_technical_switch_combinations_execute_exact_subconditions() {
    for bias in [false, true] {
        for bearish in [false, true] {
            for bias_input in [false, true] {
                let conf = VetoChainConfig {
                    bias_rate_enabled: bias,
                    bearish_alignment_enabled: bearish,
                    main_flow_enabled: false,
                    fundamental_enabled: false,
                    ..Default::default()
                };
                let mut c = context();
                c.bias_ma5 = if bias_input { 6.0 } else { 2.0 };
                c.is_bearish = !bias_input;
                let expected = if bias_input { bias } else { bearish };
                let r = evaluate(&c, conf.clone());
                assert_eq!(
                    r.aggregate.force_hold, expected,
                    "bias={bias} bearish={bearish} bias_input={bias_input}"
                );
                let old = build_chain(&conf)
                    .map(|c| c.evaluate_all(&c_context(bias_input)))
                    .unwrap_or_default();
                assert_eq!(old.force_hold, expected);
                assert_eq!(r.rules[0].contract.subconditions[0].enabled, bias);
                assert_eq!(r.rules[0].contract.subconditions[1].enabled, bearish);
            }
        }
    }
    fn c_context(bias: bool) -> VetoContext {
        let mut c = context();
        c.bias_ma5 = if bias { 6.0 } else { 2.0 };
        c.is_bearish = !bias;
        c
    }
    let mut c = context();
    c.bias_ma5 = 6.0;
    c.is_bearish = true;
    assert_eq!(BiasRateRule::default().evaluate(&c).risk_flags.len(), 2);
}
struct Custom {
    penalty: i32,
    flags: bool,
    panic_kind: u8,
}
impl VetoRule for Custom {
    fn name(&self) -> &'static str {
        "TEST_CODE_Custom"
    }
    fn evaluate(&self, _: &VetoContext) -> VetoVerdict {
        match self.panic_kind {
            1 => panic!("borrowed panic"),
            2 => std::panic::panic_any("owned panic".to_string()),
            3 => std::panic::panic_any(7u8),
            _ => {}
        }
        VetoVerdict {
            risk_flags: if self.flags {
                vec!["TEST_CODE_veto".into()]
            } else {
                vec![]
            },
            score_penalty: self.penalty,
            force_hold: self.penalty != 0,
        }
    }
}
#[test]
fn legacy_unknown_and_zero_flag_effects_do_not_become_clear() {
    let chain = VetoChain::new(vec![
        Box::new(Custom {
            penalty: 0,
            flags: false,
            panic_kind: 0,
        }),
        Box::new(Custom {
            penalty: 7,
            flags: false,
            panic_kind: 0,
        }),
    ]);
    let r = chain.evaluate_observed(&context());
    assert!(r
        .rules
        .iter()
        .all(|r| r.status == R::LegacyInputContractUnknown));
    assert_eq!(
        r.rules[1].original_verdict.as_ref().unwrap().score_penalty,
        7
    );
    assert!(!r.aggregate.force_hold);
    assert_eq!(r.aggregate.total_penalty, 0);
}
#[test]
fn panic_payloads_continue_to_the_next_actual_rule() {
    let chain = VetoChain::new(vec![
        Box::new(Custom {
            penalty: 0,
            flags: false,
            panic_kind: 1,
        }),
        Box::new(Custom {
            penalty: 0,
            flags: false,
            panic_kind: 2,
        }),
        Box::new(Custom {
            penalty: 0,
            flags: false,
            panic_kind: 3,
        }),
        Box::new(Custom {
            penalty: 3,
            flags: true,
            panic_kind: 0,
        }),
    ]);
    let r = chain.evaluate_observed(&context());
    assert!(r.rules[..3].iter().all(|r| r.status == R::Panicked));
    assert_eq!(
        r.rules[2].panic.as_ref().unwrap().payload,
        "unknown panic payload"
    );
    assert_eq!(r.aggregate.total_penalty, 3);
    assert!(!r.has_report_failure());
    assert!(r.rules[3].original_verdict.is_some());
}
#[test]
fn checked_penalty_overflow_is_typed_without_abort_or_wrap() {
    for (a, b, overflow) in [
        (i32::MAX - 1, 1, false),
        (i32::MAX, 1, true),
        (i32::MIN, -1, true),
    ] {
        let chain = VetoChain::new(vec![
            Box::new(Custom {
                penalty: a,
                flags: true,
                panic_kind: 0,
            }),
            Box::new(Custom {
                penalty: b,
                flags: true,
                panic_kind: 0,
            }),
        ]);
        let r = chain.evaluate_observed(&context());
        assert_eq!(r.has_report_failure(), overflow);
        assert_eq!(r.rules.len(), 2);
        let old = chain.evaluate_all(&context());
        assert_eq!(old.aggregation_failure.is_some(), overflow);
        assert_eq!(r.aggregate.total_penalty, old.total_penalty);
        if overflow {
            assert!(r.failures.contains(&VetoReportFailureV1::PenaltyOverflow));
        }
    }
}
#[test]
fn resource_limits_and_bounded_encoder_reject_without_empty_success() {
    let mut c = context();
    c.code = "x".repeat(257);
    let r = default_report(&c);
    assert!(r.has_report_failure());
    assert!(r.context.is_none());
    assert_eq!(r.rules.len(), 3);
    let mut config = crate::config::LiveVetoConfig::default();
    config.mode = "x".repeat(65);
    assert!(VetoConfigurationSnapshotV1::from_live_config(&config).is_err());
    let mut r = default_report(&context());
    r.aggregate.flags = vec!["x".repeat(MAX_REPORT_BYTES)];
    assert_eq!(
        r.encode_bounded().unwrap_err(),
        VetoReportFailureV1::EncodingLimit
    );
    let rules: Vec<Box<dyn VetoRule>> = (0..65)
        .map(|_| {
            Box::new(Custom {
                penalty: 0,
                flags: false,
                panic_kind: 0,
            }) as Box<dyn VetoRule>
        })
        .collect();
    let r = VetoChain::new(rules).evaluate_observed(&context());
    assert!(r.has_report_failure());
    assert_eq!(r.rules.len(), 64);
}
#[test]
fn direct_builtin_missing_behavior_remains_legacy_compatible() {
    let mut c = context();
    c.money_flow_days = None;
    c.pe_ratio = Some(0.0);
    assert!(MainFlowRule::default().evaluate(&c).is_empty());
    assert!(FundamentalDeteriorationRule::default()
        .evaluate(&c)
        .is_empty());
}

#[test]
fn nonfinite_thresholds_keep_exact_observation_without_changing_direct_rule_behavior() {
    let rule = BiasRateRule {
        bias_threshold: f64::NAN,
    };
    let chain = VetoChain::new(vec![Box::new(rule)]);
    let r = chain.evaluate_observed(&context());
    assert_eq!(r.rules[0].status, R::InputInvalid);
    assert_eq!(r.rules[0].contract.threshold_bits, [f64::NAN.to_bits()]);
    assert!(r.encode_bounded().is_ok());
}
struct Verbose {
    huge_flag: bool,
}
impl VetoRule for Verbose {
    fn name(&self) -> &'static str {
        "TEST_CODE_VERBOSE"
    }
    fn evaluate(&self, _: &VetoContext) -> VetoVerdict {
        if self.huge_flag {
            VetoVerdict {
                risk_flags: vec!["x".repeat(2049)],
                score_penalty: 0,
                force_hold: true,
            }
        } else {
            std::panic::panic_any("界".repeat(400))
        }
    }
}
#[test]
fn oversized_verdict_and_utf8_panic_diagnostics_are_bounded() {
    let chain = VetoChain::new(vec![
        Box::new(Verbose { huge_flag: true }),
        Box::new(Verbose { huge_flag: false }),
        Box::new(Custom {
            penalty: 3,
            flags: true,
            panic_kind: 0,
        }),
    ]);
    let r = chain.evaluate_observed(&context());
    assert!(r.has_report_failure());
    assert!(r.rules[0].original_verdict.is_none());
    assert_eq!(r.rules[0].status, R::ReportFailed);
    assert_eq!(
        r.rules[0].failure,
        Some(VetoReportFailureV1::ResourceLimit {
            resource: "verdict"
        })
    );
    let p = r.rules[1].panic.as_ref().unwrap();
    assert!(p.truncated);
    assert!(p.payload.len() <= 512);
    assert!(p.payload.chars().all(|c| c == '界'));
    assert_eq!(r.aggregate.total_penalty, 3);
    assert!(r.encode_bounded().is_ok());
}

#[test]
fn each_consumed_numeric_field_independently_prevents_a_clear_status() {
    for field in [
        F::BiasMa5,
        F::MoneyFlowLastMainNet,
        F::MoneyFlowLastPctChg,
        F::PeRatio,
        F::NetProfitYoy,
        F::SignalScore,
    ] {
        let mut c = context();
        let index = match field {
            F::BiasMa5 => {
                c.bias_ma5 = f64::NAN;
                0
            }
            F::MoneyFlowLastMainNet => {
                c.money_flow_days.as_mut().unwrap()[0].main_net = f64::NAN;
                1
            }
            F::MoneyFlowLastPctChg => {
                c.money_flow_days.as_mut().unwrap()[0].pct_chg = Some(f64::NAN);
                1
            }
            F::PeRatio => {
                c.pe_ratio = Some(f64::NAN);
                2
            }
            F::NetProfitYoy => {
                c.net_profit_yoy = Some(f64::NAN);
                2
            }
            F::SignalScore => {
                c.signal_score = 101;
                0
            }
            _ => unreachable!(),
        };
        let report = default_report(&c);
        assert_eq!(report.rules[index].status, R::InputInvalid, "{field:?}");
    }
}
