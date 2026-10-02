use super::*;
use chrono::TimeZone;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

fn index(provider: GlobalNewsProvider) -> usize {
    match provider {
        GlobalNewsProvider::Eastmoney => 0,
        GlobalNewsProvider::Cailianpress => 1,
        GlobalNewsProvider::Jin10 => 2,
        GlobalNewsProvider::ThePaper => 3,
    }
}

struct TestClock(AtomicI64);

impl TestClock {
    fn new() -> Self {
        Self(AtomicI64::new(1_780_000_000))
    }

    fn now(&self) -> DateTime<Utc> {
        Utc.timestamp_opt(self.0.load(Ordering::SeqCst), 0)
            .single()
            .expect("TEST_CODE clock")
    }

    fn advance(&self, seconds: i64) {
        self.0.fetch_add(seconds, Ordering::SeqCst);
    }
}

fn evidence(provider: GlobalNewsProvider) -> BatchEvidence {
    BatchEvidence {
        provider: provider.provider_id(),
        source: provider.source().to_owned(),
        source_at: Some(match provider {
            GlobalNewsProvider::Eastmoney => "2026-07-28 09:00".to_owned(),
            _ => "2026-07-28T01:00:00Z".to_owned(),
        }),
        observed_at: "2026-07-28T01:00:01Z".to_owned(),
        batch_id: format!("TEST_CODE_{}_breaker", provider.feed_name()),
    }
}

fn record(provider: GlobalNewsProvider, evidence: &BatchEvidence) -> GlobalNewsRecord {
    let published_at = DateTime::parse_from_rfc3339("2026-07-28T01:00:00Z")
        .expect("TEST_CODE publication")
        .with_timezone(&Utc);
    let observed_at = DateTime::parse_from_rfc3339("2026-07-28T01:00:01Z")
        .expect("TEST_CODE observation")
        .with_timezone(&Utc);
    GlobalNewsRecord {
        item_id: "TEST_CODE_item".to_owned(),
        title: "TEST_CODE title".to_owned(),
        summary: None,
        content: None,
        publisher: "TEST_CODE publisher".to_owned(),
        canonical_url: "https://example.com/TEST_CODE_item".to_owned(),
        published_at,
        observed_at,
        instruments: vec![],
        topics: vec![],
        language: "zh-CN".to_owned(),
        evidence: crate::market_domain::SourceEvidence::new(
            provider.provider_id(),
            evidence.observed_at.clone(),
            evidence.batch_id.clone(),
        )
        .expect("TEST_CODE record evidence")
        .with_source_at(evidence.source_at.as_deref().expect("source time"))
        .expect("TEST_CODE record source time"),
    }
}

#[derive(Clone, Copy)]
enum Scripted {
    Retryable,
    ExternalTransport,
    Nonretryable,
    Available,
    VerifiedEmpty,
    InvalidEmptyEvidence,
}

struct ScriptedPort {
    calls: [AtomicUsize; 4],
    scripts: Mutex<[VecDeque<Scripted>; 4]>,
}

impl ScriptedPort {
    fn new() -> Self {
        Self {
            calls: std::array::from_fn(|_| AtomicUsize::new(0)),
            scripts: Mutex::new(std::array::from_fn(|_| VecDeque::new())),
        }
    }

    fn push(&self, provider: GlobalNewsProvider, result: Scripted) {
        self.scripts.lock().unwrap()[index(provider)].push_back(result);
    }

    fn calls(&self, provider: GlobalNewsProvider) -> usize {
        self.calls[index(provider)].load(Ordering::SeqCst)
    }
}

#[async_trait]
impl RawGlobalNewsPort for ScriptedPort {
    async fn fetch(
        &self,
        provider: GlobalNewsProvider,
        _limit: u32,
    ) -> Result<GatewayBatch<GlobalNewsRecord>, GatewayError> {
        self.calls[index(provider)].fetch_add(1, Ordering::SeqCst);
        let scripted = self.scripts.lock().unwrap()[index(provider)]
            .pop_front()
            .unwrap_or(Scripted::VerifiedEmpty);
        match scripted {
            Scripted::ExternalTransport => Err(
                crate::data_gateway::grpc_source::map_external_connection_error(
                    crate::grpc_client::errors::GrpcError::Unavailable {
                        details: Box::default(),
                    },
                ),
            ),
            Scripted::Retryable => Err(GatewayError::unavailable(
                provider.capability(),
                Some(provider.provider_id()),
                true,
                "TEST_CODE retryable failure",
            )),
            Scripted::Nonretryable => Err(GatewayError::retired_operation(
                provider.capability(),
                Some(provider.provider_id()),
            )),
            Scripted::Available => {
                let evidence = evidence(provider);
                Ok(GatewayBatch::Available {
                    records: vec![record(provider, &evidence)],
                    evidence,
                })
            }
            Scripted::VerifiedEmpty => Ok(GatewayBatch::VerifiedEmpty(evidence(provider))),
            Scripted::InvalidEmptyEvidence => {
                let mut evidence = evidence(provider);
                evidence.source_at = None;
                Ok(GatewayBatch::VerifiedEmpty(evidence))
            }
        }
    }
}

