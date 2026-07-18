//! 多周期分时 K 线（60min / 15min）抓取。
//!
//! 数据源：东方财富 push2his K 线接口（与日线复用），仅 `klt` 不同：
//! - 60min: klt=60
//! - 15min: klt=15
//!
//! 返回的 `MinuteBar` 按时间**升序**排列（最新在末尾），便于直接喂给
//! `indicators::analyze_indicators(highs, lows, closes)`。

use anyhow::{anyhow, Result};

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

const PUSH2HIS_HOSTS: [&str; 3] = [
    "push2his.eastmoney.com",
    "push2his-bak.eastmoney.com",
    "82.push2his.eastmoney.com",
];

fn minute_transition_is_continuous(
    previous: chrono::NaiveDateTime,
    current: chrono::NaiveDateTime,
    klt: u8,
) -> bool {
    if !matches!(klt, 15 | 60) {
        return false;
    }
    let interval = chrono::Duration::minutes(i64::from(klt));
    if current.date() == previous.date() {
        if current - previous == interval {
            return true;
        }
        let morning_end = chrono::NaiveTime::from_hms_opt(11, 30, 0).unwrap();
        let afternoon_first = chrono::NaiveTime::from_hms_opt(13, 0, 0)
            .unwrap()
            .overflowing_add_signed(interval)
            .0;
        return previous.time() == morning_end && current.time() == afternoon_first;
    }
    let close = chrono::NaiveTime::from_hms_opt(15, 0, 0).unwrap();
    let first = chrono::NaiveTime::from_hms_opt(9, 30, 0)
        .unwrap()
        .overflowing_add_signed(interval)
        .0;
    previous.time() == close
        && current.time() == first
        && current.date() == crate::calendar::next_trading_day(previous.date())
}

fn parse_minute_rows(rows: &[serde_json::Value], klt: u8) -> Result<Vec<MinuteBar>> {
    let mut bars: Vec<MinuteBar> = Vec::with_capacity(rows.len());
    let mut previous_ts: Option<chrono::NaiveDateTime> = None;
    for (index, row) in rows.iter().enumerate() {
        let raw = row
            .as_str()
            .ok_or_else(|| anyhow!("分钟K线第 {} 行不是字符串", index + 1))?;
        let parts: Vec<&str> = raw.split(',').collect();
        if parts.len() < 6 {
            return Err(anyhow!(
                "分钟K线第 {} 行字段不足: expected>=6 actual={}",
                index + 1,
                parts.len()
            ));
        }
        let ts = chrono::NaiveDateTime::parse_from_str(parts[0], "%Y-%m-%d %H:%M")
            .map_err(|error| anyhow!("分钟K线第 {} 行时间非法: {error}", index + 1))?;
        if let Some(previous) = previous_ts {
            if ts <= previous || !minute_transition_is_continuous(previous, ts, klt) {
                return Err(anyhow!(
                    "分钟K线时间重复、倒序或缺口: {} -> {} (klt={klt})",
                    previous,
                    parts[0]
                ));
            }
        }
        previous_ts = Some(ts);
        let parse = |field: usize, name: &str| -> Result<f64> {
            let value = parts[field]
                .parse::<f64>()
                .map_err(|error| anyhow!("分钟K线第 {} 行 {name} 解析失败: {error}", index + 1))?;
            if !value.is_finite() {
                return Err(anyhow!("分钟K线第 {} 行 {name} 非有限", index + 1));
            }
            Ok(value)
        };
        let open = parse(1, "open")?;
        let close = parse(2, "close")?;
        let high = parse(3, "high")?;
        let low = parse(4, "low")?;
        let volume = parse(5, "volume")?;
        if open <= 0.0
            || close <= 0.0
            || high <= 0.0
            || low <= 0.0
            || volume < 0.0
            || high < open.max(close)
            || low > open.min(close)
            || high < low
        {
            return Err(anyhow!("分钟K线第 {} 行 OHLCV 非法", index + 1));
        }
        if let Some(previous) = bars.last() {
            let change = (close / previous.close - 1.0).abs();
            if change > 0.20 {
                return Err(anyhow!(
                    "分钟K线相邻收盘变化超过 ±20%: {} -> {}",
                    previous.close,
                    close
                ));
            }
        }
        bars.push(MinuteBar {
            timestamp: parts[0].to_string(),
            open,
            close,
            high,
            low,
            volume,
        });
    }
    if bars.is_empty() {
        return Err(anyhow!("分钟K线批次为空"));
    }
    Ok(bars)
}

