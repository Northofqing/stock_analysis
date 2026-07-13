//! 行情数据抓取 — 从 main.rs 提取

use stock_analysis;
use crate::{validate_position_freshness, validate_quote_freshness, monitor_freshness_config};

/// 持仓实时行情：东财 push2 为主（多主机轮询），新浪兜底
pub fn fetch_position_quotes() -> Vec<stock_analysis::market_data::TopStock> {
    let codes: Vec<String> = stock_analysis::portfolio::get_all_codes().unwrap_or_default();
    if codes.is_empty() { return vec![]; }

    let quotes = match fetch_eastmoney_quotes(&codes) {
        Ok(q) if !q.is_empty() => q,
        _ => fetch_sina_quotes(&codes),
    };
    if quotes.is_empty() {
        return quotes;
    }
    if !validate_position_freshness(chrono::Local::now()) {
        return vec![];
    }
    quotes
}

/// 东方财富 push2 实时行情（多主机轮询，含 volume_ratio + main_net_yi）
pub fn fetch_eastmoney_quotes(codes: &[String]) -> Result<Vec<stock_analysis::market_data::TopStock>, String> {
    use chrono::TimeZone;
    use stock_analysis::market_data::TopStock;
    // secids: 0.000547,1.603618 (0=深交所,1=上交所)
    let secids: Vec<String> = codes.iter().map(|c| {
        if c.starts_with('6') || c.starts_with('5') { format!("1.{}", c) }
        else { format!("0.{}", c) }
    }).collect();
    let url_path = format!("/api/qt/ulist.np/get?secids={}&fields=f2,f3,f10,f12,f14,f62,f124&fltt=2&invt=2",
        secids.join(","));

    const HOSTS: &[&str] = &["push2delay.eastmoney.com", "push2.eastmoney.com", "82.push2.eastmoney.com"];
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build().map_err(|e| e.to_string())?;

    for host in HOSTS {
        let url = format!("https://{}{}", host, url_path);
        let resp = client.get(&url)
            .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
            .header("Referer", "https://quote.eastmoney.com/")
            .send();
        match resp.and_then(|r| r.json::<serde_json::Value>()) {
            Ok(json) => {
                if let Some(arr) = json.get("data").and_then(|d| d.get("diff")).and_then(|d| d.as_array()) {
                    let stocks: Vec<TopStock> = arr.iter().filter_map(|item| {
                        let code = item.get("f12")?.as_str()?.to_string();
                        let update_time = item
                            .get("f124")
                            .and_then(|v| v.as_i64())
                            .and_then(|secs| chrono::Local.timestamp_opt(secs, 0).single())
                            .unwrap_or_else(chrono::Local::now);
                        if !validate_quote_freshness(update_time, "eastmoney", &code) {
                            return None;
                        }
                        Some(TopStock {
                            code,
                            name: item.get("f14")?.as_str()?.to_string(),
                            price: item.get("f2").and_then(|v| v.as_f64()).unwrap_or(0.0),
                            change_pct: item.get("f3").and_then(|v| v.as_f64()).unwrap_or(0.0),
                            volume_ratio: item.get("f10").and_then(|v| v.as_f64()).unwrap_or(0.0),
                            main_net_yi: item.get("f62").and_then(|v| v.as_f64()).unwrap_or(0.0) / 1e8,
                        })
                    }).collect();
                    if !stocks.is_empty() { return Ok(stocks); }
                }
            }
            Err(_) => continue,
        }
    }
    Err("所有东财主机请求失败".into())
}

pub fn infer_limit_pct(code: &str, name: &str) -> f64 {
    if name.contains("ST") || name.contains("st") {
        5.0
    } else if code.starts_with("30") || code.starts_with("688") {
        20.0
    } else if code.starts_with('8') || code.starts_with('4') {
        30.0
    } else {
        10.0
    }
}

