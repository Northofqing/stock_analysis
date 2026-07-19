//! 交易执行抽象层（M2-1）。
//!
//! 目标：在不接入真实券商前，先统一模拟交易与未来实盘交易的接口边界。
//! 当前提供 SimulatedExecutionGateway（落库到 stock_position）。
//!
//! v12 PR3-3.5: 新增 paper_trade 模块, 虚拟腿只写 paper_trades, 零写 stock_position (BR-023).

pub mod order_safety;
pub mod paper_engine; // v16.3 Commit 4a: 4 铁律接入 paper_trade 卖出
pub mod paper_trade; // v12 PR3-3.5
pub mod risk_adapter; // v16.3 Commit 1: pre-trade gate (4 项硬检查)

use crate::database::DatabaseManager;
use crate::errors::TradeError;
use crate::models::{NewStockPosition, StockPosition};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderStatus {
    Accepted,
    Filled,
    Rejected,
    Canceled,
}

#[derive(Debug, Clone)]
pub struct OrderReceipt {
    pub business_order_id: String,
    pub side: OrderSide,
    pub status: OrderStatus,
    pub code: String,
    pub quantity: i32,
    pub price: f64,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct OpenPositionCmd {
    pub business_order_id: String,
    pub code: String,
    pub name: String,
    pub trade_date: String,
    pub price: f64,
    pub quantity: i32,
    pub secondary_confirmed: bool,
    pub chain_name: String,
    pub decision_basis: String,
}

#[derive(Debug, Clone)]
pub struct ClosePositionCmd {
    pub business_order_id: String,
    pub position_id: i32,
    pub code: String,
    pub trade_date: String,
    pub price: f64,
    pub quantity: i32,
    pub secondary_confirmed: bool,
    pub decision_basis: String,
}

#[derive(Debug, Clone)]
pub struct CancelOrderCmd {
    pub business_order_id: String,
    pub code: String,
}

pub trait TradeExecutionGateway {
    fn get_open_position(&self, code: &str) -> Result<Option<StockPosition>, String>;
    fn update_position_return(
        &self,
        id: i32,
        current_price: f64,
        return_rate: f64,
    ) -> Result<(), String>;
    fn open_position(&self, cmd: &OpenPositionCmd) -> Result<OrderReceipt, String>;
    fn close_position(&self, cmd: &ClosePositionCmd) -> Result<OrderReceipt, String>;
    fn cancel_order(&self, cmd: &CancelOrderCmd) -> Result<OrderReceipt, String>;
}

pub struct SimulatedExecutionGateway;

struct RejectedAttemptAudit<'a> {
    business_order_id: &'a str,
    decision_basis: &'a str,
    side: &'a str,
    code: &'a str,
    requested_price: f64,
    quantity: i32,
}

impl Default for SimulatedExecutionGateway {
    fn default() -> Self {
        Self::new()
    }
}

impl SimulatedExecutionGateway {
    pub fn new() -> Self {
        Self
    }

