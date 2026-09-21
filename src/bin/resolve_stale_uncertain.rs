//! 2026-09-21 one-shot: resolve stale `UncertainManualReview` HoldingPlan
//! decisions as Rejected.  Their cooldown heads sit in `Uncertain` forever and
//! the prepare admission denies every new-business-day decision
//! (`"Reserved" | "Uncertain" => true` conflicts), which blocked 5/7 holding
//! plan cards on the first live day (2026-09-21).
//!
//! Resolution = ManualDisposition::Rejected (the old cards are superseded;
//! the durable layer then releases the cooldown head so today's cycle can
//! admit a fresh decision).

use sha2::{Digest, Sha256};
use stock_analysis::durable_delivery::{
    CoordinatorConfig, DurableDeliveryCoordinator, ManualDisposition, ManualResolutionCommand,
};
use stock_analysis::event::DurableDeliveryImmutableAppend;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let append = DurableDeliveryImmutableAppend::for_production()
        .map_err(|error| format!("immutable append production bind failed: {error}"))?;
    let owner = hex::encode(Sha256::digest(
        format!("resolve_stale_uncertain:{}", std::process::id()).as_bytes(),
    ));
    let coordinator = DurableDeliveryCoordinator::open(CoordinatorConfig::production(owner))
        .map_err(|error| format!("durable coordinator open failed: {error}"))?;

    // List candidate decisions read-only via rusqlite.
    let connection = rusqlite::Connection::open("data/durable_delivery.sqlite3")?;
    let mut statement = connection.prepare(
        "SELECT decision_identity, scope_key, business_date
         FROM delivery_decisions
         WHERE state='UncertainManualReview' AND push_kind='HoldingPlan'
         ORDER BY business_date, scope_key",
    )?;
    let candidates: Vec<(String, String, String)> = statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<Result<_, _>>()?;
    drop(statement);
    drop(connection);

    if candidates.is_empty() {
        println!("no stale uncertain HoldingPlan decisions");
        return Ok(());
    }

    for (decision_identity, scope_key, business_date) in &candidates {
        let command = ManualResolutionCommand {
            decision_identity: decision_identity.clone(),
            disposition: ManualDisposition::Rejected,
            operator_identity: "claude-deploy-20260921-stale-uncertain".to_owned(),
            reason: "stale uncertain delivery superseded by new business day; \
                     release cooldown head for fresh admission"
                .to_owned(),
            external_evidence: b"stale-uncertain-resolution-v1".to_vec(),
            resolved_at: chrono::Utc::now(),
        };
        let state = coordinator
            .resolve_uncertain(&command, &append)
            .map_err(|error| {
                format!(
                    "resolve {decision_identity} ({scope_key} {business_date}) failed: {error}"
                )
            })?;
        println!("resolved {decision_identity} ({scope_key} {business_date}) -> {state:?}");
    }
    println!("resolved {} stale uncertain decisions", candidates.len());
    Ok(())
}
