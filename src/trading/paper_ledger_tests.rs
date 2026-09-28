use super::*;
use crate::database::DatabaseManager;
use crate::trading::paper_trade::{Direction, PaperRiskContext, PaperSignal};
use chrono::{TimeZone, Utc};
use diesel::RunQueryDsl;

fn order(
    ledger: &PaperLedger<'_>,
    binding: &AccountBinding,
    id: &str,
    side: Direction,
    price: f64,
    at: chrono::DateTime<Utc>,
) -> ExecuteIntent {
    let view = ledger.read(binding).unwrap();
    ExecuteIntent {
        price_intent: PriceIntent::FixedSignalPriceV1,
        binding: binding.clone(),
        command_id: id.into(),
        expected_version: view.version,
        inventory_fingerprint: view.inventory_fingerprint().unwrap(),
        signal: PaperSignal {
            plan_id: format!("paper:{}:test:{id}", binding.epoch_id),
            code: "TEST_CODE_000001".into(),
            name: "fixture".into(),
            direction: side,
            price,
            quantity: 100,
            virtual_reason: "TEST_CODE_evidence".into(),
            is_limit_up: false,
            is_limit_down: false,
            is_suspended: false,
            limit_up_price: Some(price * 1.1),
            limit_down_price: Some(price * 0.9),
            secondary_confirmed: false,
            quote_observed_at: at,
            risk_context: PaperRiskContext::new(
                crate::risk::action_gate::AccountMode::Normal,
                crate::monitor::data_mode::DataMode::Full,
            ),
        },
        quote_price: Money::from_cny(price).unwrap(),
        marks: vec![Mark {
            code: "TEST_CODE_000001".into(),
            price: Money::from_cny(price).unwrap(),
            observed_at: at,
            source: "TEST_CODE_realtime".into(),
        }],
    }
}

fn mark_close(
    ledger: &PaperLedger<'_>,
    binding: &AccountBinding,
    id: &str,
    price: f64,
    at: chrono::DateTime<Utc>,
) {
    let view = ledger.read(binding).unwrap();
    ledger
        .apply(PaperCommand::Mark(ValuationBatch {
            binding: binding.clone(),
            command_id: id.into(),
            expected_version: view.version,
            inventory_fingerprint: view.inventory_fingerprint().unwrap(),
            as_of: at,
            closing: true,
            marks: vec![Mark {
                code: "TEST_CODE_000001".into(),
                price: Money::from_cny(price).unwrap(),
                observed_at: at,
                source: "TEST_CODE_validated_daily_close".into(),
            }],
        }))
        .unwrap();
}

fn instant() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 14, 2, 0, 0).unwrap()
}

