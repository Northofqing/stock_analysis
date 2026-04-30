//! 模拟持仓跟踪 & 四大铁律平仓/开仓逻辑。
//!
//! 从 `AnalysisPipeline::process_stock_inner` 中抽离的独立子模块，
//! 通过 `track_position` 入口执行：查持仓 → 应用铁律 → 平仓或开仓 →
//! 回写结果字段。所有 DB 失败只记录 warn，不中断主流程。

use log::{info, warn};

use crate::database::DatabaseManager;
use crate::data_provider::KlineData;
use crate::strategy::BollMacdAction;

use super::AnalysisResult;

/// 对单只股票跟踪模拟持仓并在满足条件时开/平仓。
pub(super) fn track_position(code: &str, data: &[KlineData], result: &mut AnalysisResult) {
    let Ok(db) = std::panic::catch_unwind(|| DatabaseManager::get()) else {
        return;
    };

    let current_price = result.current_price.unwrap_or(data[0].close);

    match db.get_open_position(code) {
        Ok(Some(pos)) => {
            let return_rate = (current_price / pos.buy_price - 1.0) * 100.0;
            result.position_buy_price = Some(pos.buy_price);
            result.position_buy_date = Some(pos.buy_date.clone());
            result.position_return = Some(return_rate);
            result.position_quantity = Some(pos.quantity);

            // ====== 四大铁律 + 布林上轨减仓 ======
            // 铁律1 止损：亏损 ≥ 8%
            let stop_loss = return_rate <= -8.0;
            // 铁律2 盈利<20% 绝不主动止盈（不触发卖出）
            // 铁律3 盈利 ≥ 20% 后，跌破 5 日均线
            let profit_trend_exit = return_rate >= 20.0
                && result.ma5.map_or(false, |ma5| current_price < ma5);
            // 铁律4 持仓 >14 天仍亏损
            let hold_days = {
                let buy = chrono::NaiveDate::parse_from_str(&pos.buy_date, "%Y-%m-%d");
                let now = chrono::Local::now().date_naive();
                buy.map(|b| (now - b).num_days()).unwrap_or(0)
            };
            let timeout_loss = hold_days > 14 && return_rate < 0.0;
            // 铁律5（新）布林上轨减仓：触上轨 + MACD 顶背离/红柱缩短/死叉
            //   仅在已有浮盈（≥ 5%）且非首日时触发，避免短期假信号洗票
            let bm_top_sell = matches!(
                result.boll_macd.as_ref().map(|s| s.action),
                Some(BollMacdAction::TopSell)
            ) && return_rate >= 5.0
                && hold_days >= 2;

            let should_sell = stop_loss || profit_trend_exit || timeout_loss || bm_top_sell;
            if should_sell {
                let reason = if stop_loss {
                    "铁律1:止损(-8%)"
                } else if profit_trend_exit {
                    "铁律3:跌破5日线止盈"
                } else if bm_top_sell {
                    "铁律5:布林上轨+MACD顶背离/红柱衰竭"
                } else {
                    "铁律4:14天不涨换股"
                };
                let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                match db.close_position(pos.id, current_price, &today) {
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
                if let Err(e) = db.update_position_return(pos.id, current_price, return_rate) {
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
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            let new_position = crate::models::NewStockPosition {
                code: code.to_string(),
                name: result.name.clone(),
                buy_date: today.clone(),
                buy_price: current_price,
                quantity: 1000,
                status: "open".to_string(),
            };
            match db.save_position(&new_position) {
                Ok(_) => {
                    let tag = match (bm_action, result.contrarian_signal) {
                        (BollMacdAction::UptrendStart, _) => "主升浪启动",
                        (BollMacdAction::BottomBuy, _) => "下轨抄底",
                        (_, true) => "反向信号",
                        _ => "其他",
                    };
                    let extra = if !bm_reason.is_empty() {
                        format!(" | {}", bm_reason)
                    } else if let Some(r) = result.contrarian_reason.as_ref() {
                        format!(" | {}", r)
                    } else {
                        String::new()
                    };
                    info!(
                        "[{}] 触发{}（AI 评分 {}），模拟买入 1000 股 @ {:.2}{}",
                        code, tag, result.sentiment_score, current_price, extra
                    );
                    result.position_buy_price = Some(current_price);
                    result.position_buy_date = Some(today);
                    result.position_return = Some(0.0);
                    result.position_quantity = Some(1000);
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
    let Ok(db) = std::panic::catch_unwind(|| DatabaseManager::get()) else {
        return;
    };
    let latest_kline = &data[0];
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
    };
    match db.save_analysis_result(&new_result) {
        Ok(_) => info!("[{}] 分析结果已保存到数据库", code),
        Err(e) => warn!("[{}] 保存分析结果失败: {}", code, e),
    }
}
