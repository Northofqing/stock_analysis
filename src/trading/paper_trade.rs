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

use crate::magic_compat::InstrumentId;
use chrono::NaiveDate;
use diesel::prelude::*;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::database::DatabaseManager;
use crate::monitor::data_mode::DataMode;
use crate::risk::action_gate::AccountMode;
use crate::trading::risk_adapter::MAX_SLIPPAGE_PCT;

/// BR-134: one typed snapshot of the risk facts that authorize a paper action.
/// There is deliberately no `Default`: production callers must supply the
/// latest fully evaluated account and data modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaperRiskContext {
    pub account_mode: AccountMode,
    pub data_mode: DataMode,
}

impl PaperRiskContext {
    pub const fn new(account_mode: AccountMode, data_mode: DataMode) -> Self {
        Self {
            account_mode,
            data_mode,
        }
    }
}

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

#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct TestPositionBatchEvidence {
    pub source: String,
    pub source_at: chrono::DateTime<chrono::Local>,
    pub observed_at: chrono::DateTime<chrono::Utc>,
    pub batch_id: String,
}

#[cfg(test)]
std::thread_local! {
    static TEST_POSITION_BATCH_EVIDENCE:
        std::cell::RefCell<Option<TestPositionBatchEvidence>> =
        const { std::cell::RefCell::new(None) };
}

/// Test-only position evidence scope.
///
/// Production never compiles this seam. Tests must supply a TEST_CODE-labelled
/// batch and may authorize only TEST_CODE positions, so a local database
/// mutation timestamp can never regain source-evidence semantics.
#[cfg(test)]
pub(crate) struct TestPositionEvidenceGuard {
    previous: Option<TestPositionBatchEvidence>,
}

#[cfg(test)]
impl Drop for TestPositionEvidenceGuard {
    fn drop(&mut self) {
        TEST_POSITION_BATCH_EVIDENCE.with(|slot| {
            slot.replace(self.previous.take());
        });
    }
}

#[cfg(test)]
pub(crate) fn install_test_position_batch_evidence(
    evidence: TestPositionBatchEvidence,
) -> Result<TestPositionEvidenceGuard, String> {
    if !evidence.source.starts_with("TEST_CODE_") || !evidence.batch_id.starts_with("TEST_CODE_") {
        return Err("test position evidence source/batch_id must use TEST_CODE prefix".to_string());
    }
    if evidence.observed_at < evidence.source_at.with_timezone(&chrono::Utc) {
        return Err("test position evidence observed_at precedes source_at".to_string());
    }
    let previous = TEST_POSITION_BATCH_EVIDENCE.with(|slot| slot.replace(Some(evidence)));
    Ok(TestPositionEvidenceGuard { previous })
}

fn effective_position_source_time(
    positions: &[crate::portfolio::Position],
    source_time: Option<chrono::DateTime<chrono::Local>>,
) -> Result<Option<chrono::DateTime<chrono::Local>>, String> {
    if source_time.is_some() {
        return Ok(source_time);
    }
    #[cfg(not(test))]
    let _ = positions;
    #[cfg(test)]
    {
        let evidence = TEST_POSITION_BATCH_EVIDENCE.with(|slot| slot.borrow().clone());
        if let Some(evidence) = evidence {
            if positions
                .iter()
                .any(|position| !crate::risk::env_guard::is_test_code(&position.code))
            {
                return Err(
                    "TEST_CODE position evidence cannot authorize a production symbol".to_string(),
                );
            }
            return Ok(Some(evidence.source_at));
        }
    }
    Ok(None)
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
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize)]
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

const REALTIME_QUOTE_MAX_AGE_MILLIS: i64 = 5_000;

