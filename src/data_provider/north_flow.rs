//! 北向资金（沪股通 + 深股通 净买入额）数据源
//!
//! 修复：QUANT_ANALYST_REVIEW §1.4
//! 原 bug：overview.north_flow 永远为 0，AGENTS.md 数据真实性红线
//!
//! 数据源：东财 push2.eastmoney.com/api/qt/kamt/get
//!
//! 注意：东财 kamt 接口的精确响应字段名需要在接入时实测。
//! 当前实现：先按 `hk2sh_net / hk2sz_net` 的常见字段名解析，失败时返回 Err
//! 让上层走"数据不可用"分支（符合 AGENTS.md"不静默填充"原则）。

use serde::Deserialize;
use std::time::Duration;

/// 单日北向资金净额
#[derive(Debug, Clone)]
pub struct NorthFlowPoint {
    /// 日期 YYYY-MM-DD
    pub date: chrono::NaiveDate,
    /// 净买入额（亿元，正=流入，负=流出）
    pub net_buy_amt: f64,
}

/// 北向资金序列
#[derive(Debug, Clone, Default)]
pub struct NorthFlowSeries {
    pub hk2sh_net: Vec<NorthFlowPoint>,
    pub hk2sz_net: Vec<NorthFlowPoint>,
    /// hk2sh + hk2sz 合并
    pub total_net: Vec<NorthFlowPoint>,
}

impl NorthFlowSeries {
    /// 最近一日的合计净流入（亿元）
    pub fn latest_total(&self) -> Option<f64> {
        self.total_net.first().map(|p| p.net_buy_amt)
    }
}

/// 内部原始响应结构
#[derive(Deserialize)]
struct RawEntry {
    /// YYYYMMDD 或 YYYY-MM-DD（两种都常见，解析时容错）
    #[serde(default)]
    date: String,
    /// 净买入额（亿元）
    #[serde(rename = "netBuyAmt", alias = "f54", default)]
    net_buy_amt: f64,
}

#[derive(Deserialize)]
struct RawData {
    #[serde(default)]
    hk2sh: Option<Vec<RawEntry>>,
    #[serde(default)]
    hk2sz: Option<Vec<RawEntry>>,
}

#[derive(Deserialize)]
struct RawResp {
    data: Option<RawData>,
}

pub struct NorthFlowClient {
    http: reqwest::Client,
}

impl NorthFlowClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(8))
            .build()
            .expect("reqwest client init failed");
        Self { http }
    }

    /// 解析响应（可单测）
    pub fn parse(json: &str) -> Result<NorthFlowSeries, String> {
        let resp: RawResp = serde_json::from_str(json)
            .map_err(|e| format!("北向资金响应非 JSON: {e}"))?;
        let data = resp.data.ok_or_else(|| "北向资金响应 data 字段为空".to_string())?;
        let hk2sh = data
            .hk2sh
            .ok_or_else(|| "北向资金响应 hk2sh 字段为空".to_string())?;
        let hk2sz = data
            .hk2sz
            .ok_or_else(|| "北向资金响应 hk2sz 字段为空".to_string())?;
        if hk2sh.is_empty() && hk2sz.is_empty() {
            return Err("北向资金响应无数据".to_string());
        }
        let parse_entry = |e: &RawEntry| -> Result<NorthFlowPoint, String> {
            let date = parse_date(&e.date)
                .ok_or_else(|| format!("北向资金日期无法解析: {}", e.date))?;
            Ok(NorthFlowPoint { date, net_buy_amt: e.net_buy_amt })
        };
        let hk2sh_net: Vec<NorthFlowPoint> = hk2sh
            .iter()
            .map(parse_entry)
            .collect::<Result<_, _>>()?;
        let hk2sz_net: Vec<NorthFlowPoint> = hk2sz
            .iter()
            .map(parse_entry)
            .collect::<Result<_, _>>()?;
        // 合并：按日期对位相加
        let mut total: Vec<NorthFlowPoint> = hk2sh_net
            .iter()
            .zip(hk2sz_net.iter())
            .map(|(s, z)| NorthFlowPoint {
                date: s.date,
                net_buy_amt: s.net_buy_amt + z.net_buy_amt,
            })
            .collect();
        total.sort_by(|a, b| b.date.cmp(&a.date)); // 降序
        Ok(NorthFlowSeries {
            hk2sh_net,
            hk2sz_net,
            total_net: total,
        })
    }

    /// 实际拉取（带网络）
    pub async fn fetch(&self) -> Result<NorthFlowSeries, String> {
        // 东财 kamt 接口（北向资金 - 沪股通/深股通）
        // 字段选取 f51=日期, f54=净买入额（亿元）
        let url = "https://push2.eastmoney.com/api/qt/kamt/get?fields1=f1,f2,f3,f4&fields2=f51,f52,f53,f54,f55,f56&klt=1&lmt=1&fields=f51,f52,f54";
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| format!("北向资金 HTTP 请求失败: {e}"))?;
        let text = resp
            .text()
            .await
            .map_err(|e| format!("北向资金响应文本读取失败: {e}"))?;
        Self::parse(&text)
    }
}

