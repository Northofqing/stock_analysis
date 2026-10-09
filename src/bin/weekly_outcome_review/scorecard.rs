//! Versioned read-only artifact contract. Family attribution fails closed.
use super::{registry::RegistryInput, report::Review};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const SCHEMA: &str = "weekly-signal-scorecard-v1";
#[derive(Debug, Clone, Serialize)]
pub struct Evidence {
    pub reader_id: String,
    pub input_snapshot_sha256: String,
    pub sample_scope: String,
    pub exclusions: String,
    pub meaning: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Grade {
    Reliable,
    Observational,
    Unavailable,
}
#[derive(Debug, Serialize)]
pub struct Metric {
    pub id: String,
    pub grade: Grade,
    pub value: Option<Value>,
    pub reason: Option<String>,
    pub evidence: Evidence,
}
impl Metric {
    fn new(
        id: impl Into<String>,
        value: Option<Value>,
        grade: Grade,
        reason: impl Into<String>,
        evidence: Evidence,
    ) -> Self {
        Self {
            id: id.into(),
            grade: if value.is_some() {
                grade
            } else {
                Grade::Unavailable
            },
            reason: value.is_none().then(|| reason.into()),
            value,
            evidence,
        }
    }
}
#[derive(Debug, Serialize, Default)]
pub struct Sections {
    pub price_observation: Vec<Metric>,
    pub simulated_fill: Vec<Metric>,
    pub net_return: Vec<Metric>,
}
#[derive(Debug, Serialize)]
pub struct FamilyCard {
    pub signal_id: String,
    pub sections: Sections,
}
#[derive(Debug, Serialize)]
pub struct Scorecard {
    pub schema_version: &'static str,
    pub registry: RegistryInput,
    pub pooled_descriptive_evidence: Sections,
    pub families: Vec<FamilyCard>,
}
#[derive(Debug, Serialize)]
pub struct EvidenceManifest {
    pub schema_version: &'static str,
    pub artifact_schema: &'static str,
    pub input_snapshot_sha256: String,
    pub period: Value,
    pub registry: Value,
    pub paper_binding: Option<String>,
    pub reader_source_sha256: String,
    pub reader_source_inputs: Vec<&'static str>,
    /// JSON-pointer scopes cover all descendant metrics in the retained legacy
    /// report; scorecard metrics also carry their own evidence inline.
    pub metric_scopes: BTreeMap<String, Evidence>,
    pub boundary: &'static str,
}
fn evidence(reader: &str, sha: &str, scope: &str, exclusions: &str, meaning: &str) -> Evidence {
    Evidence {
        reader_id: format!("weekly_outcome_review::{reader}@H16-scorecard-v1"),
        input_snapshot_sha256: sha.into(),
        sample_scope: scope.into(),
        exclusions: exclusions.into(),
        meaning: meaning.into(),
    }
}
pub fn attach(review: &mut Review, registry: RegistryInput, sha: &str) {
    let price = evidence("report::predictions/classify", sha,
        "pooled original prediction rows; T+1/3/5 separately; calendar maturity in completed_sessions; all expected independent statuses and endpoint closes revalidated as of observed_at",
        "future origins/evidence, immature windows, ambiguous/missing status or endpoint, suspension, invalid/incomplete/contradictory stored pair, ready-unrecorded; no family/version join or historical PIT authority",
        "descriptive close-to-close observation; hit threshold uses original direction (>0.5% up, <-0.5% down, absolute <=0.5% neutral); no execution, price-availability or net-return authority");
    let fill = evidence("report::verified_paper/query_effective_fills_through_from_database_typed", sha,
        "full existing effective ledger through period_completed_through; weekly fills/closed cycles restricted to completed_sessions; open cycles retained at period boundary; projection/rule identity in verified_paper",
        "effective-reader/chain/inventory/T+1 failures and any effective fact after observed_at fail the section; legacy-no-terminal rows retain declared lineage; no family/version join",
        "verified simulated ledger facts only; Filled and ledger integrity do not qualify market prices or observed brokerage execution");
    let net = evidence("report::verified_paper/report_from_effective", sha,
        "complete dependent FIFO lifecycles closed in completed_sessions; model costs from the same effective capability; open cycles are right censored",
        "no closed cycles, integrity failures, missing cost coverage and unresolved original-price disputes keep dependent/aggregate amounts null; no observed fees, executable return or family attribution",
        "lot-rates-v1 scenario cost/net PnL in CNY, not observed settlement costs or a net-return percentage");
    let mut pooled = Sections::default();
    if let Some(predictions) = &review.predictions.value {
        for horizon in &predictions.horizons {
            let counts = &horizon.maturing_this_week;
            let n = horizon.trading_days;
            pooled.price_observation.push(Metric::new(
                format!("t{n}_revalidated_samples"),
                Some(json!(counts.revalidated_observations)),
                Grade::Observational,
                "",
                price.clone(),
            ));
            for (name, value, meaning) in [
                ("hit_rate", counts.observation_hit_rate, "fraction of revalidated close observations matching original direction threshold"),
                ("mean_change_pct", counts.observation_mean_change_pct, "mean revalidated close-to-close percentage change"),
            ] {
                let mut e = price.clone(); e.meaning = meaning.into();
                pooled.price_observation.push(Metric::new(format!("t{n}_{name}"), value.map(|v| json!(v)), Grade::Observational,
                    format!("no revalidated T+{n} observations maturing in completed requested sessions; inspect predictions.windows exclusions"), e));
            }
        }
    } else {
        pooled.price_observation.push(Metric::new(
            "revalidated_samples",
            None,
            Grade::Unavailable,
            review
                .predictions
                .reason
                .as_deref()
                .unwrap_or("prediction reader unavailable"),
            price.clone(),
        ));
    }
    if let Some(paper) = &review.verified_paper.value {
        for (id, count) in [
            ("period_fill_rows", paper.period_fill_rows),
            ("closed_cycles_in_week", paper.closed_cycles_in_week),
            ("open_cycles_at_period_end", paper.open_cycles_at_period_end),
        ] {
            let grade = if paper.legacy_without_terminal_rows == 0 {
                Grade::Reliable
            } else {
                Grade::Observational
            };
            pooled.simulated_fill.push(Metric::new(
                id,
                Some(json!(count)),
                grade,
                "",
                fill.clone(),
            ));
        }
        for (id, value) in [
            (
                "closed_cycle_scenario_cost_cny",
                paper.closed_cycle_scenario_cost_cny,
            ),
            (
                "closed_cycle_scenario_net_pnl_cny",
                paper.closed_cycle_scenario_net_pnl_cny,
            ),
        ] {
            pooled.net_return.push(Metric::new(id, value.map(|v| json!(v)), Grade::Observational,
                paper.scenario_amount_unavailable_reason.as_deref().unwrap_or("no complete qualified closed-cycle scenario amount in completed requested sessions"), net.clone()));
        }
    } else {
        let reason = review
            .verified_paper
            .reason
            .as_deref()
            .unwrap_or("effective paper reader unavailable");
        pooled.simulated_fill.push(Metric::new(
            "period_fill_rows",
            None,
            Grade::Unavailable,
            reason,
            fill.clone(),
        ));
        pooled.net_return.push(Metric::new(
            "closed_cycle_scenario_net_pnl_cny",
            None,
            Grade::Unavailable,
            reason,
            net.clone(),
        ));
    }
    pooled.net_return.push(Metric::new("executable_net_return", None, Grade::Unavailable, "no qualified executable fills, observed settlement fees or historical price/PIT evidence supplied", net.clone()));
    let families = registry.content.signals.iter().map(|signal| {
        let unavailable = |section: &str| Metric::new(section, None, Grade::Unavailable,
            format!("{:?}/{}: missing authoritative original family+signal-version join to {section}; exit/cost-version lineage also required for paper net; free-text reasons/candidate/archive rows are not attribution or physical-delivery evidence", signal.name, signal.signal_version),
            evidence("scorecard::family_join_unavailable", sha, &format!("registered signal {}; windows {:?}; no attributable qualified sample", signal.id, signal.windows), "all pooled prediction/paper rows excluded from family attribution", "null means attribution unavailable, never zero samples or zero return"));
        FamilyCard { signal_id: signal.id.clone(), sections: Sections {
            price_observation: vec![unavailable("price_observation")], simulated_fill: vec![unavailable("simulated_fill")], net_return: vec![unavailable("net_return")],
        }}
    }).collect();
    review.scorecard = Some(Scorecard {
        schema_version: SCHEMA,
        registry,
        pooled_descriptive_evidence: pooled,
        families,
    });
    let whole = "whole normalized snapshot diagnostics, including future rows; not as-of qualification; COUNT/DISTINCT/MIN/MAX fixed table/date fields";
    let mut scopes = BTreeMap::new();
    for section in ["daily_bars", "independent_daily_status"] {
        scopes.insert(
            format!("/{section}"),
            evidence(
                "report::extent",
                sha,
                whole,
                "none; raw extent only",
                "raw row/code counts and date extrema do not certify qualification",
            ),
        );
    }
    scopes.insert("/predictions".into(), evidence("report::predictions/classify", sha,
        "original_rows is whole input count; horizons distinguish weekly origins, weekly maturity and history through period; windows retain original row identity; MAX_PREDICTIONS=4096, no partial denominator", &price.exclusions, &price.meaning));
    scopes.insert("/raw_paper".into(), evidence("report::raw_paper", sha,
        "historical_filled_rows/latest_filled_utc are whole snapshot diagnostics; weekly states/exits restricted to completed_sessions and observed_at; MAX_ROWS=100000", "malformed/future times excluded from weekly metrics and separately diagnosed", "raw persisted state/reasons only; not valid prices, economic qualification, fill-rate or physical delivery"));
    scopes.insert("/original_order_attempts".into(), evidence("report::attempts", sha,
        "original UTC audit timestamps converted to Shanghai; weekly attempts in completed_sessions as of observed_at; MAX_ROWS=100000", "malformed/future times excluded from weekly attempts and separately diagnosed", "raw attempts by typed source/side/outcome and recorded reasons; not verified fill or delivery denominators"));
    scopes.insert("/verified_paper".into(), fill);
    // More specific scopes override their parent for monetary fields, including
    // per-exit nullable amounts. These explicit wildcard scopes are documented.
    for path in [
        "/verified_paper/value/period_scenario_fill_cost_cny",
        "/verified_paper/value/closed_cycle_scenario_cost_cny",
        "/verified_paper/value/closed_cycle_scenario_net_pnl_cny",
        "/verified_paper/value/exits/*/scenario_cost_cny",
        "/verified_paper/value/exits/*/scenario_net_pnl_cny",
        "/verified_paper/value/actual_settlement_costs",
        "/verified_paper/value/executable_net_return",
    ] {
        scopes.insert(path.into(), net.clone());
    }
    scopes.insert(
        "/physical_delivery".into(),
        evidence(
            "report::physical_delivery_unavailable",
            sha,
            "no independent durable/card-to-row receipt input",
            "all candidate/archive/paper rows excluded",
            "physical delivery unavailable; counted/Uncertain state is not changed or inferred",
        ),
    );
    scopes.insert("/scorecard".into(), evidence("scorecard::attach", sha, "registered families and separate pooled evidence; each metric carries more specific inline evidence", "manual status/action cannot promote evidence grades", "registry is descriptive metadata; null reasons and reliable/observational/unavailable grades are reader-derived"));
    let fingerprint = [
        include_str!("../weekly_outcome_review.rs"),
        include_str!("report.rs"),
        include_str!("registry.rs"),
        include_str!("scorecard.rs"),
        include_str!("../../performance/economic_position.rs"),
        include_str!("../../trading/paper_ledger.rs"),
        include_str!("../../trading/paper_effective_fills.rs"),
        include_str!("../../trading/paper_legacy_price_disputes.rs"),
        include_str!("../../../Cargo.lock"),
    ]
    .join("\n");
    review.evidence_manifest = Some(EvidenceManifest {
        schema_version: "weekly-outcome-evidence-manifest-v1", artifact_schema: SCHEMA, input_snapshot_sha256: sha.into(),
        period: serde_json::to_value(&review.period).expect("serializable period"),
        registry: serde_json::to_value(&review.scorecard.as_ref().unwrap().registry).expect("serializable registry"),
        paper_binding: std::env::var(stock_analysis::trading::paper_ledger_runtime::BINDING_ENV).ok(),
        reader_source_sha256: super::bytes_sha256(fingerprint.as_bytes()),
        reader_source_inputs: vec!["src/bin/weekly_outcome_review.rs", "src/bin/weekly_outcome_review/report.rs", "src/bin/weekly_outcome_review/registry.rs", "src/bin/weekly_outcome_review/scorecard.rs", "src/performance/economic_position.rs", "src/trading/paper_ledger.rs", "src/trading/paper_effective_fills.rs", "src/trading/paper_legacy_price_disputes.rs", "Cargo.lock"], metric_scopes: scopes,
        boundary: "detached read-only normalized SQLite bytes; SHA is snapshot identity, not qualification. Longest JSON-pointer scope (including * array index) covers descendant legacy metrics; inline metric evidence overrides scopes. Period and original immutable facts are preserved. Registry actions/status are manual only.",
    });
}
impl Scorecard {
    pub fn markdown(&self) -> String {
        let mut text = format!("\n## 信号评分卡 / {}\n\nRegistry `{}` / SHA-256 `{}`；action/status 仅手动描述，不改变策略。家族指标与汇总证据分别列出。\n", self.schema_version, self.registry.content.registry_version, self.registry.sha256);
        for family in &self.families {
            let entry = self
                .registry
                .content
                .signals
                .iter()
                .find(|s| s.id == family.signal_id)
                .unwrap();
            text.push_str(&format!("\n### {:?} / {}\n\n版本：signal `{}` / exit `{}` / cost `{}`；action {:?} / status {:?}；T+{:?}。\n\nEntry：{}\n\nEligibility：{}\n", entry.name, entry.id, entry.signal_version, entry.exit_version, entry.cost_version, entry.action, entry.status, entry.windows, super::report::escape(&entry.entry_assumptions), super::report::escape(&entry.eligibility)));
            text.push_str(&render_sections(&family.sections));
        }
        text.push_str("\n### 汇总描述性证据（无家族归属）\n");
        text.push_str(&render_sections(&self.pooled_descriptive_evidence));
        text
    }
}
fn render_sections(sections: &Sections) -> String {
    let mut text = String::new();
    for (label, metrics) in [
        ("价格观察 / price_observation", &sections.price_observation),
        ("模拟成交 / simulated_fill", &sections.simulated_fill),
        ("净收益与情景金额 / net_return", &sections.net_return),
    ] {
        text.push_str(&format!("\n#### {label}\n"));
        for metric in metrics {
            text.push_str(&format!("\n- `{}`：{:?} / {}{}\n  reader `{}`；snapshot SHA `{}`；scope {}；exclusions {}；meaning {}。\n", metric.id, metric.grade, metric.value.as_ref().map(ToString::to_string).unwrap_or("null".into()), metric.reason.as_ref().map(|r| format!("（{}）", super::report::escape(r))).unwrap_or_default(), metric.evidence.reader_id, metric.evidence.input_snapshot_sha256, super::report::escape(&metric.evidence.sample_scope), super::report::escape(&metric.evidence.exclusions), super::report::escape(&metric.evidence.meaning)));
        }
    }
    text
}
