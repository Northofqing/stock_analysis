use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate};
use rusqlite::{params, Connection};

use crate::calendar::verified_a_share_calendar_authority_hash;
use crate::monitor::push_job::{
    CalendarId, GitSha40, MachineCatalog, Namespace, OccurrenceId, ProducerId, RunId, Sha256Digest,
    SourceContractId, SourceContractVersion, UnitId,
};

use super::activation::{
    ActivationManifest, DesiredActivationState, PromotionAction, PromotionJournalEntry,
};
use super::activation_codec::{journal_digest, manifest_digest, promotion_event_id};
use super::activation_deployment::ActivationCalendarClaims;
use super::activation_readiness::{
    construct_activation_deployment_set, deployment_set_codec_fixture,
    read_activation_deployment_set, reread_activation_deployment_set, ActivationDeploymentSetError,
    ActivationDeploymentSetRequest, SharedDependencyDeclaration, SourcePackageDeclaration,
};
use super::activation_store::inspect_raw_activation_facts;
use super::migration::FoundationSchemaMigration;
use super::operational_readiness::{DependencyKind, ReadinessScope};

fn digest(value: char) -> Sha256Digest {
    Sha256Digest::parse("TEST_CODE hash", &value.to_string().repeat(64)).expect("TEST_CODE digest")
}

fn namespace() -> Namespace {
    Namespace::test(RunId::try_new("deployment-set-test".to_owned()).expect("TEST_CODE run"))
}

fn observed_at() -> u64 {
    u64::try_from(
        DateTime::parse_from_rfc3339("2026-09-08T02:00:00Z")
            .expect("TEST_CODE timestamp")
            .timestamp_micros(),
    )
    .expect("TEST_CODE positive timestamp")
}

fn initialized_database(name: &str) -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().expect("TEST_CODE temp root");
    let database = root
        .path()
        .canonicalize()
        .expect("TEST_CODE canonical root")
        .join(name);
    let migration = FoundationSchemaMigration::bundled().expect("TEST_CODE bundled schema");
    let ddl = migration
        .ddl_bytes()
        .strip_prefix(b".bail on\n")
        .expect("TEST_CODE library-safe DDL suffix");
    let connection = Connection::open(&database).expect("TEST_CODE database");
    connection
        .execute_batch(std::str::from_utf8(ddl).expect("TEST_CODE UTF-8 DDL"))
        .expect("TEST_CODE synthetic frozen schema");
    drop(connection);
    (root, database)
}

#[derive(Clone)]
struct CurrentBinding {
    unit_id: UnitId,
    generation: u64,
    manifest_sha256: Sha256Digest,
    journal_sha256: Sha256Digest,
    source_binding_sha256: Sha256Digest,
}

