//! Process-local recovery state for the four registered raw GlobalNews feeds.
//!
//! This is not a platform-wide source registry. No state survives a monitor
//! restart, and a skipped pull cannot create gateway evidence or a receipt.

use super::{RegisteredGlobalNewsFeed, REGISTERED_PROVIDERS};
use crate::data_gateway::GlobalNewsProvider;
use chrono::{DateTime, Duration, Utc};
use std::sync::Mutex;
use thiserror::Error;

pub const GLOBAL_NEWS_BREAKER_FAILURE_THRESHOLD: u32 = 10;
pub const GLOBAL_NEWS_BREAKER_COOLDOWN_SECONDS: i64 = 60;
pub const GLOBAL_NEWS_BREAKER_COVERAGE: &str = "four_registered_raw_global_news_feeds_only";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceBreakerState {
    Closed,
    Open,
    HalfOpen,
}

/// A read-only operational view. Times refer to local completed raw gateway
/// acquisition with admitted batch evidence, not BR-244 projection success or
/// substituted upstream publication/batch observation time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRecoverySnapshot {
    pub coverage: &'static str,
    pub registration: RegisteredGlobalNewsFeed,
    pub registry_started_at: DateTime<Utc>,
    pub warming: bool,
    pub state: SourceBreakerState,
    pub consecutive_retryable_failures: u32,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub last_successful_pull_at: Option<DateTime<Utc>>,
    pub outage_started_at: Option<DateTime<Utc>>,
    pub last_failure_at: Option<DateTime<Utc>>,
    pub opened_at: Option<DateTime<Utc>>,
    pub next_probe_at: Option<DateTime<Utc>>,
    pub last_reason_code: Option<&'static str>,
    pub last_retryable: Option<bool>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SourceRegistryError {
    #[error("raw GlobalNews breaker state unavailable for {0:?}")]
    StateUnavailable(GlobalNewsProvider),
}

#[derive(Debug)]
struct SlotState {
    generation: u64,
    state: SourceBreakerState,
    consecutive_retryable_failures: u32,
    last_attempt_at: Option<DateTime<Utc>>,
    last_successful_pull_at: Option<DateTime<Utc>>,
    outage_started_at: Option<DateTime<Utc>>,
    last_failure_at: Option<DateTime<Utc>>,
    opened_at: Option<DateTime<Utc>>,
    next_probe_at: Option<DateTime<Utc>>,
    last_reason_code: Option<&'static str>,
    last_retryable: Option<bool>,
}

impl Default for SlotState {
    fn default() -> Self {
        Self {
            generation: 0,
            state: SourceBreakerState::Closed,
            consecutive_retryable_failures: 0,
            last_attempt_at: None,
            last_successful_pull_at: None,
            outage_started_at: None,
            last_failure_at: None,
            opened_at: None,
            next_probe_at: None,
            last_reason_code: None,
            last_retryable: None,
        }
    }
}

impl SlotState {
    fn open_at(&mut self, at: DateTime<Utc>) {
        self.generation = self.generation.wrapping_add(1);
        self.state = SourceBreakerState::Open;
        self.opened_at = Some(at);
        self.next_probe_at = Some(at + Duration::seconds(GLOBAL_NEWS_BREAKER_COOLDOWN_SECONDS));
    }

    fn snapshot(
        &self,
        provider: GlobalNewsProvider,
        registry_started_at: DateTime<Utc>,
    ) -> SourceRecoverySnapshot {
        SourceRecoverySnapshot {
            coverage: GLOBAL_NEWS_BREAKER_COVERAGE,
            registration: RegisteredGlobalNewsFeed::for_provider(provider),
            registry_started_at,
            warming: self.last_attempt_at.is_none(),
            state: self.state,
            consecutive_retryable_failures: self.consecutive_retryable_failures,
            last_attempt_at: self.last_attempt_at,
            last_successful_pull_at: self.last_successful_pull_at,
            outage_started_at: self.outage_started_at,
            last_failure_at: self.last_failure_at,
            opened_at: self.opened_at,
            next_probe_at: self.next_probe_at,
            last_reason_code: self.last_reason_code,
            last_retryable: self.last_retryable,
        }
    }
}

/// One registry must be retained by the monitor owner across news ticks.
/// Each source has its own short lock; no lock is held across gateway I/O.
#[derive(Debug)]
pub struct GlobalNewsSourceRegistry {
    started_at: DateTime<Utc>,
    slots: [Mutex<SlotState>; 4],
}

impl Default for GlobalNewsSourceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl GlobalNewsSourceRegistry {
    pub fn new() -> Self {
        Self::new_at(Utc::now())
    }

    pub(super) fn new_at(started_at: DateTime<Utc>) -> Self {
        Self {
            started_at,
            slots: std::array::from_fn(|_| Mutex::new(SlotState::default())),
        }
    }

    /// A filesystem/network-free snapshot for the monitor runtime. A poisoned
    /// slot is reported as unavailable instead of claiming a healthy source.
    pub fn snapshot(&self) -> Result<Vec<SourceRecoverySnapshot>, SourceRegistryError> {
        REGISTERED_PROVIDERS
            .iter()
            .copied()
            .map(|provider| {
                self.slots[slot_index(provider)]
                    .lock()
                    .map(|slot| slot.snapshot(provider, self.started_at))
                    .map_err(|_| SourceRegistryError::StateUnavailable(provider))
            })
            .collect()
    }

