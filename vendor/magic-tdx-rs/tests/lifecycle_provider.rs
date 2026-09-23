use magic_market_core::{
    AssetClass, CorporateActionRequest, CorporateActions, Exchange, InstrumentId,
};
use magic_tdx_rs::{TdxDirectClient, TdxHqClient, TdxSmartClient};

#[test]
fn every_blocking_tdx_client_exposes_the_normalized_lifecycle_contract() {
    fn accepts<P: CorporateActions<Error = magic_tdx_rs::TdxError>>(_: &P) {}
    accepts(&TdxHqClient::new());
    accepts(&TdxSmartClient::new());
    accepts(&TdxDirectClient::new("127.0.0.1", 7709, 1.0));
}

#[test]
fn beijing_lifecycle_is_explicitly_unsupported_before_transport() {
    let instrument = InstrumentId::new(Exchange::Beijing, "920118", AssetClass::Equity).unwrap();
    let request = CorporateActionRequest::new(instrument);
    let client = TdxHqClient::new();
    let error = client.corporate_actions(&request).unwrap_err();
    assert!(matches!(error, magic_tdx_rs::TdxError::Unsupported(_)));
}
