//! Read-only fingerprint of the scheduled chain report input, not Foundation shadow parity.

use anyhow::Result;
use chrono::{DateTime, FixedOffset, NaiveDate};
use sha2::{Digest, Sha256};
use stock_analysis::monitor::push_job::WeakOutcomeKind;
use stock_analysis::notification::{NotificationChannel, NotificationSendReport};
use stock_analysis::pipeline::chain_analysis::preparation::PreparedChainAnalysis;

use super::chain_acquisition::{
    ChainAcquisitionEvidence, ChainNewsEvidence, ChainSelectedNewsSourceRefV1,
    MissingSelectedNewsSourceTime,
};
use super::chain_schedule::{ChainPhase, ChainScheduleStatus};
use super::modes::ChainSendSuppression;

pub(super) const COVERAGE: &str = "incomplete";
pub(super) const REPORT_INPUTS: &str = "prepared_report_utf8_only";
pub(super) const ACQUISITION_INPUTS: &str =
    "limit_up_metadata_selected_news_titles_utf8_and_report_utf8";

/// Values sampled by the legacy scheduler before any preparation or send. A
/// later observer receives this copy instead of reopening the schedule store.
#[derive(Clone, Debug)]
pub(super) struct ChainGateCapture {
    phase: ChainPhase,
    schedule_date: NaiveDate,
    observed_at: DateTime<FixedOffset>,
    trading_day: bool,
    legacy_status: ChainScheduleStatus,
}

impl ChainGateCapture {
    pub(super) fn new(
        phase: ChainPhase,
        schedule_date: NaiveDate,
        observed_at: DateTime<FixedOffset>,
        trading_day: bool,
        legacy_status: ChainScheduleStatus,
    ) -> Self {
        Self {
            phase,
            schedule_date,
            observed_at,
            trading_day,
            legacy_status,
        }
    }

    fn status_name(&self) -> &'static str {
        match self.legacy_status {
            ChainScheduleStatus::Ready => "ready",
            ChainScheduleStatus::Uncertain => "uncertain",
            ChainScheduleStatus::Closed => "closed",
        }
    }

    pub(super) fn schedule_only_reason(&self) -> Option<&'static str> {
        match self.legacy_status {
            ChainScheduleStatus::Closed => Some("already_closed"),
            ChainScheduleStatus::Uncertain => Some("uncertain_needs_review"),
            ChainScheduleStatus::Ready if !self.trading_day => Some("nontrading_day"),
            ChainScheduleStatus::Ready
                if !self
                    .phase
                    .starts_in_window(self.schedule_date, self.observed_at.naive_local()) =>
            {
                Some("outside_send_window")
            }
            ChainScheduleStatus::Ready => None,
        }
    }

    pub(super) fn sha256(&self) -> Result<String> {
        digest_decision(&serde_json::json!({
            "schema": "chain-gate-capture-v1",
            "phase": self.phase.as_str(),
            "schedule_date": self.schedule_date,
            "observed_at": self.observed_at.to_rfc3339(),
            "trading_day": self.trading_day,
            "legacy_status": self.status_name(),
        }))
    }
}

/// A target-level weak result, in the original send order. Its optional
/// request hashes describe only the first built HTTP entity where available.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ChainWeakTargetResult {
    channel: String,
    target_index: usize,
    outcome: WeakOutcomeKind,
    built_target_sha256: Option<String>,
    built_body_sha256: Option<String>,
}

impl ChainWeakTargetResult {
    pub(super) fn from_report(report: &NotificationSendReport) -> Vec<Self> {
        report
            .attempts()
            .iter()
            .map(|attempt| Self {
                channel: attempt.channel().name().to_owned(),
                target_index: attempt.target_index(),
                outcome: attempt.outcome(),
                built_target_sha256: attempt
                    .request_entity()
                    .map(|entity| entity.target_sha256().to_owned()),
                built_body_sha256: attempt
                    .request_entity()
                    .map(|entity| entity.body_sha256().to_owned()),
            })
            .collect()
    }

    fn outcome_name(&self) -> &'static str {
        match self.outcome {
            WeakOutcomeKind::Accepted => "weak_accepted",
            WeakOutcomeKind::Rejected => "weak_rejected",
            WeakOutcomeKind::Unknown => "unknown",
        }
    }
}

