//! Registered business rules: BR-084.
//! v12 PR3-3.5: 虚拟盘成交模拟 (paper_trade).
//!
//! 设计: 虚拟腿只写 paper_trades, **零写 stock_position** (BR-023 硬性隔离).
//!        真实减仓走 position_adjustments (BR-024).
//!
//! 状态机: SignalTriggered → Filled / NotFilled / Invalidated
//!   - 涨停买 → NotFilled ("涨停不可买")
//!   - 跌停卖 → NotFilled ("跌停不可卖")
//!   - 停牌 → NotFilled ("停牌拒绝")
//!   - 滑点超 MAX_SLIPPAGE_PCT → Invalidated (v16.3 R2)
//!   - 正常 → Filled (fill_price = signal_price)
//!
//! plan_id 幂等: 用 plan_id 作为唯一键, 重复调用不重复插入.
//!
//! 费率/滑点复用 position_tracker const (:37-42) — 本 PR 不调, 仅写 signal_price.
//!
//! v16.3 Commit 1: evaluate 改签名接 quote_price, 加 5 态 Invalidated (滑点 > MAX_SLIPPAGE_PCT=2%)

use diesel::prelude::*;

use crate::database::DatabaseManager;
use crate::trading::risk_adapter::MAX_SLIPPAGE_PCT;

#[derive(Clone, diesel::QueryableByName)]
struct LedgerState {
    #[diesel(sql_type = diesel::sql_types::Text)]
    date: String,
    #[diesel(sql_type = diesel::sql_types::Double)]
    total_value: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    cash: f64,
    #[diesel(sql_type = diesel::sql_types::Double)]
    market_value: f64,
    #[diesel(sql_type = diesel::sql_types::Text)]
    created_at: String,
}

fn validate_ledger_state(
    ledger: &LedgerState,
    today: &str,
    now: chrono::NaiveDateTime,
) -> Result<(), String> {
    if ledger.date != today {
        return Err(format!(
            "account ledger stale trading day: snapshot={} today={today}",
            ledger.date
        ));
    }
    let created_at = chrono::NaiveDateTime::parse_from_str(&ledger.created_at, "%Y-%m-%d %H:%M:%S")
        .map_err(|error| format!("account ledger created_at invalid: {error}"))?;
    let age = now.signed_duration_since(created_at).num_seconds();
    if !(0..=30).contains(&age) {
        return Err(format!("account ledger stale: age_seconds={age}"));
    }
    if !ledger.total_value.is_finite()
        || ledger.total_value <= 0.0
        || !ledger.cash.is_finite()
        || ledger.cash < 0.0
        || ledger.cash > ledger.total_value
        || !ledger.market_value.is_finite()
        || ledger.market_value < 0.0
        || ledger.market_value > ledger.total_value
    {
        return Err(format!(
            "account ledger invalid: cash={} market_value={} total_value={}",
            ledger.cash, ledger.market_value, ledger.total_value
        ));
    }
    Ok(())
}

fn validate_position_snapshot(
    positions: &[crate::portfolio::Position],
    position_source_time: Option<chrono::DateTime<chrono::Local>>,
    ledger_market_value: f64,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), String> {
    if positions.is_empty() {
        if ledger_market_value.abs() > 0.005 {
            return Err(format!(
                "position snapshot is empty but ledger market_value={ledger_market_value}"
            ));
        }
    } else {
        let position_source_time = position_source_time
            .ok_or_else(|| "position snapshot is missing source time".to_string())?;
        if !crate::portfolio::position_source_is_fresh(position_source_time, now) {
            return Err(format!(
                "position snapshot stale: oldest_source_time={position_source_time}"
            ));
        }
    }
    Ok(())
}

fn position_pct(
    positions: &[crate::portfolio::Position],
    code: &str,
    quote_price: f64,
    total_value: f64,
) -> f64 {
    let shares = positions
        .iter()
        .filter(|position| position.code == code)
        .map(|position| position.shares)
        .sum::<u64>();
    shares as f64 * quote_price / total_value * 100.0
}

/// 虚拟盘状态
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PaperTradeStatus {
    SignalTriggered,
    Filled,
    NotFilled,
    Invalidated,
}

