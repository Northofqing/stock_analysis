//! 财报数据抓取（EPS / ROE / 毛利率 / 净利率 / 营收同比 / 净利润同比）
//!
//! 主数据源：东方财富 F10 `ZYZBAjaxNew`（主要财务指标，字段最全）
//! 备份数据源：东方财富 datacenter `RPT_LICO_FN_CPD`（业绩快报，URL/主机不同，抗单点故障）
//!
//! 其他数据源（新浪财经 openapi / 腾讯 ifzq.gtimg）接口在 2026-04 测试均已失效或 302 重定向，
//! 暂不接入；若后续新增可直接在 `fetch_with_fallback` 中追加候选。

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

/// 单期财报指标（用于多期序列）
#[derive(Debug, Clone, Default)]
pub struct FinancialPeriod {
    pub report_date: Option<String>,
    pub eps: Option<f64>,
    pub roe: Option<f64>,
    pub revenue_yoy: Option<f64>,
    pub net_profit_yoy: Option<f64>,
    pub gross_margin: Option<f64>,
    pub net_margin: Option<f64>,
    /// 每股经营活动现金流量（元）
    pub op_cash_flow_ps: Option<f64>,
    /// 总资产周转率（次）
    pub total_asset_turnover: Option<f64>,
    /// 资产负债率（%）
    pub debt_to_assets: Option<f64>,
}

impl FinancialPeriod {
    pub fn any(&self) -> bool {
        self.eps.is_some()
            || self.roe.is_some()
            || self.revenue_yoy.is_some()
            || self.net_profit_yoy.is_some()
            || self.gross_margin.is_some()
            || self.net_margin.is_some()
            || self.op_cash_flow_ps.is_some()
            || self.total_asset_turnover.is_some()
            || self.debt_to_assets.is_some()
    }

    /// 权益乘数 = 1 / (1 - 资产负债率)
    pub fn equity_multiplier(&self) -> Option<f64> {
        self.debt_to_assets.and_then(|d| {
            let eq_ratio = 1.0 - d / 100.0;
            if eq_ratio > 1e-6 {
                Some(1.0 / eq_ratio)
            } else {
                None
            }
        })
    }

    /// 杜邦三因子分解：返回 (净利率 %, 总资产周转率 次, 权益乘数, 理论ROE %)
    /// 理论 ROE = 净利率 × 周转率 × 权益乘数（注意净利率为 %，结果直接得 %）
    pub fn dupont(&self) -> Option<(f64, f64, f64, f64)> {
        let nm = self.net_margin?;
        let at = self.total_asset_turnover?;
        let em = self.equity_multiplier()?;
        Some((nm, at, em, nm * at * em))
    }

    /// 经营性现金流 / 净利润 ≈ 每股经营现金流 / EPS（两者同口径每股）
    /// 该比率反映盈利质量：>=1 优秀；0.5~1 健康；<0.5 偏弱；<=0 风险
    pub fn cfo_to_ni_ratio(&self) -> Option<f64> {
        match (self.op_cash_flow_ps, self.eps) {
            (Some(cfo), Some(eps)) if eps.abs() > 1e-6 => Some(cfo / eps),
            _ => None,
        }
    }
}

/// 财务异常信号评分（启发式，不等同于完整 Beneish M-Score）
#[derive(Debug, Clone, Default)]
pub struct QualityReport {
    /// 异常风险评分 0~100，分值越高风险越大
    pub risk_score: u32,
    /// 触发的红旗列表（人类可读）
    pub flags: Vec<String>,
    /// 综合等级
    pub level: &'static str,
}

