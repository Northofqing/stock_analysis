//! indices（从 market_analyzer.rs 拆分）

use anyhow::{Context, Result};
use log::info;
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
            let response = self
                .client
                .get(&full_url)
                .timeout(Duration::from_secs(10))
                .send()
                .context("请求失败")?;

            let text = response.text().context("读取响应失败")?;
            Ok(serde_json::json!({"data": text}))
        });

        let json_data = data.context("指数行情所有重试均失败")?;
        let text = json_data
            .get("data")
            .and_then(|value| value.as_str())
            .context("指数行情响应缺少 data 文本")?;
        let mut indices = Vec::with_capacity(self.main_indices.len());
        for (code, name) in &self.main_indices {
            let line = text
                .lines()
                .find(|line| line.contains(code))
                .with_context(|| format!("指数行情缺少 {name}({code})"))?;
            let data_str = self
                .parse_tencent_line(line)
                .with_context(|| format!("指数行情 {name}({code}) 引号格式非法"))?;
            let mut index = self.parse_tencent_index_data(code, name, &data_str)?;
            index.calculate_amplitude();
            indices.push(index);
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
    pub(super) fn parse_tencent_index_data(
        &self,
        code: &str,
        name: &str,
        data_str: &str,
    ) -> Result<MarketIndex> {
        let parts: Vec<&str> = data_str.split('~').collect();
        if parts.len() < 35 {
            anyhow::bail!("{name} 数据字段不足: {}", parts.len());
        }

        let parse_positive = |index: usize, label: &str| -> Result<f64> {
            let raw = parts
                .get(index)
                .copied()
                .filter(|value| !value.trim().is_empty())
                .with_context(|| format!("{name} 缺少 {label}"))?;
            let value = raw
                .parse::<f64>()
                .with_context(|| format!("{name} {label} 无法解析"))?;
            if !value.is_finite() || value <= 0.0 {
                anyhow::bail!("{name} {label} 非法: {value}");
            }
            Ok(value)
        };
        let parse_finite = |index: usize, label: &str| -> Result<f64> {
            let raw = parts
                .get(index)
                .copied()
                .filter(|value| !value.trim().is_empty())
                .with_context(|| format!("{name} 缺少 {label}"))?;
            let value = raw
                .parse::<f64>()
                .with_context(|| format!("{name} {label} 无法解析"))?;
            if !value.is_finite() {
                anyhow::bail!("{name} {label} 非法: {value}");
            }
            Ok(value)
        };
        let parse_optional_non_negative = |index: usize, label: &str| -> Result<Option<f64>> {
            let Some(raw) = parts
                .get(index)
                .copied()
                .filter(|value| !value.trim().is_empty())
            else {
                return Ok(None);
            };
            let value = raw
                .parse::<f64>()
                .with_context(|| format!("{name} {label} 无法解析"))?;
            if !value.is_finite() || value < 0.0 {
                anyhow::bail!("{name} {label} 非法: {value}");
            }
            Ok(Some(value))
        };

        // 腾讯财经指数数据格式：
        // 0:未知 1:名称 2:代码 3:当前价 4:昨收 5:今开 6:成交量(手) ... 30:涨跌 31:涨跌幅 32:最高 33:最低
        let current = parse_positive(3, "current")?;
        let prev_close = parse_positive(4, "prev_close")?;
        let open = Some(parse_positive(5, "open")?);
        let volume = parse_optional_non_negative(6, "volume")?;
        let change = parse_finite(31, "change")?;
        let change_pct = parse_finite(32, "change_pct")?;
        let high_value = parse_positive(33, "high")?;
        let low_value = parse_positive(34, "low")?;
        let high = Some(high_value);
        let low = Some(low_value);
        let amount = parse_optional_non_negative(37, "amount")?;
        if high_value < current || low_value > current {
            anyhow::bail!("{name} OHLC 关系非法");
        }

        Ok(MarketIndex {
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
            amplitude: None, // 稍后由已校验 high/low 计算
        })
    }
}
