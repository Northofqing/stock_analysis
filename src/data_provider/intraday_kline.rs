//! 多周期分时 K 线（60min / 15min）抓取。
//!
//! 数据源：东方财富 push2his K 线接口（与日线复用），仅 `klt` 不同：
//! - 60min: klt=60
//! - 15min: klt=15
//!
//! 返回的 `MinuteBar` 按时间**升序**排列（最新在末尾），便于直接喂给
//! `indicators::analyze_indicators(highs, lows, closes)`。

use anyhow::{anyhow, Context, Result};

/// 分钟级 K 线（仅保留多周期分析需要的字段）
#[derive(Debug, Clone)]
pub struct MinuteBar {
    pub timestamp: String, // 形如 "2026-04-30 14:00"
    pub open: f64,
    pub close: f64,
    pub high: f64,
    pub low: f64,
    pub volume: f64,
}

fn to_secid(code: &str) -> String {
    let market = if code.starts_with('6') || code.starts_with("688") || code.starts_with("900") {
        "1"
    } else {
        "0"
    };
    format!("{}.{}", market, code)
}

/// 抓取分钟 K 线（异步内部实现）。
async fn fetch_async(
    client: &reqwest::Client,
    code: &str,
    klt: u8,
    lmt: usize,
) -> Result<Vec<MinuteBar>> {
    let secid = to_secid(code);
    let url = format!(
        "https://push2his.eastmoney.com/api/qt/stock/kline/get?secid={}&\
         fields1=f1,f2,f3,f4,f5,f6&fields2=f51,f52,f53,f54,f55,f56,f57,f58&\
         klt={}&fqt=1&end=20500101&lmt={}",
        secid, klt, lmt
    );
    log::debug!("[分钟K线 klt={}] {}", klt, url);

    let resp = client
        .get(&url)
        .header("Referer", "https://quote.eastmoney.com/")
        .header("Accept", "application/json, text/plain, */*")
        .send()
        .await
        .context("分钟K线请求失败")?;
    if !resp.status().is_success() {
        return Err(anyhow!("分钟K线状态码 {}", resp.status()));
    }
    let text = resp.text().await.context("分钟K线读取失败")?;
    let json: serde_json::Value =
        serde_json::from_str(&text).context("分钟K线 JSON 解析失败")?;

    let klines = json
        .get("data")
        .and_then(|d| d.get("klines"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("分钟K线无 klines 数组"))?;

    let mut bars = Vec::with_capacity(klines.len());
    for k in klines {
        let s = match k.as_str() {
            Some(s) => s,
            None => continue,
        };
        // 格式: "2026-04-30 14:00,open,close,high,low,volume,amount,amplitude"
        let parts: Vec<&str> = s.split(',').collect();
        if parts.len() < 6 {
            continue;
        }
        let parse = |i: usize| parts.get(i).and_then(|p| p.parse::<f64>().ok());
        let (Some(open), Some(close), Some(high), Some(low), Some(volume)) =
            (parse(1), parse(2), parse(3), parse(4), parse(5))
        else {
            continue;
        };
        bars.push(MinuteBar {
            timestamp: parts[0].to_string(),
            open,
            close,
            high,
            low,
            volume,
        });
    }
    // EM 返回已是升序，但稳妥起见显式断言：若发现倒序则 reverse。
    if bars.len() >= 2 && bars.first().unwrap().timestamp > bars.last().unwrap().timestamp {
        bars.reverse();
    }
    Ok(bars)
}

/// 同步阻塞包装（在 tokio runtime 上下文调用）。
pub fn fetch_blocking(
    client: &reqwest::Client,
    code: &str,
    klt: u8,
    lmt: usize,
) -> Vec<MinuteBar> {
    let handle = match tokio::runtime::Handle::try_current() {
        Ok(h) => h,
        Err(_) => return Vec::new(),
    };
    let client = client.clone();
    let code = code.to_string();
    tokio::task::block_in_place(|| {
        handle.block_on(async move {
            match fetch_async(&client, &code, klt, lmt).await {
                Ok(b) => {
                    log::info!("[分钟K线] {} klt={} 取得 {} 根", code, klt, b.len());
                    b
                }
                Err(e) => {
                    log::warn!("[分钟K线] {} klt={} 抓取失败: {}", code, klt, e);
                    Vec::new()
                }
            }
        })
    })
}