#[allow(clippy::too_many_arguments)]
fn append_generation(
    database: &Path,
    unit_id: &UnitId,
    generation: u64,
    previous: Option<&CurrentBinding>,
    desired_state: DesiredActivationState,
    action: PromotionAction,
    build: char,
    source: char,
) -> CurrentBinding {
    let start = generation * 100;
    let mut manifest = ActivationManifest {
        manifest_sha256: digest('0'),
        unit_id: unit_id.clone(),
        generation,
        previous_manifest_sha256: previous.map(|value| value.manifest_sha256.clone()),
        desired_state,
        physical_owner: format!("owner-{}", unit_id.as_str()),
        build_commit: GitSha40::parse(&build.to_string().repeat(40))
            .expect("TEST_CODE build commit"),
        build_sha256: digest(build),
        catalog_sha256: digest('c'),
        business_schema_sha256: digest('d'),
        durable_schema_sha256: digest('e'),
        template_sha256: digest('f'),
        source_contract_sha256: digest(source),
        evidence_sha256: digest('9'),
        approved_by: "operator-test".to_owned(),
        approved_at: start,
        window_start: start,
        window_end: start + 50,
        rollback_target_sha256: None,
        created_at: start + 1,
    };
    manifest.manifest_sha256 = manifest_digest(&manifest);
    let event_id = promotion_event_id(unit_id.as_str(), generation);
    let mut journal = PromotionJournalEntry {
        event_id,
        unit_id: unit_id.clone(),
        generation,
        from_manifest_sha256: previous.map(|value| value.manifest_sha256.clone()),
        to_manifest_sha256: manifest.manifest_sha256.clone(),
        actor: "operator-test".to_owned(),
        action,
        reason: "activation.applied".to_owned(),
        window_start: start,
        window_end: start + 50,
        evidence_sha256: digest('9'),
        rollback_target_sha256: None,
        previous_sha256: previous.map(|value| value.journal_sha256.clone()),
        canonical_sha256: digest('0'),
        occurred_at: start + 2,
    };
    journal.canonical_sha256 = journal_digest(&journal);

    let connection = Connection::open(database).expect("TEST_CODE seed connection");
    connection
        .execute(
            "INSERT INTO push_activation_manifests(\
             manifest_sha256,unit_id,generation,previous_manifest_sha256,desired_state,\
             physical_owner,build_commit,build_sha256,catalog_sha256,business_schema_sha256,\
             durable_schema_sha256,template_sha256,source_contract_sha256,evidence_sha256,\
             approved_by,approved_at,window_start,window_end,rollback_target_sha256,created_at\
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,NULL,?19)",
            params![
                manifest.manifest_sha256.as_str(),
                unit_id.as_str(),
                generation as i64,
                manifest
                    .previous_manifest_sha256
                    .as_ref()
                    .map(Sha256Digest::as_str),
                desired_state.as_str(),
                manifest.physical_owner,
                manifest.build_commit.as_str(),
                manifest.build_sha256.as_str(),
                manifest.catalog_sha256.as_str(),
                manifest.business_schema_sha256.as_str(),
                manifest.durable_schema_sha256.as_str(),
                manifest.template_sha256.as_str(),
                manifest.source_contract_sha256.as_str(),
                manifest.evidence_sha256.as_str(),
                manifest.approved_by,
                manifest.approved_at as i64,
                manifest.window_start as i64,
                manifest.window_end as i64,
                manifest.created_at as i64,
            ],
        )
        .expect("TEST_CODE manifest insert");
    connection
        .execute(
            "INSERT INTO push_promotion_journal(\
             event_id,unit_id,generation,from_manifest_sha256,to_manifest_sha256,actor,action,\
             reason,window_start,window_end,evidence_sha256,rollback_target_sha256,\
             previous_sha256,canonical_sha256,occurred_at\
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,NULL,?12,?13,?14)",
            params![
                journal.event_id.as_str(),
                unit_id.as_str(),
                generation as i64,
                journal
                    .from_manifest_sha256
                    .as_ref()
                    .map(Sha256Digest::as_str),
                journal.to_manifest_sha256.as_str(),
                journal.actor,
                action.as_str(),
                journal.reason,
                journal.window_start as i64,
                journal.window_end as i64,
                journal.evidence_sha256.as_str(),
                journal.previous_sha256.as_ref().map(Sha256Digest::as_str),
                journal.canonical_sha256.as_str(),
                journal.occurred_at as i64,
            ],
        )
        .expect("TEST_CODE journal insert");
    CurrentBinding {
        unit_id: unit_id.clone(),
        generation,
        manifest_sha256: manifest.manifest_sha256,
        journal_sha256: journal.canonical_sha256,
        source_binding_sha256: manifest.source_contract_sha256,
    }
}

fn two_unit_database(name: &str) -> (tempfile::TempDir, PathBuf, Vec<CurrentBinding>) {
    let (root, database) = initialized_database(name);
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let shadow_unit = catalog.units()[0].id();
    let disabled_unit = catalog.units()[1].id();
    let first_shadow = append_generation(
        &database,
        shadow_unit,
        1,
        None,
        DesiredActivationState::Disabled,
        PromotionAction::Initialize,
        'a',
        '1',
    );
    let current_shadow = append_generation(
        &database,
        shadow_unit,
        2,
        Some(&first_shadow),
        DesiredActivationState::Shadow,
        PromotionAction::EnterShadow,
        'b',
        '2',
    );
    let current_disabled = append_generation(
        &database,
        disabled_unit,
        1,
        None,
        DesiredActivationState::Disabled,
        PromotionAction::Initialize,
        'c',
        '3',
    );
    (root, database, vec![current_shadow, current_disabled])
}

fn calendar(namespace: &Namespace) -> ActivationCalendarClaims {
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let date = NaiveDate::from_ymd_opt(2026, 9, 8).expect("TEST_CODE date");
    ActivationCalendarClaims {
        namespace: namespace.clone(),
        catalog_sha256: catalog.catalog_sha256().clone(),
        catalog_units: catalog
            .units()
            .iter()
            .map(|unit| unit.id().clone())
            .collect(),
        calendar_id: CalendarId::try_new("test-sse-calendar".to_owned())
            .expect("TEST_CODE calendar"),
        authority_sha256: Sha256Digest::parse(
            "TEST_CODE authority",
            verified_a_share_calendar_authority_hash(date).expect("TEST_CODE covered calendar"),
        )
        .expect("TEST_CODE authority hash"),
        utc_offset_seconds: 28_800,
    }
}

fn dependencies(seed: char) -> Vec<SharedDependencyDeclaration> {
    [
        DependencyKind::Namespace,
        DependencyKind::Durable,
        DependencyKind::Audit,
        DependencyKind::TypedAuthority,
        DependencyKind::Schema,
        DependencyKind::Manifest,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, kind)| {
        SharedDependencyDeclaration::new(
            kind,
            SourceContractId::try_new(format!("TEST_CODE-{}", kind.as_str()))
                .expect("TEST_CODE dependency id"),
            SourceContractVersion::try_new("v1".to_owned()).expect("TEST_CODE dependency version"),
            digest(char::from_u32(u32::from(seed) + index as u32).expect("TEST_CODE digest char")),
        )
    })
    .collect()
}

