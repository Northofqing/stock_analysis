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
    PushDeliveryEvent, PushRecord,
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
    draft: InitialIntentDraft,
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
    p01_case_for_mode(unit_id, subject, "Scheduled")
}

fn p01_case_for_mode(unit_id: &str, subject: SubjectId, render_mode: &str) -> P01Case {
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
    let source_binding = serde_json::to_vec(&serde_json::json!({
        "render_mode": render_mode,
        "schema_version": "P01_SOURCE_BINDING_V1",
    }))
    .expect("serialize P01 source binding");
    let draft = InitialIntentDraft::ready_for_recovery_test(
        identity,
        source_binding.clone(),
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
        source_binding,
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
        draft,
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
    unavailable: bool,
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
        if self.unavailable {
            return Err(DedicatedSourceFailure);
        }
        Ok(self.result.clone())
    }
}

struct N02Case {
    _root: tempfile::TempDir,
    draft: InitialIntentDraft,
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
    n02_case_with(
        "2026-08-18",
        NewsFlashWindow::H0930,
        b"TEST_CODE_W13_N02_RENDERED".to_vec(),
        None,
        false,
    )
}

fn n02_case_with(
    date: &str,
    window: NewsFlashWindow,
    rendered: Vec<u8>,
    source_override: Option<Vec<NewsFlashAuditSource>>,
    raw_audit_render: bool,
) -> N02Case {
    n02_case_with_mode(
        date,
        window,
        rendered,
        source_override,
        raw_audit_render,
        false,
    )
}

fn n02_case_with_mode(
    date: &str,
    window: NewsFlashWindow,
    rendered: Vec<u8>,
    source_override: Option<Vec<NewsFlashAuditSource>>,
    raw_audit_render: bool,
    opaque: bool,
) -> N02Case {
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
            BusinessDate::parse(date).expect("business date"),
            OccurrenceFamily::try_new("news-flash-window".to_owned()).expect("family"),
            OccurrenceKey::try_new(window.label().to_owned()).expect("key"),
        ),
        CompletionOwnerId::try_new("news-flash-accepted-window".to_owned()).expect("owner"),
        SourceContractId::try_new("news-flash-authority-v5".to_owned()).expect("source contract"),
        SubjectId::Global,
        AudienceId::try_new("portfolio-owner".to_owned()).expect("audience"),
    );
    let rendered_len = rendered.len();
    let source_published_at =
        chrono::DateTime::parse_from_rfc3339(&format!("{date}T01:20:00+08:00"))
            .expect("published time");
    let source_observed_at =
        chrono::DateTime::parse_from_rfc3339(&format!("{date}T01:21:00+08:00"))
            .expect("observed time");
    let attempt_observed_at =
        chrono::DateTime::parse_from_rfc3339(&format!("{date}T09:30:01+08:00"))
            .expect("attempt time");
    let receipt_accepted_at =
        chrono::DateTime::parse_from_rfc3339(&format!("{date}T09:30:03+08:00"))
            .expect("accepted time");
    let terminal_observed_at =
        chrono::DateTime::parse_from_rfc3339(&format!("{date}T09:30:04+08:00"))
            .expect("terminal time");
    let sources = vec![NewsFlashAuditSource {
        event_id: "TEST_CODE_W13_N02_EVENT".to_owned(),
        provider: "TEST_CODE_W13_N02_SOURCE_PROVIDER".to_owned(),
        source: "TEST_CODE_W13_N02_SOURCE".to_owned(),
        published_at: source_published_at,
        observed_at: source_observed_at,
        batch_id: "TEST_CODE_W13_N02_BATCH".to_owned(),
    }];
    let sources = source_override.unwrap_or(sources);
    let binding = super::intent_store::n02_test_support::reservation(
        chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
        window,
        &rendered,
        sources.clone(),
    );
    let render_sha256 = if raw_audit_render {
        raw_digest(&rendered).as_str().to_owned()
    } else {
        binding.material().news_flash_render_sha256.clone()
    };
    let reservation_sha256 = binding.material().reservation_sha256.clone();
    let draft = if opaque {
        InitialIntentDraft::ready_for_recovery_test(
            identity,
            b"TEST_CODE_OLD_OPAQUE_N02".to_vec(),
            rendered,
            template.sha256().clone(),
            raw_digest(b"TEST_CODE_W13_N02_SOURCE_CONTRACT"),
            UtcMicros::try_new(1_787_027_400_000_000).unwrap(),
        )
        .unwrap()
    } else {
        super::intent_store::n02_test_support::ready(
            identity,
            rendered,
            &binding,
            template.sha256().clone(),
            raw_digest(b"TEST_CODE_W13_N02_SOURCE_CONTRACT"),
            UtcMicros::try_new(1_787_027_400_000_000).unwrap(),
        )
    };
    let mut store = BusinessIntentStore::open(&database).unwrap();
    let snapshot = store.record_initial(&draft).unwrap().snapshot().clone();
    let evidence_sha256 = news_flash_evidence_sha256(&sources);
    let channel = "TEST_CODE_W13_N02_CHANNEL".to_owned();
    let attempt_event = PushDeliveryEvent::new_news_flash_attempt(
        "news_flash_aggregated_v1".to_owned(),
        window.decision_key(),
        channel.clone(),
        rendered_len,
        chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
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
        window.decision_key(),
        channel.clone(),
        rendered_len,
        3,
        chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
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
        draft,
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
        unavailable: false,
    }
}

fn replace_n02_terminal_stage(case: &mut N02Case, stage: NewsFlashTransactionStage) {
    let attempt = PushRecord::try_from_authoritative(&case.terminal.attempt)
        .expect("read canonical N02 attempt");
    let attempt_observed_at = attempt
        .news_flash_attempt_observed_at
        .expect("attempt observed at");
    let terminal_observed_at =
        chrono::DateTime::parse_from_rfc3339("2026-08-18T09:30:04+08:00").expect("terminal time");
    let remote_receipt =
        (stage == NewsFlashTransactionStage::Accepted).then(|| NewsFlashRemoteReceipt {
            channel: attempt.channel.clone(),
            provider: "TEST_CODE_W13_N02_SINK_PROVIDER".to_owned(),
            message_id: "TEST_CODE_W13_N02_MESSAGE".to_owned(),
            platform_message_id: "TEST_CODE_W13_N02_PLATFORM".to_owned(),
            accepted_at: case.receipt_accepted_at,
            latency_ms: 2,
        });
    let (reason_code, transport_evidence_sha256) = match stage {
        NewsFlashTransactionStage::Accepted => (None, None),
        NewsFlashTransactionStage::DefinitivelyRejected => (
            Some("TEST_CODE_W13_N02_REJECTED".to_owned()),
            Some("b".repeat(64)),
        ),
        NewsFlashTransactionStage::Uncertain => {
            (Some("TEST_CODE_W13_N02_UNCERTAIN".to_owned()), None)
        }
        NewsFlashTransactionStage::SinkAttempt => {
            panic!("terminal fixture requires terminal stage")
        }
    };
    let terminal_event = PushDeliveryEvent::new_news_flash_terminal(
        stage,
        attempt.kind,
        attempt
            .news_flash_decision_key
            .expect("attempt decision key"),
        attempt.channel,
        attempt.rendered_len,
        3,
        attempt
            .news_flash_business_date
            .expect("attempt business date"),
        attempt
            .news_flash_reservation_sha256
            .expect("attempt reservation"),
        attempt.news_flash_sources.expect("attempt sources"),
        attempt
            .news_flash_evidence_sha256
            .expect("attempt evidence SHA"),
        attempt
            .news_flash_render_sha256
            .expect("attempt render SHA"),
        attempt.news_flash_attempt_ordinal.expect("attempt ordinal"),
        attempt_observed_at,
        attempt
            .news_flash_sink_attempt_identity
            .expect("attempt identity"),
        attempt.news_flash_sink_attempt_sha256.expect("attempt SHA"),
        case.terminal.attempt.id.clone(),
        remote_receipt,
        terminal_observed_at,
        reason_code,
        transport_evidence_sha256,
    );
    case.terminal.terminal = EventEnvelope::from_event(
        &terminal_event,
        terminal_event
            .news_flash_join_sha256
            .clone()
            .expect("terminal join"),
        "TEST_CODE_W13_N02_TERMINAL_TRACE".to_owned(),
        terminal_observed_at.with_timezone(&chrono::Local),
    )
    .expect("valid replacement terminal");
    case.exact_terminal_bytes =
        serde_json::to_vec(&case.terminal.terminal).expect("replacement terminal bytes");
}

