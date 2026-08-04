//! 真实口径辅助数据：主力资金流 / 日内分时 / 筹码分布 / 产业链主线。
//!
//! 产出单份 Markdown 片段，既会被塞进 AI prompt，也会被挂到 `AnalysisResult.money_flow_section`
//! 给通知展示。

use crate::capital_flow::{IntradayShape, MoneyFlowSummary};
use crate::data_provider::KlineData;
use std::sync::Arc;

/// `fetch_extra_context` 的产物：
/// - `section`：用于通知 / AI prompt 的 Markdown 片段（与之前等价）。
/// - `money_flow`：原始资金流时序，用于打分器做 EWMA / 单日反弹判定。
#[derive(Clone)]
pub(super) struct ExtraContext {
    pub section: Option<String>,
    pub money_flow: Option<MoneyFlowSummary>,
}

fn require_complete_flow_context(
    code: &str,
    flow_result: anyhow::Result<Arc<MoneyFlowSummary>>,
    shape_result: anyhow::Result<Arc<IntradayShape>>,
) -> Result<(Arc<MoneyFlowSummary>, Arc<IntradayShape>), String> {
    let flow = flow_result.map_err(|error| format!("[{code}] 资金流不可用: {error:#}"))?;
    let shape = shape_result
        .map_err(|error| format!("[{code}] intraday money-flow shape 不可用: {error:#}"))?;
    Ok((flow, shape))
}

/// BR-114/BR-115: compose only already validated real-domain evidence.
fn compose_extra_context(
    flow: &MoneyFlowSummary,
    shape: &crate::capital_flow::IntradayShape,
    chip_section: &str,
    chain_note: Result<Option<String>, String>,
) -> ExtraContext {
    let mut section = crate::capital_flow::format_for_prompt(flow, shape);
    if !chip_section.is_empty() {
        section.push_str(chip_section);
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

fn parse_chain_business_date(value: &str) -> Result<chrono::NaiveDate, String> {
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|error| format!("chain_daily business date 非法 {value:?}: {error}"))
}

/// 抓取资金流/分时，并合并由 K 线计算的筹码分布，返回格式化后的 Markdown。
///
/// 资金流或形态 Gateway 失败会拒绝完整上下文，不会伪装成空成功。
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
    let (flow, shape) = require_complete_flow_context(code, flow_result, shape_result)?;
    Ok(compose_extra_context(
        &flow,
        &shape,
        &chip_section,
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
    let as_of = parse_chain_business_date(&row.date)?;
    let streak = db.get_chain_appearance_days_as_of_strict(&row.concept, 10, as_of)?;
    Ok(Some(render_chain_mainline_note(row, cluster_size, streak)))
}

fn render_chain_mainline_note(
    row: &crate::database::concepts::ChainDailyRow,
    cluster_size: usize,
    streak: i64,
) -> String {
    format!(
        "\n【产业链主线归属】该股属于 {} 涨停主线「{}」（簇内 {} 只涨停，近10个自然日该主线上榜 {} 天）。\
         主线发酵期个股动量通常更强，但主线退潮时会被联动补跌，研判时请结合主线生命周期。\n",
        row.date, row.concept, cluster_size, streak
    )
}

#[cfg(test)]
mod tests {
    use super::{
        chain_mainline_note, compose_extra_context, find_chain_mainline, parse_chain_business_date,
        render_chain_mainline_note, require_complete_flow_context,
    };
    use crate::capital_flow::{IntradayShape, MoneyFlowDay, MoneyFlowSummary};
    use crate::database::concepts::ChainDailyRow;

    fn flow() -> MoneyFlowSummary {
        MoneyFlowSummary {
            days: vec![MoneyFlowDay {
                date: "2026-07-18".to_string(),
                main_net: 100_000_000.0,
                xl_net: 60_000_000.0,
                big_net: 40_000_000.0,
                main_pct: 5.0,
                pct_chg: Some(2.0),
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
        let result = compose_extra_context(
            &flow,
            &shape,
            "\n【TEST_CODE_筹码证据】\n",
            Ok(Some("\n【TEST_CODE_主线证据】\n".to_string())),
        );
        let section = result.section.expect("complete section");
        for expected in ["主力资金流向", "日内分时形态", "筹码证据", "主线证据"]
        {
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
            Ok(None),
        );
        let empty_section = empty.section.expect("missing shape remains explicit");
        assert!(empty_section.contains("日内分时形态"));
        assert!(empty_section.contains("数据缺失"));
        assert!(empty.money_flow.is_none());

        let unavailable = compose_extra_context(
            &MoneyFlowSummary::default(),
            &IntradayShape::default(),
            "",
            Err("TEST_CODE_数据库失败".to_string()),
        );
        let section = unavailable.section.expect("explicit failure section");
        assert!(section.contains("产业链主线归属不可用"));
        assert!(section.contains("数据库失败"));
    }

    #[test]
    fn persisted_chain_business_date_is_strictly_validated() {
        assert!(parse_chain_business_date("2026-07-20").is_ok());
        assert!(parse_chain_business_date("bad-date").is_err());
        assert!(parse_chain_business_date("2026-02-30").is_err());
    }

    #[test]
    fn intraday_gateway_failure_rejects_the_pipeline_context() {
        let error = require_complete_flow_context(
            "TEST_CODE_600001",
            Ok(std::sync::Arc::new(flow())),
            Err(anyhow::anyhow!(
                "Magic TDX intraday-shape gateway unavailable"
            )),
        )
        .expect_err("gateway failure must reject the pipeline context");

        assert!(error.contains("TEST_CODE_600001"));
        assert!(error.contains("Magic TDX"));
        assert!(error.contains("intraday money-flow shape"));
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
        assert!(note.contains("近10个自然日"));
        assert!(note.contains("上榜 3 天"));
        assert!(render_chain_mainline_note(row, count, 0).contains("上榜 0 天"));
    }

    struct ChainNoteGuard {
        date: String,
    }

    impl Drop for ChainNoteGuard {
        fn drop(&mut self) {
            use diesel::prelude::*;
            if let Some(db) = crate::database::DatabaseManager::try_get() {
                if let Ok(mut connection) = db.get_conn() {
                    let _ = diesel::sql_query("DELETE FROM chain_daily WHERE date = ?")
                        .bind::<diesel::sql_types::Text, _>(&self.date)
                        .execute(&mut connection);
                }
            }
        }
    }

    #[test]
    #[serial_test::serial]
    fn chain_mainline_note_reads_complete_latest_cluster_from_real_sqlite() {
        crate::database::DatabaseManager::init(None).expect("test database initialization");
        let date = "2299-07-18".to_string();
        let _guard = ChainNoteGuard { date: date.clone() };
        crate::database::DatabaseManager::get()
            .save_chain_clusters(
                &date,
                &[(
                    "TEST_CODE_本地主线".to_string(),
                    vec![
                        "TEST_CODE_000001".to_string(),
                        "TEST_CODE_000002".to_string(),
                    ],
                    1,
                )],
            )
            .expect("persist complete chain evidence");

        let note = chain_mainline_note("TEST_CODE_000001")
            .expect("strict chain query")
            .expect("matching mainline");
        assert!(note.contains("TEST_CODE_本地主线"));
        assert!(note.contains("簇内 2 只涨停"));
        assert_eq!(
            chain_mainline_note("TEST_CODE_999999").expect("strict non-match"),
            None
        );
    }
}