/// 基于多期财务序列（从新到旧）评估财务异常信号
/// 返回 None 表示数据不足无法评估
pub fn assess_quality(history: &[FinancialPeriod]) -> Option<QualityReport> {
    if history.is_empty() {
        return None;
    }
    let latest = &history[0];
    let mut flags: Vec<String> = Vec::new();
    let mut score: u32 = 0;

    // 1. 利润/营收增速背离 + 现金流孱弱：典型应计利润注水
    if let (Some(np), Some(rev)) = (latest.net_profit_yoy, latest.revenue_yoy) {
        if np - rev > 20.0 {
            if let Some(r) = latest.cfo_to_ni_ratio() {
                if r < 0.5 {
                    flags.push(format!(
                        "净利增速({:.1}%)远高于营收增速({:.1}%)且 CFO/NI={:.2} 偏低 → 应计利润可疑",
                        np, rev, r
                    ));
                    score += 25;
                }
            }
        }
    }

    // 2. 毛利率突变 (与上一期相比变动 > 5pp)
    if history.len() >= 2 {
        if let (Some(cur), Some(prev)) = (latest.gross_margin, history[1].gross_margin) {
            let diff = cur - prev;
            if diff.abs() > 5.0 {
                flags.push(format!(
                    "毛利率单期突变 {:+.2}pp（{:.2}% → {:.2}%）→ 成本/口径异常",
                    diff, prev, cur
                ));
                score += 15;
            }
        }
    }

    // 3. 盈利质量突然恶化：上期健康，本期跌入风险区
    if history.len() >= 2 {
        if let (Some(cur), Some(prev)) = (latest.cfo_to_ni_ratio(), history[1].cfo_to_ni_ratio()) {
            if prev >= 0.8 && cur < 0.3 {
                flags.push(format!(
                    "CFO/NI 单期骤降 {:.2} → {:.2}（盈利含金量突恶化）",
                    prev, cur
                ));
                score += 20;
            }
        }
    }

    // 4. 超高速增长可疑（基数效应/一次性损益）
    if let Some(np) = latest.net_profit_yoy {
        if np > 150.0 {
            flags.push(format!(
                "净利 YoY {:.1}% 过高 → 警惕基数效应/非经常性损益",
                np
            ));
            score += 10;
        }
    }
    if let Some(rev) = latest.revenue_yoy {
        if rev > 100.0 {
            flags.push(format!(
                "营收 YoY {:.1}% 过高 → 警惕一次性合并/口径调整",
                rev
            ));
            score += 10;
        }
    }

    // 5. EPS 持续上升但 ROE 同期持续下降：可能稀释或资产堆积
    if history.len() >= 3 {
        let eps_v: Vec<f64> = history.iter().take(4).filter_map(|p| p.eps).collect();
        let roe_v: Vec<f64> = history.iter().take(4).filter_map(|p| p.roe).collect();
        if eps_v.len() >= 3 && roe_v.len() >= 3 {
            // history 是新→旧，反转得到时间正序
            let eps_chrono: Vec<f64> = eps_v.iter().rev().cloned().collect();
            let roe_chrono: Vec<f64> = roe_v.iter().rev().cloned().collect();
            let eps_up = eps_chrono.windows(2).all(|w| w[1] >= w[0] - 0.001);
            let roe_down = roe_chrono.windows(2).all(|w| w[1] <= w[0] + 0.001);
            if eps_up && roe_down {
                flags.push("EPS 持续上行但 ROE 持续下行 → 可能股本扩张/资产堆积稀释回报".into());
                score += 15;
            }
        }
    }

    // 6. 持续低质量盈利：近 4 期 CFO/NI 均值 < 0.3
    let ratios: Vec<f64> = history
        .iter()
        .take(4)
        .filter_map(|p| p.cfo_to_ni_ratio())
        .collect();
    if ratios.len() >= 3 {
        let avg = ratios.iter().sum::<f64>() / ratios.len() as f64;
        if avg < 0.3 {
            flags.push(format!(
                "近{}期 CFO/NI 均值仅 {:.2} → 长期盈利质量低",
                ratios.len(),
                avg
            ));
            score += 15;
        }
    }

    let score = score.min(100);
    let level = if score >= 60 {
        "高风险⚠️"
    } else if score >= 30 {
        "需关注"
    } else if score > 0 {
        "轻微提示"
    } else {
        "无明显异常"
    };

    Some(QualityReport {
        risk_score: score,
        flags,
        level,
    })
}

