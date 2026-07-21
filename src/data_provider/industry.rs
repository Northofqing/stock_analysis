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
// 修复 Top10#6 (2026-06-29 audit): 保留 std::sync::Mutex — `INDUSTRY_MAP: Mutex<Option<HashMap>>`
// 是 cache 读, lock 持有微秒级. 改 tokio Mutex 需重写所有 industry 调用方 (sync) → async.
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

#[derive(Debug, Clone)]
struct IndustryConstituent {
    code: String,
    pe: Option<f64>,
    pb: Option<f64>,
    roe: Option<f64>,
    growth: Option<f64>,
}

fn secid_for(code: &str) -> String {
    let market = if code.starts_with('6') || code.starts_with("900") {
        1
    } else {
        0
    };
    format!("{}.{}", market, code)
}

async fn try_get_from_hosts(client: &reqwest::Client, path: &str, hosts: &[&str]) -> Result<Value> {
    let mut last_err: Option<anyhow::Error> = None;
    for host in hosts {
        let url = if host.starts_with("http://") || host.starts_with("https://") {
            format!("{host}{path}")
        } else {
            format!("https://{host}{path}")
        };
        match client
            .get(&url)
            .header("Referer", "https://quote.eastmoney.com/")
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let body = resp.text().await.map_err(|error| error.to_string());
                match parse_industry_http_response(host, status, body) {
                    Ok(value) => return Ok(value),
                    Err(error) => last_err = Some(error),
                }
            }
            Err(e) => last_err = Some(anyhow!("{}: {}", host, e)),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("all hosts failed")))
}

fn parse_industry_http_response(
    host: &str,
    status: u16,
    body: std::result::Result<String, String>,
) -> Result<Value> {
    if !(200..300).contains(&status) {
        return Err(anyhow!("{host}: HTTP status {status}"));
    }
    let body = body.map_err(|error| anyhow!("{host}: response body {error}"))?;
    if body.trim().is_empty() {
        return Err(anyhow!("{host}: empty response body"));
    }
    serde_json::from_str(&body).map_err(|error| anyhow!("{host}: parse {error}"))
}

fn required_nonempty_text(row: &Value, field: &str) -> Result<String> {
    row.get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("行业字段 {field} 缺失、类型非法或为空"))
}

fn optional_finite_number(row: &Value, field: &str) -> Result<Option<f64>> {
    let Some(value) = row.get(field) else {
        return Ok(None);
    };
    match value {
        Value::Null => Ok(None),
        Value::Number(number) => number
            .as_f64()
            .filter(|number| number.is_finite())
            .map(Some)
            .ok_or_else(|| anyhow!("行业字段 {field} 不是有限数字: {value}")),
        Value::String(text) if text.trim().is_empty() => Ok(None),
        Value::String(text) => text
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite())
            .map(Some)
            .ok_or_else(|| anyhow!("行业字段 {field} 非法: {text:?}")),
        _ => Err(anyhow!("行业字段 {field} 类型非法: {value}")),
    }
}

/// BR-120: parse one complete industry-name mapping page without row skipping.
fn parse_industry_map_page(value: &Value) -> Result<HashMap<String, String>> {
    let rows = value
        .pointer("/data/diff")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("行业列表缺少 data.diff 数组"))?;
    let mut page = HashMap::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let code = required_nonempty_text(row, "f12")
            .with_context(|| format!("行业列表第 {} 行", index + 1))?;
        let name = required_nonempty_text(row, "f14")
            .with_context(|| format!("行业列表第 {} 行", index + 1))?;
        if let Some(previous) = page.insert(name.clone(), code.clone()) {
            return Err(anyhow!(
                "行业列表第 {} 行名称重复: {name} -> {previous}/{code}",
                index + 1
            ));
        }
    }
    Ok(page)
}

/// BR-120: parse one complete constituent page with explicit missing values.
fn parse_constituents_page(value: &Value) -> Result<Vec<IndustryConstituent>> {
    let rows = value
        .pointer("/data/diff")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("行业成份缺少 data.diff 数组"))?;
    let mut seen = std::collections::HashSet::with_capacity(rows.len());
    let mut parsed = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let code = required_nonempty_text(row, "f12")
            .with_context(|| format!("行业成份第 {} 行", index + 1))?;
        if !seen.insert(code.clone()) {
            return Err(anyhow!("行业成份代码重复: {code}"));
        }
        parsed.push(IndustryConstituent {
            code,
            pe: optional_finite_number(row, "f9")?,
            pb: optional_finite_number(row, "f23")?,
            roe: optional_finite_number(row, "f37")?,
            growth: optional_finite_number(row, "f129")?,
        });
    }
    Ok(parsed)
}