fn dependencies_replacing(
    replaced: DependencyKind,
    contract_id: &str,
    version: &str,
    sha256: Sha256Digest,
) -> Vec<SharedDependencyDeclaration> {
    [
        DependencyKind::Namespace,
        DependencyKind::Durable,
        DependencyKind::Audit,
        DependencyKind::TypedAuthority,
        DependencyKind::Schema,
        DependencyKind::Manifest,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, kind)| {
        let baseline_sha =
            digest(char::from_u32(u32::from('1') + index as u32).expect("TEST_CODE digest char"));
        if kind == replaced {
            SharedDependencyDeclaration::new(
                kind,
                SourceContractId::try_new(contract_id.to_owned())
                    .expect("TEST_CODE replacement dependency id"),
                SourceContractVersion::try_new(version.to_owned())
                    .expect("TEST_CODE replacement dependency version"),
                sha256.clone(),
            )
        } else {
            SharedDependencyDeclaration::new(
                kind,
                SourceContractId::try_new(format!("TEST_CODE-{}", kind.as_str()))
                    .expect("TEST_CODE dependency id"),
                SourceContractVersion::try_new("v1".to_owned())
                    .expect("TEST_CODE dependency version"),
                baseline_sha,
            )
        }
    })
    .collect()
}

fn source_packages(
    namespace: &Namespace,
    bindings: &[CurrentBinding],
) -> Vec<SourcePackageDeclaration> {
    bindings
        .iter()
        .map(|binding| {
            SourcePackageDeclaration::new(
                namespace.clone(),
                binding.unit_id.clone(),
                binding.generation,
                binding.manifest_sha256.clone(),
                binding.source_binding_sha256.clone(),
            )
        })
        .collect()
}

fn request(
    bindings: &[CurrentBinding],
    enabled_producers: Vec<ProducerId>,
    recovery_units: Vec<UnitId>,
) -> ActivationDeploymentSetRequest {
    let namespace = namespace();
    ActivationDeploymentSetRequest::new(
        namespace.clone(),
        calendar(&namespace),
        observed_at(),
        enabled_producers,
        recovery_units,
        source_packages(&namespace, bindings),
        dependencies('1'),
    )
}

fn first_producer(unit_id: &UnitId) -> ProducerId {
    MachineCatalog::bundled()
        .expect("TEST_CODE catalog")
        .producers_for_unit(unit_id)[0]
        .id()
        .clone()
}

