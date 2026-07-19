//! 行情数据抓取 — 从 main.rs 提取

use crate::freshness::validate_daily_snapshot_freshness;
use crate::{validate_position_freshness, validate_quote_freshness};

fn validate_quote_batch_codes(
    requested: &[String],
    quotes: &[stock_analysis::market_data::TopStock],
    source: &str,
) -> Result<(), String> {
    use std::collections::HashSet;

    let requested_set: HashSet<&str> = requested.iter().map(String::as_str).collect();
    if requested_set.len() != requested.len() {
        return Err(format!("{source} 请求代码包含重复项"));
    }
    let mut returned_set = HashSet::new();
    for quote in quotes {
        if !returned_set.insert(quote.code.as_str()) {
            return Err(format!("{source} 行情重复代码: {}", quote.code));
        }
    }
    if returned_set != requested_set {
        let mut missing: Vec<&str> = requested_set.difference(&returned_set).copied().collect();
        let mut extra: Vec<&str> = returned_set.difference(&requested_set).copied().collect();
        missing.sort_unstable();
        extra.sort_unstable();
        return Err(format!(
            "{source} 行情批次代码不完整: missing={missing:?} extra={extra:?}"
        ));
    }
    Ok(())
}

fn mark_capability_success(
    capability: stock_analysis::monitor::data_mode::Capability,
) -> Result<(), String> {
    stock_analysis::monitor::data_mode::mark_capability_success(capability)
}

/// 持仓实时行情：东财 push2 为主（多主机轮询），新浪兜底
pub fn fetch_position_quotes() -> Result<Vec<stock_analysis::market_data::TopStock>, String> {
    let (positions, source_time) = stock_analysis::portfolio::get_positions_with_source_time()
        .map_err(|error| format!("持仓批次查询失败: {error}"))?;
    let codes: Vec<String> = positions
        .into_iter()
        .map(|position| position.code)
        .collect();
    if codes.is_empty() {
        return Ok(vec![]);
    }
    let source_time = source_time.ok_or_else(|| "持仓批次缺少来源时间".to_string())?;
    if !validate_position_freshness(source_time) {
        return Err(format!(
            "持仓批次未通过 30 秒新鲜度门: oldest_source_time={source_time}"
        ));
    }

    let quotes = match fetch_eastmoney_quotes(&codes) {
        Ok(q) if !q.is_empty() => q,
        Ok(_) => fetch_sina_quotes(&codes)?,
        Err(primary_error) => fetch_sina_quotes(&codes).map_err(|fallback_error| {
            format!("持仓行情主备源均失败: 东财={primary_error}; 新浪={fallback_error}")
        })?,
    };
    if quotes.is_empty() {
        return Err("持仓行情源成功响应但无有效行".to_string());
    }
    mark_capability_success(stock_analysis::monitor::data_mode::Capability::Quote)?;
    Ok(quotes)
}