impl PaperTradeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            PaperTradeStatus::SignalTriggered => "SignalTriggered",
            PaperTradeStatus::Filled => "Filled",
            PaperTradeStatus::NotFilled => "NotFilled",
            PaperTradeStatus::Invalidated => "Invalidated",
        }
    }
}

/// 模拟方向
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Direction {
    Buy,
    Sell,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Buy => "buy",
            Direction::Sell => "sell",
        }
    }
}

/// 输入: 模拟成交信号
#[derive(Clone, Debug)]
pub struct PaperSignal {
    pub plan_id: String,
    pub code: String,
    pub name: String,
    pub direction: Direction,
    pub price: f64,
    pub quantity: u32,
    pub virtual_reason: String,
    /// 涨停一字板 (T 日触及涨停且不可买)
    pub is_limit_up: bool,
    /// 跌停一字板 (T 日触及跌停且不可卖)
    pub is_limit_down: bool,
    /// 停牌 (T 日停牌)
    pub is_suspended: bool,
    pub limit_up_price: Option<f64>,
    pub limit_down_price: Option<f64>,
    pub secondary_confirmed: bool,
    pub quote_observed_at: chrono::DateTime<chrono::Utc>,
    pub account_mode: String,
    pub data_mode: String,
}

/// 输出: 模拟结果
#[derive(Clone, Debug)]
pub struct PaperResult {
    pub status: PaperTradeStatus,
    pub fill_price: Option<f64>,
    pub not_fill_reason: Option<String>,
}

/// PR3-3.5 主评估: 涨停买/跌停卖/停牌 → NotFilled; v16.3 加滑点 → Invalidated; 否则 Filled
///
/// v16.3 R2 (滑点保护): quote_price > 0 时, |quote_price - signal.price| / signal.price > MAX_SLIPPAGE_PCT
/// → Invalidated (挂单价 vs 实际成交价不一致, 信号失真)
///
pub fn evaluate(signal: &PaperSignal, quote_price: f64) -> PaperResult {
    if !signal.price.is_finite()
        || signal.price <= 0.0
        || !quote_price.is_finite()
        || quote_price <= 0.0
        || signal.quantity == 0
        || !signal.quantity.is_multiple_of(100)
    {
        return PaperResult {
            status: PaperTradeStatus::Invalidated,
            fill_price: None,
            not_fill_reason: Some("价格或数量证据无效".to_string()),
        };
    }
    // 1. 停牌 → NotFilled
    if signal.is_suspended {
        return PaperResult {
            status: PaperTradeStatus::NotFilled,
            fill_price: None,
            not_fill_reason: Some("停牌拒绝".to_string()),
        };
    }

    // 2. 涨停买 → NotFilled
    if signal.direction == Direction::Buy && signal.is_limit_up {
        return PaperResult {
            status: PaperTradeStatus::NotFilled,
            fill_price: None,
            not_fill_reason: Some("涨停不可买".to_string()),
        };
    }

    // 3. 跌停卖 → NotFilled
    if signal.direction == Direction::Sell && signal.is_limit_down {
        return PaperResult {
            status: PaperTradeStatus::NotFilled,
            fill_price: None,
            not_fill_reason: Some("跌停不可卖".to_string()),
        };
    }

    // 4. v16.3 R2: 滑点保护. pre_trade_check 已保证两种价格均有效.
    let slippage = (quote_price - signal.price).abs() / signal.price * 100.0;
    if slippage > *MAX_SLIPPAGE_PCT {
        log::warn!(
            "[paper_trade] 滑点 {:.2}% 超过 MAX_SLIPPAGE_PCT={:.1}% (signal={}, quote={})",
            slippage,
            *MAX_SLIPPAGE_PCT,
            signal.price,
            quote_price
        );
        return PaperResult {
            status: PaperTradeStatus::Invalidated,
            fill_price: None,
            not_fill_reason: Some(format!(
                "滑点 {:.2}% 超过 {:.1}%",
                slippage, *MAX_SLIPPAGE_PCT
            )),
        };
    }

    // 5. 正常 → Filled (以信号价成交)
    PaperResult {
        status: PaperTradeStatus::Filled,
        fill_price: Some(signal.price),
        not_fill_reason: None,
    }
}

