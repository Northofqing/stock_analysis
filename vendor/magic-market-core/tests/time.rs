use magic_market_core::{
    unix_seconds_to_china_rfc3339, unix_seconds_to_fixed_offset_rfc3339, ClockTime,
    EvidenceTimestamp,
};

#[test]
fn fixed_offset_conversion_covers_epoch_rollover_and_leap_day() {
    assert_eq!(
        unix_seconds_to_china_rfc3339(0).unwrap(),
        "1970-01-01T08:00:00+08:00"
    );
    assert_eq!(
        unix_seconds_to_china_rfc3339(-1).unwrap(),
        "1970-01-01T07:59:59+08:00"
    );
    assert_eq!(
        unix_seconds_to_china_rfc3339(1_709_222_400).unwrap(),
        "2024-03-01T00:00:00+08:00"
    );
    let leap_day = unix_seconds_to_fixed_offset_rfc3339(1_709_164_800, 0).unwrap();
    assert_eq!(leap_day, "2024-02-29T00:00:00+00:00");
    assert!(EvidenceTimestamp::parse(&leap_day).is_ok());
}

#[test]
fn fixed_offset_conversion_rejects_invalid_offsets_and_extremes() {
    assert!(unix_seconds_to_fixed_offset_rfc3339(0, 30).is_err());
    assert!(unix_seconds_to_fixed_offset_rfc3339(0, 86_400).is_err());
    assert!(unix_seconds_to_china_rfc3339(i64::MAX).is_err());
    assert!(unix_seconds_to_china_rfc3339(i64::MIN).is_err());
}

#[test]
fn strict_clock_parsing_and_ordering_are_unambiguous() {
    let close = ClockTime::parse("15:35:00").unwrap();
    assert_eq!(close.to_string(), "15:35:00");
    assert_eq!(close.seconds_since_midnight(), 56_100);
    assert!(ClockTime::parse("15:34:59").unwrap() < close);
    assert!(ClockTime::parse("23:59:59").unwrap() > close);
    for invalid in [
        "1:35:00",
        "15:35",
        "15:35:00.0",
        "15-35-00",
        "24:00:00",
        "15:60:00",
        "15:35:60",
        "１５:３５:００",
    ] {
        assert!(ClockTime::parse(invalid).is_err(), "{invalid:?}");
    }
}