impl Default for NorthFlowClient {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_date(s: &str) -> Option<chrono::NaiveDate> {
    use chrono::NaiveDate;
    if s.is_empty() {
        return None;
    }
    if s.len() == 8 {
        // YYYYMMDD
        NaiveDate::parse_from_str(s, "%Y%m%d").ok()
    } else {
        NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .or_else(|_| NaiveDate::parse_from_str(s, "%Y/%m/%d"))
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mock_response() {
        let json = r#"{
            "data": {
                "hk2sh": [{"date": "20260627", "netBuyAmt": 12.34}],
                "hk2sz": [{"date": "20260627", "netBuyAmt": -5.67}]
            }
        }"#;
        let s = NorthFlowClient::parse(json).unwrap();
        assert!((s.hk2sh_net[0].net_buy_amt - 12.34).abs() < 1e-6);
        assert!((s.hk2sz_net[0].net_buy_amt - (-5.67)).abs() < 1e-6);
        assert!((s.total_net[0].net_buy_amt - 6.67).abs() < 1e-6);
        assert_eq!(s.latest_total(), Some(6.67));
    }

    #[test]
    fn parse_with_dash_date() {
        let json = r#"{
            "data": {
                "hk2sh": [{"date": "2026-06-27", "netBuyAmt": 1.0}],
                "hk2sz": [{"date": "2026-06-27", "netBuyAmt": 2.0}]
            }
        }"#;
        let s = NorthFlowClient::parse(json).unwrap();
        assert_eq!(s.total_net[0].date.to_string(), "2026-06-27");
        assert!((s.total_net[0].net_buy_amt - 3.0).abs() < 1e-6);
    }

    #[test]
    fn parse_missing_data_returns_err() {
        let json = r#"{"data": null}"#;
        let result = NorthFlowClient::parse(json);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("北向资金"), "msg={msg}");
    }

    #[test]
    fn parse_empty_lists_returns_err() {
        let json = r#"{"data": {"hk2sh": [], "hk2sz": []}}"#;
        let result = NorthFlowClient::parse(json);
        assert!(result.is_err());
    }

    #[test]
    fn parse_invalid_date_returns_err() {
        let json = r#"{
            "data": {
                "hk2sh": [{"date": "garbage", "netBuyAmt": 1.0}],
                "hk2sz": [{"date": "20260627", "netBuyAmt": 2.0}]
            }
        }"#;
        let result = NorthFlowClient::parse(json);
        assert!(result.is_err());
    }

    #[test]
    fn parse_missing_hk2sh_field_returns_err() {
        let json = r#"{"data": {"hk2sz": [{"date": "20260627", "netBuyAmt": 1.0}]}}"#;
        let result = NorthFlowClient::parse(json);
        assert!(result.is_err());
    }

    #[test]
    fn parse_non_json_returns_err() {
        let result = NorthFlowClient::parse("not json");
        assert!(result.is_err());
    }

    #[test]
    fn latest_total_empty_returns_none() {
        let s = NorthFlowSeries::default();
        assert_eq!(s.latest_total(), None);
    }
}