#[tokio::test]
async fn m3_runtime_health_actual_external_transport_reason_retains_registry_outage() {
    let clock = TestClock::new();
    let registry = GlobalNewsSourceRegistry::new_at(clock.now());
    let port = ScriptedPort::new();
    for _ in 0..10 {
        port.push(GlobalNewsProvider::Eastmoney, Scripted::ExternalTransport);
        tick(&port, &registry, &clock).await;
    }
    let source = east(&registry);
    assert_eq!(source.state, SourceBreakerState::Open);
    assert_eq!(source.consecutive_retryable_failures, 10);
    assert_eq!(
        source.last_reason_code,
        Some("external_transport_unavailable")
    );
    assert_eq!(source.last_retryable, Some(true));
    assert_eq!(source.outage_started_at, Some(clock.now()));
    assert_eq!(
        source.next_probe_at,
        Some(clock.now() + chrono::Duration::seconds(60))
    );
    assert!(
        crate::data_gateway::grpc_source::is_known_global_news_recovery_reason(
            source.last_reason_code.unwrap()
        )
    );
    assert!(
        !crate::data_gateway::grpc_source::is_known_global_news_recovery_reason(
            "secret_token_test_code"
        )
    );
}

async fn tick(
    port: &impl RawGlobalNewsPort,
    registry: &GlobalNewsSourceRegistry,
    clock: &TestClock,
) -> RawNewsAggregationBatch {
    let now = || clock.now();
    fetch_raw_global_news_batch_with_clock(port, registry, 20, &now)
        .await
        .expect("TEST_CODE typed four-source tick")
}

fn east(registry: &GlobalNewsSourceRegistry) -> SourceRecoverySnapshot {
    registry.snapshot().expect("TEST_CODE snapshot")[0].clone()
}

#[tokio::test]
async fn tenth_retryable_failure_opens_only_one_source_and_skips_without_evidence() {
    let clock = TestClock::new();
    let registry = GlobalNewsSourceRegistry::new_at(clock.now());
    let port = ScriptedPort::new();
    assert!(east(&registry).warming);
    for _ in 0..10 {
        port.push(GlobalNewsProvider::Eastmoney, Scripted::Retryable);
    }

    for expected in 1..=9 {
        tick(&port, &registry, &clock).await;
        let source = east(&registry);
        assert_eq!(source.state, SourceBreakerState::Closed);
        assert_eq!(source.consecutive_retryable_failures, expected);
        clock.advance(1);
    }
    tick(&port, &registry, &clock).await;
    let opened = east(&registry);
    assert_eq!(opened.coverage, GLOBAL_NEWS_BREAKER_COVERAGE);
    assert!(!opened.warming);
    assert_eq!(opened.state, SourceBreakerState::Open);
    assert_eq!(opened.consecutive_retryable_failures, 10);
    assert_eq!(
        opened.next_probe_at,
        Some(clock.now() + chrono::Duration::seconds(60))
    );
    assert_eq!(opened.outage_started_at, Some(opened.registry_started_at));

    clock.advance(1);
    let skipped = tick(&port, &registry, &clock).await;
    assert_eq!(port.calls(GlobalNewsProvider::Eastmoney), 10);
    assert_eq!(skipped.attempts().len(), 4);
    assert!(!skipped.sources_complete());
    assert_eq!(skipped.source_record_count(), 0);
    let unavailable = skipped.attempts()[0]
        .terminal()
        .unavailable()
        .expect("Open source remains unavailable");
    assert_eq!(unavailable.reason_code(), "circuit_open");
    assert_eq!(unavailable.available_evidence(), None);
    let projection = project_news_flash_events(&skipped);
    assert_eq!(projection.available_feed_count(), 0);
    assert_eq!(projection.verified_empty_feed_count(), 3);
    assert_eq!(projection.failures().len(), 1);
    for provider in &REGISTERED_PROVIDERS[1..] {
        assert_eq!(port.calls(*provider), 11, "other sources must continue");
    }
    assert_eq!(east(&registry).last_attempt_at, opened.last_attempt_at);
}

