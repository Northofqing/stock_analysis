//! statistics（从 market_analyzer.rs 拆分）

use anyhow::Result;
use log::info;

use crate::data_gateway::{BoardDataGateway, BoardKind, GatewayBatch};
use crate::market_data::{MarketOverview, SectorInfo};

use super::MarketAnalyzer;

impl MarketAnalyzer {
    /// 获取市场涨跌统计
    pub(super) fn get_market_statistics(&self, _overview: &mut MarketOverview) -> Result<()> {
        info!("[大盘] 获取市场涨跌统计...");
        anyhow::bail!(
            "全市场涨跌统计不可用: 当前 Magic 数据契约没有提供同一完整批次的全市场行情、成交额与涨跌停身份；BR-164 禁止回退到消费端直连协议"
        )
    }

    /// 获取板块涨跌榜
    ///
    /// 数据由统一 BoardDataGateway 获取；缺失涨跌幅时整批拒绝。
    pub(super) fn get_sector_rankings(&self, overview: &mut MarketOverview) -> Result<()> {
        info!("[大盘] 获取板块涨跌榜...");

        let batch = BoardDataGateway::new()
            .day1_flows_blocking(BoardKind::Industry, 20)
            .map_err(|error| anyhow::anyhow!("板块涨跌榜 Gateway 失败: {error}"))?;
        let evidence = batch.evidence().clone();
        let records = match batch {
            GatewayBatch::Available { records, .. } => records,
            GatewayBatch::VerifiedEmpty(_) => {
                anyhow::bail!(
                    "板块涨跌榜为来源确认空批次 provider={:?} observed_at={} batch_id={}",
                    evidence.provider,
                    evidence.observed_at,
                    evidence.batch_id
                )
            }
        };
        let mut sectors_data = records
            .into_iter()
            .map(|record| {
                let change_pct = record.return_pct.ok_or_else(|| {
                    anyhow::anyhow!(
                        "板块 {}({}) 缺少当日涨跌幅 provider={:?} observed_at={} batch_id={}",
                        record.name,
                        record.code,
                        evidence.provider,
                        evidence.observed_at,
                        evidence.batch_id
                    )
                })?;
                if !change_pct.is_finite() || change_pct.abs() > 20.0 {
                    anyhow::bail!(
                        "板块 {}({}) 当日涨跌幅非法: {}%",
                        record.name,
                        record.code,
                        change_pct
                    );
                }
                Ok((record.name, change_pct))
            })
            .collect::<Result<Vec<_>>>()?;
        sectors_data.sort_by(|left, right| {
            right
                .1
                .total_cmp(&left.1)
                .then_with(|| left.0.cmp(&right.0))
        });

        // 领涨板块（前3）
        for (name, change_pct) in sectors_data.iter().take(3) {
            overview.top_sectors.push(SectorInfo {
                name: name.clone(),
                change_pct: *change_pct,
            });
        }

        // 领跌板块（后3）
        for (name, change_pct) in sectors_data.iter().rev().take(3) {
            overview.bottom_sectors.push(SectorInfo {
                name: name.clone(),
                change_pct: *change_pct,
            });
        }

        info!(
            "[大盘] Gateway 板块数据 {} 条, 领涨:{} 领跌:{} provider={:?} observed_at={} batch_id={}",
            sectors_data.len(),
            overview.top_sectors.len(),
            overview.bottom_sectors.len(),
            evidence.provider,
            evidence.observed_at,
            evidence.batch_id
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 静态检查：非测试代码中不能出现伪随机作为板块数据源。
    /// 修复：QUANT_ANALYST_REVIEW §1.3
    ///
    /// 实现思路：把 `mod tests {` 之前的所有源码单独拿出来检查，
    /// 避免本测试模块自身的字符串污染检查。
    #[test]
    fn no_mock_random_in_sector_data() {
        let src = include_str!("statistics.rs");
        let test_mod_start = src.find("#[cfg(test)]\nmod tests {").unwrap_or(src.len());
        let production_src = &src[..test_mod_start];
        // 真正禁止的伪随机模式
        assert!(
            !production_src.contains("name.len() % 3"),
            "禁止使用 name.len() 模运算等伪随机作为板块数据源（AGENTS.md 红线）"
        );
        assert!(
            !production_src.contains("sectors_template"),
            "禁止在生产路径使用硬编码 sectors_template（AGENTS.md 红线）"
        );
    }

    #[test]
    fn whole_market_statistics_fail_explicitly_without_released_contract() {
        let analyzer = MarketAnalyzer::new(None).unwrap();
        let mut overview = MarketOverview::new("2026-07-18".to_string());
        let error = analyzer
            .get_market_statistics(&mut overview)
            .unwrap_err()
            .to_string();
        assert!(error.contains("Magic 数据契约"));
        assert_eq!(overview.up_count, 0);
        assert!(overview.top_stocks.is_empty());
    }
}
