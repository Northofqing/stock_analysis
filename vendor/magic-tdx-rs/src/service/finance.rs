//! Stable facade for realtime and report-file finance data.

use crate::net::finance_client::{GpcwFileInfo, TdxFinanceClient};
use crate::protocol::types::{FinanceInfo, XdXrInfo};
use crate::reader::financial::FinancialRecord;
use crate::TdxError;
use magic_market_core::{CorporateActionRequest, CorporateActionResponse};
use std::collections::HashMap;

/// Finance service, including the 34-field realtime and 45-field report APIs.
pub struct FinanceService {
    client: TdxFinanceClient,
}
impl FinanceService {
    pub fn new(ip: &str, port: u16, timeout: Option<f64>) -> Self {
        Self {
            client: TdxFinanceClient::new(ip, port, timeout),
        }
    }
    pub fn client(&self) -> &TdxFinanceClient {
        &self.client
    }
    pub fn info(&self, market: u8, code: &str) -> Result<FinanceInfo, TdxError> {
        self.client.get_finance_info(market, code)
    }
    pub fn corporate_actions(&self, market: u8, code: &str) -> Result<Vec<XdXrInfo>, TdxError> {
        self.client.get_xdxr_info(market, code)
    }
    /// Fetches complete, provider-neutral corporate-action history with evidence.
    pub fn normalized_corporate_actions(
        &self,
        request: &CorporateActionRequest,
    ) -> Result<CorporateActionResponse, TdxError> {
        let admission_as_of = crate::adapter::current_corporate_action_admission_date()?;
        crate::adapter::validate_corporate_action_request(request, &admission_as_of)?;
        let records = self.client.get_xdxr_info(
            crate::adapter::market(request.instrument())?,
            request.instrument().code(),
        )?;
        let batch = crate::adapter::normalize_corporate_actions(
            "tdx-finance",
            request,
            records,
            &admission_as_of,
        )?;
        crate::adapter::corporate_action_response(request, batch, admission_as_of)
    }
    pub fn files(&self) -> Result<Vec<GpcwFileInfo>, TdxError> {
        self.client.get_financial_list()
    }
    pub fn report(&self, filename: &str, filesize: u32) -> Result<Vec<u8>, TdxError> {
        self.client.get_report_file_by_size(filename, filesize)
    }
    pub fn records(&self, filename: &str, filesize: u32) -> Result<Vec<FinancialRecord>, TdxError> {
        self.client.get_financial_data(filename, filesize)
    }
    pub fn indicators(
        &self,
        filename: &str,
        filesize: u32,
        code: &str,
    ) -> Result<HashMap<&'static str, f64>, TdxError> {
        self.client.get_finance_indicators(filename, filesize, code)
    }
    pub fn labeled_indicators(
        &self,
        filename: &str,
        filesize: u32,
        code: &str,
    ) -> Result<Vec<(&'static str, &'static str, f64)>, TdxError> {
        self.client
            .get_finance_indicators_labeled(filename, filesize, code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finance_facade_preserves_offline_failures() {
        let service = FinanceService::new("127.0.0.1", 1, Some(0.01));
        let _ = service.client();
        assert!(service.info(1, "600001").is_err());
        assert!(service.corporate_actions(1, "600001").is_err());
        assert!(service.files().is_err());
        assert!(service.report("tdxfin/gpcw.txt", 1).is_err());
        assert!(service.records("../bad", 1).is_err());
        assert!(service.indicators("../bad", 1, "600001").is_err());
        assert!(service.labeled_indicators("../bad", 1, "600001").is_err());
    }
}