#[tokio::test]
async fn verified_empty_and_available_reset_streak_and_recovery_times_are_local() {
    let clock = TestClock::new();
    let registry = GlobalNewsSourceRegistry::new_at(clock.now());
    let port = ScriptedPort::new();
    for _ in 0..9 {
        port.push(GlobalNewsProvider::Eastmoney, Scripted::Retryable);
        tick(&port, &registry, &clock).await;
        clock.advance(1);
    }
    port.push(GlobalNewsProvider::Eastmoney, Scripted::Available);
    let available = tick(&port, &registry, &clock).await;
    assert_eq!(
        available.attempts()[0].terminal().kind(),
        RawGlobalNewsTerminalKind::Available
    );
    assert_eq!(east(&registry).consecutive_retryable_failures, 0);
    let first_success = clock.now();
    assert_eq!(east(&registry).last_successful_pull_at, Some(first_success));
    assert_eq!(east(&registry).outage_started_at, None);

    clock.advance(2);
    port.push(GlobalNewsProvider::Eastmoney, Scripted::Retryable);
    tick(&port, &registry, &clock).await;
    assert_eq!(east(&registry).consecutive_retryable_failures, 1);
    assert_eq!(east(&registry).last_successful_pull_at, Some(first_success));
    assert_eq!(east(&registry).outage_started_at, Some(clock.now()));

    clock.advance(2);
    port.push(GlobalNewsProvider::Eastmoney, Scripted::VerifiedEmpty);
    let empty = tick(&port, &registry, &clock).await;
    assert_eq!(
        empty.attempts()[0].terminal().kind(),
        RawGlobalNewsTerminalKind::VerifiedEmpty
    );
    assert_eq!(east(&registry).state, SourceBreakerState::Closed);
    assert_eq!(east(&registry).consecutive_retryable_failures, 0);
    assert_eq!(east(&registry).last_successful_pull_at, Some(clock.now()));
    assert_eq!(east(&registry).outage_started_at, None);
}

#[tokio::test]
async fn bad_evidence_and_nonretryable_error_keep_original_reason_and_break_retryable_streak() {
    let clock = TestClock::new();
    let registry = GlobalNewsSourceRegistry::new_at(clock.now());
    let port = ScriptedPort::new();
    for _ in 0..9 {
        port.push(GlobalNewsProvider::Eastmoney, Scripted::Retryable);
        tick(&port, &registry, &clock).await;
    }
    port.push(
        GlobalNewsProvider::Eastmoney,
        Scripted::InvalidEmptyEvidence,
    );
    let invalid = tick(&port, &registry, &clock).await;
    let unavailable = invalid.attempts()[0].terminal().unavailable().unwrap();
    assert_eq!(unavailable.reason_code(), "invalid_evidence");
    assert!(!unavailable.retryable());
    assert_eq!(east(&registry).consecutive_retryable_failures, 0);
    assert_eq!(east(&registry).last_successful_pull_at, None);
    assert_eq!(east(&registry).last_reason_code, Some("invalid_evidence"));
    port.push(GlobalNewsProvider::Eastmoney, Scripted::Nonretryable);
    let direct = tick(&port, &registry, &clock).await;
    assert_eq!(
        direct.attempts()[0]
            .terminal()
            .unavailable()
            .unwrap()
            .reason_code(),
        "operation_retired"
    );
    port.push(GlobalNewsProvider::Eastmoney, Scripted::Retryable);
    tick(&port, &registry, &clock).await;
    assert_eq!(east(&registry).state, SourceBreakerState::Closed);
    assert_eq!(east(&registry).consecutive_retryable_failures, 1);
}

