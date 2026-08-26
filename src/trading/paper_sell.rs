//! BR-134 虚拟仓卖出闭环（paper_trades FIFO 批次持仓 × 四大铁律）。
//!
//! `paper_trades` 的 Filled 成交按 `(ts,id)` 做 FIFO 重建，只把 T+1 可卖
//! 批次的数量、加权成本和最早日期交给每 tick 的实时价（BR-218 5s 门）+ 日K指标
//! （MA5/20/60、ATR14、布林+MACD）评估四大铁律卖出条件；触发则虚拟卖出
//! （`paper_trade::simulate(Direction::Sell)` 写 paper_trades + order_audit），
//! 返回结果供 monitor 推送。
//!
//! BR-023 隔离：本模块零写 stock_position；BR-151 快照模式：资金口径来自
//! 用户确认的真实账户快照（portfolio_state_snapshot）。
//!
//! 卖出判定统一走 `pipeline::position_tracker::evaluate_sell_rules`（BR-134
//! 抽离的纯函数），与旧模拟仓 track_position 共用，避免规则漂移。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use diesel::prelude::*;
use log::{info, warn};

use crate::data_gateway::historical_bars::HistoricalBarsGateway;
use crate::data_provider::KlineData;
use crate::database::paper_inventory_failure_audit::{
    append_failure_on_conn, PaperInventoryFailureRecord, PaperInventoryFailureStage,
    PaperInventorySourceFact,
};
use crate::database::DatabaseManager;
use crate::pipeline::position_tracker::{evaluate_sell_rules, SellEvaluation};
use crate::strategy::detect_boll_macd_signal;
use crate::trading::paper_lot_ledger::{
    parse_paper_fill_timestamp, rebuild_paper_positions, PaperFill, PaperPositionInventory,
};
use crate::trading::paper_trade::{
    portfolio_state_snapshot, simulate_with_audit_evidence, Direction, PaperAuditEvidence,
    PaperRiskContext, PaperSignal, PaperTradeStatus,
};
use crate::trend_analyzer::StockTrendAnalyzer;

