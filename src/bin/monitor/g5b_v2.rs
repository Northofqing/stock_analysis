//! Current-day model production and exact stored handoff; no day completion bit.
use crate::durable_delivery_runtime as runtime;
use chrono::NaiveDate;
use stock_analysis::monitor::g5b_analysis_v2::G5bAnalysisClaimV2;

/// Returns true only to request the existing no-provider tick backoff.
pub(crate) async fn run_tick(date: NaiveDate, fresh_window: bool) -> Result<bool, String> {
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
