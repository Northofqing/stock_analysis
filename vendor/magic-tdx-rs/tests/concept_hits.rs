use magic_market_core::{AssetClass, ConceptHits, Exchange, InstrumentId};
use magic_tdx_rs::{service::blocks::BlockService, TdxError};

#[test]
fn block_service_satisfies_the_concept_provider_contract() {
    fn assert_provider<T: ConceptHits<Error = TdxError>>() {}
    assert_provider::<BlockService>();
}

#[test]
fn invalid_or_beijing_requests_fail_before_network_io() {
    let service = BlockService::with_default("127.0.0.1");
    assert!(service.concept_hits(&[]).is_err());

    let beijing = InstrumentId::new(Exchange::Beijing, "920118", AssetClass::Equity).unwrap();
    assert!(matches!(
        service.concept_hits(&[beijing]),
        Err(TdxError::Unsupported(_))
    ));
}
