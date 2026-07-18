//! 真实口径辅助数据：主力资金流 / 日内分时 / 龙虎榜席位 / 筹码分布。
//!
//! 产出单份 Markdown 片段，既会被塞进 AI prompt，也会被挂到 `AnalysisResult.money_flow_section`
//! 给通知展示。

use crate::data_provider::money_flow::MoneyFlowSummary;
use crate::data_provider::KlineData;

/// `fetch_extra_context` 的产物：
/// - `section`：用于通知 / AI prompt 的 Markdown 片段（与之前等价）。
/// - `money_flow`：原始资金流时序，用于打分器做 EWMA / 单日反弹判定。
#[derive(Clone)]
pub(super) struct ExtraContext {
    pub section: Option<String>,
    pub money_flow: Option<MoneyFlowSummary>,
}

/// BR-114/BR-115: compose only already validated real-domain evidence.
fn compose_extra_context(
    flow: &MoneyFlowSummary,
    shape: &crate::data_provider::IntradayShape,
    chip_section: &str,
    lhb: Option<&crate::lhb_analyzer::LhbAnalysis>,
    chain_note: Result<Option<String>, String>,
) -> ExtraContext {
    let mut section = crate::data_provider::format_flow_prompt(flow, shape);
    if !chip_section.is_empty() {
        section.push_str(chip_section);
    }
    if let Some(analysis) = lhb.filter(|analysis| analysis.recent_count > 0) {
        section.push_str("\n【龙虎榜席位特征（近30日）】\n");
        section.push_str(&format!(
            "近30日上榜 {} 次 | 机构评分 {} | 游资评分 {} | 综合评分 {}\n",
            analysis.recent_count,
            analysis.inst_score,
            analysis.hot_money_score,
            analysis.total_score
        ));
        if !analysis.reason.is_empty() {
            section.push_str(&format!("席位解读: {}\n", analysis.reason));
        }
        if !analysis.risk_warning.is_empty() {
            section.push_str(&format!("⚠️ 风险: {}\n", analysis.risk_warning));
        }
    }
    match chain_note {
        Ok(Some(note)) => section.push_str(&note),
        Ok(None) => {}
        Err(error) => section.push_str(&format!("\n【产业链主线归属不可用】{error}\n")),
    }
    ExtraContext {
        section: (!section.trim().is_empty()).then_some(section),
        money_flow: (!flow.is_empty()).then(|| flow.clone()),
    }
}

/// BR-114: malformed cluster JSON rejects the complete batch; no row skipping.
fn find_chain_mainline<'a>(
    code: &str,
    rows: &'a [crate::database::concepts::ChainDailyRow],
) -> Result<Option<(&'a crate::database::concepts::ChainDailyRow, usize)>, String> {
    for row in rows {
        let codes: Vec<String> = serde_json::from_str(&row.stocks)
            .map_err(|error| format!("chain_daily {} stocks JSON 非法: {error}", row.concept))?;
        if codes.iter().any(|candidate| candidate == code) {
            return Ok(Some((row, codes.len())));
        }
    }
    Ok(None)
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
    let lhb_analysis = match crate::lhb_analyzer::LhbDataFetcher::new() {
        Ok(fetcher) => fetcher.analyze_stock_lhb(code).await.ok(),
        Err(_) => None,
    };
    Ok(compose_extra_context(
        &flow_arc,
        &shape_arc,
        &chip_section,
        lhb_analysis.as_ref(),
        chain_mainline_note(code),
    ))
}

/// 查询该股是否属于最近一次涨停主线聚类（chain_daily 表），是则返回提示片段。
fn chain_mainline_note(code: &str) -> Result<Option<String>, String> {
    let db =
        crate::database::DatabaseManager::try_get().ok_or_else(|| "数据库未初始化".to_string())?;
    let rows = db.get_latest_chain_clusters_strict()?;
    let Some((row, cluster_size)) = find_chain_mainline(code, &rows)? else {
        return Ok(None);
    };
    let streak = db.get_chain_streak_days_strict(&row.concept, 10)?;
    Ok(Some(render_chain_mainline_note(row, cluster_size, streak)))
}

fn render_chain_mainline_note(
    row: &crate::database::concepts::ChainDailyRow,
    cluster_size: usize,
    streak: i64,
) -> String {
    format!(
        "\n【产业链主线归属】该股属于 {} 涨停主线「{}」（簇内 {} 只涨停，近10日该主线上榜 {} 天）。\
         主线发酵期个股动量通常更强，但主线退潮时会被联动补跌，研判时请结合主线生命周期。\n",
        row.date, row.concept, cluster_size, streak
    )
}

