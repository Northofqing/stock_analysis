use std::cell::RefCell;

use crate::durable_delivery::{
    DecisionState, DeliveryEnvelope, DeliverySubKind, FoundationTerminalDisposition,
    P01DedicatedTerminalQuery, P01DedicatedTerminalRecord, PushKind,
};
use crate::event::envelope::{
    news_flash_evidence_sha256, NewsFlashAuditSource, NewsFlashRemoteReceipt,
    NewsFlashTransactionStage,
};
use crate::event::{
    EventEnvelope, NewsFlashWindow, NewsFlashWindowTerminalQuery, NewsFlashWindowTerminalRecord,
    PushDeliveryEvent,
};
use crate::monitor::push_job::{
    raw_digest, w09_completion_policy_fixture, AudienceId, AuthorityClass, BusinessDate, ChannelId,
    CompletionEligibility, CompletionOwnerId, CompletionPolicy, DeliveryResultView, Namespace,
    OccurrenceFamily, OccurrenceIdentityMaterial, OccurrenceKey, Sha256Digest, SourceContractId,
    SubjectId, TemplateId, TemplateVersion, UnitId, UtcMicros,
};

use super::dedicated_transport::{
    verify_n02_dedicated, verify_p01_dedicated, DedicatedConformanceError,
    DedicatedConformanceRoute, DedicatedSourceFailure, N02DedicatedTerminalSource,
    P01DedicatedTerminalSource,
};
use super::{
    BusinessIntentStore, FoundationSchemaMigration, InitialIntentDraft, InitialIntentIdentity,
    InitialIntentOutcome, IntentSnapshot, TerminalTemplateBinding,
};

struct FakeP01Source {
    result: P01DedicatedTerminalQuery,
    queried_dates: RefCell<Vec<String>>,
}

impl P01DedicatedTerminalSource for FakeP01Source {
    fn requery_p01(
        &self,
        business_date: &BusinessDate,
    ) -> Result<P01DedicatedTerminalQuery, DedicatedSourceFailure> {
        self.queried_dates
            .borrow_mut()
            .push(business_date.as_str().to_owned());
        Ok(self.result.clone())
    }
}

struct P01Case {
    _root: tempfile::TempDir,
    snapshot: IntentSnapshot,
    template: TerminalTemplateBinding,
    route: DedicatedConformanceRoute,
    policy: CompletionPolicy,
    terminal: P01DedicatedTerminalRecord,
    evidence_sha256: Sha256Digest,
}

fn p01_case() -> P01Case {
    p01_case_for("MU-p01", SubjectId::Global)
}