pub(super) struct ChainPreparedDecision<'a> {
    pub input: &'a ChainReportInputObservation,
    pub suppression: Option<ChainSendSuppression>,
    pub send_attempted: bool,
    pub report_observed: bool,
    pub send_id: Option<&'a str>,
    pub targets: &'a [ChainWeakTargetResult],
    pub mark_attempted: bool,
    pub legacy_succeeded: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ChainDecisionObservation {
    pub gate_sha256: String,
    pub binding_sha256: String,
    pub send_id: Option<String>,
    pub scope: &'static str,
    pub reason: &'static str,
    /// Exact retained report-input comparison, not channel payload parity.
    pub prepared_report_equals_input: Option<bool>,
    pub target_count: usize,
}

fn digest_decision(value: &serde_json::Value) -> Result<String> {
    let bytes = serde_json::to_vec(value)?;
    let mut hash = Sha256::new();
    hash.update(b"stock_analysis.chain_decision_observation.v1\0");
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
    Ok(format!("{:x}", hash.finalize()))
}

/// Suppressed runs have no report facts. This digest is explicitly limited to
/// the captured schedule gate; it cannot stand in for a prepared decision.
pub(super) fn observe_schedule_only(gate: &ChainGateCapture) -> Result<ChainDecisionObservation> {
    let reason = gate
        .schedule_only_reason()
        .ok_or_else(|| anyhow::anyhow!("ready chain gate requires prepared facts"))?;
    let gate_sha256 = gate.sha256()?;
    let binding_sha256 = digest_decision(&serde_json::json!({
        "schema": "chain-decision-observation-v1",
        "scope": "schedule_only",
        "gate_sha256": gate_sha256,
        "reason": reason,
    }))?;
    Ok(ChainDecisionObservation {
        gate_sha256,
        binding_sha256,
        send_id: None,
        scope: "schedule_only",
        reason,
        prepared_report_equals_input: None,
        target_count: 0,
    })
}

