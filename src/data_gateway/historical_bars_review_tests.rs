use super::*;
use crate::database::daily_change_review::{self as review, ReviewDecision};
use diesel::{Connection, RunQueryDsl};

fn setup() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    diesel::SqliteConnection,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_gateway.sqlite");
    let mut conn = diesel::SqliteConnection::establish(path.to_str().unwrap()).unwrap();
    review::install_owned_test_fixture(&mut conn);
    (dir, path, conn)
}
fn lifecycle(batch: &GatewayBatch<KlineData>) -> SecurityLifecycleContext {
    let mut evidence = batch.evidence().clone();
    evidence.batch_id = "TEST_CODE_lifecycle".into();
    SecurityLifecycleContext {
        instrument: crate::market_domain::InstrumentId::new(
            crate::market_domain::Exchange::Shenzhen,
            "TEST_CODE_300005",
            crate::market_domain::AssetClass::Equity,
        )
        .unwrap(),
        window_start: batch.records()[0].date,
        window_end: batch.records().last().unwrap().date,
        listing: ListingDateState::Available(
            super::super::security_lifecycle::AdmittedListingDate {
                listed_on: NaiveDate::from_ymd_opt(2010, 1, 1).unwrap(),
                evidence: evidence.clone(),
            },
        ),
        corporate_actions: CorporateActionState::VerifiedEmpty(evidence),
    }
}
#[derive(diesel::QueryableByName)]
struct Candidate {
    #[diesel(sql_type=diesel::sql_types::Text)]
    candidate_id: String,
}
fn candidates(conn: &mut diesel::SqliteConnection) -> Vec<String> {
    diesel::sql_query(
        "SELECT candidate_id FROM daily_change_review_event WHERE kind='Candidate' ORDER BY seq",
    )
    .load::<Candidate>(conn)
    .unwrap()
    .into_iter()
    .map(|r| r.candidate_id)
    .collect()
}

#[test]
fn task8_gateway_qualified_discover_offline_confirm_and_exact_shared_admission() {
    let (_dir, path, mut conn) = setup();
    let (batch, raw) = super::super::outcome_daily_bars::task8_review_fixture();
    let lc = lifecycle(&batch);
    let now = "2026-07-20T08:00:00Z".parse().unwrap();
    let pending =
        finalize_changes_on_conn(&mut conn, "TEST_CODE_300005", &batch, &lc, Some(&raw), now)
            .unwrap_err();
    assert_eq!(pending.reason_code(), "manual_confirmation_required");
    let ids = candidates(&mut conn);
    assert_eq!(
        ids.len(),
        1,
        "only 18.59→14.87, not 14.87→11.90, needs confirmation"
    );
    let original = review::review_on_conn(&mut conn, &ids[0], now).unwrap();
    drop(conn);
    let mut conn = diesel::SqliteConnection::establish(path.to_str().unwrap()).unwrap();
    assert!(
        finalize_changes_on_conn(&mut conn, "TEST_CODE_300005", &batch, &lc, None, now).is_err()
    );
    review::decide_on_conn(
        &mut conn,
        &ids[0],
        &original.evidence_token,
        ReviewDecision::Confirm,
        "TEST_CODE_op",
        "TEST_CODE_checked",
        now,
    )
    .unwrap();
    finalize_changes_on_conn(&mut conn, "TEST_CODE_300005", &batch, &lc, None, now)
        .expect("ordinary path uses same persisted fact");
    finalize_changes_on_conn(&mut conn, "TEST_CODE_300005", &batch, &lc, Some(&raw), now).unwrap();
    assert_eq!(
        review::review_on_conn(&mut conn, &ids[0], now)
            .unwrap()
            .snapshot,
        original.snapshot
    );
    let (mut changed, _) = super::super::outcome_daily_bars::task8_review_fixture();
    if let GatewayBatch::Available { records, .. } = &mut changed {
        records[1].close = 14.0;
    }
    assert!(
        finalize_changes_on_conn(&mut conn, "TEST_CODE_300005", &changed, &lc, None, now).is_err()
    );
    assert!(finalize_changes_on_conn(
        &mut conn,
        "TEST_CODE_300005",
        &changed,
        &lc,
        Some(&raw),
        now
    )
    .is_err());
    assert_eq!(
        candidates(&mut conn),
        ids,
        "unqualified changed batch cannot create a fact"
    );
    assert!(
        finalize_changes_on_conn(&mut conn, "TEST_CODE_OTHER", &batch, &lc, Some(&raw), now)
            .is_err()
    );
}

#[tokio::test]
async fn task8_gateway_missing_discovery_contract_and_lifecycle_create_zero_candidates() {
    let (_dir, _path, mut conn) = setup();
    let (batch, raw) = super::super::outcome_daily_bars::task8_review_fixture();
    let mut lc = lifecycle(&batch);
    let now = "2026-07-20T08:00:00Z".parse().unwrap();
    let unavailable = HistoricalBarsGateway::new()
        .pending_daily_change_confirmations_async("300005", 60)
        .await
        .unwrap_err();
    assert_eq!(
        unavailable.reason_code(),
        "daily_change_discovery_unavailable_v1"
    );
    assert!(!unavailable.retryable());
    let missing = finalize_changes_on_conn(&mut conn, "TEST_CODE_300005", &batch, &lc, None, now)
        .unwrap_err();
    assert_eq!(
        missing.reason_code(),
        "daily_change_discovery_unavailable_v1"
    );
    lc.corporate_actions = CorporateActionState::Unavailable(GatewayError::unavailable(
        "TEST_CODE",
        None,
        false,
        "TEST_CODE_lifecycle_missing",
    ));
    assert!(
        finalize_changes_on_conn(&mut conn, "TEST_CODE_300005", &batch, &lc, Some(&raw), now)
            .is_err()
    );
    assert!(candidates(&mut conn).is_empty());
}

#[test]
fn task8_gateway_unavailable_listing_metadata_cannot_create_candidate() {
    let (_dir, _path, mut conn) = setup();
    let (batch, raw) = super::super::outcome_daily_bars::task8_review_fixture();
    let mut lc = lifecycle(&batch);
    lc.listing = ListingDateState::Unavailable {
        evidence: Some(batch.evidence().clone()),
        error: GatewayError::unavailable(
            "SecurityLifecycleListing",
            Some(crate::market_domain::ProviderId::Tdx),
            false,
            "TEST_CODE_listing_metadata_unavailable",
        ),
    };

    let unavailable = finalize_changes_on_conn(
        &mut conn,
        "TEST_CODE_300005",
        &batch,
        &lc,
        Some(&raw),
        "2026-07-20T08:00:00Z".parse().unwrap(),
    )
    .unwrap_err();

    assert_eq!(
        unavailable.reason_code(),
        "listing_date_context_unavailable"
    );
    assert!(candidates(&mut conn).is_empty());
}
