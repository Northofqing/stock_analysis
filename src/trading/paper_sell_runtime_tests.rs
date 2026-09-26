use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar};

struct SlowQuoteReadIo {
    quote_entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    release: Arc<(Mutex<bool>, Condvar)>,
    sibling_progress: Arc<AtomicBool>,
    progress_before_quote_returned: AtomicBool,
    requested_codes: Mutex<Vec<String>>,
}

impl PaperSellReadIo for SlowQuoteReadIo {
    fn positions(&self, today: chrono::NaiveDate) -> Result<Vec<PaperPosition>, String> {
        Ok(["TEST_CODE_PAPER_RUNTIME_A", "TEST_CODE_PAPER_RUNTIME_B"]
            .into_iter()
            .map(|code| PaperPosition {
                code: code.to_owned(),
                name: code.to_owned(),
                quantity: 100,
                avg_buy_price: 10.0,
                buy_fee_cost: 5.0,
                first_buy_date: today.pred_opt().expect("fixture previous date"),
                inventory_audit_evidence: "TEST_CODE_UNUSED_NO_QUOTE".to_owned(),
                checkpoint: None,
            })
            .collect())
    }

    fn execution_quote(&self, code: &str) -> Result<crate::broker::ExecutionQuote, String> {
        assert!(code.starts_with("TEST_CODE_PAPER_RUNTIME_"));
        self.requested_codes.lock().unwrap().push(code.to_owned());
        if let Some(entered) = self.quote_entered.lock().unwrap().take() {
            let _ = entered.send(());
            let (released, wake) = &*self.release;
            let _release_guard = wake
                .wait_timeout_while(
                    released.lock().unwrap(),
                    Duration::from_secs(2),
                    |released| !*released,
                )
                .expect("finite quote release");
            self.progress_before_quote_returned.store(
                self.sibling_progress.load(Ordering::SeqCst),
                Ordering::SeqCst,
            );
        }
        // Real evaluate_and_sell stops here. No indicator, DB write or provider
        // can be reached, even on the intentionally failing inline scan path.
        Err("TEST_CODE controlled unavailable quote".to_owned())
    }
}

