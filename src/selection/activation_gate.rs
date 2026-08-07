//! BR-183/193 production activation gate.
//!
//! Decides the selection-v2 capability verdict from checked-in release
//! materials only (deterministic, storage-free). The gate never writes
//! configuration, never selects a last-known-good snapshot, and never
//! manufactures board evidence — every Disabled verdict names the exact
//! BR-193 `SelectionDisabledReason` token.
//!
//! Order of checks (first failure wins):
//!   1. activation file absent → `activation_missing`
//!   2. board artifact / chain snapshot / config hash / chronology
//!      verification (stages 1-5 of config activation preparation)
//!   3. trading-calendar authority files present
//!   4. otherwise → `Enabled`
//!
//! Schema amendment (BR-180 five-payload apply) is deliberately not part of
//! this gate: the production database currently has no selection-v2 tables,
//! which is the *designed* pre-BR-180 state, not an activation blocker.
//! `schema_not_amended` joins the chain when BR-180 releases the apply path.

use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::selection::activation_runtime::SelectionDisabledReason;
use crate::selection::config_activation_v2::{
    prepare_activation_materials, ACTIVATION_FILE_RELATIVE_PATH,
};
use crate::selection::trading_calendar_v2::{
    CALENDAR_MANIFEST_RELATIVE_PATH, NOTICE_MANIFEST_RELATIVE_PATH, RAW_NOTICE_ROOT_RELATIVE_PATH,
};

/// Capability verdict consumed by the process bootstrap.
#[derive(Debug, PartialEq, Eq)]
pub enum SelectionV2ActivationVerdict {
    Enabled,
    Disabled { reason_code: &'static str },
}

/// Evaluate the production selection-v2 activation against the checked-in
/// repository release materials.
pub fn evaluate_production_selection_v2_activation() -> SelectionV2ActivationVerdict {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let now = Utc::now();

    if !root.join(ACTIVATION_FILE_RELATIVE_PATH).is_file() {
        return disabled(SelectionDisabledReason::ActivationMissing);
    }

    match prepare_activation_materials(root, now) {
        Ok(materials) => {
            // Board artifact expiry beyond the release window: the artifact
            // itself is no longer the reviewed evidence → expired.
            if let Ok(expires) = rfc3339_parse(&materials.board_artifact_expires_at) {
                if now >= expires {
                    return disabled(SelectionDisabledReason::ActivationExpired);
                }
            }
        }
        Err(error) => return disabled(map_preparation_error(error.code)),
    }

    if !calendar_authority_complete(root) {
        return disabled(SelectionDisabledReason::TradingCalendarMissing);
    }

    SelectionV2ActivationVerdict::Enabled
}

fn disabled(reason: SelectionDisabledReason) -> SelectionV2ActivationVerdict {
    SelectionV2ActivationVerdict::Disabled {
        reason_code: reason.as_str(),
    }
}

/// Map a stage-1..5 preparation error onto the BR-193 fail-closed token.
fn map_preparation_error(code: &'static str) -> SelectionDisabledReason {
    if code.starts_with("board_") {
        return SelectionDisabledReason::BoardArtifactUnverified;
    }
    match code {
        "activation_expected_config_hash_mismatch" | "activation_chronology_invalid" => {
            SelectionDisabledReason::ActivationNotEffective
        }
        // Everything else about the activation file itself (schema, review
        // state, canonical timestamps, hash shape) keeps it from being a
        // released activation.
        code if code.starts_with("activation_") => SelectionDisabledReason::ActivationMissing,
        // Chain snapshot / proposal / executable-input drift means the
        // checked-in proposal no longer matches the activation claim.
        _ => SelectionDisabledReason::ProposalMissing,
    }
}

fn rfc3339_parse(value: &str) -> Result<chrono::DateTime<Utc>, ()> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|_| ())
}

/// The three calendar authority files must all be present for the calendar
/// capability to be constructible.
fn calendar_authority_complete(root: &Path) -> bool {
    [
        CALENDAR_MANIFEST_RELATIVE_PATH,
        NOTICE_MANIFEST_RELATIVE_PATH,
        RAW_NOTICE_ROOT_RELATIVE_PATH,
    ]
    .into_iter()
    .all(|relative| root.join(PathBuf::from(relative)).exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_activation_file_reports_activation_missing() {
        // In this test environment CARGO_MANIFEST_DIR points at the real
        // repository root, which currently ships no activation file — the
        // gate must fail closed with the exact token.
        let verdict = evaluate_production_selection_v2_activation();
        match verdict {
            SelectionV2ActivationVerdict::Disabled { reason_code } => {
                assert_eq!(reason_code, "activation_missing");
            }
            SelectionV2ActivationVerdict::Enabled => {
                panic!("gate must not evaluate Enabled without release materials in CI");
            }
        }
    }

    #[test]
    fn calendar_absence_classification_is_fail_closed() {
        // A wholly absent calendar without a reviewed marker is not a valid
        // authority; the gate maps it to the missing token before this is
        // ever reached — direct classification stays closed.
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert!(!calendar_authority_complete(root));
    }

    #[test]
    fn verdict_reasons_are_static_tokens() {
        let verdict = evaluate_production_selection_v2_activation();
        match verdict {
            SelectionV2ActivationVerdict::Disabled { reason_code } => {
                assert!(
                    reason_code.is_ascii()
                        && reason_code.contains('_'),
                    "reason token must be a snake_case static string, got {reason_code}"
                );
            }
            SelectionV2ActivationVerdict::Enabled => {}
        }
    }
}