/// v16.3 review fix (Issue #5): 读真实 (cash, total, pos_pct) 给 risk_adapter 检查用.
/// lib 版, bin (push_templates) 与 lib (intraday_monitor / paper_engine) 共用.
///
/// Load a <=30-second account snapshot and derive the target position ratio.
pub fn portfolio_state(code: &str, quote_price: f64) -> Result<(f64, f64, f64), String> {
    if !quote_price.is_finite() || quote_price <= 0.0 {
        return Err(format!(
            "invalid quote price for portfolio state: {quote_price}"
        ));
    }

    let db = DatabaseManager::try_get().ok_or_else(|| "DB 未初始化".to_string())?;
    let mut conn = db
        .get_conn()
        .map_err(|error| format!("DB 连接失败: {error}"))?;
    let ledger = diesel::sql_query(
        "SELECT date, total_value, cash, market_value, created_at FROM ledger ORDER BY date DESC LIMIT 1",
    )
        .get_result::<LedgerState>(&mut conn)
        .map_err(|error| format!("account ledger unavailable: {error}"))?;
    let today = chrono::Local::now().date_naive().to_string();
    validate_ledger_state(&ledger, &today, chrono::Utc::now().naive_utc())?;
    let (positions, position_source_time) = crate::portfolio::get_positions_with_source_time()?;
    validate_position_snapshot(
        &positions,
        position_source_time,
        ledger.market_value,
        chrono::Utc::now(),
    )?;
    let pos_pct = position_pct(&positions, code, quote_price, ledger.total_value);
    Ok((ledger.cash, ledger.total_value, pos_pct))
}

/// 模拟成交结果 (含 DB 写入状态)
#[derive(Clone, Debug)]
pub struct PaperOutcome {
    /// 评估结果 (Filled / NotFilled / Invalidated)
    pub result: PaperResult,
    /// true = INSERT 实际写入; false = INSERT OR IGNORE 跳过 (plan_id 重复)
    pub inserted: bool,
}

fn persist_paper_trade_with_audit(
    conn: &mut diesel::sqlite::SqliteConnection,
    sql: &str,
    signal: &PaperSignal,
    result: &PaperResult,
    observed_at: &str,
) -> diesel::QueryResult<usize> {
    conn.transaction::<_, diesel::result::Error, _>(|conn| {
        let rows = diesel::sql_query(sql).execute(conn)?;
        let duplicate_reason = "duplicate paper plan id";
        let outcome = if rows > 0 && result.status == PaperTradeStatus::Filled {
            "Filled"
        } else {
            "Rejected"
        };
        let failure_reason = if rows == 0 {
            Some(duplicate_reason)
        } else {
            result.not_fill_reason.as_deref()
        };
        let audit = crate::database::order_audit::OrderAuditRecord {
            business_order_id: &signal.plan_id,
            source: "PaperTrade",
            decision_basis: &signal.virtual_reason,
            side: signal.direction.as_str(),
            code: &signal.code,
            requested_price: signal.price,
            execution_price: result.fill_price,
            quantity: i64::from(signal.quantity),
            quote_observed_at: Some(observed_at),
            outcome,
            failure_reason,
        };
        if crate::database::order_audit::insert_order_audit_query(conn, &audit)? != 1 {
            return Err(diesel::result::Error::RollbackTransaction);
        }
        Ok(rows)
    })
}