fn p01_case_for(unit_id: &str, subject: SubjectId) -> P01Case {
    let root = tempfile::tempdir().expect("create W13 P01 store");
    let database = root.path().join("business.sqlite3");
    FoundationSchemaMigration::bundled()
        .expect("load Foundation migration")
        .apply_to(&database)
        .expect("apply Foundation migration");
    let template = TerminalTemplateBinding::new(
        TemplateId::try_new("preopen_news_hot_v1".to_owned()).expect("template id"),
        TemplateVersion::try_new("preopen_news_hot_v1".to_owned()).expect("template version"),
    );
    let identity = InitialIntentIdentity::new(
        Namespace::Production,
        UnitId::try_new(unit_id.to_owned()).expect("unit"),
        OccurrenceIdentityMaterial::new(
            BusinessDate::parse("2026-08-18").expect("business date"),
            OccurrenceFamily::try_new("p01-business-date".to_owned()).expect("family"),
            OccurrenceKey::try_new("2026-08-18".to_owned()).expect("key"),
        ),
        CompletionOwnerId::try_new("p01-business-date-once".to_owned()).expect("owner"),
        SourceContractId::try_new("p01-source".to_owned()).expect("source contract"),
        subject,
        AudienceId::try_new("portfolio-owner".to_owned()).expect("audience"),
    );
    let rendered = b"TEST_CODE_W13_P01_RENDERED".to_vec();
    let draft = InitialIntentDraft::ready_for_recovery_test(
        identity,
        b"TEST_CODE_W13_P01_PREPARED".to_vec(),
        rendered.clone(),
        template.sha256().clone(),
        raw_digest(b"TEST_CODE_W13_P01_SOURCE_CONTRACT"),
        UtcMicros::try_new(1_787_027_400_000_000).expect("created at"),
    )
    .expect("valid P01 Ready intent");
    let mut store = BusinessIntentStore::open(&database).expect("open Foundation store");
    let snapshot = match store.record_initial(&draft).expect("record P01 intent") {
        InitialIntentOutcome::Inserted(snapshot) => snapshot,
        other => panic!("expected inserted P01 intent, got {other:?}"),
    };
    let attested = snapshot
        .attested_ready_binding()
        .expect("attested P01 Ready binding");
    let envelope = DeliveryEnvelope::new(
        "2026-08-18",
        PushKind::PreopenNewsHot,
        DeliverySubKind::None,
        "GLOBAL",
        "p01:2026-08-18",
        attested.source_evidence_fingerprint.as_str(),
        serde_json::to_vec(&serde_json::json!({
            "render_mode": "Scheduled",
            "schema_version": "P01_SOURCE_BINDING_V1",
        }))
        .expect("serialize P01 source binding"),
        "TEST_CODE_W13_P01_GLOBAL_SUBJECT",
        rendered,
        false,
        None,
    )
    .expect("valid legacy P01 envelope");
    let evidence_bytes = serde_json::to_vec(&serde_json::json!({
        "kind": "Accepted",
        "receipt": {
            "accepted_at": "2026-08-18T01:08:00Z",
            "channel": "TEST_CODE_W13_P01_CHANNEL",
            "latency_ms": 7,
            "message_id": "TEST_CODE_W13_P01_MESSAGE",
            "platform_message_id": "TEST_CODE_W13_P01_PLATFORM",
            "provider": "TEST_CODE_W13_P01_PROVIDER"
        }
    }))
    .expect("serialize accepted evidence");
    let evidence_sha256 = raw_digest(&evidence_bytes);
    let terminal = P01DedicatedTerminalRecord {
        legacy_decision_identity: envelope.decision_identity.clone(),
        envelope_canonical: envelope.canonical_bytes().expect("canonical P01 envelope"),
        envelope_sha256: envelope.canonical_sha256().expect("P01 envelope SHA"),
        ref_id: "TEST_CODE_W13_P01_DISPOSITION".to_owned(),
        attempt_id: Some("TEST_CODE_W13_P01_ATTEMPT".to_owned()),
        disposition: FoundationTerminalDisposition::Accepted,
        accepted_channel: Some("TEST_CODE_W13_P01_CHANNEL".to_owned()),
        evidence_bytes,
        evidence_sha256: evidence_sha256.as_str().to_owned(),
        durable_schema_version: crate::durable_delivery::DURABLE_SCHEMA_VERSION,
    };
    let route = DedicatedConformanceRoute::try_new(
        template.clone(),
        ChannelId::try_new("TEST_CODE_W13_P01_CHANNEL".to_owned()).expect("channel"),
    )
    .expect("valid P01 route");
    let policy = w09_completion_policy_fixture(
        Box::leak(unit_id.to_owned().into_boxed_str()),
        "p01-business-date-once",
        vec![AuthorityClass::P01Dedicated],
    );

    P01Case {
        _root: root,
        snapshot,
        template,
        route,
        policy,
        terminal,
        evidence_sha256,
    }
}

fn source_for(result: P01DedicatedTerminalQuery) -> FakeP01Source {
    FakeP01Source {
        result,
        queried_dates: RefCell::new(Vec::new()),
    }
}

fn replace_terminal_envelope(
    terminal: &mut P01DedicatedTerminalRecord,
    envelope: &DeliveryEnvelope,
) {
    terminal.legacy_decision_identity = envelope.decision_identity.clone();
    terminal.envelope_canonical = envelope.canonical_bytes().expect("canonical replacement");
    terminal.envelope_sha256 = envelope.canonical_sha256().expect("replacement SHA");
}

