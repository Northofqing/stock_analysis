//! Yahoo Finance 实时行情（免费，限流宽松）。
//!
//! 用作东财 push2 API 限流时的备选数据源。
//! A 股代码映射：深交所 → code.SZ，上交所(6开头) → code.SS

use serde::Deserialize;
use serde_json::Value;

/// 雅虎返回的实时行情
#[derive(Debug, Clone)]
pub struct YahooQuote {
    pub code: String,
    pub price: Option<f64>,
    pub change_pct: Option<f64>,
    pub volume: Option<f64>,
    pub previous_close: Option<f64>,
}

/// Yahoo Finance API 响应结构
#[derive(Debug, Deserialize)]
struct QuoteResponse {
    #[serde(rename = "quoteResponse")]
    quote_response: QuoteResult,
}

#[derive(Debug, Deserialize)]
struct QuoteResult {
    result: Vec<RawQuote>,
}

#[derive(Debug, Deserialize)]
struct RawQuote {
    symbol: Option<String>,
    #[serde(rename = "regularMarketPrice")]
    regular_market_price: Option<f64>,
    #[serde(rename = "regularMarketChangePercent")]
    regular_market_change_percent: Option<f64>,
    #[serde(rename = "regularMarketVolume")]
    regular_market_volume: Option<f64>,
    #[serde(rename = "regularMarketPreviousClose")]
    regular_market_previous_close: Option<f64>,
}

/// 将 A 股代码转为雅虎符号；海外指数、外汇等 Yahoo 原生符号保持不变。
fn to_yahoo_symbol(code: &str) -> String {
    if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return code.to_string();
    }
    match code.as_bytes()[0] {
        b'5' | b'6' => format!("{code}.SS"),
        b'4' | b'8' | b'9' => format!("{code}.BJ"),
        _ => format!("{code}.SZ"),
    }
}

fn validate_optional_field(
    symbol: &str,
    field: &str,
    value: Option<f64>,
    predicate: impl FnOnce(f64) -> bool,
) -> Result<Option<f64>, String> {
    match value {
        Some(value) if value.is_finite() && predicate(value) => Ok(Some(value)),
        Some(value) => Err(format!("Yahoo {symbol} invalid {field}: {value}")),
        None => {
            log::warn!("[雅虎] {} 缺少字段 {}", symbol, field);
            Ok(None)
        }
    }
}

fn parse_quotes(
    response: QuoteResponse,
    symbol_map: &std::collections::HashMap<String, String>,
) -> Result<Vec<YahooQuote>, String> {
    let mut quotes = Vec::with_capacity(response.quote_response.result.len());
    for raw in response.quote_response.result {
        let symbol = raw
            .symbol
            .as_deref()
            .ok_or_else(|| "Yahoo response row missing symbol".to_string())?;
        let code = symbol_map
            .get(symbol)
            .cloned()
            .ok_or_else(|| format!("Yahoo returned unrequested symbol: {symbol}"))?;
        quotes.push(YahooQuote {
            code,
            price: validate_optional_field(
                symbol,
                "regularMarketPrice",
                raw.regular_market_price,
                |value| value > 0.0,
            )?,
            change_pct: validate_optional_field(
                symbol,
                "regularMarketChangePercent",
                raw.regular_market_change_percent,
                |value| value.abs() <= 20.0,
            )?,
            volume: validate_optional_field(
                symbol,
                "regularMarketVolume",
                raw.regular_market_volume,
                |value| value >= 0.0,
            )?,
            previous_close: validate_optional_field(
                symbol,
                "regularMarketPreviousClose",
                raw.regular_market_previous_close,
                |value| value > 0.0,
            )?,
        });
    }
    Ok(quotes)
}

/// 批量拉取实时行情（一次请求最多约 20 只）
pub fn fetch_quotes(codes: &[String]) -> Result<Vec<YahooQuote>, String> {
    if codes.is_empty() {
        return Ok(vec![]);
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|error| format!("build Yahoo client: {error}"))?;
    fetch_quotes_with_client(&client, codes)
}

fn fetch_quotes_with_client(
    client: &reqwest::blocking::Client,
    codes: &[String],
) -> Result<Vec<YahooQuote>, String> {
    fetch_quotes_from_base(client, codes, "https://query1.finance.yahoo.com")
}

