//! 模拟持仓跟踪 & 四大铁律平仓/开仓逻辑。
//!
//! 从 `AnalysisPipeline::process_stock_inner` 中抽离的独立子模块，
//! 通过 `track_position` 入口执行：查持仓 → 应用铁律 → 平仓或开仓 →
//! 回写结果字段。所有 DB 失败只记录 warn，不中断主流程。
//!
//! ## v2 风控统一 (P0-2)
//!
//! - 买入: PositionSizer::max_position() + MarketRegime 门控 (替代固定 position_shares)
//! - 卖出: StopLoss::triggered() + check_stops() 三级止损 (替代硬编码 8%)
//! - T+1: PositionType 锁仓检查
//! - 保留铁律2/3/4/5 作为额外保护层
//! - 配置 `[position_sizing] use_dynamic = false` 可回退到旧逻辑
//!
//! ## BR-015: 产业链集中度检查当前禁用 (2026-06-30 codex review)
//!
//! `track_position` 中 `risk_ctx.sizer.max_position(_, _, 0, 0, _)` 两个
//! 0 均为 placeholder (line 327-333), 当前监控无法拒绝同产业链第 N+1 只建仓.
//! 完整启用需要: (1) stock_position 表加 chain_name 列 (2) open_position
//! 时存 chain_name (3) DB 查同 chain 持仓 / T+1 冻结数 (4) chain_mapper
//! 关键词表算 chain. 待 v9.4+ 接 broker API 时统一处理.

use log::{info, warn};

use crate::data_provider::KlineData;
use crate::database::DatabaseManager;
use crate::monitor::risk::{MarketRegime, PositionSizer, StopLoss};
use crate::risk::stop_loss::check_stops;
use crate::strategy::BollMacdAction;
use crate::trading::{
    ClosePositionCmd, OpenPositionCmd, SimulatedExecutionGateway, TradeExecutionGateway,
};
use diesel::prelude::*;

use super::AnalysisResult;

/// 交易成本（与回测默认一致）：佣金万三、印花税千一(仅卖出)、滑点千一。
const COMMISSION_RATE: f64 = 0.0003;
const STAMP_TAX_RATE: f64 = 0.001;
const SLIPPAGE_RATE: f64 = 0.001;
/// 往返交易成本百分比（买:佣金+滑点；卖:佣金+印花税+滑点），用于把毛收益折算为净收益。
const ROUND_TRIP_COST_PCT: f64 =
    (COMMISSION_RATE + SLIPPAGE_RATE + COMMISSION_RATE + STAMP_TAX_RATE + SLIPPAGE_RATE) * 100.0;

// ============================================================================
// RiskContext — 注入 position_tracker 的风控参数
// ============================================================================

/// 风控上下文，封装所有注入 position_tracker 的风控组件。
pub struct RiskContext {
    pub sizer: PositionSizer,
    pub regime: MarketRegime,
    /// ATR 用于动态止损 (若为 None 则回退到固定 8%)
    pub atr: Option<f64>,
    /// 是否启用动态仓位 (false → 回退到旧 position_shares)
    pub use_dynamic: bool,
}

impl RiskContext {
    pub fn from_env(regime: MarketRegime, atr: Option<f64>) -> Self {
        let use_dynamic = crate::config::get_position_sizing_config().use_dynamic;
        Self {
            sizer: PositionSizer::from_env(),
            regime,
            atr,
            use_dynamic,
        }
    }
}

