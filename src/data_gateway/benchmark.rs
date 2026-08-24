use std::collections::BTreeSet;

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, TimeZone, Timelike};
use serde::{Deserialize, Serialize};

pub const HS300_CANONICAL: &str = "sh000300";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BenchmarkGranularity {
    Daily,
    Minute1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BenchmarkRange {
    Daily {
        from: NaiveDate,
        to: NaiveDate,
    },
    Minute1 {
        from: DateTime<FixedOffset>,
        to: DateTime<FixedOffset>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkRequest {
    pub instrument: String,
    pub range: BenchmarkRange,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum BenchmarkBarTime {
    Daily(NaiveDate),
    MinuteEnd(DateTime<FixedOffset>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkBar {
    pub at: BenchmarkBarTime,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: Option<f64>,
    pub amount: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BenchmarkUnsupported {
    UnsupportedInstrument,
    TestIdentityRejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BenchmarkError {
    Unsupported(BenchmarkUnsupported),
    Unavailable { code: &'static str, retryable: bool },
    FailedIntegrity { code: &'static str },
}

#[derive(Debug, Clone)]
pub struct BenchmarkRegistry {
    allowed_instruments: BTreeSet<String>,
    accepts_test_identities: bool,
}

impl BenchmarkRegistry {
    #[must_use]
    pub fn production_default() -> Self {
        Self {
            allowed_instruments: [HS300_CANONICAL.to_owned()].into_iter().collect(),
            accepts_test_identities: false,
        }
    }

    #[cfg(test)]
    fn test_only(instruments: impl IntoIterator<Item = &'static str>) -> Self {
        Self {
            allowed_instruments: instruments.into_iter().map(str::to_owned).collect(),
            accepts_test_identities: true,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum BenchmarkAdmissionCoverage<'a> {
    Daily {
        authoritative_trading_days: &'a [NaiveDate],
    },
    Minute1,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdmittedBenchmarkBatch {
    request: BenchmarkRequest,
    bars: Vec<BenchmarkBar>,
}

impl AdmittedBenchmarkBatch {
    #[must_use]
    pub fn request(&self) -> &BenchmarkRequest {
        &self.request
    }

    #[must_use]
    pub fn bars(&self) -> &[BenchmarkBar] {
        &self.bars
    }

    #[must_use]
    pub fn into_bars(self) -> Vec<BenchmarkBar> {
        self.bars
    }

    #[must_use]
    pub fn into_parts(self) -> (BenchmarkRequest, Vec<BenchmarkBar>) {
        (self.request, self.bars)
    }
}

pub fn admit_benchmark_batch(
    registry: &BenchmarkRegistry,
    request: BenchmarkRequest,
    bars: Vec<BenchmarkBar>,
    coverage: BenchmarkAdmissionCoverage<'_>,
) -> Result<AdmittedBenchmarkBatch, BenchmarkError> {
    validate_instrument(registry, &request.instrument)?;
    validate_range(&request.range)?;

    if bars.is_empty() {
        return Err(BenchmarkError::Unavailable {
            code: "benchmark_batch_empty",
            retryable: true,
        });
    }

    for bar in &bars {
        validate_bar_values(bar)?;
        validate_bar_time(&request.range, &bar.at)?;
    }
    validate_strict_order(&bars)?;

    match (&request.range, coverage) {
        (
            BenchmarkRange::Daily { from, to },
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days,
            },
        ) => validate_daily_coverage(*from, *to, &bars, authoritative_trading_days)?,
        (BenchmarkRange::Minute1 { from, to }, BenchmarkAdmissionCoverage::Minute1) => {
            validate_minute_coverage(*from, *to, &bars)?
        }
        _ => return Err(failed_integrity("benchmark_coverage_kind_mismatch")),
    }

    Ok(AdmittedBenchmarkBatch { request, bars })
}

fn validate_instrument(
    registry: &BenchmarkRegistry,
    instrument: &str,
) -> Result<(), BenchmarkError> {
    if instrument.starts_with("TEST_CODE") && !registry.accepts_test_identities {
        return Err(BenchmarkError::Unsupported(
            BenchmarkUnsupported::TestIdentityRejected,
        ));
    }
    if !registry.allowed_instruments.contains(instrument) {
        return Err(BenchmarkError::Unsupported(
            BenchmarkUnsupported::UnsupportedInstrument,
        ));
    }
    Ok(())
}

fn validate_range(range: &BenchmarkRange) -> Result<(), BenchmarkError> {
    match range {
        BenchmarkRange::Daily { from, to } if from <= to => Ok(()),
        BenchmarkRange::Daily { .. } => Err(failed_integrity("benchmark_range_reversed")),
        BenchmarkRange::Minute1 { from, to } => {
            if from > to {
                return Err(failed_integrity("benchmark_range_reversed"));
            }
            if !is_shanghai_offset(from) || !is_shanghai_offset(to) {
                return Err(failed_integrity("benchmark_time_zone_invalid"));
            }
            if from.date_naive() != to.date_naive() {
                return Err(failed_integrity("benchmark_minute_range_crosses_day"));
            }
            if !is_continuous_auction_minute_end(*from) || !is_continuous_auction_minute_end(*to) {
                return Err(failed_integrity("benchmark_minute_range_off_grid"));
            }
            Ok(())
        }
    }
}

fn validate_bar_values(bar: &BenchmarkBar) -> Result<(), BenchmarkError> {
    let prices = [bar.open, bar.high, bar.low, bar.close];
    if prices
        .iter()
        .any(|price| !price.is_finite() || *price <= 0.0)
    {
        return Err(failed_integrity("benchmark_ohlc_not_positive_finite"));
    }
    if bar.low > bar.open || bar.low > bar.close || bar.open > bar.high || bar.close > bar.high {
        return Err(failed_integrity("benchmark_ohlc_inconsistent"));
    }
    for optional_value in [bar.volume, bar.amount].into_iter().flatten() {
        if !optional_value.is_finite() || optional_value < 0.0 {
            return Err(failed_integrity(
                "benchmark_turnover_not_finite_nonnegative",
            ));
        }
    }
    Ok(())
}

fn validate_bar_time(range: &BenchmarkRange, at: &BenchmarkBarTime) -> Result<(), BenchmarkError> {
    match (range, at) {
        (BenchmarkRange::Daily { from, to }, BenchmarkBarTime::Daily(date))
            if date >= from && date <= to =>
        {
            Ok(())
        }
        (BenchmarkRange::Daily { .. }, BenchmarkBarTime::Daily(_)) => {
            Err(failed_integrity("benchmark_bar_outside_range"))
        }
        (BenchmarkRange::Minute1 { from, to }, BenchmarkBarTime::MinuteEnd(at))
            if is_shanghai_offset(at)
                && is_continuous_auction_minute_end(*at)
                && at >= from
                && at <= to =>
        {
            Ok(())
        }
        (BenchmarkRange::Minute1 { .. }, BenchmarkBarTime::MinuteEnd(_)) => {
            Err(failed_integrity("benchmark_minute_bar_invalid"))
        }
        _ => Err(failed_integrity("benchmark_bar_granularity_mismatch")),
    }
}

fn validate_strict_order(bars: &[BenchmarkBar]) -> Result<(), BenchmarkError> {
    for pair in bars.windows(2) {
        let ordered = match (&pair[0].at, &pair[1].at) {
            (BenchmarkBarTime::Daily(previous), BenchmarkBarTime::Daily(current)) => {
                previous < current
            }
            (BenchmarkBarTime::MinuteEnd(previous), BenchmarkBarTime::MinuteEnd(current)) => {
                previous < current
            }
            _ => false,
        };
        if !ordered {
            return Err(failed_integrity("benchmark_bar_order_or_duplicate"));
        }
    }
    Ok(())
}

fn validate_daily_coverage(
    from: NaiveDate,
    to: NaiveDate,
    bars: &[BenchmarkBar],
    authoritative_trading_days: &[NaiveDate],
) -> Result<(), BenchmarkError> {
    if authoritative_trading_days
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
        || authoritative_trading_days
            .iter()
            .any(|date| *date < from || *date > to)
    {
        return Err(failed_integrity("benchmark_authoritative_days_invalid"));
    }
    let mut actual = Vec::with_capacity(bars.len());
    for bar in bars {
        match &bar.at {
            BenchmarkBarTime::Daily(date) => actual.push(*date),
            BenchmarkBarTime::MinuteEnd(_) => {
                return Err(failed_integrity("benchmark_bar_granularity_mismatch"));
            }
        }
    }
    if actual != authoritative_trading_days {
        return Err(failed_integrity("benchmark_daily_coverage_incomplete"));
    }
    Ok(())
}

fn validate_minute_coverage(
    from: DateTime<FixedOffset>,
    to: DateTime<FixedOffset>,
    bars: &[BenchmarkBar],
) -> Result<(), BenchmarkError> {
    let expected = continuous_auction_minute_ends(from, to);
    let mut actual = Vec::with_capacity(bars.len());
    for bar in bars {
        match &bar.at {
            BenchmarkBarTime::MinuteEnd(at) => actual.push(*at),
            BenchmarkBarTime::Daily(_) => {
                return Err(failed_integrity("benchmark_bar_granularity_mismatch"));
            }
        }
    }
    if actual != expected {
        return Err(failed_integrity("benchmark_minute_coverage_incomplete"));
    }
    Ok(())
}

fn continuous_auction_minute_ends(
    from: DateTime<FixedOffset>,
    to: DateTime<FixedOffset>,
) -> Vec<DateTime<FixedOffset>> {
    let mut expected = Vec::new();
    let mut cursor = from.naive_local();
    let end = to.naive_local();
    while cursor <= end {
        let at = from
            .offset()
            .from_local_datetime(&cursor)
            .single()
            .expect("fixed offsets have unambiguous local datetimes");
        if is_continuous_auction_minute_end(at) {
            expected.push(at);
        }
        cursor += Duration::minutes(1);
    }
    expected
}

fn is_shanghai_offset(at: &DateTime<FixedOffset>) -> bool {
    at.offset().local_minus_utc() == 8 * 60 * 60
}

fn is_continuous_auction_minute_end(at: DateTime<FixedOffset>) -> bool {
    if at.second() != 0 || at.nanosecond() != 0 {
        return false;
    }
    let minute = at.hour() * 60 + at.minute();
    let morning = 9 * 60 + 30 < minute && minute <= 11 * 60 + 30;
    let afternoon = 13 * 60 < minute && minute <= 15 * 60;
    morning || afternoon
}

fn failed_integrity(code: &'static str) -> BenchmarkError {
    BenchmarkError::FailedIntegrity { code }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, FixedOffset, NaiveDate};

    use super::{
        admit_benchmark_batch, BenchmarkAdmissionCoverage, BenchmarkBar, BenchmarkBarTime,
        BenchmarkError, BenchmarkRange, BenchmarkRegistry, BenchmarkRequest, BenchmarkUnsupported,
        HS300_CANONICAL,
    };

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 8, day).expect("valid test date")
    }

    fn minute(value: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(value).expect("valid +08:00 minute end")
    }

    fn daily_request(from: NaiveDate, to: NaiveDate, instrument: &str) -> BenchmarkRequest {
        BenchmarkRequest {
            instrument: instrument.to_owned(),
            range: BenchmarkRange::Daily { from, to },
        }
    }

    fn minute_request(from: DateTime<FixedOffset>, to: DateTime<FixedOffset>) -> BenchmarkRequest {
        BenchmarkRequest {
            instrument: "TEST_CODE_000300".to_owned(),
            range: BenchmarkRange::Minute1 { from, to },
        }
    }

    fn bar(at: BenchmarkBarTime) -> BenchmarkBar {
        BenchmarkBar {
            at,
            open: 3_500.0,
            high: 3_510.0,
            low: 3_490.0,
            close: 3_505.0,
            volume: None,
            amount: None,
        }
    }

    fn test_registry() -> BenchmarkRegistry {
        BenchmarkRegistry::test_only(["TEST_CODE_000300"])
    }

    fn assert_failed_integrity(result: Result<super::AdmittedBenchmarkBatch, BenchmarkError>) {
        assert!(matches!(
            result,
            Err(BenchmarkError::FailedIntegrity { .. })
        ));
    }

    #[test]
    fn accepts_a_daily_batch_for_an_explicit_test_registry() {
        let trading_day = date(21);
        let request = daily_request(trading_day, trading_day, "TEST_CODE_000300");
        let bars = vec![bar(BenchmarkBarTime::Daily(trading_day))];

        let admitted = admit_benchmark_batch(
            &BenchmarkRegistry::test_only(["TEST_CODE_000300"]),
            request,
            bars,
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[trading_day],
            },
        )
        .expect("explicit test registry admits its test identity");

        assert_eq!(admitted.bars().len(), 1);
    }

    #[test]
    fn production_registry_only_accepts_the_canonical_hs300_identity() {
        let trading_day = date(21);
        let coverage = BenchmarkAdmissionCoverage::Daily {
            authoritative_trading_days: &[trading_day],
        };
        let bars = vec![bar(BenchmarkBarTime::Daily(trading_day))];

        assert!(admit_benchmark_batch(
            &BenchmarkRegistry::production_default(),
            daily_request(trading_day, trading_day, HS300_CANONICAL),
            bars.clone(),
            coverage,
        )
        .is_ok());
        assert_eq!(
            admit_benchmark_batch(
                &BenchmarkRegistry::production_default(),
                daily_request(trading_day, trading_day, "sh000905"),
                bars.clone(),
                coverage,
            ),
            Err(BenchmarkError::Unsupported(
                BenchmarkUnsupported::UnsupportedInstrument
            ))
        );
        assert_eq!(
            admit_benchmark_batch(
                &BenchmarkRegistry::production_default(),
                daily_request(trading_day, trading_day, "sz000001"),
                bars.clone(),
                coverage,
            ),
            Err(BenchmarkError::Unsupported(
                BenchmarkUnsupported::UnsupportedInstrument
            ))
        );
        assert_eq!(
            admit_benchmark_batch(
                &BenchmarkRegistry::production_default(),
                daily_request(trading_day, trading_day, "TEST_CODE_000300"),
                bars,
                coverage,
            ),
            Err(BenchmarkError::Unsupported(
                BenchmarkUnsupported::TestIdentityRejected
            ))
        );
    }

    #[test]
    fn rejects_reversed_ranges_and_non_shanghai_minute_ranges() {
        let day = date(21);
        assert_failed_integrity(admit_benchmark_batch(
            &test_registry(),
            daily_request(day, date(20), "TEST_CODE_000300"),
            vec![bar(BenchmarkBarTime::Daily(day))],
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[day],
            },
        ));
        assert_failed_integrity(admit_benchmark_batch(
            &test_registry(),
            minute_request(
                minute("2026-08-21T09:31:00+09:00"),
                minute("2026-08-21T09:32:00+09:00"),
            ),
            vec![],
            BenchmarkAdmissionCoverage::Minute1,
        ));
        assert_failed_integrity(admit_benchmark_batch(
            &test_registry(),
            minute_request(
                minute("2026-08-21T09:32:00+08:00"),
                minute("2026-08-21T09:31:00+08:00"),
            ),
            vec![],
            BenchmarkAdmissionCoverage::Minute1,
        ));
    }

    #[test]
    fn rejects_invalid_ohlc_optional_turnover_and_mismatched_time_kinds() {
        let day = date(21);
        for malformed in [
            BenchmarkBar {
                open: 0.0,
                ..bar(BenchmarkBarTime::Daily(day))
            },
            BenchmarkBar {
                high: f64::INFINITY,
                ..bar(BenchmarkBarTime::Daily(day))
            },
            BenchmarkBar {
                low: 3_506.0,
                ..bar(BenchmarkBarTime::Daily(day))
            },
            BenchmarkBar {
                volume: Some(-1.0),
                ..bar(BenchmarkBarTime::Daily(day))
            },
            BenchmarkBar {
                amount: Some(f64::NAN),
                ..bar(BenchmarkBarTime::Daily(day))
            },
            bar(BenchmarkBarTime::MinuteEnd(minute(
                "2026-08-21T09:31:00+08:00",
            ))),
        ] {
            assert_failed_integrity(admit_benchmark_batch(
                &test_registry(),
                daily_request(day, day, "TEST_CODE_000300"),
                vec![malformed],
                BenchmarkAdmissionCoverage::Daily {
                    authoritative_trading_days: &[day],
                },
            ));
        }
    }

    #[test]
    fn daily_batches_require_exact_explicit_authoritative_coverage_and_order() {
        let d1 = date(19);
        let d2 = date(20);
        let d3 = date(21);
        let request = daily_request(d1, d3, "TEST_CODE_000300");
        let complete = vec![
            bar(BenchmarkBarTime::Daily(d1)),
            bar(BenchmarkBarTime::Daily(d2)),
            bar(BenchmarkBarTime::Daily(d3)),
        ];
        assert!(admit_benchmark_batch(
            &test_registry(),
            request.clone(),
            complete.clone(),
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[d1, d2, d3],
            },
        )
        .is_ok());
        assert_failed_integrity(admit_benchmark_batch(
            &test_registry(),
            request.clone(),
            vec![
                bar(BenchmarkBarTime::Daily(d1)),
                bar(BenchmarkBarTime::Daily(d3)),
            ],
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[d1, d2, d3],
            },
        ));
        assert_failed_integrity(admit_benchmark_batch(
            &test_registry(),
            request.clone(),
            vec![
                bar(BenchmarkBarTime::Daily(d1)),
                bar(BenchmarkBarTime::Daily(d1)),
            ],
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[d1, d2, d3],
            },
        ));
        assert_failed_integrity(admit_benchmark_batch(
            &test_registry(),
            request,
            vec![
                bar(BenchmarkBarTime::Daily(d2)),
                bar(BenchmarkBarTime::Daily(d1)),
                bar(BenchmarkBarTime::Daily(d3)),
            ],
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[d1, d2, d3],
            },
        ));
        assert_failed_integrity(admit_benchmark_batch(
            &test_registry(),
            daily_request(d1, d3, "TEST_CODE_000300"),
            complete,
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[d1, d3],
            },
        ));
    }

    #[test]
    fn minute_batches_require_the_session_grid_but_allow_the_lunch_break() {
        let m1129 = minute("2026-08-21T11:29:00+08:00");
        let m1130 = minute("2026-08-21T11:30:00+08:00");
        let m1301 = minute("2026-08-21T13:01:00+08:00");
        let m1302 = minute("2026-08-21T13:02:00+08:00");
        let request = minute_request(m1129, m1302);
        let complete = vec![
            bar(BenchmarkBarTime::MinuteEnd(m1129)),
            bar(BenchmarkBarTime::MinuteEnd(m1130)),
            bar(BenchmarkBarTime::MinuteEnd(m1301)),
            bar(BenchmarkBarTime::MinuteEnd(m1302)),
        ];
        assert!(admit_benchmark_batch(
            &test_registry(),
            request.clone(),
            complete.clone(),
            BenchmarkAdmissionCoverage::Minute1,
        )
        .is_ok());
        assert_failed_integrity(admit_benchmark_batch(
            &test_registry(),
            request.clone(),
            vec![
                bar(BenchmarkBarTime::MinuteEnd(m1129)),
                bar(BenchmarkBarTime::MinuteEnd(m1130)),
                bar(BenchmarkBarTime::MinuteEnd(m1302)),
            ],
            BenchmarkAdmissionCoverage::Minute1,
        ));
        assert_failed_integrity(admit_benchmark_batch(
            &test_registry(),
            request.clone(),
            vec![
                bar(BenchmarkBarTime::MinuteEnd(m1129)),
                bar(BenchmarkBarTime::MinuteEnd(m1130)),
                bar(BenchmarkBarTime::MinuteEnd(minute(
                    "2026-08-21T11:31:00+08:00",
                ))),
                bar(BenchmarkBarTime::MinuteEnd(m1301)),
                bar(BenchmarkBarTime::MinuteEnd(m1302)),
            ],
            BenchmarkAdmissionCoverage::Minute1,
        ));
        assert_failed_integrity(admit_benchmark_batch(
            &test_registry(),
            request,
            vec![
                bar(BenchmarkBarTime::MinuteEnd(m1129)),
                bar(BenchmarkBarTime::MinuteEnd(m1129)),
                bar(BenchmarkBarTime::MinuteEnd(m1130)),
                bar(BenchmarkBarTime::MinuteEnd(m1301)),
                bar(BenchmarkBarTime::MinuteEnd(m1302)),
            ],
            BenchmarkAdmissionCoverage::Minute1,
        ));
        let descending = admit_benchmark_batch(
            &test_registry(),
            minute_request(m1129, m1301),
            vec![
                bar(BenchmarkBarTime::MinuteEnd(m1130)),
                bar(BenchmarkBarTime::MinuteEnd(m1129)),
                bar(BenchmarkBarTime::MinuteEnd(m1301)),
            ],
            BenchmarkAdmissionCoverage::Minute1,
        );
        assert!(matches!(
            descending,
            Err(BenchmarkError::FailedIntegrity { .. })
        ));
        assert_failed_integrity(admit_benchmark_batch(
            &test_registry(),
            minute_request(m1129, minute("2026-08-22T09:31:00+08:00")),
            complete,
            BenchmarkAdmissionCoverage::Minute1,
        ));
    }

    #[test]
    fn empty_batches_are_typed_unavailable_and_large_source_moves_are_preserved() {
        let day = date(21);
        assert_eq!(
            admit_benchmark_batch(
                &test_registry(),
                daily_request(day, day, "TEST_CODE_000300"),
                vec![],
                BenchmarkAdmissionCoverage::Daily {
                    authoritative_trading_days: &[day],
                },
            ),
            Err(BenchmarkError::Unavailable {
                code: "benchmark_batch_empty",
                retryable: true,
            })
        );
        let large_move = BenchmarkBar {
            open: 100.0,
            high: 135.0,
            low: 99.0,
            close: 130.0,
            ..bar(BenchmarkBarTime::Daily(day))
        };
        assert!(admit_benchmark_batch(
            &test_registry(),
            daily_request(day, day, "TEST_CODE_000300"),
            vec![large_move],
            BenchmarkAdmissionCoverage::Daily {
                authoritative_trading_days: &[day],
            },
        )
        .is_ok());
    }
}
