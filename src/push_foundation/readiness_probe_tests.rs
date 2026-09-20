use crate::monitor::push_job::{MachineCatalog, MonitorKind, ProducerId};

use super::readiness_probe::{
    CandidateReadinessInventory, CandidateReadinessInventoryError, DependencyAvailability,
    ProducerDependencyReadiness, ProducerReadinessFact, ReceiptMode, ReceiptReadiness,
};

fn available_dependencies(
    required_receipt: ReceiptMode,
    observed_receipt: ReceiptMode,
) -> ProducerDependencyReadiness {
    ProducerDependencyReadiness {
        producer_binding: DependencyAvailability::Available,
        source_contract: DependencyAvailability::Available,
        schedule_or_trigger: DependencyAvailability::Available,
        presentation: DependencyAvailability::Available,
        durable_policy: DependencyAvailability::Available,
        receipt: ReceiptReadiness::Available {
            required: required_receipt,
            observed: observed_receipt,
        },
        feature_gate: DependencyAvailability::Available,
        completion_policy: DependencyAvailability::Available,
    }
}

fn available_fact(
    producer_id: ProducerId,
    required_receipt: ReceiptMode,
    observed_receipt: ReceiptMode,
) -> ProducerReadinessFact {
    ProducerReadinessFact::new(
        producer_id,
        available_dependencies(required_receipt, observed_receipt),
    )
}

fn producer(value: &str) -> ProducerId {
    ProducerId::try_new(value.to_owned()).expect("TEST_CODE producer ID")
}

fn complete_facts(catalog: &MachineCatalog) -> Vec<ProducerReadinessFact> {
    catalog
        .producers()
        .iter()
        .map(|registration| {
            available_fact(
                registration.id().clone(),
                ReceiptMode::Strong,
                ReceiptMode::Strong,
            )
        })
        .collect()
}

fn replace_dependencies(
    facts: &mut [ProducerReadinessFact],
    producer_id: &str,
    dependencies: ProducerDependencyReadiness,
) {
    let fact = facts
        .iter_mut()
        .find(|fact| fact.producer_id().as_str() == producer_id)
        .expect("TEST_CODE catalog producer fact");
    *fact = ProducerReadinessFact::new(producer(producer_id), dependencies);
}

#[test]
fn w15_inventory_bundled_catalog_derives_independent_kind_and_producer_denominators() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE bundled catalog");
    let facts = complete_facts(&catalog);

    let inventory = CandidateReadinessInventory::evaluate(&catalog, &facts)
        .expect("TEST_CODE complete catalog inventory");

    assert_eq!(inventory.push_total(), 65);
    assert_eq!(inventory.producer_total(), 102);
    assert_eq!(inventory.enum_external_producer_total(), 10);
    assert_eq!(inventory.ready(), 36);
    assert_eq!(inventory.conditional(), 7);
    assert_eq!(inventory.compat(), 0);
    assert_eq!(inventory.inactive(), 22);
    assert!(inventory
        .ready_kind_ids()
        .contains(&MonitorKind::Announcement));
    assert_eq!(
        inventory.conditional_kind_ids(),
        &[
            MonitorKind::VirtualWatch,
            MonitorKind::IndustryChain,
            MonitorKind::PostFixedPriceOrder,
            MonitorKind::PostFixedPriceFill,
            MonitorKind::PaperReview,
            MonitorKind::EarningsBeat,
            MonitorKind::EarningsMiss,
        ]
    );
    assert!(inventory
        .inactive_kind_ids()
        .contains(&MonitorKind::HoldingEvent));
    assert_eq!(inventory.schedule_unreachable(), 0);
    assert_eq!(inventory.producer_missing(), 0);
    assert_eq!(inventory.source_missing(), 0);
    assert_eq!(inventory.presentation_missing(), 0);
    assert_eq!(inventory.durable_policy_missing(), 0);
    assert_eq!(inventory.producer_facts().len(), 102);
    assert!(inventory
        .producer_facts()
        .windows(2)
        .all(|pair| pair[0].producer_id() < pair[1].producer_id()));

    let coverage = inventory.producer_coverage();
    assert_eq!(coverage.total(), 102);
    assert_eq!(coverage.ready_ids().len(), 89);
    assert_eq!(coverage.conditional_or_disabled_ids().len(), 13);
    assert!(coverage.unassessed_ids().is_empty());
    assert!(coverage.failed_ids().is_empty());

    let external = inventory.enum_external_producer_coverage();
    assert_eq!(external.total(), 10);
    assert_eq!(
        external.ready_ids(),
        &[
            producer("chain-post-close-timer"),
            producer("chain-preopen-timer"),
            producer("cli-chain"),
            producer("cli-replay-force"),
            producer("cli-single-default"),
            producer("cli-single-lhb"),
            producer("cli-single-schedule"),
            producer("cli-summary-default"),
            producer("cli-summary-lhb"),
            producer("cli-summary-schedule"),
        ]
    );
    assert!(external.conditional_or_disabled_ids().is_empty());
    assert!(external.unassessed_ids().is_empty());
    assert!(external.failed_ids().is_empty());
}

