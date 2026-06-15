//! Yahoo Finance 实时行情（免费，限流宽松）。
//!
//! 用作东财 push2 API 限流时的备选数据源。
//! A 股代码映射：深交所 → code.SZ，上交所(6开头) → code.SS

use serde::Deserialize;

/// 雅虎返回的实时行情
#[derive(Debug, Clone)]
pub struct YahooQuote {
    pub code: String,
    pub price: f64,
    pub change_pct: f64,
    pub volume: f64,
    pub previous_close: f64,
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

/// 将 A 股代码转为雅虎符号（深交所.SZ，上交所.SS）
fn to_yahoo_symbol(code: &str) -> String {
    if code.starts_with('6') || code.starts_with('5') {
        format!("{}.SS", code)
    } else {
        format!("{}.SZ", code)
    }
}

/// 批量拉取实时行情（一次请求最多约 20 只）
pub fn fetch_quotes(codes: &[String]) -> Vec<YahooQuote> {
    if codes.is_empty() { return vec![]; }

    // 构建符号列表和反向映射
    let mut symbol_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let symbols: Vec<String> = codes.iter().map(|c| {
        let sym = to_yahoo_symbol(c);
        symbol_map.insert(sym.clone(), c.clone());
        sym
    }).collect();

    let url = format!(
        "https://query1.finance.yahoo.com/v7/finance/quote?symbols={}",
        symbols.join(",")
    );

    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
    {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let resp: QuoteResponse = match client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .and_then(|r| r.json())
    {
        Ok(r) => r,
        Err(e) => {
            log::warn!("[雅虎] API 请求失败: {}", e);
            return vec![];
        }
    };

    resp.quote_response.result.into_iter().filter_map(|r| {
        let symbol = r.symbol.as_deref()?;
        let code = symbol_map.get(symbol)?.clone();
        Some(YahooQuote {
            code,
            price: r.regular_market_price.unwrap_or(0.0),
            change_pct: r.regular_market_change_percent.unwrap_or(0.0),
            volume: r.regular_market_volume.unwrap_or(0.0),
            previous_close: r.regular_market_previous_close.unwrap_or(0.0),
        })
    }).collect()
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
    fn test_fetch_empty() {
        assert!(fetch_quotes(&[]).is_empty());
    }
}
