//! Stable facade for fund/ETF market data.

use crate::fund::client::TdxHqFundClient;
use crate::fund::types::{FundBar, FundFinanceInfo, FundInfo, FundQuote, FundXdXrInfo};
use crate::TdxError;

/// Fund service exposing the upstream fund operation set.
pub struct FundService {
    client: TdxHqFundClient,
}
impl FundService {
    /// Creates a disconnected fund service.
    pub fn new() -> Self {
        Self {
            client: TdxHqFundClient::new(),
        }
    }
    /// Accesses the underlying client for connection management.
    pub fn client(&self) -> &TdxHqFundClient {
        &self.client
    }
    pub fn list(&self, market: u8) -> Result<Vec<FundInfo>, TdxError> {
        self.client.get_fund_list(market)
    }
    pub fn bars(
        &self,
        category: u8,
        market: u8,
        code: &str,
        start: u32,
        count: u16,
    ) -> Result<Vec<FundBar>, TdxError> {
        self.client
            .get_fund_bars(category, market, code, start, count)
    }
    pub fn quotes(&self, funds: &[(u8, &str)]) -> Result<Vec<FundQuote>, TdxError> {
        self.client.get_fund_quotes(funds)
    }
    pub fn corporate_actions(&self, market: u8, code: &str) -> Result<Vec<FundXdXrInfo>, TdxError> {
        self.client.get_fund_xdxr_info(market, code)
    }
    pub fn finance(&self, market: u8, code: &str) -> Result<FundFinanceInfo, TdxError> {
        self.client.get_fund_finance_info(market, code)
    }
}
impl Default for FundService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::constants::KLINE_DAILY;

    #[test]
    fn fund_facade_rejects_non_fund_codes_before_transport() {
        let service = FundService::default();
        let _ = service.client();
        assert!(service.bars(KLINE_DAILY, 1, "600001", 0, 5).is_err());
        assert!(service.quotes(&[(1, "600001")]).is_err());
        assert!(service.corporate_actions(1, "600001").is_err());
        assert!(service.finance(1, "600001").is_err());
    }
}