#[test]
fn w15_inventory_absent_facts_are_unassessed_without_inventing_dependency_failures() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE bundled catalog");

    let inventory = CandidateReadinessInventory::evaluate(&catalog, &[])
        .expect("TEST_CODE empty candidate observations are representable");

    assert_eq!(inventory.ready(), 0);
    assert_eq!(inventory.conditional(), 7);
    assert_eq!(inventory.inactive(), 22);
    assert_eq!(inventory.producer_missing(), 36);
    assert_eq!(inventory.source_missing(), 0);
    assert_eq!(inventory.schedule_unreachable(), 0);
    assert_eq!(inventory.presentation_missing(), 0);
    assert_eq!(inventory.durable_policy_missing(), 0);
    assert_eq!(inventory.producer_coverage().unassessed_ids().len(), 89);
    assert_eq!(
        inventory
            .producer_coverage()
            .conditional_or_disabled_ids()
            .len(),
        13
    );
    assert_eq!(
        inventory
            .enum_external_producer_coverage()
            .unassessed_ids()
            .len(),
        10
    );
}

#[test]
fn w15_inventory_explicit_missing_and_compatibility_are_kind_deduplicated_and_overlap() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE bundled catalog");
    let mut facts = complete_facts(&catalog);
    let mut first = available_dependencies(ReceiptMode::Strong, ReceiptMode::Compatibility);
    first.source_contract = DependencyAvailability::Missing;
    first.schedule_or_trigger = DependencyAvailability::Missing;
    replace_dependencies(&mut facts, "limit-boards-first", first);
    let mut second = available_dependencies(ReceiptMode::Strong, ReceiptMode::Compatibility);
    second.source_contract = DependencyAvailability::Missing;
    second.presentation = DependencyAvailability::Missing;
    replace_dependencies(&mut facts, "limit-boards-second", second);
    let mut third = available_dependencies(ReceiptMode::Strong, ReceiptMode::Strong);
    third.producer_binding = DependencyAvailability::Missing;
    third.durable_policy = DependencyAvailability::Missing;
    replace_dependencies(&mut facts, "limit-boards-third-plus", third);

    let inventory = CandidateReadinessInventory::evaluate(&catalog, &facts)
        .expect("TEST_CODE explicit dependency failures");

    assert_eq!(inventory.ready(), 35);
    assert_eq!(inventory.compat(), 1);
    assert_eq!(inventory.producer_missing(), 1);
    assert_eq!(inventory.source_missing(), 1);
    assert_eq!(inventory.schedule_unreachable(), 1);
    assert_eq!(inventory.presentation_missing(), 1);
    assert_eq!(inventory.durable_policy_missing(), 1);
    assert_eq!(inventory.compat_kind_ids(), &[MonitorKind::LimitBoards]);
    assert_eq!(
        inventory.producer_missing_kind_ids(),
        &[MonitorKind::LimitBoards]
    );
    assert_eq!(
        inventory.source_missing_kind_ids(),
        &[MonitorKind::LimitBoards]
    );
    assert_eq!(
        inventory.schedule_unreachable_kind_ids(),
        &[MonitorKind::LimitBoards]
    );
    assert_eq!(
        inventory.presentation_missing_kind_ids(),
        &[MonitorKind::LimitBoards]
    );
    assert_eq!(
        inventory.durable_policy_missing_kind_ids(),
        &[MonitorKind::LimitBoards]
    );
    assert_eq!(inventory.producer_coverage().failed_ids().len(), 3);
}

#[test]
fn w15_inventory_feature_completion_and_external_gate_failures_prevent_ready() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE bundled catalog");
    let mut facts = complete_facts(&catalog);
    let mut missing_feature = available_dependencies(ReceiptMode::Strong, ReceiptMode::Strong);
    missing_feature.feature_gate = DependencyAvailability::Missing;
    replace_dependencies(&mut facts, "p01-scheduled", missing_feature);
    let mut missing_completion = available_dependencies(ReceiptMode::Strong, ReceiptMode::Strong);
    missing_completion.completion_policy = DependencyAvailability::Missing;
    replace_dependencies(&mut facts, "p01-compensation", missing_completion);
    let mut external_missing_gate =
        available_dependencies(ReceiptMode::Strong, ReceiptMode::Strong);
    external_missing_gate.feature_gate = DependencyAvailability::Missing;
    replace_dependencies(&mut facts, "cli-chain", external_missing_gate);
    let mut missing_receipt = available_dependencies(ReceiptMode::Strong, ReceiptMode::Strong);
    missing_receipt.receipt = ReceiptReadiness::Missing {
        required: ReceiptMode::Strong,
    };
    replace_dependencies(&mut facts, "news-announcement", missing_receipt);

    let inventory = CandidateReadinessInventory::evaluate(&catalog, &facts)
        .expect("TEST_CODE all eight readiness roles are assessed");

    assert_eq!(inventory.ready(), 34);
    assert_eq!(inventory.producer_missing(), 0);
    assert_eq!(inventory.source_missing(), 0);
    assert_eq!(inventory.schedule_unreachable(), 0);
    assert_eq!(inventory.presentation_missing(), 0);
    assert_eq!(inventory.durable_policy_missing(), 0);
    assert_eq!(inventory.producer_coverage().failed_ids().len(), 4);
    assert_eq!(
        inventory.enum_external_producer_coverage().failed_ids(),
        &[producer("cli-chain")]
    );
    let retained = inventory
        .producer_facts()
        .iter()
        .find(|fact| fact.producer_id().as_str() == "p01-scheduled")
        .expect("TEST_CODE retained producer fact");
    assert_eq!(
        retained.dependencies().feature_gate,
        DependencyAvailability::Missing
    );
}