/// 可卖持仓视图（由 Filled 成交按 FIFO 重建）。
#[derive(Debug, Clone)]
pub struct PaperPosition {
    pub code: String,
    pub name: String,
    /// T+1 可卖数量（股，> 0）
    pub quantity: i64,
    /// 可卖批次加权成本
    pub avg_buy_price: f64,
    /// 最早可卖批次的买入日期
    pub first_buy_date: chrono::NaiveDate,
    /// 绑定评估日、成交身份与剩余批次的规范化审计证据。
    pub inventory_audit_evidence: String,
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

/// 从 paper_trades 逐行重建当前持仓，只返回 T+1 可卖候选。
pub fn aggregate_open_positions() -> Result<Vec<PaperPosition>, String> {
    aggregate_open_positions_at(chrono::Local::now().date_naive())
}

fn aggregate_open_positions_at(
    as_of_date: chrono::NaiveDate,
) -> Result<Vec<PaperPosition>, String> {
    let db = DatabaseManager::try_get()
        .ok_or_else(|| "DB 未初始化；BR-249 来源不可用，失败审计未形成".to_string())?;
    let mut conn = db
        .get_conn()
        .map_err(|error| format!("DB 连接失败: {error}；BR-249 来源不可用，失败审计未形成"))?;
    let rows = diesel::sql_query(
        "SELECT id, code, name, direction, fill_price, quantity, \
                CAST(ts AS TEXT) AS occurred_at \
         FROM paper_trades \
         WHERE status = 'Filled' \
         ORDER BY ts ASC, id ASC",
    )
    .load::<FilledTradeRow>(&mut conn)
    .map_err(|error| {
        format!("paper_trades Filled 读取失败: {error}；BR-249 来源事实不可用，失败审计未形成")
    })?;
    let source_facts = rows
        .iter()
        .map(|row| {
            PaperInventorySourceFact::new(
                row.id,
                row.code.clone(),
                row.name.clone(),
                row.direction.clone(),
                row.fill_price,
                row.quantity,
                row.occurred_at.clone(),
            )
        })
        .collect::<Vec<_>>();

    let mut fills = Vec::with_capacity(rows.len());
    for row in rows {
        let occurred_at = match parse_paper_fill_timestamp(row.id, &row.occurred_at) {
            Ok(occurred_at) => occurred_at,
            Err(error) => {
                return Err(audit_inventory_failure(
                    &mut conn,
                    as_of_date,
                    PaperInventoryFailureStage::ParseRawFill,
                    &error,
                    &source_facts,
                ));
            }
        };
        fills.push(PaperFill {
            id: row.id,
            code: row.code,
            name: row.name,
            direction: row.direction,
            fill_price: row.fill_price,
            quantity: row.quantity,
            occurred_at,
        });
    }

    let inventories = match rebuild_paper_positions(&fills, as_of_date) {
        Ok(inventories) => inventories,
        Err(error) => {
            return Err(audit_inventory_failure(
                &mut conn,
                as_of_date,
                PaperInventoryFailureStage::RebuildFifo,
                &error,
                &source_facts,
            ));
        }
    };
    project_sellable_positions(inventories).map_err(|error| {
        audit_inventory_failure(
            &mut conn,
            as_of_date,
            PaperInventoryFailureStage::ProjectSellablePosition,
            &error,
            &source_facts,
        )
    })
}

fn project_sellable_positions(
    inventories: Vec<PaperPositionInventory>,
) -> Result<Vec<PaperPosition>, String> {
    let mut positions = Vec::new();
    for inventory in inventories {
        if inventory.sellable_quantity == 0 {
            continue;
        }
        let avg_buy_price = inventory.sellable_avg_price.ok_or_else(|| {
            format!(
                "paper position {} missing sellable average price",
                inventory.code
            )
        })?;
        let first_buy_date = inventory.earliest_sellable_date.ok_or_else(|| {
            format!(
                "paper position {} missing earliest sellable date",
                inventory.code
            )
        })?;
        let inventory_audit_evidence = inventory.audit_evidence();
        positions.push(PaperPosition {
            code: inventory.code,
            name: inventory.name,
            quantity: i64::from(inventory.sellable_quantity),
            avg_buy_price,
            first_buy_date,
            inventory_audit_evidence,
        });
    }
    Ok(positions)
}

fn audit_inventory_failure(
    conn: &mut SqliteConnection,
    as_of_date: chrono::NaiveDate,
    stage: PaperInventoryFailureStage,
    diagnostic: &str,
    source_facts: &[PaperInventorySourceFact],
) -> String {
    let record = PaperInventoryFailureRecord {
        as_of_date,
        stage,
        diagnostic,
        source_facts,
    };
    match append_failure_on_conn(conn, &record) {
        Ok(receipt) => format!(
            "{diagnostic}; BR-249 audit_id={} record_hash={} disposition={}",
            receipt.audit_id,
            receipt.record_hash,
            receipt.disposition.as_str()
        ),
        Err(error) => {
            format!("{diagnostic}; BR-249 持久失败审计不可用: {error}")
        }
    }
}

#[derive(QueryableByName)]
struct FilledTradeRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    id: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    code: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    name: String,
    #[diesel(sql_type = diesel::sql_types::Text)]
    direction: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Double>)]
    fill_price: Option<f64>,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    quantity: i64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    occurred_at: String,
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
    let today = chrono::Local::now().date_naive();
    let positions = aggregate_open_positions_at(today)?;
    if positions.is_empty() {
        return Ok(Vec::new());
    }
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
    // 1. 适配器不变量：候选必须来自早于评估日的可卖批次。
    if pos.first_buy_date >= today {
        return Err(format!(
            "BR-134 sellable inventory invariant violated: code={} first_buy_date={} today={}",
            pos.code, pos.first_buy_date, today
        ));
    }

    // 2. 实时价（BR-218 5s 门；超龄 fail-closed，下 tick 重试）
    let quote = crate::broker::execution_quote(&pos.code).map_err(|error| {
        warn!(
            "[paper_sell] {} 实时价不可用，本 tick 跳过: {error}",
            pos.code
        );
        error
    })?;

    // 3. 日K指标（15min 缓存）
    let indicators = fetch_indicators(&pos.code).map_err(|error| {
        warn!("[paper_sell] {error}，本 tick 跳过");
        error
    })?;

    // 4. 四大铁律判定（BR-134 共用纯函数）
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
        // 历史归因与回测以该前缀识别卖出事件；规则正文现登记为 BR-134，
        // 但这里保留存量事件协议，避免同一类卖出被拆成两个信号族。
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
    let audit_evidence = PaperAuditEvidence::new(pos.inventory_audit_evidence.clone())?;
    let outcome =
        simulate_with_audit_evidence(&signal, quote.price, cash, total, pos_pct, &audit_evidence)?;
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

    fn unique_code(label: &str) -> String {
        format!(
            "TEST_CODE_PAPER_SELL_{label}_{}_{}",
            std::process::id(),
            chrono::Utc::now()
                .timestamp_nanos_opt()
                .expect("test timestamp")
        )
    }

    struct PaperSellGuard {
        codes: Vec<String>,
    }

    impl PaperSellGuard {
        fn new(codes: Vec<String>) -> Self {
            init_test_db();
            Self { codes }
        }
    }

    impl Drop for PaperSellGuard {
        fn drop(&mut self) {
            if let Ok(mut conn) = DatabaseManager::get().get_conn() {
                for code in &self.codes {
                    let _ = diesel::sql_query("DELETE FROM paper_trades WHERE code = ?")
                        .bind::<diesel::sql_types::Text, _>(code)
                        .execute(&mut conn);
                }
            }
        }
    }

    fn init_test_db() {
        let _ = DatabaseManager::init(None);
    }

    fn date(year: i32, month: u32, day: u32) -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    /// 写入一笔 Filled buy（TEST_CODE_ 前缀保证测试隔离）。
    fn insert_buy(code: &str, name: &str, price: f64, qty: i64, ts: &str) {
        let db = DatabaseManager::get();
        let mut conn = db.get_conn().unwrap();
        diesel::sql_query(
            "INSERT OR IGNORE INTO paper_trades \
             (plan_id, code, name, direction, price, quantity, status, fill_price, \
              virtual_reason, account_mode, data_mode, ts) \
             VALUES (?, ?, ?, 'buy', ?, ?, 'Filled', ?, \
                     'test-fixture', 'Normal', 'Full', ?)",
        )
        .bind::<diesel::sql_types::Text, _>(format!("test-buy-{code}-{ts}"))
        .bind::<diesel::sql_types::Text, _>(code)
        .bind::<diesel::sql_types::Text, _>(name)
        .bind::<diesel::sql_types::Double, _>(price)
        .bind::<diesel::sql_types::BigInt, _>(qty)
        .bind::<diesel::sql_types::Double, _>(price)
        .bind::<diesel::sql_types::Text, _>(ts)
        .execute(&mut conn)
        .unwrap();
    }

    fn insert_sell(code: &str, price: f64, qty: i64, ts: &str) {
        let db = DatabaseManager::get();
        let mut conn = db.get_conn().unwrap();
        diesel::sql_query(
            "INSERT OR IGNORE INTO paper_trades \
             (plan_id, code, name, direction, price, quantity, status, fill_price, \
              virtual_reason, account_mode, data_mode, ts) \
             VALUES (?, ?, 'TEST', 'sell', ?, ?, 'Filled', ?, \
                     'test-fixture', 'Normal', 'Full', ?)",
        )
        .bind::<diesel::sql_types::Text, _>(format!("test-sell-{code}-{ts}"))
        .bind::<diesel::sql_types::Text, _>(code)
        .bind::<diesel::sql_types::Double, _>(price)
        .bind::<diesel::sql_types::BigInt, _>(qty)
        .bind::<diesel::sql_types::Double, _>(price)
        .bind::<diesel::sql_types::Text, _>(ts)
        .execute(&mut conn)
        .unwrap();
    }

    fn insert_fill(
        code: &str,
        name: &str,
        direction: &str,
        signal_price: f64,
        fill_price: Option<f64>,
        quantity: i64,
        occurred_at: &str,
    ) {
        let db = DatabaseManager::get();
        let mut conn = db.get_conn().unwrap();
        diesel::sql_query(
            "INSERT INTO paper_trades
             (plan_id, code, name, direction, price, quantity, status, fill_price,
              virtual_reason, account_mode, data_mode, ts)
             VALUES (?, ?, ?, ?, ?, ?, 'Filled', ?,
                     'TEST_REASON', 'Normal', 'Full', ?)",
        )
        .bind::<diesel::sql_types::Text, _>(format!(
            "TEST_CODE_PAPER_SELL_FILL_{}",
            chrono::Utc::now()
                .timestamp_nanos_opt()
                .expect("test timestamp")
        ))
        .bind::<diesel::sql_types::Text, _>(code)
        .bind::<diesel::sql_types::Text, _>(name)
        .bind::<diesel::sql_types::Text, _>(direction)
        .bind::<diesel::sql_types::Double, _>(signal_price)
        .bind::<diesel::sql_types::BigInt, _>(quantity)
        .bind::<diesel::sql_types::Nullable<diesel::sql_types::Double>, _>(fill_price)
        .bind::<diesel::sql_types::Text, _>(occurred_at)
        .execute(&mut conn)
        .unwrap();
    }

    fn inventory_failure_audit_count(code: &str) -> i64 {
        let mut conn = DatabaseManager::get().get_conn().unwrap();
        diesel::sql_query(
            "SELECT COUNT(*) AS n FROM paper_inventory_failure_audit
             WHERE instr(source_facts_json, ?) > 0",
        )
        .bind::<diesel::sql_types::Text, _>(code)
        .get_result::<CountRow>(&mut conn)
        .expect("count BR-249 audit rows")
        .n
    }

    fn order_audit_count(code: &str) -> i64 {
        let mut conn = DatabaseManager::get().get_conn().unwrap();
        diesel::sql_query("SELECT COUNT(*) AS n FROM order_audit WHERE code = ?")
            .bind::<diesel::sql_types::Text, _>(code)
            .get_result::<CountRow>(&mut conn)
            .expect("count order audit rows")
            .n
    }

    #[test]
    #[serial_test::serial]
    fn aggregate_uses_fill_price_instead_of_signal_price() {
        let code = unique_code("FILL_PRICE");
        let _guard = PaperSellGuard::new(vec![code.clone()]);
        insert_fill(
            &code,
            "测试成交价",
            "buy",
            99.0,
            Some(10.0),
            100,
            "2026-08-03 10:00:00",
        );

        let positions = aggregate_open_positions().unwrap();
        let position = positions.iter().find(|item| item.code == code).unwrap();

        assert_eq!(position.avg_buy_price, 10.0);
    }

    #[test]
    #[serial_test::serial]
    fn aggregate_computes_fifo_remaining_cost_and_first_sellable_date() {
        let code = unique_code("FIFO_REMAINING");
        let _guard = PaperSellGuard::new(vec![code.clone()]);
        insert_buy(&code, "测试甲", 10.0, 200, "2026-08-03 10:00:00");
        insert_buy(&code, "测试甲", 12.0, 100, "2026-08-05 10:00:00");
        insert_sell(&code, 11.0, 100, "2026-08-06 10:00:00");

        let positions = aggregate_open_positions_at(date(2026, 8, 7)).unwrap();
        let pos = positions
            .iter()
            .find(|p| p.code == code)
            .expect("持仓应存在");
        // FIFO 卖出最老的 100 股后，剩余为 100@10 + 100@12。
        assert_eq!(pos.quantity, 200);
        assert_eq!(pos.avg_buy_price, 11.0);
        assert_eq!(
            pos.first_buy_date,
            chrono::NaiveDate::from_ymd_opt(2026, 8, 3).unwrap()
        );
    }

    #[test]
    #[serial_test::serial]
    fn aggregate_drops_fully_sold_positions() {
        let code = unique_code("FULLY_SOLD");
        let _guard = PaperSellGuard::new(vec![code.clone()]);
        insert_buy(&code, "测试乙", 10.0, 100, "2026-08-03 10:00:00");
        insert_sell(&code, 11.0, 100, "2026-08-06 10:00:00");
        let positions = aggregate_open_positions_at(date(2026, 8, 7)).unwrap();
        assert!(!positions.iter().any(|p| p.code == code));
    }

    #[test]
    #[serial_test::serial]
    fn already_sold_today_returns_true_after_sell() {
        let sold_code = unique_code("SOLD_TODAY");
        let untouched_code = unique_code("NOT_SOLD_TODAY");
        let _guard = PaperSellGuard::new(vec![sold_code.clone(), untouched_code.clone()]);
        insert_sell(&sold_code, 11.0, 100, "2026-08-06 10:00:00");
        assert!(already_sold_today(&sold_code, "2026-08-06").unwrap());
        assert!(!already_sold_today(&sold_code, "2026-08-07").unwrap());
        assert!(!already_sold_today(&untouched_code, "2026-08-06").unwrap());
    }

    #[test]
    #[serial_test::serial]
    fn mixed_position_exposes_only_overnight_quantity_and_cost() {
        let code = unique_code("MIXED");
        let _guard = PaperSellGuard::new(vec![code.clone()]);
        insert_fill(
            &code,
            "混合持仓",
            "buy",
            10.0,
            Some(10.0),
            200,
            "2026-08-03 10:00:00",
        );
        insert_fill(
            &code,
            "混合持仓",
            "buy",
            12.0,
            Some(12.0),
            100,
            "2026-08-05 10:00:00",
        );

        let positions = aggregate_open_positions_at(date(2026, 8, 5)).unwrap();
        let position = positions.iter().find(|item| item.code == code).unwrap();

        assert_eq!(position.quantity, 200);
        assert_eq!(position.avg_buy_price, 10.0);
        assert_eq!(position.first_buy_date, date(2026, 8, 3));
    }

    #[test]
    #[serial_test::serial]
    fn same_day_only_position_is_not_a_sell_candidate() {
        let code = unique_code("LOCKED_ONLY");
        let _guard = PaperSellGuard::new(vec![code.clone()]);
        insert_fill(
            &code,
            "当日锁定",
            "buy",
            12.0,
            Some(12.0),
            100,
            "2026-08-05 10:00:00",
        );

        let positions = aggregate_open_positions_at(date(2026, 8, 5)).unwrap();

        assert!(!positions.iter().any(|item| item.code == code));
    }

    #[test]
    #[serial_test::serial]
    fn aggregate_rejects_missing_fill_price_for_the_whole_batch() {
        let code = unique_code("MISSING_FILL");
        let _guard = PaperSellGuard::new(vec![code.clone()]);
        insert_fill(
            &code,
            "缺成交价",
            "buy",
            10.0,
            None,
            100,
            "2026-08-03 10:00:00",
        );

        let first_error =
            aggregate_open_positions_at(date(2026, 8, 5)).expect_err("Filled 缺成交价必须整批失败");
        let replay_error =
            aggregate_open_positions_at(date(2026, 8, 5)).expect_err("同一坏事实重放仍必须失败");

        assert!(first_error.contains("fill_price"), "{first_error}");
        assert!(
            first_error.contains("BR-249 audit_id=")
                && first_error.contains("disposition=appended"),
            "{first_error}"
        );
        assert!(
            replay_error.contains("disposition=existing"),
            "{replay_error}"
        );
        assert_eq!(inventory_failure_audit_count(&code), 1);
    }

    #[test]
    #[serial_test::serial]
    fn aggregate_persists_t1_failure_before_any_order_attempt() {
        let code = unique_code("T1_AUDIT");
        let _guard = PaperSellGuard::new(vec![code.clone()]);
        insert_fill(
            &code,
            "T+1审计",
            "buy",
            10.0,
            Some(10.0),
            100,
            "2026-08-11 09:31:00",
        );
        insert_fill(
            &code,
            "T+1审计",
            "sell",
            10.2,
            Some(10.2),
            100,
            "2026-08-11 14:31:00",
        );
        let order_attempts_before = order_audit_count(&code);

        let error = aggregate_open_positions_at(date(2026, 8, 12))
            .expect_err("同日卖出必须在订单前失败并审计");

        assert!(error.contains("T+1"), "{error}");
        assert!(error.contains("BR-249 audit_id="), "{error}");
        assert_eq!(inventory_failure_audit_count(&code), 1);
        assert_eq!(order_audit_count(&code), order_attempts_before);
    }

    #[test]
    #[serial_test::serial]
    fn aggregate_rejects_sqlite_time_modifiers_and_partial_times() {
        for (label, raw_timestamp) in [("SQLITE_NOW", "now"), ("TIME_ONLY", "12:34")] {
            let code = unique_code(label);
            let _guard = PaperSellGuard::new(vec![code.clone()]);
            insert_fill(
                &code,
                "非法成交时间",
                "buy",
                10.0,
                Some(10.0),
                100,
                raw_timestamp,
            );

            let error = aggregate_open_positions_at(date(2099, 1, 1))
                .expect_err("raw paper fill timestamp must be complete and immutable");

            assert!(
                error.contains("timestamp invalid") && error.contains("BR-249 audit_id="),
                "{raw_timestamp}: {error}"
            );
        }
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
