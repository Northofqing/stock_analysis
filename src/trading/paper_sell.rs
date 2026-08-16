//! BR-234 虚拟仓卖出闭环（paper_trades 聚合持仓 × 四大铁律）。
//!
//! `paper_trades` 的 Filled buy 按 code 聚合成当前持仓（buy − sell 净额、
//! 加权成本、首笔买入日），每 tick 用实时价（BR-218 5s 门）+ 日K指标
//! （MA5/20/60、ATR14、布林+MACD）评估四大铁律卖出条件；触发则虚拟卖出
//! （`paper_trade::simulate(Direction::Sell)` 写 paper_trades + order_audit），
//! 返回结果供 monitor 推送。
//!
//! BR-023 隔离：本模块零写 stock_position；BR-151 快照模式：资金口径来自
//! 用户确认的真实账户快照（portfolio_state_snapshot）。
//!
//! 卖出判定统一走 `pipeline::position_tracker::evaluate_sell_rules`（BR-234
//! 抽离的纯函数），与旧模拟仓 track_position 共用，避免规则漂移。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use diesel::prelude::*;
use log::{info, warn};

use crate::data_gateway::historical_bars::HistoricalBarsGateway;
use crate::data_provider::KlineData;
use crate::database::DatabaseManager;
use crate::pipeline::position_tracker::{evaluate_sell_rules, SellEvaluation};
use crate::strategy::detect_boll_macd_signal;
use crate::trading::paper_trade::{
    portfolio_state_snapshot, simulate, Direction, PaperRiskContext, PaperSignal, PaperTradeStatus,
};
use crate::trend_analyzer::StockTrendAnalyzer;

/// 聚合持仓视图（paper_trades buy Filled − sell Filled 净额）。
#[derive(Debug, Clone)]
pub struct PaperPosition {
    pub code: String,
    pub name: String,
    /// 净持仓数量（股，> 0）
    pub quantity: i64,
    /// 加权买入成本 = Σ(price×qty) / Σqty
    pub avg_buy_price: f64,
    /// 首笔买入日期（T+1 锁仓判定用）
    pub first_buy_date: chrono::NaiveDate,
}

/// 一次卖出动作的结果（供 monitor 推送）。
#[derive(Debug, Clone)]
pub struct PaperSellResult {
    pub code: String,
    pub name: String,
    pub quantity: i64,
    pub price: f64,
    /// 净收益率（未扣往返交易成本，展示用）
    pub return_rate_pct: f64,
    pub reason: String,
}

// ============================================================================
// 日K指标缓存（15min TTL）— 避免每 30s tick 对每只持仓拉一次 K 线
// ============================================================================

#[derive(Debug, Clone)]
struct Indicators {
    ma5: Option<f64>,
    ma20: Option<f64>,
    ma60: Option<f64>,
    atr: Option<f64>,
    boll_macd: crate::strategy::BollMacdSignal,
}

const INDICATOR_TTL: Duration = Duration::from_secs(15 * 60);

static INDICATOR_CACHE: Mutex<Option<HashMap<String, (Instant, Indicators)>>> = Mutex::new(None);

fn cached_indicators(code: &str) -> Option<Indicators> {
    let guard = INDICATOR_CACHE.lock().ok()?;
    let map = guard.as_ref()?;
    let (stored_at, indicators) = map.get(code)?;
    if stored_at.elapsed() > INDICATOR_TTL {
        return None;
    }
    Some(indicators.clone())
}

fn store_indicators(code: &str, indicators: Indicators) {
    if let Ok(mut guard) = INDICATOR_CACHE.lock() {
        let map = guard.get_or_insert_with(HashMap::new);
        map.insert(code.to_string(), (Instant::now(), indicators));
        if map.len() > 500 {
            map.clear(); // 上限保护：缓存超载时整体失效，下次 tick 重拉
        }
    }
}

/// ATR14：最近 14 个交易日 high−low 的均值（K 线降序，最新在前）。
fn atr14(data: &[KlineData]) -> Option<f64> {
    let window = data.iter().take(14).collect::<Vec<_>>();
    if window.len() < 14 {
        return None;
    }
    let sum: f64 = window.iter().map(|bar| bar.high - bar.low).sum();
    Some(sum / 14.0)
}