#[tokio::test(flavor = "current_thread")]
async fn paper_runtime_slow_real_scan_allows_same_group_heartbeat_and_shutdown_progress() {
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let sibling_progress = Arc::new(AtomicBool::new(false));
    let io = Arc::new(SlowQuoteReadIo {
        quote_entered: Mutex::new(Some(entered_tx)),
        release: Arc::clone(&release),
        sibling_progress: Arc::clone(&sibling_progress),
        progress_before_quote_returned: AtomicBool::new(false),
        requested_codes: Mutex::new(Vec::new()),
    });
    let risk_context = PaperRiskContext::new(
        crate::risk::action_gate::AccountMode::Normal,
        crate::monitor::data_mode::DataMode::Full,
    );
    let today = chrono::NaiveDate::from_ymd_opt(2026, 9, 14).unwrap();

    // Exercise the session's production scan path with external reads replaced;
    // rule evaluation and per-security orchestration remain the real ones.
    let session = PaperScanSession::new();
    let scan = session.scan_with_io(risk_context, today, Arc::clone(&io));
    let heartbeat_and_shutdown = async {
        entered_rx.await.expect("real scan entered quote read");
        sibling_progress.store(true, Ordering::SeqCst);
        let (released, wake) = &*release;
        *released.lock().unwrap() = true;
        wake.notify_all();
    };

    let (result, ()) = tokio::join!(biased; scan, heartbeat_and_shutdown);
    assert!(result
        .expect("ordinary quote failures continue scanning")
        .is_empty());
    assert_eq!(
        *io.requested_codes.lock().unwrap(),
        ["TEST_CODE_PAPER_RUNTIME_A", "TEST_CODE_PAPER_RUNTIME_B"],
        "the actual orchestrator must visit both synthetic securities"
    );
    assert!(
        io.progress_before_quote_returned.load(Ordering::SeqCst),
        "same-group heartbeat/shutdown was starved until the slow real scan returned"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn paper_runtime_cancelled_caller_drains_quote_without_starting_second_security() {
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let io = Arc::new(SlowQuoteReadIo {
        quote_entered: Mutex::new(Some(entered_tx)),
        release: Arc::clone(&release),
        sibling_progress: Arc::new(AtomicBool::new(false)),
        progress_before_quote_returned: AtomicBool::new(false),
        requested_codes: Mutex::new(Vec::new()),
    });
    let session = PaperScanSession::new();
    let risk_context = PaperRiskContext::new(
        crate::risk::action_gate::AccountMode::Normal,
        crate::monitor::data_mode::DataMode::Full,
    );
    let today = chrono::NaiveDate::from_ymd_opt(2026, 9, 14).unwrap();

    {
        let scan = session.scan_with_io(risk_context, today, Arc::clone(&io));
        tokio::pin!(scan);
        tokio::select! {
            biased;
            result = &mut scan => panic!("scan returned before shutdown: {result:?}"),
            entered = entered_rx => entered.expect("real first quote entered"),
        }
        // Reproduce the supervisor dropping main_loops on shutdown. The
        // session must still own the worker after this awaiter is dropped.
    }

    let drain = session.close_and_drain();
    let release_quote = async {
        let (released, wake) = &*release;
        *released.lock().unwrap() = true;
        wake.notify_all();
    };
    // Drain is polled first, requesting cancellation before the quote is
    // released. It awaits the worker while the sibling performs the release.
    let (result, ()) = tokio::join!(biased; drain, release_quote);
    assert!(result
        .expect("retained worker must join before drain returns")
        .is_empty());
    assert_eq!(
        *io.requested_codes.lock().unwrap(),
        ["TEST_CODE_PAPER_RUNTIME_A"],
        "shutdown must not start the second security after the in-flight quote exits"
    );
}

enum SecondQuote {
    Error,
    Panic,
    Slow(SlowQuoteReadIo),
}

struct SaleReadIo {
    store: LedgerFixture,
    second: SecondQuote,
    quotes: Mutex<Vec<String>>,
}

impl SaleReadIo {
    fn new(second: SecondQuote) -> Self {
        Self {
            store: LedgerFixture::new(),
            second,
            quotes: Mutex::new(Vec::new()),
        }
    }
}

struct LedgerFixture {
    db: crate::database::DatabaseManager,
    binding: crate::trading::paper_ledger::AccountBinding,
    _directory: tempfile::TempDir,
}
impl LedgerFixture {
    fn new() -> Self {
        use crate::trading::paper_ledger::*;
        let directory = tempfile::tempdir().unwrap();
        let db = crate::database::DatabaseManager::open_isolated_for_test(
            directory.path().join("TEST_CODE_runtime_sales.db"),
        )
        .unwrap();
        diesel::sql_query("PRAGMA application_id=1398035265")
            .execute(&mut db.get_conn().unwrap())
            .unwrap();
        diesel::sql_query("PRAGMA user_version=2")
            .execute(&mut db.get_conn().unwrap())
            .unwrap();
        let cutover = chrono::Utc::now() - chrono::Duration::days(2);
        let sellable = chrono::Local::now().date_naive();
        let seed = SeedManifest {
            account_id: "TEST_CODE_SALE_ACCOUNT".into(),
            epoch_id: "TEST_CODE_SALE_EPOCH".into(),
            command_id: "seed".into(),
            cutover_at: cutover,
            account_effective_at: cutover,
            positions_effective_at: cutover,
            source_reference: "TEST_CODE_explicit_seed".into(),
            source_hash: "c".repeat(64),
            approved_by: "TEST_CODE_operator".into(),
            cash: Money::from_cny(50_000.0).unwrap(),
            original_total: Money::from_cny(53_000.0).unwrap(),
            excluded_residual: None,
            lots: ["A", "B", "C"]
                .into_iter()
                .map(|suffix| SeedLot {
                    code: format!("TEST_CODE_PAPER_RUNTIME_{suffix}"),
                    name: suffix.into(),
                    quantity: 100,
                    reported_cost: None,
                    sellable_from: Some(sellable),
                    sellability_evidence: Some("TEST_CODE_confirmed_sellable".into()),
                })
                .collect(),
            marks: ["A", "B", "C"]
                .into_iter()
                .map(|suffix| Mark {
                    code: format!("TEST_CODE_PAPER_RUNTIME_{suffix}"),
                    price: Money::from_cny(10.0).unwrap(),
                    observed_at: cutover,
                    source: "TEST_CODE_cutover".into(),
                })
                .collect(),
            policy: RiskPolicyV1::default(),
        };
        let binding = seed.binding().unwrap();
        PaperLedger::open(&db, &chrono::Utc::now)
            .apply(PaperCommand::Seed(seed))
            .unwrap();
        Self {
            db,
            binding,
            _directory: directory,
        }
    }
    fn already_sold(&self, code: &str, today: &str) -> Result<bool, String> {
        crate::trading::paper_ledger_runtime::already_sold_on(&self.db, &self.binding, code, today)
    }
    fn sold_codes(&self) -> Vec<String> {
        #[derive(diesel::QueryableByName)]
        struct Row {
            #[diesel(sql_type=diesel::sql_types::Text)]
            code: String,
        }
        diesel::sql_query(
            "SELECT code FROM paper_trades WHERE direction='sell' AND status='Filled' ORDER BY id",
        )
        .load::<Row>(&mut self.db.get_conn().unwrap())
        .unwrap()
        .into_iter()
        .map(|row| row.code)
        .collect()
    }
}

impl PaperSellReadIo for SaleReadIo {
    fn positions(&self, today: chrono::NaiveDate) -> Result<Vec<PaperPosition>, String> {
        crate::trading::paper_ledger_runtime::sellable_positions_on(
            &self.store.db,
            &self.store.binding,
            today,
        )
    }

    fn execution_quote(&self, code: &str) -> Result<crate::broker::ExecutionQuote, String> {
        self.quotes.lock().unwrap().push(code.to_owned());
        if code.ends_with("_B") {
            return match &self.second {
                SecondQuote::Error => Err("TEST_CODE ordinary quote failure".to_owned()),
                SecondQuote::Panic => panic!("TEST_CODE quote panic after first completed sale"),
                SecondQuote::Slow(io) => io.execution_quote(code),
            };
        }
        Ok(crate::broker::ExecutionQuote {
            price: 8.0,
            limit_down_price: 6.0,
            limit_up_price: 12.0,
            observed_at: chrono::Utc::now(),
        })
    }

    fn daily_bars(&self, code: &str) -> Result<Vec<KlineData>, String> {
        assert!(code.starts_with("TEST_CODE_"));
        Ok((0..60)
            .map(|day| KlineData {
                date: chrono::NaiveDate::from_ymd_opt(2026, 9, 14).unwrap()
                    - chrono::Duration::days(day),
                open: 10.0,
                high: 10.1,
                low: 9.9,
                close: 10.0,
                volume: 1000.0,
                amount: 10000.0,
                pct_chg: 0.0,
                intraday_price: None,
                settled: true,
                pe_ratio: None,
                pb_ratio: None,
                turnover_rate: None,
                market_cap: None,
                circulating_cap: None,
                eps: None,
                roe: None,
                revenue_yoy: None,
                net_profit_yoy: None,
                gross_margin: None,
                net_margin: None,
                sharpe_ratio: None,
                financials_history: None,
                valuation_history: None,
                consensus: None,
                industry: None,
                is_limit_up: false,
                is_limit_down: false,
                is_suspended: false,
                adjust: crate::data_provider::AdjustType::None,
            })
            .collect())
    }

    fn already_sold(&self, code: &str, today: &str) -> Result<bool, String> {
        self.store.already_sold(code, today)
    }

    fn execute(
        &self,
        position: &PaperPosition,
        signal: &PaperSignal,
        quote: &crate::broker::ExecutionQuote,
        cancelled: &AtomicBool,
    ) -> Result<crate::trading::paper_trade::PaperOutcome, String> {
        // Fixture valuation is distinct from the deliberately slow/failing scan
        // quote. The actual transaction, risk, FIFO and compatible writes run.
        crate::trading::paper_ledger_runtime::execute_checked_on(
            &self.store.db,
            &self.store.binding,
            signal,
            quote,
            &chrono::Utc::now,
            &|code| {
                assert!(code.starts_with("TEST_CODE_"));
                Ok(crate::broker::ExecutionQuote {
                    observed_at: chrono::Utc::now(),
                    ..quote.clone()
                })
            },
            cancelled,
            position.checkpoint.as_ref(),
        )
    }
}

fn runtime_risk() -> PaperRiskContext {
    PaperRiskContext::new(
        crate::risk::action_gate::AccountMode::Normal,
        crate::monitor::data_mode::DataMode::Full,
    )
}

#[tokio::test(flavor = "current_thread")]
async fn paper_runtime_real_sales_preserve_order_and_continue_after_quote_failure() {
    let io = Arc::new(SaleReadIo::new(SecondQuote::Error));
    let session = PaperScanSession::new();
    let sold = session
        .scan_with_io(
            runtime_risk(),
            chrono::Local::now().date_naive(),
            Arc::clone(&io),
        )
        .await
        .expect("real scan and simulate");
    assert_eq!(
        sold.iter()
            .map(|sale| sale.code.as_str())
            .collect::<Vec<_>>(),
        ["TEST_CODE_PAPER_RUNTIME_A", "TEST_CODE_PAPER_RUNTIME_C"]
    );
    assert!(sold
        .iter()
        .all(|sale| sale.price == 8.0 && sale.quantity == 100));
    assert_eq!(
        io.store.sold_codes(),
        ["TEST_CODE_PAPER_RUNTIME_A", "TEST_CODE_PAPER_RUNTIME_C"]
    );
    assert_eq!(
        *io.quotes.lock().unwrap(),
        [
            "TEST_CODE_PAPER_RUNTIME_A",
            "TEST_CODE_PAPER_RUNTIME_B",
            "TEST_CODE_PAPER_RUNTIME_C"
        ]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn paper_runtime_panic_reports_failure_and_preserves_prior_real_sale() {
    let io = Arc::new(SaleReadIo::new(SecondQuote::Panic));
    let session = PaperScanSession::new();
    let error = session
        .scan_with_io(
            runtime_risk(),
            chrono::Local::now().date_naive(),
            Arc::clone(&io),
        )
        .await
        .expect_err("panic must be explicit");
    assert!(error.detail.contains("TEST_CODE quote panic"), "{error}");
    assert_eq!(
        error
            .sold
            .iter()
            .map(|sale| sale.code.as_str())
            .collect::<Vec<_>>(),
        ["TEST_CODE_PAPER_RUNTIME_A"]
    );
    assert_eq!(io.store.sold_codes(), ["TEST_CODE_PAPER_RUNTIME_A"]);
    assert!(session
        .close_and_drain()
        .await
        .expect("worker already joined")
        .is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn paper_runtime_shutdown_retains_real_sale_and_joins_dropped_callers_worker() {
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let io = Arc::new(SaleReadIo::new(SecondQuote::Slow(SlowQuoteReadIo {
        quote_entered: Mutex::new(Some(entered_tx)),
        release: Arc::clone(&release),
        sibling_progress: Arc::new(AtomicBool::new(false)),
        progress_before_quote_returned: AtomicBool::new(false),
        requested_codes: Mutex::new(Vec::new()),
    })));
    let session = PaperScanSession::new();
    {
        let scan = session.scan_with_io(
            runtime_risk(),
            chrono::Local::now().date_naive(),
            Arc::clone(&io),
        );
        tokio::pin!(scan);
        tokio::select! {
            biased;
            result = &mut scan => panic!("scan returned before second quote: {result:?}"),
            entered = entered_rx => entered.expect("second quote entered after real first sale"),
        }
    }
    let release_quote = async {
        let (released, wake) = &*release;
        *released.lock().unwrap() = true;
        wake.notify_all();
    };
    let (result, ()) = tokio::join!(biased; session.close_and_drain(), release_quote);
    let sold = result.expect("join cancelled scan without losing sales");
    assert_eq!(
        sold.iter()
            .map(|sale| sale.code.as_str())
            .collect::<Vec<_>>(),
        ["TEST_CODE_PAPER_RUNTIME_A"]
    );
    assert_eq!(io.store.sold_codes(), ["TEST_CODE_PAPER_RUNTIME_A"]);
    assert_eq!(
        *io.quotes.lock().unwrap(),
        ["TEST_CODE_PAPER_RUNTIME_A", "TEST_CODE_PAPER_RUNTIME_B"]
    );
    assert!(session
        .scan_with_io(
            runtime_risk(),
            chrono::Local::now().date_naive(),
            Arc::clone(&io)
        )
        .await
        .is_err());
}