#[cfg(test)]
mod tests {
    use super::{compose_extra_context, find_chain_mainline, render_chain_mainline_note};
    use crate::data_provider::money_flow::{IntradayShape, MoneyFlowDay, MoneyFlowSummary};
    use crate::database::concepts::ChainDailyRow;
    use crate::lhb_analyzer::LhbAnalysis;

    fn flow() -> MoneyFlowSummary {
        MoneyFlowSummary {
            days: vec![MoneyFlowDay {
                date: "2026-07-18".to_string(),
                main_net: 100_000_000.0,
                xl_net: 60_000_000.0,
                big_net: 40_000_000.0,
                main_pct: 5.0,
                pct_chg: 2.0,
            }],
        }
    }

    #[test]
    fn complete_validated_context_composes_every_real_evidence_section() {
        let flow = flow();
        let shape = IntradayShape {
            date: "2026-07-18".to_string(),
            pre_close: 10.0,
            open_pct: 1.0,
            high_pct: 3.0,
            low_pct: -1.0,
            close_pct: 2.0,
            amplitude: 4.0,
            tail_30m_pct: Some(1.0),
            shape_label: "TEST_CODE_尾盘走强",
            present: true,
        };
        let lhb = LhbAnalysis {
            code: "TEST_CODE_000001".to_string(),
            name: "TEST_CODE_示例".to_string(),
            recent_count: 2,
            inst_score: 80,
            hot_money_score: 60,
            total_score: 70,
            reason: "TEST_CODE_机构证据".to_string(),
            risk_warning: "TEST_CODE_席位风险".to_string(),
        };
        let result = compose_extra_context(
            &flow,
            &shape,
            "\n【TEST_CODE_筹码证据】\n",
            Some(&lhb),
            Ok(Some("\n【TEST_CODE_主线证据】\n".to_string())),
        );
        let section = result.section.expect("complete section");
        for expected in [
            "主力资金流向",
            "日内分时形态",
            "筹码证据",
            "龙虎榜席位特征",
            "机构证据",
            "席位风险",
            "主线证据",
        ] {
            assert!(section.contains(expected), "missing {expected}: {section}");
        }
        assert_eq!(result.money_flow.expect("raw flow").days.len(), 1);
    }

    #[test]
    fn optional_evidence_remains_absent_or_explicitly_unavailable() {
        let empty = compose_extra_context(
            &MoneyFlowSummary::default(),
            &IntradayShape::default(),
            "",
            None,
            Ok(None),
        );
        assert_eq!(empty.section, None);
        assert!(empty.money_flow.is_none());

        let lhb = LhbAnalysis {
            code: "TEST_CODE_000001".to_string(),
            name: "TEST_CODE_示例".to_string(),
            recent_count: 1,
            inst_score: 0,
            hot_money_score: 0,
            total_score: 0,
            reason: String::new(),
            risk_warning: String::new(),
        };
        let unavailable = compose_extra_context(
            &MoneyFlowSummary::default(),
            &IntradayShape::default(),
            "",
            Some(&lhb),
            Err("TEST_CODE_数据库失败".to_string()),
        );
        let section = unavailable.section.expect("explicit failure section");
        assert!(section.contains("产业链主线归属不可用"));
        assert!(section.contains("数据库失败"));
        assert!(!section.contains("席位解读"));
        assert!(!section.contains("⚠️ 风险"));
    }

    fn chain(date: &str, concept: &str, stocks: &str) -> ChainDailyRow {
        ChainDailyRow {
            date: date.to_string(),
            concept: concept.to_string(),
            stocks: stocks.to_string(),
            continuation_count: 1,
        }
    }

    #[test]
    fn chain_match_requires_valid_complete_json_and_preserves_real_cluster_size() {
        let bad = vec![chain("2026-07-18", "TEST_CODE_坏主线", "not-json")];
        assert!(find_chain_mainline("TEST_CODE_000001", &bad).is_err());

        let rows = vec![
            chain("2026-07-18", "TEST_CODE_不匹配", r#"["TEST_CODE_999999"]"#),
            chain(
                "2026-07-18",
                "TEST_CODE_匹配主线",
                r#"["TEST_CODE_000001","TEST_CODE_000002"]"#,
            ),
        ];
        let (row, count) = find_chain_mainline("TEST_CODE_000001", &rows)
            .expect("valid rows")
            .expect("matching row");
        assert_eq!(row.concept, "TEST_CODE_匹配主线");
        assert_eq!(count, 2);
        assert!(find_chain_mainline("TEST_CODE_123456", &rows)
            .expect("valid rows")
            .is_none());

        let note = render_chain_mainline_note(row, count, 3);
        assert!(note.contains("2026-07-18"));
        assert!(note.contains("TEST_CODE_匹配主线"));
        assert!(note.contains("簇内 2 只涨停"));
        assert!(note.contains("上榜 3 天"));
    }
}