    fn db(&self) -> &'static DatabaseManager {
        DatabaseManager::get()
    }

    /// 检查并记录 business_order_id. 同 ID 在 DEDUP_WINDOW 内重复 → Err(TradeError::DuplicateOrder).
    /// 调用方: open_position / close_position / cancel_order 都应先调此方法.
    /// 修复 (2026-06-30 codex review): 之前返回 String, typed error
    ///   TradeError::DuplicateOrder 一直无人构造. 现在构造 typed error 再 to_string.
    fn dedup_check_and_record(&self, business_order_id: &str) -> Result<(), String> {
        if !self.db().reserve_business_order_id(business_order_id)? {
            let err = TradeError::DuplicateOrder {
                order_id: business_order_id.to_string(),
            };
            return Err(format!("{err} (within persistent 60s window)"));
        }
        Ok(())
    }

    fn validate_requested_price(requested: f64, quote: f64) -> Result<(), String> {
        if !requested.is_finite() || requested <= 0.0 || !quote.is_finite() || quote <= 0.0 {
            return Err(format!(
                "BR-084 invalid requested/execution price: requested={requested} quote={quote}"
            ));
        }
        let deviation_pct = (quote - requested).abs() / requested * 100.0;
        if deviation_pct > *risk_adapter::MAX_SLIPPAGE_PCT {
            return Err(format!(
                "BR-084 execution quote deviation {deviation_pct:.2}% exceeds {:.2}%",
                *risk_adapter::MAX_SLIPPAGE_PCT
            ));
        }
        Ok(())
    }

    fn finalize_attempt(
        &self,
        attempt: Result<OrderReceipt, String>,
        context: RejectedAttemptAudit<'_>,
    ) -> Result<OrderReceipt, String> {
        match attempt {
            Ok(receipt) => Ok(receipt),
            Err(reason) => {
                let audit = crate::database::order_audit::OrderAuditRecord {
                    business_order_id: context.business_order_id,
                    source: "SimulatedExecutionGateway",
                    decision_basis: context.decision_basis,
                    side: context.side,
                    code: context.code,
                    requested_price: context.requested_price,
                    execution_price: None,
                    quantity: i64::from(context.quantity),
                    quote_observed_at: None,
                    outcome: "Rejected",
                    failure_reason: Some(&reason),
                };
                self.db()
                    .record_order_audit(&audit)
                    .map_err(|audit_error| {
                        format!("{reason}; BR-086 rejected-order audit failed: {audit_error}")
                    })?;
                Err(reason)
            }
        }
    }
}

impl TradeExecutionGateway for SimulatedExecutionGateway {
    fn get_open_position(&self, code: &str) -> Result<Option<StockPosition>, String> {
        self.db().get_open_position(code).map_err(|e| e.to_string())
    }

    fn update_position_return(
        &self,
        id: i32,
        current_price: f64,
        return_rate: f64,
    ) -> Result<(), String> {
        self.db()
            .update_position_return(id, current_price, return_rate)
            .map_err(|e| e.to_string())
    }

    fn open_position(&self, cmd: &OpenPositionCmd) -> Result<OrderReceipt, String> {
        let attempt = (|| {
            self.dedup_check_and_record(&cmd.business_order_id)?;

            let quote = crate::broker::execution_quote(&cmd.code)?;
            Self::validate_requested_price(cmd.price, quote.price)?;
            let (cash, _total, _position_pct) =
                paper_trade::portfolio_state(&cmd.code, quote.price)?;
            order_safety::validate(&order_safety::OrderSafetyInput {
                code: &cmd.code,
                side: order_safety::SafetySide::Buy,
                order_price: cmd.price,
                quantity: u64::try_from(cmd.quantity)
                    .map_err(|_| format!("BR-084 invalid quantity: {}", cmd.quantity))?,
                available_cash: Some(cash),
                limit_down_price: Some(quote.limit_down_price),
                limit_up_price: Some(quote.limit_up_price),
                secondary_confirmed: cmd.secondary_confirmed,
            })?;
            order_safety::validate(&order_safety::OrderSafetyInput {
                code: &cmd.code,
                side: order_safety::SafetySide::Buy,
                order_price: quote.price,
                quantity: u64::try_from(cmd.quantity)
                    .map_err(|_| format!("BR-084 invalid quantity: {}", cmd.quantity))?,
                available_cash: Some(cash),
                limit_down_price: Some(quote.limit_down_price),
                limit_up_price: Some(quote.limit_up_price),
                secondary_confirmed: cmd.secondary_confirmed,
            })?;
            if cmd.chain_name.trim().is_empty() || cmd.chain_name == "其他" {
                return Err(format!(
                    "BR-085 missing explicit chain classification for {}",
                    cmd.code
                ));
            }

            let new_position = NewStockPosition {
                code: cmd.code.clone(),
                name: cmd.name.clone(),
                buy_date: cmd.trade_date.clone(),
                buy_price: quote.price,
                quantity: cmd.quantity,
                status: "open".to_string(),
                // v14.1 F7: 默认 None, 由 name LIKE 推断 (--backfill-st-type) 或 broker 推送
                st_type: None,
                // BR-123: gateway 已要求明确 chain；仓储层仍保留 Option 缺失语义。
                chain_name: Some(cmd.chain_name.clone()),
            };

            let observed_at = quote.observed_at.to_rfc3339();
            let audit = crate::database::order_audit::OrderAuditRecord {
                business_order_id: &cmd.business_order_id,
                source: "SimulatedExecutionGateway",
                decision_basis: &cmd.decision_basis,
                side: "buy",
                code: &cmd.code,
                requested_price: cmd.price,
                execution_price: Some(quote.price),
                quantity: i64::from(cmd.quantity),
                quote_observed_at: Some(&observed_at),
                outcome: "Filled",
                failure_reason: None,
            };
            match self.db().save_position_with_audit(&new_position, &audit) {
                Ok(()) => Ok(OrderReceipt {
                    business_order_id: cmd.business_order_id.clone(),
                    side: OrderSide::Buy,
                    status: OrderStatus::Filled,
                    code: cmd.code.clone(),
                    quantity: cmd.quantity,
                    price: quote.price,
                    message: "simulated open position filled".to_string(),
                }),
                Err(e) => Err(e.to_string()),
            }
        })();
        self.finalize_attempt(
            attempt,
            RejectedAttemptAudit {
                business_order_id: &cmd.business_order_id,
                decision_basis: &cmd.decision_basis,
                side: "buy",
                code: &cmd.code,
                requested_price: cmd.price,
                quantity: cmd.quantity,
            },
        )
    }

