use super::*;
use std::sync::Mutex;

/// External persistence fixture; the real simulate/risk/evaluate/audit code is
/// shared with production. No DatabaseManager singleton is initialized.
pub(crate) struct MemoryPaperTradeStore {
    conn: Mutex<SqliteConnection>,
    cancel_on_checkout: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl MemoryPaperTradeStore {
    pub(crate) fn new() -> Self {
        let mut conn = SqliteConnection::establish(":memory:").expect("private SQLite");
        DatabaseManager::run_migrations_for_test(&mut conn).expect("private schema");
        Self {
            conn: Mutex::new(conn),
            cancel_on_checkout: None,
        }
    }

    pub(crate) fn sold_codes(&self) -> Vec<String> {
        #[derive(QueryableByName)]
        struct Row {
            #[diesel(sql_type = diesel::sql_types::Text)]
            code: String,
        }
        diesel::sql_query(
            "SELECT code FROM paper_trades WHERE direction='sell' AND status='Filled' ORDER BY id",
        )
        .load::<Row>(&mut *self.conn.lock().unwrap())
        .expect("private filled sales")
        .into_iter()
        .map(|row| row.code)
        .collect()
    }

    pub(crate) fn already_sold(&self, code: &str, today: &str) -> Result<bool, String> {
        #[derive(QueryableByName)]
        struct Row {
            #[diesel(sql_type = diesel::sql_types::BigInt)]
            n: i64,
        }
        diesel::sql_query("SELECT COUNT(*) AS n FROM paper_trades WHERE code=? AND direction='sell' AND status='Filled' AND date(ts)=?")
            .bind::<diesel::sql_types::Text, _>(code).bind::<diesel::sql_types::Text, _>(today)
            .get_result::<Row>(&mut *self.conn.lock().unwrap()).map(|row| row.n > 0).map_err(|error| error.to_string())
    }
}

impl PaperTradeStore for MemoryPaperTradeStore {
    fn reserve(&self, plan_id: &str) -> Result<bool, String> {
        assert!(plan_id.contains("TEST_CODE_"));
        diesel::sql_query("INSERT INTO order_idempotency (business_order_id, reserved_at) VALUES (?, CURRENT_TIMESTAMP) ON CONFLICT(business_order_id) DO UPDATE SET reserved_at=CURRENT_TIMESTAMP WHERE order_idempotency.reserved_at <= datetime('now', '-60 seconds')")
            .bind::<diesel::sql_types::Text, _>(plan_id)
            .execute(&mut *self.conn.lock().unwrap()).map(|rows| rows == 1).map_err(|error| error.to_string())
    }