#[test]
fn effective_fill_sell_candidate_cas_survives_same_quantity_correction() {
    use crate::trading::paper_ledger_runtime::{execute_checked_on, InventoryCheckpoint};
    let dir = tempfile::tempdir().unwrap();
    let db =
        DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_candidate.db")).unwrap();
    declare_test_catalog_v2(&db);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    let ledger = PaperLedger::open(&db, &instant);
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let buy = ledger
        .apply(PaperCommand::Execute(order(
            &ledger,
            &binding,
            "buy",
            Direction::Buy,
            10.0,
            instant(),
        )))
        .unwrap();
    let before = ledger.read(&binding).unwrap();
    let checkpoint = InventoryCheckpoint {
        binding: binding.clone(),
        version: before.version,
        event_hash: before.event_hash.clone(),
        inventory_fingerprint: before.inventory_fingerprint().unwrap(),
    };
    let later = || instant() + chrono::Duration::days(1);
    let current = PaperLedger::open(&db, &later);
    let mut signal = order(&current, &binding, "sell", Direction::Sell, 12.0, later()).signal;
    signal.plan_id = "TEST_CODE_candidate_sell".into();
    let mut ruling = ruling_for(
        &current,
        &binding,
        buy.paper_trade_id.unwrap(),
        "correct-before-submit",
    );
    ruling.action = AdjudicationAction::CorrectionDeclared {
        price: Money::from_cny(9.0).unwrap(),
        quantity: 100,
        fact_at: instant(),
    };
    current.adjudicate(ruling).unwrap();
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let quotes = |_: &str| {
        calls.fetch_add(1, Ordering::SeqCst);
        Err("TEST_CODE external quote must not run".into())
    };
    let quote = crate::broker::ExecutionQuote {
        price: 12.0,
        limit_up_price: 13.2,
        limit_down_price: 10.8,
        observed_at: later(),
    };
    let result = execute_checked_on(
        &db,
        &binding,
        &signal,
        &quote,
        &later,
        &quotes,
        &AtomicBool::new(false),
        Some(&checkpoint),
    );
    assert!(
        result
            .as_ref()
            .is_err_and(|e| e.contains("version changed")),
        "{result:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(current.read(&binding).unwrap().version, 3);
}

#[test]
fn effective_fill_economic_consumer_freezes_position_and_fee_together() {
    let dir = tempfile::tempdir().unwrap();
    let db =
        DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_consumer.db")).unwrap();
    declare_test_catalog_v2(&db);
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let mut intent = order(&ledger, &binding, "buy", Direction::Buy, 10.0, instant());
    intent.signal.virtual_reason = "Momentum: TEST_CODE_evidence".into();
    let fill = ledger.apply(PaperCommand::Execute(intent)).unwrap();
    let request = EffectiveFillRequest {
        scope: EffectiveFillScope::Epoch(binding.clone()),
        history: EffectiveHistory::RestatedLatest,
        as_of: instant().date_naive(),
    };
    let frozen = ledger.verified_effective_fills(&request).unwrap();
    ledger
        .adjudicate(ruling_for(
            &ledger,
            &binding,
            fill.paper_trade_id.unwrap(),
            "exclude",
        ))
        .unwrap();
    let report = crate::performance::economic_position::report_from_effective(&frozen).unwrap();
    assert_eq!(report.open_positions.len(), 1);
    assert_eq!(frozen.costs().unwrap().costs[0].adverse_cost, 5.0);
    let current = ledger.verified_effective_fills(&request).unwrap();
    let restated = crate::performance::economic_position::report_from_effective(&current).unwrap();
    assert!(restated.open_positions.is_empty());
    assert!(current.costs().unwrap().costs.is_empty());
    assert_ne!(
        current.receipt().projection_hash,
        frozen.receipt().projection_hash
    );
}

#[test]
fn effective_fill_r12_uses_original_id_and_corrected_economic_version() {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_r12.db")).unwrap();
    declare_test_catalog_v2(&db);
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let mut intent = order(&ledger, &binding, "buy", Direction::Buy, 10.0, instant());
    intent.signal.virtual_reason = "Momentum: TEST_CODE_evidence".into();
    let fill = ledger.apply(PaperCommand::Execute(intent)).unwrap();
    let request = EffectiveFillRequest {
        scope: EffectiveFillScope::Epoch(binding.clone()),
        history: EffectiveHistory::RestatedLatest,
        as_of: instant().date_naive(),
    };
    let mut ruling = ruling_for(&ledger, &binding, fill.paper_trade_id.unwrap(), "correct");
    ruling.action = AdjudicationAction::CorrectionDeclared {
        price: Money::from_cny(9.0).unwrap(),
        quantity: 100,
        fact_at: instant(),
    };
    ledger.adjudicate(ruling).unwrap();
    let frozen = ledger.verified_effective_fills(&request).unwrap();
    let (entries, sells) = crate::review::backtest::entries_from_effective(&frozen, 30).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(sells, 0);
    assert_eq!(entries[0].id, fill.paper_trade_id.unwrap());
    assert_eq!(entries[0].fill_price, 9.0);
    assert_eq!(
        entries[0].ts_utc,
        instant().naive_utc(),
        "effective local economic fact must align to the real UTC market-bar anchor"
    );
}

#[test]
fn effective_fill_attribution_restates_economics_not_raw_terminal_authority() {
    use crate::performance::attribution_replay::{
        compute_effective_attribution, MinuteLabelSemantics,
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_attribution.db");
    let db = DatabaseManager::open_isolated_for_test(path).unwrap();
    declare_test_catalog_v2(&db);
    let now = std::sync::atomic::AtomicI64::new(instant().timestamp());
    let clock = || Utc.timestamp_opt(now.load(Ordering::SeqCst), 0).unwrap();
    let ledger = PaperLedger::open(&db, &clock);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let mut buy = order(&ledger, &binding, "buy", Direction::Buy, 10.0, clock());
    buy.signal.virtual_reason = "Momentum: TEST_CODE_evidence".into();
    let original = ledger.apply(PaperCommand::Execute(buy)).unwrap();
    now.store(instant().timestamp() + 86400, Ordering::SeqCst);
    ledger
        .apply(PaperCommand::Execute(order(
            &ledger,
            &binding,
            "sell",
            Direction::Sell,
            12.0,
            clock(),
        )))
        .unwrap();
    let request = EffectiveFillRequest {
        scope: EffectiveFillScope::Epoch(binding.clone()),
        history: EffectiveHistory::RestatedLatest,
        as_of: clock().date_naive(),
    };
    let frozen = ledger.verified_effective_fills(&request).unwrap();
    let semantics = MinuteLabelSemantics::EndLabelVerified {
        evidence_hash: "a".repeat(64),
    };
    let first =
        compute_effective_attribution(&frozen, instant().date_naive(), &[], &semantics).unwrap();
    assert_eq!(first.cycles()[0].scenario_net_pnl, 188.8);
    let bytes = first.canonical_bytes().unwrap();
    let mut ruling = ruling_for(
        &ledger,
        &binding,
        original.paper_trade_id.unwrap(),
        "correct",
    );
    ruling.action = AdjudicationAction::CorrectionDeclared {
        price: Money::from_cny(9.0).unwrap(),
        quantity: 100,
        fact_at: instant(),
    };
    ledger.adjudicate(ruling).unwrap();
    let current = ledger.verified_effective_fills(&request).unwrap();
    let restated =
        compute_effective_attribution(&current, instant().date_naive(), &[], &semantics).unwrap();
    assert_eq!(restated.cycles()[0].scenario_net_pnl, 288.8);
    assert_ne!(restated.result_hash(), first.result_hash());
    assert_eq!(
        bytes,
        compute_effective_attribution(&frozen, instant().date_naive(), &[], &semantics)
            .unwrap()
            .canonical_bytes()
            .unwrap()
    );
    assert!(
        matches!(&current.lineage()[0].authority,FillAuthority::EpochTerminal{audit_hash,..} if Some(audit_hash)==original.audit.as_ref().map(|a|&a.record_hash))
    );
}

#[test]
fn effective_fill_attribution_raw_loader_requires_explicit_paper_scope() {
    use crate::performance::attribution_replay::{
        AttributionReplayLoader, AttributionReplayRequest,
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_raw_scope.db");
    let db = DatabaseManager::open_isolated_for_test(path.clone()).unwrap();
    declare_test_catalog_v2(&db);
    let ledger = PaperLedger::open(&db, &instant);
    ledger.apply(PaperCommand::Seed(manifest())).unwrap();
    let date = instant().date_naive();
    let result = AttributionReplayLoader::new(path).load(&AttributionReplayRequest {
        from: date,
        to: date,
        required_trading_dates: vec![date],
        fee_ledger: None,
    });
    assert!(matches!(result,Err(crate::performance::attribution_replay::AttributionReplayError::Unavailable {code:crate::performance::attribution_replay::AttributionUnavailable::PaperScopeRequired,..})),"{result:?}");
    let reconstructed = crate::database::attribution_epochs::reconstruct_epoch_daily(
        &db,
        date - chrono::Duration::days(3),
        date,
        &std::collections::HashMap::new(),
    );
    assert!(
        matches!(
            reconstructed,
            Err(
                crate::database::attribution_epochs::AttributionEpochStoreError::Unavailable {
                    reason_code: "paper_scope_required",
                    ..
                }
            )
        ),
        "raw reconstruction bypassed explicit paper scope: {reconstructed:?}"
    );
}

#[test]
fn effective_fill_live_sell_inventory_carries_the_frozen_projection_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_live_receipt.db"))
        .unwrap();
    declare_test_catalog_v2(&db);
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let fill = ledger
        .apply(PaperCommand::Execute(order(
            &ledger,
            &binding,
            "buy",
            Direction::Buy,
            10.0,
            instant(),
        )))
        .unwrap();
    let date = day(instant()) + chrono::Duration::days(1);
    let set = ledger
        .verified_effective_fills(&EffectiveFillRequest {
            scope: EffectiveFillScope::Epoch(binding.clone()),
            history: EffectiveHistory::RestatedLatest,
            as_of: date,
        })
        .unwrap();
    let positions =
        crate::trading::paper_ledger_runtime::sellable_positions_on(&db, &binding, date).unwrap();
    assert_eq!(positions[0].quantity, 100);
    assert_eq!(positions[0].buy_fee_cost, 5.0);
    assert!(
        positions[0]
            .inventory_audit_evidence
            .contains(&set.receipt().projection_hash),
        "actual sell owner did not bind its effective projection"
    );
    let mut ruling = ruling_for(&ledger, &binding, fill.paper_trade_id.unwrap(), "correct");
    ruling.action = AdjudicationAction::CorrectionDeclared {
        price: Money::from_cny(9.0).unwrap(),
        quantity: 100,
        fact_at: instant(),
    };
    ledger.adjudicate(ruling).unwrap();
    let changed =
        crate::trading::paper_ledger_runtime::sellable_positions_on(&db, &binding, date).unwrap();
    assert_eq!(changed[0].avg_buy_price, 9.0);
    assert_ne!(positions[0].checkpoint, changed[0].checkpoint);
}

#[test]
fn effective_fill_rejects_temp_namespace_and_pre_cutover_projection() {
    let dir = tempfile::tempdir().unwrap();
    let db =
        DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_namespace_scope.db"))
            .unwrap();
    declare_test_catalog_v2(&db);
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let request = EffectiveFillRequest {
        scope: EffectiveFillScope::Epoch(binding),
        history: EffectiveHistory::RestatedLatest,
        as_of: day(instant()),
    };
    let mut conn = db.get_conn().unwrap();
    conn.batch_execute("CREATE TEMP TABLE paper_ledger_shadow(payload TEXT)")
        .unwrap();
    let result = conn.transaction(|conn| verified_effective_fills_on(conn, &request));
    assert!(
        matches!(result, Err(LedgerError::IntegrityFailure(_))),
        "TEMP paper namespace cannot bypass catalog proof: {result:?}"
    );
    conn.batch_execute("DROP TABLE temp.paper_ledger_shadow")
        .unwrap();
    let result = ledger.verified_effective_fills(&EffectiveFillRequest {
        as_of: day(instant()) - chrono::Duration::days(1),
        ..request
    });
    assert!(
        matches!(result, Err(LedgerError::InvalidInput(_))),
        "seed cannot be projected before it exists: {result:?}"
    );
}

#[test]
fn effective_fill_online_window_uses_explicit_aligned_epoch_and_restatement_owner() {
    use crate::database::attribution_epochs::{AttributionEpochStore, EpochActivationRequest};
    use crate::performance::attribution_epoch::EpochActivationSource;
    use crate::performance::attribution_replay::commit_effective_window;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_online_window.db");
    let db = DatabaseManager::open_isolated_for_test(path.clone()).unwrap();
    declare_test_catalog_v2(&db);
    let manager = &db;
    AttributionEpochStore::new(&manager)
        .activate_once(EpochActivationRequest {
            source: EpochActivationSource::Cli,
            invoked_at: chrono::DateTime::parse_from_rfc3339("2026-09-11T15:40:00+08:00").unwrap(),
        })
        .unwrap();
    let now = std::sync::atomic::AtomicI64::new(instant().timestamp());
    let clock = || Utc.timestamp_opt(now.load(Ordering::SeqCst), 0).unwrap();
    let ledger = PaperLedger::open(&db, &clock);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let mut buy_intent = order(&ledger, &binding, "buy", Direction::Buy, 10.0, clock());
    buy_intent.signal.virtual_reason = "Momentum: TEST_CODE_evidence".into();
    let buy = ledger.apply(PaperCommand::Execute(buy_intent)).unwrap();
    now.store(instant().timestamp() + 86400, Ordering::SeqCst);
    ledger
        .apply(PaperCommand::Execute(order(
            &ledger,
            &binding,
            "sell",
            Direction::Sell,
            12.0,
            clock(),
        )))
        .unwrap();
    let invoked = (clock() + chrono::Duration::hours(7))
        .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap());
    let (first, original) =
        commit_effective_window(&manager, binding.clone(), day(clock()), 30, invoked).unwrap();
    assert_eq!(first.report().cycles()[0].scenario_net_pnl, 188.8);
    let mut ruling = ruling_for(&ledger, &binding, buy.paper_trade_id.unwrap(), "correct");
    ruling.action = AdjudicationAction::CorrectionDeclared {
        price: Money::from_cny(9.0).unwrap(),
        quantity: 100,
        fact_at: instant(),
    };
    ledger.adjudicate(ruling).unwrap();
    let (restated, revised) =
        commit_effective_window(&manager, binding.clone(), day(clock()), 30, invoked).unwrap();
    assert_eq!(restated.report().cycles()[0].scenario_net_pnl, 288.8);
    assert_ne!(original.report_identity, revised.report_identity);
    assert_eq!(
        commit_effective_window(&manager, binding, day(clock()), 30, invoked)
            .unwrap()
            .1
            .report_revision_id,
        revised.report_revision_id
    );
}

#[test]
fn effective_fill_attribution_append_owner_keeps_old_revision_and_reuses_same_projection() {
    use crate::performance::attribution_epoch::AttributionEpochSelector;
    use crate::performance::attribution_replay::{
        AttributionReplayLoader, AttributionReplayRunner, ReplayMode, ReplayRequest,
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_effective_reports.db");
    let db = DatabaseManager::open_isolated_for_test(path.clone()).unwrap();
    declare_test_catalog_v2(&db);
    let now = std::sync::atomic::AtomicI64::new(instant().timestamp());
    let clock = || Utc.timestamp_opt(now.load(Ordering::SeqCst), 0).unwrap();
    let ledger = PaperLedger::open(&db, &clock);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let mut buy = order(&ledger, &binding, "buy", Direction::Buy, 10.0, clock());
    buy.signal.virtual_reason = "Momentum: TEST_CODE_evidence".into();
    let original = ledger.apply(PaperCommand::Execute(buy)).unwrap();
    now.store(instant().timestamp() + 86400, Ordering::SeqCst);
    ledger
        .apply(PaperCommand::Execute(order(
            &ledger,
            &binding,
            "sell",
            Direction::Sell,
            12.0,
            clock(),
        )))
        .unwrap();
    let manager = crate::database::attribution_reports::test_runner_database_manager(&path);
    let runner = AttributionReplayRunner::new(&manager, AttributionReplayLoader::new(&path));
    let request = ReplayRequest {
        mode: ReplayMode::Range {
            from: instant().date_naive(),
            to: clock().date_naive(),
            invoked_at: (clock() + chrono::Duration::hours(7))
                .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap()),
        },
        epoch: AttributionEpochSelector::Legacy,
        benchmark_day_manifests: Vec::new(),
    };
    let paper = EffectiveFillRequest {
        scope: EffectiveFillScope::Epoch(binding.clone()),
        history: EffectiveHistory::RestatedLatest,
        as_of: clock().date_naive(),
    };
    let preview = runner
        .preview_effective(request.clone(), paper.clone())
        .unwrap();
    assert_eq!(preview.report().cycles()[0].scenario_net_pnl, 188.8);
    let (first, receipt) = runner
        .commit_effective(request.clone(), paper.clone())
        .unwrap();
    let old = first.report().canonical_bytes().unwrap();
    ledger.settle_snapshot(&paper).unwrap();
    let (_, repeated) = runner
        .commit_effective(request.clone(), paper.clone())
        .unwrap();
    assert_eq!(receipt.report_identity, repeated.report_identity);
    assert_eq!(receipt.report_revision_id, repeated.report_revision_id);
    let mut ruling = ruling_for(
        &ledger,
        &binding,
        original.paper_trade_id.unwrap(),
        "correct",
    );
    ruling.action = AdjudicationAction::CorrectionDeclared {
        price: Money::from_cny(9.0).unwrap(),
        quantity: 100,
        fact_at: instant(),
    };
    ledger.adjudicate(ruling).unwrap();
    let (restated, revised) = runner
        .commit_effective(request.clone(), paper.clone())
        .unwrap();
    assert_eq!(restated.report().cycles()[0].scenario_net_pnl, 288.8);
    assert_ne!(revised.report_identity, receipt.report_identity);
    assert_eq!(
        revised.predecessor_report_id,
        Some(receipt.report_revision_id)
    );
    let frozen = EffectiveFillRequest {
        history: EffectiveHistory::AsKnown {
            ledger_version: Some(3),
        },
        ..paper
    };
    let historical = runner.preview_effective(request, frozen).unwrap();
    assert_eq!(historical.report().cycles()[0].scenario_net_pnl, 188.8);
    assert_eq!(first.report().canonical_bytes().unwrap(), old);
    let mut conn = manager.get_conn().unwrap();
    let stored = diesel::sql_query(
        "SELECT result_payload_json AS bytes FROM attribution_report_revision WHERE id=?",
    )
    .bind::<BigInt, _>(receipt.report_revision_id)
    .get_result::<adjudication::RawBytes>(&mut conn)
    .unwrap();
    assert!(stored.bytes.contains("188.8"));
    assert!(!stored.bytes.contains("288.8"));
}

#[test]
fn effective_fill_p04_never_reissues_a_corrected_or_quarantined_original_card() {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_p04.db")).unwrap();
    declare_test_catalog_v2(&db);
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let fill = ledger
        .apply(PaperCommand::Execute(order(
            &ledger,
            &binding,
            "buy",
            Direction::Buy,
            10.0,
            instant(),
        )))
        .unwrap();
    let id = fill.paper_trade_id.unwrap();
    let hash = fill.audit.unwrap().record_hash;
    let request = EffectiveFillRequest {
        scope: EffectiveFillScope::Epoch(binding.clone()),
        history: EffectiveHistory::RestatedLatest,
        as_of: instant().date_naive(),
    };
    let before = ledger.verified_effective_fills(&request).unwrap();
    assert!(before.original_fill_card_allowed(id, &hash).unwrap());
    let mut correction = ruling_for(&ledger, &binding, id, "correction");
    correction.action = AdjudicationAction::CorrectionDeclared {
        price: Money::from_cny(9.0).unwrap(),
        quantity: 100,
        fact_at: instant(),
    };
    let corrected = ledger.adjudicate(correction).unwrap();
    let current = ledger.verified_effective_fills(&request).unwrap();
    assert!(!current.original_fill_card_allowed(id, &hash).unwrap());
    assert!(current
        .original_fill_card_allowed(id, &"f".repeat(64))
        .is_err());
    let mut quarantine = ruling_for(&ledger, &binding, id, "quarantine");
    quarantine.expected_predecessor = Some(corrected.event_hash);
    ledger.adjudicate(quarantine).unwrap();
    assert!(!ledger
        .verified_effective_fills(&request)
        .unwrap()
        .original_fill_card_allowed(id, &hash)
        .unwrap());
}

#[test]
fn effective_fill_opening_inventory_mixed_sell_splits_fifo_and_fee_without_fake_buy() {
    let dir = tempfile::tempdir().unwrap();
    let db =
        DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_opening.db")).unwrap();
    declare_test_catalog_v2(&db);
    let now = std::sync::atomic::AtomicI64::new(instant().timestamp());
    let clock = || Utc.timestamp_opt(now.load(Ordering::SeqCst), 0).unwrap();
    let ledger = PaperLedger::open(&db, &clock);
    let mut seed = manifest();
    seed.original_total = Money::from_cny(100800.0).unwrap();
    seed.lots = vec![SeedLot {
        code: "TEST_CODE_000001".into(),
        name: "opening".into(),
        quantity: 100,
        reported_cost: None,
        sellable_from: Some(instant().date_naive()),
        sellability_evidence: Some("TEST_CODE confirmed available".into()),
    }];
    seed.marks = vec![Mark {
        code: "TEST_CODE_000001".into(),
        price: Money::from_cny(8.0).unwrap(),
        observed_at: instant(),
        source: "TEST_CODE seed basis".into(),
    }];
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let mut buy = order(&ledger, &binding, "buy", Direction::Buy, 10.0, clock());
    buy.signal.virtual_reason = "Momentum: TEST_CODE_evidence".into();
    let bought = ledger.apply(PaperCommand::Execute(buy)).unwrap();
    now.store(instant().timestamp() + 86400, Ordering::SeqCst);
    let request = EffectiveFillRequest {
        scope: EffectiveFillScope::Epoch(binding.clone()),
        history: EffectiveHistory::RestatedLatest,
        as_of: clock().date_naive(),
    };
    let inventory = crate::trading::paper_sell::positions_from_effective(
        &ledger.verified_effective_fills(&request).unwrap(),
    )
    .unwrap();
    let mut sell = order(&ledger, &binding, "sell", Direction::Sell, 12.0, clock());
    sell.signal.quantity = 200;
    let sold = ledger.apply(PaperCommand::Execute(sell)).unwrap();
    let set = ledger.verified_effective_fills(&request).unwrap();
    let report = crate::performance::economic_position::report_from_effective(&set).unwrap();
    assert_eq!(
        report.source_fill_ids,
        vec![bought.paper_trade_id.unwrap(), sold.paper_trade_id.unwrap()]
    );
    assert_eq!(report.closed_positions.len(), 1);
    assert!(
        matches!(report.closed_positions[0].net,crate::performance::economic_position::NetMetrics::Available{net_pnl,..} if (net_pnl-191.3).abs()<1e-8)
    );
    assert_eq!(
        set.rows().unwrap().len(),
        2,
        "seed must not be forged as a buy"
    );
    let opening = report.opening_inventory.as_ref().unwrap();
    assert_eq!(opening.excluded_exits[0].opening_quantity, 100);
    assert_eq!(opening.excluded_exits[0].strategy_quantity, 100);
    assert_eq!(
        opening.excluded_exits[0].opening_net_pnl,
        Money::from_cny(396.3).unwrap()
    );
    assert_eq!(
        ledger.read(&binding).unwrap().realized_pnl,
        Money::from_cny(587.6).unwrap()
    );
    let attribution = crate::performance::attribution_replay::compute_effective_attribution(
        &set,
        instant().date_naive(),
        &[],
        &crate::performance::attribution_replay::MinuteLabelSemantics::Unverified,
    )
    .unwrap();
    let payload: serde_json::Value =
        serde_json::from_slice(&attribution.canonical_bytes().unwrap()).unwrap();
    let snapshot = ledger.settle_snapshot(&request).unwrap();
    let actual = (
        inventory[0].quantity,
        inventory[0].avg_buy_price,
        snapshot.metrics.total_pnl,
        payload["opening_inventory"]["excluded_exits"][0]["opening_quantity"].as_u64(),
    );
    assert_eq!(
        actual,
        (200, 9.0, 191.3, Some(100)),
        "PaperSell, strategy snapshot and attribution must preserve opening component lineage"
    );
}

#[test]
fn effective_fill_snapshot_is_append_only_stable_after_its_own_result_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_snapshot.db");
    let db = DatabaseManager::open_isolated_for_test(path.clone()).unwrap();
    declare_test_catalog_v2(&db);
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    // Sentinel legacy material. The new owner must not touch it or depend on its layout.
    let mut conn = db.get_conn().unwrap();
    diesel::sql_query(
        "CREATE TABLE paper_performance_snapshot(date TEXT PRIMARY KEY, frozen TEXT NOT NULL)",
    )
    .execute(&mut conn)
    .unwrap();
    diesel::sql_query(
        "INSERT INTO paper_performance_snapshot VALUES('2026-09-14','TEST_CODE_frozen_bytes')",
    )
    .execute(&mut conn)
    .unwrap();
    drop(conn);
    let request = EffectiveFillRequest {
        scope: EffectiveFillScope::Epoch(binding.clone()),
        history: EffectiveHistory::RestatedLatest,
        as_of: instant().date_naive(),
    };
    let before = ledger.read(&binding).unwrap();
    let first = ledger.settle_snapshot(&request).unwrap();
    assert_eq!(first.metrics.total_trades, 0);
    assert_eq!(ledger.read(&binding).unwrap().projection, before.projection);
    let after_version = ledger.read(&binding).unwrap().version;
    let second = ledger.settle_snapshot(&request).unwrap();
    assert_eq!(encode(&first).unwrap(), encode(&second).unwrap());
    assert!(ledger.current_snapshot(&request).unwrap().is_some());
    assert!(
        ledger
            .current_snapshot(&EffectiveFillRequest {
                as_of: request.as_of + chrono::Duration::days(1),
                ..request.clone()
            })
            .unwrap()
            .is_none(),
        "old business day is not current merely because it was just written"
    );
    assert_eq!(ledger.read(&binding).unwrap().version, after_version);
    let reopened = DatabaseManager::open_isolated_for_test(path).unwrap();
    assert_eq!(
        encode(
            &PaperLedger::open(&reopened, &instant)
                .settle_snapshot(&request)
                .unwrap()
        )
        .unwrap(),
        encode(&first).unwrap()
    );
    let mut conn = reopened.get_conn().unwrap();
    assert_eq!(
        diesel::sql_query("SELECT frozen AS bytes FROM paper_performance_snapshot")
            .get_result::<adjudication::RawBytes>(&mut conn)
            .unwrap()
            .bytes,
        "TEST_CODE_frozen_bytes"
    );
}

