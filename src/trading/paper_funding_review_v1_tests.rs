use super::*;
use crate::database::global_schema_v1::{candidate_v7, investment_v8, paper_v6};
use crate::trading::paper_book_v2::{
    cutover_for_isolated_test, TestCutoverFault, TestCutoverRequest,
};
use crate::trading::paper_book_v2_budget_v1::{
    InitialLotAllocation, LotDisposition, ProfitPolicy, POLICY_VERSION,
};
use crate::trading::paper_ledger::{
    Mark, Money, PaperCommand, PaperLedger, RiskPolicyV1, SeedLot, SeedManifest,
};
use chrono::{NaiveDate, TimeZone, Utc};
use diesel::connection::SimpleConnection;
use diesel::Connection;
use std::sync::Arc;
const ACCOUNT: &str = "TEST_CODE_FUNDING_ACCOUNT";
fn at() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 28, 1, 30, 0).unwrap()
}
struct Fixture {
    _dir: tempfile::TempDir,
    db: Arc<DatabaseManager>,
}
impl Fixture {
    fn new(n: usize) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("TEST_CODE_funding_")
            .tempdir()
            .unwrap();
        let db = Arc::new(
            DatabaseManager::open_frozen_catalog_for_isolated_test(
                dir.path().join("TEST_CODE_funding.db"),
            )
            .unwrap(),
        );
        crate::database::paper_ledger_schema_v1::create_schema(&mut db.get_conn().unwrap())
            .unwrap();
        db.get_conn()
            .unwrap()
            .batch_execute("PRAGMA application_id=1398035265; PRAGMA user_version=2")
            .unwrap();
        let lots: Vec<_> = (0..n)
            .map(|i| SeedLot {
                code: format!("TEST_CODE_{i:06}"),
                name: "explicit isolated lot".into(),
                quantity: 100,
                reported_cost: None,
                sellable_from: None,
                sellability_evidence: None,
            })
            .collect();
        let marks = lots
            .iter()
            .map(|l| Mark {
                code: l.code.clone(),
                price: Money::from_micros(10_000_000),
                observed_at: at(),
                source: "TEST_CODE_explicit_mark".into(),
            })
            .collect();
        let seed = SeedManifest {
            account_id: ACCOUNT.into(),
            epoch_id: "TEST_CODE_FUNDING_V1".into(),
            command_id: "TEST_CODE_SEED".into(),
            cutover_at: at(),
            account_effective_at: at(),
            positions_effective_at: at(),
            source_reference: "TEST_CODE_explicit_seed".into(),
            source_hash: "a".repeat(64),
            approved_by: "TEST_CODE_isolated_only".into(),
            cash: Money::from_micros(100_000_000_000),
            original_total: Money::from_micros(
                100_000_000_000 + i64::try_from(n).unwrap() * 1_000_000_000,
            ),
            excluded_residual: None,
            lots,
            marks,
            policy: RiskPolicyV1 {
                max_position_bps: 10_000,
                cash_floor_bps: 0,
                max_slippage_bps: 200,
            },
        };
        let old = seed.binding().unwrap();
        PaperLedger::open(&db, &at)
            .apply(PaperCommand::Seed(seed))
            .unwrap();
        let fee =
            crate::performance::fee_policy::AShareFeePolicyV2::fixed_compatibility_assumption();
        {
            let mut c = db.get_conn().unwrap();
            crate::database::daily_change_review_schema_v1::create_schema(&mut c).unwrap();
            c.batch_execute("PRAGMA user_version=3").unwrap();
            crate::database::paper_book_owner_schema_v1::install_catalog_v4_for_isolated_test(
                &mut c, &fee,
            )
            .unwrap();
            crate::database::paper_book_owner_schema_v2::install_catalog_v5_for_isolated_test(
                &mut c,
            )
            .unwrap();
        }
        let snapshot = crate::trading::paper_ledger::verified_v1_snapshot_on(
            &mut db.get_conn().unwrap(),
            &old,
        )
        .unwrap();
        cutover_for_isolated_test(
            &db,
            &TestCutoverRequest {
                old_binding: old,
                new_epoch_id: "TEST_CODE_FUNDING_V2".into(),
                cutover_id: "TEST_CODE_CUTOVER".into(),
                command_id: "TEST_CODE_GENESIS".into(),
                expected_v1_version: snapshot.version,
                expected_v1_head_hash: snapshot.event_hash,
                expected_v1_projection_hash: snapshot.projection_hash,
                reviewed_fee_policy: fee,
            },
            TestCutoverFault::None,
        )
        .unwrap();
        paper_v6::prepare_final_selection_for_isolated_v5_test(&db).unwrap();
        paper_v6::migrate_catalog6_for_isolated_test(&db).unwrap();
        candidate_v7::migrate_catalog7_for_isolated_test(&db).unwrap();
        investment_v8::migrate_catalog8_for_isolated_test(&db).unwrap();
        Self { _dir: dir, db }
    }
    fn inputs(&self) -> (Binding, Projection) {
        investment_catalog8_session(&self.db)
            .unwrap()
            .with_readonly_catalog8(
                |c, p| {
                    let w = &mut Work::new();
                    let (b, raw) = extract(c, p, ACCOUNT, w)?;
                    Ok::<_, Error>((b, codec::projection(&raw, w)?))
                },
                |_, _, _| Ok(()),
            )
            .unwrap()
    }
    fn proposal(&self) -> Proposal {
        let (b, g) = self.inputs();
        make_proposal(&b, &g)
    }
    fn review(&self, p: &Proposal) -> Result<ObservedFundingReviewV1, Error> {
        review_funding_proposal(&self.db, ACCOUNT, &serde_json::to_vec(p).unwrap())
    }
    fn fingerprint(&self) -> String {
        let mut c = self.db.get_conn().unwrap();
        let a = diesel::sql_query(
            "SELECT hex(projection_bytes) AS value FROM paper_book_v2_head WHERE account_id=?",
        )
        .bind::<Text, _>(ACCOUNT)
        .get_result::<TextValue>(&mut c)
        .unwrap()
        .value;
        let b=diesel::sql_query("SELECT group_concat(name||':'||sql,';') AS value FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY name").get_result::<TextValue>(&mut c).unwrap().value;
        format!("{a}{b}")
    }
}
#[derive(QueryableByName)]
struct TextValue {
    #[diesel(sql_type=Text)]
    value: String,
}
fn make_proposal(b: &Binding, g: &Projection) -> Proposal {
    let mut allocations: Vec<_> = g
        .lots
        .iter()
        .enumerate()
        .map(|(i, l)| InitialLotAllocation {
            lot_id: l.lot_id.clone(),
            original_quantity: l.quantity,
            disposition: if i % 2 == 0 {
                LotDisposition::AllocatedToStrategy
            } else {
                LotDisposition::UnassignedReadOnly
            },
            chain_id: (i % 2 == 0).then(|| "TEST_CODE_CHAIN".into()),
        })
        .collect();
    allocations.sort_by(|a, b| a.lot_id.cmp(&b.lot_id));
    Proposal {
        schema: "paper-funding-proposal-v1".into(),
        version: 1,
        account_id: b.account_id.clone(),
        epoch_id: b.epoch_id.clone(),
        cutover_id: b.cutover_id.clone(),
        genesis_manifest_hash: b.genesis_manifest_hash.clone(),
        genesis_event_hash: b.genesis_event_hash.clone(),
        genesis_projection_hash: b.genesis_projection_hash.clone(),
        fee_policy_instance_id: b.fee_policy_instance_id.clone(),
        budget: BudgetRecord {
            version: POLICY_VERSION.into(),
            family_id: "TEST_CODE_FAMILY".into(),
            effective_from: NaiveDate::from_ymd_opt(2026, 9, 28).unwrap(),
            effective_through: NaiveDate::from_ymd_opt(2026, 10, 30).unwrap(),
            authorized_budget_micro_cny: 200_000_000_000,
            initial_strategy_cash_micro_cny: 50_000_000_000,
            concentration_bps: 10_000,
            chain_exposure_bps: 10_000,
            cash_floor_bps: 0,
            max_order_exposure_micro_cny: 100_000_000_000,
            original_seed_reference: format!("paper-v1-manifest-sha256:{}", b.v1_manifest_hash),
            review_reference: "TEST_CODE_review_claim_only".into(),
            profit_policy: ProfitPolicy::ReinvestWithinFixedAuthorizedBudget,
            initial_lots: allocations,
        },
    }
}
fn reset() {
    HITS.with(|h| h.set([0; 10]));
    HOOK.with(|h| *h.borrow_mut() = None);
    TAIL_FAIL.with(|h| h.set(false));
}
#[test]
fn funding_actual_nonempty_genesis_consistent_and_readonly() {
    let f = Fixture::new(2);
    let p = f.proposal();
    let before = f.fingerprint();
    reset();
    let r = f.review(&p).unwrap();
    assert_eq!(r.outcome(), Outcome::ConsistentProposal);
    assert_eq!(
        r.initial_cash().unwrap(),
        &p.budget.initial_cash(&f.inputs().1).unwrap()
    );
    assert_eq!(f.fingerprint(), before);
    assert_eq!(HITS.with(|h| h.get())[2], 1);
    let stored = read_stored_funding_review(r.canonical_bytes()).unwrap();
    assert_eq!(stored.review_id(), r.review_id());
    assert_eq!(stored.canonical_bytes(), r.canonical_bytes());
}
#[test]
fn funding_actual_cash_only_and_mixed_32_lot_controls() {
    for n in [0, 32] {
        let f = Fixture::new(n);
        let p = f.proposal();
        let r = f.review(&p).unwrap();
        assert_eq!(r.outcome(), Outcome::ConsistentProposal);
        assert_eq!(p.budget.initial_lots.len(), n);
        assert_eq!(r.initial_cash().unwrap().unassigned_cash, 50_000_000_000);
    }
}
#[test]
fn funding_actual_selector_and_each_anchor_mismatch() {
    let f = Fixture::new(0);
    for i in 0..8 {
        let mut p = f.proposal();
        let reason = match i {
            0 => {
                p.account_id.push('x');
                FundingMismatchV1::Account
            }
            1 => {
                p.epoch_id.push('x');
                FundingMismatchV1::Epoch
            }
            2 => {
                p.cutover_id.push('x');
                FundingMismatchV1::Cutover
            }
            3 => {
                p.genesis_manifest_hash = "b".repeat(64);
                FundingMismatchV1::GenesisManifest
            }
            4 => {
                p.genesis_event_hash = "b".repeat(64);
                FundingMismatchV1::GenesisEvent
            }
            5 => {
                p.genesis_projection_hash = "b".repeat(64);
                FundingMismatchV1::GenesisProjection
            }
            6 => {
                p.fee_policy_instance_id.push('x');
                FundingMismatchV1::FeePolicy
            }
            _ => {
                p.budget.original_seed_reference.push('x');
                FundingMismatchV1::SeedReference
            }
        };
        let r = f.review(&p).unwrap();
        assert_eq!(r.outcome(), Outcome::InconsistentProposal(reason));
        assert!(r.initial_cash().is_none());
        read_stored_funding_review(r.canonical_bytes()).unwrap();
    }
    assert_eq!(
        review_funding_proposal(
            &f.db,
            "TEST_CODE_missing",
            &serde_json::to_vec(&f.proposal()).unwrap()
        )
        .unwrap_err(),
        Error::MissingGenesis
    );
}
#[derive(QueryableByName)]
struct CountValue {
    #[diesel(sql_type=BigInt)]
    value: i64,
}
fn query_hits() -> [usize; 3] {
    HITS.with(|h| {
        let x = h.get();
        [
            x[Probe::MetadataQuery as usize],
            x[Probe::BindingQuery as usize],
            x[Probe::PayloadQuery as usize],
        ]
    })
}
fn schema_fingerprint(c: &mut SqliteConnection) -> String {
    diesel::sql_query("SELECT group_concat(type||':'||name||':'||sql,';') AS value FROM (SELECT type,name,sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY type,name)").get_result::<TextValue>(c).unwrap().value
}
fn corrupt_financial_fixture(f: &Fixture, case: usize) {
    let (trigger, mutation, evidence) = match case {
        0 => ("paper_book_owner_v2_no_delete", "DELETE FROM paper_book_owner_v2 WHERE account_id='TEST_CODE_FUNDING_ACCOUNT'", "SELECT COUNT(*) AS value FROM paper_book_owner_v2 WHERE account_id='TEST_CODE_FUNDING_ACCOUNT'"),
        1 => ("paper_book_owner_v2_transition", "UPDATE paper_book_owner_v2 SET active_epoch_id='TEST_CODE_WRONG_OWNER_EPOCH' WHERE account_id='TEST_CODE_FUNDING_ACCOUNT'", "SELECT COUNT(*) AS value FROM paper_book_owner_v2 WHERE account_id='TEST_CODE_FUNDING_ACCOUNT' AND active_epoch_id='TEST_CODE_WRONG_OWNER_EPOCH' AND active_generation=2 AND owner_revision=2"),
        2 => ("paper_book_owner_v2_no_reinsert", "INSERT INTO paper_book_owner_v2(account_id,active_generation,active_epoch_id,active_manifest_hash,owner_revision,cutover_id) VALUES('TEST_CODE_ORPHAN_ACCOUNT',1,'TEST_CODE_ORPHAN_EPOCH','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',1,NULL)", "SELECT COUNT(*) AS value FROM paper_book_owner_v2 o WHERE o.account_id='TEST_CODE_ORPHAN_ACCOUNT' AND NOT EXISTS(SELECT 1 FROM paper_ledger_account a WHERE a.account_id=o.account_id)"),
        3 => ("paper_ledger_event_no_update", "UPDATE paper_ledger_event SET payload='{}' WHERE account_id='TEST_CODE_FUNDING_ACCOUNT' AND seq=1", "SELECT COUNT(*) AS value FROM paper_ledger_event WHERE account_id='TEST_CODE_FUNDING_ACCOUNT' AND seq=1 AND typeof(payload)='text' AND payload='{}'"),
        _ => panic!("closed fixture mutation"),
    };
    let mut c = f.db.get_conn().unwrap();
    let before = schema_fingerprint(&mut c);
    let ddl =
        diesel::sql_query("SELECT sql AS value FROM sqlite_schema WHERE type='trigger' AND name=?")
            .bind::<Text, _>(trigger)
            .get_result::<TextValue>(&mut c)
            .unwrap()
            .value;
    c.batch_execute(&format!("DROP TRIGGER {trigger}")).unwrap();
    assert_eq!(diesel::sql_query(mutation).execute(&mut c).unwrap(), 1);
    c.batch_execute(&ddl).unwrap();
    assert_eq!(schema_fingerprint(&mut c), before);
    let changed = diesel::sql_query(evidence)
        .get_result::<CountValue>(&mut c)
        .unwrap()
        .value;
    assert_eq!(changed, if case == 0 { 0 } else { 1 });
}
#[test]
fn funding_actual_missing_wrong_owner_and_corrupt_history_refuse() {
    let f = Fixture::new(0);
    let p = f.proposal();
    reset();
    assert_eq!(
        review_funding_proposal(
            &f.db,
            "TEST_CODE_MISSING_ACCOUNT",
            &serde_json::to_vec(&p).unwrap()
        )
        .unwrap_err(),
        Error::MissingGenesis
    );
    assert_eq!(query_hits(), [1, 0, 0]);
    assert_eq!(HITS.with(|h| h.get())[Probe::Tail as usize], 0);
    assert_eq!(HITS.with(|h| h.get())[Probe::Observed as usize], 0);
    for case in 0..4 {
        let f = Fixture::new(0);
        let p = f.proposal();
        corrupt_financial_fixture(&f, case);
        reset();
        assert_eq!(
            f.review(&p).unwrap_err(),
            Error::CatalogUnavailable,
            "financial case {case}"
        );
        assert_eq!(query_hits(), [0, 0, 0], "financial case {case}");
        assert_eq!(HITS.with(|h| h.get())[Probe::Tail as usize], 0);
        assert_eq!(HITS.with(|h| h.get())[Probe::Observed as usize], 0);
    }
    // Retain the original catalog-only refusal controls, separately from data corruption.
    for sql in [
        "PRAGMA user_version=7",
        "CREATE TABLE TEST_CODE_unexpected(value TEXT)",
    ] {
        let f = Fixture::new(0);
        let p = f.proposal();
        f.db.get_conn().unwrap().batch_execute(sql).unwrap();
        reset();
        assert_eq!(f.review(&p).unwrap_err(), Error::CatalogUnavailable);
        assert_eq!(query_hits(), [0, 0, 0]);
        assert_eq!(HITS.with(|h| h.get())[Probe::Observed as usize], 0);
    }
}
#[test]
fn funding_allocation_completeness_disposition_and_cash_boundaries() {
    let f = Fixture::new(2);
    for i in 0..6 {
        let mut p = f.proposal();
        match i {
            0 => {
                p.budget.initial_lots.pop();
            }
            1 => p.budget.initial_lots[0].lot_id = "TEST_CODE_unknown".into(),
            2 => p.budget.initial_lots[0].original_quantity += 100,
            3 => p.budget.initial_strategy_cash_micro_cny = 100_000_000_001,
            4 => {
                p.budget.authorized_budget_micro_cny = 50_000_000_000;
                p.budget.max_order_exposure_micro_cny = 50_000_000_000;
            }
            _ => {
                p.budget.initial_lots[0].chain_id = None;
            }
        }
        let result = f.review(&p);
        if i == 5 {
            assert_eq!(result.unwrap_err(), Error::Schema)
        } else {
            assert_eq!(
                result.unwrap().outcome(),
                Outcome::InconsistentProposal(FundingMismatchV1::AllocationAgainstGenesis)
            );
        }
    }
    let mut p = f.proposal();
    p.budget.initial_lots.swap(0, 1);
    assert_eq!(f.review(&p).unwrap_err(), Error::Schema);
    let mut p = f.proposal();
    p.budget.initial_lots[1].lot_id = p.budget.initial_lots[0].lot_id.clone();
    assert_eq!(f.review(&p).unwrap_err(), Error::Schema);
}
#[test]
fn funding_policy_shape_and_checked_overflow_are_distinct() {
    let f = Fixture::new(1);
    let mut p = f.proposal();
    p.budget.concentration_bps = 10001;
    assert_eq!(f.review(&p).unwrap_err(), Error::Schema);
    let mut p = f.proposal();
    let mut g = f.inputs().1;
    p.budget.authorized_budget_micro_cny = i64::MAX;
    p.budget.max_order_exposure_micro_cny = i64::MAX;
    p.budget.initial_strategy_cash_micro_cny = 0;
    g.marks.values_mut().next().unwrap().price = Money::from_micros(i64::MAX);
    assert_eq!(
        allocation(&p, &g, &mut Work::new()).unwrap_err(),
        Error::ArithmeticOverflow
    );
    p.budget.effective_through = p.budget.effective_from.pred_opt().unwrap();
    assert_eq!(
        validate_proposal(&p, &mut Work::new()).unwrap_err(),
        Error::Schema
    );
}
#[test]
fn funding_actual_retained_reader_authority_tail_and_snapshot_semantics() {
    let f = Fixture::new(0);
    let p = f.proposal();
    reset();
    TAIL_FAIL.with(|v| v.set(true));
    assert_eq!(f.review(&p).unwrap_err(), Error::ChangedObservation);
    reset();
    let db = Arc::clone(&f.db);
    HOOK.with(|h|*h.borrow_mut()=Some(Box::new(move |_|{db.get_conn().unwrap().batch_execute("INSERT INTO stock_daily(code,date,open,high,low,close,volume) VALUES('TEST_CODE_funding_external','2026-09-28',10,10,10,10,100)").map_err(|_|Error::SqlRead)?;Ok(())})));
    let r = f.review(&p).unwrap();
    reset();
    assert_eq!(r.outcome(), Outcome::ConsistentProposal);
    assert_eq!(f.review(&p).unwrap().review_id(), r.review_id());
    let mut foreign = SqliteConnection::establish(":memory:").unwrap();
    investment_catalog8_session(&f.db)
        .unwrap()
        .with_readonly_catalog8(
            |_, proof| {
                reset();
                assert_eq!(
                    extract(&mut foreign, proof, ACCOUNT, &mut Work::new()).unwrap_err(),
                    Error::CatalogUnavailable
                );
                assert_eq!(HITS.with(|h| h.get())[0], 0);
                Ok::<_, Error>(())
            },
            |_, _, _| Ok(()),
        )
        .unwrap();
    f.db.get_conn()
        .unwrap()
        .batch_execute("CREATE TABLE TEST_CODE_after_review(value TEXT)")
        .unwrap();
    assert_eq!(f.review(&p).unwrap_err(), Error::CatalogUnavailable);
    read_stored_funding_review(r.canonical_bytes()).unwrap(); // historical bytes do not regain fresh authority
    let other = Fixture::new(0);
    assert_ne!(
        r.authority,
        other.review(&other.proposal()).unwrap().authority
    );
}
#[test]
fn funding_actual_oversized_or_wrong_sql_type_rejected() {
    for value in ["zeroblob(524289)", "'{}'"] {
        let f = Fixture::new(0);
        let p = f.proposal();
        let sql:String=diesel::sql_query("SELECT sql AS value FROM sqlite_schema WHERE type='trigger' AND tbl_name='paper_book_v2_head' AND sql LIKE '%BEFORE UPDATE%' LIMIT 1").get_result::<TextValue>(&mut f.db.get_conn().unwrap()).unwrap().value;
        let trigger = sql.split_whitespace().nth(2).unwrap();
        let mut c = f.db.get_conn().unwrap();
        c.batch_execute(&format!("DROP TRIGGER {trigger}; PRAGMA ignore_check_constraints=ON; UPDATE paper_book_v2_head SET projection_bytes={value}; PRAGMA ignore_check_constraints=OFF; {sql}")).unwrap();
        drop(c);
        reset();
        assert!(f.review(&p).is_err());
        // The actual Global gate rejects this damaged source before consumer SQL.
        assert_eq!(HITS.with(|h| h.get())[0], 0);
    }
}
#[test]
fn funding_metadata_and_closed_preflight_before_owned_entry() {
    for invalid in [-1, 1] {
        let m = Meta {
            owners: 1,
            accounts: 1,
            heads: 1,
            invalid,
            bytes: 1,
        };
        assert_eq!(checked_meta(&m), Err(Error::SqlShape));
    }
    for n in [-1, 0, 524289, i64::MAX] {
        assert_eq!(
            checked_meta(&Meta {
                owners: 1,
                accounts: 1,
                heads: 1,
                invalid: 0,
                bytes: n
            }),
            Err(Error::SqlShape)
        );
    }
    reset();
    for raw in [
        br#"{"cash":[[[0]]]}"#.as_slice(),
        br#"{"canonical_utf8":"fake"}"#.as_slice(),
    ] {
        assert!(codec::projection(raw, &mut Work::new()).is_err());
    }
    assert_eq!(HITS.with(|h| h.get())[1], 0);
}
// Independently generated fixed JSON/domain literals, unrelated to fixture identities.
const GOLD_PROPOSAL:&str="{\"schema\":\"paper-funding-proposal-v1\",\"version\":1,\"account_id\":\"TEST_CODE_GOLD_ACCOUNT\",\"epoch_id\":\"TEST_CODE_GOLD_V2\",\"cutover_id\":\"TEST_CODE_GOLD_CUTOVER\",\"genesis_manifest_hash\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"genesis_event_hash\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"genesis_projection_hash\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"fee_policy_instance_id\":\"TEST_CODE_GOLD_FEE\",\"budget\":{\"version\":\"paper-parent-budget/v1\",\"family_id\":\"TEST_CODE_GOLD_FAMILY\",\"effective_from\":\"2026-09-28\",\"effective_through\":\"2026-10-30\",\"authorized_budget_micro_cny\":1000,\"initial_strategy_cash_micro_cny\":500,\"concentration_bps\":10000,\"chain_exposure_bps\":10000,\"cash_floor_bps\":0,\"max_order_exposure_micro_cny\":1000,\"original_seed_reference\":\"paper-v1-manifest-sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"review_reference\":\"TEST_CODE_claim_only\",\"profit_policy\":\"ReinvestWithinFixedAuthorizedBudget\",\"initial_lots\":[]}}";
const GOLD_REVIEW:&str="{\"schema\":\"paper-funding-review-v1\",\"version\":1,\"proposal_id\":\"paper-funding-proposal-v1:6ef307a82c6ae24bcc9f85255cd19c58a626e30194f3b15092243c34c242c0d1\",\"proposal\":{\"schema\":\"paper-funding-proposal-v1\",\"version\":1,\"account_id\":\"TEST_CODE_GOLD_ACCOUNT\",\"epoch_id\":\"TEST_CODE_GOLD_V2\",\"cutover_id\":\"TEST_CODE_GOLD_CUTOVER\",\"genesis_manifest_hash\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"genesis_event_hash\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"genesis_projection_hash\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"fee_policy_instance_id\":\"TEST_CODE_GOLD_FEE\",\"budget\":{\"version\":\"paper-parent-budget/v1\",\"family_id\":\"TEST_CODE_GOLD_FAMILY\",\"effective_from\":\"2026-09-28\",\"effective_through\":\"2026-10-30\",\"authorized_budget_micro_cny\":1000,\"initial_strategy_cash_micro_cny\":500,\"concentration_bps\":10000,\"chain_exposure_bps\":10000,\"cash_floor_bps\":0,\"max_order_exposure_micro_cny\":1000,\"original_seed_reference\":\"paper-v1-manifest-sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"review_reference\":\"TEST_CODE_claim_only\",\"profit_policy\":\"ReinvestWithinFixedAuthorizedBudget\",\"initial_lots\":[]}},\"actual_binding\":{\"account_id\":\"TEST_CODE_GOLD_ACCOUNT\",\"epoch_id\":\"TEST_CODE_GOLD_V2\",\"cutover_id\":\"TEST_CODE_GOLD_CUTOVER\",\"genesis_manifest_hash\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"v1_epoch_id\":\"TEST_CODE_GOLD_V1\",\"v1_manifest_hash\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"v1_head_version\":1,\"v1_head_hash\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"v1_projection_hash\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"genesis_version\":1,\"genesis_event_hash\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"genesis_projection_hash\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"fee_policy_instance_id\":\"TEST_CODE_GOLD_FEE\"},\"outcome\":\"ConsistentProposal\",\"initial_cash\":{\"account_cash\":1000,\"strategy_cash\":500,\"unassigned_cash\":500},\"authority_state\":\"HistoricalObservationOnly\",\"approval_state\":\"NotIssued\"}";
const GOLD_PID: &str =
    "paper-funding-proposal-v1:6ef307a82c6ae24bcc9f85255cd19c58a626e30194f3b15092243c34c242c0d1";
