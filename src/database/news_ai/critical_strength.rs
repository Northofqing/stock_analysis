//! Fixed cross-profile news-base barrier and immutable score association.
use super::*;
use crate::monitor::news_ai::{AuditedCriticalNews, CriticalModelResult, CriticalNewsEvidence, NewsBaseIdentity};

pub(super) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS news_ai_n01_score (
 assessment_id TEXT PRIMARY KEY NOT NULL REFERENCES news_ai_assessment(assessment_id),
 news_base_sha256 TEXT NOT NULL CHECK(length(news_base_sha256)=64),
 evidence_json TEXT NOT NULL CHECK(length(evidence_json)>0),
 evidence_sha256 TEXT NOT NULL CHECK(length(evidence_sha256)=64)
);
CREATE TRIGGER IF NOT EXISTS trg_news_ai_n01_score_no_update BEFORE UPDATE ON news_ai_n01_score
BEGIN SELECT RAISE(ABORT,'BR244 immutable N01 score'); END;
CREATE TRIGGER IF NOT EXISTS trg_news_ai_n01_score_no_delete BEFORE DELETE ON news_ai_n01_score
BEGIN SELECT RAISE(ABORT,'BR244 immutable N01 score'); END;
"#;

#[derive(QueryableByName)]
struct ScoreRow {
    #[diesel(sql_type=Text)] assessment_id: String,
    #[diesel(sql_type=Text)] news_base_sha256: String,
    #[diesel(sql_type=Text)] evidence_json: String,
    #[diesel(sql_type=Text)] evidence_sha256: String,
}
pub(super) fn validate_scores(conn: &mut SqliteConnection) -> NewsAiAssessmentAuditResult<()> {
    let rows = diesel::sql_query("SELECT assessment_id,news_base_sha256,evidence_json,evidence_sha256 FROM news_ai_n01_score ORDER BY assessment_id")
        .load::<ScoreRow>(conn)?;
    for row in rows {
        let evidence: CriticalNewsEvidence = serde_json::from_str(&row.evidence_json).map_err(|e|audit(e.to_string()))?;
        let bytes = evidence.canonical().map_err(|e|audit(e.to_string()))?;
        let fact = evidence.fact().map_err(|e|audit(e.to_string()))?;
        if bytes != row.evidence_json.as_bytes()
            || evidence.digest().map_err(|e|audit(e.to_string()))? != row.evidence_sha256
            || evidence.assessment_id() != row.assessment_id
            || NewsBaseIdentity::from_fact(&fact).map_err(|e|audit(e.to_string()))?.digest() != row.news_base_sha256 {
            return Err(audit("N01 immutable association changed"));
        }
        let assessment = load_by_assessment_id(conn, &row.assessment_id)?.ok_or_else(||audit("N01 assessment missing"))?;
        validate_persisted_row(conn, &assessment)?;
        let frozen = load_frozen_recovery_fact(conn, &row.assessment_id)?.ok_or_else(||audit("N01 source snapshot missing"))?;
        if frozen.recovery_snapshot_canonical().map_err(|e|audit(e.to_string()))?
            != fact.recovery_snapshot_canonical().map_err(|e|audit(e.to_string()))? {
            return Err(audit("N01 source snapshot differs from score"));
        }
        let link = load_chain_for_row(conn, assessment.id)?;
        evidence.validate_assessment(&persisted_delivery_assessment(conn, &assessment)?, &link.record_hash)
            .map_err(|e|audit(e.to_string()))?;
    }
    Ok(())
}