/// 最新一期财报核心指标 + 多期历史序列
#[derive(Debug, Clone, Default)]
pub struct Financials {
    pub report_date: Option<String>,  // 报告期，如 "2025-12-31"
    pub eps: Option<f64>,             // 基本每股收益（元）
    pub roe: Option<f64>,             // 加权净资产收益率 (%)
    pub revenue_yoy: Option<f64>,     // 营业总收入同比 (%)
    pub net_profit_yoy: Option<f64>,  // 归母净利润同比 (%)
    pub gross_margin: Option<f64>,    // 销售毛利率 (%)
    pub net_margin: Option<f64>,      // 销售净利率 (%)
    pub source: Option<&'static str>, // 命中的数据源标签
    /// 多期历史序列，按时间从新到旧排列；第 0 项与顶层 latest 字段一致
    pub history: Vec<FinancialPeriod>,
}

impl Financials {
    /// 是否至少一项核心指标有值
    pub fn any(&self) -> bool {
        self.eps.is_some()
            || self.roe.is_some()
            || self.revenue_yoy.is_some()
            || self.net_profit_yoy.is_some()
            || self.gross_margin.is_some()
            || self.net_margin.is_some()
    }
}

/// 转 A 股代码为 EM 市场前缀代码，如 `600519 -> SH600519`、`000001 -> SZ000001`
fn to_em_secucode(code: &str) -> String {
    let upper_prefix = if code.starts_with('6') || code.starts_with("900") {
        "SH"
    } else if code.starts_with('0') || code.starts_with('3') || code.starts_with("200") {
        "SZ"
    } else if code.starts_with('8') || code.starts_with('4') {
        "BJ"
    } else {
        "SH"
    };
    format!("{}{}", upper_prefix, code)
}

/// 数字/字符串转有限 f64；已提供但非法的字段是整批错误，不当作缺失。
fn as_f64(v: &Value, field: &str) -> Result<Option<f64>> {
    match v {
        Value::Null => Ok(None),
        Value::Number(n) => n
            .as_f64()
            .filter(|value| value.is_finite())
            .map(Some)
            .ok_or_else(|| anyhow!("财务字段 {field} 不是有限数字: {v}")),
        Value::String(s) if s.trim().is_empty() => Ok(None),
        Value::String(s) => s
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .map(Some)
            .ok_or_else(|| anyhow!("财务字段 {field} 非法: {s:?}")),
        _ => Err(anyhow!("财务字段 {field} 类型非法: {v}")),
    }
}

/// 字段备选查找：遇到第一个非空非 null 的字段即返回
fn pick_f64(obj: &Value, keys: &[&str]) -> Result<Option<f64>> {
    for k in keys {
        if let Some(v) = obj.get(*k) {
            match as_f64(v, k)? {
                Some(value) => return Ok(Some(value)),
                None => continue,
            }
        }
    }
    Ok(None)
}

fn required_report_date(obj: &Value, keys: &[&str]) -> Result<String> {
    for k in keys {
        if let Some(v) = obj.get(*k) {
            if v.is_null() {
                continue;
            }
            let s = v
                .as_str()
                .ok_or_else(|| anyhow!("财务字段 {k} 必须是日期字符串: {v}"))?;
            let cleaned = s.split_whitespace().next().unwrap_or(s).trim();
            chrono::NaiveDate::parse_from_str(cleaned, "%Y-%m-%d")
                .map_err(|error| anyhow!("财务字段 {k} 日期非法 {cleaned:?}: {error}"))?;
            return Ok(cleaned.to_string());
        }
    }
    Err(anyhow!("财务记录缺少必填报告期字段: {keys:?}"))
}