fn fetch_quotes_from_base(
    client: &reqwest::blocking::Client,
    codes: &[String],
    base: &str,
) -> Result<Vec<YahooQuote>, String> {
    if codes.is_empty() {
        return Ok(vec![]);
    }

    // 构建符号列表和反向映射
    let mut symbol_map: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let symbols: Vec<String> = codes
        .iter()
        .map(|c| {
            let sym = to_yahoo_symbol(c);
            symbol_map.insert(sym.clone(), c.clone());
            sym
        })
        .collect();

    let url = format!(
        "{}/v7/finance/quote?symbols={}",
        base.trim_end_matches('/'),
        symbols.join(",")
    );

    let response = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .map_err(|error| format!("Yahoo request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Yahoo HTTP status failed: {error}"))?;
    let response: QuoteResponse = response
        .json()
        .map_err(|error| format!("decode Yahoo response: {error}"))?;
    let quotes = parse_quotes(response, &symbol_map)?;
    for (symbol, code) in &symbol_map {
        if !quotes.iter().any(|quote| quote.code == *code) {
            log::warn!("[雅虎] 响应缺少请求标的 {} ({})", code, symbol);
        }
    }
    Ok(quotes)
}

/// v64: 隔夜关注数据 (美股三大指数 + USD/CNY 汇率) — 雅虎财经 API
/// - 代码: "^IXIC" (纳斯达克), "^DJI" (道琼斯), "^GSPC" (标普 500),
///   "CNY=X" (USD/CNY 汇率)
/// - 返回: (us_summary_str, fx_str) — 给 R-08 明日事件日历使用
pub fn fetch_overnight_data() -> Result<(String, String), String> {
    // 美股 3 指数 + 美元/人民币汇率；这些是 Yahoo 原生符号，不能添加 A 股后缀。
    let codes = ["^IXIC", "^DJI", "^GSPC", "CNY=X"].map(str::to_string);
    let quotes = match fetch_quotes(&codes) {
        Ok(quotes) => quotes,
        Err(quote_error) => {
            log::warn!("Yahoo quote endpoint unavailable; trying chart endpoint: {quote_error}");
            fetch_chart_quotes(&codes).map_err(|chart_error| {
                format!("quote endpoint: {quote_error}; chart endpoint: {chart_error}")
            })?
        }
    };
    format_overnight_data(&quotes)
}

/// Yahoo's chart endpoint is a separate, real-data API and often remains
/// available when `/v7/finance/quote` returns 401. It is not a synthetic
/// fallback: missing or invalid chart fields still fail the overnight task.
fn fetch_chart_quotes(codes: &[String]) -> Result<Vec<YahooQuote>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|error| format!("build Yahoo chart client: {error}"))?;
    let mut quotes = Vec::with_capacity(codes.len());
    for symbol in codes {
        let url = format!(
            "https://query1.finance.yahoo.com/v8/finance/chart/{}?range=1d&interval=1d",
            urlencoding::encode(symbol)
        );
        let body: Value = client
            .get(url)
            .header("User-Agent", "Mozilla/5.0")
            .send()
            .map_err(|error| format!("chart request {symbol}: {error}"))?
            .error_for_status()
            .map_err(|error| format!("chart HTTP {symbol}: {error}"))?
            .json()
            .map_err(|error| format!("chart decode {symbol}: {error}"))?;
        let meta = body
            .pointer("/chart/result/0/meta")
            .ok_or_else(|| format!("chart response missing meta for {symbol}"))?;
        let price = meta
            .get("regularMarketPrice")
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or_else(|| format!("chart missing valid price for {symbol}"))?;
        let previous = meta
            .get("previousClose")
            .or_else(|| meta.get("chartPreviousClose"))
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or_else(|| format!("chart missing valid previous close for {symbol}"))?;
        let change_pct = (price - previous) / previous * 100.0;
        if !change_pct.is_finite() || change_pct.abs() > 20.0 {
            return Err(format!(
                "chart invalid change_pct for {symbol}: {change_pct}"
            ));
        }
        quotes.push(YahooQuote {
            code: symbol.clone(),
            price: Some(price),
            change_pct: Some(change_pct),
            volume: None,
            previous_close: Some(previous),
        });
    }
    Ok(quotes)
}

