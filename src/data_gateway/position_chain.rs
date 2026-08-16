//! BR-085/BR-170 Magic TDX position-chain assignment.

use crate::data_gateway::{
    BatchEvidence, BoardDataGateway, BoardKind, BoardMembershipRecord, GatewayBatch, GatewayError,
};
use crate::database::DatabaseManager;
use futures::{stream, StreamExt};
use crate::magic_compat::ProviderId;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

const CAPABILITY: &str = "position-chain-assignment";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CanonicalPositionMembership {
    pub board_code: String,
    pub board_name: String,
    #[serde(serialize_with = "serialize_board_kind")]
    pub kind: BoardKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionChainAssignment {
    pub code: String,
    pub primary: CanonicalPositionMembership,
    pub memberships: Vec<CanonicalPositionMembership>,
    pub evidence: BatchEvidence,
    pub assignment_id: String,
    pub content_hash: String,
}

fn serialize_board_kind<S>(kind: &BoardKind, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(match kind {
        BoardKind::Industry => "industry",
        BoardKind::Concept => "concept",
        BoardKind::Region => "region",
    })
}

fn invalid_evidence(message: impl Into<String>) -> GatewayError {
    GatewayError::invalid_evidence(CAPABILITY, None, message)
}

fn canonical_hash<T: Serialize>(domain: &str, value: &T) -> Result<String, GatewayError> {
    let payload = serde_json::to_vec(value).map_err(|error| {
        invalid_evidence(format!("serialize position chain assignment: {error}"))
    })?;
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(payload);
    Ok(hex::encode(hasher.finalize()))
}

fn kind_priority(kind: BoardKind) -> u8 {
    match kind {
        BoardKind::Industry => 0,
        BoardKind::Concept => 1,
        BoardKind::Region => 2,
    }
}

fn valid_position_code(code: &str) -> bool {
    let numeric_code = code.len() == 6 && code.bytes().all(|byte| byte.is_ascii_digit());
    #[cfg(test)]
    let test_code = code.strip_prefix("TEST_CODE_").is_some_and(|suffix| {
        suffix.len() == 6 && suffix.bytes().all(|byte| byte.is_ascii_digit())
    });
    #[cfg(not(test))]
    let test_code = false;
    numeric_code || test_code
}

fn validate_evidence(evidence: &BatchEvidence) -> Result<(), GatewayError> {
    if evidence.provider != ProviderId::Tdx {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(evidence.provider),
            "position chain assignment requires Magic TDX evidence",
        ));
    }
    for (field, value) in [
        ("source", Some(evidence.source.as_str())),
        ("source_at", evidence.source_at.as_deref()),
        ("observed_at", Some(evidence.observed_at.as_str())),
        ("batch_id", Some(evidence.batch_id.as_str())),
    ] {
        if let Some(value) = value {
            if value.trim().is_empty() || value.chars().any(char::is_control) {
                return Err(invalid_evidence(format!(
                    "{field} must be non-empty canonical text when present"
                )));
            }
        }
    }
    Ok(())
}

