//! BR-192 observable cutover guard.
//!
//! Counted delivery is an all-or-nothing production migration.  This test
//! intentionally inspects the production entry modules as a process-level
//! contract: the old in-memory budget owner must be gone and the monitor must
//! expose the durable coordinator at the sole counted-delivery seam.

#[test]
fn counted_delivery_has_one_durable_owner_and_no_process_local_budget() {
    let push_templates = include_str!("../src/bin/monitor/push_templates.rs");
    let notify = include_str!("../src/bin/monitor/notify.rs");
    let runtime = include_str!("../src/bin/monitor/durable_delivery_runtime.rs");

    for retired in [
        "DAILY_BUDGET_COUNT",
        "DAILY_BUDGET_DAY",
        "reset_budget_if_new_day",
        "counts_against_daily_budget",
    ] {
        assert!(
            !push_templates.contains(retired),
            "BR-192 counted cutover still exposes retired owner {retired}"
        );
    }

    assert!(
        notify.contains("ReviewProviderTopN"),
        "BR-192 counted cutover is missing ReviewProviderTopN"
    );
    let counted_branch = notify
        .find("if crate::durable_delivery_runtime::is_counted_kind(kind)")
        .expect("BR-192 counted branch");
    let legacy_audit_health = notify
        .find("runtime_delivery_audit_health()")
        .expect("BR-144 legacy audit health branch");
    let legacy_l6 = notify
        .find("crate::l6_sink::sink_router().route")
        .expect("legacy L6 route");
    assert!(
        counted_branch < legacy_audit_health && counted_branch < legacy_l6,
        "counted delivery must leave the legacy audit/L6 path before either owner is consulted"
    );
    for required in ["DurableDeliveryCoordinator", "reconcile_all_pending"] {
        assert!(
            runtime.contains(required),
            "BR-192 counted cutover is missing durable production seam {required}"
        );
    }
}
