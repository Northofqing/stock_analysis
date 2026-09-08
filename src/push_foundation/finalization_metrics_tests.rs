use std::time::Duration;

use super::finalization_metrics::*;
use super::finalization_sla::FinalizationSlaError;
use super::finalization_sla_tests::{
    accepted_result, micros, rejected_result, uncertain_result, Case, ACCEPTED,
};
use super::intent_store::NamespaceInventoryItem;
use super::{BusinessIntentStore, InitialDecisionKind, TerminalTemplateBinding};
use crate::monitor::push_job::{AuthorityClass, Namespace, RunId, TemplateId, TemplateVersion};

fn inspect_case(
    case: &Case,
    page_size: usize,
    max_intents: usize,
) -> Result<FinalizationMetricsReport, FinalizationMetricsError> {
    let bindings = [case.metrics_binding()];
    inspect_finalization_metrics(
        case.store(),
        FinalizationMetricsQuery {
            namespace: case.namespace(),
            observed_at: micros(ACCEPTED + 400_000_000),
            reconcile_cycle: Duration::from_secs(30),
            page_size,
            max_intents,
            bindings: &bindings,
        },
    )
}

fn query_for_case<'a>(
    case: &'a Case,
    page_size: usize,
    max_intents: usize,
    reconcile_cycle: Duration,
    bindings: &'a [FinalizationMetricsBinding<'a>],
) -> FinalizationMetricsQuery<'a> {
    FinalizationMetricsQuery {
        namespace: case.namespace(),
        observed_at: micros(ACCEPTED),
        reconcile_cycle,
        page_size,
        max_intents,
        bindings,
    }
}

#[test]
fn actual_generic_p01_and_n02_sources_enter_inventory_metrics_without_writes() {
    for class in [
        AuthorityClass::GenericCounted,
        AuthorityClass::P01Dedicated,
        AuthorityClass::N02Dedicated,
    ] {
        let mut case = Case::new(class, accepted_result(ACCEPTED), true);
        let before = case.rows();
        let audit_before = case.audit_bytes();
        let effects_before = case.effect_counts();
        let first = inspect_case(&case, 1, 1).unwrap();
        assert_eq!(first.inventory_total(), 1);
        assert_eq!(first.checked(), 1);
        assert_eq!(first.unchecked(), 0);
        assert_eq!(first.coverage(), InventoryCoverage::Complete);
        assert_eq!(first.dispositions().accepted(), 1);
        assert_eq!(first.sla_statuses().awaiting_finalization(), 1);
        assert_eq!(first.errors().total(), 0);
        assert_eq!(before, case.rows());
        assert_eq!(audit_before, case.audit_bytes());
        assert_eq!(effects_before, case.effect_counts());

        case.restart();
        let second = inspect_case(&case, 1, 1).unwrap();
        assert_eq!(first, second);
        assert_eq!(before, case.rows());
        assert_eq!(audit_before, case.audit_bytes());
        assert_eq!(effects_before, case.effect_counts());
    }
}

#[test]
fn n02_cold_read_rejects_missing_retained_lock_without_creating_files() {
    let single = Case::new(
        AuthorityClass::N02Dedicated,
        accepted_result(ACCEPTED),
        true,
    );
    let single_retained = single.retain_audit_lock_outside_authority(2026);
    let single_retained_before = std::fs::read(&single_retained).unwrap();
    let single_before = single.audit_directory_snapshot();
    let single_result = single.query(ACCEPTED + 400_000_000, Duration::from_secs(30));
    let single_after = single.audit_directory_snapshot();
    let single_retained_after = std::fs::read(&single_retained).unwrap();

    let batch = Case::new(
        AuthorityClass::N02Dedicated,
        accepted_result(ACCEPTED),
        true,
    );
    let batch_retained = batch.retain_audit_lock_outside_authority(2026);
    let batch_retained_before = std::fs::read(&batch_retained).unwrap();
    let batch_before = batch.audit_directory_snapshot();
    let batch_report = inspect_case(&batch, 1, 1).unwrap();
    let batch_after = batch.audit_directory_snapshot();
    let batch_retained_after = std::fs::read(&batch_retained).unwrap();

    assert_eq!(single_result, Err(FinalizationSlaError::SourceInvalid));
    assert_eq!(single_before, single_after);
    assert_eq!(single_retained_before, single_retained_after);
    assert_eq!(batch_report.checked(), 1);
    assert_eq!(batch_report.errors().source_invalid(), 1);
    assert_eq!(batch_report.errors().total(), 1);
    assert_eq!(batch_before, batch_after);
    assert_eq!(batch_retained_before, batch_retained_after);
}