#[test]
fn deployment_set_v1_has_independent_literal_golden_bytes_and_sha256() {
    let expected = concat!(
        "ActivationDeploymentSet/v1\0{",
        "\"calendar\":{\"authority_sha256\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",",
        "\"calendar_id\":\"golden-calendar\",\"utc_offset_seconds\":28800},",
        "\"catalog_sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",",
        "\"enabled_producers\":[\"producer-a\"],",
        "\"namespace\":{\"kind\":\"Test\",\"run_id\":\"golden-run\"},",
        "\"recovery_units\":[\"unit-b\"],\"schema_version\":1,",
        "\"shared_dependencies\":[",
        "{\"contract_id\":\"golden-audit\",\"contract_version\":\"v1\",\"dependency_kind\":\"Audit\",",
        "\"sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\"},",
        "{\"contract_id\":\"golden-durable\",\"contract_version\":\"v2\",\"dependency_kind\":\"Durable\",",
        "\"sha256\":\"2222222222222222222222222222222222222222222222222222222222222222\"},",
        "{\"contract_id\":\"golden-manifest\",\"contract_version\":\"v3\",\"dependency_kind\":\"Manifest\",",
        "\"sha256\":\"3333333333333333333333333333333333333333333333333333333333333333\"},",
        "{\"contract_id\":\"golden-namespace\",\"contract_version\":\"v4\",\"dependency_kind\":\"Namespace\",",
        "\"sha256\":\"4444444444444444444444444444444444444444444444444444444444444444\"},",
        "{\"contract_id\":\"golden-schema\",\"contract_version\":\"v5\",\"dependency_kind\":\"Schema\",",
        "\"sha256\":\"5555555555555555555555555555555555555555555555555555555555555555\"},",
        "{\"contract_id\":\"golden-typed-authority\",\"contract_version\":\"v6\",",
        "\"dependency_kind\":\"TypedAuthority\",",
        "\"sha256\":\"6666666666666666666666666666666666666666666666666666666666666666\"}],",
        "\"units\":[{\"activation_status\":\"CaughtUp\",",
        "\"build_commit\":\"1111111111111111111111111111111111111111\",",
        "\"build_sha256\":\"2222222222222222222222222222222222222222222222222222222222222222\",",
        "\"desired_state\":\"Shadow\",\"generation\":7,",
        "\"journal_event_id\":\"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\",",
        "\"journal_sha256\":\"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff\",",
        "\"manifest_sha256\":\"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\",",
        "\"physical_owner\":\"protected-owner\",",
        "\"source_binding_sha256\":\"3333333333333333333333333333333333333333333333333333333333333333\",",
        "\"unit_id\":\"unit-a\"},{\"activation_status\":\"Unregistered\",",
        "\"build_commit\":null,\"build_sha256\":null,\"desired_state\":null,",
        "\"generation\":null,\"journal_event_id\":null,\"journal_sha256\":null,",
        "\"manifest_sha256\":null,\"physical_owner\":null,",
        "\"source_binding_sha256\":null,\"unit_id\":\"unit-b\"}]}"
    )
    .as_bytes();
    let actual = deployment_set_codec_fixture(
        "protected-owner",
        "1111111111111111111111111111111111111111",
    );
    assert_eq!(actual, expected);
    assert_eq!(
        crate::monitor::push_job::raw_digest(expected).as_str(),
        "262ba7b18630b804732f2123d8f1a95acdf254e84b50f3693e31d8a6031359eb"
    );
    let expected_text = std::str::from_utf8(expected).expect("TEST_CODE golden UTF-8");
    let changed_owner_expected = expected_text.replacen(
        "\"physical_owner\":\"protected-owner\"",
        "\"physical_owner\":\"changed-owner\"",
        1,
    );
    let changed_owner_actual =
        deployment_set_codec_fixture("changed-owner", "1111111111111111111111111111111111111111");
    assert_eq!(changed_owner_actual, changed_owner_expected.as_bytes());
    assert_ne!(
        crate::monitor::push_job::raw_digest(&changed_owner_actual).as_str(),
        "262ba7b18630b804732f2123d8f1a95acdf254e84b50f3693e31d8a6031359eb"
    );

    let changed_commit_expected = expected_text.replacen(
        "\"build_commit\":\"1111111111111111111111111111111111111111\"",
        "\"build_commit\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"",
        1,
    );
    let changed_commit_actual = deployment_set_codec_fixture(
        "protected-owner",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    assert_eq!(changed_commit_actual, changed_commit_expected.as_bytes());
    assert_ne!(
        crate::monitor::push_job::raw_digest(&changed_commit_actual).as_str(),
        "262ba7b18630b804732f2123d8f1a95acdf254e84b50f3693e31d8a6031359eb"
    );
}

#[test]
fn full_catalog_real_read_preserves_two_current_generations_and_explicit_null_rows() {
    let (_root, database, bindings) = two_unit_database("full.sqlite3");
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let producer = first_producer(&bindings[0].unit_id);
    let set = read_activation_deployment_set(
        &database,
        &bindings[0].unit_id,
        request(&bindings, vec![producer], vec![bindings[1].unit_id.clone()]),
    )
    .expect("TEST_CODE full deployment set");

    let generations = set.unit_generations();
    assert_eq!(generations.len(), 52);
    assert!(generations.windows(2).all(|pair| pair[0].0 < pair[1].0));
    assert_eq!(
        generations
            .iter()
            .find(|(unit_id, _)| *unit_id == &bindings[0].unit_id)
            .map(|(_, generation)| *generation),
        Some(Some(2))
    );
    assert_eq!(
        generations
            .iter()
            .find(|(unit_id, _)| *unit_id == &bindings[1].unit_id)
            .map(|(_, generation)| *generation),
        Some(Some(1))
    );
    assert_eq!(
        generations
            .iter()
            .filter(|(_, generation)| generation.is_none())
            .count(),
        catalog.units().len() - 2
    );
    let bytes = std::str::from_utf8(set.canonical_bytes()).expect("TEST_CODE canonical UTF-8");
    assert!(bytes.starts_with("ActivationDeploymentSet/v1\0{"));
    let dependency_positions = [
        "Audit",
        "Durable",
        "Manifest",
        "Namespace",
        "Schema",
        "TypedAuthority",
    ]
    .map(|kind| {
        let needle = format!("\"dependency_kind\":\"{kind}\"");
        assert_eq!(bytes.matches(&needle).count(), 1, "TEST_CODE Core {kind}");
        bytes.find(&needle).expect("TEST_CODE Core dependency")
    });
    assert!(
        dependency_positions
            .windows(2)
            .all(|pair| pair[0] < pair[1]),
        "TEST_CODE Core dependencies use DependencyKind text order"
    );
    assert_eq!(
        bytes
            .matches("\"activation_status\":\"Unregistered\"")
            .count(),
        50
    );
    for nullable_field in [
        "generation",
        "manifest_sha256",
        "journal_event_id",
        "journal_sha256",
        "desired_state",
        "physical_owner",
        "build_commit",
        "build_sha256",
        "source_binding_sha256",
    ] {
        assert_eq!(
            bytes.matches(&format!("\"{nullable_field}\":null")).count(),
            50,
            "TEST_CODE explicit null field {nullable_field}"
        );
    }
    assert!(bytes.contains(&format!("\"build_sha256\":\"{}\"", "b".repeat(64))));
    assert!(bytes.contains(&format!("\"build_sha256\":\"{}\"", "c".repeat(64))));
    assert_eq!(
        set.sha256(),
        &crate::monitor::push_job::raw_digest(set.canonical_bytes())
    );
    let debug = format!("{set:?}");
    assert!(!debug.contains("owner-"));
    assert!(!debug.contains(database.to_string_lossy().as_ref()));
}

#[test]
fn ordering_is_stable_and_configuration_or_dependency_changes_identity() {
    let (_root, database, bindings) = two_unit_database("ordering.sqlite3");
    let p0 = first_producer(&bindings[0].unit_id);
    let p1 = first_producer(&bindings[1].unit_id);
    let baseline = read_activation_deployment_set(
        &database,
        &bindings[0].unit_id,
        request(
            &bindings,
            vec![p0.clone(), p1.clone()],
            vec![bindings[0].unit_id.clone(), bindings[1].unit_id.clone()],
        ),
    )
    .expect("TEST_CODE baseline");
    let reordered = read_activation_deployment_set(
        &database,
        &bindings[1].unit_id,
        request(
            &bindings.iter().cloned().rev().collect::<Vec<_>>(),
            vec![p1.clone(), p0.clone()],
            vec![bindings[1].unit_id.clone(), bindings[0].unit_id.clone()],
        ),
    )
    .expect("TEST_CODE reordered");
    assert_eq!(baseline.canonical_bytes(), reordered.canonical_bytes());
    assert_eq!(baseline.sha256(), reordered.sha256());

    let namespace = namespace();
    let mut dependency_input_reordered = dependencies('1');
    dependency_input_reordered.reverse();
    let reordered_dependencies = read_activation_deployment_set(
        &database,
        &bindings[0].unit_id,
        ActivationDeploymentSetRequest::new(
            namespace.clone(),
            calendar(&namespace),
            observed_at(),
            vec![p0.clone(), p1.clone()],
            vec![bindings[0].unit_id.clone(), bindings[1].unit_id.clone()],
            source_packages(&namespace, &bindings),
            dependency_input_reordered,
        ),
    )
    .expect("TEST_CODE dependency input order");
    assert_eq!(baseline.sha256(), reordered_dependencies.sha256());

    let changed_configuration = read_activation_deployment_set(
        &database,
        &bindings[0].unit_id,
        request(
            &bindings,
            vec![p0.clone()],
            vec![bindings[1].unit_id.clone()],
        ),
    )
    .expect("TEST_CODE changed configuration");
    assert_ne!(baseline.sha256(), changed_configuration.sha256());

    for (label, changed_dependencies) in [
        (
            "contract_id",
            dependencies_replacing(
                DependencyKind::Schema,
                "TEST_CODE-changed-Schema",
                "v1",
                digest('5'),
            ),
        ),
        (
            "contract_version",
            dependencies_replacing(
                DependencyKind::Schema,
                "TEST_CODE-Schema",
                "v2",
                digest('5'),
            ),
        ),
        (
            "sha256",
            dependencies_replacing(
                DependencyKind::Schema,
                "TEST_CODE-Schema",
                "v1",
                digest('a'),
            ),
        ),
    ] {
        let changed = read_activation_deployment_set(
            &database,
            &bindings[0].unit_id,
            ActivationDeploymentSetRequest::new(
                namespace.clone(),
                calendar(&namespace),
                observed_at(),
                vec![p0.clone(), p1.clone()],
                vec![bindings[0].unit_id.clone(), bindings[1].unit_id.clone()],
                source_packages(&namespace, &bindings),
                changed_dependencies,
            ),
        )
        .expect("TEST_CODE changed dependency");
        assert_ne!(baseline.sha256(), changed.sha256(), "TEST_CODE {label}");
    }
}

#[test]
fn calendar_id_changes_identity_but_covered_observation_time_does_not() {
    let (_root, database, bindings) = two_unit_database("calendar-identity.sqlite3");
    let namespace = namespace();
    let baseline = read_activation_deployment_set(
        &database,
        &bindings[0].unit_id,
        ActivationDeploymentSetRequest::new(
            namespace.clone(),
            calendar(&namespace),
            observed_at(),
            vec![],
            vec![],
            source_packages(&namespace, &bindings),
            dependencies('1'),
        ),
    )
    .expect("TEST_CODE baseline calendar");

    let observed_later = read_activation_deployment_set(
        &database,
        &bindings[0].unit_id,
        ActivationDeploymentSetRequest::new(
            namespace.clone(),
            calendar(&namespace),
            observed_at() + 1,
            vec![],
            vec![],
            source_packages(&namespace, &bindings),
            dependencies('1'),
        ),
    )
    .expect("TEST_CODE covered later observation");
    assert_eq!(baseline.sha256(), observed_later.sha256());

    let mut changed_calendar = calendar(&namespace);
    changed_calendar.calendar_id = CalendarId::try_new("changed-test-sse-calendar".to_owned())
        .expect("TEST_CODE changed calendar");
    let changed_calendar = read_activation_deployment_set(
        &database,
        &bindings[0].unit_id,
        ActivationDeploymentSetRequest::new(
            namespace.clone(),
            changed_calendar,
            observed_at(),
            vec![],
            vec![],
            source_packages(&namespace, &bindings),
            dependencies('1'),
        ),
    )
    .expect("TEST_CODE changed calendar id");
    assert_ne!(baseline.sha256(), changed_calendar.sha256());
}

#[test]
fn duplicates_unknowns_omissions_and_unregistered_references_are_rejected() {
    let (_root, database, bindings) = two_unit_database("closure.sqlite3");
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let producer = first_producer(&bindings[0].unit_id);
    let unknown_producer =
        ProducerId::try_new("TEST_CODE-unknown-producer".to_owned()).expect("TEST_CODE producer");
    let unknown_unit = UnitId::try_new("MU-unknown".to_owned()).expect("TEST_CODE unit");
    let unregistered = catalog.units()[2].id().clone();
    let unregistered_producer = first_producer(&unregistered);

    for bad in [
        request(&bindings, vec![producer.clone(), producer.clone()], vec![]),
        request(&bindings, vec![unknown_producer], vec![]),
        request(&bindings, vec![], vec![unknown_unit]),
        request(&bindings, vec![unregistered_producer], vec![]),
        request(&bindings, vec![], vec![unregistered]),
    ] {
        assert_eq!(
            read_activation_deployment_set(&database, &bindings[0].unit_id, bad),
            Err(ActivationDeploymentSetError::ConfigurationRejected)
        );
    }

    let namespace = namespace();
    let mut duplicate_sources = source_packages(&namespace, &bindings);
    duplicate_sources.push(duplicate_sources[0].clone());
    let mut missing_sources = source_packages(&namespace, &bindings);
    missing_sources.pop();
    let mut extra_sources = source_packages(&namespace, &bindings);
    extra_sources.push(SourcePackageDeclaration::new(
        namespace.clone(),
        catalog.units()[2].id().clone(),
        1,
        digest('a'),
        digest('b'),
    ));
    for sources in [duplicate_sources, missing_sources, extra_sources] {
        let bad = ActivationDeploymentSetRequest::new(
            namespace.clone(),
            calendar(&namespace),
            observed_at(),
            vec![],
            vec![],
            sources,
            dependencies('1'),
        );
        assert_eq!(
            read_activation_deployment_set(&database, &bindings[0].unit_id, bad),
            Err(ActivationDeploymentSetError::SourceDeclarationRejected)
        );
    }

    let mut duplicate_dependencies = dependencies('1');
    duplicate_dependencies.push(duplicate_dependencies[0].clone());
    let mut missing_dependencies = dependencies('1');
    missing_dependencies.pop();
    let mut extra_dependencies = dependencies('1');
    extra_dependencies.push(SharedDependencyDeclaration::new(
        DependencyKind::ProducerBinding,
        SourceContractId::try_new("TEST_CODE-extra".to_owned()).expect("TEST_CODE contract"),
        SourceContractVersion::try_new("v1".to_owned()).expect("TEST_CODE version"),
        digest('f'),
    ));
    for shared in [
        duplicate_dependencies,
        missing_dependencies,
        extra_dependencies,
    ] {
        let bad = ActivationDeploymentSetRequest::new(
            namespace.clone(),
            calendar(&namespace),
            observed_at(),
            vec![],
            vec![],
            source_packages(&namespace, &bindings),
            shared,
        );
        assert_eq!(
            read_activation_deployment_set(&database, &bindings[0].unit_id, bad),
            Err(ActivationDeploymentSetError::SharedDependenciesRejected)
        );
    }
}

#[test]
fn pending_anywhere_and_every_source_join_mismatch_fail_closed() {
    let (_pending_root, pending) = initialized_database("pending.sqlite3");
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let pending_unit = catalog.units()[0].id().clone();
    let pending_binding = append_generation(
        &pending,
        &pending_unit,
        1,
        None,
        DesiredActivationState::Disabled,
        PromotionAction::Initialize,
        'a',
        '1',
    );
    let connection = Connection::open(&pending).expect("TEST_CODE pending mutation");
    let delete_guard: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='push_promotion_journal_delete'",
            [],
            |row| row.get(0),
        )
        .expect("TEST_CODE delete guard");
    connection
        .execute_batch(
            "DROP TRIGGER push_promotion_journal_delete; DELETE FROM push_promotion_journal;",
        )
        .expect("TEST_CODE leave pending manifest");
    connection
        .execute_batch(&delete_guard)
        .expect("TEST_CODE restore guard");
    drop(connection);
    assert_eq!(
        read_activation_deployment_set(
            &pending,
            &pending_unit,
            request(&[pending_binding], vec![], vec![])
        ),
        Err(ActivationDeploymentSetError::PendingUnit)
    );

    let (_unselected_root, unselected_pending, unselected_bindings) =
        two_unit_database("unselected-pending.sqlite3");
    let connection = Connection::open(&unselected_pending).expect("TEST_CODE pending mutation");
    let delete_guard: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='push_promotion_journal_delete'",
            [],
            |row| row.get(0),
        )
        .expect("TEST_CODE delete guard");
    connection
        .execute_batch("DROP TRIGGER push_promotion_journal_delete;")
        .expect("TEST_CODE remove delete guard");
    connection
        .execute(
            "DELETE FROM push_promotion_journal WHERE unit_id=?1",
            params![unselected_bindings[1].unit_id.as_str()],
        )
        .expect("TEST_CODE make unselected unit pending");
    connection
        .execute_batch(&delete_guard)
        .expect("TEST_CODE restore delete guard");
    drop(connection);
    assert_eq!(
        read_activation_deployment_set(
            &unselected_pending,
            &unselected_bindings[0].unit_id,
            request(&unselected_bindings, vec![], vec![])
        ),
        Err(ActivationDeploymentSetError::PendingUnit)
    );

    let (_root, database, bindings) = two_unit_database("source-join.sqlite3");
    let namespace = namespace();
    for field in 0..5 {
        let mut sources = source_packages(&namespace, &bindings);
        sources[0] = match field {
            0 => SourcePackageDeclaration::new(
                Namespace::Production,
                bindings[0].unit_id.clone(),
                bindings[0].generation,
                bindings[0].manifest_sha256.clone(),
                bindings[0].source_binding_sha256.clone(),
            ),
            1 => SourcePackageDeclaration::new(
                namespace.clone(),
                UnitId::try_new("MU-unknown".to_owned()).expect("TEST_CODE unit"),
                bindings[0].generation,
                bindings[0].manifest_sha256.clone(),
                bindings[0].source_binding_sha256.clone(),
            ),
            2 => SourcePackageDeclaration::new(
                namespace.clone(),
                bindings[0].unit_id.clone(),
                bindings[0].generation + 1,
                bindings[0].manifest_sha256.clone(),
                bindings[0].source_binding_sha256.clone(),
            ),
            3 => SourcePackageDeclaration::new(
                namespace.clone(),
                bindings[0].unit_id.clone(),
                bindings[0].generation,
                digest('a'),
                bindings[0].source_binding_sha256.clone(),
            ),
            4 => SourcePackageDeclaration::new(
                namespace.clone(),
                bindings[0].unit_id.clone(),
                bindings[0].generation,
                bindings[0].manifest_sha256.clone(),
                digest('b'),
            ),
            _ => unreachable!("TEST_CODE finite matrix"),
        };
        let bad = ActivationDeploymentSetRequest::new(
            namespace.clone(),
            calendar(&namespace),
            observed_at(),
            vec![],
            vec![],
            sources,
            dependencies('1'),
        );
        assert_eq!(
            read_activation_deployment_set(&database, &bindings[0].unit_id, bad),
            Err(ActivationDeploymentSetError::SourceDeclarationRejected),
            "TEST_CODE source field {field}"
        );
    }
}