/// 东方财富 push2 实时行情（多主机轮询，含 volume_ratio + main_net_yi）
pub fn fetch_eastmoney_quotes(
    codes: &[String],
) -> Result<Vec<stock_analysis::market_data::TopStock>, String> {
    use chrono::TimeZone;
    use stock_analysis::market_data::TopStock;
    // secids: 0.000547,1.603618 (0=深交所,1=上交所)
    let secids: Vec<String> = codes
        .iter()
        .map(|c| {
            if c.starts_with('6') || c.starts_with('5') {
                format!("1.{}", c)
            } else {
                format!("0.{}", c)
            }
        })
        .collect();
    let url_path = format!(
        "/api/qt/ulist.np/get?secids={}&fields=f2,f3,f10,f12,f14,f62,f124&fltt=2&invt=2",
        secids.join(",")
    );

    const HOSTS: &[&str] = &[
        "push2delay.eastmoney.com",
        "push2.eastmoney.com",
        "82.push2.eastmoney.com",
    ];
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;

    for host in HOSTS {
        let url = format!("https://{}{}", host, url_path);
        let resp = client
            .get(&url)
            .header(
                "User-Agent",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36",
            )
            .header("Referer", "https://quote.eastmoney.com/")
            .send();
        match resp.and_then(|r| r.json::<serde_json::Value>()) {
            Ok(json) => {
                if let Some(arr) = json
                    .get("data")
                    .and_then(|d| d.get("diff"))
                    .and_then(|d| d.as_array())
                {
                    let stocks: Result<Vec<TopStock>, String> = arr
                        .iter()
                        .enumerate()
                        .map(|(index, item)| {
                            let code = item
                                .get("f12")
                                .and_then(|value| value.as_str())
                                .filter(|value| !value.is_empty())
                                .ok_or_else(|| format!("东财持仓行情第 {} 行缺少 code", index + 1))?
                                .to_string();
                            let name = item
                                .get("f14")
                                .and_then(|value| value.as_str())
                                .filter(|value| !value.is_empty())
                                .ok_or_else(|| format!("东财持仓行情 {code} 缺少 name"))?
                                .to_string();
                            let price = item
                                .get("f2")
                                .and_then(|value| value.as_f64())
                                .filter(|value| value.is_finite() && *value > 0.0)
                                .ok_or_else(|| format!("东财持仓行情 {code} price 缺失/非法"))?;
                            let change_pct = item
                                .get("f3")
                                .and_then(|value| value.as_f64())
                                .filter(|value| value.is_finite() && value.abs() <= 20.0)
                                .ok_or_else(|| {
                                    format!("东财持仓行情 {code} change_pct 缺失/超过±20%")
                                })?;
                            let update_time = item
                                .get("f124")
                                .and_then(|v| v.as_i64())
                                .and_then(|secs| chrono::Local.timestamp_opt(secs, 0).single())
                                .ok_or_else(|| format!("东财持仓行情 {code} 缺少有效更新时间"))?;
                            if !validate_quote_freshness(update_time, "eastmoney", &code) {
                                return Err(format!("东财持仓行情 {code} 超过 5 秒新鲜度"));
                            }
                            let volume_ratio = item
                                .get("f10")
                                .and_then(|value| value.as_f64())
                                .filter(|value| value.is_finite() && *value >= 0.0);
                            let main_net_yi = item
                                .get("f62")
                                .and_then(|value| value.as_f64())
                                .filter(|value| value.is_finite())
                                .map(|value| value / 1e8);
                            Ok(TopStock {
                                code,
                                name,
                                price,
                                change_pct,
                                volume_ratio,
                                main_net_yi,
                            })
                        })
                        .collect();
                    let stocks = stocks?;
                    if !stocks.is_empty() {
                        validate_quote_batch_codes(codes, &stocks, "eastmoney")?;
                        mark_capability_success(
                            stock_analysis::monitor::data_mode::Capability::Quote,
                        )?;
                        return Ok(stocks);
                    }
                }
            }
            Err(_) => continue,
        }
    }
    Err("所有东财主机请求失败".into())
}

pub fn infer_limit_pct(code: &str, name: &str) -> f64 {
    #[cfg(test)]
    let code = code.strip_prefix("TEST_CODE_").unwrap_or(code);
    if name.contains("ST") || name.contains("st") {
        5.0
    } else if code.starts_with("30") || code.starts_with("688") {
        20.0
    } else if code.starts_with('8') || code.starts_with('4') || code.starts_with("92") {
        30.0
    } else {
        10.0
    }
}

/// 批量查询连板数，返回 1=首板 / 2=二板 / 3=三板+
/// 仅向前看 4 个交易日的 K 线，够判断三板就够了。
pub fn lookup_board_level_batch(
    codes: &[(String, String)],
) -> Result<std::collections::HashMap<String, u8>, String> {
    let mut out = std::collections::HashMap::new();
    let fetcher = stock_analysis::data_provider::DataFetcherManager::new()
        .map_err(|error| format!("[连板识别] 初始化数据抓取失败: {error:#}"))?;
    let today = chrono::Local::now().date_naive();

    for (code, name) in codes {
        let (kline, source) = fetcher
            .get_daily_data(code, 5)
            .map_err(|error| format!("[连板识别] {name}({code}) 拉 K 线失败: {error:#}"))?;
        let latest = kline
            .first()
            .ok_or_else(|| format!("[连板识别] {name}({code}) K 线为空"))?;
        if !validate_daily_snapshot_freshness(latest.date, source, code) {
            return Err(format!(
                "[连板识别] {name}({code}) 最新日 K {} 不满足时效门",
                latest.date
            ));
        }
        let threshold = infer_limit_pct(code, name) - 0.2;
        let history_start = usize::from(latest.date == today);
        let prior_limit_days = kline
            .iter()
            .skip(history_start)
            .take(2)
            .take_while(|bar| bar.is_limit_up || bar.pct_chg >= threshold)
            .count();
        let level = u8::try_from(1 + prior_limit_days)
            .map_err(|_| format!("[连板识别] {name}({code}) 连板数溢出"))?;
        out.insert(code.clone(), level);
    }
    Ok(out)
}