pub(super) fn has_base(conn: &mut SqliteConnection, fact: &AdmittedNewsFact) -> NewsAiAssessmentAuditResult<bool> {
    validate_news_ai_assessment_chain(conn)?;
    validate_scores(conn)?;
    let global_match = super::global_critical::has_equity_base(conn,fact)?;
    let expected = NewsBaseIdentity::from_fact(fact).map_err(|e|audit(e.to_string()))?;
    let provider = source_provider_tag(fact.provider())?;
    let mut found = false;
    // Do not stop at the first match: every retained revision/profile for this source
    // must have a verified immutable snapshot. Missing legacy data is Unknown.
    for row in load_rows(conn)? {
        if row.source_provider != provider || row.source_item_id != fact.item_id() { continue; }
        validate_persisted_row(conn, &row)?;
        let retained = load_frozen_recovery_fact(conn, &row.assessment_id)?
            .ok_or_else(||audit("cross-profile news-base snapshot missing; not ExactAbsent"))?;
        let actual = NewsBaseIdentity::from_fact(&retained).map_err(|e|audit(e.to_string()))?;
        if source_provider_tag(retained.provider())? != row.source_provider || retained.item_id() != row.source_item_id {
            return Err(audit("cross-profile source identity changed"));
        }
        found |= actual == expected;
    }
    Ok(found || global_match)
}

pub(super) fn append(conn: &mut SqliteConnection, result: CriticalModelResult)
    -> NewsAiAssessmentAuditResult<(crate::monitor::news_ai::AuditedNewsAiAssessment, AuditedCriticalNews)> {
    conn.immediate_transaction::<_,NewsAiAssessmentAuditError,_>(|conn| {
        if has_base(conn, result.request.fact())? {
            return Err(audit("news-base appeared after pre-call barrier; no N01 capability minted"));
        }
        let input = NewsAiAssessmentAuditInput::from_core(&result.request, &result.assessment)?;
        let receipt = insert_assessment_in_transaction(conn, &input)?;
        let evidence = result.evidence(&receipt.record_hash).map_err(|e|audit(e.to_string()))?;
        let bytes = evidence.canonical().map_err(|e|audit(e.to_string()))?;
        let json = String::from_utf8(bytes).map_err(|e|audit(e.to_string()))?;
        let digest = evidence.digest().map_err(|e|audit(e.to_string()))?;
        let base = NewsBaseIdentity::from_fact(result.request.fact()).map_err(|e|audit(e.to_string()))?.digest();
        let audited = crate::monitor::news_ai::AuditedNewsAiAssessment::try_from_assessment_audit(
            result.request, result.assessment, &receipt.record_hash).map_err(|e|audit(e.to_string()))?;
        freeze_recovery_fact(conn, &receipt.assessment_id, audited.delivery().fact())?;
        let card = freeze_delivery_card(conn, &receipt.assessment_id, &audited.delivery().render_card())?;
        let audited = audited.with_frozen_card(card).map_err(|e|audit(e.to_string()))?;
        diesel::sql_query("INSERT INTO news_ai_n01_score(assessment_id,news_base_sha256,evidence_json,evidence_sha256) VALUES(?,?,?,?)")
            .bind::<Text,_>(&receipt.assessment_id).bind::<Text,_>(&base).bind::<Text,_>(&json).bind::<Text,_>(&digest).execute(conn)?;
        validate_scores(conn)?;
        let readback = diesel::sql_query("SELECT assessment_id,news_base_sha256,evidence_json,evidence_sha256 FROM news_ai_n01_score WHERE assessment_id=?")
            .bind::<Text,_>(&receipt.assessment_id).get_result::<ScoreRow>(conn)?;
        if readback.evidence_json != json || readback.evidence_sha256 != digest || readback.news_base_sha256 != base {
            return Err(audit("N01 exact score readback failed"));
        }
        let token = AuditedCriticalNews::mint(evidence, &readback.evidence_sha256).map_err(|e|audit(e.to_string()))?;
        Ok((audited,token))
    })
}
impl DatabaseManager {
    pub fn has_audited_news_base(&self, fact: &AdmittedNewsFact) -> NewsAiAssessmentAuditResult<bool> {
        let mut conn = self.get_conn().map_err(|e|NewsAiAssessmentAuditError::Connection(e.to_string()))?;
        conn.transaction::<_,NewsAiAssessmentAuditError,_>(|conn|has_base(conn,fact))
    }
    pub fn append_audited_critical_news(&self, result: CriticalModelResult)
        -> NewsAiAssessmentAuditResult<(crate::monitor::news_ai::AuditedNewsAiAssessment,AuditedCriticalNews)> {
        let mut conn = self.get_conn().map_err(|e|NewsAiAssessmentAuditError::Connection(e.to_string()))?;
        append(&mut conn,result)
    }
}
