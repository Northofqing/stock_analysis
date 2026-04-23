//! limit_up（从 market_analyzer.rs 拆分）

use anyhow::{Context, Result};
use log::{info, warn};
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
                ("asc", "0"),        // 降序
                ("node", "hs_a"),
                ("symbol", ""),
                ("_s_r_a", "page"),
            ];

            let response = self.client
                .get(url)
                .query(&params)
                .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
                .timeout(Duration::from_secs(15))
                .send();

            let response = match response {
                Ok(r) => r,
                Err(e) => {
                    warn!("[大盘] 新浪API第{}页请求失败: {}", page, e);
                    break;
                }
            };

            let text = match response.text() {
                Ok(t) => t,
                Err(e) => {
                    warn!("[大盘] 新浪API第{}页读取失败: {}", page, e);
                    break;
                }
            };

            let json: Value = match serde_json::from_str(&text) {
                Ok(j) => j,
                Err(e) => {
                    warn!("[大盘] 新浪API第{}页解析失败: {}", page, e);
                    break;
                }
            };

            let items = match json.as_array() {
                Some(arr) if !arr.is_empty() => arr,
                _ => break,
            };

            let mut min_pct = f64::MAX;

            for item in items {
                let change_pct = item.get("changepercent").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let stock_name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let raw_code = item.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
                let code = raw_code.trim_start_matches("sh")
                    .trim_start_matches("sz")
                    .trim_start_matches("bj");
                let price = item.get("trade")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0);

                if change_pct < min_pct {
                    min_pct = change_pct;
                }

                // 过滤ST股票
                if stock_name.contains("ST") || stock_name.contains("st") {
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
                    });
                }
            }

            // 本页最低涨幅低于ST涨停阈值(5%)，不用继续翻页
            if min_pct < 4.85 {
                break;
            }
        }

        info!("[大盘] 新浪API发现 {} 只涨停股票（已排除ST/北交所）", stocks.len());
        Ok(stocks)
    }

    /// 通过东方财富push2 API获取当日涨停股票
    /// 按涨幅降序分页获取，根据各板块涨跌停阈值筛选真正涨停的
    /// - 主板(10%) / 创业板+科创板(20%) / ST(5%) / 北交所(30%)
    pub(super) fn get_limit_up_from_eastmoney(&self) -> Result<Vec<crate::market_data::TopStock>> {
        use crate::market_data::TopStock;

        let url = "https://push2.eastmoney.com/api/qt/clist/get";
        let mut stocks = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // 分页获取，ST涨停阈值5%排名靠后需翻页，最多取5页兜底
        for page in 1..=5 {
            let page_str = page.to_string();
            let params = [
                ("pn", page_str.as_str()),
                ("pz", "100"),
                ("po", "1"),       // 降序
                ("np", "1"),
                ("ut", "bd1d9ddb04089700cf9c27f6f7426281"),
                ("fltt", "2"),
                ("invt", "2"),
                ("fid", "f3"),     // 按涨幅排序
                ("fs", "m:0+t:6,m:0+t:80,m:1+t:2,m:1+t:23,m:0+t:81+s:2048"),
                ("fields", "f2,f3,f12,f14"),
            ];

            let response = self.client
                .get(url)
                .query(&params)
                .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
                .timeout(Duration::from_secs(10))
                .send()
                .context("东方财富push2 API请求失败")?;

            let text = response.text().context("读取响应失败")?;
            let json: Value = serde_json::from_str(&text).context("解析JSON失败")?;

            let diff = match json.get("data").and_then(|d| d.get("diff")).and_then(|d| d.as_array()) {
                Some(arr) if !arr.is_empty() => arr,
                _ => break,
            };

            let mut min_pct = f64::MAX;
            for item in diff {
                let code = item.get("f12").and_then(|v| v.as_str()).unwrap_or("");
                let name = item.get("f14").and_then(|v| v.as_str()).unwrap_or("");
                let change_pct = item.get("f3").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let price = item.get("f2").and_then(|v| v.as_f64()).unwrap_or(0.0);

                if change_pct < min_pct {
                    min_pct = change_pct;
                }

                if seen.contains(code) {
                    continue;
                }

                // 过滤ST股票
                if name.contains("ST") || name.contains("st") {
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
    /// - ST 股票: 5%
    /// - 创业板 (30xxxx): 20%
    /// - 科创板 (688xxx): 20%
    /// - 北交所 (8xxxxx/4xxxxx): 30%
    /// - 主板 (60xxxx/00xxxx): 10%
    pub(super) fn get_limit_pct(code: &str, name: &str) -> f64 {
        if name.contains("ST") || name.contains("st") {
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