#[test]
fn calendar_exact_join_accepts_closure_observation_and_rejects_each_binding_change() {
    let (_root, database) = initialized_database("calendar.sqlite3");
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let selected = catalog.units()[0].id();
    let namespace = namespace();
    let closure_at = u64::try_from(
        DateTime::parse_from_rfc3339("2026-09-12T02:00:00Z")
            .expect("TEST_CODE closure")
            .timestamp_micros(),
    )
    .expect("TEST_CODE timestamp");
    let mut closure_calendar = calendar(&namespace);
    closure_calendar.authority_sha256 = Sha256Digest::parse(
        "TEST_CODE closure authority",
        verified_a_share_calendar_authority_hash(
            NaiveDate::from_ymd_opt(2026, 9, 12).expect("TEST_CODE closure date"),
        )
        .expect("TEST_CODE closure covered"),
    )
    .expect("TEST_CODE closure hash");
    let closure = ActivationDeploymentSetRequest::new(
        namespace.clone(),
        closure_calendar,
        closure_at,
        vec![],
        vec![],
        vec![],
        dependencies('1'),
    );
    read_activation_deployment_set(&database, selected, closure)
        .expect("TEST_CODE raw closure deployment observation");

    for field in 0..6 {
        let mut claims = calendar(&namespace);
        let request_namespace = match field {
            0 => Namespace::Production,
            1 => {
                claims.catalog_sha256 = digest('a');
                namespace.clone()
            }
            2 => {
                claims.catalog_units.pop();
                namespace.clone()
            }
            3 => {
                claims.catalog_units[1] = claims.catalog_units[0].clone();
                namespace.clone()
            }
            4 => {
                claims.authority_sha256 = digest('b');
                namespace.clone()
            }
            5 => {
                claims.utc_offset_seconds = 0;
                namespace.clone()
            }
            _ => unreachable!("TEST_CODE finite matrix"),
        };
        let bad = ActivationDeploymentSetRequest::new(
            request_namespace,
            claims,
            observed_at(),
            vec![],
            vec![],
            vec![],
            dependencies('1'),
        );
        assert_eq!(
            read_activation_deployment_set(&database, selected, bad),
            Err(ActivationDeploymentSetError::CalendarRejected),
            "TEST_CODE calendar field {field}"
        );
    }
}

