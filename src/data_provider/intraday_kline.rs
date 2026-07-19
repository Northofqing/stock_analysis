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
    #[cfg(test)]
    let code = code.strip_prefix("TEST_CODE_").unwrap_or(code);
    let market = if code.starts_with('6') || code.starts_with("688") || code.starts_with("900") {
        "1"
    } else {
        "0"
    };
    format!("{}.{}", market, code)
}

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

fn parse_minute_response(text: &str, klt: u8) -> Result<Vec<MinuteBar>> {
    if text.trim_start().starts_with('<') {
        return Err(anyhow!("非JSON回包（网关拦截）"));
    }
    let json: serde_json::Value =
        serde_json::from_str(text).map_err(|error| anyhow!("JSON解析失败: {error}"))?;
    let klines = json
        .get("data")
        .and_then(|data| data.get("klines"))
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("分钟K线无 klines 数组"))?;
    parse_minute_rows(klines, klt).map_err(|error| anyhow!("分钟K线批次校验失败: {error}"))
}

/// 抓取分钟 K 线（异步内部实现）。
pub(crate) async fn fetch_async(
    client: &reqwest::Client,
    code: &str,
    klt: u8,
    lmt: usize,
) -> Result<Vec<MinuteBar>> {
    const BASES: [&str; 3] = [
        "https://push2his.eastmoney.com",
        "https://push2his-bak.eastmoney.com",
        "https://82.push2his.eastmoney.com",
    ];
    fetch_from_bases(client, code, klt, lmt, &BASES).await
}

async fn fetch_from_bases(
    client: &reqwest::Client,
    code: &str,
    klt: u8,
    lmt: usize,
    bases: &[&str],
) -> Result<Vec<MinuteBar>> {
    let secid = to_secid(code);
    let mut last_err = String::new();

    for base in bases {
        let base = base.trim_end_matches('/');
        let url = format!(
            "{}/api/qt/stock/kline/get?secid={}&\
             fields1=f1,f2,f3,f4,f5,f6&fields2=f51,f52,f53,f54,f55,f56,f57,f58&\
             klt={}&fqt=1&end=20500101&lmt={}",
            base, secid, klt, lmt
        );
        log::debug!("[分钟K线 klt={} host={}] {}", klt, base, url);

        let resp = match client
            .get(&url)
            .header("Referer", "https://quote.eastmoney.com/")
            .header("Accept", "application/json, text/plain, */*")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                last_err = format!("{}: {}", base, e);
                continue;
            }
        };
        if !resp.status().is_success() {
            last_err = format!("{}: 状态码 {}", base, resp.status());
            continue;
        }
        let text = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                last_err = format!("{}: 读取失败 {}", base, e);
                continue;
            }
        };
        match parse_minute_response(&text, klt) {
            Ok(bars) => return Ok(bars),
            Err(error) => {
                last_err = format!("{base}: {error}");
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

    fn row(timestamp: &str, values: &str) -> serde_json::Value {
        serde_json::Value::String(format!("{timestamp},{values}"))
    }

    #[test]
    fn secid_and_session_transitions_are_deterministic() {
        assert_eq!(to_secid("TEST_CODE_600000"), "1.600000");
        assert_eq!(to_secid("TEST_CODE_000001"), "0.000001");

        let parse =
            |value: &str| chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M").unwrap();
        assert!(minute_transition_is_continuous(
            parse("2026-07-17 10:00"),
            parse("2026-07-17 10:15"),
            15,
        ));
        assert!(minute_transition_is_continuous(
            parse("2026-07-17 11:30"),
            parse("2026-07-17 13:15"),
            15,
        ));
        assert!(minute_transition_is_continuous(
            parse("2026-07-17 15:00"),
            parse("2026-07-20 09:45"),
            15,
        ));
        assert!(!minute_transition_is_continuous(
            parse("2026-07-17 10:00"),
            parse("2026-07-17 10:15"),
            5,
        ));
    }

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

    #[test]
    fn minute_rows_accept_valid_batch_and_reject_every_invalid_field_class() {
        let valid = vec![
            row("2026-07-17 09:45", "10,10.1,10.2,9.9,100"),
            row("2026-07-17 10:00", "10.1,10.2,10.3,10,120"),
        ];
        let bars = parse_minute_rows(&valid, 15).unwrap();
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[1].timestamp, "2026-07-17 10:00");
        assert_eq!(bars[1].open, 10.1);
        assert_eq!(bars[1].high, 10.3);
        assert_eq!(bars[1].low, 10.0);
        assert_eq!(bars[1].volume, 120.0);

        let invalid_batches = [
            vec![serde_json::json!({"not": "a row"})],
            vec![row("invalid-time", "10,10,10,10,1")],
            vec![row("2026-07-17 09:45", "bad,10,10,10,1")],
            vec![row("2026-07-17 09:45", "NaN,10,10,10,1")],
            vec![row("2026-07-17 09:45", "0,10,10,10,1")],
            vec![row("2026-07-17 09:45", "10,10,9,10,1")],
            vec![row("2026-07-17 09:45", "10,10,10,11,1")],
            vec![row("2026-07-17 09:45", "10,10,10,10,-1")],
        ];
        for batch in invalid_batches {
            assert!(parse_minute_rows(&batch, 15).is_err());
        }
        assert!(parse_minute_rows(&[], 15).is_err());
    }

    #[test]
    fn minute_response_requires_json_klines_and_validated_rows() {
        let valid = r#"{"data":{"klines":["2026-07-17 09:45,10,10.1,10.2,9.9,100"]}}"#;
        assert_eq!(parse_minute_response(valid, 15).unwrap().len(), 1);

        for invalid in [
            " <html>blocked</html>",
            "not-json",
            r#"{"data":null}"#,
            r#"{"data":{"klines":[]}}"#,
        ] {
            assert!(parse_minute_response(invalid, 15).is_err());
        }
    }

    #[test]
    fn blocking_fetch_requires_an_existing_runtime() {
        let client = reqwest::Client::new();
        let error = fetch_blocking(&client, "TEST_CODE_600000", 15, 1).unwrap_err();
        assert!(error.to_string().contains("无 tokio runtime"));
    }

    #[tokio::test]
    async fn loopback_minute_transport_retries_status_then_parses_complete_batch() {
        use super::super::{loopback_http_client, TestHttpResponse, TestHttpServer};

        let body = serde_json::json!({"data": {"klines": [
            "2026-07-17 09:45,10,10.1,10.2,9.9,100",
            "2026-07-17 10:00,10.1,10.2,10.3,10,120"
        ]}})
        .to_string();
        let server = TestHttpServer::new(vec![
            TestHttpResponse {
                status: 503,
                body: "unavailable".to_string(),
            },
            TestHttpResponse::json(body),
        ]);
        let base = server.base_url().to_string();
        let bars = fetch_from_bases(
            &loopback_http_client(),
            "TEST_CODE_600000",
            15,
            2,
            &[&base, &base],
        )
        .await
        .unwrap();
        assert_eq!(bars.len(), 2);
        let requests = server.finish();
        assert!(requests[0].contains("secid=1.600000"));
        assert!(requests[0].contains("klt=15"));
        assert!(requests[0].contains("lmt=2"));
    }
}
