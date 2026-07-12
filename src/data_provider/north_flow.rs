//! 北向资金（沪股通 + 深股通 净买入额）数据源
//!
//! 修复：QUANT_ANALYST_REVIEW §1.4
//! 原 bug：overview.north_flow 永远为 0，AGENTS.md 数据真实性红线
//!
//! 数据源：东财 push2.eastmoney.com/api/qt/kamt/get
//!
//! 实际响应格式 (2026-06 实测):
//! ```json
//! {
//!   "data": {
//!     "hk2sh": { "dayNetAmtIn": 0.0, "date2": "2026-06-26", ... },  // 沪股通当日净流入
//!     "sh2hk": { "dayNetAmtIn": 4200000.0, ... },                   // 沪→港 反向
//!     "hk2sz": { "dayNetAmtIn": 0.0, ... },                          // 深股通当日净流入
//!     "sz2hk": { "dayNetAmtIn": 4200000.0, ... }                    // 深→港 反向
//!   }
//! }
//! ```
//! 单位是 **元 (yuan)**，要除以 1e8 转 亿元。
//! 北向资金 = hk2sh + hk2sz；南向资金 = sh2hk + sz2hk。
//!
//! 失败时返回 Err, 绝不静默填充 (符合 AGENTS.md)。

use serde::Deserialize;

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
    /// hk2sh + hk2sz 合并（真正的北向资金）
    pub total_net: Vec<NorthFlowPoint>,
}

impl NorthFlowSeries {
    /// 最近一日的合计净流入（亿元）
    pub fn latest_total(&self) -> Option<f64> {
        self.total_net.first().map(|p| p.net_buy_amt)
    }
}

/// 单个流向（沪股通 / 沪→港 / 深股通 / 深→港）
/// dayNetAmtIn 单位是元，要除以 1e8 转 亿元
#[derive(Deserialize, Debug)]
struct RawFlow {
    #[serde(default, rename = "dayNetAmtIn")]
    day_net_amt_in: f64,
    #[serde(default, rename = "date2")]
    date2: String,
}

#[derive(Deserialize, Debug)]
struct RawData {
    #[serde(default)]
    hk2sh: Option<RawFlow>,
    #[serde(default)]
    hk2sz: Option<RawFlow>,
}

#[derive(Deserialize, Debug)]
struct RawResp {
    data: Option<RawData>,
}

pub struct NorthFlowClient {
    http: reqwest::Client,
}

impl NorthFlowClient {
    pub fn new() -> Self {
        // review #15: 复用 SHARED_FAST_HTTP_CLIENT (5s timeout, 适合快查).
        let http = crate::http_client::SHARED_FAST_HTTP_CLIENT.clone();
        Self { http }
    }

    /// 解析响应（可单测）
    pub fn parse(json: &str) -> Result<NorthFlowSeries, String> {
        let resp: RawResp =
            serde_json::from_str(json).map_err(|e| format!("北向资金响应非 JSON: {e}"))?;
        let data = resp
            .data
            .ok_or_else(|| "北向资金响应 data 字段为空".to_string())?;
        let hk2sh = data
            .hk2sh
            .ok_or_else(|| "北向资金响应 hk2sh 字段为空".to_string())?;
        let hk2sz = data
            .hk2sz
            .ok_or_else(|| "北向资金响应 hk2sz 字段为空".to_string())?;

        // date2 格式 "2026-06-26" 或空（盘中可能为空）
        let date_str = if !hk2sh.date2.is_empty() {
            hk2sh.date2.clone()
        } else if !hk2sz.date2.is_empty() {
            hk2sz.date2.clone()
        } else {
            return Err("北向资金响应无 date2 字段".to_string());
        };
        let date =
            parse_date(&date_str).ok_or_else(|| format!("北向资金日期无法解析: {date_str}"))?;

        // 元 → 亿元（1e8）
        let hk2sh_yi = hk2sh.day_net_amt_in / 1e8;
        let hk2sz_yi = hk2sz.day_net_amt_in / 1e8;
        let total_yi = hk2sh_yi + hk2sz_yi;

        Ok(NorthFlowSeries {
            hk2sh_net: vec![NorthFlowPoint {
                date,
                net_buy_amt: hk2sh_yi,
            }],
            hk2sz_net: vec![NorthFlowPoint {
                date,
                net_buy_amt: hk2sz_yi,
            }],
            total_net: vec![NorthFlowPoint {
                date,
                net_buy_amt: total_yi,
            }],
        })
    }