pub fn derive_position_chain(
    code: &str,
    batch: GatewayBatch<BoardMembershipRecord>,
) -> Result<Option<PositionChainAssignment>, GatewayError> {
    if !valid_position_code(code) {
        return Err(GatewayError::invalid_request(
            CAPABILITY,
            format!("position code must be exactly six digits: {code:?}"),
        ));
    }
    let evidence = batch.evidence().clone();
    validate_evidence(&evidence)?;
    let mut memberships = batch
        .records()
        .iter()
        .filter(|record| matches!(record.kind, BoardKind::Industry | BoardKind::Concept))
        .map(|record| {
            if record.instrument_code != code {
                return Err(invalid_evidence(format!(
                    "membership instrument {} does not match request {code}",
                    record.instrument_code
                )));
            }
            for (field, value) in [
                ("board_code", record.board_code.as_str()),
                ("board_name", record.board_name.as_str()),
            ] {
                if value.trim().is_empty() || value.chars().any(char::is_control) {
                    return Err(invalid_evidence(format!(
                        "{field} must be non-empty canonical text"
                    )));
                }
            }
            Ok(CanonicalPositionMembership {
                board_code: record.board_code.trim().to_owned(),
                board_name: record.board_name.trim().to_owned(),
                kind: record.kind,
            })
        })
        .collect::<Result<Vec<_>, GatewayError>>()?;
    memberships.sort_by(|left, right| {
        kind_priority(left.kind)
            .cmp(&kind_priority(right.kind))
            .then_with(|| left.board_code.cmp(&right.board_code))
            .then_with(|| left.board_name.cmp(&right.board_name))
    });
    if memberships.windows(2).any(|pair| {
        pair[0].kind == pair[1].kind
            && pair[0].board_code == pair[1].board_code
            && pair[0].board_name != pair[1].board_name
    }) {
        return Err(invalid_evidence(
            "conflicting board identity has more than one board name",
        ));
    }
    memberships.dedup();
    let Some(primary) = memberships.first().cloned() else {
        return Ok(None);
    };
    let evidence_hash_payload = (
        format!("{:?}", evidence.provider),
        evidence.source.as_str(),
        evidence.source_at.as_deref(),
        evidence.observed_at.as_str(),
        evidence.batch_id.as_str(),
    );
    let content_hash = canonical_hash(
        "BR170_POSITION_CHAIN_CONTENT_V1",
        &(code, &memberships, evidence_hash_payload),
    )?;
    let assignment_id = canonical_hash(
        "BR170_POSITION_CHAIN_ID_V1",
        &(
            code,
            evidence.batch_id.as_str(),
            primary.board_code.as_str(),
        ),
    )?;
    Ok(Some(PositionChainAssignment {
        code: code.to_owned(),
        primary,
        memberships,
        evidence,
        assignment_id: format!("position-chain:{assignment_id}"),
        content_hash,
    }))
}

pub(crate) fn validate_position_chain_assignment(
    assignment: &PositionChainAssignment,
) -> Result<(), GatewayError> {
    let records = assignment
        .memberships
        .iter()
        .map(|membership| BoardMembershipRecord {
            instrument_code: assignment.code.clone(),
            board_code: membership.board_code.clone(),
            board_name: membership.board_name.clone(),
            kind: membership.kind,
        })
        .collect();
    let derived = derive_position_chain(
        &assignment.code,
        GatewayBatch::Available {
            records,
            evidence: assignment.evidence.clone(),
        },
    )?
    .ok_or_else(|| invalid_evidence("assignment has no Industry or Concept membership"))?;
    if &derived != assignment {
        return Err(invalid_evidence(
            "assignment fields do not match canonical content and identity hashes",
        ));
    }
    Ok(())
}

pub(crate) trait PositionChainAssignmentSink {
    fn commit_assignment(&mut self, assignment: &PositionChainAssignment) -> Result<bool, String>;
    fn clear_assignment(&mut self, code: &str) -> Result<usize, String>;
}

struct DatabasePositionChainSink<'a> {
    database: &'a DatabaseManager,
}