    fn record_audit(
        &self,
        record: &crate::database::order_audit::OrderAuditRecord<'_>,
    ) -> Result<(), String> {
        crate::database::order_audit::insert_order_audit_query(
            &mut self.conn.lock().unwrap(),
            record,
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
    }

    fn persist(
        &self,
        sql: &str,
        signal: &PaperSignal,
        result: &PaperResult,
        observed_at: &str,
        evidence: Option<&PaperAuditEvidence>,
        cancelled: &std::sync::atomic::AtomicBool,
    ) -> Result<(usize, Option<PaperTradePersistenceReceipt>), String> {
        assert!(signal.code.starts_with("TEST_CODE_"));
        let mut conn = self.conn.lock().unwrap();
        // The external checkout completed while shutdown was requested.
        // Continue into the shared real transaction, never a fake simulate.
        if let Some(cancelled) = &self.cancel_on_checkout {
            cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        persist_paper_trade_if_active(
            &mut conn,
            sql,
            signal,
            result,
            observed_at,
            evidence,
            cancelled,
        )
    }
}

#[test]
fn paper_runtime_checkout_cancel_prevents_real_terminal_transaction() {
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut store = MemoryPaperTradeStore::new();
    store.cancel_on_checkout = Some(std::sync::Arc::clone(&cancelled));
    let signal = PaperSignal {
        plan_id: "TEST_CODE_PAPER_CHECKOUT_PLAN".to_owned(),
        code: "TEST_CODE_PAPER_CHECKOUT".to_owned(),
        name: "TEST_CODE checkout".to_owned(),
        direction: Direction::Sell,
        price: 8.0,
        quantity: 100,
        virtual_reason: "TEST_CODE real sell".to_owned(),
        is_limit_up: false,
        is_limit_down: false,
        is_suspended: false,
        limit_up_price: Some(12.0),
        limit_down_price: Some(6.0),
        secondary_confirmed: false,
        quote_observed_at: chrono::Utc::now(),
        risk_context: PaperRiskContext::new(
            crate::risk::action_gate::AccountMode::Normal,
            crate::monitor::data_mode::DataMode::Full,
        ),
    };
    let evidence = PaperAuditEvidence::new("TEST_CODE checkout evidence").unwrap();
    let result = simulate_with_audit_evidence_controlled(
        &signal,
        8.0,
        50_000.0,
        100_000.0,
        1.0,
        &evidence,
        Some(&store),
        &cancelled,
    );
    let error = result
        .expect_err("cancellation during checkout must prevent the real terminal transaction");
    assert!(error.contains("cancelled"), "{error}");
    assert!(
        store.sold_codes().is_empty(),
        "no Filled sale may appear after checkout cancellation"
    );
}

struct ControlledValuationReads {
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    cancel_first: bool,
    fail_first: bool,
    requests: Mutex<Vec<String>>,
}

impl ValuationReads for ControlledValuationReads {
    fn quote_price(&self, code: &str) -> Result<f64, String> {
        self.requests.lock().unwrap().push(format!("quote:{code}"));
        match code {
            "TEST_CODE_VALUATION_A" => {
                if self.cancel_first {
                    self.cancelled
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                }
                if self.fail_first {
                    Err("TEST_CODE missing realtime quote".to_owned())
                } else {
                    Ok(10.0)
                }
            }
            "TEST_CODE_VALUATION_B" => Ok(20.0),
            _ => panic!("unexpected external valuation request {code}"),
        }
    }

    fn daily_close(&self, code: &str) -> Result<Option<f64>, String> {
        assert_eq!(code, "TEST_CODE_VALUATION_A");
        self.requests.lock().unwrap().push(format!("daily:{code}"));
        Ok(Some(12.0))
    }
}

fn valuation_fixture() -> (
    crate::database::user_position_snapshot::UserPositionSnapshot,
    SqliteConnection,
    NaiveDate,
) {
    let today = NaiveDate::from_ymd_opt(2026, 9, 14).unwrap();
    let at = chrono::DateTime::parse_from_rfc3339("2026-09-13T10:00:00+08:00").unwrap();
    let snapshot = crate::database::user_position_snapshot::UserPositionSnapshot {
        snapshot_row_id: 1,
        snapshot_id: "TEST_CODE_VALUATION_SNAPSHOT".to_owned(),
        effective_at: at,
        confirmed_at: at,
        source: "TEST_CODE valuation".to_owned(),
        confirm_empty: false,
        evidence_sha256: "TEST_CODE_VALUATION_EVIDENCE".to_owned(),
        items: [("A", 100), ("B", 200)]
            .into_iter()
            .map(|(suffix, quantity)| {
                crate::portfolio::user_position_snapshot::UserPositionItemInput {
                    code: format!("TEST_CODE_VALUATION_{suffix}"),
                    name: suffix.to_owned(),
                    quantity,
                    cost_price: 5.0,
                }
            })
            .collect(),
    };
    let mut conn = SqliteConnection::establish(":memory:").unwrap();
    diesel::sql_query("CREATE TABLE ledger (date TEXT PRIMARY KEY, total_value REAL NOT NULL)")
        .execute(&mut conn)
        .unwrap();
    diesel::sql_query("INSERT INTO ledger(date, total_value) VALUES ('2026-09-13', 53000)")
        .execute(&mut conn)
        .unwrap();
    (snapshot, conn, today)
}

#[test]
fn paper_runtime_nested_valuation_cancel_stops_fallback_and_remaining_positions() {
    // Test both the failed-quote fallback seam and the successful-quote
    // next-security seam without replacing the actual valuation algorithm.
    for fail_first in [true, false] {
        let (snapshot, mut conn, today) = valuation_fixture();
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reads = ControlledValuationReads {
            cancelled: std::sync::Arc::clone(&cancelled),
            cancel_first: true,
            fail_first,
            requests: Mutex::new(Vec::new()),
        };
        let error = estimate_ledger_from_snapshot_with_reads(
            &snapshot, &mut conn, 50_000.0, today, &cancelled, &reads,
        )
        .expect_err("nested valuation must stop after shutdown");
        assert!(error.contains("cancelled"), "{error}");
        assert_eq!(
            *reads.requests.lock().unwrap(),
            ["quote:TEST_CODE_VALUATION_A"],
            "cancellation must prevent both fallback and next-security requests"
        );
    }
}

#[test]
fn paper_runtime_normal_nested_valuation_preserves_prices_fallback_and_daily_pnl() {
    for fail_first in [false, true] {
        let (snapshot, mut conn, today) = valuation_fixture();
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reads = ControlledValuationReads {
            cancelled: std::sync::Arc::clone(&cancelled),
            cancel_first: false,
            fail_first,
            requests: Mutex::new(Vec::new()),
        };
        let result = estimate_ledger_from_snapshot_with_reads(
            &snapshot, &mut conn, 50_000.0, today, &cancelled, &reads,
        )
        .expect("real nested valuation");
        if fail_first {
            assert_eq!(result, (55_200.0, 50_000.0, 5_200.0, 2_200.0));
            assert_eq!(
                *reads.requests.lock().unwrap(),
                [
                    "quote:TEST_CODE_VALUATION_A",
                    "daily:TEST_CODE_VALUATION_A",
                    "quote:TEST_CODE_VALUATION_B"
                ]
            );
        } else {
            assert_eq!(result, (55_000.0, 50_000.0, 5_000.0, 2_000.0));
            assert_eq!(
                *reads.requests.lock().unwrap(),
                ["quote:TEST_CODE_VALUATION_A", "quote:TEST_CODE_VALUATION_B"]
            );
        }
    }
}