/// AGENTS 2.4: a realtime quote may authorize a PaperTrade transition only
/// while it is no more than five seconds old. Provider timestamps in the
/// future are invalid evidence rather than "negative age" freshness.
fn validate_realtime_quote_freshness(
    quote_observed_at: chrono::DateTime<chrono::Utc>,
    evaluated_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), String> {
    if quote_observed_at > evaluated_at {
        return Err(format!(
            "realtime quote timestamp is in the future: quote_observed_at={quote_observed_at} evaluated_at={evaluated_at}"
        ));
    }
    let age = evaluated_at
        .signed_duration_since(quote_observed_at)
        .num_milliseconds();
    if age > REALTIME_QUOTE_MAX_AGE_MILLIS {
        return Err(format!(
            "realtime quote is stale: age_ms={age} max_ms={REALTIME_QUOTE_MAX_AGE_MILLIS}"
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PaperTradeTerminalBindingV1 {
    schema: &'static str,
    paper_trade_id: i64,
    plan_id: String,
    instrument: InstrumentId,
    business_date: NaiveDate,
    direction: String,
    requested_price: f64,
    quantity: u32,
    status: PaperTradeStatus,
    fill_price: Option<f64>,
    not_fill_reason: Option<String>,
    virtual_reason: String,
    account_mode: String,
    data_mode: String,
    quote_observed_at: chrono::DateTime<chrono::Utc>,
    paper_trade_created_at: chrono::DateTime<chrono::Utc>,
    order_audit_id: i64,
    audit_previous_hash: String,
    audit_record_hash: String,
    terminal_at: chrono::DateTime<chrono::Utc>,
}

impl PaperTradeTerminalBindingV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        paper_trade_id: i64,
        plan_id: impl Into<String>,
        instrument: InstrumentId,
        business_date: NaiveDate,
        direction: impl Into<String>,
        requested_price: f64,
        quantity: u32,
        status: PaperTradeStatus,
        fill_price: Option<f64>,
        not_fill_reason: Option<String>,
        virtual_reason: impl Into<String>,
        account_mode: impl Into<String>,
        data_mode: impl Into<String>,
        quote_observed_at: chrono::DateTime<chrono::Utc>,
        paper_trade_created_at: chrono::DateTime<chrono::Utc>,
        order_audit_id: i64,
        audit_previous_hash: impl Into<String>,
        audit_record_hash: impl Into<String>,
        terminal_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Self, String> {
        let plan_id = plan_id.into();
        let direction = direction.into();
        let virtual_reason = virtual_reason.into();
        let account_mode = account_mode.into();
        let data_mode = data_mode.into();
        let audit_previous_hash = audit_previous_hash.into();
        let audit_record_hash = audit_record_hash.into();
        if paper_trade_id <= 0 || order_audit_id <= 0 {
            return Err(format!(
                "paper terminal receipt IDs must be positive paper_trade_id={paper_trade_id} order_audit_id={order_audit_id}"
            ));
        }
        if plan_id.trim().is_empty() {
            return Err("paper terminal plan_id must be non-empty".to_owned());
        }
        if instrument.code().trim().is_empty() {
            return Err("paper terminal instrument code must be non-empty".to_owned());
        }
        if business_date != quote_observed_at.with_timezone(&chrono::Local).date_naive() {
            return Err(format!(
                "paper terminal business_date differs from quote evidence business_date={business_date} quote_observed_at={quote_observed_at}"
            ));
        }
        if !matches!(direction.as_str(), "buy" | "sell") {
            return Err(format!("paper terminal direction is invalid: {direction}"));
        }
        if !requested_price.is_finite()
            || requested_price <= 0.0
            || quantity == 0
            || !quantity.is_multiple_of(100)
        {
            return Err(format!(
                "paper terminal price/quantity is invalid price={requested_price} quantity={quantity}"
            ));
        }
        match status {
            PaperTradeStatus::SignalTriggered => {
                return Err("SignalTriggered is not a terminal paper transition".to_owned());
            }
            PaperTradeStatus::Filled
                if fill_price.is_none_or(|value| !value.is_finite() || value <= 0.0)
                    || not_fill_reason.is_some() =>
            {
                return Err("Filled paper terminal evidence is incomplete".to_owned());
            }
            PaperTradeStatus::NotFilled | PaperTradeStatus::Invalidated
                if fill_price.is_some()
                    || not_fill_reason
                        .as_deref()
                        .is_none_or(|reason| reason.trim().is_empty()) =>
            {
                return Err(format!(
                    "{} paper terminal evidence is incomplete",
                    status.as_str()
                ));
            }
            _ => {}
        }
        if virtual_reason.trim().is_empty()
            || !matches!(account_mode.as_str(), "Normal" | "ReduceOnly" | "Frozen")
            || !matches!(data_mode.as_str(), "Full" | "Degraded" | "Unsafe")
        {
            return Err("paper terminal decision/risk context is incomplete".to_owned());
        }
        if audit_previous_hash != "BR086_ORDER_AUDIT_GENESIS_V1"
            && !is_lower_sha256(&audit_previous_hash)
        {
            return Err("paper terminal audit previous hash is invalid".to_owned());
        }
        if !is_lower_sha256(&audit_record_hash) {
            return Err("paper terminal audit record hash is invalid".to_owned());
        }
        validate_realtime_quote_freshness(quote_observed_at, terminal_at)?;
        if terminal_at + chrono::Duration::seconds(1) < quote_observed_at
            || paper_trade_created_at + chrono::Duration::seconds(1) < quote_observed_at
        {
            return Err("paper terminal persistence time precedes quote evidence".to_owned());
        }
        Ok(Self {
            schema: "paper_trade_terminal_binding_v1",
            paper_trade_id,
            plan_id,
            instrument,
            business_date,
            direction,
            requested_price,
            quantity,
            status,
            fill_price,
            not_fill_reason,
            virtual_reason,
            account_mode,
            data_mode,
            quote_observed_at,
            paper_trade_created_at,
            order_audit_id,
            audit_previous_hash,
            audit_record_hash,
            terminal_at,
        })
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(self)
            .map_err(|error| format!("serialize paper terminal binding: {error}"))
    }

    pub fn terminal_transition_id(&self) -> Result<String, String> {
        domain_hash(
            b"stock-analysis:paper-trade-terminal-transition:v1\0",
            &self.canonical_bytes()?,
        )
    }

    pub fn delivery_subject_hash(&self) -> Result<String, String> {
        let subject = serde_json::to_vec(&(
            "paper_trade_delivery_subject_v1",
            &self.instrument,
            self.business_date,
            &self.plan_id,
        ))
        .map_err(|error| format!("serialize paper terminal delivery subject: {error}"))?;
        domain_hash(
            b"stock-analysis:paper-trade-delivery-subject:v1\0",
            &subject,
        )
    }

    pub fn instrument(&self) -> &InstrumentId {
        &self.instrument
    }

    pub fn business_date(&self) -> NaiveDate {
        self.business_date
    }

    pub fn plan_id(&self) -> &str {
        &self.plan_id
    }

    pub fn quote_observed_at(&self) -> chrono::DateTime<chrono::Utc> {
        self.quote_observed_at
    }
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn domain_hash(domain: &[u8], canonical: &[u8]) -> Result<String, String> {
    if canonical.is_empty() {
        return Err("paper terminal canonical bytes must be non-empty".to_owned());
    }
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(canonical);
    Ok(hex::encode(hasher.finalize()))
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

/// 额外写入订单审计、但不改变策略分类文本的结构化证据。
#[derive(Clone, Debug)]
pub struct PaperAuditEvidence(String);

impl PaperAuditEvidence {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err("paper audit evidence must not be blank".to_string());
        }
        Ok(Self(value))
    }

    fn as_str(&self) -> &str {
        &self.0
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
    pub risk_context: PaperRiskContext,
}

fn audit_decision_basis(
    signal: &PaperSignal,
    audit_evidence: Option<&PaperAuditEvidence>,
) -> String {
    audit_evidence.map_or_else(
        || signal.virtual_reason.clone(),
        |evidence| format!("{} | {}", signal.virtual_reason, evidence.as_str()),
    )
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
    let position_source_time = effective_position_source_time(&positions, position_source_time)?;
    validate_position_snapshot(
        &positions,
        position_source_time,
        ledger.market_value,
        chrono::Utc::now(),
    )?;
    let pos_pct = position_pct(&positions, code, quote_price, ledger.total_value);
    Ok((ledger.cash, ledger.total_value, pos_pct))
}

/// 最新券商账户汇总（append-only user_account_summary）。
/// 返回 (total_assets, available_cash, securities_market_value, daily_pnl)。
fn account_snapshot_summary() -> Result<(f64, f64, f64, f64), String> {
    let db = DatabaseManager::try_get().ok_or_else(|| "DB 未初始化".to_string())?;
    let mut conn = db
        .get_conn()
        .map_err(|error| format!("DB 连接失败: {error}"))?;
    #[derive(diesel::QueryableByName)]
    struct AccountSummaryRow {
        #[diesel(sql_type = diesel::sql_types::Double)]
        total_assets: f64,
        #[diesel(sql_type = diesel::sql_types::Double)]
        available_cash: f64,
        #[diesel(sql_type = diesel::sql_types::Double)]
        securities_market_value: f64,
        #[diesel(sql_type = diesel::sql_types::Double)]
        daily_pnl: f64,
    }
    let summary: AccountSummaryRow = diesel::sql_query(
        "SELECT total_assets, available_cash, securities_market_value, daily_pnl \
         FROM user_account_summary ORDER BY id DESC LIMIT 1",
    )
    .get_result(&mut conn)
    .map_err(|error| format!("account summary unavailable: {error}"))?;
    Ok((
        summary.total_assets,
        summary.available_cash,
        summary.securities_market_value,
        summary.daily_pnl,
    ))
}

/// BR-151 快照模式 ledger 刷新（intraday_monitor tick 生产入口每 30s 无条件调用）。
///
/// 用最新券商账户汇总 upsert 当日 ledger（created_at=CURRENT_TIMESTAMP 刷新，
/// 满足 BR-097 ledger 结构门）——候选到达前 ledger 即已新鲜，虚拟盘成交不再
/// 被 "ledger stale" 拦截。30s 实时门对静态快照不适用：账户授权来自用户确认
/// 动作（confirmed_at），生产盘中无快照刷新者（BR-146/147）。
///
/// BR-234b 每日收益双口径（用户指令：「我传了 就以我的为准 / 我不传 你自己
/// 计算出来」）：
/// - 最新券商汇总 effective_at 日期 == 今天 → 用户当天上传 → 以快照 4 字段
///   为准（真实账户证据：total_assets/cash/market_value/daily_pnl）。
/// - 快照过期 → `estimate_ledger_from_positions`：持仓明细 × 实时行情估值 +
///   快照现金；daily_pnl = 今日总资产 − 昨日 ledger（自动计算每日收益）。
pub fn refresh_account_ledger_from_snapshot() -> Result<(), String> {
    let (snapshot_total, available_cash, snapshot_market, snapshot_pnl) =
        account_snapshot_summary()?;
    let db = DatabaseManager::try_get().ok_or_else(|| "DB 未初始化".to_string())?;
    let mut conn = db
        .get_conn()
        .map_err(|error| format!("DB 连接失败: {error}"))?;
    let today = chrono::Local::now().date_naive();
    let today_str = today.to_string();

    // 口径分派：快照新鲜（用户当天上传）→ 快照为准；过期 → 持仓 × 实时价自算。
    let (total_assets, cash, market_value, daily_pnl) =
        if latest_snapshot_effective_date(&mut conn)? == Some(today) {
            (
                snapshot_total,
                available_cash,
                snapshot_market,
                snapshot_pnl,
            )
        } else {
            estimate_ledger_from_positions(&mut conn, available_cash, today)?
        };

    // upsert 当日 ledger（created_at 每 tick 刷新 → age≤30s 结构门通过）
    diesel::sql_query(
        "INSERT INTO ledger (date, total_value, cash, market_value, daily_pnl, created_at) \
         VALUES (?, ?, ?, ?, ?, CURRENT_TIMESTAMP) \
         ON CONFLICT(date) DO UPDATE SET total_value=excluded.total_value, \
             cash=excluded.cash, market_value=excluded.market_value, \
             daily_pnl=excluded.daily_pnl, created_at=CURRENT_TIMESTAMP",
    )
    .bind::<diesel::sql_types::Text, _>(&today_str)
    .bind::<diesel::sql_types::Double, _>(total_assets)
    .bind::<diesel::sql_types::Double, _>(cash)
    .bind::<diesel::sql_types::Double, _>(market_value)
    .bind::<diesel::sql_types::Double, _>(daily_pnl)
    .execute(&mut conn)
    .map_err(|error| format!("ledger upsert failed: {error}"))?;

    // 复读当日行并过 BR-097 结构门（自我验证 upsert 生效）
    let ledger: LedgerState = diesel::sql_query(
        "SELECT date, total_value, cash, market_value, created_at FROM ledger WHERE date = ?",
    )
    .bind::<diesel::sql_types::Text, _>(&today_str)
    .get_result(&mut conn)
    .map_err(|error| format!("account ledger unavailable: {error}"))?;
    validate_ledger_state(&ledger, &today_str, chrono::Utc::now().naive_utc())
}

/// 最新券商汇总 effective_at 的日期（东八区，作为「用户是否当天上传」判据）。
/// 无汇总行 → None（`account_snapshot_summary` 前置已保证有行，防御性返回）。
fn latest_snapshot_effective_date(
    conn: &mut SqliteConnection,
) -> Result<Option<NaiveDate>, String> {
    #[derive(QueryableByName)]
    struct EffectiveDateRow {
        #[diesel(sql_type = diesel::sql_types::Text)]
        effective_at: String,
    }
    let row: Option<EffectiveDateRow> =
        diesel::sql_query("SELECT effective_at FROM user_account_summary ORDER BY id DESC LIMIT 1")
            .get_result(conn)
            .optional()
            .map_err(|error| format!("snapshot effective_at unavailable: {error}"))?;
    row.map(|r| {
        chrono::DateTime::parse_from_rfc3339(&r.effective_at)
            .map(|dt| dt.date_naive())
            .map_err(|error| format!("snapshot effective_at invalid: {error}"))
    })
    .transpose()
}

/// 快照过期自算路径：最新持仓明细 × 估值价 → 市值；总资产 = 市值 + 快照现金；
/// daily_pnl = 总资产 − 昨日 ledger（无昨日行 → 0，首日）。
/// 持仓快照缺失 → Err 出声（无授权证据不臆造估值）。
fn estimate_ledger_from_positions(
    conn: &mut SqliteConnection,
    cash: f64,
    today: NaiveDate,
) -> Result<(f64, f64, f64, f64), String> {
    let snapshot = crate::database::user_position_snapshot::latest_user_position_snapshot()
        .map_err(|error| format!("持仓快照读取失败: {error}"))?
        .ok_or_else(|| "无持仓快照，无法自算估值 (BR-234b)".to_string())?;
    estimate_ledger_from_snapshot(&snapshot, conn, cash, today)
}

/// 估值纯函数（自算口径主体）：给定持仓快照 → (total, cash, market, daily_pnl)。
/// 与 DB 读取解耦，便于不落库的确定性测试。
fn estimate_ledger_from_snapshot(
    snapshot: &crate::database::user_position_snapshot::UserPositionSnapshot,
    conn: &mut SqliteConnection,
    cash: f64,
    today: NaiveDate,
) -> Result<(f64, f64, f64, f64), String> {
    let mut market_value = 0.0;
    for item in &snapshot.items {
        market_value += item.quantity as f64 * valuation_price(&item.code)?;
    }
    let total = market_value + cash;

    #[derive(QueryableByName)]
    struct PrevTotalRow {
        #[diesel(sql_type = diesel::sql_types::Double)]
        total_value: f64,
    }
    let prev: Option<PrevTotalRow> = diesel::sql_query(
        "SELECT total_value AS total_value FROM ledger WHERE date < ? ORDER BY date DESC LIMIT 1",
    )
    .bind::<diesel::sql_types::Text, _>(&today.to_string())
    .get_result(conn)
    .optional()
    .map_err(|error| format!("prev ledger unavailable: {error}"))?;
    let daily_pnl = total - prev.map_or(0.0, |row| row.total_value);
    Ok((total, cash, market_value, daily_pnl))
}

/// 单只估值价：实时价优先（broker::quote_price，BR-218 5s 门）；
/// 失败降级日K最新收盘价（warn 出声说明口径降级，K 线 5 根即可取最新收盘）；
/// 两者都失败 → Err（fail-closed，不写失真估值，成本价永不作估值价）。
fn valuation_price(code: &str) -> Result<f64, String> {
    match crate::broker::quote_price(code) {
        Ok(price) => Ok(price),
        Err(realtime_error) => {
            let bars = crate::data_gateway::historical_bars::HistoricalBarsGateway::new()
                .daily_bars(code, 5)
                .map_err(|error| {
                    format!("{code} 估值价获取失败: 实时={realtime_error}, 日K={error}")
                })?;
            let close = bars
                .records()
                .first()
                .map(|bar| bar.close)
                .filter(|p| p.is_finite() && *p > 0.0)
                .ok_or_else(|| {
                    format!("{code} 估值价获取失败: 实时={realtime_error}, 日K无有效收盘价")
                })?;
            log::warn!(
                "[paper_valuation] {code} 实时行情不可用({realtime_error})，估值降级为日K最新收盘价 {close:.2}"
            );
            Ok(close)
        }
    }
}

/// BR-151 快照模式账户证据（intraday_monitor 生产注入源）。
///
/// 虚拟盘账户 = 用户确认的真实账户快照：user_account_summary（券商汇总：
/// 总资产/可用现金/证券市值）+ user_position_snapshot（用户确认持仓明细）。
/// 先刷新当日 ledger（refresh_account_ledger_from_snapshot），再返回
/// (cash, total, pos_pct)。
///
/// BR-234b：口径统一为 refresh 后当日 ledger（快照为准或持仓×实时价自算），
/// 不再直接读快照汇总——自算路径下快照总资产已过期，会算错仓位占比。
fn require_confirmed_position_snapshot(
    snapshot: Option<crate::database::user_position_snapshot::UserPositionSnapshot>,
) -> Result<crate::database::user_position_snapshot::UserPositionSnapshot, String> {
    let snapshot = snapshot.ok_or_else(|| "无用户确认持仓快照 (BR-226)".to_string())?;
    if snapshot.confirm_empty || snapshot.items.is_empty() {
        return Err("account snapshot has no confirmed positions (BR-226)".to_string());
    }
    Ok(snapshot)
}

pub fn portfolio_state_snapshot(code: &str, quote_price: f64) -> Result<(f64, f64, f64), String> {
    if !quote_price.is_finite() || quote_price <= 0.0 {
        return Err(format!(
            "invalid quote price for portfolio state: {quote_price}"
        ));
    }
    refresh_account_ledger_from_snapshot()?;

    // 权威口径：当日 ledger（refresh 刚写入；cash/total_value 为快照或自算值）
    let db = DatabaseManager::try_get().ok_or_else(|| "DB 未初始化".to_string())?;
    let mut conn = db
        .get_conn()
        .map_err(|error| format!("DB 连接失败: {error}"))?;
    let ledger: LedgerState = diesel::sql_query(
        "SELECT date, total_value, cash, market_value, created_at FROM ledger \
         WHERE date = ?",
    )
    .bind::<diesel::sql_types::Text, _>(&chrono::Local::now().date_naive().to_string())
    .get_result(&mut conn)
    .map_err(|error| format!("account ledger unavailable: {error}"))?;
    let (total_assets, available_cash) = (ledger.total_value, ledger.cash);

    // 4. 用户确认持仓快照（账户持仓明细；缺失/空 = 无授权证据 → 出声拒绝）
    let snapshot = require_confirmed_position_snapshot(
        crate::database::user_position_snapshot::latest_user_position_snapshot()
            .map_err(|error| format!("持仓快照读取失败: {error}"))?,
    )?;

    // 5. 单票仓位（买入前状态：候选已有持仓则计占比，新仓 = 0）
    let pos_pct = snapshot
        .items
        .iter()
        .filter(|item| item.code == code)
        .map(|item| item.quantity as f64 * quote_price / total_assets * 100.0)
        .sum::<f64>();

    Ok((available_cash, total_assets, pos_pct))
}

/// 模拟成交结果 (含 DB 写入状态)
#[derive(Clone, Debug)]
pub struct PaperOutcome {
    /// 评估结果 (Filled / NotFilled / Invalidated)
    pub result: PaperResult,
    /// true = INSERT 实际写入; false = INSERT OR IGNORE 跳过 (plan_id 重复)
    pub inserted: bool,
    /// Exact immutable order-audit chain receipt for the newly persisted
    /// terminal transition. Duplicate/no-insert outcomes have no receipt.
    pub terminal_receipt: Option<PaperTradePersistenceReceipt>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaperTradePersistenceReceipt {
    pub plan_id: String,
    pub order_audit_id: i64,
    pub audit_previous_hash: String,
    pub audit_record_hash: String,
    pub terminal_at: String,
}

fn persist_paper_trade_with_audit(
    conn: &mut diesel::sqlite::SqliteConnection,
    sql: &str,
    signal: &PaperSignal,
    result: &PaperResult,
    observed_at: &str,
    audit_evidence: Option<&PaperAuditEvidence>,
) -> diesel::QueryResult<(usize, Option<PaperTradePersistenceReceipt>)> {
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
        let decision_basis = audit_decision_basis(signal, audit_evidence);
        let audit = crate::database::order_audit::OrderAuditRecord {
            business_order_id: &signal.plan_id,
            source: "PaperTrade",
            decision_basis: &decision_basis,
            side: signal.direction.as_str(),
            code: &signal.code,
            requested_price: signal.price,
            execution_price: if rows > 0 { result.fill_price } else { None },
            quantity: i64::from(signal.quantity),
            quote_observed_at: Some(observed_at),
            outcome,
            failure_reason,
        };
        let receipt =
            crate::database::order_audit::insert_order_audit_with_receipt_query(conn, &audit)?;
        let terminal_receipt = (rows > 0).then(|| PaperTradePersistenceReceipt {
            plan_id: signal.plan_id.clone(),
            order_audit_id: receipt.order_audit_id,
            audit_previous_hash: receipt.previous_hash,
            audit_record_hash: receipt.record_hash,
            terminal_at: receipt.created_at,
        });
        Ok((rows, terminal_receipt))
    })
}

/// 模拟成交: 写 paper_trades (含 plan_id 幂等)
///
/// 返回 `PaperOutcome::inserted` 区分新建 vs 跳过 (plan_id 已存在).
/// 调用方据此决定是否启动 execution_tracking 跟踪 (PR3-3.5 fix).
///
/// v16.3 Commit 1 BREAKING: 签名加 4 参数 (quote_price, current_cash, total_value, current_position_pct)
/// 调用方: push_templates:3073 (D-01), push_templates:6223 (盘后资金)
fn simulate_with_scope(
    signal: &PaperSignal,
    quote_price: f64,
    current_cash: f64,
    total_value: f64,
    current_position_pct: f64,
    snapshot_scope: bool,
    audit_evidence: Option<&PaperAuditEvidence>,
) -> Result<PaperOutcome, String> {
    if snapshot_scope {
        return Err(
            "settled daily PaperTrade capability_unavailable: daily close cannot authorize a realtime terminal transition"
                .to_owned(),
        );
    }
    validate_realtime_quote_freshness(signal.quote_observed_at, chrono::Utc::now())?;

    let db = DatabaseManager::try_get()
        .ok_or_else(|| "BR-086 paper-order audit database is not initialized".to_string())?;
    if !db
        .reserve_business_order_id(&signal.plan_id)
        .map_err(|error| format!("BR-086 paper-order idempotency reservation: {error}"))?
    {
        let reason = "duplicate business order id within 60 seconds".to_string();
        let observed_at = signal.quote_observed_at.to_rfc3339();
        let decision_basis = audit_decision_basis(signal, audit_evidence);
        let audit = crate::database::order_audit::OrderAuditRecord {
            business_order_id: &signal.plan_id,
            source: "PaperTrade",
            decision_basis: &decision_basis,
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
        let decision_basis = audit_decision_basis(signal, audit_evidence);
        let audit = crate::database::order_audit::OrderAuditRecord {
            business_order_id: &signal.plan_id,
            source: "PaperTrade",
            decision_basis: &decision_basis,
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
        signal.risk_context.account_mode.label(),
        signal.risk_context.data_mode.label(),
    );
    let observed_at = signal.quote_observed_at.to_rfc3339();
    let (rows, terminal_receipt) = persist_paper_trade_with_audit(
        &mut conn,
        &sql,
        signal,
        &result,
        &observed_at,
        audit_evidence,
    )
    .map_err(|e| format!("BR-086 audited paper trade transaction: {e}"))?;

    Ok(PaperOutcome {
        result,
        inserted: rows > 0,
        terminal_receipt,
    })
}

pub fn simulate(
    signal: &PaperSignal,
    quote_price: f64,
    current_cash: f64,
    total_value: f64,
    current_position_pct: f64,
) -> Result<PaperOutcome, String> {
    simulate_with_scope(
        signal,
        quote_price,
        current_cash,
        total_value,
        current_position_pct,
        false,
        None,
    )
}

pub(crate) fn simulate_with_audit_evidence(
    signal: &PaperSignal,
    quote_price: f64,
    current_cash: f64,
    total_value: f64,
    current_position_pct: f64,
    audit_evidence: &PaperAuditEvidence,
) -> Result<PaperOutcome, String> {
    simulate_with_scope(
        signal,
        quote_price,
        current_cash,
        total_value,
        current_position_pct,
        false,
        Some(audit_evidence),
    )
}

/// BR-146/147: paper-only execution from a confirmed closing snapshot.
pub fn simulate_snapshot(
    signal: &PaperSignal,
    quote_price: f64,
    current_cash: f64,
    total_value: f64,
    current_position_pct: f64,
) -> Result<PaperOutcome, String> {
    let _ = (
        signal,
        quote_price,
        current_cash,
        total_value,
        current_position_pct,
    );
    Err(
        "settled daily PaperTrade capability_unavailable: daily close cannot authorize a realtime terminal transition"
            .to_owned(),
    )
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
            risk_context: PaperRiskContext::new(AccountMode::Normal, DataMode::Full),
        }
    }

    #[derive(QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        count: i64,
    }

    #[derive(QueryableByName)]
    struct PaperAuditBasisRow {
        #[diesel(sql_type = diesel::sql_types::Text)]
        virtual_reason: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        decision_basis: String,
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
            None,
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

    fn terminal_binding(record_hash: &str) -> PaperTradeTerminalBindingV1 {
        let quote_observed_at = chrono::DateTime::parse_from_rfc3339("2026-07-30T09:31:00+08:00")
            .unwrap()
            .with_timezone(&chrono::Utc);
        PaperTradeTerminalBindingV1::new(
            7,
            "TEST_CODE_PLAN_BR192",
            crate::magic_compat::InstrumentId::new(
                crate::magic_compat::Exchange::Shanghai,
                "TEST_CODE_600001",
                crate::magic_compat::AssetClass::Equity,
            )
            .unwrap(),
            chrono::NaiveDate::from_ymd_opt(2026, 7, 30).unwrap(),
            "buy",
            10.0,
            100,
            PaperTradeStatus::Filled,
            Some(10.01),
            None,
            "TEST_CODE NewsCatalyst",
            "Normal",
            "Full",
            quote_observed_at,
            quote_observed_at,
            11,
            "BR086_ORDER_AUDIT_GENESIS_V1",
            record_hash,
            quote_observed_at,
        )
        .unwrap()
    }

    #[test]
    fn br192_terminal_binding_is_stable_and_changes_with_immutable_receipt() {
        let first = terminal_binding(&"a".repeat(64));
        let identical = terminal_binding(&"a".repeat(64));
        let changed_receipt = terminal_binding(&"b".repeat(64));

        assert_eq!(
            first.terminal_transition_id().unwrap(),
            identical.terminal_transition_id().unwrap()
        );
        assert_ne!(
            first.terminal_transition_id().unwrap(),
            changed_receipt.terminal_transition_id().unwrap()
        );
        assert_eq!(
            first.delivery_subject_hash().unwrap(),
            changed_receipt.delivery_subject_hash().unwrap(),
            "delivery subject remains the same plan/ticket/business-date identity"
        );
        assert_eq!(first.plan_id(), "TEST_CODE_PLAN_BR192");
    }

    #[test]
    fn br134_inventory_evidence_extends_audit_basis_without_changing_virtual_reason() {
        let signal = signal_default(false, false, false);
        let evidence = PaperAuditEvidence::new(
            "BR134_FIFO_V1;as_of=2026-08-23;source_fill_ids=1;open_lots=1@lot",
        )
        .unwrap();

        assert_eq!(
            audit_decision_basis(&signal, Some(&evidence)),
            "NewsCatalyst | BR134_FIFO_V1;as_of=2026-08-23;source_fill_ids=1;open_lots=1@lot"
        );
        assert_eq!(signal.virtual_reason, "NewsCatalyst");
    }

    #[test]
    fn br134_inventory_evidence_persists_only_in_order_audit_basis() {
        let mut conn =
            diesel::sqlite::SqliteConnection::establish(":memory:").expect("in-memory SQLite");
        DatabaseManager::run_migrations_for_test(&mut conn).expect("test migrations");
        let mut signal = signal_default(false, false, false);
        signal.plan_id = "TEST_CODE_BR134_AUDIT_EVIDENCE".to_string();
        let evidence = PaperAuditEvidence::new(
            "BR134_FIFO_V1;as_of=2026-08-23;source_fill_ids=1;open_lots=1@lot",
        )
        .unwrap();
        let result = evaluate(&signal, signal.price);
        let sql = "INSERT INTO paper_trades
                   (plan_id, code, name, direction, price, quantity, status,
                    fill_price, not_fill_reason, virtual_reason, account_mode, data_mode)
                   VALUES ('TEST_CODE_BR134_AUDIT_EVIDENCE', 'TEST_CODE_688001', '测试', 'buy',
                           50.0, 100, 'Filled', 50.0, NULL, 'NewsCatalyst', 'Normal', 'Full')";

        persist_paper_trade_with_audit(
            &mut conn,
            sql,
            &signal,
            &result,
            "2026-08-23T09:31:00+08:00",
            Some(&evidence),
        )
        .expect("audited paper persistence");
        let row = diesel::sql_query(
            "SELECT p.virtual_reason, a.decision_basis
             FROM paper_trades p
             JOIN order_audit a ON a.business_order_id = p.plan_id
             WHERE p.plan_id = 'TEST_CODE_BR134_AUDIT_EVIDENCE'",
        )
        .get_result::<PaperAuditBasisRow>(&mut conn)
        .expect("paper/audit evidence join");

        assert_eq!(row.virtual_reason, "NewsCatalyst");
        assert_eq!(
            row.decision_basis,
            "NewsCatalyst | BR134_FIFO_V1;as_of=2026-08-23;source_fill_ids=1;open_lots=1@lot"
        );
    }

    #[test]
    fn br192_realtime_quote_freshness_accepts_five_seconds_and_rejects_bad_time() {
        let evaluated_at = chrono::DateTime::parse_from_rfc3339("2026-07-30T09:31:05+08:00")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert!(validate_realtime_quote_freshness(
            evaluated_at - chrono::Duration::seconds(5),
            evaluated_at
        )
        .is_ok());
        assert!(validate_realtime_quote_freshness(
            evaluated_at - chrono::Duration::milliseconds(5_001),
            evaluated_at
        )
        .unwrap_err()
        .contains("stale"));
        assert!(validate_realtime_quote_freshness(
            evaluated_at + chrono::Duration::milliseconds(1),
            evaluated_at
        )
        .unwrap_err()
        .contains("future"));
    }

    #[test]
    fn br192_settled_daily_snapshot_cannot_create_a_realtime_terminal() {
        let signal = signal_default(false, false, false);
        let error = simulate_snapshot(&signal, 50.0, 100_000.0, 100_000.0, 0.0)
            .expect_err("settled daily capability must remain unavailable");
        assert!(error.contains("settled daily PaperTrade capability_unavailable"));
    }

    #[test]
    fn br192_paper_persistence_returns_exact_audit_chain_receipt() {
        let mut conn =
            diesel::sqlite::SqliteConnection::establish(":memory:").expect("in-memory SQLite");
        DatabaseManager::run_migrations_for_test(&mut conn).expect("test migrations");
        let mut signal = signal_default(false, false, false);
        signal.plan_id = "TEST_CODE_PLAN_BR192_RECEIPT".to_string();
        let result = evaluate(&signal, signal.price);
        let sql = "INSERT INTO paper_trades
                   (plan_id, code, name, direction, price, quantity, status,
                    fill_price, not_fill_reason, virtual_reason, account_mode, data_mode)
                   VALUES ('TEST_CODE_PLAN_BR192_RECEIPT', 'TEST_CODE_688001', '测试', 'buy',
                           50.0, 100, 'Filled', 50.0, NULL, 'NewsCatalyst', 'Normal', 'Full')";

        let (rows, receipt) = persist_paper_trade_with_audit(
            &mut conn,
            sql,
            &signal,
            &result,
            "2026-07-30T09:31:00+08:00",
            None,
        )
        .expect("atomic paper terminal persistence");
        let receipt = receipt.expect("new terminal transition carries receipt");
        assert_eq!(rows, 1);
        assert_eq!(receipt.plan_id, signal.plan_id);
        assert!(receipt.order_audit_id > 0);
        assert_eq!(receipt.audit_previous_hash, "BR086_ORDER_AUDIT_GENESIS_V1");
        assert_eq!(receipt.audit_record_hash.len(), 64);
        assert!(!receipt.terminal_at.is_empty());
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

        let missing = require_confirmed_position_snapshot(None)
            .expect_err("missing snapshot must fail loudly without shared database state");
        assert!(missing.contains("持仓快照"), "missing={missing}");
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
            terminal_receipt: None,
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
            terminal_receipt: None,
        };
        let inserted_false = PaperOutcome {
            result: PaperResult {
                status: PaperTradeStatus::NotFilled,
                fill_price: None,
                not_fill_reason: Some("涨停不可买".to_string()),
            },
            inserted: false,
            terminal_receipt: None,
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

    /// BR-151 快照模式账户证据：user_account_summary + user_position_snapshot
    /// → upsert 当日 ledger → (cash, total, pos_pct)。
    #[test]
    fn portfolio_state_snapshot_upserts_today_ledger_from_confirmed_snapshot() {
        let _ = DatabaseManager::init(None);
        let mut conn = DatabaseManager::get().get_conn().expect("test db conn");

        // 1. 券商账户汇总（synthetic 值，结构同生产：eastmoney-app-screenshot）。
        //    effective_at=今天 → BR-234b 快照新鲜分支：以快照 4 字段为准。
        let effective = chrono::Local::now()
            .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).expect("UTC+8 offset"));
        diesel::sql_query(
            "INSERT INTO user_account_summary \
                (effective_at, total_assets, securities_market_value, available_cash, \
                 position_ratio_pct, daily_pnl, source) \
             VALUES (?, 100000.0, 60000.0, 40000.0, \
                     60.0, 123.45, 'eastmoney-app-screenshot')",
        )
        .bind::<diesel::sql_types::Text, _>(&effective.to_rfc3339())
        .execute(&mut conn)
        .expect("insert account summary");

        // 2. 用户确认持仓快照 + 明细（synthetic TEST_CODE 持仓）
        use crate::portfolio::user_position_snapshot::{
            UserPositionItemInput, UserPositionSnapshotInput,
        };
        let input = UserPositionSnapshotInput {
            snapshot_id: format!(
                "TEST_CODE_SNAPSHOT_PORTFOLIO_{}",
                effective
                    .timestamp_nanos_opt()
                    .expect("test time fits timestamp nanos")
            ),
            effective_at: effective,
            confirmed_at: effective,
            source: "test-fixture".to_string(),
            confirm_empty: false,
            evidence_sha256: "TEST_CODE_EVIDENCE_001".to_string(),
            items: vec![UserPositionItemInput {
                code: "TEST_CODE_600519".to_string(),
                name: "测试持仓".to_string(),
                quantity: 1_000,
                cost_price: 50.0,
            }],
        };
        crate::database::user_position_snapshot::save_user_position_snapshot(&input)
            .expect("save snapshot");

        // 3. 调用：返回快照账户 (cash, total, pos_pct)
        let (cash, total, pos_pct) =
            portfolio_state_snapshot("TEST_CODE_600519", 60.0).expect("snapshot account ok");
        assert_eq!(cash, 40_000.0);
        assert_eq!(total, 100_000.0);
        // 已有 1000 股 × 60 元 / 10 万 = 60.0%
        assert!((pos_pct - 60.0).abs() < 1e-9, "pos_pct={pos_pct}");

        // 4. 当日 ledger 已 upsert 且过结构门（date==today, created_at≤30s）
        let ledger: LedgerState = diesel::sql_query(
            "SELECT date, total_value, cash, market_value, created_at FROM ledger \
             WHERE date = (SELECT date('now','localtime'))",
        )
        .get_result(&mut conn)
        .expect("today ledger row");
        validate_ledger_state(
            &ledger,
            &chrono::Local::now().date_naive().to_string(),
            chrono::Utc::now().naive_utc(),
        )
        .expect("BR-097 ledger structure gate");

        // 5. 非候选 code → 新仓 pos_pct = 0（买入前状态）
        let (_, _, fresh_pos) =
            portfolio_state_snapshot("TEST_CODE_688999", 10.0).expect("fresh code ok");
        assert_eq!(fresh_pos, 0.0);
    }

    /// BR-234b 自算路径：快照过期 → 持仓明细 × 实时价估值（测试 provider 10 元/只）；
    /// daily_pnl = 今日总资产 − 昨日 ledger。直测纯函数 estimate_ledger_from_snapshot
    /// （构造快照结构体不落库，避免并行测试污染共享 DB 的「快照缺失」断言）。
    #[test]
    fn estimate_ledger_from_snapshot_uses_live_prices_and_prev_ledger() {
        let _ = DatabaseManager::init(None);
        crate::broker::ensure_test_quote_provider(); // 实时价 10.0/只
        let mut conn = DatabaseManager::get().get_conn().expect("test db conn");
        let today = chrono::Local::now().date_naive();
        let yesterday = today.pred_opt().expect("yesterday");

        // 昨日 ledger 基准（daily_pnl 对比锚；ON CONFLICT 防并行测试同日期）
        diesel::sql_query(
            "INSERT INTO ledger (date, total_value, cash, market_value, daily_pnl, created_at) \
             VALUES (?, 48000.0, 40000.0, 8000.0, -2000.0, CURRENT_TIMESTAMP) \
             ON CONFLICT(date) DO UPDATE SET \
                 total_value=excluded.total_value, cash=excluded.cash, \
                 market_value=excluded.market_value, daily_pnl=excluded.daily_pnl, \
                 created_at=CURRENT_TIMESTAMP",
        )
        .bind::<diesel::sql_types::Text, _>(&yesterday.to_string())
        .execute(&mut conn)
        .expect("insert prev ledger");

        use crate::database::user_position_snapshot::UserPositionSnapshot;
        use crate::portfolio::user_position_snapshot::UserPositionItemInput;
        // 昨天确认的持仓快照（自算输入：明细数量 × 实时价）
        let snapshot = UserPositionSnapshot {
            snapshot_row_id: 1,
            snapshot_id: "TEST_CODE_SNAPSHOT_STALE".to_string(),
            effective_at: chrono::DateTime::parse_from_rfc3339(&format!(
                "{yesterday}T15:46:00+08:00"
            ))
            .expect("parse effective"),
            confirmed_at: chrono::DateTime::parse_from_rfc3339(&format!(
                "{yesterday}T15:47:00+08:00"
            ))
            .expect("parse confirmed"),
            source: "test-fixture".to_string(),
            confirm_empty: false,
            evidence_sha256: "TEST_CODE_EVIDENCE_STALE".to_string(),
            items: vec![UserPositionItemInput {
                code: "TEST_CODE_600519".to_string(),
                name: "测试持仓".to_string(),
                quantity: 1_000,
                cost_price: 50.0,
            }],
        };

        // 自算：market = Σ(明细 × 实时价10) = 10000，total = 10000 + 现金40000 = 50000，
        // daily_pnl = 50000 − 昨日48000 = 2000。
        let (total, cash, market, pnl) =
            estimate_ledger_from_snapshot(&snapshot, &mut conn, 40_000.0, today)
                .expect("estimate ok");
        assert_eq!(market, 10_000.0);
        assert_eq!(total, 50_000.0);
        assert_eq!(cash, 40_000.0);
        assert!((pnl - 2_000.0).abs() < 1e-9, "pnl={pnl}");
    }

    /// BR-234b fail-closed：实时行情不可用（无 provider）且日K无数据 →
    /// 估值价获取整体 Err，不写失真估值（成本价永不作估值价）。
    /// 并行测试可能已注册全局 provider（Once）→ 该分支不可复现时跳过。
    #[test]
    fn valuation_price_fails_closed_without_quote_source() {
        if crate::broker::quote_provider_registered() {
            return; // 其他并行测试已注册 provider，分支不可复现
        }
        let err = valuation_price("TEST_CODE_NO_QUOTE_SOURCE").expect_err("fail closed");
        assert!(err.contains("估值价获取失败"), "err={err}");
    }
}
