//! indices（从 market_analyzer.rs 拆分）

use anyhow::Result;
use log::info;

use crate::data_gateway::IndexDataGateway;
use crate::market_data::MarketIndex;

use super::MarketAnalyzer;

impl MarketAnalyzer {
    /// 获取主要指数实时行情
    pub(super) fn get_main_indices(&self) -> Result<Vec<MarketIndex>> {
        info!("[大盘] 获取主要指数实时行情...");

        let codes = Self::MAIN_INDICES_LIST
            .iter()
            .map(|(code, _)| (*code).to_owned())
            .collect::<Vec<_>>();
        let batch = IndexDataGateway::new().realtime_quotes(&codes)?;
        let indices = batch
            .records()
            .iter()
            .map(|quote| {
                let mut index = MarketIndex {
                    code: quote.code.clone(),
                    name: quote.name.clone(),
                    current: quote.current,
                    change: quote.change,
                    change_pct: quote.change_percent,
                    open: Some(quote.open),
                    high: Some(quote.high),
                    low: Some(quote.low),
                    prev_close: quote.previous_close,
                    volume: Some(quote.volume),
                    amount: Some(quote.amount),
                    amplitude: None,
                };
                index.calculate_amplitude();
                index
            })
            .collect::<Vec<_>>();
        if indices.len() != Self::MAIN_INDICES_LIST.len() {
            anyhow::bail!(
                "统一指数 Gateway 返回数量不完整: expected={} actual={}",
                Self::MAIN_INDICES_LIST.len(),
                indices.len()
            );
        }

        info!("[大盘] 获取到 {} 个指数行情", indices.len());
        Ok(indices)
    }
}