#[test]
fn pending_rejected_uncertain_and_manual_dispositions_remain_separate_axes() {
    let pending = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        false,
    );
    let pending_report = inspect_case(&pending, 1, 1).unwrap();
    assert_eq!(pending_report.sla_statuses().pending_seal(), 1);
    assert_eq!(pending_report.dispositions().accepted(), 0);

    let rejected = Case::new(
        AuthorityClass::GenericCounted,
        rejected_result(ACCEPTED),
        true,
    );
    let rejected_report = inspect_case(&rejected, 1, 1).unwrap();
    assert_eq!(rejected_report.sla_statuses().rejected(), 1);
    assert_eq!(rejected_report.dispositions().rejected(), 1);
    assert_eq!(rejected_report.dispositions().accepted(), 0);

    let uncertain = Case::new(
        AuthorityClass::GenericCounted,
        uncertain_result(ACCEPTED),
        true,
    );
    let uncertain_report = inspect_case(&uncertain, 1, 1).unwrap();
    assert_eq!(uncertain_report.sla_statuses().uncertain(), 1);
    assert_eq!(uncertain_report.dispositions().uncertain(), 1);
    assert_eq!(uncertain_report.max_pending_accepted_age(), None);

    for accepted in [true, false] {
        let case = Case::new(
            AuthorityClass::GenericCounted,
            uncertain_result(ACCEPTED),
            true,
        );
        case.resolve_manual(accepted);
        let report = inspect_case(&case, 1, 1).unwrap();
        if accepted {
            assert_eq!(report.sla_statuses().manual_accepted(), 1);
            assert_eq!(report.dispositions().manual_accepted(), 1);
        } else {
            assert_eq!(report.sla_statuses().manual_not_delivered(), 1);
            assert_eq!(report.dispositions().manual_not_delivered(), 1);
        }
        assert_eq!(report.dispositions().accepted(), 0);
        assert_eq!(report.max_pending_accepted_age(), None);
    }
}

#[test]
fn namespace_inventory_pages_all_business_days_states_and_reports_exact_coverage() {
    let mut case = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
    );
    let namespace = case.namespace().clone();
    case.add_inventory_intent(
        namespace.clone(),
        InitialDecisionKind::NoData,
        "2026-08-17",
        "no-data",
    );
    case.add_inventory_intent(
        namespace.clone(),
        InitialDecisionKind::Disabled,
        "2026-08-19",
        "disabled",
    );
    case.add_inventory_intent(
        namespace.clone(),
        InitialDecisionKind::Ready,
        "2026-08-20",
        "missing-source",
    );
    let other_namespace =
        Namespace::test(RunId::try_new("TEST_CODE_W19_OTHER".to_owned()).unwrap());
    case.add_inventory_intent(
        other_namespace,
        InitialDecisionKind::NoData,
        "2026-08-16",
        "isolated",
    );

    let complete = inspect_case(&case, 1, 4).unwrap();
    assert_eq!(complete.namespace(), &namespace);
    assert_eq!(complete.observed_at(), micros(ACCEPTED + 400_000_000));
    assert_eq!(complete.reconcile_cycle(), Duration::from_secs(30));
    assert_eq!(complete.two_cycle_target(), Duration::from_secs(60));
    assert_eq!(complete.inventory_total(), 4);
    assert_eq!(complete.checked(), 4);
    assert_eq!(complete.unchecked(), 0);
    assert_eq!(complete.coverage(), InventoryCoverage::Complete);
    let states = complete.business_states();
    assert_eq!(states.pending_dispatch(), 2);
    assert_eq!(states.no_data(), 1);
    assert_eq!(states.disabled(), 1);
    assert_eq!(states.awaiting_authority(), 0);
    assert_eq!(states.awaiting_finalizer(), 0);
    assert_eq!(states.completed(), 0);
    assert_eq!(states.not_delivered(), 0);
    assert_eq!(states.resolution_required(), 0);
    let statuses = complete.sla_statuses();
    assert_eq!(statuses.awaiting_finalization(), 1);
    assert_eq!(statuses.missing(), 1);
    assert_eq!(statuses.not_applicable(), 2);
    assert_eq!(complete.dispositions().accepted(), 1);
    assert_eq!(complete.errors().total(), 0);
    assert_eq!(complete.target_exceeded(), 1);
    assert_eq!(complete.hard_limit_reached(), 1);
    assert_eq!(complete.requires_block(), 1);
    assert_eq!(
        complete.max_pending_accepted_age(),
        Some(Duration::from_micros(400_000_000))
    );

    for (budget, expected_checked, expected_unchecked, coverage) in [
        (1, 1, 3, InventoryCoverage::Limited),
        (3, 3, 1, InventoryCoverage::Limited),
        (4, 4, 0, InventoryCoverage::Complete),
        (5, 4, 0, InventoryCoverage::Complete),
    ] {
        let report = inspect_case(&case, 2, budget).unwrap();
        assert_eq!(report.inventory_total(), 4);
        assert_eq!(report.checked(), expected_checked);
        assert_eq!(report.unchecked(), expected_unchecked);
        assert_eq!(report.coverage(), coverage);
        assert_eq!(
            report.checked(),
            report.sla_statuses().not_applicable()
                + report.sla_statuses().missing()
                + report.sla_statuses().awaiting_finalization()
                + report.errors().total()
        );
    }
}

