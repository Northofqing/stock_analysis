//! Registered business rules: BR-046.
//! v16.3 Commit 4a — 4 铁律 + 1 bonus 接入 paper_trade 卖出路径.
//!
//! 业务: position_tracker::track_position 已实现 4 铁律
//! (StopLoss/TakeProfit/TimeExit/BollingTop)，但只写 analysis_result 表，
//! **不调 paper_trade::simulate(Sell)**。本模块把卖出动作也写到 paper_trades 表。
//!
//! 复用策略：读取 analysis_result 最新 operation_advice；若含“铁律”/“止盈”/“止损”，
//! 则调用 `paper_trade::simulate(Direction=Sell)`。不调用写 stock_position 的
//! `ClosePositionCmd`，遵守 BR-023 虚拟腿隔离。
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
    pub limit_up_price: f64,
    pub limit_down_price: f64,
    pub quote_observed_at: chrono::DateTime<chrono::Utc>,
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
    pub limit_up_price: f64,
    pub limit_down_price: f64,
    pub quote_observed_at: chrono::DateTime<chrono::Utc>,
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

    rows.into_iter()
        .map(|r| {
            let quote = crate::broker::execution_quote(&r.code)
                .map_err(|error| format!("paper position {} quote unavailable: {error}", r.code))?;
            Ok(PaperPositionSellCheck {
                code: r.code,
                name: r.name,
                avg_cost: r.avg_cost,
                quantity: r.net_qty.max(0) as u32,
                current_price: quote.price,
                limit_up_price: quote.limit_up_price,
                limit_down_price: quote.limit_down_price,
                quote_observed_at: quote.observed_at,
            })
        })
        .collect()
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
    let codes: Vec<String> = checks.iter().map(|c| quote_sql_code(&c.code)).collect();
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
                    check.name,
                    check.code,
                    reason
                );
                decisions.push(SellDecision {
                    code: check.code.clone(),
                    name: check.name.clone(),
                    reason: reason.clone(),
                    current_price: check.current_price,
                    quantity: check.quantity,
                    limit_up_price: check.limit_up_price,
                    limit_down_price: check.limit_down_price,
                    quote_observed_at: check.quote_observed_at,
                });
            }
        }
    }

    Ok(decisions)
}

fn quote_sql_code(code: &str) -> String {
    format!("'{}'", code.replace('\'', "''"))
}