pub fn fetch_market_top_by_fid(
    fid: &str,
    top_n: usize,
) -> Result<Vec<stock_analysis::market_data::TopStock>, String> {
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

    const HOSTS: &[&str] = &[
        "push2delay.eastmoney.com",
        "push2.eastmoney.com",
        "82.push2.eastmoney.com",
    ];
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    for host in HOSTS {
        let url = format!("https://{}/api/qt/clist/get", host);
        let resp = client
            .get(&url)
            .query(&params)
            .header(
                "User-Agent",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36",
            )
            .header("Referer", "https://quote.eastmoney.com/")
            .send();
        match resp.and_then(|r| r.json::<serde_json::Value>()) {
            Ok(json) => {
                if let Some(arr) = json
                    .get("data")
                    .and_then(|d| d.get("diff"))
                    .and_then(|d| d.as_array())
                {
                    let stocks: Result<Vec<TopStock>, String> = arr
                        .iter()
                        .enumerate()
                        .filter_map(|(index, item)| {
                            let code = match item.get("f12").and_then(|value| value.as_str()) {
                                Some(value) if !value.is_empty() => value.to_string(),
                                _ => {
                                    return Some(Err(format!(
                                        "东财市场榜第 {} 行缺少 code",
                                        index + 1
                                    )))
                                }
                            };
                            let name = match item.get("f14").and_then(|value| value.as_str()) {
                                Some(value) if !value.is_empty() => value.to_string(),
                                _ => return Some(Err(format!("东财市场榜 {code} 缺少 name"))),
                            };
                            if name.contains("ST") || name.contains("st") {
                                return None;
                            }
                            if code.starts_with('8')
                                || code.starts_with('4')
                                || code.starts_with('9')
                            {
                                return None;
                            }
                            let price = match item
                                .get("f2")
                                .and_then(|value| value.as_f64())
                                .filter(|value| value.is_finite() && *value > 0.0)
                            {
                                Some(value) => value,
                                None => {
                                    return Some(Err(format!("东财市场榜 {code} price 缺失/非法")))
                                }
                            };
                            let change_pct = match item
                                .get("f3")
                                .and_then(|value| value.as_f64())
                                .filter(|value| value.is_finite() && value.abs() <= 20.0)
                            {
                                Some(value) => value,
                                None => {
                                    return Some(Err(format!(
                                        "东财市场榜 {code} change_pct 缺失/超过±20%"
                                    )))
                                }
                            };
                            let update_time = item
                                .get("f124")
                                .and_then(|v| v.as_i64())
                                .and_then(|secs| chrono::Local.timestamp_opt(secs, 0).single());
                            let Some(update_time) = update_time else {
                                return Some(Err(format!("东财市场榜 {code} 缺少有效更新时间")));
                            };
                            if !validate_quote_freshness(update_time, "eastmoney_market", &code) {
                                return Some(Err(format!("东财市场榜 {code} 超过 5 秒新鲜度")));
                            }
                            let volume_ratio = item
                                .get("f10")
                                .and_then(|value| value.as_f64())
                                .filter(|value| value.is_finite() && *value >= 0.0);
                            let main_net_yi = item
                                .get("f62")
                                .and_then(|value| value.as_f64())
                                .filter(|value| value.is_finite())
                                .map(|value| value / 1e8);
                            Some(Ok(TopStock {
                                code,
                                name,
                                price,
                                change_pct,
                                volume_ratio,
                                main_net_yi,
                            }))
                        })
                        .collect();
                    let stocks = stocks?;
                    if !stocks.is_empty() {
                        mark_capability_success(
                            stock_analysis::monitor::data_mode::Capability::Quote,
                        )?;
                        return Ok(stocks);
                    }
                }
            }
            Err(_) => continue,
        }
    }
    Err("全市场榜单请求失败（所有东财主机）".to_string())
}

