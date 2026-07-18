//! 大盘状态门控：避免在系统性普跌/普涨日对个股建议做机械化输出。
//!
//! 检测逻辑（盘后数据）：
//! - 指数维度：沪深300 当日涨跌幅
//! - 广度维度：自选股当日涨跌家数比
//!
//! 三态判定：
//! - 普跌：指数 ≤ -1% 且 ≥70% 自选股下跌
//! - 普涨：指数 ≥ +1% 且 ≥70% 自选股上涨
//! - 结构性/常态：其余情况
//!
//! 门控规则（仅普跌日生效）：
//! - 『建议减仓』且当日逆势收红或跑赢指数 ≥2pp 的个股 → 降级为『观望』。
//!   理由：普跌日的相对强度是资金抗跌信号，机械斩仓易卖在恐慌底。
//! - 『建议卖出』（评分 <20）不豁免：个股自身信号已极弱。
//! - 指数或广度数据缺失时跳过门控，绝不编造市场状态。

use log::{info, warn};

use crate::data_provider::DataFetcherManager;

use super::AnalysisResult;

const INDEX_CODE: &str = "sh000300";
const INDEX_NAME: &str = "沪深300";

/// 普跌日豁免阈值：相对指数跑赢幅度（百分点）
const OUTPERFORM_PP: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq)]
enum RegimeKind {
    /// 普跌（系统性下跌）
    BroadDecline,
    /// 普涨（系统性上涨）
    BroadRally,
    /// 结构性/常态
    Structural,
}

impl RegimeKind {
    fn label(&self) -> &'static str {
        match self {
            RegimeKind::BroadDecline => "🔻 普跌（系统性下跌）",
            RegimeKind::BroadRally => "🔺 普涨（系统性上涨）",
            RegimeKind::Structural => "🔀 结构性行情 / 常态",
        }
    }
}

/// 检测大盘状态并对普跌日的机械减仓建议做豁免调整。
///
/// 返回渲染好的 Markdown 区块（插入日报头部）；
/// 指数或广度数据不足时返回 `None`，不做任何调整。
pub(super) fn apply(
    data_manager: &DataFetcherManager,
    results: &mut [AnalysisResult],
) -> Result<Option<String>, String> {
    apply_with_index_change(results, || {
        data_manager
            .get_daily_data(INDEX_CODE, 5)
            .map(|(data, _)| data.first().map(|row| row.pct_chg))
            .map_err(|error| {
                format!(
                    "BR-122 {}({}) data source failed: {error:#}",
                    INDEX_NAME, INDEX_CODE
                )
            })
    })
}