/// 批量查询连板数，返回 1=首板 / 2=二板 / 3=三板+
/// 仅向前看 4 个交易日的 K 线，够判断三板就够了。
pub fn lookup_board_level_batch(codes: &[(String, String)]) -> std::collections::HashMap<String, u8> {
    let mut out = std::collections::HashMap::new();
    let fetcher = match stock_analysis::data_provider::DataFetcherManager::new() {
        Ok(v) => v,
        Err(e) => {
            log::warn!("[连板识别] 初始化数据抓取失败: {:#}", e);
            return out;
        }
    };

    for (code, name) in codes {
        let level = match fetcher.get_daily_data(code, 5) {
            Ok((kline, _)) if kline.len() >= 2 => {
                let threshold = infer_limit_pct(code, name) - 0.2;
                let n = kline.len();
                // kline 按时间升序，最后一条是今日。往前数连续涨停天数：
                // kline[n-2]=昨日, kline[n-3]=前天, …
                let day1 = if n >= 2 { kline[n - 2].pct_chg >= threshold } else { false };
                let day2 = if n >= 3 { kline[n - 3].pct_chg >= threshold } else { false };
                match (day1, day2) {
                    (false, _) => 1,  // 昨日未涨停 → 首板
                    (true, false) => 2, // 昨日涨停、前天未 → 二板
                    (true, true) => 3,  // 两天前均涨停 → 三板+
                }
            }
            Ok(_) => {
                log::warn!("[连板识别] {}({}) K线不足，跳过", name, code);
                continue;
            }
            Err(e) => {
                log::warn!("[连板识别] {}({}) 拉K线失败: {:#}", name, code, e);
                continue;
            }
        };
        out.insert(code.clone(), level);
    }
    out
}

pub fn fetch_market_top_by_fid(fid: &str, top_n: usize) -> Result<Vec<stock_analysis::market_data::TopStock>, String> {
    use chrono::TimeZone;
    use stock_analysis::market_data::TopStock;

    let pz = top_n.clamp(20, 200).to_string();
    let params = [
        ("pn", "1"),
        ("pz", pz.as_str()),
        ("po", "1"),
        ("np", "1"),
        ("ut", "bd1d9ddb04089700cf9c27f6f7426281"),
        ("fltt", "2"),
        ("invt", "2"),
        ("fid", fid),
        ("fs", "m:0+t:6,m:0+t:80,m:1+t:2,m:1+t:23,m:0+t:81+s:2048"),
        ("fields", "f2,f3,f10,f12,f14,f62,f124"),
    ];

    const HOSTS: &[&str] = &["push2delay.eastmoney.com", "push2.eastmoney.com", "82.push2.eastmoney.com"];
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build().map_err(|e| e.to_string())?;

    for host in HOSTS {
        let url = format!("https://{}/api/qt/clist/get", host);
        let resp = client.get(&url)
            .query(&params)
            .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
            .header("Referer", "https://quote.eastmoney.com/")
            .send();
        match resp.and_then(|r| r.json::<serde_json::Value>()) {
            Ok(json) => {
                if let Some(arr) = json.get("data").and_then(|d| d.get("diff")).and_then(|d| d.as_array()) {
                    let stocks: Vec<TopStock> = arr.iter().filter_map(|item| {
                        let code = item.get("f12")?.as_str()?.to_string();
                        let name = item.get("f14")?.as_str()?.to_string();
                        if name.contains("ST") || name.contains("st") {
                            return None;
                        }
                        if code.starts_with('8') || code.starts_with('4') || code.starts_with('9') {
                            return None;
                        }
                        let update_time = item
                            .get("f124")
                            .and_then(|v| v.as_i64())
                            .and_then(|secs| chrono::Local.timestamp_opt(secs, 0).single())
                            .unwrap_or_else(chrono::Local::now);
                        if !validate_quote_freshness(update_time, "eastmoney_market", &code) {
                            return None;
                        }
                        Some(TopStock {
                            code,
                            name,
                            price: item.get("f2").and_then(|v| v.as_f64()).unwrap_or(0.0),
                            change_pct: item.get("f3").and_then(|v| v.as_f64()).unwrap_or(0.0),
                            volume_ratio: item.get("f10").and_then(|v| v.as_f64()).unwrap_or(0.0),
                            main_net_yi: item.get("f62").and_then(|v| v.as_f64()).unwrap_or(0.0) / 1e8,
                        })
                    }).collect();
                    if !stocks.is_empty() {
                        return Ok(stocks);
                    }
                }
            }
            Err(_) => continue,
        }
    }
    Err("全市场榜单请求失败（所有东财主机）".to_string())
}