pub fn fetch_market_main_inflow_top(
    top_n: usize,
) -> Result<Vec<stock_analysis::market_data::TopStock>, String> {
    let mut stocks = fetch_market_top_by_fid("f62", top_n * 4)?;
    if stocks.iter().any(|stock| stock.main_net_yi.is_none()) {
        return Err("主力净流入榜成功响应包含缺失 f62 的行".to_string());
    }
    mark_capability_success(stock_analysis::monitor::data_mode::Capability::MoneyFlow)?;
    stocks.retain(|s| s.main_net_yi.is_some_and(|value| value > 0.0) && s.price > 0.0);
    stocks.sort_by(|a, b| match (a.main_net_yi, b.main_net_yi) {
        (Some(a_value), Some(b_value)) => b_value.total_cmp(&a_value),
        _ => std::cmp::Ordering::Equal,
    });
    stocks.truncate(top_n);
    Ok(stocks)
}

pub fn fetch_market_volume_ratio_leaders(
    top_n: usize,
) -> Result<Vec<stock_analysis::market_data::TopStock>, String> {
    let mut stocks = fetch_market_top_by_fid("f10", top_n * 6)?;
    stocks.retain(|s| {
        s.price > 0.0
            && s.volume_ratio.is_some_and(|value| value >= 1.8)
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
pub fn fetch_sina_quotes(
    codes: &[String],
) -> Result<Vec<stock_analysis::market_data::TopStock>, String> {
    use stock_analysis::market_data::TopStock;
    // 新浪 A 股符号映射：深交所 sz，上交所(6/5开头) sh
    let symbols: Vec<String> = codes
        .iter()
        .map(|c| {
            if c.starts_with('6') || c.starts_with('5') {
                format!("sh{}", c)
            } else {
                format!("sz{}", c)
            }
        })
        .collect();
    let url = format!("http://hq.sinajs.cn/list={}", symbols.join(","));

    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
    {
        Ok(c) => c,
        Err(error) => return Err(format!("新浪行情 HTTP 客户端构建失败: {error}")),
    };

    let text = match client.get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .header("Referer", "https://finance.sina.com.cn/")
        .send().and_then(|r| r.text()) // 新浪返回 GBK 文本，reqwest 自动解码
    { Ok(t) => t, Err(e) => return Err(format!("新浪行情请求/读取失败: {e}")) };

    // 逐行解析：var hq_str_sz000547="名称,今开,昨收,...";
    let mut results = Vec::new();
    for (symbol, code) in symbols.iter().zip(codes.iter()) {
        // 从文本中提取该股票的数据行
        let prefix = format!("var hq_str_{}=\"", symbol);
        let start = match text.find(&prefix) {
            Some(p) => p + prefix.len(),
            None => return Err(format!("新浪行情缺少 {code} 数据行")),
        };
        let end = match text[start..].find('"') {
            Some(p) => start + p,
            None => return Err(format!("新浪行情 {code} 响应引号不完整")),
        };
        let data = &text[start..end];
        let fields: Vec<&str> = data.split(',').collect();
        if fields.len() < 32 {
            return Err(format!("新浪行情 {code} 字段不足: {}", fields.len()));
        }

        let name = fields[0].trim().to_string();
        if name.is_empty() {
            return Err(format!("新浪行情 {code} 缺少名称"));
        }
        let prev_close = fields[2]
            .parse::<f64>()
            .map_err(|error| format!("新浪行情 {code} 昨收解析失败: {error}"))?;
        let price = fields[3]
            .parse::<f64>()
            .map_err(|error| format!("新浪行情 {code} 现价解析失败: {error}"))?;
        if !prev_close.is_finite() || prev_close <= 0.0 || !price.is_finite() || price <= 0.0 {
            return Err(format!("新浪行情 {code} 现价/昨收非法"));
        }
        let change_pct = (price - prev_close) / prev_close * 100.0;
        if !change_pct.is_finite() || change_pct.abs() > 20.0 {
            return Err(format!("新浪行情 {code} 涨跌幅超过±20%: {change_pct}"));
        }
        let source_time = chrono::NaiveDateTime::parse_from_str(
            &format!("{} {}", fields[30], fields[31]),
            "%Y-%m-%d %H:%M:%S",
        )
        .map_err(|error| format!("新浪行情 {code} 源时间非法: {error}"))?;
        let source_time = chrono::TimeZone::from_local_datetime(&chrono::Local, &source_time)
            .single()
            .ok_or_else(|| format!("新浪行情 {code} 源时间存在时区歧义"))?;
        if !validate_quote_freshness(source_time, "sina", code) {
            return Err(format!("新浪行情 {code} 未通过 5 秒新鲜度"));
        }

        results.push(TopStock {
            code: code.clone(),
            name,
            price,
            change_pct,
            volume_ratio: None, // 新浪不提供量比
            main_net_yi: None,  // 新浪不提供主力净流入
        });
    }
    validate_quote_batch_codes(codes, &results, "sina")?;
    mark_capability_success(stock_analysis::monitor::data_mode::Capability::Quote)?;
    Ok(results)
}

#[cfg(test)]
mod quote_batch_tests {
    use super::*;
    use stock_analysis::market_data::TopStock;

    fn http_proxy_once(body: String) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 8192];
            let n = stream.read(&mut request).unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&request[..n]).into_owned()
        });
        (format!("http://{addr}"), handle)
    }

    fn quote(code: &str) -> TopStock {
        TopStock {
            code: code.to_string(),
            name: code.to_string(),
            change_pct: 1.0,
            price: 10.0,
            volume_ratio: None,
            main_net_yi: None,
        }
    }

    #[test]
    fn br097_quote_batch_requires_exact_code_set() {
        let requested = vec![
            "TEST_CODE_000001".to_string(),
            "TEST_CODE_600000".to_string(),
        ];
        assert!(validate_quote_batch_codes(
            &requested,
            &[quote("TEST_CODE_000001"), quote("TEST_CODE_600000")],
            "test"
        )
        .is_ok());
        assert!(
            validate_quote_batch_codes(&requested, &[quote("TEST_CODE_000001")], "test").is_err()
        );
        assert!(validate_quote_batch_codes(
            &requested,
            &[quote("TEST_CODE_000001"), quote("TEST_CODE_000001")],
            "test"
        )
        .is_err());
        assert!(validate_quote_batch_codes(
            &requested,
            &[quote("TEST_CODE_000001"), quote("TEST_CODE_300001")],
            "test"
        )
        .is_err());
    }

    #[test]
    fn limit_percent_inference_covers_st_and_all_registered_boards() {
        assert_eq!(infer_limit_pct("TEST_CODE_600000", "普通测试股"), 10.0);
        assert_eq!(infer_limit_pct("TEST_CODE_300001", "创业板测试股"), 20.0);
        assert_eq!(infer_limit_pct("TEST_CODE_688001", "科创板测试股"), 20.0);
        assert_eq!(infer_limit_pct("TEST_CODE_830001", "北交所测试股"), 30.0);
        assert_eq!(infer_limit_pct("TEST_CODE_920001", "北交所测试股"), 30.0);
        assert_eq!(infer_limit_pct("TEST_CODE_600001", "*ST测试"), 5.0);
    }

    #[test]
    #[serial_test::serial(http_proxy_env)]
    fn sina_transport_parses_complete_batch_and_rejects_incomplete_protocol() {
        let keys = [
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy",
            "NO_PROXY",
            "no_proxy",
        ];
        let _env = crate::TestEnvGuard::capture(&keys);
        for key in keys {
            std::env::remove_var(key);
        }

        let now = chrono::Local::now();
        let mut fields = vec!["0".to_string(); 32];
        fields[0] = "测试行情".to_string();
        fields[1] = "10.00".to_string();
        fields[2] = "10.00".to_string();
        fields[3] = "10.50".to_string();
        fields[4] = "10.60".to_string();
        fields[5] = "9.90".to_string();
        fields[30] = now.format("%Y-%m-%d").to_string();
        fields[31] = now.format("%H:%M:%S").to_string();
        let body = format!("var hq_str_szTEST_CODE_000001=\"{}\";", fields.join(","));
        let (proxy, request) = http_proxy_once(body);
        std::env::set_var("HTTP_PROXY", &proxy);
        std::env::set_var("http_proxy", &proxy);
        let quotes = fetch_sina_quotes(&["TEST_CODE_000001".to_string()]).unwrap();
        assert_eq!(quotes.len(), 1);
        assert_eq!(quotes[0].name, "测试行情");
        assert!((quotes[0].change_pct - 5.0).abs() < 1e-9);
        assert!(request.join().unwrap().contains("hq.sinajs.cn/list="));

        let (proxy, request) =
            http_proxy_once("var hq_str_szTEST_CODE_000001=\"测试行情,10,10\";".to_string());
        std::env::set_var("HTTP_PROXY", &proxy);
        std::env::set_var("http_proxy", &proxy);
        let error = fetch_sina_quotes(&["TEST_CODE_000001".to_string()]).unwrap_err();
        assert!(error.contains("字段不足"));
        request.join().unwrap();
    }
}