/// Bind only values already retained by this invocation. No provider, store,
/// clock, reporter or sink is available to this projection.
pub(super) fn observe_prepared_decision(
    gate: &ChainGateCapture,
    decision: ChainPreparedDecision<'_>,
) -> Result<ChainDecisionObservation> {
    anyhow::ensure!(
        gate.schedule_only_reason().is_none(),
        "suppressed chain gate has no prepared decision"
    );
    anyhow::ensure!(
        decision.input.phase == gate.phase && decision.input.schedule_date == gate.schedule_date,
        "chain decision gate and prepared input disagree"
    );
    anyhow::ensure!(
        decision.suppression.is_some() != decision.send_attempted,
        "chain suppression and send attempt disagree"
    );
    anyhow::ensure!(
        !decision.mark_attempted || decision.send_attempted,
        "chain weak acceptance mark exists without a send attempt"
    );
    anyhow::ensure!(
        !matches!(
            decision.suppression,
            Some(
                ChainSendSuppression::NoConfiguredChannel
                    | ChainSendSuppression::BeforeSendRejected
            )
        ) || !decision.legacy_succeeded,
        "chain failed send guard cannot have a successful legacy result"
    );
    anyhow::ensure!(
        decision.report_observed || decision.targets.is_empty(),
        "chain targets lack their original send report"
    );
    anyhow::ensure!(
        decision.send_attempted || !decision.report_observed,
        "chain send report exists without a send attempt"
    );
    anyhow::ensure!(
        decision.report_observed == decision.send_id.is_some()
            && decision.send_id.is_none_or(|id| !id.is_empty()),
        "chain send id does not match the observed send report"
    );
    anyhow::ensure!(
        decision
            .targets
            .iter()
            .enumerate()
            .all(|(ordinal, target)| target.target_index == ordinal),
        "chain target results are not in original send order"
    );
    let targets = decision
        .targets
        .iter()
        .map(|target| {
            serde_json::json!({
                "channel": target.channel,
                "target_index": target.target_index,
                "outcome": target.outcome_name(),
                "built_target_sha256": target.built_target_sha256,
                "built_body_sha256": target.built_body_sha256,
            })
        })
        .collect::<Vec<_>>();
    let gate_sha256 = gate.sha256()?;
    let suppression_reason = decision.suppression.map(ChainSendSuppression::as_str);
    let binding_sha256 = digest_decision(&serde_json::json!({
        "schema": "chain-decision-observation-v2",
        "scope": "prepared",
        "gate_sha256": gate_sha256,
        "prepared_business_date": decision.input.prepared_business_date,
        "artifact_sha256": decision.input.artifact_sha256,
        "report_input_sha256": decision.input.report_input_sha256,
        "report_input_bytes": decision.input.report_input_bytes,
        "prepared_report_equals_input": decision.input.prepared_report_equals_input,
        "acquisition_report_binding_sha256": decision.input.acquisition_report_binding_sha256,
        "selected_news_source_ref_sha256": decision.input.selected_news_source_ref.as_ref().map(|source| &source.ref_sha256),
        "selected_news_source_ref_status": decision.input.selected_news_source_ref_status.reason(),
        "suppression_reason": suppression_reason,
        "send_attempted": decision.send_attempted,
        "report_observed": decision.report_observed,
        "send_id": decision.send_id,
        "targets": targets,
        "mark_attempted": decision.mark_attempted,
        "legacy_succeeded": decision.legacy_succeeded,
    }))?;
    Ok(ChainDecisionObservation {
        gate_sha256,
        binding_sha256,
        send_id: decision.send_id.map(str::to_owned),
        scope: "prepared",
        reason: suppression_reason.unwrap_or("none"),
        prepared_report_equals_input: Some(decision.input.prepared_report_equals_input),
        target_count: decision.targets.len(),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SelectedNewsSourceRefStatus {
    Observed,
    NoAcquisition,
    VerifiedEmpty,
    InvalidAvailableEmpty,
    NewsUnavailable,
    MissingSourceTime,
    Rejected,
}

impl SelectedNewsSourceRefStatus {
    pub(super) fn status(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::NoAcquisition | Self::VerifiedEmpty => "not_applicable",
            Self::InvalidAvailableEmpty | Self::NewsUnavailable | Self::MissingSourceTime => {
                "unobserved"
            }
            Self::Rejected => "rejected",
        }
    }

    pub(super) fn reason(self) -> &'static str {
        match self {
            Self::Observed => "none",
            Self::NoAcquisition => "no_acquisition",
            Self::VerifiedEmpty => "verified_empty",
            Self::InvalidAvailableEmpty => "invalid_available_empty",
            Self::NewsUnavailable => "news_unavailable",
            Self::MissingSourceTime => "missing_source_at",
            Self::Rejected => "lineage_validation_failed",
        }
    }
}

#[derive(Debug)]
pub(super) struct ChainReportInputObservation {
    pub phase: ChainPhase,
    pub schedule_date: NaiveDate,
    pub prepared_business_date: NaiveDate,
    pub artifact_sha256: String,
    pub artifact_bytes: usize,
    pub report_input_sha256: String,
    pub report_input_bytes: usize,
    pub prepared_report_equals_input: bool,
    pub acquisition_sha256: Option<String>,
    pub acquisition_report_binding_sha256: Option<String>,
    pub selected_news_source_ref: Option<ChainSelectedNewsSourceRefV1>,
    pub selected_news_source_ref_status: SelectedNewsSourceRefStatus,
    pub prepared_macro_source_status:
        stock_analysis::pipeline::chain_analysis::preparation::SourceStatus,
    pub coverage: &'static str,
    pub covered_inputs: &'static str,
}

/// A weak, invocation-local binding of one Custom target to the prepared
/// report, the retained send input and its already observed source/artifact
/// identity. The entity is the first built Reqwest request; the production
/// notification client rejects redirects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ChainCustomRequestObservation {
    pub send_id: String,
    pub target_index: usize,
    pub outcome: WeakOutcomeKind,
    pub artifact_sha256: String,
    pub acquisition_report_binding_sha256: Option<String>,
    pub report_input_sha256: String,
    pub prepared_report_equals_input: bool,
    pub built_target_sha256: Option<String>,
    pub built_body_sha256: Option<String>,
    pub built_body_bytes: Option<usize>,
    pub built_body_matches_prepared: Option<bool>,
    pub built_body_matches_report_input: Option<bool>,
    pub response_url_sha256: Option<String>,
    pub response_target_differs: Option<bool>,
    pub binding_sha256: String,
}