// These in-memory fixtures stand in for an already durable legacy terminal. Only
// the Foundation SQLite store is persisted and reopened by these tests; this
// port has no dispatch or attempt-writing method.
struct StoredDedicatedAuthority<'a> {
    snapshot: IntentSnapshot,
    route: &'a DedicatedConformanceRoute,
    source: DedicatedFixtureSource<'a>,
    descriptor: super::terminal_authority::AuthorityDescriptor,
    queries: std::cell::Cell<usize>,
}

enum DedicatedFixtureSource<'a> {
    P01(&'a FakeP01Source),
    N02(&'a FakeN02Source),
}

impl super::terminal_authority::TerminalAuthorityPort for StoredDedicatedAuthority<'_> {
    fn descriptor(&self) -> &super::terminal_authority::AuthorityDescriptor {
        &self.descriptor
    }

    fn requery_terminal(
        &self,
        decision_id: &crate::monitor::push_job::DecisionId,
    ) -> Result<
        super::terminal_authority::AuthorityQuery,
        super::terminal_authority::AuthorityQueryFailure,
    > {
        use super::terminal_authority::{AuthorityQuery, AuthorityQueryFailure};
        self.queries.set(self.queries.get() + 1);
        if &self
            .snapshot
            .attested_ready_binding()
            .map_err(|_| AuthorityQueryFailure)?
            .decision_id
            != decision_id
        {
            return Err(AuthorityQueryFailure);
        }
        let mapped = match self.source {
            DedicatedFixtureSource::P01(source) => {
                super::dedicated_transport::inspect_p01_dedicated(
                    &self.snapshot,
                    self.route,
                    source,
                )
            }
            DedicatedFixtureSource::N02(source) => {
                super::dedicated_transport::inspect_n02_dedicated(
                    &self.snapshot,
                    NewsFlashWindow::H0930,
                    self.route,
                    source,
                )
            }
        };
        match mapped {
            Ok(record) => Ok(AuthorityQuery::Terminal(Box::new(record))),
            Err(DedicatedConformanceError::TerminalMissing) => Ok(AuthorityQuery::Missing),
            Err(DedicatedConformanceError::TerminalPendingSeal) => Ok(AuthorityQuery::PendingSeal),
            Err(_) => Err(AuthorityQueryFailure),
        }
    }
}

struct DedicatedRecoveryBindings<'a> {
    route: &'a DedicatedConformanceRoute,
    policy: &'a CompletionPolicy,
    authority: &'a StoredDedicatedAuthority<'a>,
}

impl super::reconciler::RecoveryBindingsPort for DedicatedRecoveryBindings<'_> {
    fn resolve<'a>(
        &'a self,
        intent: &super::intent_store::AttestedReadyIntent,
    ) -> Result<super::reconciler::RecoveryBindings<'a>, super::reconciler::RecoveryBindingError>
    {
        assert_eq!(
            intent.intent_id,
            self.authority
                .snapshot
                .attested_ready_binding()
                .unwrap()
                .intent_id
        );
        assert_eq!(
            intent.business_date,
            self.authority
                .snapshot
                .attested_ready_binding()
                .unwrap()
                .business_date
        );
        Ok(super::reconciler::RecoveryBindings::new(
            self.route.template(),
            self.policy,
            self.authority,
        ))
    }
}

fn conformance_time(value: i64) -> UtcMicros {
    UtcMicros::try_new(value).unwrap()
}

fn conformance_dispatch(
    store: &mut BusinessIntentStore,
    snapshot: &IntentSnapshot,
) -> IntentSnapshot {
    use super::{IntentState, IntentTransitionCommand, LeaseAction, LeaseOwnerId, TransitionActor};
    use crate::monitor::push_job::ReasonCode;
    let intent = snapshot.attested_ready_binding().unwrap();
    let command = IntentTransitionCommand::try_new(
        intent.intent_id.clone(),
        IntentState::PendingDispatch,
        IntentState::AwaitingAuthority,
        snapshot.version(),
        TransitionActor::try_new("TEST_CODE_W13_DISPATCH".to_owned()).unwrap(),
        ReasonCode::IntentDispatchClaimed,
        conformance_time(1_787_027_401_000_000),
        LeaseAction::Acquire {
            owner: LeaseOwnerId::try_new("TEST_CODE_W13_FINALIZER".to_owned()).unwrap(),
            until: conformance_time(1_787_027_700_000_000),
        },
    )
    .unwrap();
    store.apply_nonterminal_transition(&command).unwrap();
    store.inspect(&intent.intent_id).unwrap().unwrap()
}

fn conformance_recovery_config() -> super::reconciler::RecoveryConfig {
    super::reconciler::RecoveryConfig::try_new(
        super::LeaseOwnerId::try_new("TEST_CODE_W13_RECOVERY".to_owned()).unwrap(),
        super::TransitionActor::try_new("TEST_CODE_W13_RECOVERY".to_owned()).unwrap(),
        conformance_time(1_787_027_800_000_000),
        conformance_time(1_787_028_000_000_000),
        10,
        5,
    )
    .unwrap()
}

fn conformance_prepare_accepted(
    store: &mut BusinessIntentStore,
    awaiting: &IntentSnapshot,
    route: &DedicatedConformanceRoute,
    policy: &CompletionPolicy,
    authority: &StoredDedicatedAuthority<'_>,
) {
    use super::business_finalizer::{
        prepare_accepted_finalization, AcceptedPreparationOutcome, AcceptedPreparationRequest,
        FinalizerFence,
    };
    let intent = awaiting.attested_ready_binding().unwrap();
    let request = AcceptedPreparationRequest::new(
        intent.intent_id.clone(),
        awaiting.version(),
        super::TransitionActor::try_new("TEST_CODE_W13_FINALIZER".to_owned()).unwrap(),
        FinalizerFence::new(
            super::LeaseOwnerId::try_new("TEST_CODE_W13_FINALIZER".to_owned()).unwrap(),
            awaiting.lease_generation(),
            awaiting.lease_until().unwrap(),
        ),
        conformance_time(1_787_027_405_000_000),
        conformance_time(1_787_027_406_000_000),
    )
    .unwrap();
    assert!(matches!(
        prepare_accepted_finalization(store, request, route.template(), policy, authority).unwrap(),
        AcceptedPreparationOutcome::Pending(_)
    ));
    assert_eq!(
        store.inspect(&intent.intent_id).unwrap().unwrap().state(),
        super::IntentState::AwaitingFinalizer
    );
}