/// 拉取上证指数涨跌幅；不将客户端/数据源/字段失败改成 0。
pub fn fetch_sh_index_change() -> Result<f64, String> {
    let analyzer = stock_analysis::market_analyzer::MarketAnalyzer::new(None)
        .map_err(|error| format!("上证指数 analyzer 初始化失败: {error}"))?;
    let overview = analyzer
        .get_market_overview()
        .map_err(|error| format!("上证指数快照获取失败: {error}"))?;
    let value = overview
        .get_sh_index()
        .map(|index| index.change_pct)
        .filter(|value| value.is_finite() && value.abs() <= 20.0)
        .ok_or_else(|| "上证指数涨跌幅缺失/超过±20%".to_string())?;
    Ok(value)
}

#[derive(Debug, Clone, Copy)]
pub struct MarketReviewSnapshot {
    pub sh_chg: f64,
    pub chinext_chg: f64,
    pub star_chg: f64,
    pub amount_yi: f64,
    pub limit_up_n: u32,
    pub limit_down_n: u32,
}

/// BR-093: 从同一次 MarketOverview 构造 R-02 必填快照。
pub fn fetch_market_review_snapshot() -> Result<MarketReviewSnapshot, String> {
    let analyzer = stock_analysis::market_analyzer::MarketAnalyzer::new(None)
        .map_err(|error| format!("create market analyzer: {error}"))?;
    let overview = analyzer
        .get_market_overview()
        .map_err(|error| format!("fetch market overview: {error}"))?;
    let find_change = |suffix: &str, label: &str| -> Result<f64, String> {
        let value = overview
            .indices
            .iter()
            .find(|index| index.code.ends_with(suffix))
            .map(|index| index.change_pct)
            .ok_or_else(|| format!("missing {label} index"))?;
        if !value.is_finite() || value.abs() > 20.0 {
            return Err(format!("invalid {label} change_pct: {value}"));
        }
        Ok(value)
    };
    if !overview.total_amount.is_finite() || overview.total_amount <= 0.0 {
        return Err(format!("invalid market amount: {}", overview.total_amount));
    }
    let limit_up_n = u32::try_from(overview.limit_up_count)
        .map_err(|_| format!("invalid limit_up_count: {}", overview.limit_up_count))?;
    let limit_down_n = u32::try_from(overview.limit_down_count)
        .map_err(|_| format!("invalid limit_down_count: {}", overview.limit_down_count))?;
    Ok(MarketReviewSnapshot {
        sh_chg: find_change("000001", "上证")?,
        chinext_chg: find_change("399006", "创业板")?,
        star_chg: find_change("000688", "科创50")?,
        amount_yi: overview.total_amount,
        limit_up_n,
        limit_down_n,
    })
}