fn parse_f10_period(item: &Value) -> Result<FinancialPeriod> {
    let period = FinancialPeriod {
        report_date: Some(required_report_date(item, &["REPORT_DATE"])?),
        eps: pick_f64(item, &["EPSJB", "EPSXS", "EPSKCJB"])?,
        roe: pick_f64(item, &["ROEJQ", "ROEKCJQ"])?,
        revenue_yoy: pick_f64(item, &["TOTALOPERATEREVETZ"])?,
        net_profit_yoy: pick_f64(item, &["PARENTNETPROFITTZ"])?,
        gross_margin: pick_f64(item, &["XSMLL"])?,
        net_margin: pick_f64(item, &["XSJLL"])?,
        op_cash_flow_ps: pick_f64(item, &["MGJYXJJE", "MGJYXJL"])?,
        total_asset_turnover: pick_f64(item, &["TOAZZL"])?,
        debt_to_assets: pick_f64(item, &["ZCFZL"])?,
    };
    if !period.any() {
        return Err(anyhow!(
            "财务记录 {} 不含任何有效指标",
            period.report_date.as_deref().unwrap_or("-")
        ));
    }
    Ok(period)
}

/// BR-115: validate the complete F10 response before retaining the newest 20 periods.
fn parse_f10_response(json: &Value) -> Result<Financials> {
    let data = json
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("EM-F10 无 data 数组"))?;
    if data.is_empty() {
        return Err(anyhow!("EM-F10 data 为空"));
    }

    let mut parsed = data
        .iter()
        .map(parse_f10_period)
        .collect::<Result<Vec<_>>>()?;
    for pair in parsed.windows(2) {
        let newer = pair[0]
            .report_date
            .as_deref()
            .ok_or_else(|| anyhow!("EM-F10 新报告期缺失"))?;
        let older = pair[1]
            .report_date
            .as_deref()
            .ok_or_else(|| anyhow!("EM-F10 旧报告期缺失"))?;
        if newer <= older {
            return Err(anyhow!("EM-F10 报告期重复或非降序: {newer} -> {older}"));
        }
    }
    parsed.truncate(20);
    let latest = parsed
        .first()
        .cloned()
        .ok_or_else(|| anyhow!("EM-F10 最新报告期缺失"))?;
    Ok(Financials {
        report_date: latest.report_date.clone(),
        eps: latest.eps,
        roe: latest.roe,
        revenue_yoy: latest.revenue_yoy,
        net_profit_yoy: latest.net_profit_yoy,
        gross_margin: latest.gross_margin,
        net_margin: latest.net_margin,
        source: Some("东方财富F10"),
        history: parsed,
    })
}

/// BR-115: validate the complete datacenter response and its real latest period.
fn parse_datacenter_response(json: &Value) -> Result<Financials> {
    let data = json
        .get("result")
        .and_then(|result| result.get("data"))
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("EM-DC 无 result.data 数组"))?;
    let latest = data.first().ok_or_else(|| anyhow!("EM-DC data 为空"))?;
    let period = FinancialPeriod {
        report_date: Some(required_report_date(latest, &["REPORTDATE"])?),
        eps: pick_f64(latest, &["BASIC_EPS"])?,
        roe: pick_f64(latest, &["WEIGHTAVG_ROE"])?,
        revenue_yoy: pick_f64(latest, &["YSTZ"])?,
        net_profit_yoy: pick_f64(latest, &["SJLTZ"])?,
        gross_margin: pick_f64(latest, &["XSMLL"])?,
        net_margin: None,
        op_cash_flow_ps: None,
        total_asset_turnover: None,
        debt_to_assets: None,
    };
    if !period.any() {
        return Err(anyhow!("EM-DC 最新报告期不含任何有效指标"));
    }
    Ok(Financials {
        report_date: period.report_date.clone(),
        eps: period.eps,
        roe: period.roe,
        revenue_yoy: period.revenue_yoy,
        net_profit_yoy: period.net_profit_yoy,
        gross_margin: period.gross_margin,
        net_margin: None,
        source: Some("东方财富DC"),
        history: vec![period],
    })
}

