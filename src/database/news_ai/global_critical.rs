//! Independent immutable GlobalCritical audit and cross-purpose absence barrier.
use super::*;
use crate::monitor::news_ai::{
    AuditedGlobalCriticalNews, GlobalCriticalEvidence, GlobalCriticalFact,
    GlobalCriticalModelResult, NewsBaseIdentity,
};
pub(super) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS news_ai_global_critical_v1 (
 seq INTEGER PRIMARY KEY AUTOINCREMENT,
 assessment_id TEXT NOT NULL UNIQUE CHECK(length(assessment_id)=64),
 evidence_json TEXT NOT NULL CHECK(length(evidence_json)>0),
 evidence_sha256 TEXT NOT NULL CHECK(length(evidence_sha256)=64),
 previous_hash TEXT NOT NULL,
 record_hash TEXT NOT NULL CHECK(length(record_hash)=64)
);
CREATE TRIGGER IF NOT EXISTS trg_global_critical_no_update BEFORE UPDATE ON news_ai_global_critical_v1 BEGIN SELECT RAISE(ABORT,'immutable global N01'); END;
CREATE TRIGGER IF NOT EXISTS trg_global_critical_no_delete BEFORE DELETE ON news_ai_global_critical_v1 BEGIN SELECT RAISE(ABORT,'immutable global N01'); END;
"#;
const GENESIS: &str = "BR244_GLOBAL_CRITICAL_GENESIS_V1";
#[derive(QueryableByName)]
struct Row {
    #[diesel(sql_type=BigInt)]
    seq: i64,
    #[diesel(sql_type=Text)]
    assessment_id: String,
    #[diesel(sql_type=Text)]
    evidence_json: String,
    #[diesel(sql_type=Text)]
    evidence_sha256: String,
    #[diesel(sql_type=Text)]
    previous_hash: String,
    #[diesel(sql_type=Text)]
    record_hash: String,
}
fn chain_hash(seq: i64, id: &str, body: &str, digest: &str, previous: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"BR244_GLOBAL_CRITICAL_CHAIN_V1\0");
    h.update(seq.to_be_bytes());
    for v in [id, body, digest, previous] {
        h.update((v.len() as u64).to_be_bytes());
        h.update(v.as_bytes());
    }
    format!("{:x}", h.finalize())
}
fn rows(conn: &mut SqliteConnection) -> NewsAiAssessmentAuditResult<Vec<Row>> {
    Ok(diesel::sql_query("SELECT seq,assessment_id,evidence_json,evidence_sha256,previous_hash,record_hash FROM news_ai_global_critical_v1 ORDER BY seq").load::<Row>(conn)?)
}
fn validate_rows(values: &[Row]) -> NewsAiAssessmentAuditResult<Vec<GlobalCriticalEvidence>> {
    let mut previous = GENESIS.to_owned();
    let mut seq = 0i64;
    let mut complete = Vec::new();
    for r in values {
        seq = seq
            .checked_add(1)
            .ok_or_else(|| audit("global audit sequence overflow"))?;
        let value: GlobalCriticalEvidence =
            serde_json::from_str(&r.evidence_json).map_err(|e| audit(e.to_string()))?;
        if r.seq != seq
            || r.previous_hash != previous
            || value.assessment_id() != r.assessment_id
            || value.canonical().map_err(|e| audit(e.to_string()))? != r.evidence_json.as_bytes()
            || value.digest().map_err(|e| audit(e.to_string()))? != r.evidence_sha256
            || chain_hash(
                r.seq,
                &r.assessment_id,
                &r.evidence_json,
                &r.evidence_sha256,
                &r.previous_hash,
            ) != r.record_hash
        {
            return Err(audit(
                "global immutable chain/source/model/readback changed",
            ));
        }
        previous = r.record_hash.clone();
        complete.push(value);
    }
    Ok(complete)
}
pub(super) fn has_equity_base(
    conn: &mut SqliteConnection,
    fact: &AdmittedNewsFact,
) -> NewsAiAssessmentAuditResult<bool> {
    let expected = NewsBaseIdentity::from_fact(fact).map_err(|e| audit(e.to_string()))?;
    let values = validate_rows(&rows(conn)?)?;
    let mut found = false;
    for value in values {
        let actual = value.news_base().map_err(|e| audit(e.to_string()))?;
        found |= actual == expected || actual.same_text_revision(&expected);
    }
    Ok(found)
}
pub(super) fn has_base(
    conn: &mut SqliteConnection,
    fact: &GlobalCriticalFact,
) -> NewsAiAssessmentAuditResult<bool> {
    validate_news_ai_assessment_chain(conn)?;
    // Full old scores are verified independently; no history token is minted.
    critical_strength::validate_scores(conn)?;
    let expected = fact.news_base();
    let provider = source_provider_tag(fact.provider())?;
    let mut found = false;
    for r in load_rows(conn)? {
        validate_persisted_row(conn, &r)?;
        let source = match load_frozen_recovery_fact(conn, &r.assessment_id)? {
            Some(source) => source,
            None => {
                if r.source_provider == provider && r.source_item_id == fact.item_id() {
                    return Err(audit(
                        "global matching source legacy snapshot Unknown, not ExactAbsent",
                    ));
                }
                // A validated unrelated legacy key supplies no text fact. This is
                // current-key/retained-text absence, never whole-history absence.
                continue;
            }
        };
        if source_provider_tag(source.provider())? != r.source_provider
            || source.item_id() != r.source_item_id
        {
            return Err(audit("global cross-purpose legacy source changed"));
        }
        let actual = NewsBaseIdentity::from_fact(&source).map_err(|e| audit(e.to_string()))?;
        // Identical text across purposes is conservative even across source items.
        found |= actual == expected || actual.same_text_revision(&expected);
    }
    for value in validate_rows(&rows(conn)?)? {
        let actual = value.news_base().map_err(|e| audit(e.to_string()))?;
        // Same-purpose key keeps provider+item+revision, never profile/version.
        found |= actual == expected;
    }
    Ok(found)
}
pub(super) fn append(
    conn: &mut SqliteConnection,
    result: GlobalCriticalModelResult,
) -> NewsAiAssessmentAuditResult<AuditedGlobalCriticalNews> {
    conn.immediate_transaction::<_,NewsAiAssessmentAuditError,_>(|conn| {
        let evidence=result.evidence;let fact=evidence.fact().map_err(|e|audit(e.to_string()))?;
        if has_base(conn,&fact)? { return Err(audit("global source appeared after pre-call barrier; no capability")); }
        let before=rows(conn)?;validate_rows(&before)?;
        let seq=before.last().map_or(Ok(1i64),|r|r.seq.checked_add(1).ok_or_else(||audit("global sequence overflow")))?;
        let previous=before.last().map_or(GENESIS,|r|r.record_hash.as_str());
        let id=evidence.assessment_id();let body=String::from_utf8(evidence.canonical().map_err(|e|audit(e.to_string()))?).map_err(|e|audit(e.to_string()))?;
        let digest=evidence.digest().map_err(|e|audit(e.to_string()))?;let record=chain_hash(seq,&id,&body,&digest,previous);
        diesel::sql_query("INSERT INTO news_ai_global_critical_v1(seq,assessment_id,evidence_json,evidence_sha256,previous_hash,record_hash) VALUES(?,?,?,?,?,?)")
            .bind::<BigInt,_>(seq).bind::<Text,_>(&id).bind::<Text,_>(&body).bind::<Text,_>(&digest).bind::<Text,_>(previous).bind::<Text,_>(&record).execute(conn)?;
        let after=rows(conn)?;validate_rows(&after)?;
        let actual=after.last().ok_or_else(||audit("global immutable exact readback absent"))?;
        if after.len()!=before.len()+1 || actual.seq!=seq || actual.assessment_id!=id || actual.evidence_json!=body || actual.evidence_sha256!=digest || actual.previous_hash!=previous || actual.record_hash!=record { return Err(audit("global exact full readback differs")); }
        // Diesel only returns this token after the real transaction COMMIT succeeds.
        AuditedGlobalCriticalNews::mint(evidence,&actual.evidence_sha256).map_err(|e|audit(e.to_string()))
    })
}
impl DatabaseManager {
    pub fn has_audited_global_news_base(
        &self,
        fact: &GlobalCriticalFact,
    ) -> NewsAiAssessmentAuditResult<bool> {
        let mut conn = self
            .get_conn()
            .map_err(|e| NewsAiAssessmentAuditError::Connection(e.to_string()))?;
        conn.transaction::<_, NewsAiAssessmentAuditError, _>(|conn| has_base(conn, fact))
    }
    pub fn append_audited_global_critical_news(
        &self,
        result: GlobalCriticalModelResult,
    ) -> NewsAiAssessmentAuditResult<AuditedGlobalCriticalNews> {
        let mut conn = self
            .get_conn()
            .map_err(|e| NewsAiAssessmentAuditError::Connection(e.to_string()))?;
        append(&mut conn, result)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) const GOOD: &str = r#"{"importance":91,"uncertainty":"TEST_CODE risk","core_logic":"TEST_CODE actual source"}"#;
    pub(crate) fn fixture(conn: &mut SqliteConnection, item: &str) -> AuditedGlobalCriticalNews {
        let fact =
            crate::monitor::news_ai::global_test_fact(item, &format!("TEST_CODE macro {item}"));
        append(
            conn,
            crate::monitor::news_ai::global_test_result(fact, GOOD).unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn news_global_n01_sqlite_cross_purpose_readback_and_two_coordinators() {
        let mut conn = super::super::tests::connection();
        let title = "TEST_CODE exact source-bound contract";
        let fact = crate::monitor::news_ai::global_test_fact("TEST_CODE_GLOBAL_FIRST", title);
        let token = append(
            &mut conn,
            crate::monitor::news_ai::global_test_result(fact.clone(), GOOD).unwrap(),
        )
        .unwrap();
        assert_eq!(token.evidence_sha256(), token.evidence().digest().unwrap());
        assert!(has_base(&mut conn, &fact).unwrap());
        assert!(append(
            &mut conn,
            crate::monitor::news_ai::global_test_result(fact.clone(), GOOD).unwrap()
        )
        .is_err());
        let (equity, assessment) =
            super::super::tests::core_assessment_for("TEST_CODE_EQUITY_OTHER", "TEST_CODE_600519");
        assert!(critical_strength::has_base(&mut conn, equity.fact()).unwrap()); // cross-purpose text, different item
                                                                                 // Ordinary four-field analysis is not gated by the new scoring purpose.
        super::super::append_audited_news_ai_assessment_on_conn(&mut conn, equity, assessment)
            .unwrap();
        let mut old = super::super::tests::connection();
        let (equity, assessment) =
            super::super::tests::core_assessment_for("TEST_CODE_LEGACY", "TEST_CODE_600519");
        super::super::append_audited_news_ai_assessment_on_conn(&mut old, equity, assessment)
            .unwrap();
        let global = crate::monitor::news_ai::global_test_fact("TEST_CODE_GLOBAL_OTHER", title);
        assert!(has_base(&mut old, &global).unwrap());
        assert!(append(
            &mut old,
            crate::monitor::news_ai::global_test_result(global.clone(), GOOD).unwrap()
        )
        .is_err());
        // Complete retained text blocks across items before its immutable snapshot is removed.
        old.batch_execute("DROP TRIGGER trg_news_ai_delivery_recovery_snapshot_no_delete; DELETE FROM news_ai_delivery_recovery_snapshot").unwrap();
        assert!(!has_base(&mut old, &global).unwrap());
        append(
            &mut old,
            crate::monitor::news_ai::global_test_result(global.clone(), GOOD).unwrap(),
        )
        .unwrap();
        assert_eq!(rows(&mut old).unwrap().len(), 1); // unrelated missing key may append
        let matching = crate::monitor::news_ai::global_test_fact(
            "TEST_CODE_LEGACY",
            "TEST_CODE changed same source revision",
        );
        let before = rows(&mut old).unwrap().len();
        assert!(has_base(&mut old, &matching).is_err());
        assert!(append(
            &mut old,
            crate::monitor::news_ai::global_test_result(matching, GOOD).unwrap()
        )
        .is_err());
        assert_eq!(rows(&mut old).unwrap().len(), before); // matching missing never becomes false
                                                           // A known unrelated snapshot is still checked; a corrupt body is not a missing legacy fact.
        let mut damaged = super::super::tests::connection();
        let (request, assessment) =
            super::super::tests::core_assessment_for("TEST_CODE_DAMAGED_OTHER", "TEST_CODE_600519");
        super::super::append_audited_news_ai_assessment_on_conn(&mut damaged, request, assessment)
            .unwrap();
        damaged.batch_execute("DROP TRIGGER trg_news_ai_delivery_recovery_snapshot_no_update; UPDATE news_ai_delivery_recovery_snapshot SET fact_snapshot='{}'").unwrap();
        assert!(has_base(&mut damaged, &global).is_err());
        assert!(append(
            &mut damaged,
            crate::monitor::news_ai::global_test_result(global.clone(), GOOD).unwrap()
        )
        .is_err());
        assert!(rows(&mut damaged).unwrap().is_empty());
        // V3 must retain its required original recovery envelope even for another item.
        use crate::monitor::news_ai::{
            ModelCallReceipt, NewsAiAnalysisProfile, NewsAiAssessment, NewsAiChainContext,
            NewsAiIdentityV3, NewsAiRequest,
        };
        use chrono::TimeZone;
        let mut v3 = super::super::tests::connection();
        let (legacy, _) =
            super::super::tests::core_assessment_for("TEST_CODE_V3_OTHER", "TEST_CODE_600519");
        let profile = NewsAiAnalysisProfile::for_configured_model(
            "TEST_CODE_MODEL_PROVIDER",
            "TEST_CODE_configured_model",
        )
        .unwrap();
        let identity = NewsAiIdentityV3::from_fact(legacy.fact(), &profile).unwrap();
        let request = NewsAiRequest::try_new_v3(
            legacy.fact().clone(),
            legacy.market().clone(),
            vec![],
            identity,
            NewsAiChainContext::default(),
        )
        .unwrap();
        let response = r#"{"impact":"positive","confidence":82,"uncertainty":"TEST_CODE execution may vary","core_logic":"TEST_CODE evidence-bound positive impact"}"#;
        let receipt = ModelCallReceipt::try_new(
            "TEST_CODE_MODEL_PROVIDER",
            "TEST_CODE_actual_model",
            Some("TEST_CODE_REQUEST_CORE"),
            request.normalized_prompt(),
            response,
            Utc.with_ymd_and_hms(2026, 7, 27, 1, 0, 3).unwrap(),
            Utc.with_ymd_and_hms(2026, 7, 27, 1, 0, 4).unwrap(),
        )
        .unwrap();
        let assessment =
            NewsAiAssessment::from_model_response(&request, response, Some(receipt)).unwrap();
        super::super::append_audited_news_ai_assessment_on_conn(&mut v3, request, assessment)
            .unwrap();
        v3.batch_execute("DROP TRIGGER trg_news_ai_delivery_recovery_snapshot_no_delete; DELETE FROM news_ai_delivery_recovery_snapshot").unwrap();
        assert!(has_base(&mut v3, &global).is_err());
        assert!(append(
            &mut v3,
            crate::monitor::news_ai::global_test_result(global.clone(), GOOD).unwrap()
        )
        .is_err());
        assert!(rows(&mut v3).unwrap().is_empty());
        // A bad assessment chain remains Unknown even when its unrelated snapshot is missing.
        let mut bad_chain = super::super::tests::connection();
        let (request, assessment) =
            super::super::tests::core_assessment_for("TEST_CODE_CHAIN_OTHER", "TEST_CODE_600519");
        super::super::append_audited_news_ai_assessment_on_conn(
            &mut bad_chain,
            request,
            assessment,
        )
        .unwrap();
        bad_chain.batch_execute("DROP TRIGGER trg_news_ai_delivery_recovery_snapshot_no_delete; DELETE FROM news_ai_delivery_recovery_snapshot; DROP TRIGGER trg_news_ai_assessment_chain_no_delete; DELETE FROM news_ai_assessment_chain").unwrap();
        assert!(has_base(&mut bad_chain, &global).is_err());
        assert!(append(
            &mut bad_chain,
            crate::monitor::news_ai::global_test_result(global.clone(), GOOD).unwrap()
        )
        .is_err());
        assert!(rows(&mut bad_chain).unwrap().is_empty());
        let file = tempfile::NamedTempFile::new().unwrap();
        let path = file.path().to_str().unwrap();
        let mut first = SqliteConnection::establish(path).unwrap();
        super::super::create_schema(&mut first).unwrap();
        let mut second = SqliteConnection::establish(path).unwrap();
        super::super::create_schema(&mut second).unwrap();
        let race = crate::monitor::news_ai::global_test_fact(
            "TEST_CODE_GLOBAL_RACE",
            "TEST_CODE unique macro race",
        );
        assert!(!has_base(&mut first, &race).unwrap() && !has_base(&mut second, &race).unwrap());
        append(
            &mut first,
            crate::monitor::news_ai::global_test_result(race.clone(), GOOD).unwrap(),
        )
        .unwrap();
        assert!(append(
            &mut second,
            crate::monitor::news_ai::global_test_result(race.clone(), GOOD).unwrap()
        )
        .is_err());
        drop(first);
        drop(second);
        let mut cold = SqliteConnection::establish(path).unwrap();
        super::super::create_schema(&mut cold).unwrap();
        assert!(has_base(&mut cold, &race).unwrap());
        assert_eq!(validate_rows(&rows(&mut cold).unwrap()).unwrap().len(), 1);
        // A true SQLite trigger aborts after INSERT; no token and no partial row survive.
        cold.batch_execute("CREATE TRIGGER TEST_CODE_ABORT_GLOBAL AFTER INSERT ON news_ai_global_critical_v1 BEGIN SELECT RAISE(ABORT,'TEST_CODE readback boundary'); END;").unwrap();
        let failed = crate::monitor::news_ai::global_test_fact(
            "TEST_CODE_GLOBAL_ABORT",
            "TEST_CODE different body",
        );
        assert!(append(
            &mut cold,
            crate::monitor::news_ai::global_test_result(failed, GOOD).unwrap()
        )
        .is_err());
        assert_eq!(rows(&mut cold).unwrap().len(), 1);
        cold.batch_execute("DROP TRIGGER trg_global_critical_no_update; UPDATE news_ai_global_critical_v1 SET record_hash='0000000000000000000000000000000000000000000000000000000000000000'").unwrap();
        assert!(has_base(&mut cold, &race).is_err());
    }
}
