//! 行业对标：取个股所属东方财富二级行业板块，与同业 PE / PB / ROE / 增速做横向对比
//!
//! 数据源：
//! 1. `push2*.eastmoney.com/api/qt/stock/get?secid=...&fields=f127` 取个股所属行业名（如"白酒Ⅱ"）
//! 2. 全 A 二级行业列表（`fs=m:90+t:2`）按名称→BK 代码映射（进程内缓存）
//! 3. `fs=b:BKxxxx` 拉取该行业全部成份股 PE/PB/ROE/净利润同比
//! 4. 计算行业中位数 + 个股百分位（PE/PB 越低越便宜=百分位 0；ROE/增速 越高越好=百分位 100）

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::OnceLock;

const HOSTS: &[&str] = &[
    "push2delay.eastmoney.com",
    "push2.eastmoney.com",
    "82.push2.eastmoney.com",
];

/// 行业对标结果
#[derive(Debug, Clone, Default)]
pub struct IndustryBenchmark {
    pub industry_name: String,
    pub board_code: String,
    pub peer_count: usize,
    pub stock_pe: Option<f64>,
    pub stock_pb: Option<f64>,
    pub stock_roe: Option<f64>,
    pub stock_growth: Option<f64>,
    pub median_pe: Option<f64>,
    pub median_pb: Option<f64>,
    pub median_roe: Option<f64>,
    pub median_growth: Option<f64>,
    /// PE/PB 百分位：值越低排名越靠前（0 = 行业最便宜，100 = 行业最贵）
    pub pe_percentile: Option<f64>,
    pub pb_percentile: Option<f64>,
    /// ROE / 增速 百分位：值越高排名越靠前（100 = 行业最优）
    pub roe_percentile: Option<f64>,
    pub growth_percentile: Option<f64>,
}

fn secid_for(code: &str) -> String {
    let market = if code.starts_with('6') || code.starts_with("688") || code.starts_with("900") {
        1
    } else if code.starts_with('8') || code.starts_with('4') {
        0 // 北交所归 0 市场
    } else {
        0
    };
    format!("{}.{}", market, code)
}

async fn try_get(client: &reqwest::Client, path: &str) -> Result<Value> {
    let mut last_err: Option<anyhow::Error> = None;
    for host in HOSTS {
        let url = format!("https://{}{}", host, path);
        match client.get(&url).header("Referer", "https://quote.eastmoney.com/").send().await {
            Ok(resp) => match resp.json::<Value>().await {
                Ok(v) => return Ok(v),
                Err(e) => last_err = Some(anyhow!("{}: parse {}", host, e)),
            },
            Err(e) => last_err = Some(anyhow!("{}: {}", host, e)),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("all hosts failed")))
}

/// 进程内缓存的 `行业名 -> BK代码` 映射
static INDUSTRY_MAP: OnceLock<Mutex<Option<HashMap<String, String>>>> = OnceLock::new();

async fn load_industry_map(client: &reqwest::Client) -> Result<HashMap<String, String>> {
    let mut out: HashMap<String, String> = HashMap::new();
    for pn in 1..=5 {
        let path = format!(
            "/api/qt/clist/get?pn={}&pz=100&po=1&np=1&fltt=2&invt=2&fid=f12&fs=m:90+t:2&fields=f12,f14",
            pn
        );
        let v = try_get(client, &path).await?;
        let rows = v.pointer("/data/diff").and_then(|x| x.as_array()).cloned().unwrap_or_default();
        if rows.is_empty() {
            break;
        }
        for r in &rows {
            let code = r.get("f12").and_then(|x| x.as_str()).unwrap_or("");
            let name = r.get("f14").and_then(|x| x.as_str()).unwrap_or("");
            if !code.is_empty() && !name.is_empty() {
                out.insert(name.to_string(), code.to_string());
            }
        }
        if rows.len() < 100 {
            break;
        }
    }
    if out.is_empty() {
        return Err(anyhow!("行业列表为空"));
    }
    Ok(out)
}

async fn get_industry_map(client: &reqwest::Client) -> Result<HashMap<String, String>> {
    let slot = INDUSTRY_MAP.get_or_init(|| Mutex::new(None));
    {
        if let Ok(g) = slot.lock() {
            if let Some(m) = g.as_ref() {
                return Ok(m.clone());
            }
        }
    }
    let m = load_industry_map(client).await?;
    if let Ok(mut g) = slot.lock() {
        *g = Some(m.clone());
    }
    Ok(m)
}

async fn fetch_industry_name(client: &reqwest::Client, code: &str) -> Result<String> {
    let path = format!(
        "/api/qt/stock/get?secid={}&fields=f127&invt=2",
        secid_for(code)
    );
    let v = try_get(client, &path).await?;
    let name = v.pointer("/data/f127").and_then(|x| x.as_str())
        .ok_or_else(|| anyhow!("缺少 f127 行业字段"))?;
    Ok(name.to_string())
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 {
        Some(v[n / 2])
    } else {
        Some((v[n / 2 - 1] + v[n / 2]) / 2.0)
    }
}

