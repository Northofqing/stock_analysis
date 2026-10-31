use super::*;

#[test]
fn calendar_conversion_rejects_extreme_day_arithmetic() {
    assert!(civil_from_days(i64::MAX).is_err());
    assert!(civil_from_days(i64::MIN).is_err());
}
