//! Empty operations use the already reconciled runtime owner on a blocking
//! worker. Cached seals are only prior observations and must be refreshed.
use super::*;
use std::collections::BTreeMap;
use stock_analysis::monitor::g5b_empty_v2::{self as empty, G5bEmptyInspectionV2, G5bEmptySealV2};

/// Scheduling observations only. No value here grants completion or dispatch.
#[derive(Debug)]
pub(crate) enum G5bEmptyTickObservation {
    ContinueNonEmpty,
    Pending {
        cohort_identity: String,
        revision: i64,
    },
    Sealed {
        seal_identity: String,
        revision: i64,
        reason: &'static str,
    },
}

struct KnownSeals {
    // Pin the actual owner so neither a reused address nor a matching namespace
    // label can transplant a previously observed source incarnation.
    coordinator: Arc<DurableDeliveryCoordinator>,
    by_date: BTreeMap<NaiveDate, G5bEmptySealV2>,
}

static KNOWN_SEALS: OnceLock<Mutex<BTreeMap<String, KnownSeals>>> = OnceLock::new();

fn ready_runtime() -> Result<Arc<RuntimeState>, String> {
    let state = runtime_state()?;
    if !state.producer_ready.load(Ordering::Acquire) {
        return Err("G5b Empty requires the existing startup reconciliation barrier".to_owned());
    }
    Ok(state)
}

/// Called only by the resident-service startup and the post-wait daily scanner
/// boundary. The caller clock schedules work; the private owner checks its own
/// fresh production clock before recording any prospective observation.
pub(crate) async fn initialize_g5b_empty_before_input_writers() -> Result<Option<NaiveDate>, String>
{
    let state = ready_runtime()?;
    tokio::task::spawn_blocking(move || {
        let Some(date) = initialization_due(Utc::now())? else {
            return Ok(None);
        };
        empty::initialize_empty_day_v2(Arc::clone(&state.coordinator), date)
            .map_err(|e| format!("G5b Empty prospective initialization unavailable: {e}"))?;
        Ok(Some(date))
    })
    .await
    .map_err(|e| format!("join G5b Empty initialization: {e}"))?
}

/// Inspect before the model path, even when fresh model work is unavailable.
/// This never reconciles startup, invokes a provider or enters a counted sink.
pub(crate) async fn inspect_g5b_empty_tick_v2(
    date: NaiveDate,
) -> Result<G5bEmptyTickObservation, String> {
    let state = ready_runtime()?;
    tokio::task::spawn_blocking(move || inspect_tick_blocking(state.as_ref(), date))
        .await
        .map_err(|e| format!("join G5b Empty inspection: {e}"))?
}

fn initialization_due(now: DateTime<Utc>) -> Result<Option<NaiveDate>, String> {
    let local = now.with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap());
    if local.time() >= chrono::NaiveTime::from_hms_opt(15, 5, 0).unwrap() {
        return Ok(None);
    }
    if stock_analysis::calendar::verified_a_share_trading_day(local.date_naive())? {
        Ok(Some(local.date_naive()))
    } else {
        Ok(None)
    }
}

fn closing_due(date: NaiveDate, now: DateTime<Utc>) -> bool {
    let local = now.with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap());
    local.date_naive() == date
        && local.time() >= chrono::NaiveTime::from_hms_opt(15, 21, 0).unwrap()
        && stock_analysis::calendar::verified_a_share_trading_day(date) == Ok(true)
}

