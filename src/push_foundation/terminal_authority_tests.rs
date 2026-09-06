use std::cell::{Cell, RefCell};

use crate::monitor::push_job::{
    raw_digest, w08_prepared_push_fixture, w09_completion_policy_fixture, AttemptId, AudienceId,
    AuthorityClass, BusinessDate, DecisionId, DeliveryResultView, DurableSchemaVersion, Namespace,
    Sha256Digest, SubjectId, TemplateId, TemplateVersion, TerminalDisposition, TerminalRefId,
    UtcMicros,
};

use super::terminal_authority::{
    reverify_for_finalization, terminal_binding_preimage_for_test, terminal_binding_sha256,
    verify_terminal, AuthorityDescriptor, AuthorityQuery, AuthorityQueryFailure,
    AuthorityTerminalRecord, TerminalAuthorityError, TerminalAuthorityPort,
    TerminalTemplateBinding,
};
use super::{
    BusinessIntentStore, FoundationSchemaMigration, InitialIntentDraft, InitialIntentIdentity,
    InitialIntentOutcome, IntentSnapshot,
};
use crate::monitor::push_job::{
    CompletionOwnerId, OccurrenceFamily, OccurrenceIdentityMaterial, OccurrenceKey,
    SourceContractId, UnitId,
};

#[derive(Clone)]
struct FakeAuthority {
    descriptor: AuthorityDescriptor,
    result: RefCell<Result<AuthorityQuery, AuthorityQueryFailure>>,
    calls: Cell<usize>,
    queried_decisions: RefCell<Vec<String>>,
}

impl FakeAuthority {
    fn terminal(record: AuthorityTerminalRecord) -> Self {
        Self {
            descriptor: AuthorityDescriptor {
                authority_class: AuthorityClass::GenericCounted,
                durable_schema_version: DurableSchemaVersion::try_new("durable-v5".to_owned())
                    .unwrap(),
            },
            result: RefCell::new(Ok(AuthorityQuery::Terminal(record))),
            calls: Cell::new(0),
            queried_decisions: RefCell::new(Vec::new()),
        }
    }
}

impl TerminalAuthorityPort for FakeAuthority {
    fn descriptor(&self) -> &AuthorityDescriptor {
        &self.descriptor
    }

    fn requery_terminal(
        &self,
        decision_id: &DecisionId,
    ) -> Result<AuthorityQuery, AuthorityQueryFailure> {
        self.calls.set(self.calls.get() + 1);
        self.queried_decisions
            .borrow_mut()
            .push(decision_id.as_str().to_owned());
        self.result.borrow().clone()
    }
}

struct Fixture {
    _root: tempfile::TempDir,
    snapshot: IntentSnapshot,
    template: TerminalTemplateBinding,
    policy: crate::monitor::push_job::CompletionPolicy,
    record: AuthorityTerminalRecord,
}

fn digest(byte: char) -> Sha256Digest {
    Sha256Digest::parse("w09 fixture", &byte.to_string().repeat(64)).unwrap()
}

fn fixture() -> Fixture {
    let template = TerminalTemplateBinding::new(
        TemplateId::try_new("auction-card".to_owned()).unwrap(),
        TemplateVersion::try_new("auction-card-v3".to_owned()).unwrap(),
    );
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("business.sqlite3");
    FoundationSchemaMigration::bundled()
        .unwrap()
        .apply_to(&database)
        .unwrap();
    let prepared = w08_prepared_push_fixture();
    let identity = InitialIntentIdentity::new(
        Namespace::Production,
        UnitId::try_new("MU-auction".to_owned()).unwrap(),
        OccurrenceIdentityMaterial::new(
            BusinessDate::parse("2026-09-07").unwrap(),
            OccurrenceFamily::try_new("auction-session".to_owned()).unwrap(),
            OccurrenceKey::try_new("main".to_owned()).unwrap(),
        ),
        CompletionOwnerId::try_new("owner-auction".to_owned()).unwrap(),
        SourceContractId::try_new("auction-source".to_owned()).unwrap(),
        SubjectId::entity("000001.SZ".to_owned()).unwrap(),
        AudienceId::try_new("portfolio-owner".to_owned()).unwrap(),
    );
    let draft = InitialIntentDraft::ready(
        identity,
        &prepared,
        template.sha256().clone(),
        digest('f'),
        UtcMicros::try_new(1_788_743_100_000_000).unwrap(),
    )
    .unwrap();
    let mut store = BusinessIntentStore::open(&database).unwrap();
    let snapshot = match store.record_initial(&draft).unwrap() {
        InitialIntentOutcome::Inserted(snapshot) => snapshot,
        other => panic!("expected inserted W09 intent, got {other:?}"),
    };
    let evidence_bytes = br#"{"kind":"Accepted","message_id":"remote-42"}"#.to_vec();
    let evidence_sha256 = raw_digest(&evidence_bytes);
    let mut record = AuthorityTerminalRecord {
        ref_id: TerminalRefId::try_new("disposition-42".to_owned()).unwrap(),
        authority_class: AuthorityClass::GenericCounted,
        namespace: Namespace::Production,
        decision_id: prepared.decision_id().clone(),
        attempt_id: Some(AttemptId::try_new("attempt-7".to_owned()).unwrap()),
        intent_id: prepared.intent_id().clone(),
        unit_id: prepared.unit_id().clone(),
        occurrence: prepared.occurrence().clone(),
        business_date: BusinessDate::parse("2026-09-07").unwrap(),
        subject: prepared.subject().clone(),
        audience: AudienceId::try_new("portfolio-owner".to_owned()).unwrap(),
        template_id: template.template_id().clone(),
        template_version: template.template_version().clone(),
        rendered_sha256: prepared.rendered_sha256().clone(),
        terminal_disposition: TerminalDisposition::Accepted,
        evidence_bytes,
        evidence_sha256,
        durable_schema_version: DurableSchemaVersion::try_new("durable-v5".to_owned()).unwrap(),
        binding_sha256: digest('0'),
    };
    record.binding_sha256 = terminal_binding_sha256(&record);

    Fixture {
        _root: root,
        snapshot,
        template,
        policy: w09_completion_policy_fixture(vec![AuthorityClass::GenericCounted]),
        record,
    }
}

