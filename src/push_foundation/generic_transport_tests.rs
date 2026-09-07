use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{TimeZone, Utc};

use crate::durable_delivery::{
    AuthoritativeDeliveryRequest, AuthoritativeSink, AuthoritativeSinkPort,
    AuthoritativeSinkResult, CoordinatorConfig, DeliverySubKind, DurableDeliveryCoordinator,
    ImmutableAppendPort, PushKind, TypedReceipt,
};
use crate::monitor::push_job::{
    ChannelId, CompatId, CompatibilityEvidenceRef, CompletionEligibility, DeliveryResult,
    DeliveryResultView, ReasonCode, TerminalDisposition, UtcMicros, WeakOutcome, WeakOutcomeKind,
};

use super::generic_transport::{
    GenericDispatchFence, GenericDispatchRequest, GenericTransportAuthorityAdapter,
    GenericTransportError, GenericTransportRoute, RequiredChannelClassification,
    RequiredChannelError, RequiredChannelObservation, RequiredChannelResults,
};
use super::terminal_authority::{terminal_binding_sha256, verify_terminal};
use super::terminal_authority_tests::fixture;
use super::terminal_authority_tests::FakeAuthority;
use super::{
    BusinessIntentStore, IntentSnapshot, IntentState, IntentTransitionCommand, LeaseAction,
    LeaseOwnerId, TransitionActor,
};

static NEXT_W12_DATABASE: AtomicUsize = AtomicUsize::new(1);

struct DurableFixture {
    database_path: PathBuf,
    coordinator: Option<Arc<DurableDeliveryCoordinator>>,
}

impl DurableFixture {
    fn new(label: &str) -> Self {
        let sequence = NEXT_W12_DATABASE.fetch_add(1, Ordering::SeqCst);
        let test_code = format!(
            "TEST_CODE_W12_GENERIC_{label}_{}_{}",
            std::process::id(),
            sequence
        );
        let root = PathBuf::from("data/test").join(&test_code);
        std::fs::create_dir_all("data/test").expect("create TEST_CODE namespace");
        std::fs::create_dir(&root).expect("create unique W12 TEST_CODE root");
        let database_path = root.join("durable_delivery.sqlite3");
        let coordinator = Arc::new(
            DurableDeliveryCoordinator::open(CoordinatorConfig::test(
                &database_path,
                &test_code,
                format!("owner-{test_code}"),
            ))
            .expect("open W12 durable coordinator"),
        );
        Self {
            database_path,
            coordinator: Some(coordinator),
        }
    }

    fn coordinator(&self) -> &Arc<DurableDeliveryCoordinator> {
        self.coordinator.as_ref().expect("live W12 coordinator")
    }
}