/// 主源：东方财富 F10 `ZYZBAjaxNew`（主要财务指标，字段最全）
///
/// URL: https://emweb.eastmoney.com/PC_HSF10/NewFinanceAnalysis/ZYZBAjaxNew?type=0&code=SH600519
async fn fetch_from_eastmoney_f10(client: &reqwest::Client, code: &str) -> Result<Financials> {
    let secucode = to_em_secucode(code);
    let url = format!(
        "https://emweb.eastmoney.com/PC_HSF10/NewFinanceAnalysis/ZYZBAjaxNew?type=0&code={}",
        secucode
    );
    log::debug!("[财报][EM-F10] {}", url);

    let resp = client
        .get(&url)
        .header("Accept", "application/json, text/plain, */*")
        .header("Referer", "https://emweb.eastmoney.com/")
        .send()
        .await
        .context("EM-F10 请求失败")?;
    if !resp.status().is_success() {
        return Err(anyhow!("EM-F10 状态码 {}", resp.status()));
    }
    let text = resp.text().await.context("EM-F10 读取响应失败")?;
    let json: Value = serde_json::from_str(&text).context("EM-F10 JSON 解析失败")?;

    parse_f10_response(&json)
}

/// 备份源：东方财富 datacenter `RPT_LICO_FN_CPD`（业绩快报）
///
/// URL 主机不同（datacenter-web.eastmoney.com），抗单点。
/// 缺少"销售净利率"（无 XSJLL 对应列）。
async fn fetch_from_eastmoney_datacenter(
    client: &reqwest::Client,
    code: &str,
) -> Result<Financials> {
    // filter 内嵌双引号，URL 编码由 reqwest 处理
    let filter = format!("(SECURITY_CODE=\"{}\")", code);
    let url = format!(
        "https://datacenter-web.eastmoney.com/api/data/v1/get?\
         sortColumns=REPORTDATE&sortTypes=-1&pageSize=1&pageNumber=1&\
         reportName=RPT_LICO_FN_CPD&\
         columns=SECURITY_CODE,REPORTDATE,BASIC_EPS,WEIGHTAVG_ROE,YSTZ,SJLTZ,XSMLL&\
         filter={}",
        urlencoding::encode(&filter)
    );
    log::debug!("[财报][EM-DC] {}", url);

    let resp = client
        .get(&url)
        .header("Accept", "application/json, text/plain, */*")
        .header("Referer", "https://data.eastmoney.com/")
        .send()
        .await
        .context("EM-DC 请求失败")?;
    if !resp.status().is_success() {
        return Err(anyhow!("EM-DC 状态码 {}", resp.status()));
    }
    let text = resp.text().await.context("EM-DC 读取响应失败")?;
    let json: Value = serde_json::from_str(&text).context("EM-DC JSON 解析失败")?;

    parse_datacenter_response(&json)
}