#[test]
fn w09_terminal_binding_has_exact_canonical_material_and_excludes_verified_at() {
    let fixture = fixture();
    let record = fixture.record;
    let expected = format!(
        concat!(
            "TerminalBinding/v1\0{{",
            "\"attempt_id\":\"attempt-7\",",
            "\"audience\":\"portfolio-owner\",",
            "\"authority_class\":\"GenericCounted\",",
            "\"business_date\":\"2026-09-07\",",
            "\"decision_id\":\"{}\",",
            "\"durable_schema_version\":\"durable-v5\",",
            "\"evidence_sha256\":\"{}\",",
            "\"intent_id\":\"{}\",",
            "\"namespace\":{{\"kind\":\"Production\",\"run_id\":null}},",
            "\"occurrence\":\"{}\",",
            "\"ref_id\":\"disposition-42\",",
            "\"rendered_sha256\":\"{}\",",
            "\"subject\":{{\"kind\":\"Entity\",\"value\":\"000001.SZ\"}},",
            "\"template_id\":\"auction-card\",",
            "\"template_version\":\"auction-card-v3\",",
            "\"terminal_disposition\":\"Accepted\",",
            "\"unit_id\":\"MU-auction\"}}"
        ),
        record.decision_id.as_str(),
        record.evidence_sha256.as_str(),
        record.intent_id.as_str(),
        record.occurrence.as_str(),
        record.rendered_sha256.as_str(),
    );

    assert_eq!(
        terminal_binding_preimage_for_test(&record),
        expected.as_bytes()
    );
    assert_eq!(
        terminal_binding_sha256(&record),
        raw_digest(expected.as_bytes())
    );

    let authority = FakeAuthority::terminal(record);
    let first = verify_terminal(
        &fixture.snapshot,
        &fixture.template,
        &fixture.policy,
        &authority,
        UtcMicros::try_new(1_788_743_101_000_000).unwrap(),
    )
    .unwrap();
    let second = verify_terminal(
        &fixture.snapshot,
        &fixture.template,
        &fixture.policy,
        &authority,
        UtcMicros::try_new(1_788_743_999_000_000).unwrap(),
    )
    .unwrap();
    assert_ne!(first.verified_at(), second.verified_at());
    assert_eq!(first.binding_sha256(), second.binding_sha256());
    assert_eq!(authority.calls.get(), 2);
}

#[test]
fn w09_first_construction_requeries_exact_decision_and_maps_strong_result() {
    let fixture = fixture();
    let expected_decision = fixture.record.decision_id.as_str().to_owned();
    let authority = FakeAuthority::terminal(fixture.record);

    let verified = verify_terminal(
        &fixture.snapshot,
        &fixture.template,
        &fixture.policy,
        &authority,
        UtcMicros::try_new(1_788_743_101_000_000).unwrap(),
    )
    .unwrap();
    let result = verified.clone().into_delivery_result();

    assert_eq!(authority.calls.get(), 1);
    assert_eq!(
        authority.queried_decisions.borrow().as_slice(),
        &[expected_decision]
    );
    assert!(matches!(
        result.view(),
        DeliveryResultView::TransportAccepted(terminal) if terminal == &verified
    ));
}