    /// 实际拉取（带网络）
    pub async fn fetch(&self) -> Result<NorthFlowSeries, String> {
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

    /// 同步拉取（修复 P1.1 hotfix: 不依赖 tokio runtime, 避免 blocking pool panic）
    ///
    /// 用 `reqwest::blocking` 客户端, 适合从非 async context 调用
    /// (例如 spawn_blocking 闭包, 或同步函数).
    /// 用法与 `fetch` 相同, 只是不返回 Future.
    pub fn fetch_blocking(&self) -> Result<NorthFlowSeries, String> {
        let url = "https://push2.eastmoney.com/api/qt/kamt/get?fields1=f1,f2,f3,f4&fields2=f51,f52,f53,f54,f55,f56&klt=1&lmt=1&fields=f51,f52,f54";
        let resp =
            reqwest::blocking::get(url).map_err(|e| format!("北向资金 HTTP 请求失败: {e}"))?;
        let text = resp
            .text()
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

    /// 真实东财响应 (2026-06-26 实测)
    #[test]
    fn parse_real_eastmoney_response() {
        let json = r#"{
            "rc":0,"rt":13,"svr":177617564,"lt":1,"full":1,
            "data":{
                "hk2sh":{"dayNetAmtIn":12345678.0,"date2":"2026-06-26"},
                "hk2sz":{"dayNetAmtIn":-5678901.0,"date2":"2026-06-26"}
            }
        }"#;
        let s = NorthFlowClient::parse(json).unwrap();
        // 12345678 / 1e8 = 0.12345678 亿
        assert!((s.hk2sh_net[0].net_buy_amt - 0.12345678).abs() < 1e-6);
        // -5678901 / 1e8 = -0.05678901 亿
        assert!((s.hk2sz_net[0].net_buy_amt - (-0.05678901)).abs() < 1e-6);
        // 总和 0.06666777 亿
        assert!((s.total_net[0].net_buy_amt - 0.06666777).abs() < 1e-6);
        assert!((s.latest_total().unwrap() - 0.06666777).abs() < 1e-6);
        assert_eq!(s.total_net[0].date.to_string(), "2026-06-26");
    }

    /// 盘中: hk2sh 有数据, hk2sz 缺失
    #[test]
    fn parse_missing_hk2sz_returns_err() {
        let json = r#"{"data": {"hk2sh": {"dayNetAmtIn": 100.0, "date2": "2026-06-26"}}}"#;
        let result = NorthFlowClient::parse(json);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("hk2sz"), "msg={msg}");
    }

    #[test]
    fn parse_missing_data_returns_err() {
        let json = r#"{"data": null}"#;
        let result = NorthFlowClient::parse(json);
        assert!(result.is_err());
    }

    #[test]
    fn parse_empty_date_returns_err() {
        let json = r#"{
            "data": {
                "hk2sh": {"dayNetAmtIn": 100.0, "date2": ""},
                "hk2sz": {"dayNetAmtIn": 200.0, "date2": ""}
            }
        }"#;
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

    /// 真实场景: 0 净流入 (南北都 0)
    #[test]
    fn parse_zero_net_flow() {
        let json = r#"{
            "data": {
                "hk2sh": {"dayNetAmtIn": 0.0, "date2": "2026-06-26"},
                "hk2sz": {"dayNetAmtIn": 0.0, "date2": "2026-06-26"}
            }
        }"#;
        let s = NorthFlowClient::parse(json).unwrap();
        assert_eq!(s.latest_total(), Some(0.0));
    }
}