/// 最多同时持有的仓位数（决定单笔仓位预算），由 `MAX_POSITIONS` 配置，默认 5。
fn max_positions() -> usize {
    std::env::var("MAX_POSITIONS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(5)
}

/// 非动态模式也只允许使用真实账户现金，不得构造默认本金。
fn position_shares(price: f64, available_cash: f64) -> Result<i32, String> {
    if !price.is_finite() || price <= 0.0 || !available_cash.is_finite() || available_cash < 0.0 {
        return Err(format!(
            "BR-085 invalid sizing evidence: price={price} available_cash={available_cash}"
        ));
    }
    let budget = available_cash / max_positions() as f64;
    let lots = (budget / price / 100.0).floor() as i32;
    if lots < 1 {
        return Err(format!(
            "BR-085 available cash cannot fund one lot: cash={available_cash} price={price}"
        ));
    }
    Ok(lots * 100)
}

/// 毛收益率 → 净收益率（扣往返交易成本）。
fn net_return_rate(gross_pct: f64) -> f64 {
    gross_pct - ROUND_TRIP_COST_PCT
}

fn validate_trade_symbol_env(code: &str) -> Result<(), String> {
    crate::risk::env_guard::validate_symbol_for_current_env(code)
}

/// BR-085: resolve a current chain classification and its real open/frozen exposure.
fn query_chain_exposure(code: &str) -> Result<(String, usize, usize), String> {
    let mut conn = DatabaseManager::get()
        .get_conn()
        .map_err(|e| format!("DB: {}", e))?;

    #[derive(diesel::QueryableByName)]
    struct ConceptCacheRow {
        #[diesel(sql_type = diesel::sql_types::Text)]
        concepts: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        updated_at: String,
    }
    let cache: Option<ConceptCacheRow> =
        diesel::sql_query("SELECT concepts, updated_at FROM stock_concepts WHERE code = ? LIMIT 1")
            .bind::<diesel::sql_types::Text, _>(code)
            .get_result(&mut conn)
            .optional()
            .map_err(|e| format!("query stock_concepts: {e}"))?;

    let effective_today = if crate::calendar::is_trading_day(chrono::Local::now().date_naive()) {
        chrono::Local::now().date_naive()
    } else {
        crate::calendar::prev_trading_day(chrono::Local::now().date_naive())
    };
    let allowed_dates = crate::calendar::recent_trading_days(effective_today, 2);
    let cached_chain = cache.and_then(|row| {
        let updated_date =
            chrono::NaiveDateTime::parse_from_str(&row.updated_at, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|value| value.date());
        if updated_date.is_some_and(|date| allowed_dates.contains(&date)) {
            serde_json::from_str::<Vec<String>>(&row.concepts)
                .ok()
                .and_then(|values| values.into_iter().find(|value| !value.trim().is_empty()))
        } else {
            None
        }
    });
    let chain = cached_chain
        .or_else(|| crate::data_provider::chain_registry::lookup(code).map(str::to_string))
        .filter(|value| value != "其他")
        .ok_or_else(|| format!("BR-085 chain classification unavailable for {code}"))?;

    #[derive(diesel::QueryableByName)]
    struct ExposureRow {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        held: i64,
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        frozen: i64,
    }
    let today = chrono::Local::now().date_naive().to_string();
    let exposure: ExposureRow = diesel::sql_query(
        "SELECT COUNT(*) AS held,
                COALESCE(SUM(CASE WHEN buy_date = ? THEN 1 ELSE 0 END), 0) AS frozen
         FROM stock_position WHERE chain_name = ? AND status = 'open'",
    )
    .bind::<diesel::sql_types::Text, _>(&today)
    .bind::<diesel::sql_types::Text, _>(&chain)
    .get_result(&mut conn)
    .map_err(|e| format!("query chain exposure: {e}"))?;
    Ok((chain, exposure.held as usize, exposure.frozen as usize))
}

/// 对单只股票跟踪模拟持仓并在满足条件时开/平仓。
pub(super) fn track_position(
    code: &str,
    data: &[KlineData],
    result: &mut AnalysisResult,
    risk_ctx: &RiskContext,
) {
    let gateway = SimulatedExecutionGateway::new();

    // AGENTS 2.5: 双向硬隔离（生产拒绝 TEST_CODE，测试拒绝真实标的）
    if let Err(reason) = validate_trade_symbol_env(code) {
        warn!(
            "[ENV_GUARD] rule_id=AGENTS-2.5 code={} env={:?} action=reject reason={} timestamp={}",
            code,
            crate::risk::env_guard::current_env(),
            reason,
            chrono::Utc::now().timestamp()
        );
        return;
    }

    let current_price = match result
        .current_price
        .or_else(|| data.first().map(|bar| bar.close))
        .filter(|price| price.is_finite() && *price > 0.0)
    {
        Some(price) => price,
        None => {
            warn!("[{}] BR-084 当前价格缺失或非法，拒绝持仓操作", code);
            return;
        }
    };

    match gateway.get_open_position(code) {
        Ok(Some(pos)) => {
            let return_rate = net_return_rate((current_price / pos.buy_price - 1.0) * 100.0);
            result.position_buy_price = Some(pos.buy_price);
            result.position_buy_date = Some(pos.buy_date.clone());
            result.position_return = Some(return_rate);
            result.position_quantity = Some(pos.quantity);

            // ====== 卖出判断: ATR 动态止损 + 三级止损 + 铁律2/3/4/5 ======

            // P0-2: ATR 动态止损替代硬编码 8%
            let stop_loss = if let Some(atr) = risk_ctx.atr {
                if atr > 0.0 {
                    let sl = StopLoss::new(pos.buy_price, atr, result.ma20);
                    sl.triggered(current_price)
                } else {
                    // ATR 异常 → 回退到固定 8%
                    warn!("[{}] ATR 异常({})，回退到固定 8% 止损", code, atr);
                    return_rate <= -8.0
                }
            } else {
                // ATR 数据缺失 → 回退到固定 8%
                return_rate <= -8.0
            };

            // P0-2: 三级止损补充检查
            let tiered_stops = check_stops(
                code,
                &result.name,
                current_price,
                pos.buy_price,
                Some(pos.buy_price * 0.92), // strategy-derived hard stop
                result.ma20,
                result.ma60,
            );

            // 铁律2 盈利<20% 绝不主动止盈（不触发卖出）
            // 铁律3 盈利 ≥ 20% 后，跌破 5 日均线
            let profit_trend_exit =
                return_rate >= 20.0 && result.ma5.is_some_and(|ma5| current_price < ma5);
            // 铁律4 持仓 >14 天仍亏损
            let hold_days = {
                let buy = chrono::NaiveDate::parse_from_str(&pos.buy_date, "%Y-%m-%d");
                let now = chrono::Local::now().date_naive();
                buy.map(|b| (now - b).num_days()).unwrap_or(0)
            };
            let timeout_loss = hold_days > 14 && return_rate < 0.0;
            // 铁律5 布林上轨减仓：触上轨 + MACD 顶背离/红柱缩短/死叉
            let bm_top_sell = matches!(
                result.boll_macd.as_ref().map(|s| s.action),
                Some(BollMacdAction::TopSell)
            ) && return_rate >= 5.0
                && hold_days >= 2;

            // P0-2: T+1 锁仓检查
            let t1_locked = {
                let buy = chrono::NaiveDate::parse_from_str(&pos.buy_date, "%Y-%m-%d");
                let now = chrono::Local::now().date_naive();
                buy.is_ok() && buy.unwrap() == now
            };

            let should_sell = stop_loss
                || !tiered_stops.is_empty()
                || profit_trend_exit
                || timeout_loss
                || bm_top_sell;

            if should_sell {
                // T+1 锁仓: A股当日买入不可卖出, 阻止所有平仓操作
                if t1_locked {
                    let reason_str = if stop_loss || !tiered_stops.is_empty() {
                        "止损信号"
                    } else if profit_trend_exit {
                        "铁律3:跌破5日线止盈"
                    } else if bm_top_sell {
                        "铁律5:布林上轨减仓"
                    } else {
                        "铁律4:14天换股"
                    };
                    warn!(
                        "[{}] T+1锁仓无法卖出(原因: {}) — 建议次日竞价挂单",
                        code, reason_str
                    );
                    result.position_status = Some("open".to_string());
                    return;
                }

                let reason = if stop_loss {
                    if risk_ctx.atr.unwrap_or(0.0) > 0.0 {
                        let sl =
                            StopLoss::new(pos.buy_price, risk_ctx.atr.unwrap_or(3.0), result.ma20);
                        format!("ATR动态止损(有效止损价 {:.2})", sl.effective())
                    } else {
                        "铁律1:止损(-8%)".to_string()
                    }
                } else if !tiered_stops.is_empty() {
                    tiered_stops
                        .iter()
                        .map(|s| s.level.label().to_string())
                        .collect::<Vec<_>>()
                        .join("+")
                } else if profit_trend_exit {
                    "铁律3:跌破5日线止盈".to_string()
                } else if bm_top_sell {
                    "铁律5:布林上轨+MACD顶背离/红柱衰竭".to_string()
                } else {
                    "铁律4:14天不涨换股".to_string()
                };
                let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                let close_cmd = ClosePositionCmd {
                    business_order_id: format!(
                        "SIM-SELL-{}-{}-{}",
                        code,
                        pos.id,
                        chrono::Utc::now().timestamp()
                    ),
                    position_id: pos.id,
                    code: code.to_string(),
                    trade_date: today.clone(),
                    price: current_price,
                    quantity: pos.quantity,
                    secondary_confirmed: false,
                    decision_basis: reason.clone(),
                };
                match gateway.close_position(&close_cmd) {
                    Ok(receipt) => {
                        info!(
                            "[{}] 触发平仓 [{}]，@ {:.2}，收益率: {:+.2}%",
                            code, reason, receipt.price, return_rate
                        );
                        result.position_status = Some("closed".to_string());
                        result.position_sell_price = Some(receipt.price);
                        result.position_sell_date = Some(today);
                    }
                    Err(e) => {
                        warn!("[{}] 平仓失败: {}", code, e);
                        result.position_status = Some("open".to_string());
                    }
                }
            } else {
                result.position_status = Some("open".to_string());
                info!(
                    "[{}] 持仓中，收益率: {:+.2}%（买入价 {:.2} → 现价 {:.2}）",
                    code, return_rate, pos.buy_price, current_price
                );
                if let Err(e) = gateway.update_position_return(pos.id, current_price, return_rate) {
                    warn!("[{}] 更新持仓收益率失败: {}", code, e);
                }
            }
        }
        Ok(None) => {
            // ====== 买入触发逻辑（B 方案：布林+MACD 共振 + 反向信号）======
            //
            // 历史数据复盘（136 笔已平仓）显示 AI 评分严重反向：
            //   - 评分 ≥ 80：胜率 0%；评分 70-79：胜率 13.9%；评分 60-69：胜率 19.2%
            //   - 评分 < 40：胜率 50%（小样本），平均 +14.68%
            // 因此放弃用 AI 评分作为买入触发，改用技术面共振：
            //   1) 布林+MACD `BottomBuy` （触下轨 + 底背离 / 0 轴下绿柱缩短）
            //   2) 布林+MACD `UptrendStart`（布林张口 + 0 轴上方金叉，主升浪）
            //   3) Contrarian 反向信号（评分 < 40 + 超跌企稳）
            // 误区拦截已下沉到 detect_boll_macd_signal —— 单纯触轨不会买。

            // P0-2: MarketRegime 门控 — 崩盘禁止新买入
            if !risk_ctx.regime.allow_new_position() {
                info!(
                    "[{}] MarketRegime::{:?} 禁止新买入，跳过",
                    code, risk_ctx.regime
                );
                return;
            }

            let bm_action = result
                .boll_macd
                .as_ref()
                .map(|s| s.action)
                .unwrap_or(BollMacdAction::None);
            let bm_reason = result
                .boll_macd
                .as_ref()
                .map(|s| s.reason.clone())
                .unwrap_or_default();

            let bm_buy = bm_action.is_buy();
            let buy_triggered = bm_buy || result.contrarian_signal;

            if !buy_triggered {
                if result.operation_advice.contains("买入") {
                    info!(
                        "[{}] 跳过买入：AI 建议买入但布林+MACD 无共振信号（动作={}，评分={}）",
                        code,
                        bm_action.name(),
                        result.sentiment_score
                    );
                }
                return;
            }

            let (chain_name, chain_held, chain_frozen) = match query_chain_exposure(code) {
                Ok(exposure) => exposure,
                Err(error) => {
                    warn!(
                        "[{}] BR-085 产业链风险证据不可用，拒绝建仓: {}",
                        code, error
                    );
                    return;
                }
            };
            let (available_cash, _, _) =
                match crate::trading::paper_trade::portfolio_state(code, current_price) {
                    Ok(state) => state,
                    Err(error) => {
                        warn!("[{}] BR-085 真实账户快照不可用，拒绝建仓: {}", code, error);
                        return;
                    }
                };

            // P0-2: 动态仓位计算 (PositionSizer 替代固定 position_shares)
            let shares = if risk_ctx.use_dynamic {
                let volatility = match result.volatility {
                    Some(value) if value.is_finite() && value > 0.0 => value,
                    _ => {
                        warn!("[{}] BR-085 波动率缺失或非法，拒绝动态建仓", code);
                        return;
                    }
                };
                let max_amount = risk_ctx.sizer.max_position(
                    risk_ctx.regime,
                    volatility,
                    chain_held,
                    chain_frozen,
                    false, // already_held 在 DB 层已检查
                );
                if max_amount <= 0.0 {
                    warn!("[{}] PositionSizer 返回 0 仓位，跳过买入", code);
                    return;
                }
                let lots = (max_amount / current_price / 100.0).floor() as i32;
                if lots < 1 {
                    warn!("[{}] BR-085 动态仓位不足一手，拒绝建仓", code);
                    return;
                }
                lots * 100
            } else {
                match position_shares(current_price, available_cash) {
                    Ok(shares) => shares,
                    Err(error) => {
                        warn!("[{}] 非动态仓位计算失败，拒绝建仓: {}", code, error);
                        return;
                    }
                }
            };

            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            let open_cmd = OpenPositionCmd {
                business_order_id: format!("SIM-BUY-{}-{}", code, chrono::Utc::now().timestamp()),
                code: code.to_string(),
                name: result.name.clone(),
                trade_date: today.clone(),
                price: current_price,
                quantity: shares,
                secondary_confirmed: false,
                chain_name,
                decision_basis: if !bm_reason.is_empty() {
                    bm_reason.clone()
                } else {
                    result
                        .contrarian_reason
                        .clone()
                        .unwrap_or_else(|| "BollMacd/Contrarian trigger".to_string())
                },
            };
            match gateway.open_position(&open_cmd) {
                Ok(receipt) => {
                    let tag = match (bm_action, result.contrarian_signal) {
                        (BollMacdAction::UptrendStart, _) => "主升浪启动",
                        (BollMacdAction::BottomBuy, _) => "下轨抄底",
                        (_, true) => "反向信号",
                        _ => "其他",
                    };
                    let sizing_label = if risk_ctx.use_dynamic {
                        format!(
                            "动态仓位({}股 ≈ {:.0}元)",
                            shares,
                            shares as f64 * current_price
                        )
                    } else {
                        format!("{}股", shares)
                    };
                    let extra = if !bm_reason.is_empty() {
                        format!(" | {}", bm_reason)
                    } else if let Some(r) = result.contrarian_reason.as_ref() {
                        format!(" | {}", r)
                    } else {
                        String::new()
                    };
                    info!(
                        "[{}] 触发{}（AI 评分 {}），模拟买入 {} @ {:.2}{}",
                        code, tag, result.sentiment_score, sizing_label, receipt.price, extra
                    );
                    result.position_buy_price = Some(receipt.price);
                    result.position_buy_date = Some(today);
                    result.position_return = Some(0.0);
                    result.position_quantity = Some(shares);
                    result.position_status = Some("new".to_string());
                }
                Err(e) => warn!("[{}] 记录模拟买入失败: {}", code, e),
            }
        }
        Err(e) => warn!("[{}] 查询持仓失败: {}", code, e),
    }
}

/// 保存当日分析结果到数据库。
pub(super) fn save_analysis_result(code: &str, data: &[KlineData], result: &AnalysisResult) {
    let Some(db) = DatabaseManager::try_get() else {
        return;
    };
    let latest_kline = &data[0];
    let score_breakdown_json = result
        .score_breakdown
        .as_ref()
        .and_then(|sb| serde_json::to_string(sb).ok());
    let veto_flags_json = result
        .veto_flags
        .as_ref()
        .and_then(|flags| serde_json::to_string(flags).ok());
    let new_result = crate::models::NewAnalysisResult {
        code: result.code.clone(),
        name: result.name.clone(),
        date: chrono::Local::now().date_naive(),
        sentiment_score: result.sentiment_score,
        operation_advice: result.operation_advice.clone(),
        trend_prediction: result.trend_prediction.clone(),
        pe_ratio: result.pe_ratio,
        pb_ratio: result.pb_ratio,
        turnover_rate: result.turnover_rate,
        market_cap: result.market_cap,
        circulating_cap: result.circulating_cap,
        close_price: Some(latest_kline.close),
        pct_chg: Some(latest_kline.pct_chg),
        data_source: None,
        score_breakdown_json,
        original_advice: result.original_advice.clone(),
        veto_flags_json,
    };
    match db.save_analysis_result(&new_result) {
        Ok(_) => info!("[{}] 分析结果已保存到数据库", code),
        Err(e) => warn!("[{}] 保存分析结果失败: {}", code, e),
    }
}

#[cfg(test)]
mod tests {
    use super::{position_shares, validate_trade_symbol_env};
    use once_cell::sync::Lazy;
    use std::sync::Mutex;

    static ENV_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    struct EnvModeGuard(Option<String>);

    impl Drop for EnvModeGuard {
        fn drop(&mut self) {
            if let Some(value) = self.0.take() {
                std::env::set_var("STOCK_ENV_MODE", value);
            } else {
                std::env::remove_var("STOCK_ENV_MODE");
            }
        }
    }

    fn with_env_mode<T>(mode: &str, f: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let prev = std::env::var("STOCK_ENV_MODE").ok();
        std::env::set_var("STOCK_ENV_MODE", mode);
        let _mode_guard = EnvModeGuard(prev);
        f()
    }

    #[test]
    fn test_prod_rejects_test_code() {
        with_env_mode("prod", || {
            let r = validate_trade_symbol_env("TEST_CODE_000001");
            assert!(r.is_err());
        });
    }

    #[test]
    fn test_test_rejects_real_code() {
        with_env_mode("test", || {
            // Environment-isolation exception: a native production symbol is
            // the invalid input this negative test must reject.
            let r = validate_trade_symbol_env("600519");
            assert!(r.is_err());
        });
    }

    #[test]
    fn test_legal_symbol_matrix_passes() {
        with_env_mode("prod", || {
            // Environment-isolation exception: production must accept only the
            // native six-digit representation.
            assert!(validate_trade_symbol_env("600519").is_ok());
        });
        with_env_mode("test", || {
            assert!(validate_trade_symbol_env("TEST_CODE_600519").is_ok());
        });
    }

    #[test]
    fn non_dynamic_sizing_uses_real_cash_without_forcing_a_lot() {
        assert_eq!(position_shares(10.0, 100_000.0), Ok(2_000));
        assert!(position_shares(100.0, 100.0).is_err());
        assert!(position_shares(0.0, 100_000.0).is_err());
    }
}