impl Drop for DurableFixture {
    fn drop(&mut self) {
        self.coordinator.take();
        for suffix in ["", "-journal", "-shm", "-wal"] {
            let path = PathBuf::from(format!("{}{suffix}", self.database_path.display()));
            let _ = std::fs::remove_file(path);
        }
        if let Some(parent) = self.database_path.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

#[derive(Default)]
struct MemoryAppend {
    records: Mutex<BTreeMap<String, (String, Vec<u8>, String)>>,
}

impl ImmutableAppendPort for MemoryAppend {
    fn append_exact(
        &self,
        record_kind: &str,
        identity: &str,
        canonical_bytes: &[u8],
        sha256: &str,
    ) -> crate::durable_delivery::Result<String> {
        let mut records = self.records.lock().expect("append records");
        let proposed = (
            record_kind.to_owned(),
            canonical_bytes.to_vec(),
            sha256.to_owned(),
        );
        if let Some(existing) = records.get(identity) {
            if existing != &proposed {
                return Err(
                    crate::durable_delivery::DurableDeliveryError::ImmutableAppendConflict(
                        identity.to_owned(),
                    ),
                );
            }
        } else {
            records.insert(identity.to_owned(), proposed);
        }
        Ok(format!("immutable://{record_kind}/{identity}"))
    }
}

struct ChannelSink {
    identity: String,
    result: AuthoritativeSinkResult,
    calls: AtomicUsize,
    templates: Mutex<Vec<String>>,
}

impl ChannelSink {
    fn new(identity: &str, result: AuthoritativeSinkResult) -> Arc<Self> {
        Arc::new(Self {
            identity: identity.to_owned(),
            result,
            calls: AtomicUsize::new(0),
            templates: Mutex::new(Vec::new()),
        })
    }
}

impl AuthoritativeSinkPort for ChannelSink {
    fn sink_identity(&self) -> &str {
        &self.identity
    }

    fn deliver(&self, request: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.templates
            .lock()
            .expect("template observations")
            .push(request.stable_template_id.clone());
        self.result.clone()
    }
}

fn micros(value: i64) -> UtcMicros {
    UtcMicros::try_new(value).expect("valid W12 timestamp")
}

fn claimed_snapshot() -> (super::terminal_authority_tests::Fixture, IntentSnapshot) {
    let fixture = fixture();
    let mut store = BusinessIntentStore::open(&fixture.database).expect("open business intent");
    let owner = LeaseOwnerId::try_new("w12-dispatcher".to_owned()).expect("lease owner");
    let command = IntentTransitionCommand::try_new(
        fixture.record.intent_id.clone(),
        IntentState::PendingDispatch,
        IntentState::AwaitingAuthority,
        fixture.snapshot.version(),
        TransitionActor::try_new("w12-dispatcher".to_owned()).expect("actor"),
        ReasonCode::IntentDispatchClaimed,
        micros(1_788_743_101_000_000),
        LeaseAction::Acquire {
            owner,
            until: micros(1_788_743_400_000_000),
        },
    )
    .expect("dispatch claim");
    store
        .apply_nonterminal_transition(&command)
        .expect("persist dispatch claim");
    let claimed = store
        .inspect(&fixture.record.intent_id)
        .expect("inspect claimed intent")
        .expect("claimed intent exists");
    (fixture, claimed)
}

fn route(template: super::TerminalTemplateBinding) -> GenericTransportRoute {
    GenericTransportRoute::try_new(
        PushKind::HoldingEvent,
        DeliverySubKind::None,
        "GLOBAL".to_owned(),
        ChannelId::try_new("TEST_CODE_W12_CHANNEL".to_owned()).expect("channel"),
        template,
    )
    .expect("valid W12 route")
}

fn fence(snapshot: &IntentSnapshot) -> GenericDispatchFence {
    GenericDispatchFence::try_new(
        LeaseOwnerId::try_new(snapshot.lease_owner().expect("lease owner").to_owned())
            .expect("lease owner type"),
        snapshot.lease_generation(),
        snapshot.lease_until().expect("lease until"),
    )
    .expect("valid dispatch fence")
}

fn receipt(channel: &str) -> TypedReceipt {
    TypedReceipt {
        channel: channel.to_owned(),
        provider: "TEST_CODE_W12_PROVIDER".to_owned(),
        message_id: "TEST_CODE_W12_MESSAGE".to_owned(),
        platform_message_id: Some("TEST_CODE_W12_PLATFORM_MESSAGE".to_owned()),
        accepted_at: Utc
            .with_ymd_and_hms(2026, 9, 7, 8, 0, 0)
            .single()
            .expect("accepted at"),
        latency_ms: Some(12),
    }
}

fn request<'a>(
    snapshot: &'a IntentSnapshot,
    route: &'a GenericTransportRoute,
    fence: &'a GenericDispatchFence,
    policy: &'a crate::monitor::push_job::CompletionPolicy,
    sink: AuthoritativeSink,
    append: &'a MemoryAppend,
) -> GenericDispatchRequest<'a> {
    GenericDispatchRequest::new(
        snapshot,
        route,
        fence,
        policy,
        sink,
        append,
        1,
        micros(1_788_743_102_000_000),
        micros(1_788_743_103_000_000),
    )
}

