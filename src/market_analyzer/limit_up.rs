//! limit_up（从 market_analyzer.rs 拆分）

use anyhow::Result;
use log::info;
use serde_json::Value;
use std::time::Duration;

use super::MarketAnalyzer;

impl MarketAnalyzer {
    /// 通过新浪API按涨幅排序获取涨停股票（作为东方财富API的备用）
    pub(super) fn get_limit_up_from_sina(&self) -> Result<Vec<crate::market_data::TopStock>> {
        use crate::market_data::TopStock;

        let url = "http://vip.stock.finance.sina.com.cn/quotes_service/api/json_v2.php/Market_Center.getHQNodeData";
        let mut stocks = Vec::new();

        // 按涨幅倒序，每页200条，翻页到涨幅低于4.85%（ST涨停阈值下限）为止
        for page in 1..=5 {
            let page_str = page.to_string();
            let params = [
                ("page", page_str.as_str()),
                ("num", "200"),
                ("sort", "changepercent"),
                ("asc", "0"), // 降序
                ("node", "hs_a"),
                ("symbol", ""),
                ("_s_r_a", "page"),
            ];

            let response = self
                .client
                .get(url)
                .query(&params)
                .header(
                    "User-Agent",
                    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36",
                )
                .timeout(Duration::from_secs(15))
                .send();

            let response =
                response.map_err(|e| anyhow::anyhow!("新浪 API 第{page}页请求失败: {e}"))?;

            let text = response
                .text()
                .map_err(|e| anyhow::anyhow!("新浪 API 第{page}页读取失败: {e}"))?;

            let json: Value = serde_json::from_str(&text)
                .map_err(|e| anyhow::anyhow!("新浪 API 第{page}页 JSON 解析失败: {e}"))?;

            let items = match json.as_array() {
                Some(arr) if !arr.is_empty() => arr,
                _ => break,
            };

            let mut min_pct = f64::MAX;

            for (row_index, item) in items.iter().enumerate() {
                let change_pct = item
                    .get("changepercent")
                    .and_then(|v| v.as_f64())
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "新浪涨停榜第{page}页第{}行 changepercent 缺失/非法",
                            row_index + 1
                        )
                    })?;
                let stock_name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        anyhow::anyhow!("新浪涨停榜第{page}页第{}行缺少 name", row_index + 1)
                    })?;
                let raw_code = item
                    .get("symbol")
                    .and_then(|v| v.as_str())
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        anyhow::anyhow!("新浪涨停榜第{page}页第{}行缺少 symbol", row_index + 1)
                    })?;
                let code = raw_code
                    .trim_start_matches("sh")
                    .trim_start_matches("sz")
                    .trim_start_matches("bj");
                let limit_pct = Self::get_limit_pct(code, stock_name);
                if change_pct.abs() > limit_pct {
                    log::warn!(
                        "[DQ-2.3] 新浪涨停榜 {code}({stock_name}) changepercent={change_pct}% 超过常规板块±{limit_pct}%，保留真实值并标记需人工确认"
                    );
                }
                let price = item
                    .get("trade")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<f64>().ok())
                    .filter(|value| value.is_finite() && *value > 0.0)
                    .ok_or_else(|| anyhow::anyhow!("新浪涨停榜 {code} trade 缺失/非法"))?;

                if change_pct < min_pct {
                    min_pct = change_pct;
                }

                // 过滤ST股票
                if crate::data_provider::limit_status::is_st_stock(stock_name) {
                    continue;
                }
                // 过滤北交所
                if code.starts_with("8") || code.starts_with("4") || code.starts_with("9") {
                    continue;
                }

                let limit_pct = Self::get_limit_pct(code, stock_name);
                if change_pct >= limit_pct - 0.15 {
                    stocks.push(TopStock {
                        code: code.to_string(),
                        name: stock_name.to_string(),
                        change_pct,
                        price,
                        volume_ratio: None, // 新浪API无此字段
                        main_net_yi: None,
                    });
                }
            }

            // 本页最低涨幅低于ST涨停阈值(5%)，不用继续翻页
            if min_pct < 4.85 {
                break;
            }
        }

        info!(
            "[大盘] 新浪API发现 {} 只涨停股票（已排除ST/北交所）",
            stocks.len()
        );
        Ok(stocks)
    }

    /// 通过东方财富push2 API获取当日涨停股票
    /// 按涨幅降序分页获取，根据各板块涨跌停阈值筛选真正涨停的
    /// - 主板(10%) / 创业板+科创板(20%) / ST(5%) / 北交所(30%)
    pub(super) fn get_limit_up_from_eastmoney(&self) -> Result<Vec<crate::market_data::TopStock>> {
        use crate::market_data::TopStock;

        // 多主机轮询（与 sector_monitor 一致，解决单主机 RST/断流）
        const PUSH2_HOSTS: &[&str] = &[
            "push2delay.eastmoney.com",
            "push2.eastmoney.com",
            "82.push2.eastmoney.com",
        ];
        let mut stocks = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for page in 1..=5 {
            let page_str = page.to_string();
            let params = [
                ("pn", page_str.as_str()),
                ("pz", "100"),
                ("po", "1"),
                ("np", "1"),
                ("ut", "bd1d9ddb04089700cf9c27f6f7426281"),
                ("fltt", "2"),
                ("invt", "2"),
                ("fid", "f3"),
                ("fs", "m:0+t:6,m:0+t:80,m:1+t:2,m:1+t:23,m:0+t:81+s:2048"),
                ("fields", "f2,f3,f10,f12,f14,f62"),
            ];

            let mut json: Option<Value> = None;
            for host in PUSH2_HOSTS {
                let url = format!("https://{}/api/qt/clist/get", host);
                let resp = self
                    .client
                    .get(&url)
                    .query(&params)
                    .header(
                        "User-Agent",
                        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36",
                    )
                    .header("Referer", "https://quote.eastmoney.com/")
                    .timeout(Duration::from_secs(10))
                    .send();
                match resp.and_then(|r| r.text()) {
                    Ok(t) => match serde_json::from_str::<Value>(&t) {
                        Ok(j) if j.get("data").is_some() => {
                            json = Some(j);
                            break;
                        }
                        _ => continue,
                    },
                    Err(_) => continue,
                }
            }
            let json = match json {
                Some(j) => j,
                None => return Err(anyhow::anyhow!("东方财富push2 API所有主机请求失败")),
            };

            let diff = match json
                .get("data")
                .and_then(|d| d.get("diff"))
                .and_then(|d| d.as_array())
            {
                Some(arr) if !arr.is_empty() => arr,
                _ => break,
            };

            let mut min_pct = f64::MAX;
            for (row_index, item) in diff.iter().enumerate() {
                let code = item
                    .get("f12")
                    .and_then(|v| v.as_str())
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        anyhow::anyhow!("东财涨停榜第{page}页第{}行缺少 code", row_index + 1)
                    })?;
                let name = item
                    .get("f14")
                    .and_then(|v| v.as_str())
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("东财涨停榜 {code} 缺少 name"))?;
                let change_pct = item
                    .get("f3")
                    .and_then(|v| v.as_f64())
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| anyhow::anyhow!("东财涨停榜 {code} change_pct 缺失/非法"))?;
                let limit_pct = Self::get_limit_pct(code, name);
                if change_pct.abs() > limit_pct {
                    log::warn!(
                        "[DQ-2.3] 东财涨停榜 {code}({name}) change_pct={change_pct}% 超过常规板块±{limit_pct}%，保留真实值并标记需人工确认"
                    );
                }
                let price = item
                    .get("f2")
                    .and_then(|v| v.as_f64())
                    .filter(|value| value.is_finite() && *value > 0.0)
                    .ok_or_else(|| anyhow::anyhow!("东财涨停榜 {code} price 缺失/非法"))?;
                let volume_ratio = item
                    .get("f10")
                    .and_then(|v| v.as_f64())
                    .filter(|value| value.is_finite() && *value >= 0.0);
                let main_net_yi = item
                    .get("f62")
                    .and_then(|v| v.as_f64())
                    .filter(|value| value.is_finite())
                    .map(|value| value / 1e8);

                if change_pct < min_pct {
                    min_pct = change_pct;
                }

                if seen.contains(code) {
                    continue;
                }

                // 过滤ST股票
                if crate::data_provider::limit_status::is_st_stock(name) {
                    continue;
                }
                // 过滤北交所 (8xxxx/4xxxx/9xxxx开头)
                if code.starts_with("8") || code.starts_with("4") || code.starts_with("9") {
                    continue;
                }

                let limit_pct = Self::get_limit_pct(code, name);
                if change_pct < limit_pct - 0.15 {
                    continue;
                }

                seen.insert(code.to_string());
                stocks.push(TopStock {
                    code: code.to_string(),
                    name: name.to_string(),
                    change_pct,
                    price,
                    volume_ratio,
                    main_net_yi,
                });
            }

            // 本页最低涨幅已低于ST涨停阈值(5%)，无需继续翻页
            if min_pct < 4.85 {
                break;
            }
        }

        Ok(stocks)
    }

    /// 根据股票代码和名称获取涨跌停幅度限制
    ///
    /// 修复 P2.2: 增加新股上市前 5 日识别
    /// - ST 股票: 5%
    /// - 创业板 (30xxxx): 20%
    /// - 科创板 (688xxx): 20%
    /// - 北交所 (8xxxxx/4xxxxx): 30%
    /// - 主板 (60xxxx/00xxxx): 10%
    ///
    /// 新股前 5 个交易日（注册制创业板/科创板/北交所）不设涨跌幅。
    /// 调用方需在 list 业务里检查并特殊处理。
    pub(super) fn get_limit_pct(code: &str, name: &str) -> f64 {
        if crate::data_provider::limit_status::is_st_stock(name) {
            5.0
        } else if code.starts_with("30") || code.starts_with("688") {
            20.0
        } else if code.starts_with("8") || code.starts_with("4") {
            30.0
        } else {
            10.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http_proxy_once(body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 8192];
            let n = stream.read(&mut request).unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&request[..n]).into_owned()
        });
        (format!("http://{addr}"), handle)
    }

    #[test]
    #[serial_test::serial(http_proxy_env)]
    fn sina_limit_up_transport_filters_registered_exclusions_and_rejects_bad_rows() {
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
        let previous: Vec<_> = keys
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect();
        for key in keys {
            std::env::remove_var(key);
        }

        let body = r#"[
            {"symbol":"sh600000","name":"测试主板","changepercent":9.9,"trade":"10.50"},
            {"symbol":"sh600001","name":"*ST测试","changepercent":5.0,"trade":"3.00"},
            {"symbol":"bj920001","name":"北交所测试","changepercent":19.0,"trade":"20.00"},
            {"symbol":"sz000002","name":"未涨停测试","changepercent":4.0,"trade":"8.00"}
        ]"#;
        let (proxy, request) = http_proxy_once(body);
        std::env::set_var("HTTP_PROXY", &proxy);
        std::env::set_var("http_proxy", &proxy);
        let analyzer = MarketAnalyzer::new(None).unwrap();
        let stocks = analyzer.get_limit_up_from_sina().unwrap();
        assert_eq!(stocks.len(), 1);
        assert_eq!(stocks[0].code, "600000");
        assert!(request
            .join()
            .unwrap()
            .contains("vip.stock.finance.sina.com.cn"));

        let bad = r#"[{"symbol":"sh600000","changepercent":9.9,"trade":"10.50"}]"#;
        let (proxy, request) = http_proxy_once(bad);
        std::env::set_var("HTTP_PROXY", &proxy);
        std::env::set_var("http_proxy", &proxy);
        let analyzer = MarketAnalyzer::new(None).unwrap();
        assert!(analyzer
            .get_limit_up_from_sina()
            .unwrap_err()
            .to_string()
            .contains("缺少 name"));
        request.join().unwrap();

        for (key, value) in previous {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }

    #[test]
    fn limit_percent_covers_st_growth_star_bse_and_main_board() {
        assert_eq!(MarketAnalyzer::get_limit_pct("600000", "*ST测试"), 5.0);
        assert_eq!(MarketAnalyzer::get_limit_pct("300001", "创业板测试"), 20.0);
        assert_eq!(MarketAnalyzer::get_limit_pct("688001", "科创板测试"), 20.0);
        assert_eq!(MarketAnalyzer::get_limit_pct("830001", "北交所测试"), 30.0);
        assert_eq!(MarketAnalyzer::get_limit_pct("600000", "主板测试"), 10.0);
    }
}
