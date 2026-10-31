#[test]
fn tdx_advertises_all_core_data_families() {
    let c = magic_tdx_rs::TdxHqClient::capabilities();
    assert!(
        c.quotes
            && c.bars
            && c.minute
            && c.trades
            && c.fundamentals
            && c.corporate_actions
            && c.blocks
    );
    assert!(!c.money_flow && c.order_book && !c.auction && c.security_metadata);
}

#[test]
fn blocking_and_smart_clients_expose_order_book_contract() {
    fn assert_impl<T: magic_market_core::OrderBooks>() {}
    assert_impl::<magic_tdx_rs::TdxHqClient>();
    assert_impl::<magic_tdx_rs::TdxSmartClient>();
}

#[test]
fn unsupported_p0_traits_are_still_callable() {
    fn assert_impl<T: magic_market_core::MoneyFlows + magic_market_core::Auctions>() {}
    assert_impl::<magic_tdx_rs::TdxHqClient>();
    assert_impl::<magic_tdx_rs::TdxSmartClient>();
}
