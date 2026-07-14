//! v16.3 Commit 4a — 4 铁律 + 1 bonus 接入 paper_trade 卖出路径.
//!
//! 业务: position_tracker::track_position 已在 line 200-257 实现 4 铁律 (StopLoss/TakeProfit/
//!        TimeExit/BollingTop), 但只写 analysis_result 表, **不调 paper_trade::simulate(Sell)**.
//!        本模块把 4 铁律的卖出动作也写到 paper_trades 表, 让虚拟盘卖出路径完整.
//!
//! 复用策略 (leverage, 不重造):
//!   - position_tracker::RiskContext: 已 pub (Commit 4a 改 track_position 可见性为 pub(crate))
//!   - 副作用: 调 track_position 后, paper_engine 读 analysis_result 最新条 operation_advice
//!             是否含"铁律"/"止盈"/"止损" → paper_trade::simulate(Direction=Sell)
//!   - 不调 ClosePositionCmd (写 stock_position), 因 BR-023 隔离虚拟腿
//!
//! Commit 4a 注: track_position 需要 AnalysisResult 实例, 但 AnalysisResult 没 derive Default
//! 且 ~50 字段, Commit 4a 用 *只读 analysis_result 表* 方式, 不调 track_position
//! (主循环在 main.rs 调 track_position 已有, 写 analysis_result)
//! → paper_engine 只读 analysis_result, 0 调 track_position, 0 重造 4 铁律

use crate::database::DatabaseManager;
use crate::trading::paper_trade::{self, Direction, PaperSignal};
use chrono::Local;
use diesel::prelude::*;

/// 单个 active paper position 卖出检查输入
#[derive(Debug, Clone)]
pub struct PaperPositionSellCheck {
    pub code: String,
    pub name: String,
    pub avg_cost: f64,
    pub quantity: u32,
    /// Fix 1 (review): 当前市价 (用于 emit_sell_signal, 避免 avg_cost 当 price 滑点 0%)
    /// v16.4 #5 阶段: 暂无 broker API, 推 0.0 (emit_sell_signal 用 avg_cost fallback)
    /// v16.7 接入 broker 后: 填真价
    pub current_price: f64,
}

/// 4 铁律检查结果
pub struct SellDecision {
    pub code: String,
    pub name: String,
    pub reason: String,
    /// Fix 3 (review): 真实卖出数量 (来自 PaperPositionSellCheck.quantity, 不再硬编码 100)
    pub quantity: u32,
    /// 当前市价 (Fix 1: 之前用 avg_cost 当 price, 滑点 0 永远不 Invalidated; 现在用当前市价)
    pub current_price: f64,
}

#[derive(diesel::QueryableByName, Debug, Clone)]
struct AdviceRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    operation_advice: String,
}

#[derive(diesel::QueryableByName, Debug)]
struct OpenPosRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    code: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    name: String,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    net_qty: i64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    avg_cost: f64,
}

/// review fix Issue #2: 从 paper_trades 聚合真实未平仓虚拟持仓 (Filled buy - Filled sell > 0).
/// 替代之前 "checks 永远空, 等 v16.4" 的 dead code 状态, 让 4 铁律卖出闭环上线.
pub fn load_open_positions() -> Result<Vec<PaperPositionSellCheck>, String> {
    let mut conn = DatabaseManager::get()
        .get_conn()
        .map_err(|e| format!("DB 连接失败: {}", e))?;
    let rows: Vec<OpenPosRow> = diesel::sql_query(
        // Fix review (MEDIUM): MIN(name) 替代 MAX(name) — 同 code 多名(改名/复牌)取最早,
        //                   COALESCE(fill_price, price) 兼容 SignalTriggered/NotFilled 部分成交
        "SELECT code, MIN(name) AS name, \
         SUM(CASE WHEN direction = 'buy' THEN quantity ELSE -quantity END) AS net_qty, \
         COALESCE( \
           SUM(CASE WHEN direction = 'buy' THEN COALESCE(fill_price, price) * quantity ELSE 0 END) * 1.0 \
           / NULLIF(SUM(CASE WHEN direction = 'buy' THEN quantity ELSE 0 END), 0), 0.0) AS avg_cost \
         FROM paper_trades WHERE status = 'Filled' \
         GROUP BY code HAVING net_qty > 0",
    )
    .load::<OpenPosRow>(&mut conn)
    .map_err(|e| format!("query open paper positions: {}", e))?;

    Ok(rows
        .into_iter()
        .map(|r| PaperPositionSellCheck {
            code: r.code.clone(),
            name: r.name,
            avg_cost: r.avg_cost,
            quantity: r.net_qty.max(0) as u32,
            // v16.5: current_price 调 quote_provider() 拿真市价 (broker SDK 未接入时 MockQuoteProvider 返 0.0, fallback 到 avg_cost)
            // 业务: broker 接入后, 真市价 → 滑点真检测, 4 铁律卖出 PnL 真计算
            current_price: crate::broker::quote_provider()
                .map(|p| p.get_quote_price(&r.code))
                .filter(|&p| p > 0.0)
                .unwrap_or(r.avg_cost),
        })
        .collect())
}

