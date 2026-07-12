//! 交易执行抽象层（M2-1）。
//!
//! 目标：在不接入真实券商前，先统一模拟交易与未来实盘交易的接口边界。
//! 当前提供 SimulatedExecutionGateway（落库到 stock_position）。
//!
//! v12 PR3-3.5: 新增 paper_trade 模块, 虚拟腿只写 paper_trades, 零写 stock_position (BR-023).

pub mod paper_trade; // v12 PR3-3.5

use crate::database::DatabaseManager;
use crate::errors::TradeError;
use crate::models::{NewStockPosition, StockPosition};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 订单幂等性窗口（AGENTS §2.6 下单幂等性）
const DEDUP_WINDOW: Duration = Duration::from_secs(60);

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
}

#[derive(Debug, Clone)]
pub struct ClosePositionCmd {
    pub business_order_id: String,
    pub position_id: i32,
    pub code: String,
    pub trade_date: String,
    pub price: f64,
    pub quantity: i32,
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

pub struct SimulatedExecutionGateway {
    /// 修复 (2026-06-30 codex review): 60s 幂等性去重表 (AGENTS §2.6).
    /// key = business_order_id, value = 首次见到时刻.
    /// 同一 business_order_id 在 DEDUP_WINDOW 内重复提交会被拒绝.
    seen: Mutex<HashMap<String, Instant>>,
}

impl Default for SimulatedExecutionGateway {
    fn default() -> Self {
        Self::new()
    }
}

impl SimulatedExecutionGateway {
    pub fn new() -> Self {
        Self {
            seen: Mutex::new(HashMap::new()),
        }
    }

    fn db(&self) -> &'static DatabaseManager {
        DatabaseManager::get()
    }

    /// 检查并记录 business_order_id. 同 ID 在 DEDUP_WINDOW 内重复 → Err(TradeError::DuplicateOrder).
    /// 调用方: open_position / close_position / cancel_order 都应先调此方法.
    /// 修复 (2026-06-30 codex review): 之前返回 String, typed error
    ///   TradeError::DuplicateOrder 一直无人构造. 现在构造 typed error 再 to_string.
    fn dedup_check_and_record(&self, business_order_id: &str) -> Result<(), String> {
        let now = Instant::now();
        let mut seen = match self.seen.lock() {
            Ok(g) => g,
            Err(e) => {
                // 修复 (2026-06-30 codex review): 之前静默吞掉 poison 错误
                log::warn!("[SimulatedExecutionGateway] dedup mutex poisoned: {e}");
                return Err(format!("dedup mutex poisoned: {e}"));
            }
        };
        // Lazy GC: 删除过期条目
        seen.retain(|_, t| now.duration_since(*t) < DEDUP_WINDOW);
        if let Some(t) = seen.get(business_order_id) {
            let err = TradeError::DuplicateOrder {
                order_id: business_order_id.to_string(),
            };
            return Err(format!(
                "{} (within {}s window, first seen {:?} ago)",
                err,
                DEDUP_WINDOW.as_secs(),
                now.duration_since(*t)
            ));
        }
        seen.insert(business_order_id.to_string(), now);
        Ok(())
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
        self.dedup_check_and_record(&cmd.business_order_id)?;

        let new_position = NewStockPosition {
            code: cmd.code.clone(),
            name: cmd.name.clone(),
            buy_date: cmd.trade_date.clone(),
            buy_price: cmd.price,
            quantity: cmd.quantity,
            status: "open".to_string(),
            // v14.1 F7: 默认 None, 由 name LIKE 推断 (--backfill-st-type) 或 broker 推送
            st_type: None,
            // v14.1 BR-015: chain_name 缺省 None → store.rs 派生 "其他"
            //   真实来源待 chain registry / position_tracker.rs 接入
            chain_name: None,
        };

        match self.db().save_position(&new_position) {
            Ok(()) => Ok(OrderReceipt {
                business_order_id: cmd.business_order_id.clone(),
                side: OrderSide::Buy,
                status: OrderStatus::Filled,
                code: cmd.code.clone(),
                quantity: cmd.quantity,
                price: cmd.price,
                message: "simulated open position filled".to_string(),
            }),
            // 失败时回滚 dedup 记录, 允许调用方重试
            Err(e) => {
                // 修复 (2026-06-30 codex review): 之前静默吞 poison 错误, 现在 warn
                match self.seen.lock() {
                    Ok(mut seen) => {
                        seen.remove(&cmd.business_order_id);
                    }
                    Err(poison) => {
                        log::warn!("[SimulatedExecutionGateway] rollback mutex poisoned: {poison}")
                    }
                }
                Err(e.to_string())
            }
        }
    }