#[test]
fn scope_is_exact_inventory_projection_not_permission() {
    let (_root, database, bindings) = two_unit_database("scope.sqlite3");
    let enabled = first_producer(&bindings[0].unit_id);
    let disabled_recovery = bindings[1].unit_id.clone();
    let set = read_activation_deployment_set(
        &database,
        &bindings[0].unit_id,
        request(
            &bindings,
            vec![enabled.clone()],
            vec![disabled_recovery.clone()],
        ),
    )
    .expect("TEST_CODE candidate set");
    assert_eq!(
        set.unit_ids_for_scope(&ReadinessScope::Core)
            .expect("TEST_CODE core inventory")
            .into_iter()
            .collect::<BTreeSet<_>>(),
        [bindings[0].unit_id.clone(), disabled_recovery]
            .into_iter()
            .collect()
    );
    assert_eq!(
        set.unit_ids_for_scope(&ReadinessScope::Producer {
            unit_id: bindings[0].unit_id.clone(),
            producer_id: enabled.clone(),
        }),
        Ok(vec![bindings[0].unit_id.clone()])
    );
    assert_eq!(
        set.unit_ids_for_scope(&ReadinessScope::Occurrence {
            unit_id: bindings[0].unit_id.clone(),
            producer_id: enabled,
            occurrence_id: OccurrenceId::from_digest(&digest('f')),
        }),
        Ok(vec![bindings[0].unit_id.clone()])
    );
    let disabled_producer = first_producer(&bindings[1].unit_id);
    assert_eq!(
        set.unit_ids_for_scope(&ReadinessScope::Producer {
            unit_id: bindings[1].unit_id.clone(),
            producer_id: disabled_producer,
        }),
        Err(ActivationDeploymentSetError::ScopeRejected)
    );
}

