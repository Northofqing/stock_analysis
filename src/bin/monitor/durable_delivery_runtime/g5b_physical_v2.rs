//! The original counted runtime observes/recloses physical completion locally.
//! Cached capabilities only retain earlier input constraints. Every use rechecks
//! the actual owner; no date fence or cache guard crosses an external append.
use super::*;
use std::collections::BTreeMap;
use stock_analysis::monitor::g5b_physical_v2::{
    self as physical, G5bPhysicalAttemptV2, G5bPhysicalSealV2,
};

#[derive(Debug)]
pub(crate) enum G5bPhysicalTickObservation {
    Incomplete,
    Sealed {
        seal_identity: String,
        cohort_identity: String,
        revision: i64,
        count: usize,
        reason: &'static str,
    },
}

struct KnownSeals {
    coordinator: Arc<DurableDeliveryCoordinator>,
    by_date: BTreeMap<NaiveDate, G5bPhysicalSealV2>,
}
static KNOWN_SEALS: OnceLock<Mutex<BTreeMap<String, KnownSeals>>> = OnceLock::new();

fn ready_runtime() -> Result<Arc<RuntimeState>, String> {
    let state = runtime_state()?;
    if !state.producer_ready.load(Ordering::Acquire) {
        return Err(
            "G5b physical observation requires the existing startup reconciliation barrier".into(),
        );
    }
    Ok(state)
}

pub(crate) async fn list_g5b_physical_cohort_dates_v2() -> Result<Vec<NaiveDate>, String> {
    let state = ready_runtime()?;
    tokio::task::spawn_blocking(move || {
        physical::list_physical_cohort_dates_v2(Arc::clone(&state.coordinator))
            .map_err(|e| format!("read real G5b cohort date membership: {e}"))
    })
    .await
    .map_err(|e| format!("join G5b cohort date list: {e}"))?
}

/// Before model routing, read current physical state. A known dirty observation
/// gets original passive recovery and a known-aware reclose; it is never reset.
pub(crate) async fn inspect_g5b_physical_tick_v2(
    date: NaiveDate,
) -> Result<G5bPhysicalTickObservation, String> {
    let state = ready_runtime()?;
    tokio::task::spawn_blocking(move || observe_blocking(state.as_ref(), date, false))
        .await
        .map_err(|e| format!("join G5b physical inspection: {e}"))?
}

/// After original child/archive progression, drain original raw/audit evidence
/// and attempt the real finalizer even outside the fresh model window.
pub(crate) async fn finalize_g5b_physical_tick_v2(
    date: NaiveDate,
) -> Result<G5bPhysicalTickObservation, String> {
    let state = ready_runtime()?;
    tokio::task::spawn_blocking(move || observe_blocking(state.as_ref(), date, true))
        .await
        .map_err(|e| format!("join G5b physical finalizer: {e}"))?
}