fn inspect_tick_blocking(
    state: &RuntimeState,
    date: NaiveDate,
) -> Result<G5bEmptyTickObservation, String> {
    let inspection = empty::inspect_empty_day_v2(Arc::clone(&state.coordinator), date)
        .map_err(|e| format!("inspect actual G5b Empty: {e}"))?;
    let mut owners = KNOWN_SEALS
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .map_err(|_| "G5b Empty observation cache mutex poisoned".to_owned())?;
    let known = owners
        .entry(state.namespace.label())
        .or_insert_with(|| KnownSeals {
            coordinator: Arc::clone(&state.coordinator),
            by_date: BTreeMap::new(),
        });
    if !Arc::ptr_eq(&known.coordinator, &state.coordinator) {
        return Err("G5b Empty cached observation belongs to another runtime owner".to_owned());
    }
    if let Some(previous) = known.by_date.get(&date) {
        if !matches!(inspection, G5bEmptyInspectionV2::EmptySealed(_)) {
            return Err("G5b Empty previously observed seal is no longer current".to_owned());
        }
        // Refresh the previous capability, not the newly read one: replacing
        // a known late positive source must not erase its inode constraint.
        let current = empty::refresh_empty_day_v2(Arc::clone(&state.coordinator), previous)
            .map_err(|e| format!("refresh prior G5b Empty observation: {e}"))?;
        return remember_seal(known, date, current);
    }
    match inspection {
        G5bEmptyInspectionV2::Absent if !closing_due(date, Utc::now()) => {
            Ok(G5bEmptyTickObservation::ContinueNonEmpty)
        }
        G5bEmptyInspectionV2::NonEmpty => Ok(G5bEmptyTickObservation::ContinueNonEmpty),
        G5bEmptyInspectionV2::Absent => {
            // Absence, NoEligible and NoProvider are never Empty evidence. The
            // real owner requires its original prospective zero receipt.
            let current = empty::close_empty_day_v2(Arc::clone(&state.coordinator), date)
                .map_err(|e| format!("close actual G5b Empty unavailable: {e}"))?;
            remember_seal(known, date, current)
        }
        G5bEmptyInspectionV2::EmptyPending(original) => {
            let recovered = empty::recover_empty_day_v2(Arc::clone(&state.coordinator), date)
                .map_err(|e| format!("recover original G5b Empty Selection: {e}"))?;
            match recovered {
                G5bEmptyInspectionV2::EmptyPending(pending) => {
                    if pending.business_date() != original.business_date()
                        || pending.cohort_identity() != original.cohort_identity()
                    {
                        return Err("G5b Empty recovery changed the original cohort".to_owned());
                    }
                    if !closing_due(date, Utc::now()) {
                        return Ok(G5bEmptyTickObservation::Pending {
                            cohort_identity: pending.cohort_identity().to_owned(),
                            revision: pending.revision(),
                        });
                    }
                    let current =
                        empty::close_empty_day_v2(Arc::clone(&state.coordinator), date)
                            .map_err(|e| format!("seal recovered G5b Empty unavailable: {e}"))?;
                    if current.cohort_identity() != original.cohort_identity() {
                        return Err("G5b Empty sealing changed the original cohort".to_owned());
                    }
                    remember_seal(known, date, current)
                }
                G5bEmptyInspectionV2::EmptySealed(current) => {
                    if current.cohort_identity() != original.cohort_identity() {
                        return Err("G5b Empty recovery changed the original cohort".to_owned());
                    }
                    refresh_and_remember(state, known, date, current)
                }
                G5bEmptyInspectionV2::Absent | G5bEmptyInspectionV2::NonEmpty => {
                    Err("G5b Empty recovery lost its original Empty cohort".to_owned())
                }
            }
        }
        G5bEmptyInspectionV2::EmptySealed(current) => {
            refresh_and_remember(state, known, date, current)
        }
    }
}

fn refresh_and_remember(
    state: &RuntimeState,
    known: &mut KnownSeals,
    date: NaiveDate,
    current: G5bEmptySealV2,
) -> Result<G5bEmptyTickObservation, String> {
    let refreshed = empty::refresh_empty_day_v2(Arc::clone(&state.coordinator), &current)
        .map_err(|e| format!("refresh actual G5b Empty observation: {e}"))?;
    remember_seal(known, date, refreshed)
}

