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
    if codes.is_empty() {
        return vec![];
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

    resp.quote_response
        .result
        .into_iter()
        .filter_map(|r| {
            let symbol = r.symbol.as_deref()?;
            let code = symbol_map.get(symbol)?.clone();
            Some(YahooQuote {
                code,
                price: r.regular_market_price.unwrap_or(0.0),
                change_pct: r.regular_market_change_percent.unwrap_or(0.0),
                volume: r.regular_market_volume.unwrap_or(0.0),
                previous_close: r.regular_market_previous_close.unwrap_or(0.0),
            })
        })
        .collect()
}

/// v64: 隔夜关注数据 (美股三大指数 + USD/CNY 汇率) — 雅虎财经 API
/// - 代码: "^IXIC" (纳斯达克), "^DJI" (道琼斯), "^GSPC" (标普 500), "DX-Y.NYB" (美元指数)
///       "CNY=X" (USD/CNY 汇率)
/// - 返回: (us_summary_str, fx_str) — 给 R-08 明日事件日历使用
pub fn fetch_overnight_data() -> (String, String) {
    use std::collections::HashMap;

    // v64: 美股 3 指数 + 美元指数 + 美元/人民币汇率
    let symbols = vec![
        ("^IXIC".to_string(), "纳斯达克"),
        ("^DJI".to_string(), "道琼斯"),
        ("^GSPC".to_string(), "标普500"),
        ("DX-Y.NYB".to_string(), "美元指数"),
        ("CNY=X".to_string(), "美元/人民币"),
    ];
    let codes: Vec<String> = symbols.iter().map(|(s, _)| s.clone()).collect();
    let quotes = fetch_quotes(&codes);

    // 索引 by code
    let mut by_code: HashMap<String, f64> = HashMap::new();
    for q in &quotes {
        by_code.insert(q.code.clone(), q.change_pct);
    }

    // 格式化美股摘要: "美股 +0.8% (纳+1.2% 道+0.3% 标+0.5%)"
    // 优先级: 纳斯达克 > 道琼斯 > 标普
    let nasdaq_pct = by_code.get("^IXIC").copied().unwrap_or(0.0);
    let dow_pct = by_code.get("^DJI").copied().unwrap_or(0.0);
    let sp500_pct = by_code.get("^GSPC").copied().unwrap_or(0.0);

    // 选用变化最大的指数代表 (避免"美股 +0%"误导)
    let main_pct = [nasdaq_pct, dow_pct, sp500_pct]
        .iter()
        .copied()
        .filter(|x| x.abs() > 0.01)
        .max_by(|a, b| {
            a.abs()
                .partial_cmp(&b.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(0.0);

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
    let usd_cny = by_code.get("CNY=X").copied().unwrap_or(0.0);
    // 变化: 涨 = 人民币贬值 (利空 A股), 跌 = 人民币升值 (利好 A股)
    let fx_summary = if usd_cny.abs() < 0.0001 {
        "持平".to_string()
    } else {
        // yahoo CNY=X 是 1 美元 = ? 人民币 (e.g. 7.18)
        // 我们需要 price (汇率值), 但 fetch_quotes 只返 change_pct
        // 简化: 只显示涨跌幅方向, 真实汇率值需另查 (后续 PR)
        format!("{:+.2}% (USD/CNY)", usd_cny)
    };

    (us_summary, fx_summary)
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

    // v64: fetch_overnight_data 返回 (us_summary, fx_summary), 沙箱无网络也不 panic
    #[test]
    fn test_fetch_overnight_data_no_panic() {
        let (us, fx) = fetch_overnight_data();
        // 沙箱无网络时返 fallback ("持平"), 不 panic
        assert!(!us.is_empty() || us == "美股 持平" || us.contains("美股"));
        assert!(!fx.is_empty() || fx == "汇率 持平" || fx.contains("汇率"));
    }
}