#[test]
fn reread_rejects_configuration_unit_and_cross_database_drift() {
    let (_root, database, mut bindings) = two_unit_database("reread.sqlite3");
    let enabled = first_producer(&bindings[0].unit_id);
    let original_request = request(&bindings, vec![enabled.clone()], vec![]);
    let original =
        read_activation_deployment_set(&database, &bindings[0].unit_id, original_request)
            .expect("TEST_CODE original");
    assert_eq!(
        reread_activation_deployment_set(
            &original,
            &database,
            &bindings[0].unit_id,
            request(&bindings, vec![enabled.clone()], vec![]),
        ),
        Ok(original.clone())
    );
    assert_eq!(
        reread_activation_deployment_set(
            &original,
            &database,
            &bindings[0].unit_id,
            request(&bindings, vec![], vec![]),
        ),
        Err(ActivationDeploymentSetError::DeploymentChanged)
    );

    let next = append_generation(
        &database,
        &bindings[1].unit_id,
        2,
        Some(&bindings[1]),
        DesiredActivationState::Shadow,
        PromotionAction::EnterShadow,
        'd',
        '4',
    );
    bindings[1] = next;
    assert_eq!(
        reread_activation_deployment_set(
            &original,
            &database,
            &bindings[0].unit_id,
            request(&bindings, vec![enabled], vec![]),
        ),
        Err(ActivationDeploymentSetError::DeploymentChanged)
    );

    let (_other_root, other_database) = initialized_database("other.sqlite3");
    assert_eq!(
        reread_activation_deployment_set(
            &original,
            &other_database,
            &bindings[0].unit_id,
            request(&[], vec![], vec![]),
        ),
        Err(ActivationDeploymentSetError::DeploymentChanged)
    );
}