#[test]
fn effective_fill_snapshot_period_ignores_future_orders_but_invalidates_relevant_late_rulings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_snapshot_period.db");
    let db = DatabaseManager::open_isolated_for_test(path.clone()).unwrap();
    declare_test_catalog_v2(&db);
    let ledger = PaperLedger::open(&db, &instant);
    let binding = manifest().binding().unwrap();
    ledger.apply(PaperCommand::Seed(manifest())).unwrap();
    let first_fill = ledger
        .apply(PaperCommand::Execute(order(
            &ledger,
            &binding,
            "day-one",
            Direction::Buy,
            10.0,
            instant(),
        )))
        .unwrap();
    let request = EffectiveFillRequest {
        scope: EffectiveFillScope::Epoch(binding.clone()),
        history: EffectiveHistory::RestatedLatest,
        as_of: day(instant()),
    };
    let first = ledger.settle_snapshot(&request).unwrap();
    let old_bytes = encode(&first).unwrap();
    let tomorrow = || instant() + chrono::Duration::days(1);
    let next = PaperLedger::open(&db, &tomorrow);
    let future_fill = next
        .apply(PaperCommand::Execute(order(
            &next,
            &binding,
            "day-two",
            Direction::Buy,
            10.0,
            tomorrow(),
        )))
        .unwrap();
    assert_eq!(
        next.current_snapshot(&request)
            .unwrap()
            .map(|r| encode(&r).unwrap()),
        Some(old_bytes.clone()),
        "next-day order must not make yesterday's snapshot unhealthy"
    );
    let version = next.read(&binding).unwrap().version;
    assert_eq!(
        encode(&next.settle_snapshot(&request).unwrap()).unwrap(),
        old_bytes
    );
    assert_eq!(next.read(&binding).unwrap().version, version);
    // Even a late ruling is irrelevant when both the original and its corrected fact are after D.
    let mut future_ruling = ruling_for(
        &next,
        &binding,
        future_fill.paper_trade_id.unwrap(),
        "future-price",
    );
    future_ruling.decision_at = tomorrow();
    future_ruling.action = AdjudicationAction::CorrectionDeclared {
        price: Money::from_cny(11.0).unwrap(),
        quantity: 100,
        fact_at: tomorrow(),
    };
    next.adjudicate(future_ruling).unwrap();
    assert_eq!(
        encode(&next.current_snapshot(&request).unwrap().unwrap()).unwrap(),
        old_bytes
    );
    let mut relevant = ruling_for(
        &next,
        &binding,
        first_fill.paper_trade_id.unwrap(),
        "yesterday-price",
    );
    relevant.decision_at = tomorrow();
    relevant.action = AdjudicationAction::CorrectionDeclared {
        price: Money::from_cny(9.0).unwrap(),
        quantity: 100,
        fact_at: instant(),
    };
    next.adjudicate(relevant).unwrap();
    assert!(next.current_snapshot(&request).unwrap().is_none());
    let revised = next.settle_snapshot(&request).unwrap();
    assert_ne!(revised.result_hash, first.result_hash);
    assert_eq!(
        encode(
            &next
                .current_snapshot(&EffectiveFillRequest {
                    history: EffectiveHistory::AsKnown {
                        ledger_version: Some(first.projection.ledger_head.unwrap().0),
                    },
                    ..request.clone()
                })
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        old_bytes,
        "old revision remains frozen and verifiable"
    );
    let reopened = DatabaseManager::open_isolated_for_test(path).unwrap();
    let reopened_ledger = PaperLedger::open(&reopened, &tomorrow);
    assert_eq!(
        encode(&reopened_ledger.settle_snapshot(&request).unwrap()).unwrap(),
        encode(&revised).unwrap()
    );
    // Scope-specific cache identity must not bypass full-source integrity validation.
    diesel::sql_query("UPDATE paper_trades SET fill_price=99 WHERE id=?")
        .bind::<BigInt, _>(future_fill.paper_trade_id.unwrap())
        .execute(&mut reopened.get_conn().unwrap())
        .unwrap();
    assert!(matches!(
        reopened_ledger.current_snapshot(&request),
        Err(LedgerError::IntegrityFailure(_))
    ));
}

fn manifest() -> SeedManifest {
    SeedManifest {
        account_id: "TEST_CODE_ACCOUNT".into(),
        epoch_id: "TEST_CODE_EPOCH_V1".into(),
        command_id: "seed-v1".into(),
        cutover_at: instant(),
        account_effective_at: instant(),
        positions_effective_at: instant(),
        source_reference: "TEST_CODE_confirmed_snapshot".into(),
        source_hash: "a".repeat(64),
        approved_by: "TEST_CODE_explicit_approval".into(),
        cash: Money::from_cny(100_000.0).unwrap(),
        original_total: Money::from_cny(100_000.0).unwrap(),
        excluded_residual: None,
        lots: vec![],
        marks: vec![],
        policy: RiskPolicyV1::default(),
    }
}

#[test]
fn effective_fill_quarantine_reverses_economics_without_rewriting_raw_or_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let db =
        DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_ruling.db")).unwrap();
    declare_test_catalog_v2(&db);
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let buy = order(
        &ledger,
        &binding,
        "bad-buy",
        Direction::Buy,
        10.0,
        instant(),
    );
    let fill = ledger.apply(PaperCommand::Execute(buy.clone())).unwrap();
    let before = ledger.read(&binding).unwrap();
    let request = Adjudication {
        binding: binding.clone(),
        request_id: "ruling-1".into(),
        expected_version: before.version,
        expected_head: before.event_hash.clone(),
        expected_predecessor: None,
        original: ledger
            .fill_fingerprint(&binding, fill.paper_trade_id.unwrap())
            .unwrap(),
        action: AdjudicationAction::Quarantine,
        reason: "TEST_CODE_bad_source".into(),
        evidence: "TEST_CODE_receipt".into(),
        operator: "TEST_CODE_operator".into(),
        source: "TEST_CODE_explicit_preview".into(),
        decision_at: instant(),
    };
    let ruling = ledger.adjudicate(request.clone()).unwrap();
    let after = ledger.read(&binding).unwrap();
    assert_eq!(after.cash, Money::from_cny(100_000.0).unwrap());
    assert_eq!(after.fees, Money::ZERO);
    assert!(after.lots.is_empty());
    assert!(ledger.effective_fills(&binding).unwrap().is_empty());
    assert_eq!(
        ledger.adjudicate(request).unwrap().event_hash,
        ruling.event_hash
    );
    assert_eq!(
        ledger.apply(PaperCommand::Execute(buy)).unwrap().event_hash,
        fill.event_hash
    );
    let mut conn = db.get_conn().unwrap();
    let count = diesel::sql_query("SELECT COUNT(*) AS value FROM paper_trades WHERE status='Filled' AND fill_price=10 AND quantity=100").get_result::<IntegerRow>(&mut conn).unwrap();
    assert_eq!(count.value, 1);
}

#[test]
fn effective_fill_rejects_original_row_changed_before_first_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_tamper_source.db"))
        .unwrap();
    declare_test_catalog_v2(&db);
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let fill = ledger
        .apply(PaperCommand::Execute(order(
            &ledger,
            &binding,
            "original",
            Direction::Buy,
            10.0,
            instant(),
        )))
        .unwrap();
    let mut conn = db.get_conn().unwrap();
    diesel::sql_query("UPDATE paper_trades SET fill_price=11 WHERE id=?")
        .bind::<BigInt, _>(fill.paper_trade_id.unwrap())
        .execute(&mut conn)
        .unwrap();
    assert!(matches!(ledger.fill_fingerprint(&binding, fill.paper_trade_id.unwrap()), Err(LedgerError::IntegrityFailure(_))), "a newly captured fingerprint must not bless a raw fill contradicting its immutable event/audit");
    diesel::sql_query("UPDATE paper_trades SET fill_price=10,virtual_reason='TEST' WHERE id=?")
        .bind::<BigInt, _>(fill.paper_trade_id.unwrap())
        .execute(&mut conn)
        .unwrap();
    assert!(
        matches!(
            ledger.fill_fingerprint(&binding, fill.paper_trade_id.unwrap()),
            Err(LedgerError::IntegrityFailure(_))
        ),
        "shortening a reason must not acquire a new source fingerprint"
    );
}

#[test]
fn effective_fill_ruling_failure_after_writes_rolls_back_and_request_payload_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_atomic_ruling.db"))
        .unwrap();
    declare_test_catalog_v2(&db);
    let mut ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let fill = ledger
        .apply(PaperCommand::Execute(order(
            &ledger,
            &binding,
            "buy",
            Direction::Buy,
            10.0,
            instant(),
        )))
        .unwrap();
    let request = ruling_for(
        &ledger,
        &binding,
        fill.paper_trade_id.unwrap(),
        "quarantine",
    );
    let before = ledger.read(&binding).unwrap();
    let preview = ledger.preview_adjudication(&request).unwrap();
    assert_eq!(
        preview.current_account.cash,
        Money::from_cny(100000.0).unwrap()
    );
    assert!(preview.current_account.changed);
    assert!(preview.historical_scope.is_none());
    assert_eq!(ledger.read(&binding).unwrap(), before);
    ledger.before_commit_fault = Some(&|| true);
    assert!(
        ledger.adjudicate(request.clone()).is_err(),
        "injected post-write failure must not commit"
    );
    ledger.before_commit_fault = None;
    assert_eq!(ledger.read(&binding).unwrap(), before);
    let receipt = ledger.adjudicate(request.clone()).unwrap();
    assert_eq!(receipt.version, 3);
    let mut changed = request.clone();
    changed.reason.push_str(" changed");
    assert!(matches!(
        ledger.adjudicate(changed),
        Err(LedgerError::IdentityConflict)
    ));
    assert_eq!(
        ledger.adjudicate(request).unwrap().event_hash,
        receipt.event_hash
    );
}

#[test]
fn effective_fill_two_real_connections_ruling_predecessor_has_one_winner() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_ruling_race.db");
    let db1 = DatabaseManager::open_isolated_for_test(path.clone()).unwrap();
    declare_test_catalog_v2(&db1);
    let db2 = DatabaseManager::open_isolated_for_test(path).unwrap();
    let ledger = PaperLedger::open(&db1, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let fill = ledger
        .apply(PaperCommand::Execute(order(
            &ledger,
            &binding,
            "buy",
            Direction::Buy,
            10.0,
            instant(),
        )))
        .unwrap();
    let first = ruling_for(&ledger, &binding, fill.paper_trade_id.unwrap(), "first");
    let mut second = first.clone();
    second.request_id = "second".into();
    let barrier = std::sync::Barrier::new(2);
    let results = std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            barrier.wait();
            PaperLedger::open(&db1, &instant).adjudicate(first)
        });
        let b = scope.spawn(|| {
            barrier.wait();
            PaperLedger::open(&db2, &instant).adjudicate(second)
        });
        [a.join().unwrap(), b.join().unwrap()]
    });
    assert_eq!(
        results.iter().filter(|r| r.is_ok()).count(),
        1,
        "{results:?}"
    );
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(LedgerError::VersionChanged)))
            .count(),
        1,
        "{results:?}"
    );
    assert_eq!(ledger.read(&binding).unwrap().version, 3);
    assert_eq!(
        ledger.read(&binding).unwrap().cash,
        Money::from_cny(100000.0).unwrap()
    );
}