#[test]
fn one_actual_generic_inventory_mixes_terminal_pending_manual_and_history_facts() {
    let mut case = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
    );
    let completed = case.add_generic_authority_intent(
        accepted_result(ACCEPTED),
        true,
        "2026-08-17",
        "completed",
    );
    case.complete_added_accepted(&completed, ACCEPTED + 20_000_000);
    let resolution = case.add_generic_authority_intent(
        accepted_result(ACCEPTED),
        true,
        "2026-08-19",
        "completed-then-resolution",
    );
    case.complete_added_accepted(&resolution, ACCEPTED + 30_000_000);
    case.require_added_resolution_after_completed(&resolution, ACCEPTED + 40_000_000);
    let manual_not_delivered = case.add_generic_authority_intent(
        uncertain_result(ACCEPTED),
        true,
        "2026-08-20",
        "manual-not-delivered",
    );
    case.complete_added_not_delivered(&manual_not_delivered, ACCEPTED + 60_000_000);
    let namespace = case.namespace().clone();
    case.add_inventory_intent(
        namespace.clone(),
        InitialDecisionKind::Ready,
        "2026-08-21",
        "missing",
    );
    case.add_inventory_intent(
        namespace.clone(),
        InitialDecisionKind::NoData,
        "2026-08-16",
        "no-data",
    );
    case.add_inventory_intent(
        namespace.clone(),
        InitialDecisionKind::Disabled,
        "2026-08-22",
        "disabled",
    );
    case.add_generic_authority_intent(
        accepted_result(ACCEPTED),
        false,
        "2026-08-23",
        "pending-seal",
    );

    let before = case.rows();
    let audit_before = case.audit_bytes();
    let effects_before = case.effect_counts();
    let report = inspect_case(&case, 2, 8).unwrap();
    assert_eq!(report.inventory_total(), 8);
    assert_eq!(report.checked(), 8);
    assert_eq!(report.unchecked(), 0);
    assert_eq!(report.coverage(), InventoryCoverage::Complete);
    let states = report.business_states();
    assert_eq!(states.pending_dispatch(), 3);
    assert_eq!(states.completed(), 1);
    assert_eq!(states.not_delivered(), 1);
    assert_eq!(states.no_data(), 1);
    assert_eq!(states.disabled(), 1);
    assert_eq!(states.resolution_required(), 1);
    assert_eq!(states.awaiting_authority(), 0);
    assert_eq!(states.awaiting_finalizer(), 0);
    let statuses = report.sla_statuses();
    assert_eq!(statuses.awaiting_finalization(), 1);
    assert_eq!(statuses.completed(), 1);
    assert_eq!(statuses.manual_not_delivered(), 1);
    assert_eq!(statuses.resolution_required(), 1);
    assert_eq!(statuses.missing(), 1);
    assert_eq!(statuses.pending_seal(), 1);
    assert_eq!(statuses.not_applicable(), 2);
    assert_eq!(
        report.checked(),
        statuses.awaiting_finalization()
            + statuses.completed()
            + statuses.manual_not_delivered()
            + statuses.resolution_required()
            + statuses.missing()
            + statuses.pending_seal()
            + statuses.not_applicable()
            + report.errors().total()
    );
    assert_eq!(report.dispositions().accepted(), 3);
    assert_eq!(report.dispositions().manual_not_delivered(), 1);
    assert_eq!(report.errors().total(), 0);
    assert_eq!(
        report.max_completed_latency(),
        Some(Duration::from_secs(30))
    );
    assert_eq!(
        report.max_pending_accepted_age(),
        Some(Duration::from_secs(400))
    );
    assert_eq!(report.target_exceeded(), 1);
    assert_eq!(report.hard_limit_reached(), 1);
    assert_eq!(report.requires_block(), 2);
    assert_eq!(before, case.rows());
    assert_eq!(audit_before, case.audit_bytes());
    assert_eq!(effects_before, case.effect_counts());

    case.restart();
    let restarted = inspect_case(&case, 2, 8).unwrap();
    assert_eq!(report, restarted);
    assert_eq!(before, case.rows());
    assert_eq!(effects_before, case.effect_counts());
}