/// BR-122: separate real index acquisition from deterministic validation and gating.
fn apply_with_index_change<F>(
    results: &mut [AnalysisResult],
    fetch_index_change: F,
) -> Result<Option<String>, String>
where
    F: FnOnce() -> Result<Option<f64>, String>,
{
    if results.is_empty() {
        return Ok(None);
    }
    let index_chg = fetch_index_change()?.ok_or_else(|| {
        format!(
            "BR-122 {}({}) data source returned empty",
            INDEX_NAME, INDEX_CODE
        )
    })?;
    if !index_chg.is_finite() || index_chg.abs() > 20.0 {
        return Err(format!(
            "BR-122 {} change is invalid: {index_chg}",
            INDEX_NAME
        ));
    }

    // 2. 自选股广度
    for result in results.iter() {
        if result
            .chg_1d
            .is_some_and(|change| !change.is_finite() || change.abs() > 20.0)
        {
            return Err(format!(
                "BR-122 {} {} daily change is invalid: {:?}",
                result.name, result.code, result.chg_1d
            ));
        }
    }
    let known: Vec<f64> = results.iter().filter_map(|r| r.chg_1d).collect();
    if known.is_empty() || known.len() * 2 < results.len() {
        warn!(
            "[大盘门控] 自选股当日涨跌数据不足（{}/{}），跳过大盘状态门控",
            known.len(),
            results.len()
        );
        return Ok(None);
    }
    let up = known.iter().filter(|c| **c > 0.0).count();
    let down = known.iter().filter(|c| **c < 0.0).count();
    let total = known.len();
    let down_ratio = down as f64 / total as f64;
    let up_ratio = up as f64 / total as f64;

    // 3. 三态判定
    let kind = if index_chg <= -1.0 && down_ratio >= 0.7 {
        RegimeKind::BroadDecline
    } else if index_chg >= 1.0 && up_ratio >= 0.7 {
        RegimeKind::BroadRally
    } else {
        RegimeKind::Structural
    };
    info!(
        "[大盘门控] {} 当日 {:+.2}% | 自选股 {} 涨 / {} 跌 → {}",
        INDEX_NAME,
        index_chg,
        up,
        down,
        kind.label()
    );

    // 4. 普跌日门控：跑赢指数的『建议减仓』降级为『观望』
    let mut adjusted: Vec<(String, String, f64)> = Vec::new();
    if kind == RegimeKind::BroadDecline {
        for r in results.iter_mut() {
            if r.operation_advice != "建议减仓" {
                continue;
            }
            let Some(chg) = r.chg_1d else { continue };
            if chg > 0.0 || chg - index_chg >= OUTPERFORM_PP {
                if r.original_advice.is_none() {
                    r.original_advice = Some(r.operation_advice.clone());
                }
                r.operation_advice = "观望".to_string();
                info!(
                    "[大盘门控] {} {} 普跌日跑赢指数（{:+.2}% vs {:+.2}%），『建议减仓』→『观望』",
                    r.name, r.code, chg, index_chg
                );
                adjusted.push((r.name.clone(), r.code.clone(), chg));
            }
        }
    }

    Ok(Some(render_section(kind, index_chg, up, down, &adjusted)))
}