#[test]
fn effective_fill_correction_partial_sell_unavailable_then_restore_and_stale_cas() {
    use std::sync::atomic::AtomicI64;
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_correction.db"))
        .unwrap();
    declare_test_catalog_v2(&db);
    let now = AtomicI64::new(instant().timestamp());
    let clock = || Utc.timestamp_opt(now.load(Ordering::SeqCst), 0).unwrap();
    let ledger = PaperLedger::open(&db, &clock);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let mut buy = order(&ledger, &binding, "buy", Direction::Buy, 10.0, clock());
    buy.signal.quantity = 200;
    let bought = ledger.apply(PaperCommand::Execute(buy.clone())).unwrap();
    now.store(instant().timestamp() + 86400, Ordering::SeqCst);
    ledger
        .apply(PaperCommand::Execute(order(
            &ledger,
            &binding,
            "sell",
            Direction::Sell,
            12.0,
            clock(),
        )))
        .unwrap();
    let before = ledger.read(&binding).unwrap();
    let stale = order(
        &ledger,
        &binding,
        "stale-sell",
        Direction::Sell,
        12.0,
        clock(),
    );
    let mut ruling = Adjudication {
        binding: binding.clone(),
        request_id: "quarantine".into(),
        expected_version: before.version,
        expected_head: before.event_hash.clone(),
        expected_predecessor: None,
        original: ledger
            .fill_fingerprint(&binding, bought.paper_trade_id.unwrap())
            .unwrap(),
        action: AdjudicationAction::Quarantine,
        reason: "TEST_CODE_disputed".into(),
        evidence: "TEST_CODE_evidence".into(),
        operator: "TEST_CODE_operator".into(),
        source: "TEST_CODE_operator_manifest".into(),
        decision_at: clock(),
    };
    let quarantined = ledger.adjudicate(ruling.clone()).unwrap();
    assert!(quarantined
        .reason
        .as_ref()
        .is_some_and(|r| r.contains("FIFO/T+1")));
    assert!(matches!(
        ledger.read(&binding),
        Err(LedgerError::EvidenceUnavailable(_))
    ));
    assert!(ledger.effective_fills(&binding).is_err());
    assert_eq!(
        ledger
            .recover_terminal(&binding, &buy.signal, buy.price_intent)
            .unwrap()
            .unwrap()
            .event_hash,
        bought.event_hash
    );
    ruling.request_id = "correction".into();
    ruling.expected_version = quarantined.version;
    ruling.expected_head = quarantined.event_hash.clone();
    ruling.expected_predecessor = Some(quarantined.event_hash);
    ruling.action = AdjudicationAction::CorrectionDeclared {
        price: Money::from_cny(9.0).unwrap(),
        quantity: 200,
        fact_at: instant(),
    };
    let restored = ledger.adjudicate(ruling.clone()).unwrap();
    assert!(restored.reason.is_none());
    let after = ledger.read(&binding).unwrap();
    assert_eq!(after.cash, Money::from_cny(99_388.80).unwrap());
    assert_eq!(after.fees, Money::from_cny(11.20).unwrap());
    assert_eq!(after.realized_pnl, Money::from_cny(291.30).unwrap());
    assert_eq!(after.lots[0].quantity, 100);
    assert_eq!(after.lots[0].name, "fixture");
    assert_eq!(
        after.lots[0].buy_fee_remaining,
        Money::from_cny(2.50).unwrap()
    );
    assert!(matches!(
        ledger.apply(PaperCommand::Execute(stale)),
        Err(LedgerError::VersionChanged)
    ));
    assert_eq!(
        ledger.adjudicate(ruling).unwrap().event_hash,
        restored.event_hash
    );
    assert_eq!(
        ledger.read_at_version(&binding, before.version).unwrap(),
        before
    );
}

fn ruling_for(
    ledger: &PaperLedger<'_>,
    binding: &AccountBinding,
    fill_id: i64,
    request_id: &str,
) -> Adjudication {
    let view = ledger.read(binding).unwrap();
    Adjudication {
        binding: binding.clone(),
        request_id: request_id.into(),
        expected_version: view.version,
        expected_head: view.event_hash,
        expected_predecessor: None,
        original: ledger.fill_fingerprint(binding, fill_id).unwrap(),
        action: AdjudicationAction::Quarantine,
        reason: "TEST_CODE_reason".into(),
        evidence: "TEST_CODE_evidence".into(),
        operator: "TEST_CODE_operator".into(),
        source: "TEST_CODE_manifest".into(),
        decision_at: instant(),
    }
}

#[test]
fn effective_fill_verified_set_freezes_rows_fees_lineage_and_as_known_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_set.db");
    let db = DatabaseManager::open_isolated_for_test(path.clone()).unwrap();
    let ledger = PaperLedger::open(&db, &instant);
    declare_test_catalog_v2(&db);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let fill = ledger
        .apply(PaperCommand::Execute(order(
            &ledger,
            &binding,
            "buy",
            Direction::Buy,
            10.0,
            instant(),
        )))
        .unwrap();
    let request = EffectiveFillRequest {
        scope: EffectiveFillScope::Epoch(binding.clone()),
        history: EffectiveHistory::AsKnown {
            ledger_version: Some(fill.version),
        },
        as_of: day(instant()),
    };
    let frozen = ledger.verified_effective_fills(&request).unwrap();
    assert_eq!(frozen.rows().unwrap().len(), 1);
    assert_eq!(frozen.costs().unwrap().costs[0].adverse_cost, 5.0);
    let ruled = ledger
        .adjudicate(ruling_for(
            &ledger,
            &binding,
            fill.paper_trade_id.unwrap(),
            "quarantine",
        ))
        .unwrap();
    let current = ledger
        .verified_effective_fills(&EffectiveFillRequest {
            history: EffectiveHistory::RestatedLatest,
            ..request.clone()
        })
        .unwrap();
    assert!(current.rows().unwrap().is_empty());
    assert!(current.costs().unwrap().costs.is_empty());
    assert_eq!(current.lineage()[0].ruling_hash, Some(ruled.event_hash));
    assert!(current.lineage()[0].quarantined);
    assert_ne!(
        current.receipt().projection_hash,
        frozen.receipt().projection_hash
    );
    assert_eq!(frozen.rows().unwrap().len(), 1);
    let reopened = DatabaseManager::open_isolated_for_test(path).unwrap();
    assert_eq!(
        PaperLedger::open(&reopened, &instant)
            .verified_effective_fills(&request)
            .unwrap(),
        frozen
    );
}

#[test]
fn effective_fill_adjudication_commit_unknown_recovers_same_request() {
    let dir = tempfile::tempdir().unwrap();
    let db =
        DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_unknown.db")).unwrap();
    declare_test_catalog_v2(&db);
    let mut ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let fill = ledger
        .apply(PaperCommand::Execute(order(
            &ledger,
            &binding,
            "buy",
            Direction::Buy,
            10.0,
            instant(),
        )))
        .unwrap();
    let request = ruling_for(
        &ledger,
        &binding,
        fill.paper_trade_id.unwrap(),
        "quarantine",
    );
    let fail_once = AtomicBool::new(true);
    let fault = || fail_once.swap(false, Ordering::SeqCst);
    ledger.after_commit_fault = Some(&fault);
    assert!(matches!(
        ledger.adjudicate(request.clone()),
        Err(LedgerError::CommitOutcomeUnknown)
    ));
    let recovered = ledger.adjudicate(request).unwrap();
    assert!(recovered.already_applied);
    assert_eq!(recovered.version, 3);
}

fn declare_test_catalog_v2(db: &DatabaseManager) {
    db.get_conn()
        .unwrap()
        .batch_execute("PRAGMA application_id=1398035265; PRAGMA user_version=2")
        .unwrap();
}

#[test]
fn task8_catalog_v3_preserves_paper_projection_and_adjudication() {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_catalog_v3.db"))
        .unwrap();
    declare_test_catalog_v2(&db);
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let fill = ledger
        .apply(PaperCommand::Execute(order(
            &ledger,
            &binding,
            "buy",
            Direction::Buy,
            10.0,
            instant(),
        )))
        .unwrap();
    let request = EffectiveFillRequest {
        scope: EffectiveFillScope::Epoch(binding.clone()),
        history: EffectiveHistory::RestatedLatest,
        as_of: day(instant()),
    };
    let before = ledger.verified_effective_fills(&request).unwrap();

    {
        let mut conn = db.get_conn().unwrap();
        crate::database::daily_change_review_schema_v1::create_schema(&mut conn).unwrap();
        conn.batch_execute("PRAGMA user_version=3").unwrap();
    }

    let after_migration = ledger
        .verified_effective_fills(&request)
        .expect("CatalogV3 is a strict extension of the qualified paper catalog");
    assert_eq!(after_migration.rows().unwrap(), before.rows().unwrap());
    assert_eq!(after_migration.lineage(), before.lineage());

    ledger
        .adjudicate(ruling_for(
            &ledger,
            &binding,
            fill.paper_trade_id.unwrap(),
            "catalog-v3-quarantine",
        ))
        .expect("paper adjudication remains available after the V2 to V3 migration");
    assert!(ledger
        .verified_effective_fills(&request)
        .unwrap()
        .rows()
        .unwrap()
        .is_empty());
}

#[test]
fn task8_catalog_v3_requires_the_exact_review_namespace_for_paper_economics() {
    for tamper in [None, Some("DROP TRIGGER daily_change_review_no_delete")] {
        let dir = tempfile::tempdir().unwrap();
        let db = DatabaseManager::open_isolated_for_test(
            dir.path().join("TEST_CODE_catalog_v3_incomplete.db"),
        )
        .unwrap();
        declare_test_catalog_v2(&db);
        let ledger = PaperLedger::open(&db, &instant);
        let seed = manifest();
        let binding = seed.binding().unwrap();
        ledger.apply(PaperCommand::Seed(seed)).unwrap();

        {
            let mut conn = db.get_conn().unwrap();
            if let Some(tamper) = tamper {
                crate::database::daily_change_review_schema_v1::create_schema(&mut conn).unwrap();
                conn.batch_execute(tamper).unwrap();
            }
            conn.batch_execute("PRAGMA user_version=3").unwrap();
        }

        let result = ledger.verified_effective_fills(&EffectiveFillRequest {
            scope: EffectiveFillScope::Epoch(binding),
            history: EffectiveHistory::RestatedLatest,
            as_of: day(instant()),
        });
        assert!(
            matches!(result, Err(LedgerError::IntegrityFailure(_))),
            "CatalogV3 paper economics accepted an absent/tampered review namespace: {result:?}"
        );
    }
}
fn legacy_buy(db: &DatabaseManager) -> i64 {
    let mut conn = db.get_conn().unwrap();
    diesel::sql_query("INSERT INTO paper_trades(plan_id,code,name,direction,price,quantity,status,fill_price,virtual_reason,account_mode,data_mode,ts,updated_at) VALUES ('TEST_CODE_legacy','TEST_CODE_000001','fixture','buy',10,100,'Filled',10,'TEST_CODE_legacy','Normal','Full','2026-07-10 10:00:00','2026-07-10 10:00:00')").execute(&mut conn).unwrap();
    diesel::sql_query("SELECT last_insert_rowid() AS value")
        .get_result::<IntegerRow>(&mut conn)
        .unwrap()
        .value
}

#[test]
fn effective_fill_legacy_raw_is_explicit_as_known_and_cannot_authorize_adjudication() {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_legacy_read.db"))
        .unwrap();
    declare_test_catalog_v2(&db);
    let id = legacy_buy(&db);
    let ledger = PaperLedger::open(&db, &instant);
    let request = EffectiveFillRequest {
        scope: EffectiveFillScope::LegacyRaw,
        history: EffectiveHistory::AsKnown {
            ledger_version: None,
        },
        as_of: day(instant()),
    };
    let source = ledger.verified_effective_fills(&request).unwrap();
    assert_eq!(source.rows().unwrap()[0].id, id);
    assert_eq!(
        source.lineage()[0].authority,
        FillAuthority::LegacyNoTerminal
    );
    assert!(source.receipt().ledger_head.is_none());
    assert!(ledger
        .verified_effective_fills(&EffectiveFillRequest {
            history: EffectiveHistory::RestatedLatest,
            ..request.clone()
        })
        .is_err());
    assert!(matches!(
        ledger.fill_fingerprint(&manifest().binding().unwrap(), id),
        Err(LedgerError::NotSeeded)
    ));
    ledger.apply(PaperCommand::Seed(manifest())).unwrap();
    assert!(
        ledger.verified_effective_fills(&request).is_err(),
        "once seeded, callers must choose a bound scope"
    );
}