#[test]
fn same_unit_multiple_persisted_templates_select_exact_historical_binding() {
    let mut case = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
    );
    let historical_template = TerminalTemplateBinding::new(
        TemplateId::try_new("auction-card-historical".to_owned()).unwrap(),
        TemplateVersion::try_new("auction-card-historical-v1".to_owned()).unwrap(),
    );
    let historical = case.inventory_draft_with_template(
        case.namespace().clone(),
        InitialDecisionKind::NoData,
        "2026-08-17",
        "historical-template",
        &historical_template,
    );
    case.add_inventory_draft(&historical);

    let current = case.metrics_binding();
    let historical_source = case.metrics_binding();
    let bindings = [
        current,
        FinalizationMetricsBinding {
            unit: historical_source.unit,
            template: &historical_template,
            policy: historical_source.policy,
            source: historical_source.source,
        },
    ];
    let report = inspect_finalization_metrics(
        case.store(),
        FinalizationMetricsQuery {
            namespace: case.namespace(),
            observed_at: micros(ACCEPTED + 1),
            reconcile_cycle: Duration::from_micros(1),
            page_size: 1,
            max_intents: 2,
            bindings: &bindings,
        },
    )
    .unwrap();
    assert_eq!(report.inventory_total(), 2);
    assert_eq!(report.sla_statuses().awaiting_finalization(), 1);
    assert_eq!(report.sla_statuses().not_applicable(), 1);
    assert_eq!(report.errors().total(), 0);

    let current_only = [case.metrics_binding()];
    let missing_historical_route = inspect_finalization_metrics(
        case.store(),
        FinalizationMetricsQuery {
            namespace: case.namespace(),
            observed_at: micros(ACCEPTED + 1),
            reconcile_cycle: Duration::from_micros(1),
            page_size: 2,
            max_intents: 2,
            bindings: &current_only,
        },
    )
    .unwrap();
    assert_eq!(missing_historical_route.errors().route_mismatch(), 1);
    assert_eq!(missing_historical_route.errors().missing_binding(), 0);
}

#[test]
fn empty_inventory_is_complete_inventory_fact_not_a_health_claim() {
    let case = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
    );
    let empty_namespace =
        Namespace::test(RunId::try_new("TEST_CODE_W19_EMPTY".to_owned()).unwrap());
    let bindings = [case.metrics_binding()];
    let report = inspect_finalization_metrics(
        case.store(),
        FinalizationMetricsQuery {
            namespace: &empty_namespace,
            observed_at: micros(ACCEPTED),
            reconcile_cycle: Duration::from_micros(1),
            page_size: 1,
            max_intents: 1,
            bindings: &bindings,
        },
    )
    .unwrap();
    assert_eq!(report.inventory_total(), 0);
    assert_eq!(report.checked(), 0);
    assert_eq!(report.unchecked(), 0);
    assert_eq!(report.coverage(), InventoryCoverage::Complete);
    assert_eq!(report.errors().total(), 0);
    assert!(!format!("{report:?}").contains("health"));
}

