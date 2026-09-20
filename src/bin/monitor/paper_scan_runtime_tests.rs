use super::*;
use diesel::prelude::*;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const CHILD_CASE: &str = "TEST_CODE_PAPER_RUNTIME_CHILD_CASE";
const CHILD_ROOT: &str = "TEST_CODE_PAPER_RUNTIME_CHILD_ROOT";

struct ChildGuard(std::process::Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

/// Parent retains the directory until its isolated test child has exited.
fn run_child(test: &str, case: &str) {
    let root = tempfile::Builder::new()
        .prefix("TEST_CODE_PAPER_RUNTIME_")
        .tempdir()
        .expect("owned child database root");
    let canonical_root = root.path().canonicalize().unwrap();
    let mut child = ChildGuard(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--nocapture", "--test-threads=1"])
            .env(CHILD_CASE, case)
            .env(CHILD_ROOT, &canonical_root)
            .env("STOCK_ENV_MODE", "test")
            .env("PAPER_CASH_FLOOR_PCT", "15")
            .env("PAPER_MAX_SLIPPAGE", "2")
            .env("PAPER_MAX_POSITION_PCT", "10")
            .current_dir(&canonical_root)
            .spawn()
            .expect("spawn only the current test binary"),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.0.try_wait().expect("test child status") {
            assert!(status.success(), "isolated {case} child failed: {status}");
            return;
        }
        assert!(Instant::now() < deadline, "isolated {case} child timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct FiniteQuote {
    entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    release: Arc<(Mutex<bool>, Condvar)>,
    codes: Arc<Mutex<Vec<String>>>,
    order: Arc<Mutex<Vec<&'static str>>>,
}

impl stock_analysis::broker::QuoteProvider for FiniteQuote {
    fn get_execution_quote(
        &self,
        code: &str,
    ) -> Result<stock_analysis::broker::ExecutionQuote, String> {
        assert!(matches!(
            code,
            "TEST_CODE_PAPER_MAIN_A" | "TEST_CODE_PAPER_MAIN_B"
        ));
        self.codes.lock().unwrap().push(code.to_owned());
        if let Some(entered) = self.entered.lock().unwrap().take() {
            let _ = entered.send(());
            let (released, wake) = &*self.release;
            let _guard = wake
                .wait_timeout_while(released.lock().unwrap(), Duration::from_secs(2), |value| {
                    !*value
                })
                .expect("finite external quote wait");
        }
        self.order.lock().unwrap().push("quote_exit");
        Err("TEST_CODE controlled quote unavailable".to_owned())
    }
}

async fn child_case(missing_writer: bool) {
    use stock_analysis::database::DatabaseManager;
    assert!(
        DatabaseManager::try_get().is_none(),
        "child must own a fresh global DB"
    );
    assert!(!stock_analysis::broker::quote_provider_registered());
    let root = std::path::PathBuf::from(std::env::var_os(CHILD_ROOT).expect("parent-owned root"));
    assert!(root.is_absolute());
    let root = root.canonicalize().unwrap();
    assert!(root.starts_with(std::env::temp_dir().canonicalize().unwrap()));
    assert!(root
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("TEST_CODE_PAPER_RUNTIME_"));
    let path = root.join("stock.db");
    assert!(!path.exists());
    DatabaseManager::init(Some(path.clone()))
        .expect("explicit private DB path in non-cfg-test library");
    let mut conn = DatabaseManager::get().get_conn().unwrap();
    #[derive(QueryableByName)]
    struct DbFile {
        #[diesel(sql_type = diesel::sql_types::Text)]
        name: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        file: String,
    }
    let files = diesel::sql_query("PRAGMA database_list")
        .load::<DbFile>(&mut conn)
        .unwrap();
    let actual = &files
        .iter()
        .find(|file| file.name == "main")
        .expect("SQLite main file")
        .file;
    assert_eq!(
        std::path::Path::new(actual).canonicalize().unwrap(),
        path.canonicalize().unwrap()
    );
    let yesterday = chrono::Local::now().date_naive().pred_opt().unwrap();
    for code in ["TEST_CODE_PAPER_MAIN_A", "TEST_CODE_PAPER_MAIN_B"] {
        diesel::sql_query("INSERT INTO paper_trades (plan_id, code, name, direction, price, quantity, status, fill_price, virtual_reason, account_mode, data_mode, ts) VALUES (?, ?, 'TEST_CODE', 'buy', 10, 100, 'Filled', 10, 'TEST_CODE fixture', 'Normal', 'Full', ?)")
            .bind::<diesel::sql_types::Text, _>(format!("TEST_CODE_buy_{code}"))
            .bind::<diesel::sql_types::Text, _>(code)
            .bind::<diesel::sql_types::Text, _>(format!("{yesterday} 10:00:00"))
            .execute(&mut conn).unwrap();
    }
    drop(conn);
    let order = Arc::new(Mutex::new(Vec::new()));
    let codes = Arc::new(Mutex::new(Vec::new()));
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let (entered_tx, mut entered_rx) = tokio::sync::oneshot::channel();
    stock_analysis::broker::register_quote_provider(Box::new(FiniteQuote {
        entered: Mutex::new(Some(entered_tx)),
        release: Arc::clone(&release),
        codes: Arc::clone(&codes),
        order: Arc::clone(&order),
    }))
    .expect("only controlled provider");
    let session = PaperScanSession::new();
    let risk = stock_analysis::trading::paper_trade::PaperRiskContext::new(
        stock_analysis::risk::action_gate::AccountMode::Normal,
        stock_analysis::monitor::data_mode::DataMode::Full,
    );
    if missing_writer {
        let scan = session.scan(PaperScanPhase::PostClose, risk);
        tokio::pin!(scan);
        tokio::select! {
            biased;
            result = &mut scan => panic!("scan returned before real quote: {result:?}"),
            entered = &mut entered_rx => entered.expect("real registered provider entered"),
        }
    }
    let bus = stock_analysis::event::EventBus::new_for_test(8);
    let mut writer = if missing_writer {
        None
    } else {
        let mut receiver = bus.subscribe().unwrap();
        let writer_order = Arc::clone(&order);
        Some(tokio::spawn(async move {
            while receiver.recv().await.is_ok() {}
            writer_order.lock().unwrap().push("writer");
            Ok(())
        }))
    };
    struct Marker(Arc<Mutex<Vec<&'static str>>>);
    impl Drop for Marker {
        fn drop(&mut self) {
            self.0.lock().unwrap().push("producer");
        }
    }
    let producer_order = Arc::clone(&order);
    let producer = tokio::spawn(async move {
        let _marker = Marker(producer_order);
        std::future::pending::<()>().await;
    });
    tokio::task::yield_now().await;
    let main_loops = async {
        if !missing_writer {
            let _ = session.scan(PaperScanPhase::PostClose, risk).await;
        }
        std::future::pending::<()>().await;
    };
    let signal = async {
        if missing_writer {
            std::future::pending::<()>().await;
        }
        (&mut entered_rx)
            .await
            .expect("real registered provider entered");
        Ok(())
    };
    let lifecycle = supervise_long_running_lifecycle(
        &bus,
        &mut writer,
        &session,
        vec![("TEST_CODE producer", producer)],
        main_loops,
        signal,
    );
    let release_quote = async {
        let cancelled = tokio::time::timeout(Duration::from_secs(1), async {
            while !session.is_cancelled() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_ok();
        if !missing_writer {
            assert_eq!(
                bus.receiver_count(),
                1,
                "writer must remain open while quote is in flight"
            );
        }
        let (released, wake) = &*release;
        *released.lock().unwrap() = true;
        wake.notify_all();
        cancelled
    };
    let (result, cancelled) = tokio::join!(biased; lifecycle, release_quote);
    assert!(
        cancelled,
        "supervisor must cancel and drain retained paper work even when writer is missing"
    );
    if missing_writer {
        assert!(result
            .expect_err("missing writer is explicit")
            .contains("writer handle is missing"));
        assert_eq!(*order.lock().unwrap(), ["quote_exit", "producer"]);
    } else {
        result.expect("signal shutdown");
        assert_eq!(*order.lock().unwrap(), ["quote_exit", "producer", "writer"]);
    }
    assert_eq!(*codes.lock().unwrap(), ["TEST_CODE_PAPER_MAIN_A"]);
    assert!(writer.is_none());
    assert!(session
        .close_and_drain()
        .await
        .expect("no remaining worker")
        .is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn paper_runtime_supervisor_joins_real_scan_before_writer_close() {
    if std::env::var(CHILD_CASE).as_deref() == Ok("signal") {
        child_case(false).await;
    } else {
        run_child("paper_scan_runtime_tests::paper_runtime_supervisor_joins_real_scan_before_writer_close", "signal");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn paper_runtime_missing_writer_still_drains_real_scan() {
    if std::env::var(CHILD_CASE).as_deref() == Ok("missing") {
        child_case(true).await;
    } else {
        run_child(
            "paper_scan_runtime_tests::paper_runtime_missing_writer_still_drains_real_scan",
            "missing",
        );
    }
}

#[test]
fn paper_runtime_both_phase_call_sites_use_shared_owned_session() {
    let source = include_str!("main.rs")
        .split_whitespace()
        .collect::<String>();
    assert_eq!(
        source
            .matches("paper_scans.scan(PaperScanPhase::Intraday,risk_context).await")
            .count(),
        1
    );
    assert_eq!(
        source
            .matches("paper_scans.scan(PaperScanPhase::PostClose,risk_context).await")
            .count(),
        1
    );
    assert!(source.contains("monitor_loop(&paper_scans)"));
    assert!(source.contains("observe_paper_scan_drain(paper_scans.close_and_drain().await)"));
    assert!(!source.contains("paper_sell::scan_and_sell("));
    assert!(!source.contains("paper_sell::scan_and_sell_post_close("));
}