/// Compare each first built Custom HTTP entity with the UTF-8 report input
/// retained from this send. The send report still supplies only a weak outcome.
pub(super) fn observe_custom_requests(
    prepared: &PreparedChainAnalysis,
    input: &ChainReportInputObservation,
    report_input: &[u8],
    report: &NotificationSendReport,
) -> Result<Vec<ChainCustomRequestObservation>> {
    anyhow::ensure!(
        prepared.business_date() == input.prepared_business_date,
        "产业链 Custom 请求与准备对象业务日期不一致"
    );
    anyhow::ensure!(
        report_input.len() == input.report_input_bytes
            && format!("{:x}", Sha256::digest(report_input)) == input.report_input_sha256
            && (prepared.report().as_bytes() == report_input) == input.prepared_report_equals_input,
        "产业链 Custom 请求与已观察的报告输入不一致"
    );
    let report_input = std::str::from_utf8(report_input)?;
    report
        .attempts()
        .iter()
        .filter(|attempt| attempt.channel() == NotificationChannel::Custom)
        .map(|attempt| {
            let entity = attempt.request_entity();
            let built_target_sha256 = entity.map(|entity| entity.target_sha256().to_owned());
            let built_body_sha256 = entity.map(|entity| entity.body_sha256().to_owned());
            let built_body_bytes = entity.map(|entity| entity.body_len());
            let built_body_matches_prepared =
                entity.map(|entity| entity.matches_custom_content(prepared.report()));
            let built_body_matches_report_input =
                entity.map(|entity| entity.matches_custom_content(report_input));
            let response_url_sha256 = entity
                .and_then(|entity| entity.response_url_sha256())
                .map(str::to_owned);
            let response_target_differs =
                entity.and_then(|entity| entity.response_target_differs());
            let outcome = match attempt.outcome() {
                WeakOutcomeKind::Accepted => "accepted",
                WeakOutcomeKind::Rejected => "rejected",
                WeakOutcomeKind::Unknown => "unknown",
            };
            let binding = serde_json::to_vec(&serde_json::json!({
                "schema": "chain-custom-request-observation-v2",
                "phase": input.phase.as_str(),
                "schedule_date": input.schedule_date.to_string(),
                "prepared_business_date": input.prepared_business_date.to_string(),
                "send_id": report.send_id(),
                "target_index": attempt.target_index(),
                "outcome": outcome,
                "artifact_sha256": &input.artifact_sha256,
                "acquisition_report_binding_sha256": &input.acquisition_report_binding_sha256,
                "report_input_sha256": &input.report_input_sha256,
                "prepared_report_equals_input": input.prepared_report_equals_input,
                "built_target_sha256": &built_target_sha256,
                "built_body_sha256": &built_body_sha256,
                "built_body_bytes": built_body_bytes,
                "built_body_matches_prepared": built_body_matches_prepared,
                "built_body_matches_report_input": built_body_matches_report_input,
                "response_url_sha256": &response_url_sha256,
                "response_target_differs": response_target_differs,
            }))?;
            Ok(ChainCustomRequestObservation {
                send_id: report.send_id().to_owned(),
                target_index: attempt.target_index(),
                outcome: attempt.outcome(),
                artifact_sha256: input.artifact_sha256.clone(),
                acquisition_report_binding_sha256: input.acquisition_report_binding_sha256.clone(),
                report_input_sha256: input.report_input_sha256.clone(),
                prepared_report_equals_input: input.prepared_report_equals_input,
                built_target_sha256,
                built_body_sha256,
                built_body_bytes,
                built_body_matches_prepared,
                built_body_matches_report_input,
                response_url_sha256,
                response_target_differs,
                binding_sha256: format!("{:x}", Sha256::digest(binding)),
            })
        })
        .collect()
}