#[test]
fn invalid_limits_cycle_and_duplicate_or_mismatched_binding_bindings_fail_before_read() {
    let case = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
    );
    let one = [case.metrics_binding()];
    assert_eq!(
        inspect_finalization_metrics(
            case.store(),
            query_for_case(&case, 0, 1, Duration::from_micros(1), &one)
        ),
        Err(FinalizationMetricsError::InvalidPageSize)
    );
    assert_eq!(
        inspect_finalization_metrics(
            case.store(),
            query_for_case(&case, 1_001, 1, Duration::from_micros(1), &one)
        ),
        Err(FinalizationMetricsError::InvalidPageSize)
    );
    assert_eq!(
        inspect_finalization_metrics(
            case.store(),
            query_for_case(&case, 1, 0, Duration::from_micros(1), &one)
        ),
        Err(FinalizationMetricsError::InvalidMaxIntents)
    );
    assert_eq!(
        inspect_finalization_metrics(
            case.store(),
            query_for_case(&case, 1, 100_001, Duration::from_micros(1), &one)
        ),
        Err(FinalizationMetricsError::InvalidMaxIntents)
    );
    assert_eq!(
        inspect_finalization_metrics(
            case.store(),
            query_for_case(&case, 1, 1, Duration::ZERO, &one)
        ),
        Err(FinalizationMetricsError::InvalidCycle)
    );
    let duplicate = [case.metrics_binding(), case.metrics_binding()];
    assert_eq!(
        inspect_finalization_metrics(
            case.store(),
            query_for_case(&case, 1, 1, Duration::from_micros(1), &duplicate)
        ),
        Err(FinalizationMetricsError::DuplicateBinding)
    );
    let p01 = Case::new(
        AuthorityClass::P01Dedicated,
        accepted_result(ACCEPTED),
        true,
    );
    let generic_binding = case.metrics_binding();
    let p01_binding = p01.metrics_binding();
    let mismatched = [FinalizationMetricsBinding {
        unit: generic_binding.unit,
        template: generic_binding.template,
        policy: generic_binding.policy,
        source: p01_binding.source,
    }];
    assert_eq!(
        inspect_finalization_metrics(
            case.store(),
            query_for_case(&case, 1, 1, Duration::from_micros(1), &mismatched)
        ),
        Err(FinalizationMetricsError::InvalidBinding)
    );
}

#[test]
fn missing_binding_wrong_template_and_unsupported_n02_route_stay_in_error_denominator() {
    let generic = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
    );
    let missing = inspect_finalization_metrics(
        generic.store(),
        FinalizationMetricsQuery {
            namespace: generic.namespace(),
            observed_at: micros(ACCEPTED + 1),
            reconcile_cycle: Duration::from_micros(1),
            page_size: 1,
            max_intents: 1,
            bindings: &[],
        },
    )
    .unwrap();
    assert_eq!(missing.checked(), 1);
    assert_eq!(missing.errors().missing_binding(), 1);
    assert_eq!(missing.errors().total(), 1);

    let wrong_template = TerminalTemplateBinding::new(
        TemplateId::try_new("wrong-template".to_owned()).unwrap(),
        TemplateVersion::try_new("wrong-template-v1".to_owned()).unwrap(),
    );
    let base = generic.metrics_binding();
    let wrong = [FinalizationMetricsBinding {
        unit: base.unit,
        template: &wrong_template,
        policy: base.policy,
        source: base.source,
    }];
    let wrong_report = inspect_finalization_metrics(
        generic.store(),
        FinalizationMetricsQuery {
            namespace: generic.namespace(),
            observed_at: micros(ACCEPTED + 1),
            reconcile_cycle: Duration::from_micros(1),
            page_size: 1,
            max_intents: 1,
            bindings: &wrong,
        },
    )
    .unwrap();
    assert_eq!(wrong_report.errors().route_mismatch(), 1);

    let unsupported = Case::variant(
        AuthorityClass::N02Dedicated,
        accepted_result(ACCEPTED),
        true,
        InitialDecisionKind::Ready,
        Some("unknown-news-family"),
    );
    let unsupported_report = inspect_case(&unsupported, 1, 1).unwrap();
    assert_eq!(unsupported_report.checked(), 1);
    assert_eq!(
        unsupported_report.errors().unsupported_occurrence_route(),
        1
    );
    assert_eq!(unsupported_report.errors().total(), 1);

    let unsupported_window = Case::variant_occurrence(
        AuthorityClass::N02Dedicated,
        accepted_result(ACCEPTED),
        true,
        InitialDecisionKind::Ready,
        None,
        Some("16:00"),
    );
    let unsupported_window_report = inspect_case(&unsupported_window, 1, 1).unwrap();
    assert_eq!(unsupported_window_report.checked(), 1);
    assert_eq!(
        unsupported_window_report
            .errors()
            .unsupported_occurrence_route(),
        1
    );
}