#[test]
fn effective_fill_legacy_preview_separates_history_from_current_account_and_never_writes() {
    for quarantine in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TEST_CODE_legacy_preview.db");
        let db = DatabaseManager::open_isolated_for_test(path.clone()).unwrap();
        declare_test_catalog_v2(&db);
        let id = legacy_buy(&db);
        diesel::sql_query("INSERT INTO paper_trades(plan_id,code,name,direction,price,quantity,status,fill_price,virtual_reason,account_mode,data_mode,ts,updated_at) VALUES ('TEST_CODE_legacy_sell','TEST_CODE_000001','fixture','sell',11,100,'Filled',11,'TEST_CODE_legacy','Normal','Full','2026-07-13 02:00:00','2026-07-13 02:00:00')")
            .execute(&mut db.get_conn().unwrap()).unwrap();
        let ledger = PaperLedger::open(&db, &instant);
        let binding = manifest().binding().unwrap();
        ledger.apply(PaperCommand::Seed(manifest())).unwrap();
        let before = ledger.read(&binding).unwrap();
        let mut request = ruling_for(&ledger, &binding, id, "historical-preview");
        if !quarantine {
            request.action = AdjudicationAction::CorrectionDeclared {
                price: Money::from_cny(9.0).unwrap(),
                quantity: 100,
                fact_at: request.original.fact_at,
            };
        }
        let preview = ledger.preview_adjudication(&request).unwrap();
        let value = serde_json::to_value(&preview).unwrap();
        assert!(
            value["historical_scope"].is_object(),
            "historical preview must not report only current cash: {value}"
        );
        let historical = &value["historical_scope"];
        assert_eq!(
            historical["scope"],
            serde_json::to_value(EffectiveFillScope::LegacyBeforeCutover(binding.clone())).unwrap()
        );
        assert_ne!(historical["before_hash"], historical["after_hash"]);
        assert_eq!(historical["unavailable"].is_string(), quarantine);
        assert_eq!(value["current_account"]["changed"], false);
        assert_eq!(value["current_account"]["cash"], 100_000_000_000_i64);
        assert!(value["current_account"]["unavailable"].is_null());
        assert_eq!(ledger.preview_adjudication(&request).unwrap(), preview);
        assert_eq!(
            ledger.read(&binding).unwrap(),
            before,
            "preview is zero-write"
        );
        let reopened = DatabaseManager::open_isolated_for_test(path.clone()).unwrap();
        let reopened_ledger = PaperLedger::open(&reopened, &instant);
        assert_eq!(
            reopened_ledger.preview_adjudication(&request).unwrap(),
            preview
        );
        let receipt = reopened_ledger.adjudicate(request.clone()).unwrap();
        assert_eq!(
            serde_json::to_value(&receipt.reason).unwrap(),
            historical["unavailable"]
        );
        assert_eq!(
            reopened_ledger.read(&binding).unwrap().projection,
            before.projection
        );
        let history_request = EffectiveFillRequest {
            scope: EffectiveFillScope::LegacyBeforeCutover(binding.clone()),
            history: EffectiveHistory::RestatedLatest,
            as_of: day(instant()),
        };
        let effective = reopened_ledger
            .verified_effective_fills(&history_request)
            .unwrap();
        if quarantine {
            assert!(
                matches!(effective.rows(), Err(LedgerError::EvidenceUnavailable(reason)) if Some(reason.as_str()) == receipt.reason.as_deref())
            );
        } else {
            assert_eq!(effective.rows().unwrap()[0].fill_price, Some(9.0));
        }
        let reopened_again = DatabaseManager::open_isolated_for_test(path).unwrap();
        let ledger_again = PaperLedger::open(&reopened_again, &instant);
        assert_eq!(
            ledger_again
                .verified_effective_fills(&history_request)
                .unwrap(),
            effective
        );
        assert_eq!(
            ledger_again.adjudicate(request).unwrap().reason,
            receipt.reason
        );
    }
}

#[test]
fn effective_fill_legacy_correction_is_historical_only_and_does_not_reseed_cash() {
    let dir = tempfile::tempdir().unwrap();
    let db =
        DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_legacy_correction.db"))
            .unwrap();
    declare_test_catalog_v2(&db);
    let id = legacy_buy(&db);
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    let seeded = ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let request = EffectiveFillRequest {
        scope: EffectiveFillScope::LegacyBeforeCutover(binding.clone()),
        history: EffectiveHistory::AsKnown {
            ledger_version: Some(seeded.version),
        },
        as_of: day(instant()),
    };
    let old = ledger.verified_effective_fills(&request).unwrap();
    assert_eq!(old.rows().unwrap()[0].fill_price, Some(10.0));
    let mut ruling = ruling_for(&ledger, &binding, id, "historical-correction");
    ruling.action = AdjudicationAction::CorrectionDeclared {
        price: Money::from_cny(9.0).unwrap(),
        quantity: 100,
        fact_at: ruling.original.fact_at,
    };
    ledger.adjudicate(ruling).unwrap();
    let current = ledger
        .verified_effective_fills(&EffectiveFillRequest {
            history: EffectiveHistory::RestatedLatest,
            ..request.clone()
        })
        .unwrap();
    assert_eq!(current.rows().unwrap()[0].fill_price, Some(9.0));
    assert_eq!(
        current.lineage()[0].authority,
        FillAuthority::LegacyNoTerminal
    );
    assert_eq!(
        current.rows().unwrap()[0].occurred_at,
        old.rows().unwrap()[0].occurred_at,
        "price-only ruling must not shift legacy fact time"
    );
    assert_eq!(ledger.verified_effective_fills(&request).unwrap(), old);
    let account = ledger.read(&binding).unwrap();
    assert_eq!(account.cash, Money::from_cny(100000.0).unwrap());
    assert_eq!(account.fees, Money::ZERO);
    assert!(account.lots.is_empty());
}

#[test]
fn effective_fill_namespace_missing_tampered_extra_and_unknown_generation_fail_closed() {
    for mutation in [
        "PRAGMA user_version=999",
        "DROP TRIGGER paper_ledger_event_no_delete",
        "CREATE TABLE paper_ledger_shadow(payload TEXT)",
        "DROP INDEX paper_ledger_terminal_plan; CREATE INDEX paper_ledger_terminal_plan ON paper_ledger_event(account_id)",
    ] {
        let dir=tempfile::tempdir().unwrap();let db=DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_namespace.db")).unwrap();
        declare_test_catalog_v2(&db);let ledger=PaperLedger::open(&db,&instant);let seed=manifest();let binding=seed.binding().unwrap();ledger.apply(PaperCommand::Seed(seed)).unwrap();
        db.get_conn().unwrap().batch_execute(mutation).unwrap();
        assert!(matches!(ledger.verified_effective_fills(&EffectiveFillRequest{scope:EffectiveFillScope::Epoch(binding),history:EffectiveHistory::RestatedLatest,as_of:day(instant())}),Err(LedgerError::IntegrityFailure(_))),"accepted schema mutation: {mutation}");
    }
}

#[test]
fn effective_fill_declared_generation_never_falls_back_to_implicit_raw_economics() {
    use crate::performance::attribution_replay::{
        AttributionReplayLoader, AttributionReplayRequest,
    };
    for generation in [2, 999] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TEST_CODE_raw_generation.db");
        let db = DatabaseManager::open_isolated_for_test(path.clone()).unwrap();
        db.get_conn()
            .unwrap()
            .batch_execute(&format!(
                "PRAGMA user_version={generation}; PRAGMA application_id=1398035265"
            ))
            .unwrap();
        let date = day(instant());
        let raw = AttributionReplayLoader::new(path).load(&AttributionReplayRequest {
            from: date,
            to: date,
            required_trading_dates: vec![date],
            fee_ledger: None,
        });
        assert!(
            raw.is_err(),
            "declared generation {generation} with no account fell back to raw: {raw:?}"
        );
        assert!(
            crate::database::attribution_epochs::reconstruct_epoch_daily(
                &db,
                date - chrono::Duration::days(3),
                date,
                &std::collections::HashMap::new()
            )
            .is_err(),
            "reconstruction ignored generation {generation}"
        );
    }
}

#[test]
fn effective_fill_original_missing_chain_truncated_and_unknown_payload_fail_closed() {
    for mutation in [
        "PRAGMA foreign_keys=OFF; DELETE FROM paper_trades WHERE id=(SELECT MAX(id) FROM paper_trades)",
        "DROP TRIGGER paper_ledger_event_no_delete; DELETE FROM paper_ledger_event WHERE seq=3",
        "DROP TRIGGER paper_ledger_event_no_update; UPDATE paper_ledger_event SET payload='{}' WHERE seq=3",
    ] {
        let dir=tempfile::tempdir().unwrap();let db=DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_source_tamper.db")).unwrap();declare_test_catalog_v2(&db);
        let ledger=PaperLedger::open(&db,&instant);let seed=manifest();let binding=seed.binding().unwrap();ledger.apply(PaperCommand::Seed(seed)).unwrap();
        let fill=ledger.apply(PaperCommand::Execute(order(&ledger,&binding,"buy",Direction::Buy,10.0,instant()))).unwrap();
        ledger.adjudicate(ruling_for(&ledger,&binding,fill.paper_trade_id.unwrap(),"quarantine")).unwrap();
        let mut conn=db.get_conn().unwrap();conn.batch_execute(mutation).unwrap();
        // Restore the frozen trigger when this mutation removed it, so the
        // behavior probes the chain/payload rather than only the catalog gate.
        for (kind,name,_,sql) in crate::database::paper_ledger_schema_v1::STATEMENTS {
            if *kind=="trigger" && mutation.contains(name) {conn.batch_execute(sql).unwrap();}
        }
        let result=conn.transaction(|conn|verified_effective_fills_on(conn,&EffectiveFillRequest{scope:EffectiveFillScope::Epoch(binding.clone()),history:EffectiveHistory::RestatedLatest,as_of:day(instant())}));
        assert!(result.is_err(),"accepted source mutation: {mutation}");
    }
}

#[test]
fn effective_fill_correction_cannot_invent_a_weekend_execution() {
    let dir = tempfile::tempdir().unwrap();
    let db =
        DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_weekend.db")).unwrap();
    declare_test_catalog_v2(&db);
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let fill = ledger
        .apply(PaperCommand::Execute(order(
            &ledger,
            &binding,
            "buy",
            Direction::Buy,
            10.0,
            instant(),
        )))
        .unwrap();
    let later = || instant() + chrono::Duration::days(6);
    let current = PaperLedger::open(&db, &later);
    let mut ruling = ruling_for(&current, &binding, fill.paper_trade_id.unwrap(), "weekend");
    ruling.decision_at = later();
    ruling.action = AdjudicationAction::CorrectionDeclared {
        price: Money::from_cny(9.0).unwrap(),
        quantity: 100,
        fact_at: later(),
    };
    assert!(
        matches!(
            current.adjudicate(ruling),
            Err(LedgerError::InvalidInput(_))
        ),
        "weekend cannot become a corrected execution date"
    );
    assert_eq!(current.read(&binding).unwrap().version, 2);
}

#[test]
fn paper_ledger_seed_is_once_and_ignores_later_account_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_paper_seed.db");
    let seed = manifest();
    let binding = seed.binding().unwrap();
    {
        let db = DatabaseManager::open_isolated_for_test(path.clone()).unwrap();
        let ledger = PaperLedger::open(&db, &instant);
        assert!(matches!(ledger.read(&binding), Err(LedgerError::NotSeeded)));
        let first = ledger.apply(PaperCommand::Seed(seed.clone())).unwrap();
        assert!(!first.already_applied);
        let repeat = ledger.apply(PaperCommand::Seed(seed.clone())).unwrap();
        assert!(repeat.already_applied);
        assert_eq!(repeat.event_hash, first.event_hash);
        assert_eq!(repeat.version, 1);
        let mut changed = seed.clone();
        changed.cash = Money::from_cny(200_000.0).unwrap();
        assert!(matches!(
            ledger.apply(PaperCommand::Seed(changed)),
            Err(LedgerError::IdentityConflict)
        ));
        let mut connection = db.get_conn().unwrap();
        diesel::sql_query("INSERT INTO ledger(date,total_value,cash,market_value) VALUES ('2026-09-14',900000,900000,0)")
            .execute(&mut connection).unwrap();
        let view = ledger.read(&binding).unwrap();
        assert_eq!(view.cash, Money::from_cny(100_000.0).unwrap());
        assert_eq!(view.equity().unwrap(), Money::from_cny(100_000.0).unwrap());
        assert_eq!(view.fees, Money::ZERO);
        assert!(view.daily_pnl().is_none());
    }
    let reopened = DatabaseManager::open_isolated_for_test(path).unwrap();
    let view = PaperLedger::open(&reopened, &instant)
        .read(&binding)
        .unwrap();
    assert_eq!(view.cash, Money::from_cny(100_000.0).unwrap());
    assert_eq!(view.version, 1);
}

#[test]
fn paper_ledger_two_buys_partial_sell_and_next_day_mark() {
    use std::sync::atomic::{AtomicI64, Ordering};
    let dir = tempfile::tempdir().unwrap();
    let db =
        DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_paper_lifecycle.db"))
            .unwrap();
    let now = AtomicI64::new(instant().timestamp());
    let clock = || Utc.timestamp_opt(now.load(Ordering::SeqCst), 0).unwrap();
    let ledger = PaperLedger::open(&db, &clock);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let buy1 = order(&ledger, &binding, "buy-1", Direction::Buy, 10.0, clock());
    assert_eq!(
        ledger.apply(PaperCommand::Execute(buy1)).unwrap().status,
        LedgerStatus::Filled
    );
    let view = ledger.read(&binding).unwrap();
    assert_eq!(view.cash, Money::from_cny(98_995.0).unwrap());
    assert_eq!(view.equity().unwrap(), Money::from_cny(99_995.0).unwrap());
    let buy2 = order(&ledger, &binding, "buy-2", Direction::Buy, 12.0, clock());
    ledger.apply(PaperCommand::Execute(buy2)).unwrap();
    let view = ledger.read(&binding).unwrap();
    assert_eq!(view.cash, Money::from_cny(97_790.0).unwrap());
    assert_eq!(view.equity().unwrap(), Money::from_cny(100_190.0).unwrap());
    let same_day_sell = order(&ledger, &binding, "t0-sell", Direction::Sell, 11.0, clock());
    assert_eq!(
        ledger
            .apply(PaperCommand::Execute(same_day_sell))
            .unwrap()
            .status,
        LedgerStatus::Rejected
    );
    assert_eq!(ledger.read(&binding).unwrap().cash, view.cash);
    now.store(instant().timestamp() + 5 * 3600, Ordering::SeqCst);
    mark_close(&ledger, &binding, "close-d0", 12.0, clock());
    now.store(instant().timestamp() + 86400, Ordering::SeqCst);
    let sell = order(&ledger, &binding, "sell-1", Direction::Sell, 11.0, clock());
    assert_eq!(
        ledger.apply(PaperCommand::Execute(sell)).unwrap().status,
        LedgerStatus::Filled
    );
    let view = ledger.read(&binding).unwrap();
    assert_eq!(view.cash, Money::from_cny(98_883.90).unwrap());
    assert_eq!(view.equity().unwrap(), Money::from_cny(99_983.90).unwrap());
    assert_eq!(view.fees, Money::from_cny(16.10).unwrap());
    assert_eq!(view.realized_pnl, Money::from_cny(88.90).unwrap());
    assert_eq!(view.lots.len(), 1);
    assert_eq!(view.lots[0].quantity, 100);
    assert_eq!(
        view.lots[0].buy_fee_remaining,
        Money::from_cny(5.0).unwrap()
    );
    now.store(instant().timestamp() + 86400 + 5 * 3600, Ordering::SeqCst);
    mark_close(&ledger, &binding, "close-d1", 13.0, clock());
    let view = ledger.read(&binding).unwrap();
    assert_eq!(view.cash, Money::from_cny(98_883.90).unwrap());
    assert_eq!(view.equity().unwrap(), Money::from_cny(100_183.90).unwrap());
    assert_eq!(view.daily_pnl(), Some(Money::from_cny(-6.10).unwrap()));
}