/// 拉取并缓存日K指标（MA5/20/60、ATR14、布林+MACD）。
/// 失败出声（warn 在调用方），本 tick 跳过该只（fail-closed）。
fn fetch_indicators(code: &str) -> Result<Indicators, String> {
    if let Some(cached) = cached_indicators(code) {
        return Ok(cached);
    }
    let admitted = HistoricalBarsGateway::new()
        .daily_bars(code, 90)
        .map_err(|error| format!("{code} 日K获取失败: {error}"))?;
    let records = admitted.records();
    if records.is_empty() {
        return Err(format!("{code} 日K为空"));
    }
    let trend = StockTrendAnalyzer::new().analyze_with_kline(records, code);
    let indicators = Indicators {
        ma5: (trend.ma5 > 0.0).then_some(trend.ma5),
        ma20: (trend.ma20 > 0.0).then_some(trend.ma20),
        ma60: (trend.ma60 > 0.0).then_some(trend.ma60),
        atr: atr14(records),
        boll_macd: detect_boll_macd_signal(records),
    };
    store_indicators(code, indicators.clone());
    Ok(indicators)
}

// ============================================================================
// 聚合持仓
// ============================================================================

/// 从 paper_trades 聚合当前持仓：Filled buy − Filled sell（净额 > 0 保留）。
pub fn aggregate_open_positions() -> Result<Vec<PaperPosition>, String> {
    let db = DatabaseManager::try_get().ok_or_else(|| "DB 未初始化".to_string())?;
    let mut conn = db
        .get_conn()
        .map_err(|error| format!("DB 连接失败: {error}"))?;
    let rows = diesel::sql_query(
        "SELECT b.code AS code, b.name AS name, \
                (b.qty - COALESCE(s.qty, 0)) AS net_qty, \
                (b.amt / b.qty) AS avg_price, \
                b.first_ts AS first_ts \
         FROM (SELECT code, name, SUM(quantity) AS qty, SUM(price * quantity) AS amt, \
                      MIN(ts) AS first_ts \
               FROM paper_trades WHERE direction = 'buy' AND status = 'Filled' \
               GROUP BY code, name) b \
         LEFT JOIN (SELECT code, SUM(quantity) AS qty \
                    FROM paper_trades WHERE direction = 'sell' AND status = 'Filled' \
                    GROUP BY code) s ON b.code = s.code \
         WHERE b.qty - COALESCE(s.qty, 0) > 0",
    )
    .load::<AggregateRow>(&mut conn)
    .map_err(|error| format!("paper_trades 聚合失败: {error}"))?;

    let mut positions = Vec::new();
    for row in rows {
        let first_buy_date = chrono::NaiveDate::parse_from_str(&row.first_ts[..10], "%Y-%m-%d")
            .map_err(|error| format!("{} 首买日期解析失败: {error}", row.code))?;
        positions.push(PaperPosition {
            code: row.code,
            name: row.name,
            quantity: row.net_qty,
            avg_buy_price: row.avg_price,
            first_buy_date,
        });
    }
    Ok(positions)
}

#[derive(QueryableByName)]
struct AggregateRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    code: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    name: String,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    net_qty: i64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    avg_price: f64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    first_ts: String,
}

/// 当日是否已对该 code 卖出（当日一票一卖幂等）。
fn already_sold_today(code: &str, today: &str) -> Result<bool, String> {
    let db = DatabaseManager::try_get().ok_or_else(|| "DB 未初始化".to_string())?;
    let mut conn = db
        .get_conn()
        .map_err(|error| format!("DB 连接失败: {error}"))?;
    let count: i64 = diesel::sql_query(
        "SELECT COUNT(*) AS n FROM paper_trades \
         WHERE code = ? AND direction = 'sell' AND status = 'Filled' AND date(ts) = ?",
    )
    .bind::<diesel::sql_types::Text, _>(code)
    .bind::<diesel::sql_types::Text, _>(today)
    .load::<CountRow>(&mut conn)
    .map_err(|error| format!("当日卖出幂等检查失败: {error}"))?
    .first()
    .map(|row: &CountRow| row.n)
    .unwrap_or(0);
    Ok(count > 0)
}

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    n: i64,
}

// ============================================================================
// 盘中时间窗（北京时间 9:30-11:30 / 13:00-15:00，周一至五）
// ============================================================================

fn in_trading_session() -> bool {
    use chrono::{Datelike, Timelike};
    let now = chrono::Local::now();
    match now.weekday() {
        chrono::Weekday::Sat | chrono::Weekday::Sun => return false,
        _ => {}
    }
    let minute = now.hour() * 60 + now.minute();
    (9 * 60 + 30..=11 * 60 + 30).contains(&minute) || (13 * 60..=15 * 60).contains(&minute)
}

// ============================================================================
// 卖出扫描
// ============================================================================

/// 盘中卖出扫描（30s tick）：带交易时段守卫。
pub fn scan_and_sell(risk_context: PaperRiskContext) -> Result<Vec<PaperSellResult>, String> {
    if !in_trading_session() {
        return Ok(Vec::new());
    }
    scan_and_sell_inner(risk_context)
}

/// 收盘后卖出扫描（15:30 evening_review）：无交易时段守卫。
pub fn scan_and_sell_post_close(
    risk_context: PaperRiskContext,
) -> Result<Vec<PaperSellResult>, String> {
    scan_and_sell_inner(risk_context)
}