fn format_overnight_data(quotes: &[YahooQuote]) -> Result<(String, String), String> {
    // 索引 by code
    let mut by_code = std::collections::HashMap::new();
    for q in quotes {
        if let Some(change_pct) = q.change_pct {
            by_code.insert(q.code.clone(), change_pct);
        }
    }

    // 格式化美股摘要: "美股 +0.8% (纳+1.2% 道+0.3% 标+0.5%)"
    // 优先级: 纳斯达克 > 道琼斯 > 标普
    let required_change = |symbol: &str| {
        by_code
            .get(symbol)
            .copied()
            .ok_or_else(|| format!("Yahoo overnight data missing change_pct for {symbol}"))
    };
    let nasdaq_pct = required_change("^IXIC")?;
    let dow_pct = required_change("^DJI")?;
    let sp500_pct = required_change("^GSPC")?;

    // 选用变化最大的指数代表 (避免"美股 +0%"误导)
    let main_pct = [nasdaq_pct, dow_pct, sp500_pct]
        .iter()
        .copied()
        .filter(|x| x.abs() > 0.01)
        .fold(0.0_f64, |largest, candidate| {
            if candidate.abs() > largest.abs() {
                candidate
            } else {
                largest
            }
        });

    let us_summary = if main_pct.abs() < 0.01 {
        "持平".to_string()
    } else {
        format!(
            "{}{:.1}% (纳{:+.1}% 道{:+.1}% 标{:+.1}%)",
            if main_pct > 0.0 { "+" } else { "" },
            main_pct,
            nasdaq_pct,
            dow_pct,
            sp500_pct
        )
    };

    // 汇率: 美元/人民币
    let usd_cny = required_change("CNY=X")?;
    // 变化: 涨 = 人民币贬值 (利空 A股), 跌 = 人民币升值 (利好 A股)
    let fx_summary = if usd_cny.abs() < 0.0001 {
        "持平".to_string()
    } else {
        // yahoo CNY=X 是 1 美元 = ? 人民币 (e.g. 7.18)
        // 我们需要 price (汇率值), 但 fetch_quotes 只返 change_pct
        // 简化: 只显示涨跌幅方向, 真实汇率值需另查 (后续 PR)
        format!("{:+.2}% (USD/CNY)", usd_cny)
    };

    Ok((us_summary, fx_summary))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_yahoo_symbol_sz() {
        assert_eq!(to_yahoo_symbol("000547"), "000547.SZ");
        assert_eq!(to_yahoo_symbol("002421"), "002421.SZ");
    }

    #[test]
    fn test_to_yahoo_symbol_ss() {
        assert_eq!(to_yahoo_symbol("603618"), "603618.SS");
        assert_eq!(to_yahoo_symbol("600519"), "600519.SS");
    }

    #[test]
    fn test_to_yahoo_symbol_preserves_native_symbols_and_maps_bse() {
        assert_eq!(to_yahoo_symbol("^IXIC"), "^IXIC");
        assert_eq!(to_yahoo_symbol("CNY=X"), "CNY=X");
        assert_eq!(to_yahoo_symbol("920001"), "920001.BJ");
    }

    #[test]
    fn test_fetch_empty() {
        assert!(fetch_quotes(&[]).expect("empty request").is_empty());
        let client = reqwest::blocking::Client::new();
        assert!(fetch_quotes_with_client(&client, &[])
            .expect("empty injected request")
            .is_empty());
    }

    #[test]
    fn test_parse_quotes_preserves_missing_fields() {
        let response: QuoteResponse = serde_json::from_str(
            r#"{"quoteResponse":{"result":[{"symbol":"000547.SZ","regularMarketPrice":10.5}]}}"#,
        )
        .expect("fixture");
        let symbol_map =
            std::collections::HashMap::from([("000547.SZ".to_string(), "000547".to_string())]);
        let quotes = parse_quotes(response, &symbol_map).expect("parse");
        assert_eq!(quotes[0].price, Some(10.5));
        assert_eq!(quotes[0].change_pct, None);
        assert_eq!(quotes[0].volume, None);
        assert_eq!(quotes[0].previous_close, None);
    }

    #[test]
    fn test_parse_quotes_rejects_invalid_required_domain_value() {
        let response: QuoteResponse = serde_json::from_str(
            r#"{"quoteResponse":{"result":[{"symbol":"000547.SZ","regularMarketPrice":0.0}]}}"#,
        )
        .expect("fixture");
        let symbol_map =
            std::collections::HashMap::from([("000547.SZ".to_string(), "000547".to_string())]);
        let error = parse_quotes(response, &symbol_map).expect_err("zero price must fail");
        assert!(error.contains("regularMarketPrice"));
    }

    fn overnight_quote(code: &str, change_pct: Option<f64>) -> YahooQuote {
        YahooQuote {
            code: code.to_string(),
            price: None,
            change_pct,
            volume: None,
            previous_close: None,
        }
    }

    #[test]
    fn test_overnight_data_requires_every_change_field() {
        let quotes = vec![
            overnight_quote("^IXIC", Some(1.2)),
            overnight_quote("^DJI", Some(0.3)),
            overnight_quote("^GSPC", None),
            overnight_quote("CNY=X", Some(-0.2)),
        ];
        let error = format_overnight_data(&quotes).expect_err("missing S&P change must fail");
        assert!(error.contains("^GSPC"));
    }

    #[test]
    fn test_overnight_data_distinguishes_real_flat_from_missing() {
        let quotes = vec![
            overnight_quote("^IXIC", Some(0.0)),
            overnight_quote("^DJI", Some(0.0)),
            overnight_quote("^GSPC", Some(0.0)),
            overnight_quote("CNY=X", Some(0.0)),
        ];
        assert_eq!(
            format_overnight_data(&quotes).expect("complete flat snapshot"),
            ("持平".to_string(), "持平".to_string())
        );
    }

    #[test]
    fn yahoo_transport_failure_is_not_an_empty_quote_batch() {
        let client = reqwest::blocking::Client::builder()
            .proxy(reqwest::Proxy::all("http://127.0.0.1:9").unwrap())
            .connect_timeout(std::time::Duration::from_millis(25))
            .timeout(std::time::Duration::from_millis(100))
            .build()
            .unwrap();
        let error = fetch_quotes_with_client(&client, &["TEST_CODE_000001".to_string()])
            .expect_err("unreachable Yahoo transport must fail");
        assert!(error.contains("request failed"));
    }

    #[test]
    fn loopback_blocking_transport_preserves_complete_and_missing_quote_rows() {
        use super::super::{TestHttpResponse, TestHttpServer};

        let body = serde_json::json!({"quoteResponse": {"result": [{
            "symbol": "TEST_CODE_000001",
            "regularMarketPrice": 10.1,
            "regularMarketChangePercent": 1.0,
            "regularMarketVolume": 1234.0,
            "regularMarketPreviousClose": 10.0
        }]}})
        .to_string();
        let server = TestHttpServer::new(vec![TestHttpResponse::json(body)]);
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .build()
            .unwrap();
        let quotes = fetch_quotes_from_base(
            &client,
            &[
                "TEST_CODE_000001".to_string(),
                "TEST_CODE_MISSING".to_string(),
            ],
            server.base_url(),
        )
        .unwrap();
        assert_eq!(quotes.len(), 1);
        assert_eq!(quotes[0].code, "TEST_CODE_000001");
        assert_eq!(quotes[0].price, Some(10.1));
        let requests = server.finish();
        assert!(requests[0].contains("symbols=TEST_CODE_000001,TEST_CODE_MISSING"));

        let server = TestHttpServer::new(vec![TestHttpResponse {
            status: 503,
            body: "unavailable".to_string(),
        }]);
        let error = fetch_quotes_from_base(
            &client,
            &["TEST_CODE_000001".to_string()],
            server.base_url(),
        )
        .unwrap_err();
        assert!(error.contains("HTTP status"));
        assert_eq!(server.finish().len(), 1);
    }
}

#[cfg(test)]
#[path = "../gate_d_yahoo_regression.rs"]
mod gate_d_regression;
