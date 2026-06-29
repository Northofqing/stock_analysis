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
            if eq_ratio > 1e-6 { Some(1.0 / eq_ratio) } else { None }
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
        if let (Some(cur), Some(prev)) =
            (latest.cfo_to_ni_ratio(), history[1].cfo_to_ni_ratio())
        {
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
            flags.push(format!("净利 YoY {:.1}% 过高 → 警惕基数效应/非经常性损益", np));
            score += 10;
        }
    }
    if let Some(rev) = latest.revenue_yoy {
        if rev > 100.0 {
            flags.push(format!("营收 YoY {:.1}% 过高 → 警惕一次性合并/口径调整", rev));
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
                flags.push(
                    "EPS 持续上行但 ROE 持续下行 → 可能股本扩张/资产堆积稀释回报".into(),
                );
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
            flags.push(format!("近{}期 CFO/NI 均值仅 {:.2} → 长期盈利质量低", ratios.len(), avg));
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
    } else if code.starts_with("688") {
        "SH"
    } else if code.starts_with('0')
        || code.starts_with('3')
        || code.starts_with("200")
    {
        "SZ"
    } else if code.starts_with('8') || code.starts_with('4') {
        "BJ"
    } else {
        "SH"
    };
    format!("{}{}", upper_prefix, code)
}

/// 数字/字符串转 f64
fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// 字段备选查找：遇到第一个非空非 null 的字段即返回
fn pick_f64(obj: &Value, keys: &[&str]) -> Option<f64> {
    for k in keys {
        if let Some(v) = obj.get(*k) {
            if !v.is_null() {
                if let Some(f) = as_f64(v) {
                    return Some(f);
                }
            }
        }
    }
    None
}

fn pick_string(obj: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(v) = obj.get(*k) {
            if let Some(s) = v.as_str() {
                // 截掉 " 00:00:00" 后缀
                let cleaned = s.split_whitespace().next().unwrap_or(s).to_string();
                if !cleaned.is_empty() {
                    return Some(cleaned);
                }
            }
        }
    }
    None
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

    let data = json
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("EM-F10 无 data 数组"))?;
    if data.is_empty() {
        return Err(anyhow!("EM-F10 data 为空"));
    }

    // 解析所有期（接口本身已按报告期从新到旧排列）；最多保留 20 期 (~5 年)
    const MAX_PERIODS: usize = 20;
    let history: Vec<FinancialPeriod> = data
        .iter()
        .take(MAX_PERIODS)
        .map(|item| FinancialPeriod {
            report_date: pick_string(item, &["REPORT_DATE"]),
            eps: pick_f64(item, &["EPSJB", "EPSXS", "EPSKCJB"]),
            roe: pick_f64(item, &["ROEJQ", "ROEKCJQ"]),
            revenue_yoy: pick_f64(item, &["TOTALOPERATEREVETZ"]),
            net_profit_yoy: pick_f64(item, &["PARENTNETPROFITTZ"]),
            gross_margin: pick_f64(item, &["XSMLL"]),
            net_margin: pick_f64(item, &["XSJLL"]),
            op_cash_flow_ps: pick_f64(item, &["MGJYXJJE", "MGJYXJL"]),
            total_asset_turnover: pick_f64(item, &["TOAZZL"]),
            debt_to_assets: pick_f64(item, &["ZCFZL"]),
        })
        .filter(|p| p.any() || p.report_date.is_some())
        .collect();

    let latest_p = history.first().cloned().unwrap_or_default();
    let f = Financials {
        report_date: latest_p.report_date.clone(),
        eps: latest_p.eps,
        roe: latest_p.roe,
        revenue_yoy: latest_p.revenue_yoy,
        net_profit_yoy: latest_p.net_profit_yoy,
        gross_margin: latest_p.gross_margin,
        net_margin: latest_p.net_margin,
        source: Some("东方财富F10"),
        history,
    };
    Ok(f)
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

    let data = json
        .get("result")
        .and_then(|r| r.get("data"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("EM-DC 无 result.data 数组"))?;
    let latest = data
        .first()
        .ok_or_else(|| anyhow!("EM-DC data 为空"))?;

    let period = FinancialPeriod {
        report_date: pick_string(latest, &["REPORTDATE"]),
        eps: pick_f64(latest, &["BASIC_EPS"]),
        roe: pick_f64(latest, &["WEIGHTAVG_ROE"]),
        revenue_yoy: pick_f64(latest, &["YSTZ"]),
        net_profit_yoy: pick_f64(latest, &["SJLTZ"]),
        gross_margin: pick_f64(latest, &["XSMLL"]),
        net_margin: None,
        op_cash_flow_ps: None,
        total_asset_turnover: None,
        debt_to_assets: None,
    };
    let f = Financials {
        report_date: period.report_date.clone(),
        eps: period.eps,
        roe: period.roe,
        revenue_yoy: period.revenue_yoy,
        net_profit_yoy: period.net_profit_yoy,
        gross_margin: period.gross_margin,
        net_margin: None, // 该报告不含净利率
        source: Some("东方财富DC"),
        history: vec![period],
    };
    Ok(f)
}

/// 多源带回退异步入口：依次尝试主源 → 备份源，返回首个 `any()==true` 的结果；
/// 所有源都失败返回 `Default`（全 None），调用方视为"未获取到"。
pub async fn fetch_with_fallback_async(client: &reqwest::Client, code: &str) -> Financials {
    match fetch_from_eastmoney_f10(client, code).await {
        Ok(f) if f.any() => {
            log::info!(
                "[财报] {} 命中 EM-F10（报告期 {}）",
                code,
                f.report_date.as_deref().unwrap_or("-")
            );
            return f;
        }
        Ok(_) => log::warn!("[财报] {} EM-F10 返回空数据", code),
        Err(e) => log::warn!("[财报] {} EM-F10 失败: {}", code, e),
    }

    match fetch_from_eastmoney_datacenter(client, code).await {
        Ok(f) if f.any() => {
            log::info!(
                "[财报] {} 命中 EM-DC（报告期 {}）",
                code,
                f.report_date.as_deref().unwrap_or("-")
            );
            return f;
        }
        Ok(_) => log::warn!("[财报] {} EM-DC 返回空数据", code),
        Err(e) => log::warn!("[财报] {} EM-DC 失败: {}", code, e),
    }

    Financials::default()
}

/// 同步包装：在已有 tokio runtime 上下文内调用；无 runtime 时返回 Default。
pub fn fetch_with_fallback_blocking(client: &reqwest::Client, code: &str) -> Financials {
    // 修复 Top10#5 (2026-06-29 audit): 用 crate::block_on_async 统一替代
    if tokio::runtime::Handle::try_current().is_err() {
        log::debug!("[财报] 无 tokio runtime，跳过财报抓取");
        return Financials::default();
    }
    let client = client.clone();
    let code_s = code.to_string();
    crate::block_on_async(async move {
        fetch_with_fallback_async(&client, &code_s).await
    })
}