#[test]
fn corrupted_transition_chain_is_counted_once_after_schema_is_restored() {
    let mut case = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
    );
    case.complete(ACCEPTED + 10_000_000);
    case.corrupt_first_transition_hash();
    case.restart();

    let report = inspect_case(&case, 1, 1).unwrap();
    assert_eq!(report.checked(), 1);
    assert_eq!(report.errors().persisted_chain_invalid(), 1);
    assert_eq!(report.errors().total(), 1);
    let debug = format!("{report:?}");
    assert!(!debug.contains(case.business_path().to_str().unwrap()));
    assert!(!debug.contains("push_intent_transitions"));
}

#[test]
fn corrupted_snapshot_identity_is_counted_once_after_schema_is_restored() {
    let mut case = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
    );
    case.corrupt_persisted_intent_identity();
    case.restart();

    let report = inspect_case(&case, 1, 1).unwrap();
    assert_eq!(report.checked(), 1);
    assert_eq!(report.errors().persisted_snapshot_invalid(), 1);
    assert_eq!(report.errors().total(), 1);
    let debug = format!("{report:?}");
    assert!(!debug.contains("TEST_CODE_INVALID_DECISION"));
    assert!(!debug.contains(case.business_path().to_str().unwrap()));
}

#[test]
fn second_connection_change_during_page_visit_is_excluded_from_existing_snapshot() {
    let mut case = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
    );
    let namespace = case.namespace().clone();
    case.add_inventory_intent(
        namespace.clone(),
        InitialDecisionKind::NoData,
        "2026-08-17",
        "snapshot-a",
    );
    case.add_inventory_intent(
        namespace.clone(),
        InitialDecisionKind::Disabled,
        "2026-08-19",
        "snapshot-b",
    );
    let pragma = rusqlite::Connection::open(case.business_path()).unwrap();
    let mode: String = pragma
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    drop(pragma);
    let late = case.inventory_draft(
        namespace.clone(),
        InitialDecisionKind::NoData,
        "2026-08-20",
        "snapshot-late",
    );
    let mut writer = BusinessIntentStore::open(case.business_path()).unwrap();
    let mut visited = 0_u64;
    let mut inserted = false;
    let read = case
        .store()
        .scan_namespace_inventory(&namespace, 1, 100, |item| {
            assert!(matches!(item, NamespaceInventoryItem::Verified(_)));
            visited += 1;
            if !inserted {
                writer.record_initial(&late).unwrap();
                inserted = true;
            }
        })
        .unwrap();
    assert!(inserted);
    assert_eq!(visited, 3);
    assert_eq!(read.total(), 3);
    assert_eq!(read.checked(), 3);
    assert_eq!(writer.intent_count().unwrap(), 4);
}

#[test]
fn completed_latency_is_separate_from_current_pending_accepted_age() {
    let mut completed = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
    );
    completed.complete(ACCEPTED + 20_000_000);
    let completed_report = inspect_case(&completed, 1, 1).unwrap();
    assert_eq!(completed_report.sla_statuses().completed(), 1);
    assert_eq!(completed_report.dispositions().accepted(), 1);
    assert_eq!(
        completed_report.max_completed_latency(),
        Some(Duration::from_micros(20_000_000))
    );
    assert_eq!(completed_report.max_pending_accepted_age(), None);
    assert_eq!(completed_report.target_exceeded(), 0);
    assert_eq!(completed_report.hard_limit_reached(), 0);
    assert_eq!(completed_report.requires_block(), 0);

    let mut reopened = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
    );
    reopened.complete(ACCEPTED + 20_000_000);
    reopened.require_resolution_after_completed(ACCEPTED + 30_000_000);
    let reopened_report = inspect_case(&reopened, 1, 1).unwrap();
    assert_eq!(reopened_report.sla_statuses().completed(), 0);
    assert_eq!(reopened_report.sla_statuses().resolution_required(), 1);
    assert_eq!(reopened_report.dispositions().accepted(), 1);
    assert_eq!(
        reopened_report.max_completed_latency(),
        Some(Duration::from_micros(20_000_000))
    );
    assert_eq!(
        reopened_report.max_pending_accepted_age(),
        Some(Duration::from_micros(400_000_000))
    );
    assert_eq!(reopened_report.requires_block(), 1);
}

