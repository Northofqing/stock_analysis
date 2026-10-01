//! Read-only P-05 occurrence evidence across the prediction and counted stores.
//!
//! The two SQLite reads cannot form one atomic snapshot. Freezes and admitted
//! envelope identities are immutable; a terminal may advance after this read,
//! in which case a pending result is conservative and can be read again.

use crate::database::p05_prediction_freeze::{
    CandidateBoardFreezeError, FrozenCandidateBoardV2, FrozenCandidateRow,
};
use crate::database::DatabaseManager;
use crate::durable_delivery::{
    CandidateBoardCardObservationV1, CandidateBoardCardTerminalV1, CandidateBoardSourceLinkV1,
    CooldownScope, DeliveryEnvelope, DeliverySubKind, DurableDeliveryCoordinator,
    DurableDeliveryError, PushKind,
};

#[derive(Debug, thiserror::Error)]
pub enum CandidateBoardLinkError {
    #[error("P-05 cross-DB link mismatch: {0}")]
    Mismatch(&'static str),
    #[error("P-05 prediction freeze read: {0}")]
    Prediction(#[from] CandidateBoardFreezeError),
    #[error("P-05 counted read: {0}")]
    Counted(#[from] DurableDeliveryError),
}

/// `UnlinkedV1` never gains row membership, even if a later producer freeze
/// happens to use the same occurrence. `FrozenOnly` covers a committed freeze
/// before counted admission. `VerifiedV2` proves matching immutable evidence,
/// while its card terminal still determines whether physical acceptance exists.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CandidateBoardOccurrenceLinkV1 {
    Absent,
    UnlinkedV1 {
        card: CandidateBoardCardObservationV1,
    },
    FrozenOnly {
        source_sha256: String,
    },
    VerifiedV2 {
        card: CandidateBoardCardObservationV1,
        ordered_rows: Vec<FrozenCandidateRow>,
    },
}

impl CandidateBoardOccurrenceLinkV1 {
    /// The ordered prediction rows belong to an authoritative Accepted card.
    /// A single card receipt covers the set; it is not a per-row sink receipt.
    pub fn accepted_rows(&self) -> Option<&[FrozenCandidateRow]> {
        let Self::VerifiedV2 { card, ordered_rows } = self else {
            return None;
        };
        if card.terminal() == CandidateBoardCardTerminalV1::Accepted
            && card.terminal_attempt_identity().is_some()
            && card.disposition_identity().is_some()
            && card.terminal_evidence_sha256().is_some()
            && card.accepted_channel().is_some()
        {
            Some(ordered_rows)
        } else {
            None
        }
    }
}

/// Reconcile one exact occurrence without admission, sink calls, or writes.
/// Any missing or conflicting v2 prediction evidence is an error, rather than
/// an empty or delivered result. The counted reader validates terminal receipt
/// authority in one durable snapshot; the freeze reader rechecks actual rows in
/// one prediction snapshot.
pub fn read_candidate_board_occurrence_link(
    prediction_db: &DatabaseManager,
    counted: &DurableDeliveryCoordinator,
    business_date: &str,
    occurrence_identity: &str,
) -> Result<CandidateBoardOccurrenceLinkV1, CandidateBoardLinkError> {
    let observations =
        counted.candidate_board_card_observations_with_source_for_date(business_date)?;
    if !occurrence_identity.starts_with(&format!("candidate-board:{business_date}:")) {
        return Err(CandidateBoardLinkError::Mismatch(
            "occurrence does not match requested business date",
        ));
    }
    let observation = observations
        .into_iter()
        .find(|item| item.card().occurrence_identity() == occurrence_identity);
    if let Some(observation) = &observation {
        if matches!(
            observation.source_link(),
            CandidateBoardSourceLinkV1::UnlinkedV1
        ) {
            return Ok(CandidateBoardOccurrenceLinkV1::UnlinkedV1 {
                card: observation.card().clone(),
            });
        }
    }

    let freeze = prediction_db.read_candidate_board_v2_freeze(occurrence_identity)?;
    match (observation, freeze) {
        (None, None) => Ok(CandidateBoardOccurrenceLinkV1::Absent),
        (None, Some(freeze)) => Ok(CandidateBoardOccurrenceLinkV1::FrozenOnly {
            source_sha256: freeze.source_sha256().to_owned(),
        }),
        (Some(_), None) => Err(CandidateBoardLinkError::Mismatch(
            "v2 counted occurrence has no verified prediction freeze",
        )),
        (Some(observation), Some(freeze)) => {
            let CandidateBoardSourceLinkV1::DeclaredV2 { ordered_rows } = observation.source_link()
            else {
                return Err(CandidateBoardLinkError::Mismatch(
                    "counted source version changed during read",
                ));
            };
            let card = observation.card();
            if freeze.business_date() != business_date
                || freeze.occurrence_identity() != occurrence_identity
                || freeze.source_sha256() != card.source_binding_sha256()
                || freeze.rendered_sha256() != card.rendered_content_sha256()
                || ordered_rows.len() != freeze.ordered_rows().len()
                || ordered_rows
                    .iter()
                    .zip(freeze.ordered_rows())
                    .any(|(declared, actual)| {
                        declared.prediction_row_id() != actual.prediction_row_id()
                            || declared.code() != actual.code()
                    })
            {
                return Err(CandidateBoardLinkError::Mismatch(
                    "v2 counted source differs from prediction freeze",
                ));
            }
            let expected = frozen_candidate_board_envelope(&freeze)?;
            if expected.cooldown_scope != CooldownScope::Global
                || expected.decision_identity != card.decision_identity()
                || expected.canonical_sha256()? != card.envelope_sha256()
            {
                return Err(CandidateBoardLinkError::Mismatch(
                    "v2 counted decision differs from frozen envelope",
                ));
            }
            Ok(CandidateBoardOccurrenceLinkV1::VerifiedV2 {
                card: card.clone(),
                ordered_rows: freeze.ordered_rows().to_vec(),
            })
        }
    }
}

/// Shared exact factory consumes the actual owned freeze reader's opaque value.
/// It does not grant counted admission or physical receipt authority.
pub(crate) fn frozen_candidate_board_envelope(
    freeze: &FrozenCandidateBoardV2,
) -> Result<DeliveryEnvelope, DurableDeliveryError> {
    DeliveryEnvelope::new(
        freeze.business_date(),
        PushKind::CandidateBoard,
        DeliverySubKind::None,
        "GLOBAL",
        freeze.occurrence_identity(),
        freeze.source_sha256(),
        freeze.source_canonical().to_vec(),
        freeze.source_sha256(),
        freeze.rendered_bytes().to_vec(),
        false,
        None,
    )
}
