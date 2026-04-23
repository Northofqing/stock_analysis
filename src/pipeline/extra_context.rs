//! 真实口径辅助数据：主力资金流 / 日内分时 / 龙虎榜席位 / 筹码分布。
//!
//! 产出单份 Markdown 片段，既会被塞进 AI prompt，也会被挂到 `AnalysisResult.money_flow_section`
//! 给通知展示。

use crate::data_provider::KlineData;

/// 在 blocking 线程池里抓取资金流/分时/LHB 数据，并合并由 K 线计算的筹码分布，返回格式化后的 Markdown。
///
/// 抓取失败或全部为空时返回 `None`。
pub(super) async fn fetch_extra_context(
    code: &str,
    kline_data: &[KlineData],
) -> Option<String> {
    let code_owned = code.to_string();
    // 先在调用线程内计算筹码分布（纯 CPU，不需要阻塞 I/O 线程池）
    let chip = crate::data_provider::compute_chip_distribution(kline_data);
    let chip_section = crate::data_provider::format_chip_prompt(&chip);

    tokio::task::spawn_blocking(move || {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .user_agent(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
                 AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            )
            .build()
            .ok()?;
        let flow = crate::data_provider::fetch_money_flow_blocking(&client, &code_owned, 10);
        let shape = crate::data_provider::fetch_intraday_shape_blocking(&client, &code_owned);
        let mut s = crate::data_provider::format_flow_prompt(&flow, &shape);

        // 筹码分布（纯本地计算）追加
        if !chip_section.is_empty() {
            s.push_str(&chip_section);
        }

        // 龙虎榜席位（近 30 日）
        if let Ok(lhb) = crate::lhb_analyzer::LhbDataFetcher::new() {
            if let Ok(rt) = tokio::runtime::Handle::try_current() {
                if let Ok(a) = rt.block_on(lhb.analyze_stock_lhb(&code_owned)) {
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
        }

        if s.trim().is_empty() {
            None
        } else {
            Some(s)
        }
    })
    .await
    .ok()
    .flatten()
}
