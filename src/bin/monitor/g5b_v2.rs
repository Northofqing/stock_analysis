//! Current-day model production and exact stored handoff; no day completion bit.
use crate::durable_delivery_runtime as runtime;
use chrono::NaiveDate;
use stock_analysis::monitor::g5b_analysis_v2::G5bAnalysisClaimV2;

/// Returns true only to request the existing no-provider tick backoff.
pub(crate) async fn run_tick(date: NaiveDate, fresh_window: bool) -> Result<bool, String> {
    if !runtime::frozen_platform_features_enabled() {
        return Ok(false);
    }
    // Current Empty inspection remains first. Its completed/pending route must
    // not hide another real saved NonEmpty date requiring passive recovery.
    let empty = runtime::inspect_g5b_empty_tick_v2(date).await;
    let saved_dates = runtime::list_g5b_physical_cohort_dates_v2().await?;
    let historical = recover_dates_with(saved_dates, date, |saved| async move {
        run_physical_nonempty_tick(saved, false).await.map(|_| ())
    })
    .await;
    let current = route_empty_before_models(
        || std::future::ready(empty),
        || run_physical_nonempty_tick(date, fresh_window),
    )
    .await;
    match (historical, current) {
        (Ok(()), result) => result,
        (Err(history), Ok(_)) => Err(history),
        (Err(history), Err(current)) => {
            Err(format!("{history}; current G5b date={date}: {current}"))
        }
    }
}

async fn run_physical_nonempty_tick(date: NaiveDate, fresh_window: bool) -> Result<bool, String> {
    if let runtime::G5bPhysicalTickObservation::Sealed {
        seal_identity,
        cohort_identity,
        revision,
        count,
        reason,
    } = runtime::inspect_g5b_physical_tick_v2(date).await?
    {
        log::info!("[g5b] actual physical prefix freshly verified date={date} cohort={cohort_identity} seal={seal_identity} revision={revision} selected={count} reason={reason}");
        return Ok(false);
    }
    after_recovery_with(
        || run_nonempty_tick(date, fresh_window),
        || async move {
            let observed = runtime::finalize_g5b_physical_tick_v2(date).await?;
            log_physical_finalizer(date, observed);
            Ok(())
        },
    )
    .await
}

fn log_physical_finalizer(date: NaiveDate, observed: runtime::G5bPhysicalTickObservation) {
    match observed {
        runtime::G5bPhysicalTickObservation::Incomplete => {
            log::info!("[g5b] original physical completion remains unproven date={date}");
        }
        runtime::G5bPhysicalTickObservation::Sealed {
            seal_identity,
            cohort_identity,
            revision,
            count,
            reason,
        } => {
            log::info!("[g5b] fresh physical finalizer verified date={date} cohort={cohort_identity} seal={seal_identity} revision={revision} selected={count} reason={reason}");
        }
    }
}