#[test]
fn w15_inventory_receipt_requirement_matrix_preserves_compatibility_without_strength_upgrade() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE bundled catalog");
    for (receipt, expected_ready, expected_compat, expected_failed) in [
        (
            ReceiptReadiness::Available {
                required: ReceiptMode::Strong,
                observed: ReceiptMode::Strong,
            },
            36,
            0,
            false,
        ),
        (
            ReceiptReadiness::Available {
                required: ReceiptMode::Strong,
                observed: ReceiptMode::Compatibility,
            },
            35,
            1,
            true,
        ),
        (
            ReceiptReadiness::Available {
                required: ReceiptMode::Compatibility,
                observed: ReceiptMode::Strong,
            },
            36,
            0,
            false,
        ),
        (
            ReceiptReadiness::Available {
                required: ReceiptMode::Compatibility,
                observed: ReceiptMode::Compatibility,
            },
            36,
            1,
            false,
        ),
        (
            ReceiptReadiness::Missing {
                required: ReceiptMode::Strong,
            },
            35,
            0,
            true,
        ),
    ] {
        let mut facts = complete_facts(&catalog);
        let mut dependencies = available_dependencies(ReceiptMode::Strong, ReceiptMode::Strong);
        dependencies.receipt = receipt;
        replace_dependencies(&mut facts, "news-announcement", dependencies);

        let inventory = CandidateReadinessInventory::evaluate(&catalog, &facts)
            .expect("TEST_CODE receipt requirement assessment");

        assert_eq!(inventory.ready(), expected_ready);
        assert_eq!(inventory.compat(), expected_compat);
        assert_eq!(
            inventory
                .producer_coverage()
                .failed_ids()
                .contains(&producer("news-announcement")),
            expected_failed
        );
        if expected_compat == 1 {
            assert_eq!(inventory.compat_kind_ids(), &[MonitorKind::Announcement]);
        } else {
            assert!(inventory.compat_kind_ids().is_empty());
        }
        let retained = inventory
            .producer_facts()
            .iter()
            .find(|fact| fact.producer_id().as_str() == "news-announcement")
            .expect("TEST_CODE retained receipt fact");
        assert_eq!(retained.dependencies().receipt, receipt);
    }
}

#[test]
fn w15_inventory_unknown_and_duplicate_producer_facts_are_rejected() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE bundled catalog");
    let unknown = producer("TEST_CODE-unknown-producer");
    assert_eq!(
        CandidateReadinessInventory::evaluate(
            &catalog,
            &[available_fact(
                unknown.clone(),
                ReceiptMode::Strong,
                ReceiptMode::Strong
            )]
        ),
        Err(CandidateReadinessInventoryError::UnknownProducer {
            producer_id: unknown
        })
    );

    let duplicate = producer("p01-scheduled");
    assert_eq!(
        CandidateReadinessInventory::evaluate(
            &catalog,
            &[
                available_fact(duplicate.clone(), ReceiptMode::Strong, ReceiptMode::Strong,),
                available_fact(duplicate.clone(), ReceiptMode::Strong, ReceiptMode::Strong,),
            ]
        ),
        Err(CandidateReadinessInventoryError::DuplicateProducerFact {
            producer_id: duplicate
        })
    );
}

#[test]
fn w15_inventory_fact_reordering_does_not_change_counts_or_affected_ids() {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE bundled catalog");
    let mut facts = complete_facts(&catalog);
    let mut missing = available_dependencies(ReceiptMode::Strong, ReceiptMode::Compatibility);
    missing.source_contract = DependencyAvailability::Missing;
    replace_dependencies(&mut facts, "limit-boards-first", missing);
    let original = CandidateReadinessInventory::evaluate(&catalog, &facts)
        .expect("TEST_CODE original fact ordering");

    facts.reverse();
    let reordered =
        CandidateReadinessInventory::evaluate(&catalog, &facts).expect("TEST_CODE reordered facts");

    assert_eq!(original, reordered);
}
