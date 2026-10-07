//! Missing flow/volume observations stay absent through detection and rendering.
use stock_analysis::monitor::{
    alert::{format_alert, format_t1_alert},
    detector::{AlertCategory, Detector, DetectorConfig, StockSnapshot},
};

fn snapshot() -> StockSnapshot {
    StockSnapshot {
        code: "TEST_CODE_MISSING".into(),
        name: "TEST_CODE_NAME".into(),
        price: 10.98,
        change_pct: 9.8,
        volume_ratio: None,
        main_net_yi: None,
        limit_up_price: Some(11.0),
        was_limit_up: false,
        t1_locked: false,
    }
}

#[test]
fn missing_metrics_do_not_block_price_alert_or_fill_its_detail() {
    let detector = Detector::new(DetectorConfig::default());
    let events = detector.scan_stock(&snapshot());
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.category, AlertCategory::LimitUp);
    assert_eq!(event.detail.volume_ratio, None);
    assert_eq!(event.detail.main_flow_yi, None);
    let text = format_alert(event);
    assert!(text.contains("TEST_CODE_NAME"));
    assert!(!text.contains("量比"), "{text}");
    assert!(!text.contains("主力净"), "{text}");
}

#[test]
fn observed_zero_is_distinct_from_missing() {
    let mut stock = snapshot();
    stock.volume_ratio = Some(0.0);
    stock.main_net_yi = Some(0.0);
    let events = Detector::new(DetectorConfig::default()).scan_stock(&stock);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].detail.volume_ratio, Some(0.0));
    assert_eq!(events[0].detail.main_flow_yi, Some(0.0));
    let text = format_alert(&events[0]);
    assert!(text.contains("主力净流入：+0.00亿"), "{text}");
    assert!(text.contains("量比：0.0"), "{text}");
}

#[test]
fn available_negative_flow_still_triggers_without_volume_observation() {
    let mut stock = snapshot();
    stock.change_pct = -2.0;
    stock.main_net_yi = Some(-0.5);
    let events = Detector::new(DetectorConfig::default()).scan_stock(&stock);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].category, AlertCategory::MainOutflow);
    assert_eq!(events[0].detail.main_flow_yi, Some(-0.5));
    assert_eq!(events[0].detail.volume_ratio, None);
}

#[test]
fn available_volume_still_triggers_without_flow_observation() {
    let mut stock = snapshot();
    stock.change_pct = 6.0;
    stock.volume_ratio = Some(4.0);
    let events = Detector::new(DetectorConfig::default()).scan_stock(&stock);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].category, AlertCategory::VolBurst);
    assert_eq!(events[0].detail.volume_ratio, Some(4.0));
    assert_eq!(events[0].detail.main_flow_yi, None);
}

#[test]
fn missing_t1_price_is_omitted_and_real_price_is_preserved() {
    let mut stock = snapshot();
    stock.change_pct = -10.0;
    stock.t1_locked = true;
    let mut event = Detector::new(DetectorConfig::default())
        .check_limit_down(&stock)
        .unwrap();
    let present = format_t1_alert(&event, "TEST_CODE_SELL_DATE");
    assert!(present.contains("现价：10.98"), "{present}");
    event.detail.price = None;
    let missing = format_t1_alert(&event, "TEST_CODE_SELL_DATE");
    assert!(!missing.contains("现价"), "{missing}");
    assert!(!missing.contains("0.00"), "{missing}");
    assert!(missing.contains("TEST_CODE_SELL_DATE"), "{missing}");
}
