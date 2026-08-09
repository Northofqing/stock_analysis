//! Eastmoney 概念板块排行统一网关 (I-09 修复)。
//!
//! 背景: 6f4b601 (2026-08-04) data_gateway 统一边界引入后, ConceptBoard
//! 完整契约 (vol_ratio/turnover/day1_ratio/day5_ratio) 未发布,
//! sector_monitor::fetch_board_ranking 硬编码 bail → 当前二进制板块样本
//! (I-09) 必失败。8/7 生产板块样本正常是因为跑的是 6f4b601 前的旧二进制。
//!
//! 本网关在统一边界内实现真实 Eastmoney 板块排行:
//!   fs=m:90+t:3+f:!50 (概念板块), fields=f3(涨幅),f8(换手),f10(量比),
//!   f12(代码),f14(名称),f62(主力净流入),f128(领涨股),f184(今日主力净占比),
//!   f165(5日主力净占比) — 与 ConceptBoard 契约逐字段对应。
//!
//! 缺失/解析失败 → 显式错误 (不回退、不拼接)。

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

const CLIST_ENDPOINT: &str = "https://push2.eastmoney.com/api/qt/clist/get";
const TOKEN: &str = "bd1d9ddb04089700cf9c27f6f7426281";
const BOARD_FILTER: &str = "m:90+t:3+f:!50";

#[derive(Debug, Clone)]
pub struct BoardRankingFact {
    /// 板块代码, 例如 "BK0815"
    pub code: String,
    /// 板块名称, 例如 "机器人概念"
    pub name: String,
    /// 当日涨跌幅 (%)
    pub change_pct: f64,
    /// 主力净流入金额 (元)
    pub main_inflow: f64,
    /// 领涨股名称
    pub leader_name: String,
    /// 量比 (f10)
    pub vol_ratio: f64,
    /// 换手率 (f8, %)
    pub turnover: f64,
    /// 今日主力净占比 (f184, %)
    pub day1_ratio: f64,
    /// 5日主力净占比 (f165, %)
    pub day5_ratio: f64,
}

#[derive(Debug, Default)]
pub struct BoardRankingGateway;

impl BoardRankingGateway {
    pub fn new() -> Self {
        Self
    }

    /// 同步拉取概念板块排行 (fid=f3 涨幅 / f62 主力净流入)。
    pub fn fetch_top(&self, fid: &str, top_n: usize) -> Result<Vec<BoardRankingFact>> {
        if !matches!(fid, "f3" | "f62") || top_n == 0 {
            anyhow::bail!("板块排行请求非法: fid={fid:?} top_n={top_n}");
        }
        let params = [
            ("pn", "1"),
            ("pz", &top_n.to_string()),
            ("po", "1"),
            ("np", "1"),
            ("ut", TOKEN),
            ("fltt", "2"),
            ("invt", "2"),
            ("fid", fid),
            ("fs", BOARD_FILTER),
            ("fields", "f3,f8,f10,f12,f14,f62,f128,f184,f165"),
            ("_", "1"),
        ];
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .context("板块排行 HTTP client 构建失败")?;
        let response = client
            .get(CLIST_ENDPOINT)
            .query(&params)
            .header("User-Agent", "Mozilla/5.0")
            .send()
            .context("板块排行请求失败")?
            .error_for_status()
            .context("板块排行 HTTP 状态异常")?;
        let json: Value = response.json().context("板块排行响应 JSON 解析失败")?;
        let diff = json
            .get("data")
            .and_then(|data| data.get("diff"))
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("板块排行响应缺少 data.diff 数组"))?;
        if diff.is_empty() {
            return Ok(Vec::new());
        }
        let mut facts = Vec::with_capacity(diff.len());
        for (index, item) in diff.iter().enumerate() {
            facts.push(parse_board_item(item, index)?);
        }
        Ok(facts)
    }
}

fn parse_board_item(item: &Value, index: usize) -> Result<BoardRankingFact> {
    let field = |name: &str| -> Result<f64> {
        item.get(name)
            .and_then(Value::as_f64)
            .ok_or_else(|| anyhow!("板块排行第 {index} 项缺少字段 {name}"))
    };
    let text = |name: &str| -> Result<String> {
        item.get(name)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("板块排行第 {index} 项缺少文本字段 {name}"))
    };
    Ok(BoardRankingFact {
        code: text("f12")?,
        name: text("f14")?,
        change_pct: field("f3")?,
        main_inflow: field("f62")?,
        leader_name: text("f128")?,
        vol_ratio: field("f10")?,
        turnover: field("f8")?,
        day1_ratio: field("f184")?,
        day5_ratio: field("f165")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_requests_are_rejected_before_any_network() {
        let gateway = BoardRankingGateway::new();
        assert!(gateway.fetch_top("f9", 5).is_err());
        assert!(gateway.fetch_top("f3", 0).is_err());
    }

    #[test]
    fn board_item_parse_maps_all_contract_fields() {
        let item = serde_json::json!({
            "f12": "BK0815",
            "f14": "机器人概念",
            "f3": 2.34,
            "f62": 1234567.0,
            "f128": "机器人龙头",
            "f10": 1.2,
            "f8": 3.4,
            "f184": 5.6,
            "f165": 7.8,
        });
        let fact = parse_board_item(&item, 0).expect("parse");
        assert_eq!(fact.code, "BK0815");
        assert_eq!(fact.name, "机器人概念");
        assert_eq!(fact.change_pct, 2.34);
        assert_eq!(fact.main_inflow, 1234567.0);
        assert_eq!(fact.leader_name, "机器人龙头");
        assert_eq!(fact.vol_ratio, 1.2);
        assert_eq!(fact.turnover, 3.4);
        assert_eq!(fact.day1_ratio, 5.6);
        assert_eq!(fact.day5_ratio, 7.8);
    }

    #[test]
    fn missing_field_is_an_explicit_error() {
        let item = serde_json::json!({ "f12": "BK0815", "f14": "机器人概念" });
        assert!(parse_board_item(&item, 0).is_err());
    }
}