fn scan_and_sell_inner(risk_context: PaperRiskContext) -> Result<Vec<PaperSellResult>, String> {
    let positions = aggregate_open_positions()?;
    if positions.is_empty() {
        return Ok(Vec::new());
    }
    let today = chrono::Local::now().date_naive();
    let mut sold = Vec::new();
    for pos in &positions {
        match evaluate_and_sell(pos, risk_context, today) {
            Ok(Some(result)) => sold.push(result),
            Ok(None) => {}
            Err(error) => warn!("[paper_sell] {} 评估失败: {error}", pos.code),
        }
    }
    Ok(sold)
}

/// 对单只聚合持仓评估四大铁律并（触发时）虚拟卖出。
fn evaluate_and_sell(
    pos: &PaperPosition,
    risk_context: PaperRiskContext,
    today: chrono::NaiveDate,
) -> Result<Option<PaperSellResult>, String> {
    // 1. 实时价（BR-218 5s 门；超龄 fail-closed，下 tick 重试）
    let quote = crate::broker::execution_quote(&pos.code).map_err(|error| {
        warn!(
            "[paper_sell] {} 实时价不可用，本 tick 跳过: {error}",
            pos.code
        );
        error
    })?;

    // 2. 日K指标（15min 缓存）
    let indicators = fetch_indicators(&pos.code).map_err(|error| {
        warn!("[paper_sell] {error}，本 tick 跳过");
        error
    })?;

    // 3. 四大铁律判定（BR-234 纯函数）
    let eval = SellEvaluation {
        code: &pos.code,
        name: &pos.name,
        buy_price: pos.avg_buy_price,
        buy_date: pos.first_buy_date,
        current_price: quote.price,
        ma5: indicators.ma5,
        ma20: indicators.ma20,
        ma60: indicators.ma60,
        atr: indicators.atr,
        boll_macd: Some(&indicators.boll_macd),
        today,
    };
    let Some(reason) = evaluate_sell_rules(&eval) else {
        return Ok(None);
    };

    // 4. T+1 锁仓：A股当日买入不可卖出（warn 建议次日竞价挂单）
    if pos.first_buy_date == today {
        warn!(
            "[paper_sell] {} T+1锁仓无法卖出(原因: {}) — 建议次日竞价挂单",
            pos.code, reason
        );
        return Ok(None);
    }

    // 5. 当日一票一卖幂等
    let today_str = today.format("%Y-%m-%d").to_string();
    if already_sold_today(&pos.code, &today_str)? {
        info!("[paper_sell] {} 当日已卖出，跳过", pos.code);
        return Ok(None);
    }

    // 6. 虚拟卖出（跌停/滑点判定 + INSERT paper_trades + order_audit）
    let gross_pct = (quote.price / pos.avg_buy_price - 1.0) * 100.0;
    let (cash, total, pos_pct) = portfolio_state_snapshot(&pos.code, quote.price)?;
    let signal = PaperSignal {
        plan_id: format!("paper-sell-{}-{}", pos.code, today_str),
        code: pos.code.clone(),
        name: pos.name.clone(),
        direction: Direction::Sell,
        price: quote.price,
        quantity: pos.quantity as u32,
        virtual_reason: format!("BR-234四大铁律卖出:{reason}"),
        is_limit_up: false,
        is_limit_down: quote.price <= quote.limit_down_price,
        is_suspended: false,
        limit_up_price: Some(quote.limit_up_price),
        limit_down_price: Some(quote.limit_down_price),
        secondary_confirmed: false,
        quote_observed_at: quote.observed_at,
        risk_context,
    };
    let outcome = simulate(&signal, quote.price, cash, total, pos_pct)?;
    if outcome.result.status != PaperTradeStatus::Filled {
        warn!(
            "[paper_sell] {} 卖出未成交: {:?}",
            pos.code, outcome.result.status
        );
        return Ok(None);
    }
    info!(
        "[paper_sell] {} 虚拟卖出 {}股 @{:.2}，收益率 {:+.2}%（原因: {}）",
        pos.name, pos.quantity, quote.price, gross_pct, reason
    );
    Ok(Some(PaperSellResult {
        code: pos.code.clone(),
        name: pos.name.clone(),
        quantity: pos.quantity,
        price: quote.price,
        return_rate_pct: gross_pct,
        reason,
    }))
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::DatabaseManager;

    fn init_test_db() {
        let _ = DatabaseManager::init(None);
    }

    /// 写入一笔 Filled buy（TEST_CODE_ 前缀保证测试隔离）。
    fn insert_buy(code: &str, name: &str, price: f64, qty: i64, ts: &str) {
        let db = DatabaseManager::get();
        let mut conn = db.get_conn().unwrap();
        diesel::sql_query(
            "INSERT OR IGNORE INTO paper_trades \
             (plan_id, code, name, direction, price, quantity, status, virtual_reason, account_mode, data_mode, ts) \
             VALUES (?, ?, ?, 'buy', ?, ?, 'Filled', 'test-fixture', 'Normal', 'Full', ?)",
        )
        .bind::<diesel::sql_types::Text, _>(format!("test-buy-{code}-{ts}"))
        .bind::<diesel::sql_types::Text, _>(code)
        .bind::<diesel::sql_types::Text, _>(name)
        .bind::<diesel::sql_types::Double, _>(price)
        .bind::<diesel::sql_types::BigInt, _>(qty)
        .bind::<diesel::sql_types::Text, _>(ts)
        .execute(&mut conn)
        .unwrap();
    }

    fn insert_sell(code: &str, price: f64, qty: i64, ts: &str) {
        let db = DatabaseManager::get();
        let mut conn = db.get_conn().unwrap();
        diesel::sql_query(
            "INSERT OR IGNORE INTO paper_trades \
             (plan_id, code, name, direction, price, quantity, status, virtual_reason, account_mode, data_mode, ts) \
             VALUES (?, ?, 'TEST', 'sell', ?, ?, 'Filled', 'test-fixture', 'Normal', 'Full', ?)",
        )
        .bind::<diesel::sql_types::Text, _>(format!("test-sell-{code}-{ts}"))
        .bind::<diesel::sql_types::Text, _>(code)
        .bind::<diesel::sql_types::Double, _>(price)
        .bind::<diesel::sql_types::BigInt, _>(qty)
        .bind::<diesel::sql_types::Text, _>(ts)
        .execute(&mut conn)
        .unwrap();
    }

    #[test]
    fn aggregate_computes_net_quantity_weighted_cost_and_first_buy_date() {
        init_test_db();
        insert_buy(
            "TEST_CODE_600001",
            "测试甲",
            10.0,
            200,
            "2026-08-03 10:00:00",
        );
        insert_buy(
            "TEST_CODE_600001",
            "测试甲",
            12.0,
            100,
            "2026-08-05 10:00:00",
        );
        insert_sell("TEST_CODE_600001", 11.0, 100, "2026-08-06 10:00:00");

        let positions = aggregate_open_positions().unwrap();
        let pos = positions
            .iter()
            .find(|p| p.code == "TEST_CODE_600001")
            .expect("持仓应存在");
        // 净额 = 300 - 100 = 200；加权成本 = (10×200 + 12×100)/300 = 10.6667
        assert_eq!(pos.quantity, 200);
        assert!((pos.avg_buy_price - 10.6667).abs() < 0.01);
        assert_eq!(
            pos.first_buy_date,
            chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap()
        );
    }

    #[test]
    fn aggregate_drops_fully_sold_positions() {
        init_test_db();
        insert_buy(
            "TEST_CODE_600002",
            "测试乙",
            10.0,
            100,
            "2026-08-03 10:00:00",
        );
        insert_sell("TEST_CODE_600002", 11.0, 100, "2026-08-06 10:00:00");
        let positions = aggregate_open_positions().unwrap();
        assert!(
            !positions.iter().any(|p| p.code == "TEST_CODE_600002"),
            "净持仓为 0 的票不应出现在聚合结果"
        );
    }

    #[test]
    fn already_sold_today_returns_true_after_sell() {
        init_test_db();
        insert_sell("TEST_CODE_600003", 11.0, 100, "2026-08-06 10:00:00");
        assert!(already_sold_today("TEST_CODE_600003", "2026-08-06").unwrap());
        assert!(!already_sold_today("TEST_CODE_600003", "2026-08-07").unwrap());
        assert!(!already_sold_today("TEST_CODE_600004", "2026-08-06").unwrap());
    }

    #[test]
    fn atr14_computes_window_mean() {
        let bars: Vec<KlineData> = (0..14)
            .map(|_| KlineData {
                date: chrono::NaiveDate::from_ymd_opt(2026, 8, 1).unwrap(),
                open: 10.0,
                high: 11.0,
                low: 9.0,
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
            .collect();
        // 每根 high-low = 2.0，14 根均值 = 2.0
        assert_eq!(atr14(&bars), Some(2.0));
        let short: Vec<KlineData> = bars[..5].to_vec();
        assert_eq!(atr14(&short), None);
    }

    #[test]
    fn trading_session_window_does_not_panic() {
        use chrono::Datelike;
        // 系统时钟不可冻结——验证窗口边界逻辑本身不崩溃；
        // 若当前为周末，则确认必不在交易时段
        let now = chrono::Local::now();
        if matches!(now.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun) {
            assert!(!in_trading_session());
        } else {
            let _ = in_trading_session();
        }
    }
}