#[derive(diesel::QueryableByName)]
struct GoldenTextValue {
    #[diesel(sql_type = diesel::sql_types::Text)]
    value: String,
}

#[derive(diesel::QueryableByName)]
struct GoldenCount {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    value: i64,
}

fn golden_order_fees(db: &DatabaseManager, account_id: &str, command_id: &str) -> (Money, Money) {
    let payload = diesel::sql_query(
        "SELECT payload AS value FROM paper_ledger_event WHERE account_id=? AND command_id=?",
    )
    .bind::<diesel::sql_types::Text, _>(account_id)
    .bind::<diesel::sql_types::Text, _>(command_id)
    .get_result::<GoldenTextValue>(&mut db.get_conn().unwrap())
    .unwrap()
    .value;
    match serde_json::from_str::<Fact>(&payload).unwrap() {
        Fact::Order(order) => (order.commission, order.stamp),
        _ => panic!("golden sale event must be an order"),
    }
}

fn golden_filled_trade_count(db: &DatabaseManager) -> i64 {
    diesel::sql_query(
        "SELECT COUNT(*) AS value FROM paper_trades WHERE status='Filled'",
    )
    .get_result::<GoldenCount>(&mut db.get_conn().unwrap())
    .unwrap()
    .value
}

#[test]
fn paper_ledger_v1_golden_minimum_fees_fifo_replay() {
    use std::sync::atomic::{AtomicI64, Ordering};
    #[derive(Debug, diesel::QueryableByName)]
    struct TextValue {
        #[diesel(sql_type = diesel::sql_types::Text)]
        value: String,
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_v1_golden_minimum.db");
    let now = AtomicI64::new(instant().timestamp());
    let clock = || Utc.timestamp_opt(now.load(Ordering::SeqCst), 0).unwrap();
    let mut seed = manifest();
    seed.account_id = "TEST_CODE_V1_GOLDEN_MINIMUM".into();
    seed.epoch_id = "TEST_CODE_V1_GOLDEN_MINIMUM_EPOCH".into();
    seed.source_reference = "TEST_CODE_v1_golden_minimum_snapshot".into();
    let binding = seed.binding().unwrap();
    let (commands, hashes, old_report) = {
        let db = DatabaseManager::open_isolated_for_test(path.clone()).unwrap();
        let ledger = PaperLedger::open(&db, &clock);
        let seeded = ledger.apply(PaperCommand::Seed(seed)).unwrap();
        let buy_one = order(&ledger, &binding, "v1-golden-buy-10", Direction::Buy, 10.0, clock());
        let bought_one = ledger.apply(PaperCommand::Execute(buy_one.clone())).unwrap();
        let buy_two = order(&ledger, &binding, "v1-golden-buy-12", Direction::Buy, 12.0, clock());
        let bought_two = ledger.apply(PaperCommand::Execute(buy_two.clone())).unwrap();
        now.store(instant().timestamp() + 86400, Ordering::SeqCst);
        let sell = order(&ledger, &binding, "v1-golden-sell-11", Direction::Sell, 11.0, clock());
        let sold = ledger.apply(PaperCommand::Execute(sell.clone())).unwrap();
        let view = ledger.read(&binding).unwrap();
        let report = serde_json::to_string(&*view).unwrap();
        let head = diesel::sql_query("SELECT projection_hash AS value FROM paper_ledger_head WHERE account_id=?")
            .bind::<diesel::sql_types::Text, _>(&binding.account_id)
            .get_result::<TextValue>(&mut db.get_conn().unwrap()).unwrap();
        let model = diesel::sql_query("SELECT fee_model AS value FROM paper_ledger_account WHERE account_id=?")
            .bind::<diesel::sql_types::Text, _>(&binding.account_id)
            .get_result::<TextValue>(&mut db.get_conn().unwrap()).unwrap();
        assert_eq!(model.value, "lot-rates-v1");
        assert_eq!(binding.manifest_hash, "1e660132eccbbdeb97f032c4563da29cf6f0c551c59223f430ce97966d8806e6");
        assert_eq!(seeded.event_hash, "122741b580f8da54f096a8bb0df8da9e4ec242229548f8a02551f92bc5f39770");
        assert_ne!(bought_one.event_hash, bought_two.event_hash);
        assert_ne!(bought_two.event_hash, sold.event_hash);
        assert_eq!(head.value, "6c63fb6457630277f623bb2a16af312c88a416cc473818898e6d8357a2fcaed2");
        assert_eq!(report, r#"{"cash":98883900000,"lots":[{"lot_id":"fill:v1-golden-buy-12","code":"TEST_CODE_000001","name":"fixture","quantity":100,"basis_price":12000000,"buy_fee_remaining":5000000,"acquired_on":"2026-09-14","sellable_from":"2026-09-15","reported_cost":null}],"marks":{"TEST_CODE_000001":{"code":"TEST_CODE_000001","price":11000000,"observed_at":"2026-09-15T02:00:00Z","source":"TEST_CODE_realtime"}},"fees":16100000,"realized_pnl":88900000,"seed_equity":100000000000,"as_of":"2026-09-15T02:00:00Z","closes":{}}"#);
        assert_eq!([bought_one.fee, bought_two.fee, sold.fee], [Money::from_cny(5.0).unwrap(), Money::from_cny(5.0).unwrap(), Money::from_cny(6.10).unwrap()]);
        assert_eq!(
            golden_order_fees(&db, &binding.account_id, "v1-golden-sell-11"),
            (Money::from_cny(5.0).unwrap(), Money::from_cny(1.10).unwrap())
        );
        assert_eq!(golden_filled_trade_count(&db), 3);
        assert_eq!(view.cash, Money::from_cny(98_883.90).unwrap());
        assert_eq!(view.fees, Money::from_cny(16.10).unwrap());
        assert_eq!(view.realized_pnl, Money::from_cny(88.90).unwrap());
        assert_eq!(view.lots.len(), 1);
        assert_eq!(view.lots[0].basis_price, Money::from_cny(12.0).unwrap());
        assert_eq!(view.lots[0].quantity, 100);
        assert_eq!(view.lots[0].buy_fee_remaining, Money::from_cny(5.0).unwrap());
        assert_eq!(FEE_MODEL, "lot-rates-v1");
        let rejected = diesel::sql_query("INSERT INTO paper_ledger_account(account_id,epoch_id,manifest_hash,manifest_bytes,money_model,fee_model) SELECT 'TEST_CODE_V1_GOLDEN_REJECT_V2','TEST_CODE_V1_GOLDEN_REJECT_V2_EPOCH',manifest_hash,manifest_bytes,money_model,'lot-rates-v2' FROM paper_ledger_account WHERE account_id=?")
            .bind::<diesel::sql_types::Text, _>(&binding.account_id)
            .execute(&mut db.get_conn().unwrap());
        assert!(
            matches!(&rejected, Err(diesel::result::Error::DatabaseError(diesel::result::DatabaseErrorKind::CheckViolation, info)) if info.message().contains("fee_model")),
            "v1 fee_model CHECK must reject a v2 identity: {rejected:?}"
        );
        ((buy_one, buy_two, sell), [seeded.event_hash, bought_one.event_hash, bought_two.event_hash, sold.event_hash], report)
    };
    let db = DatabaseManager::open_isolated_for_test(path).unwrap();
    let ledger = PaperLedger::open(&db, &clock);
    for (command, expected) in [(commands.0, &hashes[1]), (commands.1, &hashes[2]), (commands.2, &hashes[3])] {
        let receipt = ledger.apply(PaperCommand::Execute(command)).unwrap();
        assert!(receipt.already_applied);
        assert_eq!(&receipt.event_hash, expected);
    }
    let reopened = ledger.read(&binding).unwrap();
    assert_eq!(reopened.version, 4);
    assert_eq!(golden_filled_trade_count(&db), 3);
    assert_eq!(serde_json::to_string(&*reopened).unwrap(), old_report);
    assert_eq!(reopened.event_hash, hashes[3]);
}

#[test]
fn paper_ledger_v1_golden_percentage_fees_replay() {
    use std::sync::atomic::{AtomicI64, Ordering};
    #[derive(Debug, diesel::QueryableByName)]
    struct TextValue {
        #[diesel(sql_type = diesel::sql_types::Text)]
        value: String,
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_v1_golden_percentage.db");
    let now = AtomicI64::new(instant().timestamp());
    let clock = || Utc.timestamp_opt(now.load(Ordering::SeqCst), 0).unwrap();
    let mut seed = manifest();
    seed.account_id = "TEST_CODE_V1_GOLDEN_PERCENTAGE".into();
    seed.epoch_id = "TEST_CODE_V1_GOLDEN_PERCENTAGE_EPOCH".into();
    seed.source_reference = "TEST_CODE_v1_golden_percentage_snapshot".into();
    seed.cash = Money::from_cny(1_000_000.0).unwrap();
    seed.original_total = seed.cash;
    let binding = seed.binding().unwrap();
    let (commands, hashes, old_report) = {
        let db = DatabaseManager::open_isolated_for_test(path.clone()).unwrap();
        let ledger = PaperLedger::open(&db, &clock);
        let seeded = ledger.apply(PaperCommand::Seed(seed)).unwrap();
        let buy = order(&ledger, &binding, "v1-golden-buy-200", Direction::Buy, 200.0, clock());
        let bought = ledger.apply(PaperCommand::Execute(buy.clone())).unwrap();
        now.store(instant().timestamp() + 86400, Ordering::SeqCst);
        let sell = order(&ledger, &binding, "v1-golden-sell-210", Direction::Sell, 210.0, clock());
        let sold = ledger.apply(PaperCommand::Execute(sell.clone())).unwrap();
        let view = ledger.read(&binding).unwrap();
        let report = serde_json::to_string(&*view).unwrap();
        let head = diesel::sql_query("SELECT projection_hash AS value FROM paper_ledger_head WHERE account_id=?")
            .bind::<diesel::sql_types::Text, _>(&binding.account_id)
            .get_result::<TextValue>(&mut db.get_conn().unwrap()).unwrap();
        let model = diesel::sql_query("SELECT fee_model AS value FROM paper_ledger_account WHERE account_id=?")
            .bind::<diesel::sql_types::Text, _>(&binding.account_id)
            .get_result::<TextValue>(&mut db.get_conn().unwrap()).unwrap();
        assert_eq!(model.value, "lot-rates-v1");
        assert_eq!(binding.manifest_hash, "9b25b982eb2218fc661995dbe1de70663c3ab45f186159ef782230e8622523dd");
        assert_eq!(seeded.event_hash, "e5f6c5340a5ec7e96e939491e3a8e63b4451b6ed4e62fb7991997a41a80e3b3f");
        assert_ne!(bought.event_hash, sold.event_hash);
        assert_eq!(head.value, "0ce46e10ee317b7d133047c4b178351f87221e259e4f00628c03f5e615b74aa7");
        assert_eq!(report, r#"{"cash":1000966700000,"lots":[],"marks":{},"fees":33300000,"realized_pnl":966700000,"seed_equity":1000000000000,"as_of":"2026-09-15T02:00:00Z","closes":{}}"#);
        assert_eq!(bought.status, LedgerStatus::Filled);
        assert_eq!(sold.status, LedgerStatus::Filled);
        assert_eq!(bought.fee, Money::from_cny(6.0).unwrap());
        assert_eq!(sold.fee, Money::from_cny(27.30).unwrap());
        assert_eq!(
            golden_order_fees(&db, &binding.account_id, "v1-golden-sell-210"),
            (Money::from_cny(6.30).unwrap(), Money::from_cny(21.0).unwrap())
        );
        assert_eq!(golden_filled_trade_count(&db), 2);
        assert_eq!(view.cash, Money::from_cny(1_000_966.70).unwrap());
        assert_eq!(view.fees, Money::from_cny(33.30).unwrap());
        assert_eq!(view.realized_pnl, Money::from_cny(966.70).unwrap());
        assert!(view.lots.is_empty());
        assert_eq!(FEE_MODEL, "lot-rates-v1");
        ((buy, sell), [seeded.event_hash, bought.event_hash, sold.event_hash], report)
    };
    let db = DatabaseManager::open_isolated_for_test(path).unwrap();
    let ledger = PaperLedger::open(&db, &clock);
    for (command, expected) in [(commands.0, &hashes[1]), (commands.1, &hashes[2])] {
        let receipt = ledger.apply(PaperCommand::Execute(command)).unwrap();
        assert!(receipt.already_applied);
        assert_eq!(&receipt.event_hash, expected);
    }
    let reopened = ledger.read(&binding).unwrap();
    assert_eq!(reopened.version, 3);
    assert_eq!(golden_filled_trade_count(&db), 2);
    assert_eq!(serde_json::to_string(&*reopened).unwrap(), old_report);
    assert_eq!(reopened.event_hash, hashes[2]);
}

#[test]
fn paper_ledger_idempotency_reopens_before_stale_version_or_quote_checks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_paper_idempotency.db");
    let seed = manifest();
    let binding = seed.binding().unwrap();
    let (intent, first) = {
        let db = DatabaseManager::open_isolated_for_test(path.clone()).unwrap();
        let ledger = PaperLedger::open(&db, &instant);
        ledger.apply(PaperCommand::Seed(seed)).unwrap();
        let intent = order(
            &ledger,
            &binding,
            "stable-buy",
            Direction::Buy,
            10.0,
            instant(),
        );
        let first = ledger.apply(PaperCommand::Execute(intent.clone())).unwrap();
        (intent, first)
    };
    let db = DatabaseManager::open_isolated_for_test(path).unwrap();
    let later = || instant() + chrono::Duration::days(1);
    let ledger = PaperLedger::open(&db, &later);
    for i in 0..10 {
        let mut repeated = intent.clone();
        if i > 0 {
            repeated.command_id = format!("new-attempt-{i}");
        }
        let result = ledger.apply(PaperCommand::Execute(repeated)).unwrap();
        assert!(result.already_applied);
        assert_eq!(result.event_hash, first.event_hash);
        assert_eq!(result.paper_trade_id, first.paper_trade_id);
    }
    let mut conflict = intent;
    conflict.signal.quantity = 200;
    assert!(matches!(
        ledger.apply(PaperCommand::Execute(conflict)),
        Err(LedgerError::IdentityConflict)
    ));
    let view = ledger.read(&binding).unwrap();
    assert_eq!(view.version, 2);
    assert_eq!(view.cash, Money::from_cny(98_995.0).unwrap());
    assert_eq!(view.fees, Money::from_cny(5.0).unwrap());
    assert_eq!(view.lots[0].quantity, 100);
}

#[test]
fn paper_ledger_idempotency_recovers_unknown_committed_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_paper_unknown.db"))
        .unwrap();
    let mut ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let intent = order(
        &ledger,
        &binding,
        "unknown-buy",
        Direction::Buy,
        10.0,
        instant(),
    );
    ledger.after_commit_fault = Some(&|| true);
    assert!(matches!(
        ledger.apply(PaperCommand::Execute(intent.clone())),
        Err(LedgerError::CommitOutcomeUnknown)
    ));
    ledger.after_commit_fault = None;
    let recovered = ledger.apply(PaperCommand::Execute(intent)).unwrap();
    assert!(recovered.already_applied);
    assert_eq!(recovered.status, LedgerStatus::Filled);
    assert_eq!(
        ledger.read(&binding).unwrap().cash,
        Money::from_cny(98_995.0).unwrap()
    );
}

#[test]
fn paper_ledger_concurrent_market_intent_replays_changed_quote_but_fixed_price_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    let db =
        DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_paper_price_intent.db"))
            .unwrap();
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let mut market = order(&ledger, &binding, "market", Direction::Buy, 10.0, instant());
    market.price_intent = PriceIntent::SignalQuoteMarketV1;
    let first = ledger.apply(PaperCommand::Execute(market.clone())).unwrap();
    market.command_id = "market-next-quote".into();
    market.signal.price = 12.0;
    market.quote_price = Money::from_cny(12.0).unwrap();
    market.signal.quote_observed_at = instant() - chrono::Duration::days(1);
    let replayed = ledger.apply(PaperCommand::Execute(market)).unwrap();
    assert!(replayed.already_applied);
    assert_eq!(replayed.event_hash, first.event_hash);
    let fixed = order(&ledger, &binding, "fixed", Direction::Buy, 10.0, instant());
    ledger.apply(PaperCommand::Execute(fixed.clone())).unwrap();
    let mut changed = fixed;
    changed.signal.price = 12.0;
    assert!(matches!(
        ledger.apply(PaperCommand::Execute(changed)),
        Err(LedgerError::IdentityConflict)
    ));
}

#[test]
fn paper_ledger_concurrent_orders_cannot_double_spend_cash_or_concentration() {
    use std::sync::{Arc, Barrier};
    for concentrated in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TEST_CODE_paper_concurrent.db");
        let db1 = DatabaseManager::open_isolated_for_test(path.clone()).unwrap();
        let db2 = DatabaseManager::open_isolated_for_test(path).unwrap();
        let ledger = PaperLedger::open(&db1, &instant);
        let mut seed = manifest();
        if concentrated {
            seed.cash = Money::from_cny(91_000.0).unwrap();
            seed.lots.push(SeedLot {
                code: "TEST_CODE_000001".into(),
                name: "fixture".into(),
                quantity: 900,
                reported_cost: None,
                sellable_from: None,
                sellability_evidence: None,
            });
            seed.marks.push(Mark {
                code: "TEST_CODE_000001".into(),
                price: Money::from_cny(10.0).unwrap(),
                observed_at: instant(),
                source: "TEST_CODE_cutover".into(),
            });
        } else {
            seed.cash = Money::from_cny(1005.0).unwrap();
            seed.original_total = seed.cash;
            seed.policy.max_position_bps = 10000;
            seed.policy.cash_floor_bps = 0;
        }
        let binding = seed.binding().unwrap();
        ledger.apply(PaperCommand::Seed(seed)).unwrap();
        let first = order(
            &ledger,
            &binding,
            "parallel-a",
            Direction::Buy,
            10.0,
            instant(),
        );
        let second = order(
            &ledger,
            &binding,
            "parallel-b",
            Direction::Buy,
            10.0,
            instant(),
        );
        let barrier = Arc::new(Barrier::new(2));
        let results = std::thread::scope(|scope| {
            let a = Arc::clone(&barrier);
            let first_db = &db1;
            let first = scope.spawn(move || {
                a.wait();
                PaperLedger::open(first_db, &instant).apply(PaperCommand::Execute(first))
            });
            let b = Arc::clone(&barrier);
            let second_db = &db2;
            let second = scope.spawn(move || {
                b.wait();
                PaperLedger::open(second_db, &instant).apply(PaperCommand::Execute(second))
            });
            [first.join().unwrap(), second.join().unwrap()]
        });
        assert_eq!(
            results
                .iter()
                .filter(|r| r.as_ref().is_ok_and(|r| r.status == LedgerStatus::Filled))
                .count(),
            1,
            "{results:?}"
        );
        assert_eq!(
            results
                .iter()
                .filter(|r| matches!(r, Err(LedgerError::VersionChanged)))
                .count(),
            1,
            "{results:?}"
        );
        let fresh_attempt = order(
            &ledger,
            &binding,
            "fresh-retry",
            Direction::Buy,
            10.0,
            instant(),
        );
        assert_eq!(
            ledger
                .apply(PaperCommand::Execute(fresh_attempt))
                .unwrap()
                .status,
            LedgerStatus::Rejected
        );
        let view = ledger.read(&binding).unwrap();
        assert_eq!(
            view.cash,
            Money::from_cny(if concentrated { 89_995.0 } else { 0.0 }).unwrap()
        );
        assert_eq!(view.fees, Money::from_cny(5.0).unwrap());
        assert_eq!(
            view.lots.iter().map(|lot| lot.quantity).sum::<u32>(),
            if concentrated { 1000 } else { 100 }
        );
    }
}