impl PositionChainAssignmentSink for DatabasePositionChainSink<'_> {
    fn commit_assignment(&mut self, assignment: &PositionChainAssignment) -> Result<bool, String> {
        self.database.commit_position_chain_assignment(assignment)
    }

    fn clear_assignment(&mut self, code: &str) -> Result<usize, String> {
        self.database.clear_position_chain_link(code)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PositionChainRefreshStatus {
    Assigned {
        inserted: bool,
    },
    VerifiedEmpty {
        cleared_positions: usize,
    },
    Failed {
        reason_code: String,
        retryable: bool,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionChainRefreshOutcome {
    pub code: String,
    pub status: PositionChainRefreshStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionChainRefreshReport {
    pub outcomes: Vec<PositionChainRefreshOutcome>,
}

impl PositionChainRefreshReport {
    pub fn assigned(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|outcome| matches!(outcome.status, PositionChainRefreshStatus::Assigned { .. }))
            .count()
    }

    pub fn verified_empty(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|outcome| {
                matches!(
                    outcome.status,
                    PositionChainRefreshStatus::VerifiedEmpty { .. }
                )
            })
            .count()
    }

    pub fn failed(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|outcome| matches!(outcome.status, PositionChainRefreshStatus::Failed { .. }))
            .count()
    }

    pub fn has_failures(&self) -> bool {
        self.failed() > 0
    }
}

fn gateway_failure(error: GatewayError) -> PositionChainRefreshStatus {
    PositionChainRefreshStatus::Failed {
        reason_code: error.reason_code().to_owned(),
        retryable: error.retryable(),
        message: error.to_string(),
    }
}

fn database_failure(message: String) -> PositionChainRefreshStatus {
    PositionChainRefreshStatus::Failed {
        reason_code: "database_failure".to_owned(),
        retryable: true,
        message,
    }
}

pub(crate) fn apply_position_chain_results<S: PositionChainAssignmentSink>(
    sink: &mut S,
    mut results: Vec<(
        String,
        Result<GatewayBatch<BoardMembershipRecord>, GatewayError>,
    )>,
) -> PositionChainRefreshReport {
    results.sort_by(|left, right| left.0.cmp(&right.0));
    let outcomes = results
        .into_iter()
        .map(|(code, result)| {
            let status = match result {
                Err(error) => gateway_failure(error),
                Ok(batch) => match derive_position_chain(&code, batch) {
                    Err(error) => gateway_failure(error),
                    Ok(Some(assignment)) => match sink.commit_assignment(&assignment) {
                        Ok(inserted) => PositionChainRefreshStatus::Assigned { inserted },
                        Err(error) => database_failure(error),
                    },
                    Ok(None) => match sink.clear_assignment(&code) {
                        Ok(cleared_positions) => {
                            PositionChainRefreshStatus::VerifiedEmpty { cleared_positions }
                        }
                        Err(error) => database_failure(error),
                    },
                },
            };
            PositionChainRefreshOutcome { code, status }
        })
        .collect();
    PositionChainRefreshReport { outcomes }
}

pub async fn acquire_candidate_position_chain(
    code: &str,
) -> Result<PositionChainAssignment, GatewayError> {
    let batch = BoardDataGateway::new().memberships(code).await?;
    derive_position_chain(code, batch)?.ok_or_else(|| {
        GatewayError::unavailable(
            CAPABILITY,
            Some(ProviderId::Tdx),
            false,
            format!(
                "complete Magic TDX membership batch has no Industry/Concept assignment for {code}"
            ),
        )
    })
}

pub async fn refresh_position_chains(
    database: &DatabaseManager,
    codes: &[String],
) -> PositionChainRefreshReport {
    let codes = codes.iter().cloned().collect::<BTreeSet<_>>();
    let mut results = stream::iter(codes.into_iter().map(|code| async move {
        let result = BoardDataGateway::new().memberships(&code).await;
        (code, result)
    }))
    .buffer_unordered(4)
    .collect::<Vec<_>>()
    .await;
    results.sort_by(|left, right| left.0.cmp(&right.0));
    let mut sink = DatabasePositionChainSink { database };
    apply_position_chain_results(&mut sink, results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::magic_compat::ProviderId;
    use std::collections::BTreeMap;

    fn evidence() -> BatchEvidence {
        BatchEvidence {
            provider: ProviderId::Tdx,
            source: "TEST_CODE_tdx-board-memberships".to_owned(),
            source_at: Some("2026-07-27T15:00:00+08:00".to_owned()),
            observed_at: "2026-07-27T15:00:01+08:00".to_owned(),
            batch_id: "TEST_CODE_board_batch_1".to_owned(),
        }
    }

    #[test]
    fn industry_membership_is_the_primary_position_chain() {
        let batch = GatewayBatch::Available {
            records: vec![
                BoardMembershipRecord {
                    instrument_code: "TEST_CODE_600396".to_owned(),
                    board_code: "TEST_CODE_CONCEPT".to_owned(),
                    board_name: "绿色电力".to_owned(),
                    kind: BoardKind::Concept,
                },
                BoardMembershipRecord {
                    instrument_code: "TEST_CODE_600396".to_owned(),
                    board_code: "TEST_CODE_INDUSTRY".to_owned(),
                    board_name: "电力".to_owned(),
                    kind: BoardKind::Industry,
                },
            ],
            evidence: evidence(),
        };

        let assignment = derive_position_chain("TEST_CODE_600396", batch)
            .expect("complete membership batch")
            .expect("position chain");

        assert_eq!(assignment.primary.board_code, "TEST_CODE_INDUSTRY");
        assert_eq!(assignment.primary.board_name, "电力");
        assert_eq!(assignment.primary.kind, BoardKind::Industry);
        assert_eq!(assignment.memberships.len(), 2);
    }

    #[test]
    fn conflicting_names_for_one_board_code_are_rejected() {
        let batch = GatewayBatch::Available {
            records: vec![
                BoardMembershipRecord {
                    instrument_code: "TEST_CODE_600396".to_owned(),
                    board_code: "TEST_CODE_INDUSTRY".to_owned(),
                    board_name: "电力".to_owned(),
                    kind: BoardKind::Industry,
                },
                BoardMembershipRecord {
                    instrument_code: "TEST_CODE_600396".to_owned(),
                    board_code: "TEST_CODE_INDUSTRY".to_owned(),
                    board_name: "火力发电".to_owned(),
                    kind: BoardKind::Industry,
                },
            ],
            evidence: evidence(),
        };

        let error = derive_position_chain("TEST_CODE_600396", batch)
            .expect_err("one board identity cannot have conflicting names");

        assert!(error.to_string().contains("conflicting board identity"));
    }

    #[test]
    fn verified_empty_membership_batch_has_no_position_chain() {
        let assignment =
            derive_position_chain("TEST_CODE_600396", GatewayBatch::VerifiedEmpty(evidence()))
                .expect("verified empty is valid source evidence");

        assert_eq!(assignment, None);
    }

    #[test]
    fn empty_batch_source_evidence_is_rejected() {
        let mut invalid = evidence();
        invalid.source.clear();

        let error = derive_position_chain("TEST_CODE_600396", GatewayBatch::VerifiedEmpty(invalid))
            .expect_err("empty source cannot become an auditable assignment decision");

        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(error.to_string().contains("source"));
    }

    #[test]
    fn concept_is_used_when_no_industry_membership_exists() {
        let batch = GatewayBatch::Available {
            records: vec![BoardMembershipRecord {
                instrument_code: "TEST_CODE_600396".to_owned(),
                board_code: "TEST_CODE_CONCEPT".to_owned(),
                board_name: "绿色电力".to_owned(),
                kind: BoardKind::Concept,
            }],
            evidence: evidence(),
        };

        let assignment = derive_position_chain("TEST_CODE_600396", batch)
            .expect("complete membership batch")
            .expect("concept assignment");

        assert_eq!(assignment.primary.kind, BoardKind::Concept);
        assert_eq!(assignment.primary.board_name, "绿色电力");
    }

    #[test]
    fn exact_duplicate_memberships_are_folded() {
        let membership = BoardMembershipRecord {
            instrument_code: "TEST_CODE_600396".to_owned(),
            board_code: "TEST_CODE_INDUSTRY".to_owned(),
            board_name: "电力".to_owned(),
            kind: BoardKind::Industry,
        };
        let batch = GatewayBatch::Available {
            records: vec![membership.clone(), membership],
            evidence: evidence(),
        };

        let assignment = derive_position_chain("TEST_CODE_600396", batch)
            .expect("complete membership batch")
            .expect("industry assignment");

        assert_eq!(assignment.memberships.len(), 1);
    }

    #[test]
    fn mismatched_instrument_membership_is_rejected() {
        let batch = GatewayBatch::Available {
            records: vec![BoardMembershipRecord {
                instrument_code: "TEST_CODE_000001".to_owned(),
                board_code: "TEST_CODE_INDUSTRY".to_owned(),
                board_name: "银行".to_owned(),
                kind: BoardKind::Industry,
            }],
            evidence: evidence(),
        };

        let error = derive_position_chain("TEST_CODE_600396", batch)
            .expect_err("cross-instrument evidence must fail closed");

        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(error.to_string().contains("does not match request"));
    }

    #[test]
    fn control_characters_in_membership_text_are_rejected() {
        let batch = GatewayBatch::Available {
            records: vec![BoardMembershipRecord {
                instrument_code: "TEST_CODE_600396".to_owned(),
                board_code: "TEST_CODE_INDUSTRY".to_owned(),
                board_name: "电力\n伪造日志".to_owned(),
                kind: BoardKind::Industry,
            }],
            evidence: evidence(),
        };

        let error = derive_position_chain("TEST_CODE_600396", batch)
            .expect_err("control characters are not canonical evidence");

        assert_eq!(error.reason_code(), "invalid_evidence");
    }

    #[test]
    fn invalid_position_code_is_rejected() {
        let error = derive_position_chain(
            "TEST_CODE_POSITION",
            GatewayBatch::VerifiedEmpty(evidence()),
        )
        .expect_err("production position identity is exactly six digits");

        assert_eq!(error.reason_code(), "invalid_request");
    }

    #[test]
    fn non_tdx_evidence_is_rejected() {
        let mut invalid = evidence();
        invalid.provider = ProviderId::Eastmoney;

        let error = derive_position_chain("TEST_CODE_600396", GatewayBatch::VerifiedEmpty(invalid))
            .expect_err("BR-170 assignment source is Magic TDX");

        assert_eq!(error.reason_code(), "invalid_evidence");
        assert!(error.to_string().contains("Magic TDX"));
    }

    #[derive(Default)]
    struct RecordingSink {
        assignments: BTreeMap<String, PositionChainAssignment>,
        cleared: Vec<String>,
    }

    impl PositionChainAssignmentSink for RecordingSink {
        fn commit_assignment(
            &mut self,
            assignment: &PositionChainAssignment,
        ) -> Result<bool, String> {
            self.assignments
                .insert(assignment.code.clone(), assignment.clone());
            Ok(true)
        }

        fn clear_assignment(&mut self, code: &str) -> Result<usize, String> {
            self.cleared.push(code.to_owned());
            Ok(0)
        }
    }

    #[test]
    fn one_failed_code_does_not_suppress_a_successful_assignment() {
        let successful_batch = GatewayBatch::Available {
            records: vec![BoardMembershipRecord {
                instrument_code: "TEST_CODE_600396".to_owned(),
                board_code: "TEST_CODE_INDUSTRY".to_owned(),
                board_name: "测试电力".to_owned(),
                kind: BoardKind::Industry,
            }],
            evidence: evidence(),
        };
        let unavailable = GatewayError::unavailable(
            CAPABILITY,
            Some(ProviderId::Tdx),
            true,
            "TEST_CODE provider unavailable",
        );
        let mut sink = RecordingSink::default();

        let report = apply_position_chain_results(
            &mut sink,
            vec![
                ("TEST_CODE_000001".to_owned(), Err(unavailable)),
                ("TEST_CODE_600396".to_owned(), Ok(successful_batch)),
            ],
        );

        assert_eq!(report.assigned(), 1);
        assert_eq!(report.failed(), 1);
        assert!(report.has_failures());
        assert!(sink.assignments.contains_key("TEST_CODE_600396"));
        assert!(!sink.assignments.contains_key("TEST_CODE_000001"));
        assert!(sink.cleared.is_empty());
    }
}
