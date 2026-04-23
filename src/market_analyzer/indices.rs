//! indices（从 market_analyzer.rs 拆分）

use anyhow::{Context, Result};
use log::{info, warn};
use std::time::Duration;

use crate::market_data::MarketIndex;

use super::MarketAnalyzer;

impl MarketAnalyzer {
    /// 获取主要指数实时行情
    pub(super) fn get_main_indices(&self) -> Result<Vec<MarketIndex>> {
        info!("[大盘] 获取主要指数实时行情...");

        // 使用腾讯财经接口获取指数行情
        let url = "http://qt.gtimg.cn/q=";
        let codes: Vec<String> = self.main_indices.keys().cloned().collect();
        let codes_str = codes.join(",");
        let full_url = format!("{}{}", url, codes_str);

        let data = self.call_api_with_retry("指数行情", 2, || {
            let response = self.client
                .get(&full_url)
                .timeout(Duration::from_secs(10))
                .send()
                .context("请求失败")?;

            let text = response.text().context("读取响应失败")?;
            Ok(serde_json::json!({"data": text}))
        });

        let mut indices = Vec::new();

        if let Some(json_data) = data {
            if let Some(text) = json_data.get("data").and_then(|v| v.as_str()) {
                // 解析腾讯财经返回的数据格式
                // v_sh000001="1~上证指数~000001~4139.90~4132.61~4125.22~...";
                for line in text.lines() {
                    for (code, name) in &self.main_indices {
                        if line.contains(code) {
                            if let Some(data_str) = self.parse_tencent_line(line) {
                                if let Some(mut index) = self.parse_tencent_index_data(code, name, &data_str) {
                                    index.calculate_amplitude();
                                    indices.push(index);
                                }
                            }
                        }
                    }
                }
            }
        }

        info!("[大盘] 获取到 {} 个指数行情", indices.len());
        Ok(indices)
    }

    /// 解析腾讯财经数据行
    pub(super) fn parse_tencent_line(&self, line: &str) -> Option<String> {
        // 格式: v_sh000001="数据";
        if let Some(start) = line.find('"') {
            if let Some(end) = line.rfind('"') {
                if start < end {
                    return Some(line[start + 1..end].to_string());
                }
            }
        }
        None
    }

    /// 解析腾讯指数数据
    /// 腾讯接口格式：v_sh000001="1~上证指数~000001~当前价~昨收~今开~成交量~...~涨跌~涨跌幅~最高~最低~..."
    pub(super) fn parse_tencent_index_data(&self, code: &str, name: &str, data_str: &str) -> Option<MarketIndex> {
        let parts: Vec<&str> = data_str.split('~').collect();
        if parts.len() < 33 {
            warn!("[大盘] {} 数据字段不足: {}", name, parts.len());
            return None;
        }

        // 腾讯财经指数数据格式：
        // 0:未知 1:名称 2:代码 3:当前价 4:昨收 5:今开 6:成交量(手) ... 30:涨跌 31:涨跌幅 32:最高 33:最低
        let current = parts.get(3)?.parse::<f64>().ok()?;
        let prev_close = parts.get(4)?.parse::<f64>().ok()?;
        let open = parts.get(5)?.parse::<f64>().ok()?;
        let volume = parts.get(6)?.parse::<f64>().unwrap_or(0.0);
        let change = parts.get(31)?.parse::<f64>().ok()?;
        let change_pct = parts.get(32)?.parse::<f64>().ok()?;
        let high = parts.get(33)?.parse::<f64>().ok()?;
        let low = parts.get(34)?.parse::<f64>().ok()?;

        // 成交额在后面的字段，简化处理
        let amount = 0.0;

        Some(MarketIndex {
            code: code.to_string(),
            name: name.to_string(),
            current,
            change,
            change_pct,
            open,
            high,
            low,
            prev_close,
            volume,
            amount,
            amplitude: 0.0, // 稍后计算
        })
    }

}
