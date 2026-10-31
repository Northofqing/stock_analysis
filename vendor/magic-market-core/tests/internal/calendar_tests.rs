use super::*;

#[test]
fn calendar_requests_revalidate_bounds() {
    assert!(EconomicCalendarRequest::new(PositiveU32::new(20).unwrap()).is_ok());
    assert!(EconomicCalendarRequest::new(PositiveU32::new(21).unwrap()).is_err());
    assert!(FuturesDeliveryRequest::new(
        PositiveU32::new(2026).unwrap(),
        PositiveU32::new(13).unwrap()
    )
    .is_err());
    assert!(serde_json::from_str::<FuturesDeliveryRequest>(r#"{"year":2026,"month":13}"#).is_err());
}

#[test]
fn economic_calendar_request_preserves_optional_country() {
    let request = EconomicCalendarRequest::new(PositiveU32::new(7).unwrap())
        .unwrap()
        .with_country("China")
        .unwrap();
    assert_eq!(request.limit().get(), 7);
    assert_eq!(request.country().unwrap().as_str(), "China");

    let restored: EconomicCalendarRequest =
        serde_json::from_str(r#"{"limit":7,"country":"China"}"#).unwrap();
    assert_eq!(restored, request);

    let without_country: EconomicCalendarRequest =
        serde_json::from_str(r#"{"limit":1,"country":null}"#).unwrap();
    assert!(without_country.country().is_none());
    assert!(
        serde_json::from_str::<EconomicCalendarRequest>(r#"{"limit":1,"country":"   "}"#).is_err()
    );
}

#[test]
fn futures_delivery_request_revalidates_year_and_exposes_month() {
    let request = FuturesDeliveryRequest::new(
        PositiveU32::new(2026).unwrap(),
        PositiveU32::new(7).unwrap(),
    )
    .unwrap();
    assert_eq!(request.year().get(), 2026);
    assert_eq!(request.month().get(), 7);
    assert_eq!(
        serde_json::from_str::<FuturesDeliveryRequest>(r#"{"year":2026,"month":7}"#).unwrap(),
        request
    );
    assert!(FuturesDeliveryRequest::new(
        PositiveU32::new(1999).unwrap(),
        PositiveU32::new(1).unwrap()
    )
    .is_err());
    assert!(FuturesDeliveryRequest::new(
        PositiveU32::new(10_000).unwrap(),
        PositiveU32::new(1).unwrap()
    )
    .is_err());
}

#[test]
fn calendar_records_expose_source_identity() {
    let evidence =
        SourceEvidence::new(crate::ProviderId::Cffex, "observed", "calendar-batch").unwrap();
    let event = EconomicEvent {
        event_id: NonEmptyText::new("event-1").unwrap(),
        indicator_id: PositiveU32::new(1).unwrap(),
        country: NonEmptyText::new("中国").unwrap(),
        name: NonEmptyText::new("工业企业利润").unwrap(),
        period: None,
        scheduled_at: NonEmptyText::new("2026-07-25T09:30:00+08:00").unwrap(),
        released_at: NonEmptyText::new("2026-07-25T09:30:01+08:00").unwrap(),
        previous: None,
        consensus: None,
        actual: None,
        revised: None,
        unit: None,
        importance: PositiveU32::new(1).unwrap(),
        impact: None,
        evidence: evidence.clone(),
    };
    assert_eq!(event.provider_id(), crate::ProviderId::Cffex);
    assert_eq!(event.evidence_batch_id(), "calendar-batch");

    let delivery = FuturesDeliveryEvent {
        product: FuturesProduct::If,
        contract_code: NonEmptyText::new("IF2607").unwrap(),
        last_trading_date: None,
        delivery_date: IsoDate::new("2026-07-17").unwrap(),
        method: FuturesDeliveryMethod::NotProvided,
        notice_url: HttpsUrl::new("https://www.cffex.com.cn/notice.html").unwrap(),
        evidence,
    };
    assert_eq!(delivery.provider_id(), crate::ProviderId::Cffex);
    assert_eq!(delivery.evidence_batch_id(), "calendar-batch");
    let restored: FuturesDeliveryEvent =
        serde_json::from_str(&serde_json::to_string(&delivery).unwrap()).unwrap();
    assert_eq!(restored, delivery);
    assert_eq!(restored.method, FuturesDeliveryMethod::NotProvided);
    assert!(restored.last_trading_date.is_none());
}