fn render_section(
    kind: RegimeKind,
    index_chg: f64,
    up: usize,
    down: usize,
    adjusted: &[(String, String, f64)],
) -> String {
    let mut s = String::new();
    s.push_str("## 🌡️ 大盘状态\n\n");
    s.push_str("| 维度 | 数值 |\n|------|------|\n");
    s.push_str(&format!("| {} 当日 | {:+.2}% |\n", INDEX_NAME, index_chg));
    s.push_str(&format!("| 自选股广度 | {} 涨 / {} 跌 |\n", up, down));
    s.push_str(&format!("| 行情定性 | {} |\n\n", kind.label()));

    match kind {
        RegimeKind::BroadDecline => {
            s.push_str(
                "> ⚠️ 今日为普跌日，个股普遍收跌含系统性因素，下方『减仓/卖出』建议需结合相对强度解读，谨防恐慌底机械斩仓。\n",
            );
            if adjusted.is_empty() {
                s.push_str("> 本次无个股触发普跌豁免（逆势收红或跑赢指数 ≥2pp）。\n");
            } else {
                s.push_str(&format!(
                    "> 已豁免 {} 只跑赢指数个股的机械减仓建议（『建议减仓』→『观望』）：\n\n",
                    adjusted.len()
                ));
                s.push_str(&format!(
                    "| 股票 | 代码 | 今日 | 相对{} | 调整 |\n|------|------|------|------|------|\n",
                    INDEX_NAME
                ));
                for (name, code, chg) in adjusted {
                    s.push_str(&format!(
                        "| {} | {} | {:+.2}% | {:+.2}pp | 建议减仓 → 观望 |\n",
                        name,
                        code,
                        chg,
                        chg - index_chg
                    ));
                }
            }
        }
        RegimeKind::BroadRally => {
            s.push_str(
                "> ⚠️ 今日为普涨日，个股普遍收涨含情绪因素，下方『买入』建议需警惕水涨船高式透支，优先确认量价与资金配合。\n",
            );
        }
        RegimeKind::Structural => {
            s.push_str("> 今日为结构性行情，个股建议以自身信号为准。\n");
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::apply_with_index_change;
    use crate::pipeline::AnalysisResult;

    fn result(code: &str, advice: &str, change: Option<f64>) -> AnalysisResult {
        let mut result: AnalysisResult = serde_json::from_value(serde_json::json!({
            "code": code,
            "name": format!("TEST_CODE_{code}"),
            "sentiment_score": 50,
            "ranking_score": 50,
            "operation_advice": advice,
            "trend_prediction": "TEST_CODE_盘整",
            "analysis_summary": "TEST_CODE_正文",
            "is_limit_up": false,
            "contrarian_signal": false
        }))
        .expect("valid result fixture");
        result.chg_1d = change;
        result
    }

    #[test]
    fn source_errors_empty_index_and_incomplete_breadth_are_explicit() {
        let mut results = vec![result("A", "观望", Some(1.0))];
        assert!(
            apply_with_index_change(&mut results, || Err("TEST_CODE_source".to_string()))
                .unwrap_err()
                .contains("TEST_CODE_source")
        );
        assert!(apply_with_index_change(&mut results, || Ok(None))
            .unwrap_err()
            .contains("empty"));
        assert!(apply_with_index_change(&mut [], || Ok(Some(0.0)))
            .unwrap()
            .is_none());

        let mut incomplete = vec![
            result("A", "观望", Some(1.0)),
            result("B", "观望", None),
            result("C", "观望", None),
        ];
        assert!(apply_with_index_change(&mut incomplete, || Ok(Some(0.0)))
            .unwrap()
            .is_none());
    }

    #[test]
    fn invalid_index_or_present_stock_change_rejects_complete_batch() {
        for index in [f64::NAN, 20.1] {
            let mut results = vec![result("A", "观望", Some(1.0))];
            assert!(apply_with_index_change(&mut results, || Ok(Some(index)))
                .unwrap_err()
                .contains("BR-122"));
        }
        for change in [f64::INFINITY, -20.1] {
            let mut results = vec![result("A", "观望", Some(change))];
            assert!(apply_with_index_change(&mut results, || Ok(Some(0.0)))
                .unwrap_err()
                .contains("BR-122"));
        }
    }

    #[test]
    fn broad_decline_adjusts_only_registered_reduce_advice_branches() {
        let mut results = vec![
            result("POSITIVE", "建议减仓", Some(0.1)),
            result("OUTPERFORM", "建议减仓", Some(-1.0)),
            result("WEAK", "建议减仓", Some(-2.5)),
            result("SELL", "建议卖出", Some(-3.0)),
            result("MISSING", "建议减仓", None),
        ];
        results[1].original_advice = Some("TEST_CODE_既有原建议".to_string());
        let rendered = apply_with_index_change(&mut results, || Ok(Some(-3.0)))
            .unwrap()
            .expect("complete decline section");
        assert_eq!(results[0].operation_advice, "观望");
        assert_eq!(results[0].original_advice.as_deref(), Some("建议减仓"));
        assert_eq!(results[1].operation_advice, "观望");
        assert_eq!(
            results[1].original_advice.as_deref(),
            Some("TEST_CODE_既有原建议")
        );
        assert_eq!(results[2].operation_advice, "建议减仓");
        assert_eq!(results[3].operation_advice, "建议卖出");
        assert_eq!(results[4].operation_advice, "建议减仓");
        for expected in ["普跌", "已豁免 2 只", "POSITIVE", "OUTPERFORM", "+3.10pp"] {
            assert!(
                rendered.contains(expected),
                "missing {expected}: {rendered}"
            );
        }
    }

    #[test]
    fn all_three_regimes_and_no_adjustment_render_their_registered_guidance() {
        for (index, changes, expected) in [
            (1.5, vec![1.0, 2.0, 3.0, -1.0], "普涨日"),
            (0.0, vec![1.0, -1.0, 0.0, 0.0], "结构性行情"),
            (-1.5, vec![-2.0, -2.0, -2.0, 1.0], "无个股触发普跌豁免"),
        ] {
            let mut results: Vec<_> = changes
                .into_iter()
                .enumerate()
                .map(|(idx, change)| result(&idx.to_string(), "观望", Some(change)))
                .collect();
            let rendered = apply_with_index_change(&mut results, || Ok(Some(index)))
                .unwrap()
                .expect("complete section");
            assert!(
                rendered.contains(expected),
                "missing {expected}: {rendered}"
            );
        }
    }
}
