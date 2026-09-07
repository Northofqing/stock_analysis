use std::cell::RefCell;

use crate::durable_delivery::{
    DeliveryEnvelope, DeliverySubKind, FoundationTerminalDisposition, P01DedicatedTerminalQuery,
    P01DedicatedTerminalRecord, PushKind,
};
use crate::monitor::push_job::{
    raw_digest, w09_completion_policy_fixture, AudienceId, AuthorityClass, BusinessDate, ChannelId,
    CompletionOwnerId, DeliveryResultView, Namespace, OccurrenceFamily, OccurrenceIdentityMaterial,
    OccurrenceKey, SourceContractId, SubjectId, TemplateId, TemplateVersion, UnitId, UtcMicros,
};

use super::dedicated_transport::{
    verify_p01_dedicated, DedicatedConformanceRoute, DedicatedSourceFailure,
    P01DedicatedTerminalSource,
};
use super::{
    BusinessIntentStore, FoundationSchemaMigration, InitialIntentDraft, InitialIntentIdentity,
    InitialIntentOutcome, TerminalTemplateBinding,
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

#[test]
fn w13_p01_dedicated_maps_exact_accepted_through_w09() {
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
        UnitId::try_new("MU-p01".to_owned()).expect("unit"),
        OccurrenceIdentityMaterial::new(
            BusinessDate::parse("2026-08-18").expect("business date"),
            OccurrenceFamily::try_new("p01-business-date".to_owned()).expect("family"),
            OccurrenceKey::try_new("2026-08-18".to_owned()).expect("key"),
        ),
        CompletionOwnerId::try_new("p01-business-date-once".to_owned()).expect("owner"),
        SourceContractId::try_new("p01-source".to_owned()).expect("source contract"),
        SubjectId::Global,
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
        evidence_bytes,
        evidence_sha256: evidence_sha256.as_str().to_owned(),
        durable_schema_version: crate::durable_delivery::DURABLE_SCHEMA_VERSION,
    };
    let source = FakeP01Source {
        result: P01DedicatedTerminalQuery::Terminal(Box::new(terminal)),
        queried_dates: RefCell::new(Vec::new()),
    };
    let route = DedicatedConformanceRoute::try_new(
        template,
        ChannelId::try_new("TEST_CODE_W13_P01_CHANNEL".to_owned()).expect("channel"),
    )
    .expect("valid P01 route");
    let policy = w09_completion_policy_fixture(
        "MU-p01",
        "p01-business-date-once",
        vec![AuthorityClass::P01Dedicated],
    );

    let result = verify_p01_dedicated(
        &snapshot,
        &route,
        &policy,
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
    assert_eq!(verified.evidence_sha256(), &evidence_sha256);
}