    fn close_position(&self, cmd: &ClosePositionCmd) -> Result<OrderReceipt, String> {
        let attempt = (|| {
            self.dedup_check_and_record(&cmd.business_order_id)?;

            let quote = crate::broker::execution_quote(&cmd.code)?;
            Self::validate_requested_price(cmd.price, quote.price)?;
            let _account_state = paper_trade::portfolio_state(&cmd.code, quote.price)?;
            let position = self
                .db()
                .get_open_position(&cmd.code)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("BR-084 no open position for {}", cmd.code))?;
            if position.id != cmd.position_id || position.quantity != cmd.quantity {
                return Err(format!(
                "BR-084 close quantity/position mismatch: requested id={} qty={}, available id={} qty={}",
                cmd.position_id, cmd.quantity, position.id, position.quantity
            ));
            }
            order_safety::validate(&order_safety::OrderSafetyInput {
                code: &cmd.code,
                side: order_safety::SafetySide::Sell,
                order_price: cmd.price,
                quantity: u64::try_from(cmd.quantity)
                    .map_err(|_| format!("BR-084 invalid quantity: {}", cmd.quantity))?,
                available_cash: None,
                limit_down_price: Some(quote.limit_down_price),
                limit_up_price: Some(quote.limit_up_price),
                secondary_confirmed: cmd.secondary_confirmed,
            })?;
            order_safety::validate(&order_safety::OrderSafetyInput {
                code: &cmd.code,
                side: order_safety::SafetySide::Sell,
                order_price: quote.price,
                quantity: u64::try_from(cmd.quantity)
                    .map_err(|_| format!("BR-084 invalid quantity: {}", cmd.quantity))?,
                available_cash: None,
                limit_down_price: Some(quote.limit_down_price),
                limit_up_price: Some(quote.limit_up_price),
                secondary_confirmed: cmd.secondary_confirmed,
            })?;

            let observed_at = quote.observed_at.to_rfc3339();
            let audit = crate::database::order_audit::OrderAuditRecord {
                business_order_id: &cmd.business_order_id,
                source: "SimulatedExecutionGateway",
                decision_basis: &cmd.decision_basis,
                side: "sell",
                code: &cmd.code,
                requested_price: cmd.price,
                execution_price: Some(quote.price),
                quantity: i64::from(cmd.quantity),
                quote_observed_at: Some(&observed_at),
                outcome: "Filled",
                failure_reason: None,
            };
            match self.db().close_position_with_audit(
                cmd.position_id,
                &cmd.code,
                quote.price,
                &cmd.trade_date,
                &audit,
            ) {
                Ok(()) => Ok(OrderReceipt {
                    business_order_id: cmd.business_order_id.clone(),
                    side: OrderSide::Sell,
                    status: OrderStatus::Filled,
                    code: cmd.code.clone(),
                    quantity: cmd.quantity,
                    price: quote.price,
                    message: "simulated close position filled".to_string(),
                }),
                Err(e) => Err(e.to_string()),
            }
        })();
        self.finalize_attempt(
            attempt,
            RejectedAttemptAudit {
                business_order_id: &cmd.business_order_id,
                decision_basis: &cmd.decision_basis,
                side: "sell",
                code: &cmd.code,
                requested_price: cmd.price,
                quantity: cmd.quantity,
            },
        )
    }

    fn cancel_order(&self, cmd: &CancelOrderCmd) -> Result<OrderReceipt, String> {
        let attempt = (|| {
            self.dedup_check_and_record(&cmd.business_order_id)?;
            Err("simulated gateway does not support pending-order cancel".to_string())
        })();
        self.finalize_attempt(
            attempt,
            RejectedAttemptAudit {
                business_order_id: &cmd.business_order_id,
                decision_basis: "cancel requested",
                side: "cancel",
                code: &cmd.code,
                requested_price: 0.0,
                quantity: 0,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diesel::prelude::*;

    fn init_test_db() {
        DatabaseManager::init(None).expect("test database init");
    }

    fn unique_id(label: &str) -> String {
        format!(
            "TEST_ORDER_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }

    fn unique_code(label: &str) -> String {
        unique_id(label).replace("TEST_ORDER", "TEST_CODE")
    }

    struct TestLedgerGuard {
        date: String,
    }

    impl Drop for TestLedgerGuard {
        fn drop(&mut self) {
            if let Ok(mut conn) = DatabaseManager::get().get_conn() {
                let _ = diesel::sql_query("DELETE FROM ledger WHERE date = ?")
                    .bind::<diesel::sql_types::Text, _>(&self.date)
                    .execute(&mut conn);
            }
        }
    }

    fn prepare_fresh_account_state() -> TestLedgerGuard {
        init_test_db();
        crate::broker::ensure_test_quote_provider();
        let today = chrono::Local::now().date_naive().to_string();
        let mut conn = DatabaseManager::get()
            .get_conn()
            .expect("test database connection");
        diesel::sql_query(
            "INSERT INTO ledger (date, total_value, cash, market_value, daily_pnl, created_at)
             VALUES (?, 100000.0, 100000.0, 0.0, 0.0, CURRENT_TIMESTAMP)
             ON CONFLICT(date) DO UPDATE SET
                 total_value = excluded.total_value,
                 cash = excluded.cash,
                 market_value = excluded.market_value,
                 daily_pnl = excluded.daily_pnl,
                 created_at = CURRENT_TIMESTAMP",
        )
        .bind::<diesel::sql_types::Text, _>(&today)
        .execute(&mut conn)
        .expect("prepare same-day test ledger");
        diesel::sql_query(
            "UPDATE stock_position SET updated_at = CURRENT_TIMESTAMP WHERE status = 'open'",
        )
        .execute(&mut conn)
        .expect("refresh isolated test-position evidence");
        TestLedgerGuard { date: today }
    }

    fn audit_outcome(business_order_id: &str) -> String {
        #[derive(diesel::QueryableByName)]
        struct AuditOutcome {
            #[diesel(sql_type = diesel::sql_types::Text)]
            outcome: String,
        }

        let mut conn = DatabaseManager::get()
            .get_conn()
            .expect("test database connection");
        diesel::sql_query(
            "SELECT outcome FROM order_audit WHERE business_order_id = ? ORDER BY id DESC LIMIT 1",
        )
        .bind::<diesel::sql_types::Text, _>(business_order_id)
        .get_result::<AuditOutcome>(&mut conn)
        .expect("audited gateway attempt")
        .outcome
    }

    fn cmd_buy(id: &str) -> OpenPositionCmd {
        OpenPositionCmd {
            business_order_id: id.to_string(),
            code: "TEST_CODE_000001".to_string(),
            name: "测试股".to_string(),
            trade_date: "2026-06-30".to_string(),
            price: 10.0,
            quantity: 100,
            secondary_confirmed: false,
            chain_name: "测试产业链".to_string(),
            decision_basis: "测试决策".to_string(),
        }
    }

    /// 测试辅助: 替换 dedup_window 让测试不依赖真实 60s.
    /// 实际使用 trait 内常量 DEDUP_WINDOW, 这里我们只验证"同 ID 重复拒绝".
    #[test]
    #[serial_test::serial]
    fn test_dedup_same_id_rejected() {
        init_test_db();
        let gw = SimulatedExecutionGateway::new();
        let id = unique_id("SAME");
        assert!(gw.dedup_check_and_record(&id).is_ok());
        // 第二次同 ID 应被拒绝
        let err = gw.dedup_check_and_record(&id).unwrap_err();
        // 修复 (2026-06-30 codex review): 错误信息现在包含 TradeError::DuplicateOrder
        // 的 Display ("重复订单...") + dedup 上下文 ("within 60s window")
        assert!(
            err.contains("重复订单") && err.contains("persistent 60s window"),
            "expected typed TradeError::DuplicateOrder + dedup context, got: {err}"
        );
    }

    #[test]
    #[serial_test::serial]
    fn test_dedup_different_id_accepted() {
        init_test_db();
        let gw = SimulatedExecutionGateway::new();
        assert!(gw.dedup_check_and_record(&unique_id("A")).is_ok());
        assert!(gw.dedup_check_and_record(&unique_id("B")).is_ok());
        assert!(gw.dedup_check_and_record(&unique_id("C")).is_ok());
    }

    /// 验证 dedup_check_and_record 接口签名 (买/卖/撤消都应能调).
    /// 不实际调 db, 因为 db 需要 DatabaseManager::get() 全局状态.
    #[test]
    #[serial_test::serial]
    fn test_dedup_interface_signatures() {
        init_test_db();
        let gw = SimulatedExecutionGateway::new();
        // 接口是 &str, 返回 Result<(), String>
        let _: Result<(), String> = gw.dedup_check_and_record(&unique_id("SIGNATURE"));
    }

    #[test]
    fn test_requested_price_must_be_valid_and_within_slippage() {
        assert!(SimulatedExecutionGateway::validate_requested_price(10.0, 10.1).is_ok());
        assert!(SimulatedExecutionGateway::validate_requested_price(0.0, 10.0).is_err());
        assert!(SimulatedExecutionGateway::validate_requested_price(10.0, 10.3).is_err());
    }

    #[test]
    #[serial_test::serial]
    fn gateway_fills_open_and_close_with_atomic_audit() {
        let _ledger = prepare_fresh_account_state();
        let gateway = SimulatedExecutionGateway::new();
        let code = unique_code("ROUND_TRIP");
        let open_id = unique_id("OPEN_FILL");
        let mut open = cmd_buy(&open_id);
        open.code.clone_from(&code);

        let opened = gateway.open_position(&open).expect("safe open fills");
        assert_eq!(opened.business_order_id, open_id);
        assert_eq!(opened.side, OrderSide::Buy);
        assert_eq!(opened.status, OrderStatus::Filled);
        assert_eq!(opened.code, code);
        assert_eq!(opened.quantity, 100);
        assert_eq!(opened.price, 10.0);
        assert!(opened.message.contains("filled"));
        assert_eq!(audit_outcome(&open_id), "Filled");

        let position = gateway
            .get_open_position(&code)
            .expect("read open position")
            .expect("position exists");
        gateway
            .update_position_return(position.id, 10.5, 5.0)
            .expect("update return evidence");

        let _close_ledger = prepare_fresh_account_state();
        let close_id = unique_id("CLOSE_FILL");
        let closed = gateway
            .close_position(&ClosePositionCmd {
                business_order_id: close_id.clone(),
                position_id: position.id,
                code: code.clone(),
                trade_date: chrono::Local::now().date_naive().to_string(),
                price: 10.0,
                quantity: 100,
                secondary_confirmed: false,
                decision_basis: "TEST_CODE risk exit".to_string(),
            })
            .expect("safe close fills");
        assert_eq!(closed.side, OrderSide::Sell);
        assert_eq!(closed.status, OrderStatus::Filled);
        assert_eq!(closed.price, 10.0);
        assert_eq!(audit_outcome(&close_id), "Filled");
        assert!(gateway
            .get_open_position(&code)
            .expect("read closed position")
            .is_none());
    }

    #[test]
    #[serial_test::serial]
    fn gateway_rejections_are_explicit_and_audited() {
        let _ledger = prepare_fresh_account_state();
        let gateway = SimulatedExecutionGateway::new();

        let missing_chain_id = unique_id("MISSING_CHAIN");
        let mut missing_chain = cmd_buy(&missing_chain_id);
        missing_chain.code = unique_code("MISSING_CHAIN");
        missing_chain.chain_name = "其他".to_string();
        let error = gateway.open_position(&missing_chain).unwrap_err();
        assert!(error.contains("BR-085"));
        assert_eq!(audit_outcome(&missing_chain_id), "Rejected");

        let bad_quantity_id = unique_id("BAD_QUANTITY");
        let mut bad_quantity = cmd_buy(&bad_quantity_id);
        bad_quantity.code = unique_code("BAD_QUANTITY");
        bad_quantity.quantity = 99;
        let error = gateway.open_position(&bad_quantity).unwrap_err();
        assert!(error.contains("divisible by 100"));
        assert_eq!(audit_outcome(&bad_quantity_id), "Rejected");

        let stale_request_id = unique_id("PRICE_DEVIATION");
        let mut stale_request = cmd_buy(&stale_request_id);
        stale_request.code = unique_code("PRICE_DEVIATION");
        stale_request.price = 10.3;
        let error = gateway.open_position(&stale_request).unwrap_err();
        assert!(error.contains("deviation"));
        assert_eq!(audit_outcome(&stale_request_id), "Rejected");

        let missing_position_id = unique_id("NO_POSITION");
        let missing_code = unique_code("NO_POSITION");
        let error = gateway
            .close_position(&ClosePositionCmd {
                business_order_id: missing_position_id.clone(),
                position_id: i32::MAX,
                code: missing_code,
                trade_date: chrono::Local::now().date_naive().to_string(),
                price: 10.0,
                quantity: 100,
                secondary_confirmed: false,
                decision_basis: "TEST_CODE missing position".to_string(),
            })
            .unwrap_err();
        assert!(error.contains("no open position"));
        assert_eq!(audit_outcome(&missing_position_id), "Rejected");

        let cancel_id = unique_id("CANCEL");
        let error = gateway
            .cancel_order(&CancelOrderCmd {
                business_order_id: cancel_id.clone(),
                code: unique_code("CANCEL"),
            })
            .unwrap_err();
        assert!(error.contains("does not support"));
        assert_eq!(audit_outcome(&cancel_id), "Rejected");
    }

    // 保留对命令结构的最小引用, 防止 unused import 警告
    #[allow(dead_code)]
    fn _cmd_silence() -> OpenPositionCmd {
        cmd_buy("X")
    }
}
