use magic_market_core::{
    AssetClass, ContractMonth, Exchange, FiniteNumber, InstrumentId, IsoDate, Money, NonEmptyText,
    OptionContract, OptionGreeks, OptionKind, OptionQuote, Price, ProviderId, Quantity, Ratio,
    RatioUnit, SourceEvidence, SourcedRecord,
};

fn underlying() -> InstrumentId {
    InstrumentId::new(Exchange::Shanghai, "510050", AssetClass::Fund).unwrap()
}

fn evidence() -> SourceEvidence {
    SourceEvidence::new(ProviderId::Sina, "observed", "option").unwrap()
}

fn contract() -> OptionContract {
    OptionContract {
        contract_code: NonEmptyText::new("10000001").unwrap(),
        underlying: underlying(),
        expiry_month: ContractMonth::new("2026-08").unwrap(),
        expiry: Some(IsoDate::new("2026-08-26").unwrap()),
        kind: OptionKind::Call,
        strike: Some(Price::new(3.0).unwrap()),
        evidence: evidence(),
    }
}

#[test]
fn option_contract_quote_and_greeks_are_distinct_records() {
    let quote = OptionQuote {
        contract_code: contract().contract_code.clone(),
        name: Some(NonEmptyText::new("50ETF购8月3000").unwrap()),
        bid: Some(Price::new(0.1).unwrap()),
        bid_quantity: Some(Quantity::new(10.0).unwrap()),
        ask: Some(Price::new(0.11).unwrap()),
        ask_quantity: Some(Quantity::new(12.0).unwrap()),
        last: None,
        previous_close: Some(Price::new(0.09).unwrap()),
        open: Some(Price::new(0.1).unwrap()),
        high: Some(Price::new(0.12).unwrap()),
        low: Some(Price::new(0.08).unwrap()),
        upper_limit: Some(Price::new(0.5).unwrap()),
        lower_limit: Some(Price::new(0.001).unwrap()),
        strike: Some(Price::new(3.0).unwrap()),
        volume: Some(Quantity::new(0.0).unwrap()),
        open_interest: None,
        amount: Some(Money::new(0.0).unwrap()),
        change: Some(Ratio::new(1.25, RatioUnit::Percent).unwrap()),
        amplitude: Some(Ratio::new(4.5, RatioUnit::Percent).unwrap()),
        quote_at: None,
        evidence: evidence(),
    };
    let greeks = OptionGreeks {
        contract_code: contract().contract_code.clone(),
        name: Some(NonEmptyText::new("50ETF购8月3000").unwrap()),
        volume: Some(Quantity::new(20.0).unwrap()),
        delta: Some(FiniteNumber::new(0.5).unwrap()),
        gamma: Some(FiniteNumber::new(0.1).unwrap()),
        theta: Some(FiniteNumber::new(-0.01).unwrap()),
        vega: Some(FiniteNumber::new(0.2).unwrap()),
        rho: None,
        implied_volatility: Some(FiniteNumber::new(0.25).unwrap()),
        high: Some(Price::new(0.12).unwrap()),
        low: Some(Price::new(0.08).unwrap()),
        trade_code: Some(NonEmptyText::new("510050C2608M03000").unwrap()),
        strike: Some(Price::new(3.0).unwrap()),
        last: Some(Price::new(0.1).unwrap()),
        theoretical_price: Some(Price::new(0.105).unwrap()),
        evidence: evidence(),
    };

    assert_eq!(quote.provider_id(), ProviderId::Sina);
    assert_eq!(quote.bid_quantity.unwrap().get(), 10.0);
    assert_eq!(greeks.delta.unwrap().get(), 0.5);
    assert_eq!(
        serde_json::from_str::<OptionGreeks>(&serde_json::to_string(&greeks).unwrap()).unwrap(),
        greeks
    );
}

#[test]
fn option_asset_identity_is_available_without_weakening_underlying() {
    let option_id = InstrumentId::new(Exchange::Shanghai, "10000001", AssetClass::Option).unwrap();
    assert_eq!(option_id.asset_class(), AssetClass::Option);
    assert_eq!(contract().underlying.asset_class(), AssetClass::Fund);
}

#[test]
fn contract_month_is_checked_and_exact_expiry_and_strike_can_be_absent() {
    let contract = OptionContract {
        contract_code: NonEmptyText::new("10000002").unwrap(),
        underlying: underlying(),
        expiry_month: ContractMonth::new("2026-09").unwrap(),
        expiry: None,
        kind: OptionKind::Put,
        strike: None,
        evidence: evidence(),
    };
    assert_eq!(contract.expiry_month.as_str(), "2026-09");
    assert!(contract.expiry.is_none());
    assert!(contract.strike.is_none());
    assert!(ContractMonth::new("202608").is_err());
    assert!(ContractMonth::new("2026/08").is_err());
    assert!(ContractMonth::new("202X-08").is_err());
    assert!(ContractMonth::new("2026-13").is_err());
    assert!(serde_json::from_str::<ContractMonth>("\"2026-00\"").is_err());
    assert_eq!(
        serde_json::from_str::<OptionContract>(&serde_json::to_string(&contract).unwrap()).unwrap(),
        contract
    );
}