#[test]
fn w13_p01_dedicated_maps_exact_accepted_through_w09() {
    let case = p01_case();
    let attested = case
        .snapshot
        .attested_ready_binding()
        .expect("attested P01 Ready binding");
    let source = source_for(P01DedicatedTerminalQuery::Terminal(Box::new(
        case.terminal.clone(),
    )));

    let result = verify_p01_dedicated(
        &case.snapshot,
        &case.route,
        &case.policy,
        &source,
        UtcMicros::try_new(1_787_027_401_000_000).expect("verified at"),
    )
    .expect("verify exact P01 authority");

    assert_eq!(source.queried_dates.borrow().as_slice(), ["2026-08-18"]);
    let DeliveryResultView::TransportAccepted(verified) = result.view() else {
        panic!("expected P01 TransportAccepted, got {:?}", result.view());
    };
    assert_eq!(verified.authority_class(), AuthorityClass::P01Dedicated);
    assert_eq!(verified.decision_id(), &attested.decision_id);
    assert_eq!(verified.intent_id(), &attested.intent_id);
    assert_eq!(verified.occurrence(), &attested.occurrence);
    assert_eq!(verified.evidence_sha256(), &case.evidence_sha256);
}

#[test]
fn w13_p01_dedicated_preserves_dispositions_and_pending_states() {
    for (disposition, expected) in [
        (FoundationTerminalDisposition::Accepted, "accepted"),
        (FoundationTerminalDisposition::Rejected, "rejected"),
        (FoundationTerminalDisposition::Uncertain, "uncertain"),
        (FoundationTerminalDisposition::ManualAccepted, "manual"),
        (FoundationTerminalDisposition::ManualNotDelivered, "manual"),
    ] {
        let case = p01_case();
        let mut terminal = case.terminal.clone();
        terminal.disposition = disposition;
        if disposition != FoundationTerminalDisposition::Accepted {
            terminal.evidence_bytes = format!("TEST_CODE_W13_P01_{disposition:?}").into_bytes();
            terminal.evidence_sha256 = raw_digest(&terminal.evidence_bytes).as_str().to_owned();
            terminal.accepted_channel = None;
        }
        let source = source_for(P01DedicatedTerminalQuery::Terminal(Box::new(terminal)));
        let result = verify_p01_dedicated(
            &case.snapshot,
            &case.route,
            &case.policy,
            &source,
            UtcMicros::try_new(1_787_027_401_000_000).expect("verified at"),
        )
        .expect("map P01 terminal disposition");
        match (expected, result.view()) {
            ("accepted", DeliveryResultView::TransportAccepted(_))
            | ("rejected", DeliveryResultView::TransportRejected(_))
            | ("uncertain", DeliveryResultView::TransportUncertain(_))
            | ("manual", DeliveryResultView::AlreadyTerminal(_)) => {}
            (_, observed) => panic!("unexpected P01 disposition view: {observed:?}"),
        }
        if matches!(expected, "rejected" | "uncertain") {
            assert_eq!(
                result.completion_eligibility(),
                CompletionEligibility::Never
            );
        }
    }

    let case = p01_case();
    let missing = source_for(P01DedicatedTerminalQuery::Missing);
    assert_eq!(
        verify_p01_dedicated(
            &case.snapshot,
            &case.route,
            &case.policy,
            &missing,
            UtcMicros::try_new(1_787_027_401_000_000).unwrap(),
        ),
        Err(DedicatedConformanceError::TerminalMissing)
    );
    let pending = source_for(P01DedicatedTerminalQuery::PendingSeal {
        state: DecisionState::Reserved,
    });
    assert_eq!(
        verify_p01_dedicated(
            &case.snapshot,
            &case.route,
            &case.policy,
            &pending,
            UtcMicros::try_new(1_787_027_401_000_000).unwrap(),
        ),
        Err(DedicatedConformanceError::TerminalPendingSeal)
    );
}

#[test]
fn w13_p01_manual_accepted_optional_receipt_channel_is_exact() {
    let case = p01_case();
    let mut terminal = case.terminal.clone();
    terminal.disposition = FoundationTerminalDisposition::ManualAccepted;
    terminal.evidence_bytes = b"TEST_CODE_W13_P01_MANUAL_ACCEPTED".to_vec();
    terminal.evidence_sha256 = raw_digest(&terminal.evidence_bytes).as_str().to_owned();
    terminal.accepted_channel = Some("TEST_CODE_W13_WRONG_CHANNEL".to_owned());
    let source = source_for(P01DedicatedTerminalQuery::Terminal(Box::new(terminal)));

    assert_eq!(
        verify_p01_dedicated(
            &case.snapshot,
            &case.route,
            &case.policy,
            &source,
            UtcMicros::try_new(1_787_027_401_000_000).unwrap(),
        ),
        Err(DedicatedConformanceError::P01ChannelMismatch)
    );
}