#[test]
fn paper_ledger_integrity_rejects_negative_seed_residual() {
    let dir = tempfile::tempdir().unwrap();
    let db =
        DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_negative_residual.db"))
            .unwrap();
    let ledger = PaperLedger::open(&db, &instant);
    let mut seed = manifest();
    seed.original_total = Money::from_cny(99_999.0).unwrap();
    seed.excluded_residual = Some(Money::from_cny(-1.0).unwrap());
    let binding = seed.binding().unwrap();
    assert!(matches!(
        ledger.apply(PaperCommand::Seed(seed)),
        Err(LedgerError::InvalidInput(_))
    ));
    assert!(matches!(ledger.read(&binding), Err(LedgerError::NotSeeded)));
}

#[test]
fn paper_ledger_integrity_failures_roll_back_trade_audit_event_and_cash() {
    for trigger in [
        "CREATE TRIGGER TEST_CODE_fault BEFORE INSERT ON order_audit_chain BEGIN SELECT RAISE(ABORT,'TEST_CODE_audit_failure'); END",
        "CREATE TRIGGER TEST_CODE_fault BEFORE INSERT ON paper_ledger_event BEGIN SELECT RAISE(ABORT,'TEST_CODE_event_failure'); END",
        "CREATE TRIGGER TEST_CODE_fault BEFORE UPDATE ON paper_ledger_head BEGIN SELECT RAISE(IGNORE); END",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_atomic_failure.db")).unwrap();
        let ledger = PaperLedger::open(&db,&instant);
        let seed = manifest();
        let binding = seed.binding().unwrap();
        ledger.apply(PaperCommand::Seed(seed)).unwrap();
        let before = ledger.read(&binding).unwrap();
        let intent = order(&ledger,&binding,"rollback",Direction::Buy,10.0,instant());
        let mut conn = db.get_conn().unwrap();
        conn.batch_execute(trigger).unwrap();
        assert!(ledger.apply(PaperCommand::Execute(intent.clone())).is_err());
        assert_eq!(ledger.read(&binding).unwrap(),before);
        for table in ["paper_trades","order_audit","order_audit_chain"] {
            let count = diesel::sql_query(format!("SELECT COUNT(*) AS value FROM {table}")).get_result::<IntegerRow>(&mut conn).unwrap().value;
            assert_eq!(count,0,"orphan in {table}");
        }
        conn.batch_execute("DROP TRIGGER TEST_CODE_fault").unwrap();
        let applied = ledger.apply(PaperCommand::Execute(intent)).unwrap();
        assert_eq!(applied.status,LedgerStatus::Filled);
        assert_eq!(ledger.read(&binding).unwrap().version,2);
    }
}

#[test]
fn paper_ledger_integrity_cache_loss_is_explicitly_repaired_not_trusted() {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_cache_repair.db"))
        .unwrap();
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    ledger
        .apply(PaperCommand::Execute(order(
            &ledger,
            &binding,
            "buy",
            Direction::Buy,
            10.0,
            instant(),
        )))
        .unwrap();
    let before = ledger.read(&binding).unwrap();
    let mut conn = db.get_conn().unwrap();
    diesel::sql_query("DELETE FROM paper_ledger_head")
        .execute(&mut conn)
        .unwrap();
    assert!(matches!(
        ledger.read(&binding),
        Err(LedgerError::IntegrityFailure(_))
    ));
    assert_eq!(ledger.repair_missing_projection(&binding).unwrap(), before);
    assert_eq!(ledger.read(&binding).unwrap(), before);
    diesel::sql_query("UPDATE paper_ledger_head SET projection_hash='tampered'")
        .execute(&mut conn)
        .unwrap();
    assert!(matches!(
        ledger.read(&binding),
        Err(LedgerError::IntegrityFailure(_))
    ));
    assert!(matches!(
        ledger.repair_missing_projection(&binding),
        Err(LedgerError::IntegrityFailure(_))
    ));
    assert!(
        diesel::sql_query("UPDATE paper_ledger_event SET payload='tampered'")
            .execute(&mut conn)
            .is_err()
    );
    conn.batch_execute("DROP TRIGGER paper_ledger_event_no_update; UPDATE paper_ledger_event SET payload='tampered' WHERE seq=2").unwrap();
    assert!(matches!(
        ledger.read(&binding),
        Err(LedgerError::IntegrityFailure(_))
    ));
}

#[test]
fn paper_ledger_integrity_cancel_and_lock_delay_do_not_authorize_stale_order() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_cancel_busy.db");
    let db = DatabaseManager::open_isolated_for_test(path.clone()).unwrap();
    let competing_db = DatabaseManager::open_isolated_for_test(path).unwrap();
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let intent = order(
        &ledger,
        &binding,
        "cancelled",
        Direction::Buy,
        10.0,
        instant(),
    );
    let cancelled = AtomicBool::new(false);
    let cancel_at_lock = || {
        cancelled.store(true, Ordering::SeqCst);
        instant()
    };
    assert!(matches!(
        PaperLedger::open(&db, &cancel_at_lock)
            .apply_controlled(PaperCommand::Execute(intent.clone()), &cancelled),
        Err(LedgerError::Cancelled)
    ));
    let mut writer = db.get_conn().unwrap();
    writer.batch_execute("BEGIN IMMEDIATE").unwrap();
    let result =
        PaperLedger::open(&competing_db, &instant).apply(PaperCommand::Execute(intent.clone()));
    writer.batch_execute("ROLLBACK").unwrap();
    assert!(
        matches!(result, Err(LedgerError::BusyRetryable)),
        "{result:?}"
    );
    let after_wait = || instant() + chrono::Duration::seconds(6);
    assert!(matches!(
        PaperLedger::open(&db, &after_wait).apply(PaperCommand::Execute(intent)),
        Err(LedgerError::EvidenceUnavailable(_))
    ));
    assert_eq!(ledger.read(&binding).unwrap().version, 1);
}