/// 调 paper_trade::simulate(Sell) 写 paper_trades
///
/// Fix 3: SellDecision 加 quantity 字段, 不再硬编码 100
/// Price is the validated realtime quote captured by `load_open_positions`.
pub fn emit_sell_signal(decision: &SellDecision) -> Result<(), String> {
    let now = Local::now();
    let effective_price = decision.current_price;
    let signal = PaperSignal {
        // Fix 1: plan_id 含铁律 + ts (同 code 同日多铁律可各写 1 次)
        plan_id: format!(
            "exit-{}-{}-{}",
            decision.code,
            now.format("%Y%m%d"),
            decision
                .reason
                .replace(' ', "_")
                .chars()
                .take(16)
                .collect::<String>()
        ),
        code: decision.code.clone(),
        name: decision.name.clone(),
        direction: Direction::Sell,
        price: effective_price,
        quantity: decision.quantity,
        virtual_reason: format!("4-IronRule:{}", decision.reason),
        is_limit_up: false,
        is_limit_down: false,
        is_suspended: false,
        limit_up_price: Some(decision.limit_up_price),
        limit_down_price: Some(decision.limit_down_price),
        secondary_confirmed: false,
        quote_observed_at: decision.quote_observed_at,
        account_mode: "Normal".to_string(),
        data_mode: "Full".to_string(),
    };

    // review fix Issue #5: 传真实 portfolio state (Sell 路径 AccountMode/DataMode 检查仍生效)
    let (cash, total, pos_pct) = paper_trade::portfolio_state(&decision.code, effective_price)?;
    // v16.5 #4: 预生成 order_id/exec_id/decision_id, simulate 后 publish TradingBus (2 emit)
    let order_id = crate::bus::new_order_id();
    let exec_id = crate::bus::new_execution_id();
    let decision_id = crate::bus::new_decision_id();
    let plan_id_for_event = order_id.clone(); // emit 用 (不暴露 plan_id 给 TradingEvent)
    match paper_trade::simulate(&signal, effective_price, cash, total, pos_pct) {
        Ok(outcome) => {
            log::info!(
                "[paper_engine] 4 铁律卖出 {}({}) status={} reason={}",
                decision.name,
                decision.code,
                outcome.result.status.as_str(),
                decision.reason
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
                decision.name,
                decision.code,
                e
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

    fn unique_code(label: &str) -> String {
        format!(
            "TEST_CODE_PAPER_ENGINE_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        )
    }

    struct PaperEngineGuard {
        codes: Vec<String>,
        ledger_date: String,
    }

    impl Drop for PaperEngineGuard {
        fn drop(&mut self) {
            if let Ok(mut conn) = DatabaseManager::get().get_conn() {
                for code in &self.codes {
                    let _ = diesel::sql_query("DELETE FROM paper_trades WHERE code = ?")
                        .bind::<diesel::sql_types::Text, _>(code)
                        .execute(&mut conn);
                    let _ = diesel::sql_query("DELETE FROM analysis_result WHERE code = ?")
                        .bind::<diesel::sql_types::Text, _>(code)
                        .execute(&mut conn);
                }
                let _ = diesel::sql_query("DELETE FROM ledger WHERE date = ?")
                    .bind::<diesel::sql_types::Text, _>(&self.ledger_date)
                    .execute(&mut conn);
            }
        }
    }

    fn prepare_account(codes: Vec<String>) -> PaperEngineGuard {
        DatabaseManager::init(None).expect("test database init");
        crate::broker::ensure_test_quote_provider();
        let ledger_date = Local::now().date_naive().to_string();
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
        .bind::<diesel::sql_types::Text, _>(&ledger_date)
        .execute(&mut conn)
        .expect("prepare same-day ledger");
        diesel::sql_query(
            "UPDATE stock_position SET updated_at = CURRENT_TIMESTAMP WHERE status = 'open'",
        )
        .execute(&mut conn)
        .expect("refresh test position evidence");
        PaperEngineGuard { codes, ledger_date }
    }

    #[test]
    fn detects_iron_rule_1_stop_loss() {
        assert!(is_iron_rule_triggered("铁律1:止损(-8%)"));
        assert!(is_iron_rule_triggered("操作建议: 触发铁律1止损"));
    }

    #[test]
    fn quotes_stock_codes_for_batch_filter_without_losing_leading_zeroes() {
        // Protocol-format exception: SQL serialization must preserve a native
        // six-digit symbol's leading zeroes exactly.
        assert_eq!(quote_sql_code("000001"), "'000001'");
        assert_eq!(quote_sql_code("A'B"), "'A''B'");
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
        eprintln!(
            "DEBUG: input len={}, r len={}, r={}",
            input.len(),
            r.len(),
            r
        );
        assert_eq!(r.chars().count(), 30);
    }

    #[test]
    #[serial_test::serial]
    fn paper_engine_round_trips_open_positions_decisions_and_sell_execution() {
        let code = unique_code("TRIGGER");
        let hold_code = unique_code("HOLD");
        let _guard = prepare_account(vec![code.clone(), hold_code.clone()]);
        let mut conn = DatabaseManager::get()
            .get_conn()
            .expect("test database connection");
        for (plan, direction, price, quantity, status) in [
            ("BUY_A", "buy", 10.0, 200_i64, "Filled"),
            ("BUY_B", "buy", 12.0, 100_i64, "Filled"),
            ("SELL_A", "sell", 11.0, 100_i64, "Filled"),
            ("IGNORED", "buy", 9.0, 500_i64, "NotFilled"),
        ] {
            diesel::sql_query(
                "INSERT INTO paper_trades
                 (plan_id, code, name, direction, price, quantity, status, fill_price,
                  virtual_reason, account_mode, data_mode)
                 VALUES (?, ?, '虚拟持仓', ?, ?, ?, ?, ?, 'TEST_REASON', 'Normal', 'Full')",
            )
            .bind::<diesel::sql_types::Text, _>(format!("{plan}_{code}"))
            .bind::<diesel::sql_types::Text, _>(&code)
            .bind::<diesel::sql_types::Text, _>(direction)
            .bind::<diesel::sql_types::Double, _>(price)
            .bind::<diesel::sql_types::BigInt, _>(quantity)
            .bind::<diesel::sql_types::Text, _>(status)
            .bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>, _>(
                (status == "Filled").then_some(price),
            )
            .execute(&mut conn)
            .expect("insert isolated paper trade");
        }
        let positions = load_open_positions().expect("aggregate open paper positions");
        let position = positions
            .iter()
            .find(|position| position.code == code)
            .expect("isolated open paper position");
        assert_eq!(position.name, "虚拟持仓");
        assert_eq!(position.quantity, 200);
        assert!((position.avg_cost - (3_200.0 / 300.0)).abs() < 1e-9);
        assert_eq!(position.current_price, 10.0);
        assert_eq!(position.limit_down_price, 9.0);
        assert_eq!(position.limit_up_price, 11.0);

        let day = chrono::NaiveDate::from_ymd_opt(2026, 7, 18).unwrap();
        for (target, advice) in [
            (&code, "操作建议：触发铁律3，执行止盈"),
            (&hold_code, "持有观望"),
        ] {
            DatabaseManager::get()
                .save_analysis_result(&crate::models::NewAnalysisResult {
                    code: target.clone(),
                    name: "纸面引擎".to_string(),
                    date: day,
                    sentiment_score: 70,
                    operation_advice: advice.to_string(),
                    trend_prediction: "测试".to_string(),
                    pe_ratio: None,
                    pb_ratio: None,
                    turnover_rate: None,
                    market_cap: None,
                    circulating_cap: None,
                    close_price: Some(10.0),
                    pct_chg: Some(0.0),
                    data_source: Some("TEST_SOURCE".to_string()),
                    score_breakdown_json: None,
                    original_advice: None,
                    veto_flags_json: None,
                })
                .expect("save decision evidence");
        }
        let mut checks = vec![position.clone()];
        checks.push(PaperPositionSellCheck {
            code: hold_code,
            name: "未触发".to_string(),
            avg_cost: 10.0,
            quantity: 100,
            current_price: 10.0,
            limit_up_price: 11.0,
            limit_down_price: 9.0,
            quote_observed_at: chrono::Utc::now(),
        });
        assert!(check_4_iron_rules(&[]).unwrap().is_empty());
        let decisions = check_4_iron_rules(&checks).expect("batch iron-rule decision");
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].code, code);
        assert_eq!(decisions[0].quantity, 200);
        assert_eq!(decisions[0].reason, "铁律3:跌破5日线止盈");
        emit_sell_signal(&decisions[0]).expect("audited paper sell");

        let mut invalid = SellDecision {
            code: unique_code("INVALID"),
            name: "坏手数".to_string(),
            reason: "铁律1:止损(-8%)".to_string(),
            quantity: 0,
            current_price: 10.0,
            limit_up_price: 11.0,
            limit_down_price: 9.0,
            quote_observed_at: chrono::Utc::now(),
        };
        assert!(emit_sell_signal(&invalid).is_err());
        invalid.current_price = f64::NAN;
        assert!(emit_sell_signal(&invalid).is_err());
    }
}