#[test]
fn w09_authority_decision_bytes_and_subject_are_each_exactly_bound() {
    let fixture = fixture();
    let mutations: [(&str, fn(&mut AuthorityTerminalRecord)); 4] = [
        ("authority_class", |record| {
            record.authority_class = AuthorityClass::P01Dedicated;
        }),
        ("decision_id", |record| {
            record.decision_id = DecisionId::try_new("wrong-decision".to_owned()).unwrap();
        }),
        ("rendered_sha256", |record| {
            record.rendered_sha256 = digest('9');
        }),
        ("subject", |record| {
            record.subject = SubjectId::entity("600000.SH".to_owned()).unwrap();
        }),
    ];

    for (field, mutate) in mutations {
        let mut record = fixture.record.clone();
        mutate(&mut record);
        record.binding_sha256 = terminal_binding_sha256(&record);
        let authority = FakeAuthority::terminal(record);
        let error = verify_terminal(
            &fixture.snapshot,
            &fixture.template,
            &fixture.policy,
            &authority,
            UtcMicros::try_new(1_788_743_101_000_000).unwrap(),
        )
        .unwrap_err();
        assert!(
            matches!(
                error,
                TerminalAuthorityError::BindingMismatch { field: actual }
                    | TerminalAuthorityError::AuthorityDescriptorMismatch { field: actual }
                    if actual == field
            ),
            "wrong failure for {field}: {error:?}"
        );
    }
}

#[test]
fn w09_recomputes_evidence_and_declared_binding_instead_of_trusting_hashes() {
    let fixture = fixture();

    let mut evidence_drift = fixture.record.clone();
    evidence_drift.evidence_bytes.push(b'!');
    let evidence_authority = FakeAuthority::terminal(evidence_drift);
    assert!(matches!(
        verify_terminal(
            &fixture.snapshot,
            &fixture.template,
            &fixture.policy,
            &evidence_authority,
            UtcMicros::try_new(1_788_743_101_000_000).unwrap(),
        ),
        Err(TerminalAuthorityError::EvidenceHashMismatch)
    ));

    let mut binding_drift = fixture.record;
    binding_drift.binding_sha256 = digest('8');
    let binding_authority = FakeAuthority::terminal(binding_drift);
    assert!(matches!(
        verify_terminal(
            &fixture.snapshot,
            &fixture.template,
            &fixture.policy,
            &binding_authority,
            UtcMicros::try_new(1_788_743_101_000_000).unwrap(),
        ),
        Err(TerminalAuthorityError::TerminalBindingHashMismatch)
    ));
}

#[test]
fn w09_transport_requires_attempt_but_manual_terminal_stays_already_terminal() {
    for disposition in [
        TerminalDisposition::Accepted,
        TerminalDisposition::Rejected,
        TerminalDisposition::Uncertain,
    ] {
        let fixture = fixture();
        let mut record = fixture.record;
        record.attempt_id = None;
        record.terminal_disposition = disposition;
        record.binding_sha256 = terminal_binding_sha256(&record);
        let authority = FakeAuthority::terminal(record);
        assert!(matches!(
            verify_terminal(
                &fixture.snapshot,
                &fixture.template,
                &fixture.policy,
                &authority,
                UtcMicros::try_new(1_788_743_101_000_000).unwrap(),
            ),
            Err(TerminalAuthorityError::AttemptRequired)
        ));
    }

    for disposition in [
        TerminalDisposition::ManualConfirmedAccepted,
        TerminalDisposition::ManualConfirmedNotDelivered,
    ] {
        let fixture = fixture();
        let mut record = fixture.record;
        record.attempt_id = None;
        record.terminal_disposition = disposition;
        record.binding_sha256 = terminal_binding_sha256(&record);
        let authority = FakeAuthority::terminal(record);
        let terminal = verify_terminal(
            &fixture.snapshot,
            &fixture.template,
            &fixture.policy,
            &authority,
            UtcMicros::try_new(1_788_743_101_000_000).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            terminal.into_delivery_result().view(),
            DeliveryResultView::AlreadyTerminal(_)
        ));
    }
}