/// 百分位：peers 中严格小于 target 的占比（0..100），用于"target 越低排名越靠前"语义
fn percentile_low(target: f64, peers: &[f64]) -> Option<f64> {
    let valid: Vec<f64> = peers.iter().copied().filter(|x| x.is_finite()).collect();
    if valid.is_empty() {
        return None;
    }
    let below = valid.iter().filter(|x| **x < target).count();
    Some(below as f64 / valid.len() as f64 * 100.0)
}

async fn fetch_constituents(
    client: &reqwest::Client,
    bk_code: &str,
) -> Result<Vec<(String, f64, f64, f64, f64)>> {
    // 返回 (code, pe, pb, roe, growth)
    let path = format!(
        "/api/qt/clist/get?pn=1&pz=200&po=1&np=1&fltt=2&invt=2&fid=f3&fs=b:{}&fields=f12,f9,f23,f37,f129",
        bk_code
    );
    let v = try_get(client, &path).await?;
    let rows = v.pointer("/data/diff").and_then(|x| x.as_array()).cloned().unwrap_or_default();
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let code = r.get("f12").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let pe = r.get("f9").and_then(|x| x.as_f64()).unwrap_or(f64::NAN);
        let pb = r.get("f23").and_then(|x| x.as_f64()).unwrap_or(f64::NAN);
        let roe = r.get("f37").and_then(|x| x.as_f64()).unwrap_or(f64::NAN);
        let growth = r.get("f129").and_then(|x| x.as_f64()).unwrap_or(f64::NAN);
        if !code.is_empty() {
            out.push((code, pe, pb, roe, growth));
        }
    }
    Ok(out)
}

pub async fn fetch_async(client: &reqwest::Client, code: &str) -> Result<IndustryBenchmark> {
    let industry_name = fetch_industry_name(client, code).await.context("取行业名")?;
    let map = get_industry_map(client).await.context("加载行业列表")?;
    let bk_code = map
        .get(&industry_name)
        .cloned()
        .ok_or_else(|| anyhow!("行业名未匹配到 BK 代码: {}", industry_name))?;

    let rows = fetch_constituents(client, &bk_code).await.context("取成份股")?;
    if rows.is_empty() {
        return Err(anyhow!("行业 {} 无成份股", bk_code));
    }

    let mut stock_pe = None;
    let mut stock_pb = None;
    let mut stock_roe = None;
    let mut stock_growth = None;
    let mut pes = Vec::new();
    let mut pbs = Vec::new();
    let mut roes = Vec::new();
    let mut growths = Vec::new();

    for (c, pe, pb, roe, gr) in &rows {
        // PE 排除负值（亏损股不参与估值排名）
        if pe.is_finite() && *pe > 0.0 {
            pes.push(*pe);
        }
        if pb.is_finite() && *pb > 0.0 {
            pbs.push(*pb);
        }
        if roe.is_finite() {
            roes.push(*roe);
        }
        if gr.is_finite() {
            growths.push(*gr);
        }
        if c == code {
            if pe.is_finite() && *pe > 0.0 {
                stock_pe = Some(*pe);
            }
            if pb.is_finite() && *pb > 0.0 {
                stock_pb = Some(*pb);
            }
            if roe.is_finite() {
                stock_roe = Some(*roe);
            }
            if gr.is_finite() {
                stock_growth = Some(*gr);
            }
        }
    }

    let pe_percentile = stock_pe.and_then(|x| percentile_low(x, &pes));
    let pb_percentile = stock_pb.and_then(|x| percentile_low(x, &pbs));
    // ROE/增速 用"高于多少同业"的百分位
    let roe_percentile = stock_roe.and_then(|x| percentile_low(x, &roes));
    let growth_percentile = stock_growth.and_then(|x| percentile_low(x, &growths));

    Ok(IndustryBenchmark {
        industry_name,
        board_code: bk_code,
        peer_count: rows.len(),
        stock_pe,
        stock_pb,
        stock_roe,
        stock_growth,
        median_pe: median(&pes),
        median_pb: median(&pbs),
        median_roe: median(&roes),
        median_growth: median(&growths),
        pe_percentile,
        pb_percentile,
        roe_percentile,
        growth_percentile,
    })
}

/// 同步包装：在已有 tokio runtime 上下文内调用
pub fn fetch_blocking(client: &reqwest::Client, code: &str) -> Option<IndustryBenchmark> {
    // 修复 Top10#5 (2026-06-29 audit): 用 crate::block_on_async 统一替代
    if tokio::runtime::Handle::try_current().is_err() {
        return None;
    }
    let client = client.clone();
    let code_s = code.to_string();
    crate::block_on_async(async move {
        match fetch_async(&client, &code_s).await {
            Ok(v) => Some(v),
            Err(e) => {
                log::warn!("[行业对标] {} 失败: {}", code_s, e);
                None
            }
        }
    })
}