fn remember_seal(
    known: &mut KnownSeals,
    date: NaiveDate,
    current: G5bEmptySealV2,
) -> Result<G5bEmptyTickObservation, String> {
    if current.business_date() != date {
        return Err("G5b Empty seal belongs to another business date".to_owned());
    }
    let observation = G5bEmptyTickObservation::Sealed {
        seal_identity: current.identity().to_owned(),
        revision: current.revision(),
        reason: current.reason(),
    };
    known.by_date.insert(date, current);
    Ok(observation)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(local: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(local)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn g5b_empty_runtime_scheduling_resamples_rollover_and_never_closes_other_dates() {
        let previous = utc("2026-09-28T15:22:00+08:00");
        let next = utc("2026-09-29T09:30:00+08:00");
        assert_eq!(initialization_due(previous).unwrap(), None);
        let next_date = NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();
        assert_eq!(initialization_due(next).unwrap(), Some(next_date));
        assert!(!closing_due(next_date, previous));
        assert!(!closing_due(next_date, next));
        assert_eq!(
            initialization_due(utc("2026-09-29T15:05:00+08:00")).unwrap(),
            None
        );
        assert!(!closing_due(next_date, utc("2026-09-29T15:20:59+08:00")));
        assert!(closing_due(next_date, utc("2026-09-29T15:21:00+08:00")));
        assert_eq!(
            initialization_due(utc("2026-10-02T09:30:00+08:00")).unwrap(),
            None
        );
        assert!(!closing_due(
            NaiveDate::from_ymd_opt(2026, 10, 2).unwrap(),
            utc("2026-10-02T15:21:00+08:00")
        ));
    }

    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn g5b_empty_runtime_uses_actual_existing_owner_and_requires_startup_barrier() {
        let _guard = crate::TestEnvGuard::dry_run_non_quiet();
        let state = runtime_state().unwrap();
        assert!(!state.producer_ready.load(Ordering::Acquire));
        let first = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let today = Utc::now()
            .with_timezone(&chrono::FixedOffset::east_opt(8 * 3600).unwrap())
            .date_naive();
        let date = if first == today {
            NaiveDate::from_ymd_opt(2026, 9, 29).unwrap()
        } else {
            first
        };
        assert!(inspect_g5b_empty_tick_v2(date)
            .await
            .unwrap_err()
            .contains("startup reconciliation"));
        assert!(initialize_g5b_empty_before_input_writers()
            .await
            .unwrap_err()
            .contains("startup reconciliation"));
        ensure_startup_reconciled().await.unwrap();
        assert!(Arc::ptr_eq(&state, &runtime_state().unwrap()));
        assert!(matches!(
            inspect_g5b_empty_tick_v2(date).await.unwrap(),
            G5bEmptyTickObservation::ContinueNonEmpty
        ));
        let code = match &state.namespace {
            RuntimeNamespace::Test { test_code } => test_code,
            RuntimeNamespace::Production => panic!("test cannot use production owner"),
        };
        let namespace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("data/test")
            .join(code);
        let log = stock_analysis::monitor::alert_log::AlertLog::for_test(&namespace).unwrap();
        log.initialize_date_input_head(date).unwrap();
        let head = namespace.join(format!("{}.input-head.v1.json", date.format("%Y%m%d")));
        let before = std::fs::read(&head).unwrap();
        #[cfg(unix)]
        let before_inode = {
            use std::os::unix::fs::MetadataExt;
            std::fs::metadata(&head).unwrap().ino()
        };
        // A legacy zero head is still Absent routing, not a prospective receipt
        // or an Empty completion. Ordinary NonEmpty work remains reachable.
        assert!(matches!(
            inspect_g5b_empty_tick_v2(date).await.unwrap(),
            G5bEmptyTickObservation::ContinueNonEmpty
        ));
        // Remove only this test's empty scheduling entry. With fresh model work
        // disabled, the real bin tick must still visit the actual Empty reader.
        let removed = KNOWN_SEALS
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .remove(&state.namespace.label())
            .unwrap();
        assert!(removed.by_date.is_empty());
        assert!(!crate::g5b_v2::run_tick(date, false).await.unwrap());
        assert!(KNOWN_SEALS
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .contains_key(&state.namespace.label()));
        assert_eq!(std::fs::read(&head).unwrap(), before);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(std::fs::metadata(&head).unwrap().ino(), before_inode);
        }
        let connection =
            rusqlite::Connection::open(namespace.join("durable_delivery.sqlite3")).unwrap();
        let rows: (i64, i64, i64) = connection
            .query_row(
                "SELECT (SELECT COUNT(*) FROM g5b_cohorts),
                        (SELECT COUNT(*) FROM g5b_day_seals),
                        (SELECT COUNT(*) FROM delivery_decisions)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(rows, (0, 0, 0));
        let owners = KNOWN_SEALS.get().unwrap().lock().unwrap();
        let known = owners.get(&state.namespace.label()).unwrap();
        assert!(Arc::ptr_eq(&known.coordinator, &state.coordinator));
        assert!(known.by_date.is_empty());
    }
}