#[test]
fn w13_shared_sqlite_date_and_window_material_derive_distinct_intents() {
    let p01 = p01_case();
    let mut p01_store =
        BusinessIntentStore::open(&p01._root.path().join("business.sqlite3")).unwrap();
    let next_day = InitialIntentIdentity::new(
        Namespace::Production,
        UnitId::try_new("MU-p01".to_owned()).unwrap(),
        OccurrenceIdentityMaterial::new(
            BusinessDate::parse("2026-08-19").unwrap(),
            OccurrenceFamily::try_new("p01-business-date".to_owned()).unwrap(),
            OccurrenceKey::try_new("2026-08-19".to_owned()).unwrap(),
        ),
        CompletionOwnerId::try_new("p01-business-date-once".to_owned()).unwrap(),
        SourceContractId::try_new("p01-source".to_owned()).unwrap(),
        SubjectId::Global,
        AudienceId::try_new("portfolio-owner".to_owned()).unwrap(),
    );
    let next_day_draft = InitialIntentDraft::ready_for_recovery_test(
        next_day,
        p01.snapshot.prepared_push_bytes().unwrap().to_vec(),
        p01.snapshot.rendered_bytes().unwrap().to_vec(),
        p01.template.sha256().clone(),
        raw_digest(b"TEST_CODE_W13_P01_SOURCE_CONTRACT"),
        conformance_time(1_787_027_400_000_000),
    )
    .unwrap();
    let InitialIntentOutcome::Inserted(next_day_snapshot) =
        p01_store.record_initial(&next_day_draft).unwrap()
    else {
        panic!("different P01 date must insert a different intent");
    };
    assert_ne!(next_day_snapshot.intent_id(), p01.snapshot.intent_id());
    assert_eq!(next_day_snapshot.business_date(), "2026-08-19");
    assert!(matches!(
        p01_store.record_initial(&p01.draft).unwrap(),
        InitialIntentOutcome::ExistingIdentical(_)
    ));

    let n02 = n02_case();
    let mut n02_store =
        BusinessIntentStore::open(&n02._root.path().join("business.sqlite3")).unwrap();
    for (date, window) in [("2026-08-19", "09:30"), ("2026-08-18", "11:30")] {
        let identity = InitialIntentIdentity::new(
            Namespace::Production,
            UnitId::try_new("MU-news-flash-aggregate".to_owned()).unwrap(),
            OccurrenceIdentityMaterial::new(
                BusinessDate::parse(date).unwrap(),
                OccurrenceFamily::try_new("news-flash-window".to_owned()).unwrap(),
                OccurrenceKey::try_new(window.to_owned()).unwrap(),
            ),
            CompletionOwnerId::try_new("news-flash-accepted-window".to_owned()).unwrap(),
            SourceContractId::try_new("news-flash-authority-v5".to_owned()).unwrap(),
            SubjectId::Global,
            AudienceId::try_new("portfolio-owner".to_owned()).unwrap(),
        );
        let draft = super::intent_store::n02_test_support::ready_default(
            identity,
            n02.snapshot.rendered_bytes().unwrap().to_vec(),
            n02.route.template().sha256().clone(),
            raw_digest(b"TEST_CODE_W13_N02_SOURCE_CONTRACT"),
            conformance_time(1_787_027_400_000_000),
        );
        let InitialIntentOutcome::Inserted(snapshot) = n02_store.record_initial(&draft).unwrap()
        else {
            panic!("different N02 date/window material must insert a different intent");
        };
        assert_ne!(snapshot.intent_id(), n02.snapshot.intent_id());
        assert_eq!(snapshot.business_date(), date);
    }
    assert!(matches!(
        n02_store.record_initial(&n02.draft).unwrap(),
        InitialIntentOutcome::ExistingIdentical(_)
    ));
}

#[test]
fn w13_p01_shared_sqlite_accepted_terminal_recovers_after_reopen() {
    use super::IntentState;
    let case = p01_case();
    let database = case._root.path().join("business.sqlite3");
    let source = source_for(P01DedicatedTerminalQuery::Terminal(Box::new(
        case.terminal.clone(),
    )));
    let authority = StoredDedicatedAuthority {
        snapshot: case.snapshot.clone(),
        route: &case.route,
        source: DedicatedFixtureSource::P01(&source),
        descriptor: case.route.authority_descriptor().unwrap(),
        queries: std::cell::Cell::new(0),
    };
    let mut store = BusinessIntentStore::open(&database).unwrap();
    let awaiting = conformance_dispatch(&mut store, &case.snapshot);
    let direct = verify_p01_dedicated(
        &awaiting,
        &case.route,
        &case.policy,
        &source,
        conformance_time(1_787_027_405_000_000),
    )
    .unwrap();
    assert!(matches!(
        direct.view(),
        DeliveryResultView::TransportAccepted(_)
    ));
    let compensation = p01_case_for_mode("MU-p01", SubjectId::Global, "Compensation");
    let scheduled_envelope: DeliveryEnvelope =
        serde_json::from_slice(&case.terminal.envelope_canonical).unwrap();
    let compensation_envelope: DeliveryEnvelope =
        serde_json::from_slice(&compensation.terminal.envelope_canonical).unwrap();
    assert_ne!(
        compensation.terminal.legacy_decision_identity,
        case.terminal.legacy_decision_identity
    );
    assert_eq!(
        compensation_envelope.schedule_occurrence_identity,
        scheduled_envelope.schedule_occurrence_identity
    );
    assert_eq!(
        compensation_envelope.rendered_content_sha256,
        scheduled_envelope.rendered_content_sha256
    );
    let compensation_source = source_for(P01DedicatedTerminalQuery::Terminal(Box::new(
        compensation.terminal.clone(),
    )));
    assert!(matches!(
        verify_p01_dedicated(
            &compensation.snapshot,
            &compensation.route,
            &compensation.policy,
            &compensation_source,
            conformance_time(1_787_027_405_000_000)
        )
        .unwrap()
        .view(),
        DeliveryResultView::TransportAccepted(_)
    ));
    assert_eq!(
        verify_p01_dedicated(
            &awaiting,
            &case.route,
            &case.policy,
            &compensation_source,
            conformance_time(1_787_027_405_000_000)
        ),
        Err(DedicatedConformanceError::P01BindingMismatch {
            field: "source_evidence_fingerprint",
        })
    );
    let intent = awaiting.attested_ready_binding().unwrap();
    conformance_prepare_accepted(&mut store, &awaiting, &case.route, &case.policy, &authority);
    let bindings = DedicatedRecoveryBindings {
        route: &case.route,
        policy: &case.policy,
        authority: &authority,
    };
    drop(store);
    let mut store = BusinessIntentStore::open(&database).unwrap();
    let report =
        super::reconciler::reconcile_startup(&mut store, &conformance_recovery_config(), &bindings)
            .unwrap();
    assert_eq!(report.entries().len(), 1);
    let completed = store.inspect(&intent.intent_id).unwrap().unwrap();
    assert_eq!(completed.state(), IntentState::Completed);
    assert!(matches!(
        store.record_initial(&case.draft).unwrap(),
        InitialIntentOutcome::ExistingIdentical(_)
    ));
    assert_eq!(completed.business_date(), "2026-08-18");
    assert_eq!(completed.rendered_sha256(), case.snapshot.rendered_sha256());
    let chain = store.inspect_transition_chain(&intent.intent_id).unwrap();
    let receipt = chain.last().unwrap();
    assert_eq!(
        receipt.terminal_ref_id(),
        Some(case.terminal.ref_id.as_str())
    );
    let mapped =
        super::dedicated_transport::inspect_p01_dedicated(&case.snapshot, &case.route, &source)
            .unwrap();
    assert_eq!(
        receipt.terminal_binding_sha256(),
        Some(&mapped.binding_sha256)
    );
    assert_eq!(mapped.evidence_sha256, case.evidence_sha256);
    assert_eq!(mapped.unit_id.as_str(), "MU-p01");
    assert_eq!(
        mapped.rendered_sha256,
        *completed.rendered_sha256().unwrap()
    );
    assert_eq!(
        receipt.terminal_disposition(),
        Some(crate::monitor::push_job::TerminalDisposition::Accepted)
    );
    assert_eq!(
        source
            .queried_dates
            .borrow()
            .iter()
            .all(|date| date == "2026-08-18"),
        true
    );
    assert!(authority.queries.get() >= 2);
    let version = completed.version();
    let second =
        super::reconciler::reconcile_startup(&mut store, &conformance_recovery_config(), &bindings)
            .unwrap();
    assert!(second.entries().is_empty());
    assert_eq!(
        store.inspect(&intent.intent_id).unwrap().unwrap().version(),
        version
    );
    assert_eq!(
        store
            .inspect_transition_chain(&intent.intent_id)
            .unwrap()
            .len(),
        chain.len()
    );
}