/// `report_input` is the UTF-8 report supplied to the legacy sender, not a
/// rendered transport payload or channel wire bytes.
pub(super) fn observe(
    phase: ChainPhase,
    schedule_date: NaiveDate,
    prepared: &PreparedChainAnalysis,
    report_input: &[u8],
    acquisition: Option<&ChainAcquisitionEvidence>,
) -> Result<ChainReportInputObservation> {
    let artifact = prepared.to_artifact_bytes()?;
    let (selected_news_source_ref, selected_news_source_ref_status) = match acquisition {
        None => (None, SelectedNewsSourceRefStatus::NoAcquisition),
        Some(retained) => match retained.selected_news_source_ref(prepared) {
            Ok(Some(source_ref)) => (Some(source_ref), SelectedNewsSourceRefStatus::Observed),
            Ok(None) => (
                None,
                match &retained.news {
                    ChainNewsEvidence::VerifiedEmpty(_) => {
                        SelectedNewsSourceRefStatus::VerifiedEmpty
                    }
                    ChainNewsEvidence::InvalidAvailableEmpty(_) => {
                        SelectedNewsSourceRefStatus::InvalidAvailableEmpty
                    }
                    ChainNewsEvidence::Unavailable { .. } => {
                        SelectedNewsSourceRefStatus::NewsUnavailable
                    }
                    ChainNewsEvidence::Available { .. } => SelectedNewsSourceRefStatus::Rejected,
                },
            ),
            Err(error)
                if error
                    .downcast_ref::<MissingSelectedNewsSourceTime>()
                    .is_some() =>
            {
                (None, SelectedNewsSourceRefStatus::MissingSourceTime)
            }
            Err(_) => (None, SelectedNewsSourceRefStatus::Rejected),
        },
    };
    let acquisition_sha256 = acquisition
        .map(|retained| {
            anyhow::ensure!(
                retained.business_date == prepared.business_date(),
                "产业链采集与准备的业务日期不一致"
            );
            retained.sha256()
        })
        .transpose()?;
    let artifact_sha256 = format!("{:x}", Sha256::digest(&artifact));
    let report_input_sha256 = format!("{:x}", Sha256::digest(report_input));
    let acquisition_report_binding_sha256 = acquisition_sha256
        .as_ref()
        .map(|source_sha256| {
            let binding = serde_json::to_vec(&serde_json::json!({
                "schema": "chain-acquisition-report-binding-v1",
                "source_sha256": source_sha256,
                "artifact_sha256": artifact_sha256,
                "report_input_sha256": report_input_sha256,
            }))?;
            Ok::<_, anyhow::Error>(format!("{:x}", Sha256::digest(binding)))
        })
        .transpose()?;
    Ok(ChainReportInputObservation {
        phase,
        schedule_date,
        prepared_business_date: prepared.business_date(),
        artifact_sha256,
        artifact_bytes: artifact.len(),
        report_input_sha256,
        report_input_bytes: report_input.len(),
        prepared_report_equals_input: prepared.report().as_bytes() == report_input,
        acquisition_sha256,
        acquisition_report_binding_sha256,
        selected_news_source_ref,
        selected_news_source_ref_status,
        prepared_macro_source_status: prepared.macro_source().status().clone(),
        coverage: COVERAGE,
        covered_inputs: if acquisition.is_some() {
            ACQUISITION_INPUTS
        } else {
            REPORT_INPUTS
        },
    })
}