pub fn fetch_market_main_inflow_top(top_n: usize) -> Result<Vec<stock_analysis::market_data::TopStock>, String> {
    let mut stocks = fetch_market_top_by_fid("f62", top_n * 4)?;
    stocks.retain(|s| s.main_net_yi > 0.0 && s.price > 0.0);
    stocks.sort_by(|a, b| b.main_net_yi.partial_cmp(&a.main_net_yi).unwrap_or(std::cmp::Ordering::Equal));
    stocks.truncate(top_n);
    Ok(stocks)
}

pub fn fetch_market_volume_ratio_leaders(top_n: usize) -> Result<Vec<stock_analysis::market_data::TopStock>, String> {
    let mut stocks = fetch_market_top_by_fid("f10", top_n * 6)?;
    stocks.retain(|s| {
        s.price > 0.0
            && s.volume_ratio >= 1.8
            && s.change_pct >= 0.5
            && s.change_pct <= 9.5
    });
    stocks.sort_by(|a, b| {
        b.volume_ratio
            .partial_cmp(&a.volume_ratio)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    stocks.truncate(top_n);
    Ok(stocks)
}

/// 新浪行情 API：免费、稳定、无频率限制、无需 Referer/Cookie/Token。
/// URL: http://hq.sinajs.cn/list=sz000547,sh603618
/// 返回: var hq_str_sz000547="名称,今开,昨收,现价,最高,最低,..."
pub fn fetch_sina_quotes(codes: &[String]) -> Vec<stock_analysis::market_data::TopStock> {
    use stock_analysis::market_data::TopStock;
    // 新浪 A 股符号映射：深交所 sz，上交所(6/5开头) sh
    let symbols: Vec<String> = codes.iter().map(|c| {
        if c.starts_with('6') || c.starts_with('5') { format!("sh{}", c) }
        else { format!("sz{}", c) }
    }).collect();
    let url = format!("http://hq.sinajs.cn/list={}", symbols.join(","));

    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build() { Ok(c) => c, Err(_) => return vec![] };

    let text = match client.get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .header("Referer", "https://finance.sina.com.cn/")
        .send().and_then(|r| r.text()) // 新浪返回 GBK 文本，reqwest 自动解码
    { Ok(t) => t, Err(e) => { log::warn!("[新浪行情] 请求失败: {}", e); return vec![]; } };

    // 逐行解析：var hq_str_sz000547="名称,今开,昨收,...";
    let mut results = Vec::new();
    for (symbol, code) in symbols.iter().zip(codes.iter()) {
        // 从文本中提取该股票的数据行
        let prefix = format!("var hq_str_{}=\"", symbol);
        let start = match text.find(&prefix) { Some(p) => p + prefix.len(), None => continue };
        let end = match text[start..].find('"') { Some(p) => start + p, None => continue };
        let data = &text[start..end];
        let fields: Vec<&str> = data.split(',').collect();
        if fields.len() < 4 { continue; }

        let name = fields[0].to_string();
        let prev_close: f64 = fields.get(2).and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let price: f64 = fields.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let change_pct = if prev_close > 0.0 { (price - prev_close) / prev_close * 100.0 } else { 0.0 };
        if !validate_quote_freshness(chrono::Local::now(), "sina", code) {
            continue;
        }

        results.push(TopStock {
            code: code.clone(), name,
            price, change_pct,
            volume_ratio: 0.0,   // 新浪不提供量比
            main_net_yi: 0.0,    // 新浪不提供主力净流入
        });
    }
    results
}

/// 拉取上证指数涨跌幅（新浪 API）
pub fn fetch_sh_index_change() -> f64 {
    fn is_reasonable_index_change(change_pct: f64) -> bool {
        change_pct.is_finite() && change_pct.abs() <= 20.0
    }

    if let Ok(analyzer) = stock_analysis::market_analyzer::MarketAnalyzer::new(None) {
        if let Ok(overview) = analyzer.get_market_overview() {
            if let Some(sh_index) = overview.get_sh_index() {
                if is_reasonable_index_change(sh_index.change_pct) {
                    return sh_index.change_pct;
                } else {
                    log::warn!(
                        "[收盘总结] 上证指数涨跌幅异常，已忽略概览数据: {:.2}%",
                        sh_index.change_pct
                    );
                }
            }
        }
    }

    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build() { Ok(c) => c, Err(_) => return 0.0 };
    let text = match client.get("http://hq.sinajs.cn/list=s_sh000001")
        .header("User-Agent", "Mozilla/5.0")
        .header("Referer", "https://finance.sina.com.cn/")
        .send().and_then(|r| r.text())
    { Ok(t) => t, Err(_) => return 0.0 };
    // 格式：var hq_str_s_sh000001="上证指数,3267.19,3258.86,..."
    if let Some(start) = text.find('"') {
        if let Some(end) = text[start+1..].find('"') {
            let data = &text[start+1..start+1+end];
            let fields: Vec<&str> = data.split(',').collect();
            // fields[1]=当前价, fields[2]=昨收
            if fields.len() >= 3 {
                let price: f64 = fields[1].parse().unwrap_or(0.0);
                let prev: f64 = fields[2].parse().unwrap_or(0.0);
                if prev > 0.0 {
                    let change_pct = (price - prev) / prev * 100.0;
                    if is_reasonable_index_change(change_pct) {
                        return change_pct;
                    }
                    log::warn!(
                        "[收盘总结] 新浪上证指数涨跌幅异常，已回退为 0: {:.2}% (price={:.2}, prev={:.2})",
                        change_pct,
                        price,
                        prev
                    );
                }
            }
        }
    }
    0.0
}

/// P0-3: 批量取三大指数涨跌幅 (上证/创业板指/科创50), 来自 overview.indices 真实数据
///   替代之前 chinext=sh*0.8 估算 / star=0.0 硬编码. 任一缺失 → 0.0 (调用方按"暂无"展示)
///   返回 (sh_chg, chinext_chg, star_chg)
pub fn fetch_index_changes() -> (f64, f64, f64) {
    fn reasonable(x: f64) -> bool {
        x.is_finite() && x.abs() <= 20.0
    }
    if let Ok(analyzer) = stock_analysis::market_analyzer::MarketAnalyzer::new(None) {
        if let Ok(overview) = analyzer.get_market_overview() {
            let mut sh = 0.0_f64;
            let mut chinext = 0.0_f64;
            let mut star = 0.0_f64;
            for idx in &overview.indices {
                if !reasonable(idx.change_pct) {
                    continue;
                }
                if idx.code.ends_with("399006") {
                    chinext = idx.change_pct;
                } else if idx.code.ends_with("000688") {
                    star = idx.change_pct;
                } else if idx.code.ends_with("000001") {
                    sh = idx.change_pct;
                }
            }
            return (sh, chinext, star);
        }
    }
    (0.0, 0.0, 0.0)
}

/// P0-3: 取两市成交额(亿), 来自 overview.total_amount 真实累加; 缺失 → 0.0 (按"暂无"展示)
///   替代之前 amount_yi=0.0 占位 / 500只样本 r2_mv 误导
pub fn fetch_market_amount_yi() -> f64 {
    if let Ok(analyzer) = stock_analysis::market_analyzer::MarketAnalyzer::new(None) {
        if let Ok(overview) = analyzer.get_market_overview() {
            if overview.total_amount > 0.0 {
                return overview.total_amount;
            }
        }
    }
    0.0
}