#[test]
fn w13_n02_shared_sqlite_accepted_terminal_recovers_after_reopen() {
    use super::IntentState;
    let case = n02_case();
    let database = case._root.path().join("business.sqlite3");
    let source = n02_source(&case, 0);
    let authority = StoredDedicatedAuthority {
        snapshot: case.snapshot.clone(),
        route: &case.route,
        source: DedicatedFixtureSource::N02(&source),
        descriptor: case.route.authority_descriptor().unwrap(),
        queries: std::cell::Cell::new(0),
    };
    let mut store = BusinessIntentStore::open(&database).unwrap();
    let awaiting = conformance_dispatch(&mut store, &case.snapshot);
    let direct = verify_n02_dedicated(
        &awaiting,
        NewsFlashWindow::H0930,
        &case.route,
        &case.policy,
        &source,
        conformance_time(1_787_027_405_000_000),
    )
    .unwrap();
    let DeliveryResultView::TransportAccepted(verified) = direct.view() else {
        panic!("expected accepted N02 terminal");
    };
    assert_eq!(
        verified.evidence_sha256(),
        &raw_digest(&case.exact_terminal_bytes)
    );
    let intent = awaiting.attested_ready_binding().unwrap();
    conformance_prepare_accepted(&mut store, &awaiting, &case.route, &case.policy, &authority);
    drop(store);
    let mut store = BusinessIntentStore::open(&database).unwrap();
    let persisted = store.inspect(&intent.intent_id).unwrap().unwrap();
    assert_eq!(
        persisted.prepared_push_bytes(),
        case.snapshot.prepared_push_bytes()
    );
    assert_eq!(persisted.rendered_bytes(), case.snapshot.rendered_bytes());
    assert_eq!(
        persisted.attested_n02_binding().unwrap().reservation,
        case.snapshot.attested_n02_binding().unwrap().reservation
    );
    // Foundation SQLite is reopened; legacy authority remains a read-only in-memory stand-in.
    let authority = StoredDedicatedAuthority {
        snapshot: persisted,
        route: &case.route,
        source: DedicatedFixtureSource::N02(&source),
        descriptor: case.route.authority_descriptor().unwrap(),
        queries: std::cell::Cell::new(0),
    };
    let bindings = DedicatedRecoveryBindings {
        route: &case.route,
        policy: &case.policy,
        authority: &authority,
    };
    super::reconciler::reconcile_startup(&mut store, &conformance_recovery_config(), &bindings)
        .unwrap();
    let completed = store.inspect(&intent.intent_id).unwrap().unwrap();
    assert_eq!(completed.state(), IntentState::Completed);
    assert!(matches!(
        store.record_initial(&case.draft).unwrap(),
        InitialIntentOutcome::ExistingIdentical(_)
    ));
    assert_eq!(completed.business_date(), "2026-08-18");
    assert_eq!(completed.rendered_sha256(), case.snapshot.rendered_sha256());
    let chain = store.inspect_transition_chain(&intent.intent_id).unwrap();
    let receipt = chain.last().unwrap();
    assert_eq!(
        receipt.terminal_ref_id(),
        Some(case.terminal.terminal.id.as_str())
    );
    let mapped = super::dedicated_transport::inspect_n02_dedicated(
        &case.snapshot,
        NewsFlashWindow::H0930,
        &case.route,
        &source,
    )
    .unwrap();
    assert_eq!(
        receipt.terminal_binding_sha256(),
        Some(&mapped.binding_sha256)
    );
    assert_eq!(
        mapped.evidence_sha256,
        raw_digest(&case.exact_terminal_bytes)
    );
    assert_eq!(mapped.unit_id.as_str(), "MU-news-flash-aggregate");
    assert_eq!(
        mapped.rendered_sha256,
        *completed.rendered_sha256().unwrap()
    );
    assert_eq!(
        receipt.terminal_disposition(),
        Some(crate::monitor::push_job::TerminalDisposition::Accepted)
    );
    assert!(source
        .queried
        .borrow()
        .iter()
        .all(|(date, window)| date == "2026-08-18" && *window == NewsFlashWindow::H0930));
    assert!(authority.queries.get() >= 1);
    let version = completed.version();
    super::reconciler::reconcile_startup(&mut store, &conformance_recovery_config(), &bindings)
        .unwrap();
    assert_eq!(
        store.inspect(&intent.intent_id).unwrap().unwrap().version(),
        version
    );
    assert_eq!(
        store
            .inspect_transition_chain(&intent.intent_id)
            .unwrap()
            .len(),
        chain.len()
    );
    let NewsFlashWindowTerminalQuery::Terminal(stored) = &source.result else {
        panic!("N02 authority fixture changed during recovery");
    };
    assert_eq!(stored.attempt.id, case.terminal.attempt.id);
    assert_eq!(stored.terminal.id, case.terminal.terminal.id);
}