    pub(super) fn begin<'a>(
        &'a self,
        provider: GlobalNewsProvider,
        now: DateTime<Utc>,
        clock: &'a (dyn Fn() -> DateTime<Utc> + Sync),
    ) -> SourceAcquire<'a> {
        let Ok(mut slot) = self.slots[slot_index(provider)].lock() else {
            return SourceAcquire::Skipped(SourceSkipReason::StateUnavailable);
        };
        let half_open = match slot.state {
            SourceBreakerState::Closed => false,
            SourceBreakerState::Open => {
                if slot.next_probe_at.is_none_or(|next| now < next) {
                    return SourceAcquire::Skipped(SourceSkipReason::CircuitOpen);
                }
                slot.state = SourceBreakerState::HalfOpen;
                slot.generation = slot.generation.wrapping_add(1);
                slot.next_probe_at = None;
                true
            }
            SourceBreakerState::HalfOpen => {
                return SourceAcquire::Skipped(SourceSkipReason::CircuitOpen);
            }
        };
        slot.last_attempt_at = Some(now);
        SourceAcquire::Call(SourceCallPermit {
            registry: self,
            provider,
            clock,
            half_open,
            generation: slot.generation,
            completed: false,
        })
    }

    fn finish(
        &self,
        provider: GlobalNewsProvider,
        half_open: bool,
        generation: u64,
        terminal: SourceTerminal,
        at: DateTime<Utc>,
    ) {
        let Ok(mut slot) = self.slots[slot_index(provider)].lock() else {
            return;
        };
        // A Closed request can finish after a later outage opened the source
        // and even after its HalfOpen probe began. Its old result must not
        // release or overwrite the current probe lease.
        if slot.generation != generation {
            return;
        }
        match terminal {
            SourceTerminal::Verified => {
                slot.state = SourceBreakerState::Closed;
                slot.consecutive_retryable_failures = 0;
                slot.last_successful_pull_at = Some(at);
                slot.outage_started_at = None;
                slot.opened_at = None;
                slot.next_probe_at = None;
            }
            SourceTerminal::Unavailable {
                reason_code,
                retryable,
            } => {
                slot.outage_started_at.get_or_insert(at);
                slot.last_failure_at = Some(at);
                slot.last_reason_code = Some(reason_code);
                slot.last_retryable = Some(retryable);
                if retryable {
                    if half_open {
                        slot.open_at(at);
                    } else {
                        slot.consecutive_retryable_failures =
                            slot.consecutive_retryable_failures.saturating_add(1);
                        if slot.consecutive_retryable_failures
                            >= GLOBAL_NEWS_BREAKER_FAILURE_THRESHOLD
                        {
                            slot.open_at(at);
                        }
                    }
                } else {
                    // A nonretryable evidence/audit failure breaks the retryable
                    // streak while retaining its original typed failure.
                    slot.consecutive_retryable_failures = 0;
                    slot.state = SourceBreakerState::Closed;
                    slot.opened_at = None;
                    slot.next_probe_at = None;
                }
            }
        }
    }

    fn abandon_half_open(&self, provider: GlobalNewsProvider, generation: u64, at: DateTime<Utc>) {
        if let Ok(mut slot) = self.slots[slot_index(provider)].lock() {
            if slot.state == SourceBreakerState::HalfOpen && slot.generation == generation {
                // Cancellation is not a provider failure. Reserve another
                // probe only after a fresh cooldown, avoiding a stuck slot.
                slot.open_at(at);
            }
        }
    }
}

fn slot_index(provider: GlobalNewsProvider) -> usize {
    match provider {
        GlobalNewsProvider::Eastmoney => 0,
        GlobalNewsProvider::Cailianpress => 1,
        GlobalNewsProvider::Jin10 => 2,
        GlobalNewsProvider::ThePaper => 3,
    }
}

pub(super) enum SourceSkipReason {
    CircuitOpen,
    StateUnavailable,
}

pub(super) enum SourceAcquire<'a> {
    Call(SourceCallPermit<'a>),
    Skipped(SourceSkipReason),
}

pub(super) enum SourceTerminal {
    Verified,
    Unavailable {
        reason_code: &'static str,
        retryable: bool,
    },
}

pub(super) struct SourceCallPermit<'a> {
    registry: &'a GlobalNewsSourceRegistry,
    provider: GlobalNewsProvider,
    clock: &'a (dyn Fn() -> DateTime<Utc> + Sync),
    half_open: bool,
    generation: u64,
    completed: bool,
}

impl SourceCallPermit<'_> {
    pub(super) fn finish(mut self, terminal: SourceTerminal) {
        self.registry.finish(
            self.provider,
            self.half_open,
            self.generation,
            terminal,
            (self.clock)(),
        );
        self.completed = true;
    }
}

impl Drop for SourceCallPermit<'_> {
    fn drop(&mut self) {
        if !self.completed && self.half_open {
            self.registry
                .abandon_half_open(self.provider, self.generation, (self.clock)());
        }
    }
}
