//! Provider-neutral concept-board ranking gateway.
//!
//! Transport is exclusively the remote market-data gRPC bridge. Missing
//! transport or invalid responses fail explicitly; no local provider fallback
//! remains in this repository.

use anyhow::Result;

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
        match super::grpc_source::bridge_for("BoardRanking") {
            Ok(bridge) => {
                let batch = bridge.board_ranking(fid, top_n).map_err(|error| {
                    anyhow::anyhow!("板块排行 gRPC 桥失败 (fid={fid} top_n={top_n}): {error}")
                })?;
                return Ok(batch.records().to_vec());
            }
            Err(error) => return Err(anyhow::anyhow!("板块排行 gRPC 桥不可用: {error}")),
        }
    }
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
}