#[test]
fn w13_p01_dedicated_fails_closed_on_binding_corruption() {
    let case = p01_case();
    let mut corrupt = case.terminal.clone();
    corrupt.legacy_decision_identity = "TEST_CODE_W13_WRONG_DECISION".to_owned();
    let source = source_for(P01DedicatedTerminalQuery::Terminal(Box::new(corrupt)));
    assert!(verify_p01_dedicated(
        &case.snapshot,
        &case.route,
        &case.policy,
        &source,
        UtcMicros::try_new(1_787_027_401_000_000).unwrap(),
    )
    .is_err());

    let case = p01_case();
    let attested = case.snapshot.attested_ready_binding().unwrap();
    let source_binding = serde_json::to_vec(&serde_json::json!({
        "render_mode": "Scheduled",
        "schema_version": "P01_SOURCE_BINDING_V1",
    }))
    .unwrap();
    let rendered = case.snapshot.rendered_bytes().unwrap().to_vec();
    let mismatched_envelopes = [
        DeliveryEnvelope::new(
            "2026-08-19",
            PushKind::PreopenNewsHot,
            DeliverySubKind::None,
            "GLOBAL",
            "p01:2026-08-19",
            attested.source_evidence_fingerprint.as_str(),
            source_binding.clone(),
            "TEST_CODE_W13_P01_GLOBAL_SUBJECT",
            rendered.clone(),
            false,
            None,
        )
        .unwrap(),
        DeliveryEnvelope::new(
            "2026-08-18",
            PushKind::PreopenNewsHot,
            DeliverySubKind::None,
            "GLOBAL",
            "p01:TEST_CODE_WRONG_OCCURRENCE",
            attested.source_evidence_fingerprint.as_str(),
            source_binding.clone(),
            "TEST_CODE_W13_P01_GLOBAL_SUBJECT",
            rendered.clone(),
            false,
            None,
        )
        .unwrap(),
        DeliveryEnvelope::new(
            "2026-08-18",
            PushKind::HoldingEvent,
            DeliverySubKind::None,
            "GLOBAL",
            "p01:2026-08-18",
            attested.source_evidence_fingerprint.as_str(),
            source_binding.clone(),
            "TEST_CODE_W13_P01_GLOBAL_SUBJECT",
            rendered.clone(),
            false,
            None,
        )
        .unwrap(),
        DeliveryEnvelope::new(
            "2026-08-18",
            PushKind::PreopenNewsHot,
            DeliverySubKind::None,
            "GLOBAL",
            "p01:2026-08-18",
            "TEST_CODE_W13_WRONG_SOURCE_FINGERPRINT",
            source_binding.clone(),
            "TEST_CODE_W13_P01_GLOBAL_SUBJECT",
            rendered.clone(),
            false,
            None,
        )
        .unwrap(),
        DeliveryEnvelope::new(
            "2026-08-18",
            PushKind::PreopenNewsHot,
            DeliverySubKind::None,
            "GLOBAL",
            "p01:2026-08-18",
            attested.source_evidence_fingerprint.as_str(),
            source_binding.clone(),
            "TEST_CODE_W13_P01_GLOBAL_SUBJECT",
            b"TEST_CODE_W13_WRONG_RENDER".to_vec(),
            false,
            None,
        )
        .unwrap(),
    ];
    for envelope in mismatched_envelopes {
        let mut corrupt = case.terminal.clone();
        replace_terminal_envelope(&mut corrupt, &envelope);
        let source = source_for(P01DedicatedTerminalQuery::Terminal(Box::new(corrupt)));
        assert!(verify_p01_dedicated(
            &case.snapshot,
            &case.route,
            &case.policy,
            &source,
            UtcMicros::try_new(1_787_027_401_000_000).unwrap(),
        )
        .is_err());
    }

    for mutate in ["sub_kind", "scope"] {
        let mut corrupt = case.terminal.clone();
        let mut envelope: DeliveryEnvelope =
            serde_json::from_slice(&corrupt.envelope_canonical).unwrap();
        match mutate {
            "sub_kind" => envelope.sub_kind = DeliverySubKind::FactorIC,
            "scope" => {
                envelope.cooldown_scope = crate::durable_delivery::CooldownScope::PerTicket;
                envelope.scope_key = "TEST_CODE_W13_WRONG_SCOPE".to_owned();
            }
            _ => unreachable!(),
        }
        corrupt.envelope_canonical = serde_json::to_vec(&envelope).unwrap();
        corrupt.envelope_sha256 = raw_digest(&corrupt.envelope_canonical).as_str().to_owned();
        let source = source_for(P01DedicatedTerminalQuery::Terminal(Box::new(corrupt)));
        assert!(verify_p01_dedicated(
            &case.snapshot,
            &case.route,
            &case.policy,
            &source,
            UtcMicros::try_new(1_787_027_401_000_000).unwrap(),
        )
        .is_err());
    }

    let mut corrupt = case.terminal.clone();
    corrupt.envelope_canonical.push(b' ');
    corrupt.envelope_sha256 = raw_digest(&corrupt.envelope_canonical).as_str().to_owned();
    let source = source_for(P01DedicatedTerminalQuery::Terminal(Box::new(corrupt)));
    assert!(verify_p01_dedicated(
        &case.snapshot,
        &case.route,
        &case.policy,
        &source,
        UtcMicros::try_new(1_787_027_401_000_000).unwrap(),
    )
    .is_err());

    let case = p01_case();
    let mut corrupt = case.terminal.clone();
    corrupt.envelope_sha256 = "0".repeat(64);
    let source = source_for(P01DedicatedTerminalQuery::Terminal(Box::new(corrupt)));
    assert!(verify_p01_dedicated(
        &case.snapshot,
        &case.route,
        &case.policy,
        &source,
        UtcMicros::try_new(1_787_027_401_000_000).unwrap(),
    )
    .is_err());

    for source_binding in [
        serde_json::json!({
            "render_mode": "Scheduled",
            "schema_version": "P01_SOURCE_BINDING_V2"
        }),
        serde_json::json!({
            "render_mode": "Replay",
            "schema_version": "P01_SOURCE_BINDING_V1"
        }),
        serde_json::json!({
            "extra": true,
            "render_mode": "Scheduled",
            "schema_version": "P01_SOURCE_BINDING_V1"
        }),
    ] {
        let case = p01_case();
        let mut corrupt = case.terminal.clone();
        let mut envelope: DeliveryEnvelope =
            serde_json::from_slice(&corrupt.envelope_canonical).expect("parse P01 envelope");
        envelope.replace_source_binding_preserving_identity(
            serde_json::to_vec(&source_binding).expect("serialize corrupt source binding"),
        );
        corrupt.envelope_canonical = envelope.canonical_bytes().expect("canonical envelope");
        corrupt.envelope_sha256 = envelope.canonical_sha256().expect("envelope SHA");
        let source = source_for(P01DedicatedTerminalQuery::Terminal(Box::new(corrupt)));
        assert!(verify_p01_dedicated(
            &case.snapshot,
            &case.route,
            &case.policy,
            &source,
            UtcMicros::try_new(1_787_027_401_000_000).unwrap(),
        )
        .is_err());
    }

    for (unit, subject) in [
        ("MU-not-p01", SubjectId::Global),
        (
            "MU-p01",
            SubjectId::entity("TEST_CODE_W13_ENTITY".to_owned()).expect("entity"),
        ),
    ] {
        let case = p01_case_for(unit, subject);
        let source = source_for(P01DedicatedTerminalQuery::Terminal(Box::new(
            case.terminal.clone(),
        )));
        assert!(verify_p01_dedicated(
            &case.snapshot,
            &case.route,
            &case.policy,
            &source,
            UtcMicros::try_new(1_787_027_401_000_000).unwrap(),
        )
        .is_err());
    }

    let wrong_template = TerminalTemplateBinding::new(
        TemplateId::try_new("not_p01".to_owned()).expect("template"),
        TemplateVersion::try_new("not_p01_v1".to_owned()).expect("version"),
    );
    assert_eq!(
        DedicatedConformanceRoute::try_new(
            wrong_template,
            ChannelId::try_new("TEST_CODE_W13_P01_CHANNEL".to_owned()).unwrap(),
        ),
        Err(DedicatedConformanceError::InvalidRoute)
    );

    let wrong_channel_route = DedicatedConformanceRoute::try_new(
        case.template.clone(),
        ChannelId::try_new("TEST_CODE_W13_WRONG_CHANNEL".to_owned()).unwrap(),
    )
    .unwrap();
    let source = source_for(P01DedicatedTerminalQuery::Terminal(Box::new(
        case.terminal.clone(),
    )));
    assert_eq!(
        verify_p01_dedicated(
            &case.snapshot,
            &wrong_channel_route,
            &case.policy,
            &source,
            UtcMicros::try_new(1_787_027_401_000_000).unwrap(),
        ),
        Err(DedicatedConformanceError::P01ChannelMismatch)
    );

    let case = p01_case();
    let mut corrupt = case.terminal.clone();
    corrupt.attempt_id = None;
    let source = source_for(P01DedicatedTerminalQuery::Terminal(Box::new(corrupt)));
    assert_eq!(
        verify_p01_dedicated(
            &case.snapshot,
            &case.route,
            &case.policy,
            &source,
            UtcMicros::try_new(1_787_027_401_000_000).unwrap(),
        ),
        Err(DedicatedConformanceError::InvalidP01Disposition)
    );

    let mut corrupt = case.terminal.clone();
    corrupt.disposition = FoundationTerminalDisposition::Rejected;
    let source = source_for(P01DedicatedTerminalQuery::Terminal(Box::new(corrupt)));
    assert_eq!(
        verify_p01_dedicated(
            &case.snapshot,
            &case.route,
            &case.policy,
            &source,
            UtcMicros::try_new(1_787_027_401_000_000).unwrap(),
        ),
        Err(DedicatedConformanceError::InvalidP01Disposition)
    );

    let mut corrupt = case.terminal.clone();
    corrupt.evidence_sha256 = "0".repeat(64);
    let source = source_for(P01DedicatedTerminalQuery::Terminal(Box::new(corrupt)));
    assert!(verify_p01_dedicated(
        &case.snapshot,
        &case.route,
        &case.policy,
        &source,
        UtcMicros::try_new(1_787_027_401_000_000).unwrap(),
    )
    .is_err());

    let mut corrupt = case.terminal.clone();
    corrupt.durable_schema_version += 1;
    let source = source_for(P01DedicatedTerminalQuery::Terminal(Box::new(corrupt)));
    assert!(verify_p01_dedicated(
        &case.snapshot,
        &case.route,
        &case.policy,
        &source,
        UtcMicros::try_new(1_787_027_401_000_000).unwrap(),
    )
    .is_err());
}

