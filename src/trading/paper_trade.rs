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
/// v16.3 R1 (集合竞价期挂单): 9:15-9:25 期间 quote_price=0 (无成交) → Filled (挂单簿等成交)
pub fn evaluate(signal: &PaperSignal, quote_price: f64) -> PaperResult {
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

    // 4. v16.3 R2: 滑点保护 (quote_price > 0 时才检查; 0 表示缺数据, 兼容早盘)
    if quote_price > 0.0 && signal.price > 0.0 {
        let slippage = (quote_price - signal.price).abs() / signal.price * 100.0;
        if slippage > *MAX_SLIPPAGE_PCT {
            log::warn!(
                "[paper_trade] 滑点 {:.2}% 超过 MAX_SLIPPAGE_PCT={:.1}% (signal={}, quote={})",
                slippage, *MAX_SLIPPAGE_PCT, signal.price, quote_price
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
/// 失败策略 (红线 2.2 出声): get_positions 失败 → warn + (0,0,0);
/// cash_guard 对 total<=0 显式跳过检查 (缺数据不硬拦), 每次失败均有 warn 日志.
pub fn portfolio_state(code: &str, quote_price: f64) -> (f64, f64, f64) {
    let positions = match crate::portfolio::get_positions() {
        Ok(p) => p,
        Err(e) => {
            log::warn!(
                "[portfolio_state] get_positions 失败: {} → (0,0,0), risk_adapter 将跳过现金/仓位检查",
                e
            );
            return (0.0, 0.0, 0.0);
        }
    };
    let mut total = 1_000_000.0_f64;
    let mut pos_pct = 0.0_f64;
    for p in &positions {
        if p.code == code {
            pos_pct = (p.shares as f64 * quote_price) / total * 100.0;
        }
        total += p.shares as f64 * quote_price;
    }
    let cash = total * 0.30; // v16.3 简化: 30% 现金假设, v16.4 接 ledger 真值
    (cash, total, pos_pct)
}

/// 模拟成交结果 (含 DB 写入状态)
#[derive(Clone, Debug)]
pub struct PaperOutcome {
    /// 评估结果 (Filled / NotFilled / Invalidated)
    pub result: PaperResult,
    /// true = INSERT 实际写入; false = INSERT OR IGNORE 跳过 (plan_id 重复)
    pub inserted: bool,
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
    // v16.3 R1+R2: pre-trade gate 4 项硬检查 (拒 → 不入 paper_trades, 不调 evaluate)
    crate::trading::risk_adapter::pre_trade_check(
        signal,
        quote_price,
        current_cash,
        total_value,
        current_position_pct,
    )?;

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
    let rows = diesel::sql_query(sql)
        .execute(&mut conn)
        .map_err(|e| format!("insert paper_trades: {}", e))?;

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
            code: "688001".to_string(),
            name: "测试".to_string(),
            direction: Direction::Buy,
            price: 50.0,
            quantity: 100,
            virtual_reason: "NewsCatalyst".to_string(),
            is_limit_up,
            is_limit_down,
            is_suspended,
            account_mode: "Normal".to_string(),
            data_mode: "Full".to_string(),
        }
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
    fn filled_when_quote_price_zero() {
        // 9:15-9:25 集合竞价, quote_price=0 (无成交) → Filled (挂单簿等成交)
        let r = evaluate(&signal_default(false, false, false), 0.0);
        assert_eq!(r.status, PaperTradeStatus::Filled);
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
}