/// 模拟成交: 写 paper_trades (含 plan_id 幂等)
///
/// 返回 `PaperOutcome::inserted` 区分新建 vs 跳过 (plan_id 已存在).
/// 调用方据此决定是否启动 execution_tracking 跟踪 (PR3-3.5 fix).
///
/// v16.3 Commit 1 BREAKING: 签名加 4 参数 (quote_price, current_cash, total_value, current_position_pct)
/// 调用方: push_templates:3073 (D-01), push_templates:6223 (盘后资金)
pub fn simulate(
    signal: &PaperSignal,
    quote_price: f64,
    current_cash: f64,
    total_value: f64,
    current_position_pct: f64,
) -> Result<PaperOutcome, String> {
    let db = DatabaseManager::try_get()
        .ok_or_else(|| "BR-086 paper-order audit database is not initialized".to_string())?;
    if !db
        .reserve_business_order_id(&signal.plan_id)
        .map_err(|error| format!("BR-086 paper-order idempotency reservation: {error}"))?
    {
        let reason = "duplicate business order id within 60 seconds".to_string();
        let observed_at = signal.quote_observed_at.to_rfc3339();
        let audit = crate::database::order_audit::OrderAuditRecord {
            business_order_id: &signal.plan_id,
            source: "PaperTrade",
            decision_basis: &signal.virtual_reason,
            side: signal.direction.as_str(),
            code: &signal.code,
            requested_price: signal.price,
            execution_price: None,
            quantity: i64::from(signal.quantity),
            quote_observed_at: Some(&observed_at),
            outcome: "Rejected",
            failure_reason: Some(&reason),
        };
        db.record_order_audit(&audit)
            .map_err(|error| format!("{reason}; BR-086 duplicate audit failed: {error}"))?;
        return Err(reason);
    }

    // v16.3 R1+R2: pre-trade gate 4 项硬检查 (拒 → 不入 paper_trades, 不调 evaluate)
    if let Err(reason) = crate::trading::risk_adapter::pre_trade_check(
        signal,
        quote_price,
        current_cash,
        total_value,
        current_position_pct,
    ) {
        let observed_at = signal.quote_observed_at.to_rfc3339();
        let audit = crate::database::order_audit::OrderAuditRecord {
            business_order_id: &signal.plan_id,
            source: "PaperTrade",
            decision_basis: &signal.virtual_reason,
            side: signal.direction.as_str(),
            code: &signal.code,
            requested_price: signal.price,
            execution_price: None,
            quantity: i64::from(signal.quantity),
            quote_observed_at: Some(&observed_at),
            outcome: "Rejected",
            failure_reason: Some(&reason),
        };
        db.record_order_audit(&audit)
            .map_err(|audit_error| format!("{reason}; BR-086 audit failed: {audit_error}"))?;
        return Err(reason);
    }

    let result = evaluate(signal, quote_price);
    let mut conn = DatabaseManager::get()
        .get_conn()
        .map_err(|e| format!("DB 连接失败: {}", e))?;

    let esc = |s: &str| s.replace('\'', "''");
    let fill_price = result
        .fill_price
        .map(|v| v.to_string())
        .unwrap_or_else(|| "NULL".to_string());
    let not_fill_reason = result
        .not_fill_reason
        .as_deref()
        .map(|s| format!("'{}'", esc(s)))
        .unwrap_or_else(|| "NULL".to_string());

    // 使用 INSERT OR IGNORE 实现 plan_id 幂等 (依赖 uniq_paper_trades_plan_id)
    let sql = format!(
        "INSERT OR IGNORE INTO paper_trades \
         (plan_id, code, name, direction, price, quantity, status, fill_price, not_fill_reason, virtual_reason, account_mode, data_mode) \
         VALUES ('{}', '{}', '{}', '{}', {}, {}, '{}', {}, {}, '{}', '{}', '{}')",
        esc(&signal.plan_id),
        esc(&signal.code),
        esc(&signal.name),
        signal.direction.as_str(),
        signal.price,
        signal.quantity,
        result.status.as_str(),
        fill_price,
        not_fill_reason,
        esc(&signal.virtual_reason),
        esc(&signal.account_mode),
        esc(&signal.data_mode),
    );
    let observed_at = signal.quote_observed_at.to_rfc3339();
    let rows = persist_paper_trade_with_audit(&mut conn, &sql, signal, &result, &observed_at)
        .map_err(|e| format!("BR-086 audited paper trade transaction: {e}"))?;

    Ok(PaperOutcome {
        result,
        inserted: rows > 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal_default(is_limit_up: bool, is_limit_down: bool, is_suspended: bool) -> PaperSignal {
        PaperSignal {
            plan_id: "plan-001".to_string(),
            code: "TEST_CODE_688001".to_string(),
            name: "测试".to_string(),
            direction: Direction::Buy,
            price: 50.0,
            quantity: 100,
            virtual_reason: "NewsCatalyst".to_string(),
            is_limit_up,
            is_limit_down,
            is_suspended,
            limit_up_price: Some(55.0),
            limit_down_price: Some(45.0),
            secondary_confirmed: false,
            quote_observed_at: chrono::Utc::now(),
            account_mode: "Normal".to_string(),
            data_mode: "Full".to_string(),
        }
    }

    #[derive(QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        count: i64,
    }

    #[test]
    fn br086_chain_insert_failure_rolls_back_paper_trade() {
        let mut conn =
            diesel::sqlite::SqliteConnection::establish(":memory:").expect("in-memory SQLite");
        DatabaseManager::run_migrations_for_test(&mut conn).expect("test migrations");
        diesel::sql_query(
            "CREATE TRIGGER test_fail_paper_audit_chain_insert
             BEFORE INSERT ON order_audit_chain
             BEGIN SELECT RAISE(ABORT, 'TEST_CODE forced paper chain failure'); END",
        )
        .execute(&mut conn)
        .expect("install chain failure trigger");
        let mut signal = signal_default(false, false, false);
        signal.plan_id = "TEST_PLAN_BR086_ROLLBACK".to_string();
        let result = evaluate(&signal, signal.price);
        let sql = "INSERT INTO paper_trades
                   (plan_id, code, name, direction, price, quantity, status,
                    fill_price, not_fill_reason, virtual_reason, account_mode, data_mode)
                   VALUES ('TEST_PLAN_BR086_ROLLBACK', 'TEST_CODE_688001', '测试', 'buy',
                           50.0, 100, 'Filled', 50.0, NULL, 'NewsCatalyst', 'Normal', 'Full')";

        persist_paper_trade_with_audit(
            &mut conn,
            sql,
            &signal,
            &result,
            "2026-07-18T09:30:00+08:00",
        )
        .expect_err("chain failure must roll back paper row and audit row");
        for table in ["paper_trades", "order_audit", "order_audit_chain"] {
            let count = diesel::sql_query(format!("SELECT COUNT(*) AS count FROM {table}"))
                .get_result::<CountRow>(&mut conn)
                .expect("count rollback rows")
                .count;
            assert_eq!(count, 0, "{table} must be rolled back");
        }
    }

    #[test]
    fn portfolio_state_validators_reject_stale_or_inconsistent_account_evidence() {
        let now = chrono::NaiveDate::from_ymd_opt(2026, 7, 18)
            .unwrap()
            .and_hms_opt(2, 0, 30)
            .unwrap();
        let complete = LedgerState {
            date: "2026-07-18".into(),
            total_value: 100_000.0,
            cash: 40_000.0,
            market_value: 60_000.0,
            created_at: "2026-07-18 02:00:00".into(),
        };
        validate_ledger_state(&complete, "2026-07-18", now).expect("30-second boundary");

        let mut invalid = complete.clone();
        invalid.date = "2026-07-17".into();
        assert!(validate_ledger_state(&invalid, "2026-07-18", now)
            .expect_err("previous trading day is stale")
            .contains("stale trading day"));

        invalid = complete.clone();
        invalid.created_at = "not-a-time".into();
        assert!(validate_ledger_state(&invalid, "2026-07-18", now)
            .expect_err("invalid source time must fail")
            .contains("created_at invalid"));

        for created_at in ["2026-07-18 02:00:31", "2026-07-18 01:59:59"] {
            invalid = complete.clone();
            invalid.created_at = created_at.into();
            assert!(validate_ledger_state(&invalid, "2026-07-18", now)
                .expect_err("future or older-than-30-second ledger must fail")
                .contains("ledger stale"));
        }

        let invalid_values = [
            (f64::NAN, 40_000.0, 60_000.0),
            (0.0, 0.0, 0.0),
            (100_000.0, f64::NAN, 60_000.0),
            (100_000.0, -1.0, 60_000.0),
            (100_000.0, 100_001.0, 0.0),
            (100_000.0, 40_000.0, f64::NAN),
            (100_000.0, 40_000.0, -1.0),
            (100_000.0, 40_000.0, 100_001.0),
        ];
        for (total_value, cash, market_value) in invalid_values {
            invalid = complete.clone();
            invalid.total_value = total_value;
            invalid.cash = cash;
            invalid.market_value = market_value;
            assert!(validate_ledger_state(&invalid, "2026-07-18", now)
                .expect_err("invalid account amount must fail")
                .contains("ledger invalid"));
        }

        for quote in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(portfolio_state("TEST_CODE_600519", quote)
                .expect_err("invalid quote must fail before database access")
                .contains("invalid quote price"));
        }
    }

    #[test]
    fn portfolio_position_snapshot_requires_complete_fresh_source_evidence() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-07-18T02:00:30Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert!(validate_position_snapshot(&[], None, 0.0, now).is_ok());
        assert!(validate_position_snapshot(&[], None, 0.006, now)
            .expect_err("non-zero ledger market value needs positions")
            .contains("snapshot is empty"));

        let position = crate::portfolio::Position {
            code: "TEST_CODE_600519".into(),
            name: "测试持仓".into(),
            shares: 1_000,
            cost_price: 10.0,
            hard_stop: None,
            added_at: chrono::NaiveDate::from_ymd_opt(2026, 7, 17).unwrap(),
            status: crate::portfolio::PositionStatus::Holding,
            sector: "测试板块".into(),
            is_st: false,
            star_st: false,
        };
        assert!(
            validate_position_snapshot(std::slice::from_ref(&position), None, 10_000.0, now)
                .expect_err("non-empty snapshot requires source time")
                .contains("missing source time")
        );
        assert!(validate_position_snapshot(
            std::slice::from_ref(&position),
            Some((now - chrono::Duration::milliseconds(30_001)).with_timezone(&chrono::Local)),
            10_000.0,
            now,
        )
        .expect_err("stale position evidence must fail")
        .contains("snapshot stale"));
        validate_position_snapshot(
            std::slice::from_ref(&position),
            Some((now - chrono::Duration::seconds(30)).with_timezone(&chrono::Local)),
            10_000.0,
            now,
        )
        .expect("30-second position boundary");

        let mut second = position.clone();
        second.shares = 500;
        let unrelated = crate::portfolio::Position {
            code: "TEST_CODE_000001".into(),
            shares: 10_000,
            ..position
        };
        let pct = position_pct(&[second, unrelated], "TEST_CODE_600519", 20.0, 100_000.0);
        assert!((pct - 10.0).abs() < f64::EPSILON);
    }

    // ---- 涨停买必 NotFilled (PR3-3.5 硬性要求) ----

    #[test]
    fn limit_up_buy_returns_not_filled() {
        let r = evaluate(&signal_default(true, false, false), 50.0);
        assert_eq!(r.status, PaperTradeStatus::NotFilled);
        assert_eq!(r.not_fill_reason.as_deref(), Some("涨停不可买"));
        assert!(r.fill_price.is_none());
    }

    // ---- 跌停卖必 NotFilled ----

    #[test]
    fn limit_down_sell_returns_not_filled() {
        let mut s = signal_default(false, true, false);
        s.direction = Direction::Sell;
        let r = evaluate(&s, 50.0);
        assert_eq!(r.status, PaperTradeStatus::NotFilled);
        assert_eq!(r.not_fill_reason.as_deref(), Some("跌停不可卖"));
    }

    // ---- 停牌拒绝 ----

    #[test]
    fn suspended_returns_not_filled() {
        let r = evaluate(&signal_default(false, false, true), 50.0);
        assert_eq!(r.status, PaperTradeStatus::NotFilled);
        assert_eq!(r.not_fill_reason.as_deref(), Some("停牌拒绝"));
    }

    // ---- 正常 → Filled ----

    #[test]
    fn normal_returns_filled() {
        let r = evaluate(&signal_default(false, false, false), 50.0);
        assert_eq!(r.status, PaperTradeStatus::Filled);
        assert_eq!(r.fill_price, Some(50.0));
        assert!(r.not_fill_reason.is_none());
    }

    // ---- 优先级: 停牌优先于涨跌停 ----

    #[test]
    fn suspended_takes_priority() {
        // 同时: 停牌 + 涨停买 → NotFilled("停牌拒绝")
        let r = evaluate(&signal_default(true, false, true), 50.0);
        assert_eq!(r.not_fill_reason.as_deref(), Some("停牌拒绝"));
    }

    // ---- v16.3 R2: 滑点边界 case ----

    #[test]
    fn invalidated_when_slippage_exceeds_2pct() {
        // signal=50, quote=51.5 → 滑点 3% → Invalidated
        let r = evaluate(&signal_default(false, false, false), 51.5);
        assert_eq!(r.status, PaperTradeStatus::Invalidated);
        assert!(r.not_fill_reason.as_deref().unwrap().contains("滑点"));
    }

    #[test]
    fn filled_when_slippage_within_2pct() {
        // signal=50, quote=50.25 → 滑点 0.5% → Filled
        let r = evaluate(&signal_default(false, false, false), 50.25);
        assert_eq!(r.status, PaperTradeStatus::Filled);
    }

    #[test]
    fn filled_at_slippage_boundary_2pct() {
        // signal=50, quote=51.0 → 滑点 2.0% → Filled (边界 ≤ 不 >)
        let r = evaluate(&signal_default(false, false, false), 51.0);
        assert_eq!(r.status, PaperTradeStatus::Filled);
    }

    #[test]
    fn invalidated_at_slippage_2_5pct() {
        // signal=50, quote=51.25 → 滑点 2.5% → Invalidated
        let r = evaluate(&signal_default(false, false, false), 51.25);
        assert_eq!(r.status, PaperTradeStatus::Invalidated);
    }

    #[test]
    fn invalidated_when_quote_price_zero() {
        let r = evaluate(&signal_default(false, false, false), 0.0);
        assert_eq!(r.status, PaperTradeStatus::Invalidated);
        assert!(r.fill_price.is_none());
    }

    #[test]
    fn filled_sell_with_low_slippage() {
        // 卖出方向, 滑点 0.3% (downward, quote < signal)
        let mut s = signal_default(false, false, false);
        s.direction = Direction::Sell;
        let r = evaluate(&s, 49.85); // |49.85-50|/50 = 0.3%
        assert_eq!(r.status, PaperTradeStatus::Filled);
    }

    // ---- 状态字符串 ----

    #[test]
    fn status_strings() {
        assert_eq!(PaperTradeStatus::Filled.as_str(), "Filled");
        assert_eq!(PaperTradeStatus::NotFilled.as_str(), "NotFilled");
        assert_eq!(PaperTradeStatus::Invalidated.as_str(), "Invalidated");
        assert_eq!(
            PaperTradeStatus::SignalTriggered.as_str(),
            "SignalTriggered"
        );
    }

    #[test]
    fn direction_strings() {
        assert_eq!(Direction::Buy.as_str(), "buy");
        assert_eq!(Direction::Sell.as_str(), "sell");
    }

    // ---- PaperOutcome.inserted 字段 (Bug #2 fix) ----

    #[test]
    fn paper_outcome_struct_fields() {
        // PaperOutcome 必须含 inserted 字段, 调用方据此决定是否启动 T+1 跟踪
        let o = PaperOutcome {
            result: PaperResult {
                status: PaperTradeStatus::Filled,
                fill_price: Some(10.0),
                not_fill_reason: None,
            },
            inserted: true,
        };
        assert!(o.inserted);
        assert!(matches!(o.result.status, PaperTradeStatus::Filled));
    }

    #[test]
    fn paper_outcome_inserted_flag_semantic() {
        // inserted=true: 实际写入 (rows_affected > 0)
        // inserted=false: plan_id 已存在 (rows_affected = 0, INSERT OR IGNORE 跳过)
        // 调用方: inserted=true 才启动 execution_tracking
        let inserted_true = PaperOutcome {
            result: PaperResult {
                status: PaperTradeStatus::Filled,
                fill_price: Some(10.0),
                not_fill_reason: None,
            },
            inserted: true,
        };
        let inserted_false = PaperOutcome {
            result: PaperResult {
                status: PaperTradeStatus::NotFilled,
                fill_price: None,
                not_fill_reason: Some("涨停不可买".to_string()),
            },
            inserted: false,
        };
        assert!(inserted_true.inserted, "新建场景应 inserted=true");
        assert!(
            !inserted_false.inserted,
            "重复 plan_id 应 inserted=false (避免假成功)"
        );
    }

    #[test]
    fn br086_rejected_paper_attempt_still_reserves_business_id() {
        let _ = DatabaseManager::init(None);
        let mut signal = signal_default(false, false, false);
        signal.plan_id = format!(
            "TEST_CODE_REJECTED_PLAN_{}_{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        );
        signal.quantity = 99;

        let first = simulate(&signal, 50.0, 100_000.0, 100_000.0, 0.0)
            .expect_err("invalid lot must be rejected");
        assert!(first.contains("100"));
        let second = simulate(&signal, 50.0, 100_000.0, 100_000.0, 0.0)
            .expect_err("same rejected business id must be deduplicated");
        assert!(second.contains("duplicate business order id within 60 seconds"));
    }
}
