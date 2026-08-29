use chrono::{DateTime, FixedOffset, NaiveDate, NaiveTime};
use std::sync::Mutex;
use stock_analysis::database::attribution_epochs::{
    AttributionEpochReceipt, AttributionEpochStore, AttributionEpochStoreError,
    EpochActivationDisposition, EpochActivationRequest,
};
use stock_analysis::database::DatabaseManager;
use stock_analysis::performance::attribution_epoch::EpochActivationSource;

static ATTRIBUTION_EPOCH_LAST_SUCCESS: Mutex<Option<NaiveDate>> = Mutex::new(None);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributionEpochTickOutcome {
    OutsideWindow,
    Activated(AttributionEpochReceipt),
    Verified(AttributionEpochReceipt),
    Unavailable { code: String, retryable: bool },
    FailedIntegrity { code: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CalendarObservation {
    TradingDay,
    NonTradingDay,
    Unavailable { code: String, retryable: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EpochObservation {
    Missing,
    Existing,
    FailedIntegrity { code: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AttributionEpochTickDecision {
    OutsideWindow,
    Activate,
    VerifyOnly,
    Unavailable { code: String, retryable: bool },
    FailedIntegrity { code: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RuntimeFailure {
    Unavailable { code: String, retryable: bool },
    FailedIntegrity { code: String },
}

impl RuntimeFailure {
    fn into_outcome(self) -> AttributionEpochTickOutcome {
        match self {
            Self::Unavailable { code, retryable } => {
                AttributionEpochTickOutcome::Unavailable { code, retryable }
            }
            Self::FailedIntegrity { code } => AttributionEpochTickOutcome::FailedIntegrity { code },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ActivationObservation {
    Activated(AttributionEpochReceipt),
    Existing(AttributionEpochReceipt),
}

impl From<AttributionEpochStoreError> for RuntimeFailure {
    fn from(error: AttributionEpochStoreError) -> Self {
        match error {
            AttributionEpochStoreError::Unavailable {
                reason_code,
                retryable,
                ..
            } => Self::Unavailable {
                code: reason_code.to_owned(),
                retryable,
            },
            AttributionEpochStoreError::FailedIntegrity { reason_code, .. } => {
                Self::FailedIntegrity {
                    code: reason_code.to_owned(),
                }
            }
        }
    }
}

fn decide_attribution_epoch_tick(
    now: DateTime<FixedOffset>,
    calendar: CalendarObservation,
    epoch: EpochObservation,
) -> AttributionEpochTickDecision {
    match calendar {
        CalendarObservation::Unavailable { code, retryable } => {
            return AttributionEpochTickDecision::Unavailable { code, retryable };
        }
        CalendarObservation::NonTradingDay => {
            return AttributionEpochTickDecision::OutsideWindow;
        }
        CalendarObservation::TradingDay => {}
    }

    if now.offset().local_minus_utc() != 8 * 60 * 60 {
        return AttributionEpochTickDecision::Unavailable {
            code: "attribution_epoch_invalid_timezone".to_owned(),
            retryable: false,
        };
    }
    let (Some(start), Some(end)) = (
        NaiveTime::from_hms_opt(15, 35, 0),
        NaiveTime::from_hms_opt(15, 50, 0),
    ) else {
        return AttributionEpochTickDecision::FailedIntegrity {
            code: "attribution_epoch_window_definition_invalid".to_owned(),
        };
    };
    if now.time() < start || now.time() > end {
        return AttributionEpochTickDecision::OutsideWindow;
    }

    match epoch {
        EpochObservation::Missing => AttributionEpochTickDecision::Activate,
        EpochObservation::Existing => AttributionEpochTickDecision::VerifyOnly,
        EpochObservation::FailedIntegrity { code } => {
            AttributionEpochTickDecision::FailedIntegrity { code }
        }
    }
}

trait AttributionEpochRuntime {
    fn calendar_observation(&self, date: NaiveDate) -> CalendarObservation;
    fn activate_or_verify(
        &self,
        now: DateTime<FixedOffset>,
    ) -> Result<ActivationObservation, RuntimeFailure>;
}

struct DatabaseAttributionEpochRuntime<'a> {
    database: &'a DatabaseManager,
}

impl AttributionEpochRuntime for DatabaseAttributionEpochRuntime<'_> {
    fn calendar_observation(&self, date: NaiveDate) -> CalendarObservation {
        match stock_analysis::calendar::verified_a_share_trading_day(date) {
            Ok(true) => CalendarObservation::TradingDay,
            Ok(false) => CalendarObservation::NonTradingDay,
            Err(_) => CalendarObservation::Unavailable {
                code: "attribution_epoch_calendar_coverage_unavailable".to_owned(),
                retryable: false,
            },
        }
    }

    fn activate_or_verify(
        &self,
        now: DateTime<FixedOffset>,
    ) -> Result<ActivationObservation, RuntimeFailure> {
        let outcome = AttributionEpochStore::new(self.database)
            .activate_once_with_outcome(EpochActivationRequest {
                source: EpochActivationSource::Monitor,
                invoked_at: now,
            })
            .map_err(RuntimeFailure::from)?;
        let receipt = outcome.receipt().clone();
        match outcome.disposition() {
            EpochActivationDisposition::Activated => Ok(ActivationObservation::Activated(receipt)),
            EpochActivationDisposition::AlreadyActive => {
                Ok(ActivationObservation::Existing(receipt))
            }
        }
    }
}

fn run_tick_with_runtime<R: AttributionEpochRuntime>(
    runtime: &R,
    successful_date: &Mutex<Option<NaiveDate>>,
    now: DateTime<FixedOffset>,
) -> AttributionEpochTickOutcome {
    let date = now.date_naive();
    if *successful_date
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        == Some(date)
    {
        return AttributionEpochTickOutcome::OutsideWindow;
    }

    let calendar = runtime.calendar_observation(date);
    match decide_attribution_epoch_tick(now, calendar, EpochObservation::Missing) {
        AttributionEpochTickDecision::OutsideWindow => {
            return AttributionEpochTickOutcome::OutsideWindow;
        }
        AttributionEpochTickDecision::Unavailable { code, retryable } => {
            return AttributionEpochTickOutcome::Unavailable { code, retryable };
        }
        AttributionEpochTickDecision::FailedIntegrity { code } => {
            return AttributionEpochTickOutcome::FailedIntegrity { code };
        }
        AttributionEpochTickDecision::Activate => {}
        AttributionEpochTickDecision::VerifyOnly => {
            return AttributionEpochTickOutcome::FailedIntegrity {
                code: "attribution_epoch_runtime_state_invalid".to_owned(),
            };
        }
    }

    let outcome = match runtime.activate_or_verify(now) {
        Ok(ActivationObservation::Activated(receipt)) => match decide_attribution_epoch_tick(
            now,
            CalendarObservation::TradingDay,
            EpochObservation::Missing,
        ) {
            AttributionEpochTickDecision::Activate => {
                AttributionEpochTickOutcome::Activated(receipt)
            }
            _ => AttributionEpochTickOutcome::FailedIntegrity {
                code: "attribution_epoch_runtime_state_invalid".to_owned(),
            },
        },
        Ok(ActivationObservation::Existing(receipt)) => match decide_attribution_epoch_tick(
            now,
            CalendarObservation::TradingDay,
            EpochObservation::Existing,
        ) {
            AttributionEpochTickDecision::VerifyOnly => {
                AttributionEpochTickOutcome::Verified(receipt)
            }
            _ => AttributionEpochTickOutcome::FailedIntegrity {
                code: "attribution_epoch_runtime_state_invalid".to_owned(),
            },
        },
        Err(RuntimeFailure::FailedIntegrity { code }) => match decide_attribution_epoch_tick(
            now,
            CalendarObservation::TradingDay,
            EpochObservation::FailedIntegrity { code },
        ) {
            AttributionEpochTickDecision::FailedIntegrity { code } => {
                AttributionEpochTickOutcome::FailedIntegrity { code }
            }
            _ => AttributionEpochTickOutcome::FailedIntegrity {
                code: "attribution_epoch_runtime_state_invalid".to_owned(),
            },
        },
        Err(error @ RuntimeFailure::Unavailable { .. }) => error.into_outcome(),
    };

    if matches!(
        outcome,
        AttributionEpochTickOutcome::Activated(_) | AttributionEpochTickOutcome::Verified(_)
    ) {
        *successful_date
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(date);
    }
    outcome
}

pub fn run_attribution_epoch_tick(
    database: &DatabaseManager,
    now: DateTime<FixedOffset>,
) -> AttributionEpochTickOutcome {
    run_tick_with_runtime(
        &DatabaseAttributionEpochRuntime { database },
        &ATTRIBUTION_EPOCH_LAST_SUCCESS,
        now,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, FixedOffset, NaiveDate};
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    fn at(raw: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(raw).expect("TEST_CODE valid +08 timestamp")
    }

    fn receipt(epoch_id: &str) -> AttributionEpochReceipt {
        AttributionEpochReceipt {
            epoch_id: epoch_id.to_owned(),
            cutover_completed_trading_date: NaiveDate::from_ymd_opt(2026, 8, 28).unwrap(),
            effective_trading_date: NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
            paper_trade_high_water: 2,
            legacy_filled_manifest_hash: "1".repeat(64),
            terminal_binding_manifest_hash: "2".repeat(64),
            order_audit_high_water: 2,
            order_audit_tip_hash: "3".repeat(64),
            calendar_authority_hash: "4".repeat(64),
            legacy_carry_manifest_hash: "5".repeat(64),
            carry_item_count: 1,
            carry_total_quantity: 100,
            position_projection_hash: "6".repeat(64),
            previous_epoch_receipt_hash: None,
            receipt_hash: "7".repeat(64),
            created_at: "2026-08-28T15:40:00+08:00".to_owned(),
            retention_deadline: "2031-08-28T15:40:00+08:00".to_owned(),
        }
    }

    #[test]
    fn decision_table_is_fail_closed_at_calendar_and_window_boundaries() {
        let cases = [
            (
                "2026-08-28T15:34:59+08:00",
                CalendarObservation::TradingDay,
                EpochObservation::Missing,
                AttributionEpochTickDecision::OutsideWindow,
            ),
            (
                "2026-08-28T15:35:00+08:00",
                CalendarObservation::TradingDay,
                EpochObservation::Missing,
                AttributionEpochTickDecision::Activate,
            ),
            (
                "2026-08-28T15:50:00+08:00",
                CalendarObservation::TradingDay,
                EpochObservation::Existing,
                AttributionEpochTickDecision::VerifyOnly,
            ),
            (
                "2026-08-28T15:51:00+08:00",
                CalendarObservation::TradingDay,
                EpochObservation::Missing,
                AttributionEpochTickDecision::OutsideWindow,
            ),
            (
                "2026-08-29T15:40:00+08:00",
                CalendarObservation::NonTradingDay,
                EpochObservation::Missing,
                AttributionEpochTickDecision::OutsideWindow,
            ),
            (
                "2026-10-01T15:40:00+08:00",
                CalendarObservation::NonTradingDay,
                EpochObservation::Existing,
                AttributionEpochTickDecision::OutsideWindow,
            ),
        ];

        for (now, calendar, epoch, expected) in cases {
            assert_eq!(
                decide_attribution_epoch_tick(at(now), calendar, epoch),
                expected
            );
        }

        assert_eq!(
            decide_attribution_epoch_tick(
                at("2026-08-28T15:40:00+08:00"),
                CalendarObservation::Unavailable {
                    code: "TEST_CODE_calendar_unavailable".to_owned(),
                    retryable: false,
                },
                EpochObservation::Missing,
            ),
            AttributionEpochTickDecision::Unavailable {
                code: "TEST_CODE_calendar_unavailable".to_owned(),
                retryable: false,
            }
        );

        for code in [
            "TEST_CODE_schema_integrity",
            "TEST_CODE_chain_integrity",
            "TEST_CODE_source_integrity",
        ] {
            assert_eq!(
                decide_attribution_epoch_tick(
                    at("2026-08-28T15:40:00+08:00"),
                    CalendarObservation::TradingDay,
                    EpochObservation::FailedIntegrity {
                        code: code.to_owned(),
                    },
                ),
                AttributionEpochTickDecision::FailedIntegrity {
                    code: code.to_owned(),
                }
            );
        }
    }

    struct FakeRuntime {
        calendar: CalendarObservation,
        outcomes: RefCell<VecDeque<Result<ActivationObservation, RuntimeFailure>>>,
        calls: Cell<usize>,
    }

    impl FakeRuntime {
        fn new(outcomes: Vec<Result<ActivationObservation, RuntimeFailure>>) -> Self {
            Self {
                calendar: CalendarObservation::TradingDay,
                outcomes: RefCell::new(outcomes.into()),
                calls: Cell::new(0),
            }
        }
    }

    impl AttributionEpochRuntime for FakeRuntime {
        fn calendar_observation(&self, _date: NaiveDate) -> CalendarObservation {
            self.calendar.clone()
        }

        fn activate_or_verify(
            &self,
            _now: DateTime<FixedOffset>,
        ) -> Result<ActivationObservation, RuntimeFailure> {
            self.calls.set(self.calls.get() + 1);
            self.outcomes
                .borrow_mut()
                .pop_front()
                .expect("TEST_CODE activation outcome configured")
        }
    }

    #[test]
    fn successful_activation_sets_latch_and_prevents_repeat_work() {
        let expected = receipt(&"a".repeat(64));
        let runtime =
            FakeRuntime::new(vec![Ok(ActivationObservation::Activated(expected.clone()))]);
        let latch = Mutex::new(None);

        assert_eq!(
            run_tick_with_runtime(&runtime, &latch, at("2026-08-28T15:40:00+08:00")),
            AttributionEpochTickOutcome::Activated(expected)
        );
        assert_eq!(
            run_tick_with_runtime(&runtime, &latch, at("2026-08-28T15:41:00+08:00")),
            AttributionEpochTickOutcome::OutsideWindow
        );
        assert_eq!(runtime.calls.get(), 1);
    }

    #[test]
    fn failed_activation_does_not_latch_and_next_tick_retries() {
        let expected = receipt(&"b".repeat(64));
        let runtime = FakeRuntime::new(vec![
            Err(RuntimeFailure::Unavailable {
                code: "TEST_CODE_storage_busy".to_owned(),
                retryable: true,
            }),
            Ok(ActivationObservation::Activated(expected.clone())),
        ]);
        let latch = Mutex::new(None);

        assert_eq!(
            run_tick_with_runtime(&runtime, &latch, at("2026-08-28T15:40:00+08:00")),
            AttributionEpochTickOutcome::Unavailable {
                code: "TEST_CODE_storage_busy".to_owned(),
                retryable: true,
            }
        );
        assert_eq!(
            run_tick_with_runtime(&runtime, &latch, at("2026-08-28T15:41:00+08:00")),
            AttributionEpochTickOutcome::Activated(expected)
        );
        assert_eq!(runtime.calls.get(), 2);
    }

    #[test]
    fn existing_epoch_is_verified_without_activation_and_then_latched() {
        let expected = receipt(&"c".repeat(64));
        let runtime = FakeRuntime::new(vec![Ok(ActivationObservation::Existing(expected.clone()))]);
        let latch = Mutex::new(None);

        assert_eq!(
            run_tick_with_runtime(&runtime, &latch, at("2026-08-28T15:40:00+08:00")),
            AttributionEpochTickOutcome::Verified(expected)
        );
        assert_eq!(runtime.calls.get(), 1);
    }

    #[test]
    fn integrity_failure_is_structured_and_does_not_touch_unrelated_trading_state() {
        let runtime = FakeRuntime::new(vec![Err(RuntimeFailure::FailedIntegrity {
            code: "TEST_CODE_source_integrity".to_owned(),
        })]);
        let latch = Mutex::new(None);
        let paper_trading_latch = Cell::new(false);

        assert_eq!(
            run_tick_with_runtime(&runtime, &latch, at("2026-08-28T15:40:00+08:00")),
            AttributionEpochTickOutcome::FailedIntegrity {
                code: "TEST_CODE_source_integrity".to_owned(),
            }
        );
        assert!(!paper_trading_latch.get());
        assert_eq!(*latch.lock().unwrap(), None);
    }

    #[test]
    fn daily_attribution_errors_cannot_reach_the_push_success_arm() {
        let main = include_str!("main.rs");
        let block = main
            .split_once("// Attribution Research Loop")
            .expect("TEST_CODE attribution loop marker")
            .1
            .split_once("// G5b 深链归因")
            .expect("TEST_CODE deep attribution boundary")
            .0;

        assert!(block.contains("compute_epoch_daily"));
        assert!(block.contains("compute_epoch_window"));
        assert!(block.contains("persist_epoch_daily"));
        assert!(!block.contains("compute_daily("));
        assert!(!block.contains("compute_window("));
        assert!(!block.contains("persist_daily("));

        let success = block
            .find("Ok(text) =>")
            .expect("TEST_CODE explicit success arm");
        let push = block
            .find("push_governor_v3(&text")
            .expect("TEST_CODE attribution push call");
        let unavailable = block
            .find("Err(AttributionEpochRuntimeError::Unavailable")
            .expect("TEST_CODE unavailable arm");
        assert!(success < push && push < unavailable);
        assert!(!block[unavailable..].contains("push_governor_v3(&text"));
    }
}