#[test]
fn paper_ledger_runtime_terminal_recovery_precedes_all_quote_io_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("TEST_CODE_runtime_recovery.db");
    let seed = manifest();
    let binding = seed.binding().unwrap();
    let quote = crate::broker::ExecutionQuote {
        price: 10.0,
        limit_up_price: 11.0,
        limit_down_price: 9.0,
        observed_at: instant(),
    };
    let cancelled = AtomicBool::new(false);
    let (mut signal, before) = {
        let db = DatabaseManager::open_isolated_for_test(path.clone()).unwrap();
        let ledger = PaperLedger::open(&db, &instant);
        ledger.apply(PaperCommand::Seed(seed)).unwrap();
        let mut signal = order(
            &ledger,
            &binding,
            "pushed-stock-buy-v1:recover",
            Direction::Buy,
            10.0,
            instant(),
        )
        .signal;
        signal.plan_id = "pushed-stock-buy-v1:recover".into();
        let source = |_: &str| Ok(quote.clone());
        let first = crate::trading::paper_ledger_runtime::execute_on(
            &db, &binding, &signal, &quote, &instant, &source, &cancelled,
        )
        .unwrap();
        assert!(first.inserted);
        let mut other = signal.clone();
        other.plan_id = "pushed-stock-buy-v1:other".into();
        other.code = "TEST_CODE_OTHER_HOLDING".into();
        crate::trading::paper_ledger_runtime::execute_on(
            &db, &binding, &other, &quote, &instant, &source, &cancelled,
        )
        .unwrap();
        (signal, ledger.read(&binding).unwrap())
    };
    let reopened = DatabaseManager::open_isolated_for_test(path).unwrap();
    let later = || instant() + chrono::Duration::days(1);
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let unavailable = |_: &str| {
        calls.fetch_add(1, Ordering::SeqCst);
        Err("TEST_CODE all external quotes unavailable".into())
    };
    // Both supplied evidence and the external source are now unusable. A
    // committed market-plan receipt is still independent of them.
    signal.price = 12.0;
    let again = crate::trading::paper_ledger_runtime::execute_on(
        &reopened,
        &binding,
        &signal,
        &quote,
        &later,
        &unavailable,
        &cancelled,
    )
    .expect("recover committed receipt without any quote I/O");
    assert!(!again.inserted);
    assert_eq!(again.result.fill_price, Some(10.0));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    signal.quantity += 100;
    let conflict = crate::trading::paper_ledger_runtime::execute_on(
        &reopened,
        &binding,
        &signal,
        &quote,
        &later,
        &unavailable,
        &cancelled,
    )
    .unwrap_err();
    assert!(conflict.contains("identity conflict"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    signal.plan_id = format!("paper:{}:{}", binding.epoch_id, signal.plan_id);
    assert!(matches!(
        PaperLedger::open(&reopened, &later).recover_terminal(
            &binding,
            &signal,
            PriceIntent::SignalQuoteMarketV1,
        ),
        Err(LedgerError::IdentityConflict)
    ));
    assert_eq!(
        PaperLedger::open(&reopened, &later).read(&binding).unwrap(),
        before
    );
}

#[test]
fn paper_ledger_runtime_binding_and_epoch_isolate_legacy_fills_and_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_runtime_epoch.db"))
        .unwrap();
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed.clone())).unwrap();
    let mut signal = order(
        &ledger,
        &binding,
        "pushed-stock-buy-v1:42",
        Direction::Buy,
        10.0,
        instant(),
    )
    .signal;
    signal.plan_id = "pushed-stock-buy-v1:42".into();
    let quote = crate::broker::ExecutionQuote {
        price: 10.0,
        limit_up_price: 11.0,
        limit_down_price: 9.0,
        observed_at: instant(),
    };
    let source = |_: &str| Ok(quote.clone());
    let cancelled = AtomicBool::new(false);
    let mut conn = db.get_conn().unwrap();
    conn.batch_execute("INSERT INTO paper_trades(plan_id,code,name,direction,price,quantity,ts,status,fill_price,virtual_reason,account_mode,data_mode) VALUES ('pushed-stock-buy-v1:42','TEST_CODE_000001','legacy','sell',10,900,'2026-09-13 10:00:00','Filled',10,'TEST_CODE_legacy','Normal','Full')").unwrap();
    let first = crate::trading::paper_ledger_runtime::execute_on(
        &db, &binding, &signal, &quote, &instant, &source, &cancelled,
    )
    .unwrap();
    assert!(first.inserted);
    let mut changed = signal.clone();
    changed.price = 12.0;
    let again = crate::trading::paper_ledger_runtime::execute_on(
        &db, &binding, &changed, &quote, &instant, &source, &cancelled,
    )
    .unwrap();
    assert!(!again.inserted);
    assert_eq!(again.result.fill_price, Some(10.0));
    let mut second_seed = seed;
    second_seed.account_id = "TEST_CODE_SECOND_ACCOUNT".into();
    second_seed.epoch_id = "TEST_CODE_SECOND_EPOCH".into();
    let second = second_seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(second_seed)).unwrap();
    crate::trading::paper_ledger_runtime::execute_on(
        &db, &second, &signal, &quote, &instant, &source, &cancelled,
    )
    .unwrap();
    for binding in [&binding, &second] {
        let view = ledger.read(binding).unwrap();
        assert_eq!(view.cash, Money::from_cny(98_995.0).unwrap());
        assert_eq!(view.lots[0].quantity, 100);
        assert_eq!(view.fees, Money::from_cny(5.0).unwrap());
    }
    let absent = AccountBinding {
        account_id: "TEST_CODE_UNSEEDED".into(),
        ..binding
    };
    assert!(crate::trading::paper_ledger_runtime::execute_on(
        &db, &absent, &signal, &quote, &instant, &source, &cancelled
    )
    .is_err());
    let count = diesel::sql_query("SELECT COUNT(*) AS value FROM paper_trades")
        .get_result::<IntegerRow>(&mut conn)
        .unwrap()
        .value;
    assert_eq!(
        count, 3,
        "original legacy fill must remain alongside two isolated epochs"
    );
}

#[test]
fn paper_ledger_history_read_points_and_effective_fill_seam_share_one_chain() {
    let dir = tempfile::tempdir().unwrap();
    let db =
        DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_history.db")).unwrap();
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let genesis = ledger.read(&binding).unwrap();
    let receipt = ledger
        .apply(PaperCommand::Execute(order(
            &ledger,
            &binding,
            "buy",
            Direction::Buy,
            10.0,
            instant(),
        )))
        .unwrap();
    assert_eq!(ledger.read_at_version(&binding, 1).unwrap(), genesis);
    assert_eq!(
        ledger.read_at_version(&binding, 2).unwrap(),
        ledger.read(&binding).unwrap()
    );
    assert!(ledger.read_at_version(&binding, 0).is_err());
    assert!(ledger.read_at_version(&binding, 3).is_err());
    let fills = ledger.effective_fills(&binding).unwrap();
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].paper_trade_id, receipt.paper_trade_id.unwrap());
    assert_eq!(fills[0].event_hash, receipt.event_hash);
    assert_eq!(fills[0].fill_price, Money::from_cny(10.0).unwrap());
    assert_eq!(fills[0].fee, Money::from_cny(5.0).unwrap());
    assert_eq!(
        ledger.read(&binding).unwrap().unrealized_pnl().unwrap(),
        Money::from_cny(-5.0).unwrap()
    );
}

#[test]
fn paper_ledger_history_terminal_nonfills_marks_and_explicit_rejected_retry() {
    let dir = tempfile::tempdir().unwrap();
    let db =
        DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_terminals.db")).unwrap();
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    for invalidated in [false, true] {
        let mut intent = order(
            &ledger,
            &binding,
            if invalidated { "slip" } else { "halt" },
            Direction::Buy,
            10.0,
            instant(),
        );
        intent.price_intent = PriceIntent::SignalQuoteMarketV1;
        if invalidated {
            intent.quote_price = Money::from_cny(11.0).unwrap();
        } else {
            intent.signal.is_suspended = true;
        }
        let receipt = ledger.apply(PaperCommand::Execute(intent.clone())).unwrap();
        assert_eq!(
            receipt.status,
            if invalidated {
                LedgerStatus::Invalidated
            } else {
                LedgerStatus::NotFilled
            }
        );
        intent.command_id.push_str("retry");
        intent.signal.is_suspended = false;
        intent.signal.quote_observed_at = instant() - chrono::Duration::days(1);
        assert_eq!(
            ledger
                .apply(PaperCommand::Execute(intent))
                .unwrap()
                .event_hash,
            receipt.event_hash
        );
    }
    let mut rejected = order(
        &ledger,
        &binding,
        "rejected",
        Direction::Buy,
        10.0,
        instant(),
    );
    rejected.signal.risk_context = PaperRiskContext::new(
        crate::risk::action_gate::AccountMode::Normal,
        crate::monitor::data_mode::DataMode::Unsafe,
    );
    let receipt = ledger
        .apply(PaperCommand::Execute(rejected.clone()))
        .unwrap();
    assert_eq!(receipt.status, LedgerStatus::Rejected);
    rejected.signal.risk_context = PaperRiskContext::new(
        crate::risk::action_gate::AccountMode::Normal,
        crate::monitor::data_mode::DataMode::Full,
    );
    assert_eq!(
        ledger
            .apply(PaperCommand::Execute(rejected.clone()))
            .unwrap()
            .event_hash,
        receipt.event_hash
    );
    rejected.command_id.push_str("explicit-new-attempt");
    rejected.expected_version = ledger.read(&binding).unwrap().version;
    assert_eq!(
        ledger
            .apply(PaperCommand::Execute(rejected))
            .unwrap()
            .status,
        LedgerStatus::Filled
    );
    let before = ledger.read(&binding).unwrap();
    let batch = ValuationBatch {
        binding: binding.clone(),
        command_id: "mark".into(),
        expected_version: before.version,
        inventory_fingerprint: before.inventory_fingerprint().unwrap(),
        as_of: instant(),
        closing: false,
        marks: vec![Mark {
            code: "TEST_CODE_000001".into(),
            price: Money::from_cny(12.0).unwrap(),
            observed_at: instant(),
            source: "TEST_CODE_mark".into(),
        }],
    };
    let mark = ledger.apply(PaperCommand::Mark(batch.clone())).unwrap();
    assert_eq!(
        ledger.apply(PaperCommand::Mark(batch)).unwrap().event_hash,
        mark.event_hash
    );
    let view = ledger.read(&binding).unwrap();
    assert_eq!(view.cash, before.cash);
    assert_eq!(view.lots, before.lots);
    assert!(view.daily_pnl().is_none());
    let mut missing = order(
        &ledger,
        &binding,
        "missing",
        Direction::Buy,
        10.0,
        instant(),
    );
    missing.marks.clear();
    assert!(matches!(
        ledger.apply(PaperCommand::Execute(missing)),
        Err(LedgerError::EvidenceUnavailable(_))
    ));
    assert_eq!(ledger.read(&binding).unwrap(), view);
}

#[test]
fn paper_ledger_history_exposure_reducing_sell_ignores_buy_only_floors() {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_sell_floor.db"))
        .unwrap();
    let ledger = PaperLedger::open(&db, &instant);
    let mut seed = manifest();
    seed.cash = Money::ZERO;
    seed.original_total = Money::from_cny(1000.0).unwrap();
    seed.lots = vec![SeedLot {
        code: "TEST_CODE_000001".into(),
        name: "fixture".into(),
        quantity: 100,
        reported_cost: Some(Money::from_cny(20.0).unwrap()),
        sellable_from: Some(day(instant())),
        sellability_evidence: Some("TEST_CODE_confirmed_inventory".into()),
    }];
    seed.marks = vec![Mark {
        code: "TEST_CODE_000001".into(),
        price: Money::from_cny(10.0).unwrap(),
        observed_at: instant(),
        source: "TEST_CODE_cutover".into(),
    }];
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let sell = order(
        &ledger,
        &binding,
        "reduce",
        Direction::Sell,
        10.0,
        instant(),
    );
    assert_eq!(
        ledger.apply(PaperCommand::Execute(sell)).unwrap().status,
        LedgerStatus::Filled
    );
    let view = ledger.read(&binding).unwrap();
    assert_eq!(view.cash, Money::from_cny(994.0).unwrap());
    assert_eq!(
        view.realized_pnl,
        Money::from_cny(-6.0).unwrap(),
        "reported historical cost must not become post-cutover loss"
    );
    assert!(view.lots.is_empty());
    assert!(crate::trading::paper_trade::simulate(
        &order(
            &ledger,
            &binding,
            "disabled",
            Direction::Buy,
            10.0,
            instant()
        )
        .signal,
        10.0,
        1_000_000.0,
        1_000_000.0,
        0.0
    )
    .unwrap_err()
    .contains("disabled"));
}

#[test]
fn paper_ledger_history_partial_lot_absorbs_fee_remainder_and_overflow_rolls_back() {
    let dir = tempfile::tempdir().unwrap();
    let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_fee_remainder.db"))
        .unwrap();
    let ledger = PaperLedger::open(&db, &instant);
    let seed = manifest();
    let binding = seed.binding().unwrap();
    ledger.apply(PaperCommand::Seed(seed)).unwrap();
    let excessive = order(
        &ledger,
        &binding,
        "overflow",
        Direction::Buy,
        100_000_000_000.0,
        instant(),
    );
    assert!(matches!(
        ledger.apply(PaperCommand::Execute(excessive)),
        Err(LedgerError::Overflow)
    ));
    assert_eq!(ledger.read(&binding).unwrap().version, 1);
    let mut buy = order(
        &ledger,
        &binding,
        "buy-300",
        Direction::Buy,
        10.0,
        instant(),
    );
    buy.signal.quantity = 300;
    ledger.apply(PaperCommand::Execute(buy)).unwrap();
    let tomorrow = || instant() + chrono::Duration::days(1);
    let ledger = PaperLedger::open(&db, &tomorrow);
    for (index, remaining_fee) in [(1, 3.333334), (2, 1.666667), (3, 0.0)] {
        let sell = order(
            &ledger,
            &binding,
            &format!("sell-{index}"),
            Direction::Sell,
            10.0,
            tomorrow(),
        );
        assert_eq!(
            ledger.apply(PaperCommand::Execute(sell)).unwrap().status,
            LedgerStatus::Filled
        );
        let view = ledger.read(&binding).unwrap();
        assert_eq!(
            view.lots
                .first()
                .map(|lot| lot.buy_fee_remaining)
                .unwrap_or(Money::ZERO),
            Money::from_cny(remaining_fee).unwrap()
        );
    }
    let view = ledger.read(&binding).unwrap();
    assert_eq!(view.cash, Money::from_cny(99_977.0).unwrap());
    assert_eq!(view.realized_pnl, Money::from_cny(-23.0).unwrap());
    assert_eq!(view.fees, Money::from_cny(23.0).unwrap());
}