struct FakeN02Source {
    result: NewsFlashWindowTerminalQuery,
    queried: RefCell<Vec<(String, NewsFlashWindow)>>,
    n01_quota_probe: u32,
}

impl N02DedicatedTerminalSource for FakeN02Source {
    fn requery_n02(
        &self,
        business_date: &BusinessDate,
        window: NewsFlashWindow,
    ) -> Result<NewsFlashWindowTerminalQuery, DedicatedSourceFailure> {
        self.queried
            .borrow_mut()
            .push((business_date.as_str().to_owned(), window));
        Ok(self.result.clone())
    }
}

struct N02Case {
    _root: tempfile::TempDir,
    snapshot: IntentSnapshot,
    route: DedicatedConformanceRoute,
    policy: CompletionPolicy,
    terminal: NewsFlashWindowTerminalRecord,
    exact_terminal_bytes: Vec<u8>,
    source_published_at: chrono::DateTime<chrono::FixedOffset>,
    source_observed_at: chrono::DateTime<chrono::FixedOffset>,
    receipt_accepted_at: chrono::DateTime<chrono::FixedOffset>,
}

fn n02_case() -> N02Case {
    let root = tempfile::tempdir().expect("create W13 N02 store");
    let database = root.path().join("business.sqlite3");
    FoundationSchemaMigration::bundled()
        .expect("load Foundation migration")
        .apply_to(&database)
        .expect("apply Foundation migration");
    let template = TerminalTemplateBinding::new(
        TemplateId::try_new("news_flash_aggregated_v1".to_owned()).expect("template id"),
        TemplateVersion::try_new("news_flash_aggregated_v1".to_owned()).expect("template version"),
    );
    let identity = InitialIntentIdentity::new(
        Namespace::Production,
        UnitId::try_new("MU-news-flash-aggregate".to_owned()).expect("unit"),
        OccurrenceIdentityMaterial::new(
            BusinessDate::parse("2026-08-18").expect("business date"),
            OccurrenceFamily::try_new("news-flash-window".to_owned()).expect("family"),
            OccurrenceKey::try_new("09:30".to_owned()).expect("key"),
        ),
        CompletionOwnerId::try_new("news-flash-accepted-window".to_owned()).expect("owner"),
        SourceContractId::try_new("news-flash-authority-v5".to_owned()).expect("source contract"),
        SubjectId::Global,
        AudienceId::try_new("portfolio-owner".to_owned()).expect("audience"),
    );
    let rendered = b"TEST_CODE_W13_N02_RENDERED".to_vec();
    let render_sha256 = raw_digest(&rendered);
    let draft = InitialIntentDraft::ready_for_recovery_test(
        identity,
        b"TEST_CODE_W13_N02_PREPARED".to_vec(),
        rendered,
        template.sha256().clone(),
        raw_digest(b"TEST_CODE_W13_N02_SOURCE_CONTRACT"),
        UtcMicros::try_new(1_787_027_400_000_000).expect("created at"),
    )
    .expect("valid N02 Ready intent");
    let mut store = BusinessIntentStore::open(&database).expect("open Foundation store");
    let snapshot = match store.record_initial(&draft).expect("record N02 intent") {
        InitialIntentOutcome::Inserted(snapshot) => snapshot,
        other => panic!("expected inserted N02 intent, got {other:?}"),
    };

    let source_published_at =
        chrono::DateTime::parse_from_rfc3339("2026-08-18T01:20:00+08:00").expect("published time");
    let source_observed_at =
        chrono::DateTime::parse_from_rfc3339("2026-08-18T01:21:00+08:00").expect("observed time");
    let attempt_observed_at =
        chrono::DateTime::parse_from_rfc3339("2026-08-18T09:30:01+08:00").expect("attempt time");
    let receipt_accepted_at =
        chrono::DateTime::parse_from_rfc3339("2026-08-18T09:30:03+08:00").expect("accepted time");
    let terminal_observed_at =
        chrono::DateTime::parse_from_rfc3339("2026-08-18T09:30:04+08:00").expect("terminal time");
    let sources = vec![NewsFlashAuditSource {
        event_id: "TEST_CODE_W13_N02_EVENT".to_owned(),
        provider: "TEST_CODE_W13_N02_SOURCE_PROVIDER".to_owned(),
        source: "TEST_CODE_W13_N02_SOURCE".to_owned(),
        published_at: source_published_at,
        observed_at: source_observed_at,
        batch_id: "TEST_CODE_W13_N02_BATCH".to_owned(),
    }];
    let evidence_sha256 = news_flash_evidence_sha256(&sources);
    let reservation_sha256 = "a".repeat(64);
    let channel = "TEST_CODE_W13_N02_CHANNEL".to_owned();
    let attempt_event = PushDeliveryEvent::new_news_flash_attempt(
        "news_flash_aggregated_v1".to_owned(),
        "window:09:30".to_owned(),
        channel.clone(),
        27,
        chrono::NaiveDate::from_ymd_opt(2026, 8, 18).expect("date"),
        reservation_sha256.clone(),
        sources.clone(),
        evidence_sha256.clone(),
        render_sha256.as_str().to_owned(),
        1,
        attempt_observed_at,
    );
    let attempt = EventEnvelope::from_event(
        &attempt_event,
        attempt_event
            .news_flash_join_sha256
            .clone()
            .expect("attempt join"),
        "TEST_CODE_W13_N02_ATTEMPT_TRACE".to_owned(),
        attempt_observed_at.with_timezone(&chrono::Local),
    )
    .expect("valid attempt envelope");
    let receipt = NewsFlashRemoteReceipt {
        channel: channel.clone(),
        provider: "TEST_CODE_W13_N02_SINK_PROVIDER".to_owned(),
        message_id: "TEST_CODE_W13_N02_MESSAGE".to_owned(),
        platform_message_id: "TEST_CODE_W13_N02_PLATFORM".to_owned(),
        accepted_at: receipt_accepted_at,
        latency_ms: 2,
    };
    let terminal_event = PushDeliveryEvent::new_news_flash_terminal(
        NewsFlashTransactionStage::Accepted,
        "news_flash_aggregated_v1".to_owned(),
        "window:09:30".to_owned(),
        channel.clone(),
        27,
        3,
        chrono::NaiveDate::from_ymd_opt(2026, 8, 18).expect("date"),
        reservation_sha256,
        sources,
        evidence_sha256,
        render_sha256.as_str().to_owned(),
        1,
        attempt_observed_at,
        attempt_event
            .news_flash_sink_attempt_identity
            .clone()
            .expect("attempt identity"),
        attempt_event
            .news_flash_sink_attempt_sha256
            .clone()
            .expect("attempt SHA"),
        attempt.id.clone(),
        Some(receipt),
        terminal_observed_at,
        None,
        None,
    );
    let terminal = EventEnvelope::from_event(
        &terminal_event,
        terminal_event
            .news_flash_join_sha256
            .clone()
            .expect("terminal join"),
        "TEST_CODE_W13_N02_TERMINAL_TRACE".to_owned(),
        terminal_observed_at.with_timezone(&chrono::Local),
    )
    .expect("valid terminal envelope");
    let exact_terminal_bytes = serde_json::to_vec(&terminal).expect("canonical terminal bytes");
    let route =
        DedicatedConformanceRoute::try_new(template, ChannelId::try_new(channel).expect("channel"))
            .expect("valid N02 route");
    let policy = w09_completion_policy_fixture(
        "MU-news-flash-aggregate",
        "news-flash-accepted-window",
        vec![AuthorityClass::N02Dedicated],
    );

    N02Case {
        _root: root,
        snapshot,
        route,
        policy,
        terminal: NewsFlashWindowTerminalRecord { attempt, terminal },
        exact_terminal_bytes,
        source_published_at,
        source_observed_at,
        receipt_accepted_at,
    }
}