// Scheduling helpers carry no receipt/capability. Actual production callbacks
// above consume only the original private-owner file/SQL readers.
async fn recover_dates_with<Dates, Recover, Future>(
    dates: Dates,
    current: NaiveDate,
    mut recover: Recover,
) -> Result<(), String>
where
    Dates: IntoIterator<Item = NaiveDate>,
    Recover: FnMut(NaiveDate) -> Future,
    Future: std::future::Future<Output = Result<(), String>>,
{
    let mut errors = Vec::new();
    for date in dates {
        if date == current {
            continue;
        }
        if let Err(e) = recover(date).await {
            errors.push(format!("saved G5b date={date}: {e}"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

async fn after_recovery_with<Recover, RecoverFuture, Finalize, FinalizeFuture>(
    recover: Recover,
    finalize: Finalize,
) -> Result<bool, String>
where
    Recover: FnOnce() -> RecoverFuture,
    RecoverFuture: std::future::Future<Output = Result<bool, String>>,
    Finalize: FnOnce() -> FinalizeFuture,
    FinalizeFuture: std::future::Future<Output = Result<(), String>>,
{
    let progress = recover().await;
    let finalizer = finalize().await;
    match (progress, finalizer) {
        (Ok(backoff), Ok(())) => Ok(backoff),
        (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
        (Err(progress), Err(finalizer)) => {
            Err(format!("{progress}; physical finalizer: {finalizer}"))
        }
    }
}

/// Empty initialization failure cannot disable ordinary scanners or NonEmpty
/// recovery. A legacy head without its original receipt stays Unknown.
pub(crate) async fn initialize_before_input_writers() {
    if !runtime::frozen_platform_features_enabled() {
        log::info!("[g5b] v2 disabled reason=user_scope_frozen; provider/sql/file operations=0");
        return;
    }
    match runtime::initialize_g5b_empty_before_input_writers().await {
        Ok(Some(date)) => log::info!("[g5b] prospective zero input observed for {date}"),
        Ok(None) => {}
        Err(e) => log::warn!("[g5b] Empty prospective observation unavailable: {e}"),
    }
}

async fn route_empty_before_models<Inspect, InspectFuture, Models, ModelsFuture>(
    inspect: Inspect,
    models: Models,
) -> Result<bool, String>
where
    Inspect: FnOnce() -> InspectFuture,
    InspectFuture: std::future::Future<Output = Result<runtime::G5bEmptyTickObservation, String>>,
    Models: FnOnce() -> ModelsFuture,
    ModelsFuture: std::future::Future<Output = Result<bool, String>>,
{
    match inspect().await? {
        runtime::G5bEmptyTickObservation::ContinueNonEmpty => models().await,
        runtime::G5bEmptyTickObservation::Pending {
            cohort_identity,
            revision,
        } => {
            log::info!("[g5b] original Empty Selection remains pending cohort={cohort_identity} revision={revision}");
            Ok(false)
        }
        runtime::G5bEmptyTickObservation::Sealed {
            seal_identity,
            revision,
            reason,
        } => {
            log::info!("[g5b] closed zero prefix freshly verified seal={seal_identity} revision={revision} reason={reason}");
            Ok(false)
        }
    }
}

async fn run_nonempty_tick(date: NaiveDate, fresh_window: bool) -> Result<bool, String> {
    let saved_count = runtime::inspect_g5b_cohort_v2(date).await?;
    let mut first_claim = None;
    let count = match saved_count {
        Some(count) => count,
        None if !fresh_window => return Ok(false),
        None => {
            use stock_analysis::llm::registry::LlmRegistry;
            use stock_analysis::monitor::alert_log::read_today_records;
            use stock_analysis::monitor::attribution_deep::{
                top_events_for_deep, DEEP_ATTRIBUTION_MAX_EVENTS,
            };
            // This is only a scheduling preflight. The private claim owner
            // independently selects the actual locked current raw prefix.
            let eligible = tokio::task::spawn_blocking(|| {
                !top_events_for_deep(read_today_records(), DEEP_ATTRIBUTION_MAX_EVENTS).is_empty()
            })
            .await
            .map_err(|e| format!("join G5b input preflight: {e}"))?;
            if !eligible {
                return Ok(false);
            }
            if LlmRegistry::from_env().select("g5b").is_none() {
                return Ok(true);
            }
            first_claim = Some(runtime::claim_g5b_analysis_v2(date, 0).await?);
            runtime::inspect_g5b_cohort_v2(date)
                .await?
                .ok_or_else(|| "G5b original cohort absent after claim".to_owned())?
        }
    };
    runtime::archive_g5b_v2(date).await?;
    // Recover every saved handoff before any fresh member can fail its
    // provider/window check. A gap at an earlier index cannot hide later data.
    let pending = recover_members_with(count, |index| async move {
        if let Some(view) = runtime::inspect_g5b_dispatch_v2(date, index).await? {
            dispatch(view).await;
            Ok(true)
        } else {
            Ok(false)
        }
    })
    .await?;
    if !pending.contains(&0) {
        first_claim = None;
    }
    if !fresh_window {
        return Ok(false);
    }
    for index in pending {
        let claim = match first_claim.take() {
            Some(claim) if index == 0 => claim,
            Some(_) => return Err("G5b original first work lost its member index".to_owned()),
            None => match runtime::claim_g5b_analysis_v2(date, index).await {
                Ok(claim) => claim,
                Err(e) => {
                    log::warn!("[g5b] v2 member {index} cannot start a fresh attempt: {e}");
                    continue;
                }
            },
        };
        match claim {
            G5bAnalysisClaimV2::Ready(work) => {
                // The actual assess preflight contains bounded synchronous
                // file/SQL checks, then drops its guard before the provider.
                let handle = tokio::runtime::Handle::current();
                let result = tokio::task::spawn_blocking(move || handle.block_on(work.assess()))
                    .await
                    .map_err(|e| format!("join G5b model worker: {e}"))?;
                match result {
                    Ok(completed) => runtime::freeze_g5b_analysis_v2(date, completed).await?,
                    Err(e) => {
                        log::warn!(
                            "[g5b] v2 member {index} original attempt remains consumed: {e}"
                        );
                        continue;
                    }
                }
            }
            G5bAnalysisClaimV2::Frozen(_) => runtime::archive_g5b_v2(date).await?,
            G5bAnalysisClaimV2::CompletionUnproven {
                occurrence_identity,
            } => {
                log::warn!("[g5b] v2 original attempt completion unproven: {occurrence_identity}");
                continue;
            }
        }
        if let Some(view) = runtime::inspect_g5b_dispatch_v2(date, index).await? {
            dispatch(view).await;
        }
    }
    Ok(false)
}

async fn dispatch(view: stock_analysis::monitor::g5b_analysis_v2::G5bModelDispatchViewV2) {
    let token = match crate::presentation_registry::acquire_token(
        "G5b-attribution-deep",
        crate::notify::PushKind::G5bAttribution,
        "g5b_attribution_dispatcher",
        "render_deep_attribution",
    ) {
        Ok(token) => token,
        Err(e) => {
            log::error!("[g5b] v2 presentation rejected: {e}");
            return;
        }
    };
    let outcome = crate::notify::push_g5b_model_dispatch_v2(token, view).await;
    log::info!("[g5b] v2 counted delivery observation: {outcome:?}");
}

// The bool here schedules model work only; no callback can mint a receipt,
// counted owner or completion capability. Production consumes actual views.
async fn recover_members_with<Inspect, Future>(
    count: usize,
    mut inspect: Inspect,
) -> Result<Vec<usize>, String>
where
    Inspect: FnMut(usize) -> Future,
    Future: std::future::Future<Output = Result<bool, String>>,
{
    let mut pending = Vec::new();
    for index in 0..count {
        if !inspect(index).await? {
            pending.push(index);
        }
    }
    Ok(pending)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn g5b_physical_v2_bin_saved_date_failure_does_not_hide_other_date_or_repeat_current() {
        let current = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        let older = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let later = NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();
        let seen = std::cell::RefCell::new(Vec::new());
        let error = recover_dates_with([older, current, later], current, |date| {
            seen.borrow_mut().push(date);
            std::future::ready(if date == older {
                Err("TEST_CODE original committed file unavailable".into())
            } else {
                Ok(())
            })
        })
        .await
        .unwrap_err();
        assert_eq!(*seen.borrow(), vec![older, later]);
        assert!(
            error.contains("2026-09-28") && error.contains("original committed file unavailable")
        );
    }

    #[tokio::test]
    async fn g5b_physical_v2_bin_post_recovery_finalizer_runs_without_fresh_model_work() {
        let calls = std::cell::RefCell::new(Vec::new());
        let result = after_recovery_with(
            || {
                calls
                    .borrow_mut()
                    .push("original stored recovery outside fresh window");
                std::future::ready(Ok(false))
            },
            || {
                calls
                    .borrow_mut()
                    .push("fresh physical reader and finalizer");
                std::future::ready(Ok(()))
            },
        )
        .await
        .unwrap();
        assert!(
            !result,
            "only the original no-provider path requests backoff"
        );
        assert_eq!(
            *calls.borrow(),
            vec![
                "original stored recovery outside fresh window",
                "fresh physical reader and finalizer"
            ]
        );
    }

    #[tokio::test]
    async fn g5b_physical_v2_bin_progress_and_finalizer_errors_remain_visible() {
        let calls = std::cell::Cell::new(0);
        let error = after_recovery_with(
            || std::future::ready(Err("TEST_CODE original source failed".into())),
            || {
                calls.set(calls.get() + 1);
                std::future::ready(Err("TEST_CODE known prefix verification failed".into()))
            },
        )
        .await
        .unwrap_err();
        assert_eq!(calls.get(), 1);
        assert!(
            error.contains("original source failed")
                && error.contains("known prefix verification failed")
        );
    }

    #[test]
    fn g5b_physical_v2_bin_real_callback_order_supplements_protocol_fixtures() {
        let source = include_str!("g5b_v2.rs");
        let tick = &source[source.find("pub(crate) async fn run_tick").unwrap()
            ..source.find("async fn run_physical_nonempty_tick").unwrap()];
        assert!(
            tick.find("inspect_g5b_empty_tick_v2").unwrap()
                < tick.find("list_g5b_physical_cohort_dates_v2").unwrap()
        );
        assert!(
            tick.find("run_physical_nonempty_tick(saved, false)")
                .unwrap()
                < tick
                    .find("run_physical_nonempty_tick(date, fresh_window)")
                    .unwrap()
        );
        let physical = &source[source.find("async fn run_physical_nonempty_tick").unwrap()
            ..source.find("async fn recover_dates_with").unwrap()];
        assert!(
            physical.find("inspect_g5b_physical_tick_v2").unwrap()
                < physical
                    .find("run_nonempty_tick(date, fresh_window)")
                    .unwrap()
        );
        assert!(
            physical
                .find("run_nonempty_tick(date, fresh_window)")
                .unwrap()
                < physical.find("finalize_g5b_physical_tick_v2").unwrap()
        );
        let runtime = include_str!("durable_delivery_runtime/g5b_physical_v2.rs");
        assert!(!runtime.contains(".prepare("));
        assert!(!runtime.contains("resume_deliverable("));
        assert!(!runtime.contains("known.by_date.remove("));
    }

    #[tokio::test]
    async fn g5b_empty_runtime_saved_routing_precedes_models_and_provider_backoff() {
        // Only low authority scheduling observations are supplied here. Actual
        // Pending recovery/Sealed refresh and source-inode authority are covered
        // by the library's real g5b_empty_facade_behavior_tests Fixture cases.
        for observation in [
            runtime::G5bEmptyTickObservation::Pending {
                cohort_identity: "routing-only".to_owned(),
                revision: 1,
            },
            runtime::G5bEmptyTickObservation::Sealed {
                seal_identity: "routing-only".to_owned(),
                revision: 1,
                reason: "routing-only",
            },
        ] {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let read = Arc::clone(&calls);
            let models = Arc::clone(&calls);
            let result = route_empty_before_models(
                move || async move {
                    read.lock().unwrap().push("inspect");
                    Ok(observation)
                },
                move || async move {
                    models.lock().unwrap().push("model");
                    Ok(true)
                },
            )
            .await
            .unwrap();
            assert!(
                !result,
                "Empty routing must not request no-provider backoff"
            );
            assert_eq!(*calls.lock().unwrap(), vec!["inspect"]);
        }
        let calls = Arc::new(Mutex::new(Vec::new()));
        let read = Arc::clone(&calls);
        let models = Arc::clone(&calls);
        let result = route_empty_before_models(
            move || async move {
                read.lock().unwrap().push("inspect");
                Ok(runtime::G5bEmptyTickObservation::ContinueNonEmpty)
            },
            move || async move {
                models.lock().unwrap().push("model");
                Ok(true)
            },
        )
        .await
        .unwrap();
        assert!(
            result,
            "only ordinary model preflight may request provider backoff"
        );
        assert_eq!(*calls.lock().unwrap(), vec!["inspect", "model"]);
    }

    #[tokio::test]
    async fn g5b_empty_runtime_failed_actual_observation_cannot_fall_through_to_models() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let output = Arc::clone(&calls);
        let error = route_empty_before_models(
            || async { Err("known positive source replaced".to_owned()) },
            move || async move {
                output.lock().unwrap().push("model");
                Ok(false)
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error, "known positive source replaced");
        assert!(calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn g5b_v2_runtime_saved_later_member_is_recovered_after_earlier_gaps() {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let output = Arc::clone(&observed);
        let pending = recover_members_with(3, move |index| {
            let output = Arc::clone(&output);
            async move {
                output.lock().unwrap().push(index);
                // Earlier attempt-only and untouched observations do not
                // suppress a later saved handoff. This fixture grants no authority.
                Ok(index == 2)
            }
        })
        .await
        .unwrap();
        assert_eq!(*observed.lock().unwrap(), vec![0, 1, 2]);
        assert_eq!(pending, vec![0, 1]);
    }
}