#[tokio::test]
async fn one_half_open_probe_at_a_time_and_failed_probe_starts_fresh_cooldown() {
    let clock = Arc::new(TestClock::new());
    let registry = Arc::new(GlobalNewsSourceRegistry::new_at(clock.now()));
    let port = Arc::new(BlockingProbePort::new());
    for _ in 0..10 {
        tick(port.as_ref(), registry.as_ref(), clock.as_ref()).await;
    }
    assert_eq!(east(&registry).state, SourceBreakerState::Open);
    clock.advance(60);

    let pending = {
        let port = Arc::clone(&port);
        let registry = Arc::clone(&registry);
        let clock = Arc::clone(&clock);
        tokio::spawn(async move { tick(port.as_ref(), registry.as_ref(), clock.as_ref()).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(3), port.started.notified())
        .await
        .expect("TEST_CODE half-open probe must enter the port");
    assert_eq!(east(&registry).state, SourceBreakerState::HalfOpen);
    let skipped = tick(port.as_ref(), registry.as_ref(), clock.as_ref()).await;
    assert_eq!(
        skipped.attempts()[0]
            .terminal()
            .unavailable()
            .unwrap()
            .reason_code(),
        "circuit_open"
    );
    assert_eq!(port.east_calls.load(Ordering::SeqCst), 11);
    port.release.notify_one();
    let failed_probe = tokio::time::timeout(std::time::Duration::from_secs(3), pending)
        .await
        .expect("TEST_CODE half-open probe must finish")
        .unwrap();
    assert_eq!(
        failed_probe.attempts()[0]
            .terminal()
            .unavailable()
            .unwrap()
            .reason_code(),
        "no_verified_batch"
    );
    assert_eq!(east(&registry).state, SourceBreakerState::Open);
    assert_eq!(
        east(&registry).next_probe_at,
        Some(clock.now() + chrono::Duration::seconds(60))
    );
    tick(port.as_ref(), registry.as_ref(), clock.as_ref()).await;
    assert_eq!(port.east_calls.load(Ordering::SeqCst), 11);
    clock.advance(60);
    let recovered = tick(port.as_ref(), registry.as_ref(), clock.as_ref()).await;
    assert_eq!(
        recovered.attempts()[0].terminal().kind(),
        RawGlobalNewsTerminalKind::VerifiedEmpty
    );
    assert_eq!(east(&registry).state, SourceBreakerState::Closed);
    assert_eq!(east(&registry).consecutive_retryable_failures, 0);
}

#[test]
fn late_closed_completion_cannot_release_or_overwrite_a_half_open_probe() {
    let clock = TestClock::new();
    let registry = GlobalNewsSourceRegistry::new_at(clock.now());
    let now = || clock.now();
    let provider = GlobalNewsProvider::Eastmoney;
    let old = match registry.begin(provider, clock.now(), &now) {
        SourceAcquire::Call(permit) => permit,
        SourceAcquire::Skipped(_) => panic!("TEST_CODE initial call must be allowed"),
    };
    for _ in 0..10 {
        let permit = match registry.begin(provider, clock.now(), &now) {
            SourceAcquire::Call(permit) => permit,
            SourceAcquire::Skipped(_) => panic!("TEST_CODE failure call must be allowed"),
        };
        permit.finish(SourceTerminal::Unavailable {
            reason_code: "no_verified_batch",
            retryable: true,
        });
    }
    assert_eq!(east(&registry).state, SourceBreakerState::Open);
    clock.advance(60);
    let probe = match registry.begin(provider, clock.now(), &now) {
        SourceAcquire::Call(permit) => permit,
        SourceAcquire::Skipped(_) => panic!("TEST_CODE due probe must be allowed"),
    };
    assert_eq!(east(&registry).state, SourceBreakerState::HalfOpen);

    old.finish(SourceTerminal::Verified);
    assert_eq!(east(&registry).state, SourceBreakerState::HalfOpen);
    assert_eq!(east(&registry).last_successful_pull_at, None);
    assert!(matches!(
        registry.begin(provider, clock.now(), &now),
        SourceAcquire::Skipped(SourceSkipReason::CircuitOpen)
    ));

    probe.finish(SourceTerminal::Unavailable {
        reason_code: "no_verified_batch",
        retryable: true,
    });
    assert_eq!(east(&registry).state, SourceBreakerState::Open);
    assert_eq!(
        east(&registry).next_probe_at,
        Some(clock.now() + chrono::Duration::seconds(60))
    );
}

struct BlockingProbePort {
    east_calls: AtomicUsize,
    started: Notify,
    release: Notify,
}

impl BlockingProbePort {
    fn new() -> Self {
        Self {
            east_calls: AtomicUsize::new(0),
            started: Notify::new(),
            release: Notify::new(),
        }
    }
}

#[async_trait]
impl RawGlobalNewsPort for BlockingProbePort {
    async fn fetch(
        &self,
        provider: GlobalNewsProvider,
        _limit: u32,
    ) -> Result<GatewayBatch<GlobalNewsRecord>, GatewayError> {
        if provider != GlobalNewsProvider::Eastmoney {
            return Ok(GatewayBatch::VerifiedEmpty(evidence(provider)));
        }
        match self.east_calls.fetch_add(1, Ordering::SeqCst) {
            0..=9 => Err(GatewayError::unavailable(
                provider.capability(),
                Some(provider.provider_id()),
                true,
                "TEST_CODE initial outage",
            )),
            10 => {
                self.started.notify_one();
                self.release.notified().await;
                Err(GatewayError::unavailable(
                    provider.capability(),
                    Some(provider.provider_id()),
                    true,
                    "TEST_CODE failed probe",
                ))
            }
            11 => Ok(GatewayBatch::VerifiedEmpty(evidence(provider))),
            _ => panic!("TEST_CODE unexpected extra Eastmoney probe"),
        }
    }
}