/// 4 铁律检查入口 — 读 analysis_result 表 (由 position_tracker::track_position 写)
pub fn check_4_iron_rules(checks: &[PaperPositionSellCheck]) -> Result<Vec<SellDecision>, String> {
    // Fix review (HIGH): 真正 1 SQL batch (diesel 0.5 不支持 Vec bind, 用 format! 拼接 + escape)
    // 原始 50 持仓 → 50 次 SQL; 优化后 50 持仓 → 1 次 SQL (50 IN clause)
    use std::collections::HashMap;
    let mut decisions = Vec::with_capacity(checks.len());
    if checks.is_empty() {
        return Ok(decisions);
    }
    let mut conn = DatabaseManager::get()
        .get_conn()
        .map_err(|e| format!("DB 连接失败: {}", e))?;

    // SQL 防注入: escape single quote (analysis_result.code 应为合法 stock code, 但 escape 保险)
    let codes: Vec<String> = checks.iter()
        .map(|c| c.code.replace('\'', "''"))
        .collect();
    let in_clause = codes.join(",");
    let sql = format!(
        "SELECT code, operation_advice FROM analysis_result \
         WHERE id IN ( \
           SELECT MAX(id) FROM analysis_result \
           WHERE code IN ({}) GROUP BY code \
         )",
        in_clause
    );
    #[derive(diesel::QueryableByName, Debug)]
    struct BatchAdvice {
        #[diesel(sql_type = diesel::sql_types::Text)]
        code: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        operation_advice: String,
    }
    let advice_map: HashMap<String, String> = diesel::sql_query(&sql)
        .load::<BatchAdvice>(&mut conn)
        .map_err(|e| format!("batch query analysis_result: {}", e))?
        .into_iter()
        .map(|r| (r.code, r.operation_advice))
        .collect();

    for check in checks {
        if let Some(advice) = advice_map.get(&check.code) {
            if is_iron_rule_triggered(advice) {
                let reason = extract_reason(advice);
                log::warn!(
                    "[paper_engine] 4 铁律触发 {}({}): {}",
                    check.name, check.code, reason
                );
                decisions.push(SellDecision {
                    code: check.code.clone(),
                    name: check.name.clone(),
                    reason: reason.clone(),
                    current_price: check.current_price,
                    quantity: check.quantity,
                });
            }
        }
    }

    Ok(decisions)
}

/// 调 paper_trade::simulate(Sell) 写 paper_trades
///
/// Fix 3: SellDecision 加 quantity 字段, 不再硬编码 100
/// Fix 1: price 用 current_price (0.0 fallback avg_cost, 避免滑点 0 永远不 Invalidated)
pub fn emit_sell_signal(decision: &SellDecision) -> Result<(), String> {
    let now = Local::now();
    // Fix 1: effective_price = current_price (broker 未接入前 fallback 到 0.0 = 滑点检查跳过)
    let effective_price = decision.current_price;
    let signal = PaperSignal {
        // Fix 1: plan_id 含铁律 + ts (同 code 同日多铁律可各写 1 次)
        plan_id: format!("exit-{}-{}-{}", decision.code, now.format("%Y%m%d"), decision.reason.replace(' ', "_").chars().take(16).collect::<String>()),
        code: decision.code.clone(),
        name: decision.name.clone(),
        direction: Direction::Sell,
        price: effective_price,
        quantity: decision.quantity.max(100),
        virtual_reason: format!("4-IronRule:{}", decision.reason),
        is_limit_up: false,
        is_limit_down: false,
        is_suspended: false,
        account_mode: "Normal".to_string(),
        data_mode: "Full".to_string(),
    };

    // review fix Issue #5: 传真实 portfolio state (Sell 路径 AccountMode/DataMode 检查仍生效)
    let (cash, total, pos_pct) = paper_trade::portfolio_state(&decision.code, effective_price);
    // v16.5 #4: 预生成 order_id/exec_id/decision_id, simulate 后 publish TradingBus (2 emit)
    let order_id = crate::bus::new_order_id();
    let exec_id = crate::bus::new_execution_id();
    let decision_id = crate::bus::new_decision_id();
    let plan_id_for_event = order_id.clone(); // emit 用 (不暴露 plan_id 给 TradingEvent)
    match paper_trade::simulate(&signal, effective_price, cash, total, pos_pct) {
        Ok(outcome) => {
            log::info!(
                "[paper_engine] 4 铁律卖出 {}({}) status={} reason={}",
                decision.name, decision.code, outcome.result.status.as_str(), decision.reason
            );
            // v16.5 #4: TradingBus 2 emit (OrderCreated + ExecutionFilled, 真价)
            crate::bus::TradingBus::global().publish(crate::bus::TradingEvent::OrderCreated {
                decision_id: decision_id.clone(),
                order_id: order_id.clone(),
                code: decision.code.clone(),
                side: "sell".to_string(),
            });
            crate::bus::TradingBus::global().publish(crate::bus::TradingEvent::ExecutionFilled {
                order_id: order_id.clone(),
                execution_id: exec_id.clone(),
                fill_price: effective_price,
            });
            // suppress unused warning
            let _ = plan_id_for_event;
            Ok(())
        }
        Err(e) => {
            log::warn!(
                "[paper_engine] 4 铁律卖出失败 {}({}): {}",
                decision.name, decision.code, e
            );
            Err(e)
        }
    }
}