#[test]
fn raw_constructor_rejects_incomplete_duplicate_and_unknown_catalog_coverage() {
    let (_root, database) = initialized_database("raw-coverage.sqlite3");
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let selected = catalog.units()[0].id();
    let facts = inspect_raw_activation_facts(&database, selected).expect("TEST_CODE raw facts");

    let mut missing = facts.clone();
    missing.units.pop();
    assert_eq!(
        construct_activation_deployment_set(&missing, request(&[], vec![], vec![])),
        Err(ActivationDeploymentSetError::UnitCoverageRejected)
    );
    let mut duplicate = facts.clone();
    duplicate.units[1] = duplicate.units[0].clone();
    assert_eq!(
        construct_activation_deployment_set(&duplicate, request(&[], vec![], vec![])),
        Err(ActivationDeploymentSetError::UnitCoverageRejected)
    );
    let mut unknown = facts;
    unknown.units[0].unit_id = UnitId::try_new("MU-unknown".to_owned()).expect("TEST_CODE unit");
    assert_eq!(
        construct_activation_deployment_set(&unknown, request(&[], vec![], vec![])),
        Err(ActivationDeploymentSetError::UnitCoverageRejected)
    );
}

#[test]
fn read_errors_are_stable_and_do_not_expose_database_paths() {
    let root = tempfile::tempdir().expect("TEST_CODE temp root");
    let missing = root.path().join("protected-owner-secret.sqlite3");
    let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
    let error = read_activation_deployment_set(
        &missing,
        catalog.units()[0].id(),
        request(&[], vec![], vec![]),
    )
    .expect_err("TEST_CODE missing database");
    assert_eq!(error, ActivationDeploymentSetError::ActivationFactsRejected);
    assert!(!error.to_string().contains("protected-owner-secret"));
}