    fn close_position(&self, cmd: &ClosePositionCmd) -> Result<OrderReceipt, String> {
        self.dedup_check_and_record(&cmd.business_order_id)?;

        match self
            .db()
            .close_position(cmd.position_id, cmd.price, &cmd.trade_date)
        {
            Ok(()) => Ok(OrderReceipt {
                business_order_id: cmd.business_order_id.clone(),
                side: OrderSide::Sell,
                status: OrderStatus::Filled,
                code: cmd.code.clone(),
                quantity: cmd.quantity,
                price: cmd.price,
                message: "simulated close position filled".to_string(),
            }),
            Err(e) => {
                match self.seen.lock() {
                    Ok(mut seen) => {
                        seen.remove(&cmd.business_order_id);
                    }
                    Err(poison) => {
                        log::warn!("[SimulatedExecutionGateway] rollback mutex poisoned: {poison}")
                    }
                }
                Err(e.to_string())
            }
        }
    }

    fn cancel_order(&self, cmd: &CancelOrderCmd) -> Result<OrderReceipt, String> {
        // cancel 是幂等检查 (不落库), 同样走 dedup 防重复 cancel
        self.dedup_check_and_record(&cmd.business_order_id)?;

        Ok(OrderReceipt {
            business_order_id: cmd.business_order_id.clone(),
            side: OrderSide::Buy,
            status: OrderStatus::Rejected,
            code: cmd.code.clone(),
            quantity: 0,
            price: 0.0,
            message: "simulated gateway does not support pending-order cancel".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd_buy(id: &str) -> OpenPositionCmd {
        OpenPositionCmd {
            business_order_id: id.to_string(),
            code: "000001".to_string(),
            name: "测试股".to_string(),
            trade_date: "2026-06-30".to_string(),
            price: 10.0,
            quantity: 100,
        }
    }

    /// 测试辅助: 替换 dedup_window 让测试不依赖真实 60s.
    /// 实际使用 trait 内常量 DEDUP_WINDOW, 这里我们只验证"同 ID 重复拒绝".
    #[test]
    fn test_dedup_same_id_rejected() {
        let gw = SimulatedExecutionGateway::new();
        let id = "TEST-ORDER-001";
        // 第一次进入记录 (不调 db)
        assert!(gw.dedup_check_and_record(id).is_ok());
        // 第二次同 ID 应被拒绝
        let err = gw.dedup_check_and_record(id).unwrap_err();
        // 修复 (2026-06-30 codex review): 错误信息现在包含 TradeError::DuplicateOrder
        // 的 Display ("重复订单...") + dedup 上下文 ("within 60s window")
        assert!(
            err.contains("重复订单") && err.contains("within 60s window"),
            "expected typed TradeError::DuplicateOrder + dedup context, got: {err}"
        );
    }

    #[test]
    fn test_dedup_different_id_accepted() {
        let gw = SimulatedExecutionGateway::new();
        assert!(gw.dedup_check_and_record("ORDER-A").is_ok());
        assert!(gw.dedup_check_and_record("ORDER-B").is_ok());
        assert!(gw.dedup_check_and_record("ORDER-C").is_ok());
    }

    #[test]
    fn test_dedup_keeps_window_size_bounded() {
        // 验证 lazy GC 不会让 hashmap 无界增长 (此处测 retain 触发但不实际等 60s)
        let gw = SimulatedExecutionGateway::new();
        // 直接插 100 条不同 ID
        for i in 0..100 {
            assert!(gw.dedup_check_and_record(&format!("ORDER-{i}")).is_ok());
        }
        let size_before = gw.seen.lock().unwrap().len();
        assert_eq!(size_before, 100);

        // 模拟"所有条目过期": 把 seen 表里所有时间改到 2 分钟前
        {
            let mut seen = gw.seen.lock().unwrap();
            let past = Instant::now() - Duration::from_secs(120);
            for t in seen.values_mut() {
                *t = past;
            }
        }
        // 再插一条新 ID, retain 会清空过期条目
        assert!(gw.dedup_check_and_record("ORDER-NEW").is_ok());
        let size_after = gw.seen.lock().unwrap().len();
        assert_eq!(size_after, 1, "GC should drop all expired entries");
    }

    /// 验证 dedup_check_and_record 接口签名 (买/卖/撤消都应能调).
    /// 不实际调 db, 因为 db 需要 DatabaseManager::get() 全局状态.
    #[test]
    fn test_dedup_interface_signatures() {
        let gw = SimulatedExecutionGateway::new();
        // 接口是 &str, 返回 Result<(), String>
        let _: Result<(), String> = gw.dedup_check_and_record("X");
    }

    // 保留对命令结构的最小引用, 防止 unused import 警告
    #[allow(dead_code)]
    fn _cmd_silence() -> OpenPositionCmd {
        cmd_buy("X")
    }
}