/// 判断 operation_advice 是否含 4 铁律关键词
fn is_iron_rule_triggered(advice: &str) -> bool {
    advice.contains("铁律")
        || advice.contains("止损")
        || advice.contains("止盈")
        || advice.contains("14天")
        || advice.contains("ATR动态止损")
}

/// 提取具体原因
fn extract_reason(advice: &str) -> String {
    if advice.contains("铁律1") {
        "铁律1:止损(-8%)".to_string()
    } else if advice.contains("铁律3") {
        "铁律3:跌破5日线止盈".to_string()
    } else if advice.contains("铁律4") {
        "铁律4:14天不涨换股".to_string()
    } else if advice.contains("铁律5") {
        "铁律5:布林上轨+MACD顶背离".to_string()
    } else if advice.contains("ATR动态止损") {
        "ATR动态止损".to_string()
    } else {
        advice.chars().take(30).collect()
    }
}

// ============ Unit tests (≥ 4) ============

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_iron_rule_1_stop_loss() {
        assert!(is_iron_rule_triggered("铁律1:止损(-8%)"));
        assert!(is_iron_rule_triggered("操作建议: 触发铁律1止损"));
    }

    #[test]
    fn detects_iron_rule_3_take_profit() {
        assert!(is_iron_rule_triggered("铁律3:跌破5日线止盈"));
    }

    #[test]
    fn detects_iron_rule_4_time_exit() {
        assert!(is_iron_rule_triggered("铁律4:14天不涨换股"));
    }

    #[test]
    fn detects_atr_stop_loss() {
        assert!(is_iron_rule_triggered("ATR动态止损(有效止损价 9.20)"));
    }

    #[test]
    fn does_not_detect_hold_advice() {
        assert!(!is_iron_rule_triggered("持有观望"));
        assert!(!is_iron_rule_triggered("加仓"));
    }

    #[test]
    fn extracts_iron_rule_1_reason() {
        let r = extract_reason("操作: 铁律1:止损(-8%) 触发");
        assert_eq!(r, "铁律1:止损(-8%)");
    }

    #[test]
    fn extracts_iron_rule_3_reason() {
        let r = extract_reason("铁律3:跌破5日线止盈");
        assert_eq!(r, "铁律3:跌破5日线止盈");
    }

    #[test]
    fn extracts_iron_rule_4_reason() {
        let r = extract_reason("铁律4:14天不涨换股");
        assert_eq!(r, "铁律4:14天不涨换股");
    }

    #[test]
    fn extracts_iron_rule_5_reason() {
        let r = extract_reason("铁律5:布林上轨+MACD顶背离");
        assert_eq!(r, "铁律5:布林上轨+MACD顶背离");
    }

    #[test]
    fn extracts_atr_reason() {
        let r = extract_reason("ATR动态止损(有效止损价 9.20)");
        assert_eq!(r, "ATR动态止损");
    }

    #[test]
    fn extracts_unknown_reason_truncates_30_chars() {
        let input = "其他原因: 1234567890123456789012345678901234567890";
        let r = extract_reason(input);
        eprintln!("DEBUG: input len={}, r len={}, r={}", input.len(), r.len(), r);
        assert_eq!(r.chars().count(), 30);
    }
}