/// 抓取分钟 K 线（异步内部实现）。
pub(crate) async fn fetch_async(
    client: &reqwest::Client,
    code: &str,
    klt: u8,
    lmt: usize,
) -> Result<Vec<MinuteBar>> {
    let secid = to_secid(code);
    let mut last_err = String::new();

    for host in PUSH2HIS_HOSTS {
        let url = format!(
            "https://{}/api/qt/stock/kline/get?secid={}&\
             fields1=f1,f2,f3,f4,f5,f6&fields2=f51,f52,f53,f54,f55,f56,f57,f58&\
             klt={}&fqt=1&end=20500101&lmt={}",
            host, secid, klt, lmt
        );
        log::debug!("[分钟K线 klt={} host={}] {}", klt, host, url);

        let resp = match client
            .get(&url)
            .header("Referer", "https://quote.eastmoney.com/")
            .header("Accept", "application/json, text/plain, */*")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                last_err = format!("{}: {}", host, e);
                continue;
            }
        };
        if !resp.status().is_success() {
            last_err = format!("{}: 状态码 {}", host, resp.status());
            continue;
        }
        let text = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                last_err = format!("{}: 读取失败 {}", host, e);
                continue;
            }
        };
        let body = text.trim_start();
        if body.starts_with('<') {
            last_err = format!("{}: 非JSON回包（网关拦截）", host);
            continue;
        }
        let json: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                last_err = format!("{}: JSON解析失败 {}", host, e);
                continue;
            }
        };

        let Some(klines) = json
            .get("data")
            .and_then(|d| d.get("klines"))
            .and_then(|v| v.as_array())
        else {
            last_err = format!("{}: 分钟K线无 klines 数组", host);
            continue;
        };

        match parse_minute_rows(klines, klt) {
            Ok(bars) => return Ok(bars),
            Err(error) => {
                last_err = format!("{host}: 分钟K线批次校验失败: {error}");
            }
        }
    }
    Err(anyhow!("分钟K线全部主机失败: {}", last_err))
}

/// 同步阻塞包装（在 tokio runtime 上下文调用）。
pub fn fetch_blocking(
    client: &reqwest::Client,
    code: &str,
    klt: u8,
    lmt: usize,
) -> Result<Vec<MinuteBar>> {
    // 修复 Top10#5 (2026-06-29 audit): 用 crate::block_on_async 统一替代
    if tokio::runtime::Handle::try_current().is_err() {
        return Err(anyhow!("[分钟K线] 无 tokio runtime，无法抓取 {code}"));
    }
    let client = client.clone();
    let code_s = code.to_string();
    crate::block_on_async(async move { fetch_async(&client, &code_s, klt, lmt).await })
}

#[cfg(test)]
mod br115_tests {
    use super::*;

    #[test]
    fn malformed_minute_row_rejects_entire_batch() {
        let rows = vec![
            serde_json::Value::String("2026-07-18 09:30,10,10.1,10.2,9.9,100,1000,3".to_string()),
            serde_json::Value::String("broken".to_string()),
        ];
        assert!(parse_minute_rows(&rows, 15).is_err());
    }

    #[test]
    fn minute_batch_rejects_time_gap_and_adjacent_price_jump() {
        let gap = vec![
            serde_json::Value::String("2026-07-17 09:45,10,10,10.1,9.9,100".to_string()),
            serde_json::Value::String("2026-07-17 10:15,10,10.1,10.2,9.9,100".to_string()),
        ];
        assert!(parse_minute_rows(&gap, 15).is_err());

        let jump = vec![
            serde_json::Value::String("2026-07-17 09:45,10,10,10.1,9.9,100".to_string()),
            serde_json::Value::String("2026-07-17 10:00,13,13,13.1,12.9,100".to_string()),
        ];
        assert!(parse_minute_rows(&jump, 15).is_err());
    }
}