/// 进程内缓存的 `行业名 -> BK代码` 映射
static INDUSTRY_MAP: OnceLock<Mutex<Option<HashMap<String, String>>>> = OnceLock::new();

fn merge_industry_map_page(out: &mut HashMap<String, String>, value: &Value) -> Result<bool> {
    let page = parse_industry_map_page(value)?;
    if page.is_empty() {
        return Ok(true);
    }
    let page_len = page.len();
    for (name, code) in &page {
        if let Some(previous) = out.get(name) {
            return Err(anyhow!("行业列表跨页名称重复: {name} -> {previous}/{code}"));
        }
    }
    out.extend(page);
    Ok(page_len < 100)
}

async fn load_industry_map(client: &reqwest::Client) -> Result<HashMap<String, String>> {
    load_industry_map_from_hosts(client, HOSTS).await
}

async fn load_industry_map_from_hosts(
    client: &reqwest::Client,
    hosts: &[&str],
) -> Result<HashMap<String, String>> {
    let mut out: HashMap<String, String> = HashMap::new();
    for pn in 1..=5 {
        let path = format!(
            "/api/qt/clist/get?pn={}&pz=100&po=1&np=1&fltt=2&invt=2&fid=f12&fs=m:90+t:2&fields=f12,f14",
            pn
        );
        let v = try_get_from_hosts(client, &path, hosts).await?;
        if merge_industry_map_page(&mut out, &v)? {
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
    get_or_load_industry_map(slot, || load_industry_map(client)).await
}

async fn get_or_load_industry_map<F, Fut>(
    slot: &Mutex<Option<HashMap<String, String>>>,
    loader: F,
) -> Result<HashMap<String, String>>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<HashMap<String, String>>>,
{
    {
        if let Ok(g) = slot.lock() {
            if let Some(m) = g.as_ref() {
                return Ok(m.clone());
            }
        }
    }
    let m = loader().await?;
    if let Ok(mut g) = slot.lock() {
        *g = Some(m.clone());
    }
    Ok(m)
}

async fn fetch_industry_name(client: &reqwest::Client, code: &str) -> Result<String> {
    fetch_industry_name_from_hosts(client, code, HOSTS).await
}

/// 只读取个股所属行业名称，不额外拉取行业列表和成份股。
///
/// 盘后 R-03 用它补齐持仓/自选中缺失的行业字段；失败会由调用方按股票隔离，
/// 不会用猜测值或静默默认值替代真实数据。
pub async fn fetch_industry_name_only(client: &reqwest::Client, code: &str) -> Result<String> {
    fetch_industry_name(client, code).await
}

async fn fetch_industry_name_from_hosts(
    client: &reqwest::Client,
    code: &str,
    hosts: &[&str],
) -> Result<String> {
    let path = format!(
        "/api/qt/stock/get?secid={}&fields=f127&invt=2",
        secid_for(code)
    );
    let v = try_get_from_hosts(client, &path, hosts).await?;
    parse_industry_name(&v)
}

fn parse_industry_name(value: &Value) -> Result<String> {
    let name = value
        .pointer("/data/f127")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
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
) -> Result<Vec<IndustryConstituent>> {
    fetch_constituents_from_hosts(client, bk_code, HOSTS).await
}

async fn fetch_constituents_from_hosts(
    client: &reqwest::Client,
    bk_code: &str,
    hosts: &[&str],
) -> Result<Vec<IndustryConstituent>> {
    let path = format!(
        "/api/qt/clist/get?pn=1&pz=200&po=1&np=1&fltt=2&invt=2&fid=f3&fs=b:{}&fields=f12,f9,f23,f37,f129",
        bk_code
    );
    let v = try_get_from_hosts(client, &path, hosts).await?;
    parse_constituents_page(&v)
}

fn build_industry_benchmark(
    industry_name: String,
    board_code: String,
    code: &str,
    rows: &[IndustryConstituent],
) -> Result<IndustryBenchmark> {
    if rows.is_empty() {
        return Err(anyhow!("行业 {board_code} 无成份股"));
    }

    let mut stock_pe = None;
    let mut stock_pb = None;
    let mut stock_roe = None;
    let mut stock_growth = None;
    let mut pes = Vec::new();
    let mut pbs = Vec::new();
    let mut roes = Vec::new();
    let mut growths = Vec::new();

    for row in rows {
        if let Some(value) = row.pe.filter(|value| *value > 0.0) {
            pes.push(value);
        }
        if let Some(value) = row.pb.filter(|value| *value > 0.0) {
            pbs.push(value);
        }
        if let Some(value) = row.roe {
            roes.push(value);
        }
        if let Some(value) = row.growth {
            growths.push(value);
        }
        if row.code == code {
            stock_pe = row.pe.filter(|value| *value > 0.0);
            stock_pb = row.pb.filter(|value| *value > 0.0);
            stock_roe = row.roe;
            stock_growth = row.growth;
        }
    }

    let pe_percentile = stock_pe.and_then(|value| percentile_low(value, &pes));
    let pb_percentile = stock_pb.and_then(|value| percentile_low(value, &pbs));
    let roe_percentile = stock_roe.and_then(|value| percentile_low(value, &roes));
    let growth_percentile = stock_growth.and_then(|value| percentile_low(value, &growths));

    Ok(IndustryBenchmark {
        industry_name,
        board_code,
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

pub async fn fetch_async(client: &reqwest::Client, code: &str) -> Result<IndustryBenchmark> {
    let industry_name = fetch_industry_name(client, code)
        .await
        .context("取行业名")?;
    let map = get_industry_map(client).await.context("加载行业列表")?;
    let bk_code = resolve_industry_board_code(&map, &industry_name)?;
    let rows = fetch_constituents(client, &bk_code)
        .await
        .context("取成份股")?;
    build_industry_benchmark(industry_name, bk_code, code, &rows)
}

#[cfg(test)]
async fn fetch_async_from_hosts(
    client: &reqwest::Client,
    code: &str,
    hosts: &[&str],
) -> Result<IndustryBenchmark> {
    let industry_name = fetch_industry_name_from_hosts(client, code, hosts)
        .await
        .context("取行业名")?;
    let map = load_industry_map_from_hosts(client, hosts)
        .await
        .context("加载行业列表")?;
    let bk_code = resolve_industry_board_code(&map, &industry_name)?;
    let rows = fetch_constituents_from_hosts(client, &bk_code, hosts)
        .await
        .context("取成份股")?;
    build_industry_benchmark(industry_name, bk_code, code, &rows)
}

fn resolve_industry_board_code(
    map: &HashMap<String, String>,
    industry_name: &str,
) -> Result<String> {
    map.get(industry_name)
        .cloned()
        .ok_or_else(|| anyhow!("行业名未匹配到 BK 代码: {industry_name}"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secid_covers_shanghai_shenzhen_and_beijing_prefixes() {
        assert_eq!(secid_for("600519"), "1.600519");
        assert_eq!(secid_for("900901"), "1.900901");
        assert_eq!(secid_for("000001"), "0.000001");
        assert_eq!(secid_for("430047"), "0.430047");
    }

    #[test]
    fn median_filters_nonfinite_values_and_handles_odd_even_sets() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[f64::NAN, f64::INFINITY]), None);
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), Some(2.5));
        assert_eq!(median(&[f64::NAN, 4.0, 2.0]), Some(3.0));
    }

    #[test]
    fn percentile_counts_only_finite_peers_strictly_below_target() {
        assert_eq!(percentile_low(10.0, &[]), None);
        assert_eq!(percentile_low(10.0, &[f64::NAN]), None);
        let percentile =
            percentile_low(10.0, &[5.0, 10.0, 15.0, f64::NAN]).expect("finite peer percentile");
        assert!((percentile - 100.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn blocking_wrapper_without_runtime_returns_missing_instead_of_fake_data() {
        let client = reqwest::Client::new();
        assert!(fetch_blocking(&client, "TEST_CODE_000001").is_none());
    }

    #[test]
    fn industry_map_page_requires_complete_nonconflicting_rows() {
        let page = serde_json::json!({"data": {"diff": [
            {"f12": "BK0001", "f14": "测试行业"},
            {"f12": "BK0002", "f14": "第二行业"}
        ]}});
        let map = parse_industry_map_page(&page).expect("complete map page");
        assert_eq!(map.get("测试行业").map(String::as_str), Some("BK0001"));

        assert!(parse_industry_map_page(&serde_json::json!({})).is_err());
        assert!(
            parse_industry_map_page(&serde_json::json!({"data": {"diff": [
                {"f12": 1, "f14": "测试行业"}
            ]}}))
            .is_err()
        );
        assert!(
            parse_industry_map_page(&serde_json::json!({"data": {"diff": [
                {"f12": "BK0001", "f14": "测试行业"},
                {"f12": "BK9999", "f14": "测试行业"}
            ]}}))
            .is_err()
        );
    }

    #[test]
    fn industry_http_and_name_parsers_require_complete_protocol_facts() {
        let value = parse_industry_http_response(
            "TEST_CODE_host",
            200,
            Ok(r#"{"data":{"f127":"测试行业"}}"#.to_string()),
        )
        .expect("complete response");
        assert_eq!(parse_industry_name(&value).unwrap(), "测试行业");

        for result in [
            parse_industry_http_response("TEST_CODE_host", 503, Ok("{}".to_string())),
            parse_industry_http_response("TEST_CODE_host", 200, Err("断流".to_string())),
            parse_industry_http_response("TEST_CODE_host", 200, Ok(String::new())),
            parse_industry_http_response(
                "TEST_CODE_host",
                200,
                Ok("<html>限流</html>".to_string()),
            ),
        ] {
            assert!(result.is_err());
        }
        assert!(parse_industry_name(&serde_json::json!({})).is_err());
        assert!(parse_industry_name(&serde_json::json!({"data":{"f127":" "}})).is_err());
    }

    #[test]
    fn industry_map_pages_and_resolved_benchmark_are_atomic() {
        let mut map = HashMap::new();
        assert!(merge_industry_map_page(
            &mut map,
            &serde_json::json!({"data":{"diff":[
                {"f12":"BK0001","f14":"测试行业"}
            ]}}),
        )
        .unwrap());
        assert_eq!(map.get("测试行业").map(String::as_str), Some("BK0001"));
        assert!(merge_industry_map_page(
            &mut map,
            &serde_json::json!({"data":{"diff":[
                {"f12":"BK9999","f14":"测试行业"}
            ]}}),
        )
        .is_err());

        let mut full_page = Vec::new();
        for index in 0..100 {
            full_page.push(serde_json::json!({
                "f12": format!("BK{index:04}"),
                "f14": format!("TEST_CODE_行业{index:03}"),
            }));
        }
        let mut full_map = HashMap::new();
        assert!(!merge_industry_map_page(
            &mut full_map,
            &serde_json::json!({"data":{"diff":full_page}}),
        )
        .unwrap());
        assert!(
            merge_industry_map_page(&mut full_map, &serde_json::json!({"data":{"diff":[]}}),)
                .unwrap()
        );

        assert_eq!(
            resolve_industry_board_code(&map, "测试行业").unwrap(),
            "BK0001"
        );
        assert!(resolve_industry_board_code(&map, "不存在行业").is_err());
    }

    #[test]
    fn constituent_page_preserves_missing_values_and_rejects_bad_or_duplicate_rows() {
        let page = serde_json::json!({"data": {"diff": [
            {"f12": "TEST_CODE_000001", "f9": 10.0, "f23": "2.0", "f37": 12.0, "f129": -5.0},
            {"f12": "TEST_CODE_000002", "f9": null, "f23": "", "f37": 8.0, "f129": 10.0}
        ]}});
        let rows = parse_constituents_page(&page).expect("complete constituents");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].pe, Some(10.0));
        assert_eq!(rows[0].pb, Some(2.0));
        assert_eq!(rows[1].pe, None);
        assert_eq!(rows[1].pb, None);

        let malformed = serde_json::json!({"data": {"diff": [
            {"f12": "TEST_CODE_000001", "f9": "bad"}
        ]}});
        assert!(parse_constituents_page(&malformed).is_err());

        let duplicate = serde_json::json!({"data": {"diff": [
            {"f12": "TEST_CODE_000001"}, {"f12": "TEST_CODE_000001"}
        ]}});
        assert!(parse_constituents_page(&duplicate).is_err());
    }

    #[test]
    fn benchmark_uses_only_eligible_real_peer_values() {
        let rows = vec![
            IndustryConstituent {
                code: "TEST_CODE_000001".into(),
                pe: Some(10.0),
                pb: Some(2.0),
                roe: Some(12.0),
                growth: Some(20.0),
            },
            IndustryConstituent {
                code: "TEST_CODE_000002".into(),
                pe: Some(20.0),
                pb: Some(3.0),
                roe: Some(8.0),
                growth: Some(-10.0),
            },
            IndustryConstituent {
                code: "TEST_CODE_000003".into(),
                pe: Some(-5.0),
                pb: None,
                roe: Some(16.0),
                growth: None,
            },
        ];

        let benchmark = build_industry_benchmark(
            "测试行业".into(),
            "BK0001".into(),
            "TEST_CODE_000001",
            &rows,
        )
        .expect("valid benchmark");

        assert_eq!(benchmark.peer_count, 3);
        assert_eq!(benchmark.stock_pe, Some(10.0));
        assert_eq!(benchmark.median_pe, Some(15.0));
        assert_eq!(benchmark.median_pb, Some(2.5));
        assert_eq!(benchmark.median_roe, Some(12.0));
        assert_eq!(benchmark.median_growth, Some(5.0));
        assert_eq!(benchmark.pe_percentile, Some(0.0));
        assert!((benchmark.roe_percentile.expect("ROE percentile") - 100.0 / 3.0).abs() < 1e-12);

        assert!(build_industry_benchmark(
            "测试行业".into(),
            "BK0001".into(),
            "TEST_CODE_000001",
            &[]
        )
        .is_err());
    }

    #[tokio::test]
    async fn real_industry_hosts_fail_without_creating_a_benchmark() {
        let client = super::super::unreachable_http_client();
        assert!(fetch_async(&client, "TEST_CODE_000001").await.is_err());
        assert!(try_get_from_hosts(&client, "/api/qt/stock/get", HOSTS)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn loopback_transport_executes_name_map_and_constituent_acquisition() {
        let server = super::super::TestHttpServer::new(vec![
            super::super::TestHttpResponse::json(r#"{"data":{"f127":"TEST_CODE_测试行业"}}"#),
            super::super::TestHttpResponse::json(
                r#"{"data":{"diff":[{"f12":"BK0001","f14":"TEST_CODE_测试行业"}]}}"#,
            ),
            super::super::TestHttpResponse::json(
                r#"{"data":{"diff":[{"f12":"TEST_CODE_000001","f9":10.0,"f23":2.0,"f37":12.0,"f129":20.0},{"f12":"TEST_CODE_000002","f9":20.0,"f23":3.0,"f37":8.0,"f129":-10.0}]}}"#,
            ),
        ]);
        let hosts = [server.base_url()];
        let benchmark = fetch_async_from_hosts(
            &super::super::loopback_http_client(),
            "TEST_CODE_000001",
            &hosts,
        )
        .await
        .expect("complete industry transport");
        assert_eq!(benchmark.industry_name, "TEST_CODE_测试行业");
        assert_eq!(benchmark.board_code, "BK0001");
        assert_eq!(benchmark.peer_count, 2);
        assert_eq!(benchmark.stock_pe, Some(10.0));
        assert_eq!(benchmark.median_pe, Some(15.0));

        let requests = server.finish();
        assert_eq!(requests.len(), 3);
        assert!(requests[0].starts_with("/api/qt/stock/get?"));
        assert!(requests[1].contains("fs=m:90+t:2") || requests[1].contains("fs=m%3A90%2Bt%3A2"));
        assert!(requests[2].contains("fs=b:BK0001") || requests[2].contains("fs=b%3ABK0001"));
    }

    #[tokio::test]
    async fn industry_cache_commits_only_successful_complete_loader_results() {
        let slot = Mutex::new(None);
        let expected = HashMap::from([("TEST_CODE_测试行业".into(), "BK_TEST_1".into())]);
        let first = get_or_load_industry_map(&slot, || async { Ok(expected.clone()) })
            .await
            .expect("complete map must commit");
        assert_eq!(first, expected);
        let cached = get_or_load_industry_map(&slot, || async {
            Err(anyhow!("TEST_CODE cached loader must not run"))
        })
        .await
        .expect("cache hit must bypass loader");
        assert_eq!(cached, expected);

        let failed_slot = Mutex::new(None);
        assert!(get_or_load_industry_map(&failed_slot, || async {
            Err(anyhow!("TEST_CODE explicit source failure"))
        })
        .await
        .is_err());
        assert!(failed_slot.lock().expect("cache lock").is_none());
    }
}
