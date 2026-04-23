//! 财报数据抓取（EPS / ROE / 毛利率 / 净利率 / 营收同比 / 净利润同比）
//!
//! 主数据源：东方财富 F10 `ZYZBAjaxNew`（主要财务指标，字段最全）
//! 备份数据源：东方财富 datacenter `RPT_LICO_FN_CPD`（业绩快报，URL/主机不同，抗单点故障）
//!
//! 其他数据源（新浪财经 openapi / 腾讯 ifzq.gtimg）接口在 2026-04 测试均已失效或 302 重定向，
//! 暂不接入；若后续新增可直接在 `fetch_with_fallback` 中追加候选。

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

/// 最新一期财报的核心指标
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
    let latest = data
        .first()
        .ok_or_else(|| anyhow!("EM-F10 data 为空"))?;

    let f = Financials {
        report_date: pick_string(latest, &["REPORT_DATE"]),
        eps: pick_f64(latest, &["EPSJB", "EPSXS", "EPSKCJB"]),
        roe: pick_f64(latest, &["ROEJQ", "ROEKCJQ"]),
        revenue_yoy: pick_f64(latest, &["TOTALOPERATEREVETZ"]),
        net_profit_yoy: pick_f64(latest, &["PARENTNETPROFITTZ"]),
        gross_margin: pick_f64(latest, &["XSMLL"]),
        net_margin: pick_f64(latest, &["XSJLL"]),
        source: Some("东方财富F10"),
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

    let f = Financials {
        report_date: pick_string(latest, &["REPORTDATE"]),
        eps: pick_f64(latest, &["BASIC_EPS"]),
        roe: pick_f64(latest, &["WEIGHTAVG_ROE"]),
        revenue_yoy: pick_f64(latest, &["YSTZ"]),
        net_profit_yoy: pick_f64(latest, &["SJLTZ"]),
        gross_margin: pick_f64(latest, &["XSMLL"]),
        net_margin: None, // 该报告不含净利率
        source: Some("东方财富DC"),
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
    let handle = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => {
            log::debug!("[财报] 无 tokio runtime，跳过财报抓取");
            return Financials::default();
        }
    };
    let client = client.clone();
    let code = code.to_string();
    tokio::task::block_in_place(|| {
        handle.block_on(async move { fetch_with_fallback_async(&client, &code).await })
    })
}