fn select_financial_source_results(
    code: &str,
    results: Vec<(&'static str, Result<Financials>)>,
) -> Result<Financials> {
    let mut failures = Vec::with_capacity(results.len());
    for (source, result) in results {
        match result {
            Ok(financials) if financials.any() => {
                log::info!(
                    "[财报] {} 命中 {}（报告期 {}）",
                    code,
                    source,
                    financials.report_date.as_deref().unwrap_or("-")
                );
                return Ok(financials);
            }
            Ok(_) => failures.push(format!("{source}: empty")),
            Err(error) => failures.push(format!("{source}: {error}")),
        }
    }
    Err(anyhow!(
        "[财报] {code} 全部真实来源失败: {}",
        failures.join("; ")
    ))
}

/// 多源带回退异步入口：依次尝试主源 → 备份源，返回首个包含真实字段的结果。
/// 所有源失败或返回空数据时保留完整错误，不生成默认财务对象。
pub async fn fetch_with_fallback_async(client: &reqwest::Client, code: &str) -> Result<Financials> {
    let primary = fetch_from_eastmoney_f10(client, code).await;
    if matches!(&primary, Ok(financials) if financials.any()) {
        return select_financial_source_results(code, vec![("EM-F10", primary)]);
    }
    let secondary = fetch_from_eastmoney_datacenter(client, code).await;
    select_financial_source_results(code, vec![("EM-F10", primary), ("EM-DC", secondary)])
}

/// 同步包装：仅允许从已有 tokio runtime 上下文调用；失败显式返回。
pub fn fetch_with_fallback_blocking(client: &reqwest::Client, code: &str) -> Result<Financials> {
    // 修复 Top10#5 (2026-06-29 audit): 用 crate::block_on_async 统一替代
    if tokio::runtime::Handle::try_current().is_err() {
        return Err(anyhow!("[财报] 无 tokio runtime，无法抓取 {code}"));
    }
    let client = client.clone();
    let code_s = code.to_string();
    crate::block_on_async(async move { fetch_with_fallback_async(&client, &code_s).await })
}

#[cfg(test)]
mod br115_tests {
    use super::*;

    #[test]
    fn all_financial_source_failures_remain_errors() {
        let result = select_financial_source_results(
            "TEST_CODE600519",
            vec![
                ("EM-F10", Err(anyhow!("timeout"))),
                ("EM-DC", Ok(Financials::default())),
            ],
        );

        let error = result.expect_err("all failed or empty sources must not become defaults");
        assert!(error.to_string().contains("EM-F10: timeout"));
        assert!(error.to_string().contains("EM-DC: empty"));
    }

    #[test]
    fn malformed_present_financial_field_rejects_the_entire_period() {
        let item = serde_json::json!({
            "REPORT_DATE": "2026-06-30 00:00:00",
            "EPSJB": "not-a-number",
            "ROEJQ": 12.5
        });
        assert!(parse_f10_period(&item).is_err());
    }

    #[test]
    fn financial_period_requires_a_valid_report_date_and_at_least_one_metric() {
        assert!(parse_f10_period(&serde_json::json!({"EPSJB": 1.0})).is_err());
        assert!(parse_f10_period(&serde_json::json!({
            "REPORT_DATE": "2026-99-99",
            "EPSJB": 1.0
        }))
        .is_err());
        assert!(parse_f10_period(&serde_json::json!({
            "REPORT_DATE": "2026-06-30"
        }))
        .is_err());
    }

    #[test]
    fn financial_period_ratios_are_available_only_from_valid_real_inputs() {
        let empty = FinancialPeriod::default();
        assert!(!empty.any());
        assert_eq!(empty.equity_multiplier(), None);
        assert_eq!(empty.dupont(), None);
        assert_eq!(empty.cfo_to_ni_ratio(), None);

        let period = FinancialPeriod {
            eps: Some(2.0),
            net_margin: Some(10.0),
            op_cash_flow_ps: Some(3.0),
            total_asset_turnover: Some(0.5),
            debt_to_assets: Some(50.0),
            ..Default::default()
        };
        assert!(period.any());
        assert_eq!(period.equity_multiplier(), Some(2.0));
        assert_eq!(period.dupont(), Some((10.0, 0.5, 2.0, 10.0)));
        assert_eq!(period.cfo_to_ni_ratio(), Some(1.5));

        let insolvent = FinancialPeriod {
            debt_to_assets: Some(100.0),
            eps: Some(0.0),
            op_cash_flow_ps: Some(1.0),
            ..Default::default()
        };
        assert_eq!(insolvent.equity_multiplier(), None);
        assert_eq!(insolvent.cfo_to_ni_ratio(), None);
    }

    #[test]
    fn quality_assessment_covers_all_risk_levels_and_caps_extreme_scores() {
        assert!(assess_quality(&[]).is_none());
        let clean = FinancialPeriod {
            eps: Some(1.0),
            roe: Some(10.0),
            revenue_yoy: Some(5.0),
            net_profit_yoy: Some(5.0),
            gross_margin: Some(30.0),
            op_cash_flow_ps: Some(1.0),
            ..Default::default()
        };
        let clean_report = assess_quality(std::slice::from_ref(&clean)).expect("clean report");
        assert_eq!(clean_report.risk_score, 0);
        assert_eq!(clean_report.level, "无明显异常");

        let light = FinancialPeriod {
            net_profit_yoy: Some(151.0),
            ..Default::default()
        };
        assert_eq!(assess_quality(&[light]).unwrap().level, "轻微提示");

        let attention = FinancialPeriod {
            eps: Some(1.0),
            op_cash_flow_ps: Some(0.1),
            revenue_yoy: Some(0.0),
            net_profit_yoy: Some(30.0),
            gross_margin: Some(50.0),
            ..Default::default()
        };
        let previous = FinancialPeriod {
            eps: Some(1.0),
            op_cash_flow_ps: Some(0.5),
            gross_margin: Some(40.0),
            ..Default::default()
        };
        assert_eq!(
            assess_quality(&[attention, previous]).unwrap().level,
            "需关注"
        );

        let extreme = vec![
            FinancialPeriod {
                eps: Some(3.0),
                roe: Some(10.0),
                revenue_yoy: Some(120.0),
                net_profit_yoy: Some(200.0),
                gross_margin: Some(50.0),
                op_cash_flow_ps: Some(0.3),
                ..Default::default()
            },
            FinancialPeriod {
                eps: Some(2.0),
                roe: Some(11.0),
                gross_margin: Some(40.0),
                op_cash_flow_ps: Some(1.6),
                ..Default::default()
            },
            FinancialPeriod {
                eps: Some(1.0),
                roe: Some(12.0),
                gross_margin: Some(39.0),
                op_cash_flow_ps: Some(0.1),
                ..Default::default()
            },
            FinancialPeriod {
                eps: Some(0.5),
                roe: Some(13.0),
                gross_margin: Some(38.0),
                op_cash_flow_ps: Some(0.0),
                ..Default::default()
            },
        ];
        let report = assess_quality(&extreme).expect("extreme report");
        assert_eq!(report.risk_score, 100);
        assert_eq!(report.level, "高风险⚠️");
        assert!(report
            .flags
            .iter()
            .any(|flag| flag.contains("应计利润可疑")));
        assert!(report
            .flags
            .iter()
            .any(|flag| flag.contains("毛利率单期突变")));
        assert!(report
            .flags
            .iter()
            .any(|flag| flag.contains("CFO/NI 单期骤降")));
        assert!(report
            .flags
            .iter()
            .any(|flag| flag.contains("EPS 持续上行")));
        assert!(report
            .flags
            .iter()
            .any(|flag| flag.contains("长期盈利质量低")));
    }

    #[test]
    fn financial_protocol_helpers_cover_market_codes_aliases_and_strict_types() {
        assert_eq!(to_em_secucode("600519"), "SH600519");
        assert_eq!(to_em_secucode("900901"), "SH900901");
        assert_eq!(to_em_secucode("000001"), "SZ000001");
        assert_eq!(to_em_secucode("300750"), "SZ300750");
        assert_eq!(to_em_secucode("430047"), "BJ430047");
        assert_eq!(to_em_secucode("TEST_CODE_000001"), "SHTEST_CODE_000001");

        assert_eq!(as_f64(&Value::Null, "x").unwrap(), None);
        assert_eq!(as_f64(&serde_json::json!(1.5), "x").unwrap(), Some(1.5));
        assert_eq!(as_f64(&serde_json::json!(" 2.5 "), "x").unwrap(), Some(2.5));
        assert!(as_f64(&Value::Bool(true), "x").is_err());
        assert_eq!(
            pick_f64(
                &serde_json::json!({"a": null, "b": "", "c": 3}),
                &["a", "b", "c"]
            )
            .unwrap(),
            Some(3.0)
        );
        assert_eq!(pick_f64(&serde_json::json!({}), &["x"]).unwrap(), None);
    }

    #[test]
    fn complete_f10_period_uses_alias_fields_and_preserves_report_date() {
        let period = parse_f10_period(&serde_json::json!({
            "REPORT_DATE": "2026-06-30 00:00:00",
            "EPSXS": "1.2",
            "ROEKCJQ": 12.0,
            "TOTALOPERATEREVETZ": 8.0,
            "PARENTNETPROFITTZ": 9.0,
            "XSMLL": 30.0,
            "XSJLL": 10.0,
            "MGJYXJL": 1.5,
            "TOAZZL": 0.6,
            "ZCFZL": 50.0
        }))
        .expect("complete F10 period");
        assert_eq!(period.report_date.as_deref(), Some("2026-06-30"));
        assert_eq!(period.eps, Some(1.2));
        assert_eq!(period.roe, Some(12.0));
        assert_eq!(period.op_cash_flow_ps, Some(1.5));
        assert_eq!(period.total_asset_turnover, Some(0.6));
        assert_eq!(period.debt_to_assets, Some(50.0));
    }

    #[test]
    fn source_selector_returns_first_nonempty_real_financial_batch() {
        let selected = Financials {
            report_date: Some("2026-06-30".into()),
            eps: Some(1.0),
            source: Some("测试真实源"),
            ..Default::default()
        };
        let result = select_financial_source_results(
            "TEST_CODE_000001",
            vec![("EMPTY", Ok(Financials::default())), ("REAL", Ok(selected))],
        )
        .expect("first nonempty source");
        assert_eq!(result.eps, Some(1.0));
        assert!(result.any());
    }

    #[test]
    fn blocking_financial_fetch_requires_an_existing_runtime() {
        assert!(fetch_with_fallback_blocking(&reqwest::Client::new(), "TEST_CODE_000001").is_err());
    }

    #[test]
    fn f10_document_parser_keeps_newest_twenty_strict_periods() {
        let data: Vec<Value> = (0..21)
            .map(|index| {
                serde_json::json!({
                    "REPORT_DATE": format!("{:04}-12-31", 2026 - index),
                    "EPSJB": 1.0 + index as f64,
                    "ROEJQ": 10.0
                })
            })
            .collect();
        let parsed = parse_f10_response(&serde_json::json!({"data": data}))
            .expect("valid descending F10 response");
        assert_eq!(parsed.source, Some("东方财富F10"));
        assert_eq!(parsed.history.len(), 20);
        assert_eq!(parsed.report_date.as_deref(), Some("2026-12-31"));
        assert_eq!(parsed.eps, Some(1.0));

        assert!(parse_f10_response(&serde_json::json!({})).is_err());
        assert!(parse_f10_response(&serde_json::json!({"data": []})).is_err());
        let wrong_order = serde_json::json!({"data": [
            {"REPORT_DATE": "2025-12-31", "EPSJB": 1.0},
            {"REPORT_DATE": "2026-12-31", "EPSJB": 2.0}
        ]});
        assert!(parse_f10_response(&wrong_order).is_err());
    }

    #[test]
    fn datacenter_document_parser_requires_one_real_latest_period() {
        let parsed = parse_datacenter_response(&serde_json::json!({"result": {"data": [{
            "REPORTDATE": "2026-06-30 00:00:00",
            "BASIC_EPS": "1.5",
            "WEIGHTAVG_ROE": 12.0,
            "YSTZ": 8.0,
            "SJLTZ": 9.0,
            "XSMLL": 30.0
        }]}}))
        .expect("valid datacenter response");
        assert_eq!(parsed.source, Some("东方财富DC"));
        assert_eq!(parsed.report_date.as_deref(), Some("2026-06-30"));
        assert_eq!(parsed.history.len(), 1);
        assert_eq!(parsed.net_margin, None);

        assert!(parse_datacenter_response(&serde_json::json!({})).is_err());
        assert!(parse_datacenter_response(&serde_json::json!({"result": {"data": []}})).is_err());
        assert!(
            parse_datacenter_response(&serde_json::json!({"result": {"data": [{
                "REPORTDATE": "2026-06-30"
            }]}}))
            .is_err()
        );
    }
}