#[test]
fn w12_generic_transport_dispatches_once_and_requeries_exact_w09_terminal() {
    let (business, claimed) = claimed_snapshot();
    let durable = DurableFixture::new("ACCEPTED");
    let route = route(business.template.clone());
    let fence = fence(&claimed);
    let append = MemoryAppend::default();
    let sink = ChannelSink::new(
        "TEST_CODE_W12_CHANNEL",
        AuthoritativeSinkResult::Accepted(receipt("TEST_CODE_W12_CHANNEL")),
    );
    let adapter = GenericTransportAuthorityAdapter::new(durable.coordinator());

    let first = adapter
        .dispatch(request(
            &claimed,
            &route,
            &fence,
            &business.policy,
            sink.clone(),
            &append,
        ))
        .expect("first W12 dispatch");
    let first_terminal = match first.view() {
        DeliveryResultView::TransportAccepted(terminal) => terminal,
        other => panic!("expected accepted W12 result, got {other:?}"),
    };
    let first_binding = first_terminal.binding_sha256().clone();
    let first_evidence = first_terminal.evidence_sha256().clone();
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        sink.templates.lock().expect("templates").as_slice(),
        &["auction-card".to_owned()]
    );

    let replay = adapter
        .dispatch(request(
            &claimed,
            &route,
            &fence,
            &business.policy,
            sink.clone(),
            &append,
        ))
        .expect("idempotent W12 replay");
    let replay_terminal = match replay.view() {
        DeliveryResultView::TransportAccepted(terminal) => terminal,
        other => panic!("expected replayed accepted W12 result, got {other:?}"),
    };
    assert_eq!(replay_terminal.binding_sha256(), &first_binding);
    assert_eq!(replay_terminal.evidence_sha256(), &first_evidence);
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn w12_generic_transport_rejects_wrong_descriptor_before_sink_or_reservation() {
    let (business, claimed) = claimed_snapshot();
    let durable = DurableFixture::new("DESCRIPTOR_MISMATCH");
    let route = route(business.template.clone());
    let fence = fence(&claimed);
    let append = MemoryAppend::default();
    let sink = ChannelSink::new(
        "TEST_CODE_WRONG_CHANNEL",
        AuthoritativeSinkResult::Accepted(receipt("TEST_CODE_W12_CHANNEL")),
    );
    let adapter = GenericTransportAuthorityAdapter::new(durable.coordinator());

    assert_eq!(
        adapter.dispatch(request(
            &claimed,
            &route,
            &fence,
            &business.policy,
            sink.clone(),
            &append,
        )),
        Err(GenericTransportError::SinkDescriptorMismatch)
    );
    assert_eq!(sink.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn w12_generic_transport_records_wrong_receipt_channel_as_uncertain() {
    let (business, claimed) = claimed_snapshot();
    let durable = DurableFixture::new("RECEIPT_MISMATCH");
    let route = route(business.template.clone());
    let fence = fence(&claimed);
    let append = MemoryAppend::default();
    let sink = ChannelSink::new(
        "TEST_CODE_W12_CHANNEL",
        AuthoritativeSinkResult::Accepted(receipt("TEST_CODE_WRONG_CHANNEL")),
    );
    let adapter = GenericTransportAuthorityAdapter::new(durable.coordinator());

    let result = adapter
        .dispatch(request(
            &claimed,
            &route,
            &fence,
            &business.policy,
            sink.clone(),
            &append,
        ))
        .expect("wrong channel is a durable uncertainty");
    let terminal = match result.view() {
        DeliveryResultView::TransportUncertain(terminal) => terminal,
        other => panic!("expected uncertain W12 result, got {other:?}"),
    };
    assert_eq!(
        terminal.terminal_disposition(),
        TerminalDisposition::Uncertain
    );
    assert_eq!(sink.calls.load(Ordering::SeqCst), 1);
    assert!(result.requires_manual_quarantine());
}

#[test]
fn w12_generic_transport_rejects_stale_business_lease_before_sink() {
    let (business, claimed) = claimed_snapshot();
    let durable = DurableFixture::new("STALE_FENCE");
    let route = route(business.template.clone());
    let stale = GenericDispatchFence::try_new(
        LeaseOwnerId::try_new("w12-dispatcher".to_owned()).expect("owner"),
        claimed.lease_generation() + 1,
        claimed.lease_until().expect("lease until"),
    )
    .expect("well-formed stale fence");
    let append = MemoryAppend::default();
    let sink = ChannelSink::new(
        "TEST_CODE_W12_CHANNEL",
        AuthoritativeSinkResult::Accepted(receipt("TEST_CODE_W12_CHANNEL")),
    );
    let adapter = GenericTransportAuthorityAdapter::new(durable.coordinator());

    assert_eq!(
        adapter.dispatch(request(
            &claimed,
            &route,
            &stale,
            &business.policy,
            sink.clone(),
            &append,
        )),
        Err(GenericTransportError::BusinessLeaseMismatch)
    );
    assert_eq!(sink.calls.load(Ordering::SeqCst), 0);
}

fn strong_result(disposition: TerminalDisposition) -> DeliveryResult {
    let fixture = fixture();
    let mut record = fixture.record;
    record.terminal_disposition = disposition;
    record.binding_sha256 = terminal_binding_sha256(&record);
    let authority = FakeAuthority::terminal(record);
    verify_terminal(
        &fixture.snapshot,
        &fixture.template,
        &fixture.policy,
        &authority,
        micros(1_788_743_103_000_000),
    )
    .expect("verified strong result")
    .into_delivery_result()
}

fn compat_accepted_result() -> DeliveryResult {
    let fixture = fixture();
    let channel = ChannelId::try_new("TEST_CODE_COMPAT_CHANNEL".to_owned()).expect("channel");
    let evidence = CompatibilityEvidenceRef::try_new(
        CompatId::try_new("TEST_CODE_COMPAT".to_owned()).expect("compat id"),
        fixture.record.intent_id,
        fixture.record.unit_id,
        fixture.record.occurrence,
        vec![channel.clone()],
        vec![channel.clone()],
        vec![WeakOutcome::new(
            channel,
            WeakOutcomeKind::Accepted,
            crate::monitor::push_job::raw_digest(b"TEST_CODE_COMPAT_EVIDENCE"),
        )],
        crate::monitor::push_job::raw_digest(b"TEST_CODE_COMPAT_EVIDENCE"),
        micros(1_788_743_103_000_000),
    )
    .expect("compat evidence");
    DeliveryResult::best_effort_accepted(evidence).expect("compat accepted")
}

fn channel(value: &str) -> ChannelId {
    ChannelId::try_new(value.to_owned()).expect("required channel")
}

#[test]
fn w12_required_channel_results_are_ordered_exact_and_all_accepted_only() {
    let required = vec![channel("TEST_CODE_SMS"), channel("TEST_CODE_WECHAT")];
    let results = RequiredChannelResults::try_classify(
        required.clone(),
        vec![
            RequiredChannelObservation::new(
                channel("TEST_CODE_WECHAT"),
                strong_result(TerminalDisposition::ManualConfirmedAccepted),
            ),
            RequiredChannelObservation::new(
                channel("TEST_CODE_SMS"),
                strong_result(TerminalDisposition::Accepted),
            ),
        ],
    )
    .expect("all required channels accepted");

    assert_eq!(
        results.classification(),
        RequiredChannelClassification::AllRequiredAccepted
    );
    assert_eq!(results.ordered_channels(), required.as_slice());
    assert_eq!(
        results.completion_eligibility(),
        CompletionEligibility::PolicyBound
    );
}

#[test]
fn w12_required_channel_results_reject_missing_duplicate_extra_and_compat() {
    let required = vec![channel("TEST_CODE_SMS"), channel("TEST_CODE_WECHAT")];
    assert_eq!(
        RequiredChannelResults::try_classify(Vec::new(), Vec::new()),
        Err(RequiredChannelError::RequiredChannelsEmpty)
    );
    assert_eq!(
        RequiredChannelResults::try_classify(
            vec![channel("TEST_CODE_SMS"), channel("TEST_CODE_SMS")],
            Vec::new(),
        ),
        Err(RequiredChannelError::DuplicateRequiredChannel)
    );
    assert_eq!(
        RequiredChannelResults::try_classify(
            required.clone(),
            vec![RequiredChannelObservation::new(
                channel("TEST_CODE_SMS"),
                strong_result(TerminalDisposition::Accepted),
            )],
        ),
        Err(RequiredChannelError::ChannelSetMismatch)
    );
    assert_eq!(
        RequiredChannelResults::try_classify(
            required.clone(),
            vec![
                RequiredChannelObservation::new(
                    channel("TEST_CODE_SMS"),
                    strong_result(TerminalDisposition::Accepted),
                ),
                RequiredChannelObservation::new(
                    channel("TEST_CODE_SMS"),
                    strong_result(TerminalDisposition::Accepted),
                ),
            ],
        ),
        Err(RequiredChannelError::DuplicateObservation)
    );
    assert_eq!(
        RequiredChannelResults::try_classify(
            required.clone(),
            vec![
                RequiredChannelObservation::new(
                    channel("TEST_CODE_SMS"),
                    strong_result(TerminalDisposition::Accepted),
                ),
                RequiredChannelObservation::new(
                    channel("TEST_CODE_EXTRA"),
                    strong_result(TerminalDisposition::Accepted),
                ),
            ],
        ),
        Err(RequiredChannelError::ChannelSetMismatch)
    );
    assert_eq!(
        RequiredChannelResults::try_classify(
            vec![channel("TEST_CODE_COMPAT_CHANNEL")],
            vec![RequiredChannelObservation::new(
                channel("TEST_CODE_COMPAT_CHANNEL"),
                compat_accepted_result(),
            )],
        ),
        Err(RequiredChannelError::StrongAuthorityRequired)
    );
}

#[test]
fn w12_required_channel_partial_rejected_and_uncertain_never_complete() {
    let required = vec![channel("TEST_CODE_SMS"), channel("TEST_CODE_WECHAT")];
    let partial = RequiredChannelResults::try_classify(
        required.clone(),
        vec![
            RequiredChannelObservation::new(
                channel("TEST_CODE_SMS"),
                strong_result(TerminalDisposition::Accepted),
            ),
            RequiredChannelObservation::new(
                channel("TEST_CODE_WECHAT"),
                strong_result(TerminalDisposition::Rejected),
            ),
        ],
    )
    .expect("strong partial results");
    assert_eq!(
        partial.classification(),
        RequiredChannelClassification::PartialRequiredChannels
    );
    assert_eq!(
        partial.completion_eligibility(),
        CompletionEligibility::Never
    );

    let rejected = RequiredChannelResults::try_classify(
        required.clone(),
        vec![
            RequiredChannelObservation::new(
                channel("TEST_CODE_SMS"),
                strong_result(TerminalDisposition::Rejected),
            ),
            RequiredChannelObservation::new(
                channel("TEST_CODE_WECHAT"),
                strong_result(TerminalDisposition::ManualConfirmedNotDelivered),
            ),
        ],
    )
    .expect("strong rejected results");
    assert_eq!(
        rejected.classification(),
        RequiredChannelClassification::RejectedRequiredChannels
    );
    assert_eq!(
        rejected.completion_eligibility(),
        CompletionEligibility::Never
    );

    let uncertain = RequiredChannelResults::try_classify(
        required,
        vec![
            RequiredChannelObservation::new(
                channel("TEST_CODE_SMS"),
                strong_result(TerminalDisposition::Accepted),
            ),
            RequiredChannelObservation::new(
                channel("TEST_CODE_WECHAT"),
                strong_result(TerminalDisposition::Uncertain),
            ),
        ],
    )
    .expect("strong uncertain results");
    assert_eq!(
        uncertain.classification(),
        RequiredChannelClassification::UncertainRequiredChannels
    );
    assert_eq!(
        uncertain.completion_eligibility(),
        CompletionEligibility::Never
    );
}