#[test]
fn w13_n02_shared_sqlite_nonaccepted_terminal_never_completes() {
    use super::IntentState;
    for (stage, expected) in [
        (
            NewsFlashTransactionStage::Uncertain,
            IntentState::ResolutionRequired,
        ),
        (
            NewsFlashTransactionStage::DefinitivelyRejected,
            IntentState::AwaitingAuthority,
        ),
    ] {
        let mut case = n02_case();
        replace_n02_terminal_stage(&mut case, stage);
        let database = case._root.path().join("business.sqlite3");
        let source = n02_source(&case, 0);
        let authority = StoredDedicatedAuthority {
            snapshot: case.snapshot.clone(),
            route: &case.route,
            source: DedicatedFixtureSource::N02(&source),
            descriptor: case.route.authority_descriptor().unwrap(),
            queries: std::cell::Cell::new(0),
        };
        let mut store = BusinessIntentStore::open(&database).unwrap();
        let awaiting = conformance_dispatch(&mut store, &case.snapshot);
        let direct = verify_n02_dedicated(
            &awaiting,
            NewsFlashWindow::H0930,
            &case.route,
            &case.policy,
            &source,
            conformance_time(1_787_027_405_000_000),
        )
        .unwrap();
        assert_eq!(
            direct.completion_eligibility(),
            CompletionEligibility::Never
        );
        let intent = awaiting.attested_ready_binding().unwrap();
        let bindings = DedicatedRecoveryBindings {
            route: &case.route,
            policy: &case.policy,
            authority: &authority,
        };
        drop(store);
        let mut store = BusinessIntentStore::open(&database).unwrap();
        super::reconciler::reconcile_startup(&mut store, &conformance_recovery_config(), &bindings)
            .unwrap();
        let current = store.inspect(&intent.intent_id).unwrap().unwrap();
        assert_eq!(current.state(), expected);
        assert!(store
            .inspect_transition_chain(&intent.intent_id)
            .unwrap()
            .iter()
            .all(|receipt| receipt.to_state() != IntentState::Completed));
        let version = current.version();
        super::reconciler::reconcile_startup(&mut store, &conformance_recovery_config(), &bindings)
            .unwrap();
        assert_eq!(
            store.inspect(&intent.intent_id).unwrap().unwrap().version(),
            version
        );
        assert!(source
            .queried
            .borrow()
            .iter()
            .all(|(date, window)| date == "2026-08-18" && *window == NewsFlashWindow::H0930));
    }
}

#[test]
fn w13_p01_shared_sqlite_missing_pending_and_mismatched_authority_block_recovery() {
    use super::reconciler::RecoveryBoundary;
    use super::IntentState;
    for scenario in 0..3 {
        let case = p01_case();
        let result = match scenario {
            0 => P01DedicatedTerminalQuery::Missing,
            1 => P01DedicatedTerminalQuery::PendingSeal {
                state: DecisionState::Reserved,
            },
            _ => {
                let mut terminal = case.terminal.clone();
                terminal.accepted_channel = Some("TEST_CODE_W13_WRONG_CHANNEL".to_owned());
                P01DedicatedTerminalQuery::Terminal(Box::new(terminal))
            }
        };
        let database = case._root.path().join("business.sqlite3");
        let source = source_for(result);
        let authority = StoredDedicatedAuthority {
            snapshot: case.snapshot.clone(),
            route: &case.route,
            source: DedicatedFixtureSource::P01(&source),
            descriptor: case.route.authority_descriptor().unwrap(),
            queries: std::cell::Cell::new(0),
        };
        let mut store = BusinessIntentStore::open(&database).unwrap();
        let awaiting = conformance_dispatch(&mut store, &case.snapshot);
        let intent = awaiting.attested_ready_binding().unwrap();
        let bindings = DedicatedRecoveryBindings {
            route: &case.route,
            policy: &case.policy,
            authority: &authority,
        };
        drop(store);
        let mut store = BusinessIntentStore::open(&database).unwrap();
        let report = super::reconciler::reconcile_startup(
            &mut store,
            &conformance_recovery_config(),
            &bindings,
        )
        .unwrap();
        assert_eq!(
            report.entry(intent.intent_id.as_str()).unwrap().boundary(),
            RecoveryBoundary::AuthorityBlocked
        );
        let current = store.inspect(&intent.intent_id).unwrap().unwrap();
        assert_eq!(current.state(), IntentState::AwaitingAuthority);
        assert!(store
            .inspect_transition_chain(&intent.intent_id)
            .unwrap()
            .iter()
            .all(|event| event.to_state() != IntentState::Completed));
        assert!(source
            .queried_dates
            .borrow()
            .iter()
            .all(|date| date == "2026-08-18"));
        assert!(authority.queries.get() > 0);
        let version = current.version();
        super::reconciler::reconcile_startup(&mut store, &conformance_recovery_config(), &bindings)
            .unwrap();
        assert_eq!(
            store.inspect(&intent.intent_id).unwrap().unwrap().version(),
            version
        );
    }
}

