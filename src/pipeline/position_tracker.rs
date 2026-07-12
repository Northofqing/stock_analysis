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

/// 模拟账户总本金（元），由 `TOTAL_CAPITAL` 配置，默认 10 万。
/// 保留用于 fallback 路径。
fn total_capital() -> f64 {
    std::env::var("TOTAL_CAPITAL")
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| *v > 0.0)
        .unwrap_or(100_000.0)
}

/// 最多同时持有的仓位数（决定单笔仓位预算），由 `MAX_POSITIONS` 配置，默认 5。
/// 保留用于 fallback 路径。
fn max_positions() -> usize {
    std::env::var("MAX_POSITIONS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(5)
}

/// [fallback] 按本金计算买入股数：单笔预算 = 总本金 / 最大仓位数。
/// use_dynamic=false 时使用此函数。
fn position_shares(price: f64) -> i32 {
    if price <= 0.0 {
        return 100;
    }
    let budget = total_capital() / max_positions() as f64;
    let lots = (budget / price / 100.0).floor() as i32;
    lots.max(1) * 100
}

/// 毛收益率 → 净收益率（扣往返交易成本）。
fn net_return_rate(gross_pct: f64) -> f64 {
    gross_pct - ROUND_TRIP_COST_PCT
}

fn validate_trade_symbol_env(code: &str) -> Result<(), String> {
    crate::risk::env_guard::validate_symbol_for_current_env(code)
}

/// v12 PR3-3.6 (BR-015 偿还): 查同 chain 已持仓数 (open status).
///
/// 实现: 先取本标的**最近建仓**的 chain_name (ORDER BY buy_date DESC, id DESC),
///       再数同 chain 的 open 持仓数.
/// DB 错误时返回 0 (不阻断, 走 fallback 旧 hardcoded 行为).
///
/// Bug #3 fix (2026-07-05): 无 ORDER BY 的 LIMIT 1 在 SQLite 下非稳定,
/// 同 code 多 chain 时不同时间查询可能返回不同 row. 加 ORDER BY 后取最新建仓.
fn query_chain_held_count(code: &str) -> Result<i32, String> {
    let mut conn = DatabaseManager::get()
        .get_conn()
        .map_err(|e| format!("DB: {}", e))?;

    // raw SQL 全部 (避免 diesel count/select trait bound 不稳定)
    let esc = |s: &str| s.replace('\'', "''");
    // 取最新建仓的 chain_name (排除 '其他' 占位 + NULL)
    // ORDER BY buy_date DESC, id DESC 保证稳定性 (id 是 rowid 唯一)
    let sql_chain = format!(
        "SELECT chain_name FROM stock_position \
         WHERE code = '{}' AND status = 'open' \
           AND chain_name IS NOT NULL AND chain_name != '' AND chain_name != '其他' \
         ORDER BY buy_date DESC, id DESC LIMIT 1",
        esc(code)
    );
    let chain_row: Option<ChainNameRow> = diesel::sql_query(sql_chain)
        .get_result(&mut conn)
        .optional()
        .map_err(|e| format!("query chain_name: {}", e))?;

    let chain = match chain_row {
        Some(r) if !r.chain_name.is_empty() && r.chain_name != "其他" => r.chain_name,
        _ => return Ok(0),
    };

    let count_sql = format!(
        "SELECT COUNT(*) AS cnt FROM stock_position WHERE chain_name = '{}' AND status = 'open'",
        esc(&chain)
    );
    let count: i64 = diesel::sql_query(count_sql)
        .get_result::<CountRow>(&mut conn)
        .map(|r| r.cnt)
        .map_err(|e| format!("count chain: {}", e))?;

    Ok(count as i32)
}

#[derive(diesel::QueryableByName)]
struct CountRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    cnt: i64,
}

#[derive(diesel::QueryableByName)]
struct ChainNameRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    chain_name: String,
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

    let current_price = result.current_price.unwrap_or(data[0].close);

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
                pos.buy_price * 0.92, // hard stop = 买入价 × 0.92
                result.ma20,
                result.ma60,
            );

            // 铁律2 盈利<20% 绝不主动止盈（不触发卖出）
            // 铁律3 盈利 ≥ 20% 后，跌破 5 日均线
            let profit_trend_exit =
                return_rate >= 20.0 && result.ma5.map_or(false, |ma5| current_price < ma5);
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
                };
                match gateway.close_position(&close_cmd) {
                    Ok(_) => {
                        info!(
                            "[{}] 触发平仓 [{}]，@ {:.2}，收益率: {:+.2}%",
                            code, reason, current_price, return_rate
                        );
                        result.position_status = Some("closed".to_string());
                        result.position_sell_price = Some(current_price);
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

            // P0-2: 动态仓位计算 (PositionSizer 替代固定 position_shares)
            let shares = if risk_ctx.use_dynamic {
                let volatility = result.volatility.unwrap_or(3.0);
                // 修复 B6 + BR-015 + v12 PR3-3.6 (2026-07-05):
                // stock_position 表已加 chain_name 列 (migration v12-p0-paper-and-adjust)
                // 现在可以查同 chain 持仓数, 接真值替换之前的 hardcoded 0.
                if !risk_ctx.use_dynamic {
                    warn!("[{}] chain 集中度检查暂未启用 (BR-015), max_position 用 base × vol × regime", code);
                }
                // PR3-3.6: 接入真值 (DB 查同 chain 持仓数). 失败时 fallback 0 (不阻断).
                let chain_held = query_chain_held_count(code).unwrap_or(0) as usize;
                let chain_frozen: usize = 0; // T+1 冻结数: 后续 PR 接入 buy_date 索引 + 同 chain 查
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
                lots.max(1) * 100
            } else {
                position_shares(current_price)
            };

            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            let open_cmd = OpenPositionCmd {
                business_order_id: format!("SIM-BUY-{}-{}", code, chrono::Utc::now().timestamp()),
                code: code.to_string(),
                name: result.name.clone(),
                trade_date: today.clone(),
                price: current_price,
                quantity: shares,
            };
            match gateway.open_position(&open_cmd) {
                Ok(_) => {
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
                        code, tag, result.sentiment_score, sizing_label, current_price, extra
                    );
                    result.position_buy_price = Some(current_price);
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

#[cfg(test)]
mod tests {
    use super::validate_trade_symbol_env;
    use once_cell::sync::Lazy;
    use std::sync::Mutex;

    static ENV_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    fn with_env_mode<T>(mode: &str, f: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        let prev = std::env::var("STOCK_ENV_MODE").ok();
        std::env::set_var("STOCK_ENV_MODE", mode);
        let out = f();
        if let Some(v) = prev {
            std::env::set_var("STOCK_ENV_MODE", v);
        } else {
            std::env::remove_var("STOCK_ENV_MODE");
        }
        out
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
            let r = validate_trade_symbol_env("600519");
            assert!(r.is_err());
        });
    }

    #[test]
    fn test_legal_symbol_matrix_passes() {
        with_env_mode("prod", || {
            assert!(validate_trade_symbol_env("600519").is_ok());
        });
        with_env_mode("test", || {
            assert!(validate_trade_symbol_env("TEST_CODE_600519").is_ok());
        });
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
