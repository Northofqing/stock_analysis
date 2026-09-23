use magic_market_core::{AssetClass, BarInterval, BarsRequest, Exchange, InstrumentId};
#[test]
fn service_facade_constructs_without_network() {
    let service = magic_tdx_rs::TdxService::new();
    let id = InstrumentId::new(Exchange::Shanghai, "600519", AssetClass::Equity).unwrap();
    let request = BarsRequest::new(id, BarInterval::Day, 1).unwrap();
    assert_eq!(request.limit(), 1);
    let _ = service.client();
}

#[test]
fn unsupported_p0_capabilities_are_explicit() {
    let service = magic_tdx_rs::TdxService::new();
    let result = service.money_flows(&[]);
    let message = result.as_ref().unwrap_err().to_string();
    assert!(matches!(
        result,
        Err(magic_tdx_rs::TdxError::Unsupported(_))
    ));
    assert!(message.contains("main/net inflow"));
    let result = service.auction_snapshots(&[]);
    let message = result.as_ref().unwrap_err().to_string();
    assert!(matches!(
        result,
        Err(magic_tdx_rs::TdxError::Unsupported(_))
    ));
    assert!(message.contains("matched/unmatched"));
}