#[test]
fn w09_missing_pending_unavailable_and_disallowed_authority_fail_closed() {
    let fixture = fixture();
    for (query, expected) in [
        (
            Ok(AuthorityQuery::Missing),
            TerminalAuthorityError::TerminalMissing,
        ),
        (
            Ok(AuthorityQuery::PendingSeal),
            TerminalAuthorityError::TerminalPendingSeal,
        ),
        (
            Err(AuthorityQueryFailure),
            TerminalAuthorityError::AuthorityUnavailable,
        ),
    ] {
        let authority = FakeAuthority {
            descriptor: AuthorityDescriptor {
                authority_class: AuthorityClass::GenericCounted,
                durable_schema_version: DurableSchemaVersion::try_new("durable-v5".to_owned())
                    .unwrap(),
            },
            result: RefCell::new(query),
            calls: Cell::new(0),
            queried_decisions: RefCell::new(Vec::new()),
        };
        assert_eq!(
            verify_terminal(
                &fixture.snapshot,
                &fixture.template,
                &fixture.policy,
                &authority,
                UtcMicros::try_new(1_788_743_101_000_000).unwrap(),
            )
            .unwrap_err(),
            expected
        );
        assert_eq!(authority.calls.get(), 1);
    }

    let disallowed_policy = w09_completion_policy_fixture(vec![AuthorityClass::P01Dedicated]);
    let authority = FakeAuthority::terminal(fixture.record);
    assert!(matches!(
        verify_terminal(
            &fixture.snapshot,
            &fixture.template,
            &disallowed_policy,
            &authority,
            UtcMicros::try_new(1_788_743_101_000_000).unwrap(),
        ),
        Err(TerminalAuthorityError::AuthorityNotAllowed)
    ));
}

#[test]
fn w09_finalization_requeries_authority_and_returns_only_the_fresh_reference() {
    let fixture = fixture();
    let authority = FakeAuthority::terminal(fixture.record);
    let first_verified_at = UtcMicros::try_new(1_788_743_101_000_000).unwrap();
    let final_verified_at = UtcMicros::try_new(1_788_743_102_000_000).unwrap();
    let prior = verify_terminal(
        &fixture.snapshot,
        &fixture.template,
        &fixture.policy,
        &authority,
        first_verified_at,
    )
    .unwrap();

    let finalization = reverify_for_finalization(
        &prior,
        &fixture.snapshot,
        &fixture.template,
        &fixture.policy,
        &authority,
        final_verified_at,
    )
    .unwrap();

    assert_eq!(authority.calls.get(), 2);
    assert_eq!(
        finalization.verified_terminal().verified_at(),
        final_verified_at
    );
    assert_eq!(
        finalization.verified_terminal().binding_sha256(),
        prior.binding_sha256()
    );
    let consumed = finalization.into_verified_terminal();
    assert_eq!(consumed.verified_at(), final_verified_at);
}

#[test]
fn w09_finalization_rejects_authority_drift_after_the_initial_verification() {
    let fixture = fixture();
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let prior = verify_terminal(
        &fixture.snapshot,
        &fixture.template,
        &fixture.policy,
        &authority,
        UtcMicros::try_new(1_788_743_101_000_000).unwrap(),
    )
    .unwrap();

    let mut changed = fixture.record;
    changed.ref_id = TerminalRefId::try_new("disposition-replaced".to_owned()).unwrap();
    changed.binding_sha256 = terminal_binding_sha256(&changed);
    *authority.result.borrow_mut() = Ok(AuthorityQuery::Terminal(changed));

    assert!(matches!(
        reverify_for_finalization(
            &prior,
            &fixture.snapshot,
            &fixture.template,
            &fixture.policy,
            &authority,
            UtcMicros::try_new(1_788_743_102_000_000).unwrap(),
        ),
        Err(TerminalAuthorityError::PriorReferenceChanged)
    ));
    assert_eq!(authority.calls.get(), 2);
}

#[test]
fn w09_finalization_repeats_full_binding_validation_and_fails_closed() {
    let fixture = fixture();
    let authority = FakeAuthority::terminal(fixture.record.clone());
    let prior = verify_terminal(
        &fixture.snapshot,
        &fixture.template,
        &fixture.policy,
        &authority,
        UtcMicros::try_new(1_788_743_101_000_000).unwrap(),
    )
    .unwrap();

    let mut changed = fixture.record;
    changed.subject = SubjectId::entity("600000.SH".to_owned()).unwrap();
    changed.binding_sha256 = terminal_binding_sha256(&changed);
    *authority.result.borrow_mut() = Ok(AuthorityQuery::Terminal(changed));

    assert!(matches!(
        reverify_for_finalization(
            &prior,
            &fixture.snapshot,
            &fixture.template,
            &fixture.policy,
            &authority,
            UtcMicros::try_new(1_788_743_102_000_000).unwrap(),
        ),
        Err(TerminalAuthorityError::BindingMismatch { field: "subject" })
    ));
    assert_eq!(authority.calls.get(), 2);
}
