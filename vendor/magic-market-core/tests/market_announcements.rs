use magic_market_core::{
    DataBatch, IsoDate, MarketAnnouncementRequest, MarketAnnouncements, PositiveU32,
};

#[test]
fn market_announcement_request_is_bounded_and_requires_an_ordered_range() {
    let start = IsoDate::new("2026-07-24").unwrap();
    let end = IsoDate::new("2026-07-25").unwrap();
    let request =
        MarketAnnouncementRequest::new(start.clone(), end.clone(), PositiveU32::new(300).unwrap())
            .unwrap();

    assert_eq!(request.start(), &start);
    assert_eq!(request.end(), &end);
    assert_eq!(request.limit().get(), 300);
    assert!(MarketAnnouncementRequest::new(
        start.clone(),
        end.clone(),
        PositiveU32::new(301).unwrap()
    )
    .is_err());
    assert!(MarketAnnouncementRequest::new(end, start, PositiveU32::new(1).unwrap()).is_err());

    let mut wire = serde_json::to_value(&request).unwrap();
    wire["limit"] = serde_json::json!(301);
    assert!(serde_json::from_value::<MarketAnnouncementRequest>(wire).is_err());
}

#[allow(dead_code)]
fn provider_contract<P>(
    provider: &P,
    request: &MarketAnnouncementRequest,
) -> Result<DataBatch<magic_market_core::Announcement>, P::Error>
where
    P: MarketAnnouncements,
{
    provider.market_announcements(request)
}