fn n02_source(case: &N02Case, n01_quota_probe: u32) -> FakeN02Source {
    FakeN02Source {
        result: NewsFlashWindowTerminalQuery::Terminal(Box::new(case.terminal.clone())),
        queried: RefCell::new(Vec::new()),
        n01_quota_probe,
    }
}

#[test]
fn w13_n02_dedicated_maps_exact_accepted_through_w09() {
    let case = n02_case();
    let source = n02_source(&case, 0);
    let result = verify_n02_dedicated(
        &case.snapshot,
        NewsFlashWindow::H0930,
        &case.route,
        &case.policy,
        &source,
        UtcMicros::try_new(1_787_027_405_000_000).expect("verified at"),
    )
    .expect("verify exact N02 authority");

    assert_eq!(
        source.queried.borrow().as_slice(),
        [("2026-08-18".to_owned(), NewsFlashWindow::H0930)]
    );
    let DeliveryResultView::TransportAccepted(verified) = result.view() else {
        panic!("expected N02 TransportAccepted, got {:?}", result.view());
    };
    assert_eq!(verified.authority_class(), AuthorityClass::N02Dedicated);
    assert_eq!(
        verified.evidence_sha256(),
        &raw_digest(&case.exact_terminal_bytes)
    );
    assert_ne!(case.receipt_accepted_at, case.source_published_at);
    assert_ne!(case.receipt_accepted_at, case.source_observed_at);
}