#[test]
fn completed_business_conflict_keeps_current_accepted_backlog_age() {
    let mut case = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
    );
    case.complete_with_historical_terminal_mismatch("ref");
    let report = inspect_case(&case, 1, 1).unwrap();
    assert_eq!(report.business_states().completed(), 1);
    assert_eq!(report.sla_statuses().conflict(), 1);
    assert_eq!(report.sla_statuses().completed(), 0);
    assert_eq!(report.dispositions().accepted(), 1);
    assert_eq!(report.requires_block(), 1);
    assert_eq!(report.max_completed_latency(), None);
    assert_eq!(
        report.max_pending_accepted_age(),
        Some(Duration::from_secs(400))
    );
}

#[test]
fn clock_uncertain_rows_are_counted_without_valid_latency_samples() {
    let case = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
    );
    let bindings = [case.metrics_binding()];
    let report = inspect_finalization_metrics(
        case.store(),
        FinalizationMetricsQuery {
            namespace: case.namespace(),
            observed_at: micros(ACCEPTED - 1),
            reconcile_cycle: Duration::from_secs(30),
            page_size: 1,
            max_intents: 1,
            bindings: &bindings,
        },
    )
    .unwrap();
    assert_eq!(report.sla_statuses().clock_uncertain(), 1);
    assert_eq!(report.dispositions().accepted(), 1);
    assert_eq!(report.max_completed_latency(), None);
    assert_eq!(report.max_pending_accepted_age(), None);
    assert_eq!(report.target_exceeded(), 0);
    assert_eq!(report.hard_limit_reached(), 0);
}

#[test]
fn current_business_conflict_does_not_count_as_normal_completion() {
    let mut case = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
    );
    case.transition_ready_to_no_data(ACCEPTED + 1);
    let report = inspect_case(&case, 1, 1).unwrap();
    assert_eq!(report.business_states().no_data(), 1);
    assert_eq!(report.sla_statuses().conflict(), 1);
    assert_eq!(report.sla_statuses().completed(), 0);
    assert_eq!(report.dispositions().accepted(), 1);
    assert_eq!(report.requires_block(), 1);
}

#[test]
fn awaiting_authority_awaiting_finalizer_and_not_delivered_states_are_observed() {
    let mut awaiting_authority = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
    );
    awaiting_authority.claim();
    let authority_report = inspect_case(&awaiting_authority, 1, 1).unwrap();
    assert_eq!(authority_report.business_states().awaiting_authority(), 1);
    assert_eq!(authority_report.sla_statuses().awaiting_finalization(), 1);

    let mut awaiting_finalizer = Case::new(
        AuthorityClass::GenericCounted,
        accepted_result(ACCEPTED),
        true,
    );
    awaiting_finalizer.qualify_accepted(ACCEPTED + 1);
    let finalizer_report = inspect_case(&awaiting_finalizer, 1, 1).unwrap();
    assert_eq!(finalizer_report.business_states().awaiting_finalizer(), 1);
    assert_eq!(finalizer_report.sla_statuses().awaiting_finalization(), 1);

    let mut not_delivered = Case::new(
        AuthorityClass::GenericCounted,
        uncertain_result(ACCEPTED),
        true,
    );
    not_delivered.complete_not_delivered(ACCEPTED + 20_000_000);
    let not_delivered_report = inspect_case(&not_delivered, 1, 1).unwrap();
    assert_eq!(not_delivered_report.business_states().not_delivered(), 1);
    assert_eq!(
        not_delivered_report.sla_statuses().manual_not_delivered(),
        1
    );
    assert_eq!(
        not_delivered_report.dispositions().manual_not_delivered(),
        1
    );
    assert_eq!(not_delivered_report.dispositions().accepted(), 0);
}