#[test]
fn option_contract_deserialization_rejects_expiry_outside_contract_month() {
    let mut value = serde_json::to_value(contract()).unwrap();
    value["expiry"] = serde_json::json!("2026-09-01");
    assert!(serde_json::from_value::<OptionContract>(value).is_err());
}

#[test]
fn option_quote_deserialization_rechecks_cross_field_invariants() {
    let quote = OptionQuote {
        contract_code: contract().contract_code.clone(),
        name: None,
        bid: Some(Price::new(0.1).unwrap()),
        bid_quantity: Some(Quantity::new(10.0).unwrap()),
        ask: Some(Price::new(0.11).unwrap()),
        ask_quantity: Some(Quantity::new(12.0).unwrap()),
        last: Some(Price::new(0.105).unwrap()),
        previous_close: Some(Price::new(0.09).unwrap()),
        open: Some(Price::new(0.1).unwrap()),
        high: Some(Price::new(0.12).unwrap()),
        low: Some(Price::new(0.08).unwrap()),
        upper_limit: Some(Price::new(0.5).unwrap()),
        lower_limit: Some(Price::new(0.001).unwrap()),
        strike: Some(Price::new(3.0).unwrap()),
        volume: Some(Quantity::new(20.0).unwrap()),
        open_interest: Some(Quantity::new(30.0).unwrap()),
        amount: Some(Money::new(100.0).unwrap()),
        change: Some(Ratio::new(1.0, RatioUnit::Percent).unwrap()),
        amplitude: Some(Ratio::new(2.0, RatioUnit::Percent).unwrap()),
        quote_at: Some(NonEmptyText::new("2026-08-03T14:30:00+08:00").unwrap()),
        evidence: evidence(),
    };
    let valid = serde_json::to_value(&quote).unwrap();
    assert_eq!(
        serde_json::from_value::<OptionQuote>(valid.clone()).unwrap(),
        quote
    );

    for (field, invalid) in [
        ("bid_quantity", serde_json::Value::Null),
        ("ask", serde_json::json!(0.09)),
        ("high", serde_json::json!(0.07)),
        ("upper_limit", serde_json::json!(0.0005)),
        ("amount", serde_json::json!(-1.0)),
        ("amplitude", serde_json::json!(-0.01)),
        ("quote_at", serde_json::json!("2026-02-30T14:30:00+08:00")),
        ("quote_at", serde_json::json!("2026-08-03T14:30:00+24:00")),
        ("quote_at", serde_json::json!("2026-08-03T14:30:00+08:60")),
    ] {
        let mut candidate = valid.clone();
        candidate[field] = invalid;
        assert!(
            serde_json::from_value::<OptionQuote>(candidate).is_err(),
            "{field} bypassed option quote validation"
        );
    }

    let mut unicode = valid;
    unicode["quote_at"] = serde_json::json!("2026-07-23T一一一Z");
    let result = std::panic::catch_unwind(|| serde_json::from_value::<OptionQuote>(unicode));
    assert!(result.is_ok(), "malformed Unicode timestamp panicked");
    assert!(result.unwrap().is_err());
}

#[test]
fn option_greeks_deserialization_rechecks_source_domains() {
    let greeks = OptionGreeks {
        contract_code: contract().contract_code.clone(),
        name: None,
        volume: Some(Quantity::new(20.0).unwrap()),
        delta: Some(FiniteNumber::new(0.5).unwrap()),
        gamma: Some(FiniteNumber::new(0.1).unwrap()),
        theta: Some(FiniteNumber::new(-0.01).unwrap()),
        vega: Some(FiniteNumber::new(0.2).unwrap()),
        rho: None,
        implied_volatility: Some(FiniteNumber::new(0.25).unwrap()),
        high: Some(Price::new(0.12).unwrap()),
        low: Some(Price::new(0.08).unwrap()),
        trade_code: None,
        strike: Some(Price::new(3.0).unwrap()),
        last: Some(Price::new(0.1).unwrap()),
        theoretical_price: Some(Price::new(0.105).unwrap()),
        evidence: evidence(),
    };
    let valid = serde_json::to_value(&greeks).unwrap();
    assert_eq!(
        serde_json::from_value::<OptionGreeks>(valid.clone()).unwrap(),
        greeks
    );

    for (field, invalid) in [
        ("delta", serde_json::json!(1.01)),
        ("gamma", serde_json::json!(-0.01)),
        ("vega", serde_json::json!(-0.01)),
        ("implied_volatility", serde_json::json!(-0.01)),
        ("high", serde_json::json!(0.07)),
    ] {
        let mut candidate = valid.clone();
        candidate[field] = invalid;
        assert!(
            serde_json::from_value::<OptionGreeks>(candidate).is_err(),
            "{field} bypassed option Greek validation"
        );
    }
}