const GOLD_RID: &str =
    "paper-funding-review-v1:0ce6f0c7ddc596b4deff2f5102ce03b847e6023b9e9a18a622f92c5c2ce62895";
#[test]
fn funding_proposal_and_review_literal_goldens_and_stored_roundtrip() {
    let p = codec::proposal(GOLD_PROPOSAL.as_bytes(), &mut Work::new()).unwrap();
    validate_proposal(&p, &mut Work::new()).unwrap();
    assert_eq!(
        codec::encode(&p, codec::PROPOSAL_LIMIT, &mut Work::new()).unwrap(),
        GOLD_PROPOSAL.as_bytes()
    );
    let r = read_stored_funding_review(GOLD_REVIEW.as_bytes()).unwrap();
    assert_eq!(r.review_id(), GOLD_RID);
    assert_eq!(r.proposal_id(), GOLD_PID);
    assert_eq!(r.outcome(), Outcome::ConsistentProposal);
}
#[test]
fn funding_each_policy_field_changes_domain_identity() {
    let f = Fixture::new(0);
    let base = f.review(&f.proposal()).unwrap();
    for i in 0..12 {
        let mut p = f.proposal();
        match i {
            0 => p.budget.family_id.push('x'),
            1 => p.budget.effective_from = p.budget.effective_from.succ_opt().unwrap(),
            2 => p.budget.effective_through = p.budget.effective_through.succ_opt().unwrap(),
            3 => p.budget.authorized_budget_micro_cny += 1,
            4 => p.budget.initial_strategy_cash_micro_cny -= 1,
            5 => p.budget.concentration_bps -= 1,
            6 => p.budget.chain_exposure_bps -= 1,
            7 => p.budget.cash_floor_bps += 1,
            8 => p.budget.max_order_exposure_micro_cny -= 1,
            9 => p.budget.original_seed_reference.push('x'),
            10 => p.budget.review_reference.push('x'),
            _ => p.budget.initial_lots.push(InitialLotAllocation {
                lot_id: "TEST_CODE_extra".into(),
                original_quantity: 100,
                disposition: LotDisposition::UnassignedReadOnly,
                chain_id: None,
            }),
        }
        let changed = f.review(&p).unwrap();
        assert_ne!(changed.proposal_id(), base.proposal_id());
        assert_ne!(changed.review_id(), base.review_id());
        read_stored_funding_review(changed.canonical_bytes()).unwrap();
    }
    // The closed policy version/sole profit enum cannot be replaced by a new policy.
    for (a, b) in [
        ("paper-parent-budget/v1", "paper-parent-budget/v2"),
        (
            "ReinvestWithinFixedAuthorizedBudget",
            "CallerInventedPolicy",
        ),
    ] {
        let raw = serde_json::to_string(&f.proposal()).unwrap().replace(a, b);
        assert_eq!(
            review_funding_proposal(&f.db, ACCOUNT, raw.as_bytes()).unwrap_err(),
            Error::Schema
        );
    }
}
#[test]
fn funding_wrong_path_escape_numeric_array_and_cardinality_attacks() {
    for (from, to) in [
        ("\"version\":1", "\"version\":[[[0]]]"),
        ("\"version\":1", "\"version\":18446744073709551616"),
        ("\"account_id\":", "\"body_hex\":"),
        (
            "\"chain_exposure_bps\":10000",
            "\"chain_exposure_bps\":null",
        ),
    ] {
        reset();
        let raw = GOLD_PROPOSAL.replacen(from, to, 1);
        assert!(codec::proposal(raw.as_bytes(), &mut Work::new()).is_err());
        assert_eq!(HITS.with(|h| h.get())[1], 0);
    }
    let raw = GOLD_PROPOSAL.replacen("TEST_CODE_GOLD_ACCOUNT", &"\\u0022".repeat(257), 1);
    reset();
    assert!(codec::proposal(raw.as_bytes(), &mut Work::new()).is_err());
    assert_eq!(HITS.with(|h| h.get())[1], 0);
    let f = Fixture::new(0);
    let mut p = f.proposal();
    for i in 0..257 {
        p.budget.initial_lots.push(InitialLotAllocation {
            lot_id: format!("TEST_CODE_{i:03}"),
            original_quantity: 100,
            disposition: LotDisposition::UnassignedReadOnly,
            chain_id: None,
        });
    }
    let raw = serde_json::to_vec(&p).unwrap();
    reset();
    assert!(codec::proposal(&raw, &mut Work::new()).is_err());
    assert_eq!(HITS.with(|h| h.get())[1], 0);
    p.budget.initial_lots.pop();
    assert!(codec::proposal(&serde_json::to_vec(&p).unwrap(), &mut Work::new()).is_ok());
}
#[test]
fn funding_shared_work_exact_boundary_sticky_failure_and_tail_no_refund() {
    for kind in [
        FixedQuery::Metadata,
        FixedQuery::Binding,
        FixedQuery::Payload,
    ] {
        let cost = kind.reservation().unwrap();
        for exact in [false, true] {
            let w = &mut Work::new();
            w.own(8 * codec::MIB - cost + usize::from(!exact)).unwrap();
            reset();
            let result = fixed_query(kind, w);
            assert_eq!(query_hits().iter().sum::<usize>(), usize::from(exact));
            if exact {
                drop(result.unwrap());
                assert_eq!(w.own(1), Err(Error::OwnedBudget));
            } else {
                assert_eq!(result.unwrap_err(), Error::OwnedBudget);
                assert!(w.scan(0).is_err());
            }
        }
        let w = &mut Work::new();
        w.scan(32 * codec::MIB - kind.sql().len() + 1).unwrap();
        reset();
        assert_eq!(fixed_query(kind, w).unwrap_err(), Error::WorkBudget);
        assert_eq!(query_hits(), [0, 0, 0]);
        assert!(w.own(0).is_err());
    }
    let f0 = Fixture::new(0);
    let p0 = f0.proposal();
    let g0 = f0.inputs().1;
    for exact in [false, true] {
        let w = &mut Work::new();
        w.own(8 * codec::MIB - LEGACY_ERROR_RESERVATION + usize::from(!exact))
            .unwrap();
        reset();
        let result = validate_proposal(&p0, w);
        assert_eq!(
            HITS.with(|h| h.get())[Probe::LegacyShape as usize],
            usize::from(exact)
        );
        if exact {
            result.unwrap();
            assert_eq!(w.own(1), Err(Error::OwnedBudget));
        } else {
            assert_eq!(result, Err(Error::OwnedBudget));
            assert!(w.scan(0).is_err());
        }
        let w = &mut Work::new();
        w.own(8 * codec::MIB - LEGACY_ERROR_RESERVATION + usize::from(!exact))
            .unwrap();
        reset();
        let result = allocation(&p0, &g0, w);
        assert_eq!(
            HITS.with(|h| h.get())[Probe::LegacyInitialCash as usize],
            usize::from(exact)
        );
        if exact {
            assert_eq!(result.unwrap().0, Outcome::ConsistentProposal);
            assert_eq!(w.own(1), Err(Error::OwnedBudget));
        } else {
            assert_eq!(result.unwrap_err(), Error::OwnedBudget);
            assert!(w.scan(0).is_err());
        }
    }
    for exact in [false, true] {
        let w = &mut Work::new();
        w.own(8 * codec::MIB - LEGACY_ERROR_RESERVATION + usize::from(!exact))
            .unwrap();
        reset();
        let result = validate_cash_partitions(
            &CashPartitions {
                account_cash: 1000,
                strategy_cash: 500,
                unassigned_cash: 500,
            },
            w,
        );
        assert_eq!(
            HITS.with(|h| h.get())[Probe::LegacyCashPartitions as usize],
            usize::from(exact)
        );
        if exact {
            result.unwrap();
            assert_eq!(w.own(1), Err(Error::OwnedBudget));
        } else {
            assert_eq!(result, Err(Error::OwnedBudget));
            assert!(w.scan(0).is_err());
        }
    }
    let mut invalid = f0.proposal();
    invalid.budget.concentration_bps = 10001;
    let w = &mut Work::new();
    reset();
    assert_eq!(validate_proposal(&invalid, w), Err(Error::Schema));
    assert_eq!(HITS.with(|h| h.get())[Probe::LegacyShape as usize], 1);
    assert!(w.scan(0).is_err()); // semantic legacy error poisons, not only resource failure

    let w = &mut Work::new();
    w.scan(32 * codec::MIB).unwrap();
    assert_eq!(w.scan(1), Err(Error::WorkBudget));
    assert!(w.own(0).is_err());
    let w = &mut Work::new();
    w.own(8 * codec::MIB).unwrap();
    assert_eq!(w.own(1), Err(Error::OwnedBudget));
    assert!(w.scan(0).is_err());
    let w = &mut Work::new();
    w.scan(32 * codec::MIB - 1).unwrap();
    reset();
    assert!(codec::proposal(GOLD_PROPOSAL.as_bytes(), w).is_err());
    assert_eq!(HITS.with(|h| h.get())[1], 0);
    assert!(w.scan(0).is_err());
    let f = Fixture::new(0);
    let proposal = f.proposal();
    reset();
    let observed = f.review(&proposal).unwrap();
    assert_eq!(observed.outcome(), Outcome::ConsistentProposal);
    assert_eq!(query_hits(), [2, 2, 2]);
    assert_eq!(HITS.with(|h| h.get())[Probe::Observed as usize], 1);
    // Positive proof, insufficient metadata reservation: no application constructor.
    investment_catalog8_session(&f.db)
        .unwrap()
        .with_readonly_catalog8(
            |c, p| {
                let w = &mut Work::new();
                w.own(8 * codec::MIB - FixedQuery::Metadata.reservation()? + 1)?;
                reset();
                assert_eq!(extract(c, p, ACCOUNT, w).unwrap_err(), Error::OwnedBudget);
                assert_eq!(query_hits(), [0, 0, 0]);
                assert!(w.scan(0).is_err());
                Ok::<_, Error>(())
            },
            |_, _, _| Ok(()),
        )
        .unwrap();
    // The retained-reader tail shares the first extraction's debits, without refund.
    let shared = RefCell::new(Work::new());
    reset();
    let result = investment_catalog8_session(&f.db)
        .unwrap()
        .with_readonly_catalog8(
            |c, p| {
                let w = &mut *shared.borrow_mut();
                let (_, raw) = extract(c, p, ACCOUNT, w)?;
                assert_eq!(query_hits(), [1, 1, 1]);
                let spent = [
                    FixedQuery::Metadata,
                    FixedQuery::Binding,
                    FixedQuery::Payload,
                ]
                .iter()
                .try_fold(4096 + std::mem::size_of::<Binding>() + raw.len(), |n, q| {
                    n.checked_add(q.reservation().ok()?)
                })
                .unwrap();
                let remaining = FixedQuery::Metadata.reservation()? - 1;
                w.own(8 * codec::MIB - spent - remaining)?;
                Ok::<_, Error>(())
            },
            |c, p, _| extract(c, p, ACCOUNT, &mut shared.borrow_mut()).map(|_| ()),
        );
    assert!(matches!(result,Err(crate::database::global_schema_v1::investment_v8::InvestmentCatalog8ReadbackError::Consumer(Error::OwnedBudget))));
    assert_eq!(query_hits(), [1, 1, 1]);
    assert!(shared.borrow_mut().scan(0).is_err());
    assert_eq!(HITS.with(|h| h.get())[Probe::Observed as usize], 0);
}
#[test]
fn funding_production_origin_refuses_before_consumer_sql() {
    DatabaseManager::init(None).unwrap();
    let db = DatabaseManager::get();
    assert!(!db.has_isolated_p05_consumer_origin());
    reset();
    assert_eq!(
        review_funding_proposal(db, "TEST_CODE_GOLD_ACCOUNT", GOLD_PROPOSAL.as_bytes())
            .unwrap_err(),
        Error::CatalogUnavailable
    );
    assert_eq!(HITS.with(|h| h.get())[0], 0);
}
#[test]
fn funding_stored_value_never_restores_observed_or_approval_authority() {
    let s = read_stored_funding_review(GOLD_REVIEW.as_bytes()).unwrap();
    assert_eq!(s.initial_cash().unwrap().strategy_cash, 500);
    assert!(crate::decision::approved_paper_intent_v1::require_production_approval().is_err());
    let changed = GOLD_REVIEW.replace("HistoricalObservationOnly", "Approved");
    assert!(read_stored_funding_review(changed.as_bytes()).is_err());
    let changed = GOLD_REVIEW.replace("NotIssued", "Approved");
    assert!(read_stored_funding_review(changed.as_bytes()).is_err());
}