#[cfg(test)]
mod decision_tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};

    fn gate(phase: ChainPhase, status: ChainScheduleStatus, time: &str) -> ChainGateCapture {
        ChainGateCapture::new(
            phase,
            NaiveDate::from_ymd_opt(2026, 9, 29).unwrap(),
            DateTime::parse_from_rfc3339(&format!("2026-09-29T{time}+08:00")).unwrap(),
            true,
            status,
        )
    }

    fn target(index: usize, channel: &str, outcome: WeakOutcomeKind) -> ChainWeakTargetResult {
        ChainWeakTargetResult {
            channel: channel.into(),
            target_index: index,
            outcome,
            built_target_sha256: None,
            built_body_sha256: None,
        }
    }

    #[test]
    fn closed_uncertain_and_window_expiry_bind_schedule_only_without_report_facts() {
        for phase in [ChainPhase::Preopen, ChainPhase::Postclose] {
            let opening = if phase == ChainPhase::Preopen {
                "09:05:00"
            } else {
                "15:30:00"
            };
            let expiry = if phase == ChainPhase::Preopen {
                "09:15:00"
            } else {
                "15:35:00"
            };
            let closed =
                observe_schedule_only(&gate(phase, ChainScheduleStatus::Closed, opening)).unwrap();
            let uncertain =
                observe_schedule_only(&gate(phase, ChainScheduleStatus::Uncertain, opening))
                    .unwrap();
            let outside =
                observe_schedule_only(&gate(phase, ChainScheduleStatus::Ready, expiry)).unwrap();
            assert_eq!(closed.scope, "schedule_only");
            assert_eq!(closed.send_id, None);
            assert_eq!(closed.reason, "already_closed");
            assert_eq!(closed.prepared_report_equals_input, None);
            assert_eq!(uncertain.reason, "uncertain_needs_review");
            assert_eq!(outside.reason, "outside_send_window");
            assert_ne!(closed.binding_sha256, uncertain.binding_sha256);
            assert_ne!(uncertain.binding_sha256, outside.binding_sha256);
            assert_eq!(outside.target_count, 0);
            assert!(
                observe_schedule_only(&gate(phase, ChainScheduleStatus::Ready, opening)).is_err()
            );
        }
    }

    #[tokio::test]
    async fn prepared_binding_uses_one_prior_session_artifact_and_all_captured_decision_facts() {
        let business_date = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let schedule_date = NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();
        let preparations = Rc::new(Cell::new(0));
        let prepared = test_prepared(business_date, preparations.clone()).await;
        let mut input = observe(
            ChainPhase::Preopen,
            schedule_date,
            &prepared,
            prepared.report().as_bytes(),
            None,
        )
        .unwrap();
        let gate = gate(ChainPhase::Preopen, ChainScheduleStatus::Ready, "09:05:00");
        let targets = [
            target(0, "wechat", WeakOutcomeKind::Accepted),
            target(1, "custom", WeakOutcomeKind::Unknown),
        ];
        let bind = |input: &ChainReportInputObservation,
                    suppression,
                    targets: &[ChainWeakTargetResult],
                    send_attempted,
                    send_id,
                    mark_attempted,
                    legacy_succeeded| {
            observe_prepared_decision(
                &gate,
                ChainPreparedDecision {
                    input,
                    suppression,
                    send_attempted,
                    report_observed: send_attempted,
                    send_id,
                    targets,
                    mark_attempted,
                    legacy_succeeded,
                },
            )
        };
        let first = bind(
            &input,
            None,
            &targets,
            true,
            Some("TEST_CODE_SEND_A"),
            false,
            false,
        )
        .unwrap();
        assert_eq!(first.scope, "prepared");
        assert_eq!(first.target_count, 2);
        assert_eq!(first.send_id.as_deref(), Some("TEST_CODE_SEND_A"));
        assert_eq!(first.prepared_report_equals_input, Some(true));
        assert_eq!(first.gate_sha256, gate.sha256().unwrap());
        assert_eq!(
            first,
            bind(
                &input,
                None,
                &targets,
                true,
                Some("TEST_CODE_SEND_A"),
                false,
                false
            )
            .unwrap()
        );
        assert_ne!(
            first.binding_sha256,
            bind(
                &input,
                None,
                &targets,
                true,
                Some("TEST_CODE_SEND_B"),
                false,
                false
            )
            .unwrap()
            .binding_sha256
        );
        for suppression in [
            ChainSendSuppression::NotificationDisabled,
            ChainSendSuppression::NoConfiguredChannel,
            ChainSendSuppression::BeforeSendRejected,
        ] {
            let observed = bind(&input, Some(suppression), &[], false, None, false, false).unwrap();
            assert_eq!(observed.reason, suppression.as_str());
            assert_eq!(observed.prepared_report_equals_input, Some(true));
        }
        let changed_outcome = [
            target(0, "wechat", WeakOutcomeKind::Unknown),
            target(1, "custom", WeakOutcomeKind::Accepted),
        ];
        assert_ne!(
            first.binding_sha256,
            bind(
                &input,
                None,
                &changed_outcome,
                true,
                Some("TEST_CODE_SEND_A"),
                false,
                false
            )
            .unwrap()
            .binding_sha256
        );
        assert_ne!(
            first.binding_sha256,
            bind(
                &input,
                Some(ChainSendSuppression::BeforeSendRejected),
                &[],
                false,
                None,
                false,
                false
            )
            .unwrap()
            .binding_sha256
        );
        assert_ne!(
            first.binding_sha256,
            bind(
                &input,
                None,
                &targets,
                true,
                Some("TEST_CODE_SEND_A"),
                true,
                true
            )
            .unwrap()
            .binding_sha256
        );
        let altered_input = observe(
            ChainPhase::Preopen,
            schedule_date,
            &prepared,
            b"changed report input",
            None,
        )
        .unwrap();
        let altered = bind(
            &altered_input,
            None,
            &targets,
            true,
            Some("TEST_CODE_SEND_A"),
            false,
            false,
        )
        .unwrap();
        assert_eq!(altered.prepared_report_equals_input, Some(false));
        assert_ne!(first.binding_sha256, altered.binding_sha256);
        input.acquisition_report_binding_sha256 = Some("TEST_CODE_DIFFERENT_SOURCE".into());
        assert_ne!(
            first.binding_sha256,
            bind(
                &input,
                None,
                &targets,
                true,
                Some("TEST_CODE_SEND_A"),
                false,
                false
            )
            .unwrap()
            .binding_sha256
        );
        let out_of_order = [
            target(1, "custom", WeakOutcomeKind::Unknown),
            target(0, "wechat", WeakOutcomeKind::Accepted),
        ];
        assert!(bind(
            &input,
            None,
            &out_of_order,
            true,
            Some("TEST_CODE_SEND_A"),
            false,
            false
        )
        .is_err());
        assert!(bind(&input, None, &targets, true, None, false, false).is_err());
        assert!(bind(&input, None, &[], false, None, false, false).is_err());
        assert!(bind(
            &input,
            Some(ChainSendSuppression::BeforeSendRejected),
            &targets,
            true,
            Some("TEST_CODE_SEND_A"),
            false,
            false
        )
        .is_err());
        assert!(bind(
            &input,
            Some(ChainSendSuppression::BeforeSendRejected),
            &[],
            false,
            None,
            true,
            false
        )
        .is_err());
        assert!(bind(
            &input,
            Some(ChainSendSuppression::NoConfiguredChannel),
            &[],
            false,
            None,
            false,
            true
        )
        .is_err());
        assert_eq!(preparations.get(), 1);
    }
}