fn observe_blocking(
    state: &RuntimeState,
    date: NaiveDate,
    finalize: bool,
) -> Result<G5bPhysicalTickObservation, String> {
    let _critical = state
        .counted_delivery_critical_section
        .lock()
        .map_err(|_| "BR-239 counted delivery critical-section mutex poisoned".to_owned())?;
    {
        let mut owners = KNOWN_SEALS
            .get_or_init(|| Mutex::new(BTreeMap::new()))
            .lock()
            .map_err(|_| "G5b physical observation cache mutex poisoned".to_owned())?;
        let known = owner_entry(&mut owners, state)?;
        if let Some(previous) = known.by_date.get(&date) {
            match physical::try_seal_physical_day_v2(
                Arc::clone(&state.coordinator),
                date,
                Some(previous),
            )
            .map_err(|e| format!("verify prior G5b physical observation: {e}"))?
            {
                G5bPhysicalAttemptV2::Sealed(current) => return remember(known, date, current),
                G5bPhysicalAttemptV2::Incomplete => {}
            }
        } else if let Some(current) =
            physical::read_physical_day_v2(Arc::clone(&state.coordinator), date)
                .map_err(|e| format!("read actual G5b physical seal: {e}"))?
        {
            return remember(known, date, current);
        } else if !finalize {
            return Ok(G5bPhysicalTickObservation::Incomplete);
        }
    }
    // The local session and cache lock above are both released before the
    // immutable append port. Reconciliation grants no model/sink opening.
    let context = format!("G5b physical cohort date={date}");
    let hydrations = reconcile_pending_with_context(state, &context)?;
    queue_hydrations(state, &unique_hydrations(hydrations))?;
    let mut owners = KNOWN_SEALS
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .map_err(|_| "G5b physical observation cache mutex poisoned".to_owned())?;
    let known = owner_entry(&mut owners, state)?;
    let value = physical::try_seal_physical_day_v2(
        Arc::clone(&state.coordinator),
        date,
        known.by_date.get(&date),
    )
    .map_err(|e| format!("fresh G5b physical finalizer: {e}"))?;
    match value {
        G5bPhysicalAttemptV2::Incomplete => Ok(G5bPhysicalTickObservation::Incomplete),
        G5bPhysicalAttemptV2::Sealed(current) => remember(known, date, current),
    }
}

fn owner_entry<'a>(
    owners: &'a mut BTreeMap<String, KnownSeals>,
    state: &RuntimeState,
) -> Result<&'a mut KnownSeals, String> {
    let known = owners
        .entry(state.namespace.label())
        .or_insert_with(|| KnownSeals {
            coordinator: Arc::clone(&state.coordinator),
            by_date: BTreeMap::new(),
        });
    if !Arc::ptr_eq(&known.coordinator, &state.coordinator) {
        return Err(
            "G5b physical cached observation belongs to another actual runtime owner".into(),
        );
    }
    Ok(known)
}
fn remember(
    known: &mut KnownSeals,
    date: NaiveDate,
    current: G5bPhysicalSealV2,
) -> Result<G5bPhysicalTickObservation, String> {
    if current.business_date() != date {
        return Err("G5b physical seal belongs to another business date".into());
    }
    let observation = G5bPhysicalTickObservation::Sealed {
        seal_identity: current.identity().to_owned(),
        cohort_identity: current.cohort_identity().to_owned(),
        revision: current.revision(),
        count: current.selected_count(),
        reason: current.reason(),
    };
    known.by_date.insert(date, current);
    Ok(observation)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[serial_test::serial(cooldown_memo)]
    async fn g5b_physical_v2_runtime_absent_real_owner_needs_barrier_and_creates_no_seal() {
        let _guard = crate::TestEnvGuard::dry_run_non_quiet();
        let state = runtime_state().unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        assert!(inspect_g5b_physical_tick_v2(date)
            .await
            .unwrap_err()
            .contains("startup reconciliation"));
        assert!(list_g5b_physical_cohort_dates_v2()
            .await
            .unwrap_err()
            .contains("startup reconciliation"));
        ensure_startup_reconciled().await.unwrap();
        assert!(Arc::ptr_eq(&state, &runtime_state().unwrap()));
        assert!(list_g5b_physical_cohort_dates_v2()
            .await
            .unwrap()
            .is_empty());
        assert!(matches!(
            inspect_g5b_physical_tick_v2(date).await.unwrap(),
            G5bPhysicalTickObservation::Incomplete
        ));
        assert!(matches!(
            finalize_g5b_physical_tick_v2(date).await.unwrap(),
            G5bPhysicalTickObservation::Incomplete
        ));
        let code = match &state.namespace {
            RuntimeNamespace::Test { test_code } => test_code,
            RuntimeNamespace::Production => panic!("test must not use production"),
        };
        let connection = rusqlite::Connection::open_with_flags(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("data/test")
                .join(code)
                .join("durable_delivery.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        for table in [
            "g5b_day_seals",
            "g5b_cohorts",
            "g5b_day_heads",
            "delivery_decisions",
        ] {
            let count: i64 = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0, "absence cannot manufacture {table}");
        }
    }
}
