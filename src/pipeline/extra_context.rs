//! 真实口径辅助数据：主力资金流 / 日内分时 / 龙虎榜席位 / 筹码分布。
//!
//! 产出单份 Markdown 片段，既会被塞进 AI prompt，也会被挂到 `AnalysisResult.money_flow_section`
//! 给通知展示。

use crate::data_provider::money_flow::MoneyFlowSummary;
use crate::data_provider::KlineData;

/// `fetch_extra_context` 的产物：
/// - `section`：用于通知 / AI prompt 的 Markdown 片段（与之前等价）。
/// - `money_flow`：原始资金流时序，用于打分器做 EWMA / 单日反弹判定。
pub(super) struct ExtraContext {
    pub section: Option<String>,
    pub money_flow: Option<MoneyFlowSummary>,
}

/// 抓取资金流/分时/LHB 数据，并合并由 K 线计算的筹码分布，返回格式化后的 Markdown。
///
/// 抓取失败或全部为空时返回 `None`。
///
/// 资金流 / 分时 走 [`crate::data_provider::service`] 缓存层，
/// 与 ReAct Agent 的 `fetch_fund_flow` 工具共享同一份结果，避免重复抓取。
pub(super) async fn fetch_extra_context(
    code: &str,
    kline_data: &[KlineData],
) -> Result<ExtraContext, String> {
    // 筹码分布（纯本地计算）
    let chip = crate::data_provider::compute_chip_distribution(kline_data);
    let chip_section = crate::data_provider::format_chip_prompt(&chip);

    // 资金流 + 日内分时（缓存复用）
    let svc = crate::data_provider::service::service();
    let (flow_result, shape_result) =
        tokio::join!(svc.get_money_flow(code, 10), svc.get_intraday_shape(code));
    let flow_arc = flow_result.map_err(|error| format!("[{code}] 资金流不可用: {error}"))?;
    let shape_arc = shape_result.map_err(|error| format!("[{code}] 分时不可用: {error}"))?;
    let mut s = crate::data_provider::format_flow_prompt(&flow_arc, &shape_arc);

    if !chip_section.is_empty() {
        s.push_str(&chip_section);
    }

    // 龙虎榜席位（近 30 日）
    if let Ok(lhb) = crate::lhb_analyzer::LhbDataFetcher::new() {
        if let Ok(a) = lhb.analyze_stock_lhb(code).await {
            if a.recent_count > 0 {
                s.push_str("\n【龙虎榜席位特征（近30日）】\n");
                s.push_str(&format!(
                    "近30日上榜 {} 次 | 机构评分 {} | 游资评分 {} | 综合评分 {}\n",
                    a.recent_count, a.inst_score, a.hot_money_score, a.total_score
                ));
                if !a.reason.is_empty() {
                    s.push_str(&format!("席位解读: {}\n", a.reason));
                }
                if !a.risk_warning.is_empty() {
                    s.push_str(&format!("⚠️ 风险: {}\n", a.risk_warning));
                }
            }
        }
    }

    // 产业链主线归属（来自最近一次涨停主线聚类，chain_daily 表）
    match chain_mainline_note(code) {
        Ok(Some(chain_note)) => s.push_str(&chain_note),
        Ok(None) => {}
        Err(error) => {
            s.push_str(&format!("\n【产业链主线归属不可用】{error}\n"));
        }
    }

    if s.trim().is_empty() {
        Ok(ExtraContext {
            section: None,
            money_flow: if flow_arc.is_empty() {
                None
            } else {
                Some((*flow_arc).clone())
            },
        })
    } else {
        Ok(ExtraContext {
            section: Some(s),
            money_flow: if flow_arc.is_empty() {
                None
            } else {
                Some((*flow_arc).clone())
            },
        })
    }
}

/// 查询该股是否属于最近一次涨停主线聚类（chain_daily 表），是则返回提示片段。
fn chain_mainline_note(code: &str) -> Result<Option<String>, String> {
    let db =
        crate::database::DatabaseManager::try_get().ok_or_else(|| "数据库未初始化".to_string())?;
    let rows = db.get_latest_chain_clusters_strict()?;
    for row in rows {
        let codes: Vec<String> = serde_json::from_str(&row.stocks)
            .map_err(|error| format!("chain_daily {} stocks JSON 非法: {error}", row.concept))?;
        if codes.iter().any(|c| c == code) {
            let streak = db.get_chain_streak_days_strict(&row.concept, 10)?;
            return Ok(Some(format!(
                "\n【产业链主线归属】该股属于 {} 涨停主线「{}」（簇内 {} 只涨停，近10日该主线上榜 {} 天）。\
                 主线发酵期个股动量通常更强，但主线退潮时会被联动补跌，研判时请结合主线生命周期。\n",
                row.date,
                row.concept,
                codes.len(),
                streak
            )));
        }
    }
    Ok(None)
}