#[cfg(test)]
pub(super) async fn test_prepared(
    date: NaiveDate,
    preparations: std::rc::Rc<std::cell::Cell<usize>>,
) -> PreparedChainAnalysis {
    test_prepared_with_macro(date, preparations, None).await
}

#[cfg(test)]
pub(super) async fn test_prepared_with_macro(
    date: NaiveDate,
    preparations: std::rc::Rc<std::cell::Cell<usize>>,
    macro_input: Option<String>,
) -> PreparedChainAnalysis {
    use stock_analysis::pipeline::chain_analysis::preparation::{
        prepare_chain_analysis_with_io, ChainPreparationIo,
    };
    struct ScriptedIo(std::rc::Rc<std::cell::Cell<usize>>);
    #[async_trait::async_trait(?Send)]
    impl ChainPreparationIo for ScriptedIo {
        fn validate_fixed_input(
            &mut self,
            _date: NaiveDate,
            _stocks: &[stock_analysis::market_data::TopStock],
            _macro_news: &Option<String>,
        ) -> Result<()> {
            self.0.set(self.0.get() + 1);
            Ok(())
        }
        async fn concepts(
            &mut self,
            _codes: &[String],
        ) -> Result<std::collections::HashMap<String, Vec<String>>> {
            panic!("empty scripted pool must not request concepts")
        }
    }
    prepare_chain_analysis_with_io(date, Vec::new(), macro_input, &mut ScriptedIo(preparations))
        .await
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::Cell,
        io::{Read, Write},
        net::TcpListener,
        rc::Rc,
        thread,
        time::Duration,
    };
    use stock_analysis::notification::{NotificationConfig, NotificationService};

    #[tokio::test]
    async fn separates_schedule_and_business_dates_and_labels_digest_scope() {
        let business_date = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let schedule_date = NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();
        let preparations = Rc::new(Cell::new(0));
        let prepared = test_prepared(business_date, preparations.clone()).await;
        let input = prepared.report().as_bytes();
        let observed = observe(ChainPhase::Preopen, schedule_date, &prepared, input, None).unwrap();
        assert_eq!(preparations.get(), 1);
        assert_eq!(observed.schedule_date, schedule_date);
        assert_eq!(observed.prepared_business_date, business_date);
        assert!(observed.prepared_report_equals_input);
        assert_eq!(observed.coverage, "incomplete");
        assert_eq!(observed.covered_inputs, REPORT_INPUTS);
        assert_eq!(observed.report_input_bytes, input.len());
        assert_eq!(
            observed.artifact_bytes,
            prepared.to_artifact_bytes().unwrap().len()
        );
        assert_eq!(observed.artifact_sha256.len(), 64);
        assert_eq!(observed.report_input_sha256.len(), 64);
    }

    #[tokio::test]
    async fn altered_report_input_is_a_visible_difference() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();
        let prepared = test_prepared(date, Rc::new(Cell::new(0))).await;
        let original = observe(
            ChainPhase::Postclose,
            date,
            &prepared,
            prepared.report().as_bytes(),
            None,
        )
        .unwrap();
        let altered = observe(
            ChainPhase::Postclose,
            date,
            &prepared,
            b"changed input",
            None,
        )
        .unwrap();
        assert!(!altered.prepared_report_equals_input);
        assert_ne!(original.report_input_sha256, altered.report_input_sha256);
        assert_eq!(original.artifact_sha256, altered.artifact_sha256);
    }

    #[tokio::test]
    async fn one_custom_send_binds_built_request_to_prepared_artifact_and_input() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();
        let preparations = Rc::new(Cell::new(0));
        let prepared = test_prepared(date, preparations.clone()).await;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/custom", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let size = stream.read(&mut buffer).unwrap();
                assert!(size > 0);
                request.extend_from_slice(&buffer[..size]);
                if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let body_start = end + 4;
                    let headers = std::str::from_utf8(&request[..body_start]).unwrap();
                    let body_len: usize = headers
                        .split("\r\n")
                        .filter_map(|line| line.split_once(':'))
                        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .map(|(_, value)| value.trim())
                        .unwrap()
                        .parse()
                        .unwrap();
                    if request.len() >= body_start + body_len {
                        break;
                    }
                }
            }
            let body = r#"{"ok":true}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
            request
        });
        let service = NotificationService::new(NotificationConfig {
            custom_webhook_urls: vec![url],
            ..NotificationConfig::default()
        });
        let report = service.send_report(prepared.report()).await;
        let input = observe(
            ChainPhase::Postclose,
            date,
            &prepared,
            prepared.report().as_bytes(),
            None,
        )
        .unwrap();
        let requests =
            observe_custom_requests(&prepared, &input, prepared.report().as_bytes(), &report)
                .unwrap();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.send_id, report.send_id());
        let gate = ChainGateCapture::new(
            ChainPhase::Postclose,
            date,
            DateTime::parse_from_rfc3339("2026-09-29T15:30:00+08:00").unwrap(),
            true,
            ChainScheduleStatus::Ready,
        );
        let targets = ChainWeakTargetResult::from_report(&report);
        let decision = observe_prepared_decision(
            &gate,
            ChainPreparedDecision {
                input: &input,
                suppression: None,
                send_attempted: true,
                report_observed: true,
                send_id: Some(report.send_id()),
                targets: &targets,
                mark_attempted: false,
                legacy_succeeded: false,
            },
        )
        .unwrap();
        assert_eq!(decision.send_id.as_deref(), Some(request.send_id.as_str()));
        assert_eq!(request.artifact_sha256, input.artifact_sha256);
        assert_eq!(request.report_input_sha256, input.report_input_sha256);
        assert!(request.prepared_report_equals_input);
        assert_eq!(request.built_body_matches_prepared, Some(true));
        assert_eq!(request.built_body_matches_report_input, Some(true));
        assert_eq!(request.response_target_differs, Some(false));
        assert_eq!(request.binding_sha256.len(), 64);

        let changed_input = observe(
            ChainPhase::Postclose,
            date,
            &prepared,
            b"changed input",
            None,
        )
        .unwrap();
        let changed =
            observe_custom_requests(&prepared, &changed_input, b"changed input", &report).unwrap();
        assert!(!changed[0].prepared_report_equals_input);
        assert_eq!(changed[0].built_body_matches_prepared, Some(true));
        assert_eq!(changed[0].built_body_matches_report_input, Some(false));
        assert_ne!(changed[0].binding_sha256, request.binding_sha256);
        assert!(observe_custom_requests(
            &prepared,
            &changed_input,
            prepared.report().as_bytes(),
            &report,
        )
        .is_err());
        assert_eq!(preparations.get(), 1);
        let sent = server.join().unwrap();
        assert!(sent.starts_with(b"POST /custom HTTP/1.1"));
        let body_start = sent
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
            .unwrap()
            + 4;
        let expected_body = serde_json::to_vec(&serde_json::json!({
            "content": prepared.report(),
        }))
        .unwrap();
        assert_eq!(&sent[body_start..], expected_body.as_slice());
    }
}