#[test]
fn w13_n02_is_independent_from_n01_event_and_quota_observations() {
    let case = n02_case();
    let empty_n01 = n02_source(&case, 0);
    let exhausted_n01 = n02_source(&case, u32::MAX);
    let first = verify_n02_dedicated(
        &case.snapshot,
        NewsFlashWindow::H0930,
        &case.route,
        &case.policy,
        &empty_n01,
        UtcMicros::try_new(1_787_027_405_000_000).expect("verified at"),
    )
    .expect("verify N02 with empty N01 observation");
    let second = verify_n02_dedicated(
        &case.snapshot,
        NewsFlashWindow::H0930,
        &case.route,
        &case.policy,
        &exhausted_n01,
        UtcMicros::try_new(1_787_027_405_000_000).expect("verified at"),
    )
    .expect("verify N02 with exhausted N01 observation");

    assert_ne!(empty_n01.n01_quota_probe, exhausted_n01.n01_quota_probe);
    assert_eq!(
        empty_n01.queried.borrow().as_slice(),
        exhausted_n01.queried.borrow().as_slice()
    );
    let DeliveryResultView::TransportAccepted(first) = first.view() else {
        panic!("expected first N02 Accepted");
    };
    let DeliveryResultView::TransportAccepted(second) = second.view() else {
        panic!("expected second N02 Accepted");
    };
    assert_eq!(first.binding_sha256(), second.binding_sha256());
    assert_eq!(first.evidence_sha256(), second.evidence_sha256());
}