#[test]
fn w13_n02_shared_sqlite_missing_pending_and_mismatched_authority_block_recovery() {
    use super::reconciler::RecoveryBoundary;
    use super::IntentState;
    for scenario in 0..3 {
        let case = n02_case();
        let result = match scenario {
            0 => NewsFlashWindowTerminalQuery::Missing,
            1 => NewsFlashWindowTerminalQuery::PendingSeal,
            _ => {
                let mut terminal = case.terminal.clone();
                terminal.terminal.payload["news_flash_attempt_envelope_id"] =
                    serde_json::json!("TEST_CODE_W13_WRONG_ATTEMPT_ID");
                NewsFlashWindowTerminalQuery::Terminal(Box::new(terminal))
            }
        };
        let database = case._root.path().join("business.sqlite3");
        let source = FakeN02Source {
            result,
            queried: RefCell::new(Vec::new()),
            n01_quota_probe: 0,
            unavailable: false,
        };
        let authority = StoredDedicatedAuthority {
            snapshot: case.snapshot.clone(),
            route: &case.route,
            source: DedicatedFixtureSource::N02(&source),
            descriptor: case.route.authority_descriptor().unwrap(),
            queries: std::cell::Cell::new(0),
        };
        let mut store = BusinessIntentStore::open(&database).unwrap();
        let awaiting = conformance_dispatch(&mut store, &case.snapshot);
        let intent = awaiting.attested_ready_binding().unwrap();
        let bindings = DedicatedRecoveryBindings {
            route: &case.route,
            policy: &case.policy,
            authority: &authority,
        };
        drop(store);
        let mut store = BusinessIntentStore::open(&database).unwrap();
        let report = super::reconciler::reconcile_startup(
            &mut store,
            &conformance_recovery_config(),
            &bindings,
        )
        .unwrap();
        assert_eq!(
            report.entry(intent.intent_id.as_str()).unwrap().boundary(),
            RecoveryBoundary::AuthorityBlocked
        );
        let current = store.inspect(&intent.intent_id).unwrap().unwrap();
        assert_eq!(current.state(), IntentState::AwaitingAuthority);
        assert!(store
            .inspect_transition_chain(&intent.intent_id)
            .unwrap()
            .iter()
            .all(|event| event.to_state() != IntentState::Completed));
        assert!(source
            .queried
            .borrow()
            .iter()
            .all(|(date, window)| date == "2026-08-18" && *window == NewsFlashWindow::H0930));
        assert!(authority.queries.get() > 0);
        let version = current.version();
        super::reconciler::reconcile_startup(&mut store, &conformance_recovery_config(), &bindings)
            .unwrap();
        assert_eq!(
            store.inspect(&intent.intent_id).unwrap().unwrap().version(),
            version
        );
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

#[test]
fn w13_n02_dedicated_preserves_rejected_uncertain_and_open_states() {
    for (stage, expected) in [
        (NewsFlashTransactionStage::DefinitivelyRejected, "rejected"),
        (NewsFlashTransactionStage::Uncertain, "uncertain"),
    ] {
        let mut case = n02_case();
        replace_n02_terminal_stage(&mut case, stage);
        let source = n02_source(&case, 0);
        let result = verify_n02_dedicated(
            &case.snapshot,
            NewsFlashWindow::H0930,
            &case.route,
            &case.policy,
            &source,
            UtcMicros::try_new(1_787_027_405_000_000).expect("verified at"),
        )
        .expect("verify N02 non-Accepted terminal");
        match (expected, result.view()) {
            ("rejected", DeliveryResultView::TransportRejected(_))
            | ("uncertain", DeliveryResultView::TransportUncertain(_)) => {}
            (_, observed) => panic!("unexpected N02 disposition: {observed:?}"),
        }
        assert_eq!(
            result.completion_eligibility(),
            CompletionEligibility::Never
        );
    }

    let case = n02_case();
    for result in [
        NewsFlashWindowTerminalQuery::Missing,
        NewsFlashWindowTerminalQuery::PendingSeal,
    ] {
        let source = FakeN02Source {
            result,
            queried: RefCell::new(Vec::new()),
            n01_quota_probe: 0,
            unavailable: false,
        };
        assert!(matches!(
            verify_n02_dedicated(
                &case.snapshot,
                NewsFlashWindow::H0930,
                &case.route,
                &case.policy,
                &source,
                UtcMicros::try_new(1_787_027_405_000_000).expect("verified at"),
            ),
            Err(DedicatedConformanceError::TerminalMissing)
                | Err(DedicatedConformanceError::TerminalPendingSeal)
        ));
    }
    let unavailable = FakeN02Source {
        result: NewsFlashWindowTerminalQuery::Missing,
        queried: RefCell::new(Vec::new()),
        n01_quota_probe: 0,
        unavailable: true,
    };
    assert_eq!(
        verify_n02_dedicated(
            &case.snapshot,
            NewsFlashWindow::H0930,
            &case.route,
            &case.policy,
            &unavailable,
            UtcMicros::try_new(1_787_027_405_000_000).expect("verified at"),
        ),
        Err(DedicatedConformanceError::SourceUnavailable)
    );
}

#[test]
fn w13_n02_dedicated_fails_closed_on_exact_binding_corruption() {
    let case = n02_case();
    let wrong_window = n02_source(&case, 0);
    assert!(verify_n02_dedicated(
        &case.snapshot,
        NewsFlashWindow::H1130,
        &case.route,
        &case.policy,
        &wrong_window,
        UtcMicros::try_new(1_787_027_405_000_000).expect("verified at"),
    )
    .is_err());

    let wrong_channel_route = DedicatedConformanceRoute::try_new(
        TerminalTemplateBinding::new(
            TemplateId::try_new("news_flash_aggregated_v1".to_owned()).expect("template id"),
            TemplateVersion::try_new("news_flash_aggregated_v1".to_owned())
                .expect("template version"),
        ),
        ChannelId::try_new("TEST_CODE_W13_N02_WRONG_CHANNEL".to_owned()).expect("wrong channel"),
    )
    .expect("valid N02 route shape");
    let source = n02_source(&case, 0);
    assert!(verify_n02_dedicated(
        &case.snapshot,
        NewsFlashWindow::H0930,
        &wrong_channel_route,
        &case.policy,
        &source,
        UtcMicros::try_new(1_787_027_405_000_000).expect("verified at"),
    )
    .is_err());

    for field in [
        "audit_schema_version",
        "kind",
        "news_flash_business_date",
        "news_flash_decision_key",
        "news_flash_reservation_sha256",
        "news_flash_attempt_ordinal",
        "news_flash_attempt_observed_at",
        "news_flash_sink_attempt_identity",
        "news_flash_sink_attempt_sha256",
        "news_flash_attempt_envelope_id",
        "news_flash_evidence_sha256",
        "news_flash_render_sha256",
        "rendered_len",
        "channel",
        "news_flash_remote_receipt",
    ] {
        let case = n02_case();
        let mut corrupt = case.terminal.clone();
        corrupt.terminal.payload[field] = match field {
            "audit_schema_version" | "news_flash_attempt_ordinal" | "rendered_len" => {
                serde_json::json!(99)
            }
            "news_flash_remote_receipt" => serde_json::json!({
                "accepted_at": "2026-08-18T09:30:03+08:00",
                "channel": "TEST_CODE_W13_N02_WRONG_CHANNEL",
                "latency_ms": 2,
                "message_id": "TEST_CODE_W13_N02_MESSAGE",
                "platform_message_id": "TEST_CODE_W13_N02_PLATFORM",
                "provider": "TEST_CODE_W13_N02_SINK_PROVIDER"
            }),
            _ => serde_json::json!("TEST_CODE_W13_N02_CORRUPT"),
        };
        let source = FakeN02Source {
            result: NewsFlashWindowTerminalQuery::Terminal(Box::new(corrupt)),
            queried: RefCell::new(Vec::new()),
            n01_quota_probe: 0,
            unavailable: false,
        };
        assert!(
            verify_n02_dedicated(
                &case.snapshot,
                NewsFlashWindow::H0930,
                &case.route,
                &case.policy,
                &source,
                UtcMicros::try_new(1_787_027_405_000_000).expect("verified at"),
            )
            .is_err(),
            "corrupt field must fail closed: {field}"
        );
    }

    let case = n02_case();
    let mut alternate = case.terminal.clone();
    alternate.terminal.trace_id = "TEST_CODE_W13_N02_OTHER_TERMINAL_TRACE".to_owned();
    let alternate_bytes = serde_json::to_vec(&alternate.terminal).expect("alternate exact bytes");
    let source = FakeN02Source {
        result: NewsFlashWindowTerminalQuery::Terminal(Box::new(alternate)),
        queried: RefCell::new(Vec::new()),
        n01_quota_probe: 0,
        unavailable: false,
    };
    let result = verify_n02_dedicated(
        &case.snapshot,
        NewsFlashWindow::H0930,
        &case.route,
        &case.policy,
        &source,
        UtcMicros::try_new(1_787_027_405_000_000).expect("verified at"),
    )
    .expect("valid alternate exact terminal envelope");
    let DeliveryResultView::TransportAccepted(verified) = result.view() else {
        panic!("expected alternate N02 Accepted");
    };
    assert_eq!(verified.evidence_sha256(), &raw_digest(&alternate_bytes));
    assert_ne!(
        verified.evidence_sha256(),
        &raw_digest(&case.exact_terminal_bytes)
    );
}

// Exact Task 1 real-gate A/B vectors. This library fixture exercises the adapter;
// news_aggregator_init::tests::n02_legacy_identity_real_gate_vectors establishes gate origin.
fn golden_n02_case(suffix: &str) -> N02Case {
    let sources = vec![NewsFlashAuditSource {
        event_id: format!("TEST_CODE_EVENT_{suffix}"),
        provider: "TEST_CODE_PROVIDER".into(),
        source: "TEST_CODE_SOURCE".into(),
        published_at: chrono::DateTime::parse_from_rfc3339("2026-09-28T00:00:00+00:00").unwrap(),
        observed_at: chrono::DateTime::parse_from_rfc3339("2026-09-28T00:01:00+00:00").unwrap(),
        batch_id: format!("TEST_CODE_BATCH_{suffix}"),
    }];
    n02_case_with(
        "2026-09-28",
        NewsFlashWindow::H0930,
        "📰 新闻时段聚合 (09:30) Top3:\n1. [政策] TEST_CODE_TITLE_A (强度0 确定性100)\n"
            .as_bytes()
            .to_vec(),
        Some(sources),
        false,
    )
}

fn assert_n02_pair_valid(case: &N02Case) {
    crate::event::PushRecord::try_from_authoritative(&case.terminal.attempt).unwrap();
    crate::event::PushRecord::try_from_authoritative(&case.terminal.terminal).unwrap();
}

#[test]
fn w13_n02_same_date_render_alternate_real_reservation_is_rejected() {
    let a = golden_n02_case("A");
    let b = golden_n02_case("B");
    assert_eq!(a.snapshot.rendered_bytes(), b.snapshot.rendered_bytes());
    for (case, sha) in [
        (
            &a,
            "eba6d8e675bcfa34f1823268099f2244856cf9ab0df9d67e5245d28d8916b05f",
        ),
        (
            &b,
            "774799317c09db050295597009fe1695320178654b8f6b2ec628ed4a5c1f8507",
        ),
    ] {
        assert_eq!(
            case.snapshot
                .attested_n02_binding()
                .unwrap()
                .reservation
                .material()
                .reservation_sha256,
            sha
        );
        assert_n02_pair_valid(case);
        super::dedicated_transport::inspect_n02_dedicated(
            &case.snapshot,
            NewsFlashWindow::H0930,
            &case.route,
            &n02_source(case, 0),
        )
        .unwrap();
    }
    assert!(matches!(
        super::dedicated_transport::inspect_n02_dedicated(
            &a.snapshot,
            NewsFlashWindow::H0930,
            &a.route,
            &n02_source(&b, 0)
        ),
        Err(DedicatedConformanceError::N02BindingMismatch {
            field: "reservation_sha256"
        })
    ));
}

#[test]
fn w13_n02_coherent_ordered_source_substitution_is_rejected() {
    // Faithful source-order fixture: identical display fields produce identical lines.
    // This is not an additional execution of the monitor gate.
    let mut sources = super::intent_store::n02_test_support::sources();
    let mut second = sources[0].clone();
    second.event_id = "TEST_CODE_N02_SECOND".into();
    second.batch_id = "TEST_CODE_N02_SECOND_BATCH".into();
    sources.push(second);
    let rendered = "📰 新闻时段聚合 (09:30) Top3:\n1. [政策] SAME (强度0 确定性100)\n2. [政策] SAME (强度0 确定性100)\n".as_bytes().to_vec();
    let a = n02_case_with(
        "2026-08-18",
        NewsFlashWindow::H0930,
        rendered.clone(),
        Some(sources.clone()),
        false,
    );
    sources.reverse();
    let b = n02_case_with(
        "2026-08-18",
        NewsFlashWindow::H0930,
        rendered,
        Some(sources),
        false,
    );
    assert_n02_pair_valid(&b);
    super::dedicated_transport::inspect_n02_dedicated(
        &b.snapshot,
        NewsFlashWindow::H0930,
        &b.route,
        &n02_source(&b, 0),
    )
    .unwrap();
    assert!(super::dedicated_transport::inspect_n02_dedicated(
        &a.snapshot,
        NewsFlashWindow::H0930,
        &a.route,
        &n02_source(&b, 0)
    )
    .is_err());
}

#[test]
fn w13_n02_direct_wrong_window_rejects_before_lookup() {
    let a = n02_case();
    let b = n02_case_with(
        "2026-08-18",
        NewsFlashWindow::H1130,
        a.snapshot.rendered_bytes().unwrap().to_vec(),
        None,
        false,
    );
    assert_n02_pair_valid(&b);
    let source = n02_source(&b, 0);
    assert_eq!(
        super::dedicated_transport::inspect_n02_dedicated(
            &a.snapshot,
            NewsFlashWindow::H1130,
            &a.route,
            &source
        ),
        Err(DedicatedConformanceError::N02BindingMismatch { field: "window" })
    );
    assert!(source.queried.borrow().is_empty());
}

#[test]
fn w13_n02_raw_audit_render_cannot_replace_legacy_domain_digest() {
    let case = n02_case_with(
        "2026-08-18",
        NewsFlashWindow::H0930,
        b"TEST_CODE_W13_N02_RENDERED".to_vec(),
        None,
        true,
    );
    assert_n02_pair_valid(&case);
    assert_eq!(
        super::dedicated_transport::inspect_n02_dedicated(
            &case.snapshot,
            NewsFlashWindow::H0930,
            &case.route,
            &n02_source(&case, 0)
        ),
        Err(DedicatedConformanceError::N02BindingMismatch {
            field: "render_sha256"
        })
    );
}

#[test]
fn w13_n02_old_opaque_rejects_before_source_lookup() {
    let case = n02_case_with_mode(
        "2026-08-18",
        NewsFlashWindow::H0930,
        b"TEST_CODE_W13_N02_RENDERED".to_vec(),
        None,
        false,
        true,
    );
    assert_n02_pair_valid(&case);
    let source = n02_source(&case, 0);
    assert!(case.snapshot.attested_ready_binding().is_ok());
    assert_eq!(
        super::dedicated_transport::inspect_n02_dedicated(
            &case.snapshot,
            NewsFlashWindow::H0930,
            &case.route,
            &source
        ),
        Err(DedicatedConformanceError::InvalidN02Binding)
    );
    assert!(source.queried.borrow().is_empty());
}

#[test]
fn n02_v2_contract_rejects_legacy_v1_before_authority_query() {
    let case = n02_case();
    let identity = InitialIntentIdentity::new(
        Namespace::test(
            crate::monitor::push_job::RunId::try_new("TEST_CODE_N02_V2_REJECTION".into()).unwrap(),
        ),
        UnitId::try_new("MU-news-flash-aggregate".into()).unwrap(),
        OccurrenceIdentityMaterial::new(
            BusinessDate::parse("2026-08-18").unwrap(),
            OccurrenceFamily::try_new("news-flash-window".into()).unwrap(),
            OccurrenceKey::try_new("09:30".into()).unwrap(),
        ),
        CompletionOwnerId::try_new("news-flash-accepted-window".into()).unwrap(),
        SourceContractId::try_new(crate::monitor::push_job::N02_SOURCE_CONTRACT_ID.into()).unwrap(),
        SubjectId::Global,
        AudienceId::try_new("portfolio-owner".into()).unwrap(),
    );
    let draft = super::intent_store::n02_test_support::ready_default(
        identity,
        case.snapshot.rendered_bytes().unwrap().to_vec(),
        case.route.template().sha256().clone(),
        raw_digest(b"TEST_CODE_N02_V2_CONTRACT"),
        conformance_time(1_787_027_400_000_000),
    );
    let database = case._root.path().join("business.sqlite3");
    let mut store = BusinessIntentStore::open(&database).unwrap();
    let row = store.record_initial(&draft).unwrap().snapshot().clone();
    assert!(row.attested_ready_binding().is_ok());
    let source = n02_source(&case, 0);
    assert_eq!(
        super::dedicated_transport::inspect_n02_dedicated(
            &row,
            NewsFlashWindow::H0930,
            &case.route,
            &source,
        ),
        Err(DedicatedConformanceError::InvalidN02Binding)
    );
    assert!(source.queried.borrow().is_empty());
    let after = store.inspect(draft.intent_id()).unwrap().unwrap();
    assert_eq!(after.state(), row.state());
    assert_eq!(after.version(), row.version());
}

#[test]
fn w13_n02_shared_sqlite_coherent_mismatch_records_only_blocked_recovery_transitions() {
    for variant in ["reservation", "order", "window", "raw_render", "opaque"] {
        let order_sources = if variant == "order" {
            let mut sources = super::intent_store::n02_test_support::sources();
            let mut other = sources[0].clone();
            other.event_id = "TEST_CODE_OTHER_ORDER".into();
            other.batch_id = "TEST_CODE_OTHER_ORDER_BATCH".into();
            sources.push(other);
            Some(sources)
        } else {
            None
        };
        let case = n02_case_with_mode(
            "2026-08-18",
            NewsFlashWindow::H0930,
            b"TEST_CODE_W13_N02_RENDERED".to_vec(),
            order_sources,
            false,
            variant == "opaque",
        );
        let mut sources = case
            .snapshot
            .attested_n02_binding()
            .ok()
            .map(|a| a.reservation.material().sources.clone())
            .unwrap_or_else(super::intent_store::n02_test_support::sources);
        if variant == "reservation" {
            sources[0].event_id = "TEST_CODE_ALTERNATE_EVENT".into();
        }
        if variant == "order" {
            sources.reverse();
        }
        let alternate = n02_case_with(
            "2026-08-18",
            if variant == "window" {
                NewsFlashWindow::H1130
            } else {
                NewsFlashWindow::H0930
            },
            case.snapshot.rendered_bytes().unwrap().to_vec(),
            Some(sources),
            variant == "raw_render",
        );
        assert_n02_pair_valid(&alternate);
        let source = n02_source(&alternate, 0);
        let database = case._root.path().join("business.sqlite3");
        let mut store = BusinessIntentStore::open(&database).unwrap();
        let awaiting = conformance_dispatch(&mut store, &case.snapshot);
        let intent = awaiting.attested_ready_binding().unwrap().intent_id;
        let count = store.inspect_transition_chain(&intent).unwrap().len();
        assert!(
            super::dedicated_transport::inspect_n02_dedicated(
                &awaiting,
                NewsFlashWindow::H0930,
                &case.route,
                &source
            )
            .is_err(),
            "{variant}"
        );
        assert_eq!(
            store.inspect(&intent).unwrap().unwrap().version(),
            awaiting.version(),
            "inspection must not mutate {variant}"
        );
        drop(store);
        let mut store = BusinessIntentStore::open(&database).unwrap();
        let persisted = store.inspect(&intent).unwrap().unwrap();
        let authority = StoredDedicatedAuthority {
            snapshot: persisted,
            route: &case.route,
            source: DedicatedFixtureSource::N02(&source),
            descriptor: case.route.authority_descriptor().unwrap(),
            queries: std::cell::Cell::new(0),
        };
        let bindings = DedicatedRecoveryBindings {
            route: &case.route,
            policy: &case.policy,
            authority: &authority,
        };
        let report = super::reconciler::reconcile_startup(
            &mut store,
            &conformance_recovery_config(),
            &bindings,
        )
        .unwrap();
        assert_eq!(
            report.entry(intent.as_str()).unwrap().boundary(),
            super::reconciler::RecoveryBoundary::AuthorityBlocked,
            "{variant}"
        );
        let after = store.inspect(&intent).unwrap().unwrap();
        assert_eq!(after.state(), super::IntentState::AwaitingAuthority);
        // Existing recovery first takes the expired lease, then records the blocked query.
        // Neither transition qualifies Accepted finalization or stores a terminal receipt.
        assert_eq!(after.version(), awaiting.version() + 2, "{variant}");
        let chain = store.inspect_transition_chain(&intent).unwrap();
        assert_eq!(chain.len(), count + 2, "{variant}");
        for (receipt, reason) in chain[count..].iter().zip([
            crate::monitor::push_job::ReasonCode::IntentDispatchClaimed,
            crate::monitor::push_job::ReasonCode::FinalizerTerminalRefInvalid,
        ]) {
            assert_eq!(receipt.reason(), reason, "{variant}");
            assert_eq!(receipt.from_state(), super::IntentState::AwaitingAuthority);
            assert_eq!(receipt.to_state(), super::IntentState::AwaitingAuthority);
            assert!(receipt.terminal_ref_id().is_none());
            assert!(receipt.terminal_disposition().is_none());
            assert!(receipt.terminal_binding_sha256().is_none());
        }
        super::reconciler::reconcile_startup(&mut store, &conformance_recovery_config(), &bindings)
            .unwrap();
        assert_eq!(
            store.inspect(&intent).unwrap().unwrap().version(),
            after.version(),
            "repeat {variant}"
        );
        assert_eq!(
            store.inspect_transition_chain(&intent).unwrap().len(),
            chain.len(),
            "repeat {variant}"
        );
    }
}

#[test]
fn w13_n02_shared_sqlite_awaiting_authority_recovers_after_reopen() {
    let case = n02_case();
    let source = n02_source(&case, 0); // Read-only in-memory stand-in for durable legacy audit.
    let database = case._root.path().join("business.sqlite3");
    let mut store = BusinessIntentStore::open(&database).unwrap();
    let awaiting = conformance_dispatch(&mut store, &case.snapshot);
    let intent = awaiting.attested_ready_binding().unwrap().intent_id;
    drop(store);
    let mut store = BusinessIntentStore::open(&database).unwrap();
    let authority = StoredDedicatedAuthority {
        snapshot: store.inspect(&intent).unwrap().unwrap(),
        route: &case.route,
        source: DedicatedFixtureSource::N02(&source),
        descriptor: case.route.authority_descriptor().unwrap(),
        queries: std::cell::Cell::new(0),
    };
    let bindings = DedicatedRecoveryBindings {
        route: &case.route,
        policy: &case.policy,
        authority: &authority,
    };
    super::reconciler::reconcile_startup(&mut store, &conformance_recovery_config(), &bindings)
        .unwrap();
    let completed = store.inspect(&intent).unwrap().unwrap();
    assert_eq!(completed.state(), super::IntentState::Completed);
    assert_eq!(
        completed.prepared_push_bytes(),
        awaiting.prepared_push_bytes()
    );
    assert_eq!(completed.rendered_bytes(), awaiting.rendered_bytes());
    let chain = store.inspect_transition_chain(&intent).unwrap();
    assert_eq!(
        chain.last().unwrap().terminal_ref_id(),
        Some(case.terminal.terminal.id.as_str())
    );
    super::reconciler::reconcile_startup(&mut store, &conformance_recovery_config(), &bindings)
        .unwrap();
    assert_eq!(
        store.inspect(&intent).unwrap().unwrap().version(),
        completed.version()
    );
    assert_eq!(
        store.inspect_transition_chain(&intent).unwrap().len(),
        chain.len()
    );
}
