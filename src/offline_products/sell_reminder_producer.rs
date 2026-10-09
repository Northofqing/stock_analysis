//! Independent after-hours producer, with no database initialization or sink.
//!
//! Observation files cannot mint a dispatch capability. The current real lot,
//! reservation, lifecycle and fee source contracts are not delivered. A future
//! reviewed source adapter may call the private assembly seam after validating
//! all of those independent contracts; it must not use paper lots or T-21's
//! executed-paper-sale presentation as a real-account suggestion.
use super::{preview_observed, EvidencePack, Preview, QualifiedSource, State};
use crate::offline_products::{at, escaped, hash, Clock};
use chrono::Duration;
use serde::Serialize;

pub const VERSION: &str = "afterhours-sell-producer/v1";
const MAX_STARTUP_MS: i64 = 120_000;
const ACCOUNT_FRESH_MS: i64 = 30_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ProductionState {
    NotReady,
    NoSuggestion,
    Ready,
    OutsidePreparationWindow,
    StartupBudgetExceeded,
    Expired,
}

/// This serialized receipt records observation only. It is not an inbox job,
/// counted envelope, source authority or evidence of a delivered notification.
#[derive(Serialize)]
pub struct ProductionReceipt {
    pub schema: &'static str,
    pub state: ProductionState,
    pub started_at: Clock,
    pub completed_at: Clock,
    pub elapsed_ms: i64,
    pub dispatch_not_before: Clock,
    pub expires_at: Clock,
    pub source_input_sha256: String,
    pub source_status: &'static str,
    pub missing: Vec<String>,
    pub dispatch_candidate_count: usize,
    pub delivery: &'static str,
    pub preview: Preview,
}

pub struct Production {
    receipt: ProductionReceipt,
    candidate: Option<DispatchCandidate>,
}

impl Production {
    pub fn receipt(&self) -> &ProductionReceipt {
        &self.receipt
    }
    pub fn into_candidate(self) -> Option<DispatchCandidate> {
        self.candidate
    }
    pub fn markdown(&self) -> String {
        let mut text = format!(
            "盘后卖出 producer：{:?}。启动/读取耗时 {} ms（预算 {} ms）；投递窗口 {} 至 {}。\n\n当前没有发出通知，序列化观察结果不能恢复投递资格。\n",
            self.receipt.state,
            self.receipt.elapsed_ms,
            MAX_STARTUP_MS,
            self.receipt.dispatch_not_before,
            self.receipt.expires_at,
        );
        for missing in &self.receipt.missing {
            text.push_str(&format!("\n- {}", escaped(missing)));
        }
        text.push_str(&format!("\n\n{}", self.receipt.preview.markdown()));
        text
    }
}

/// Scheduled producer accepts raw observations without upgrading their authority.
pub fn produce_observed(
    pack: &EvidencePack,
    started_at: Clock,
    completed_at: Clock,
) -> Result<Production, String> {
    assemble(
        preview_observed(pack, completed_at),
        started_at,
        completed_at,
        None,
    )
}

/// Explicit detached database observation; the preview's existing safe reader
/// never initializes a database, manufactures real lots, or reads paper inventory.
pub fn produce_database_observed(
    path: &std::path::Path,
    started_at: Clock,
    completed_at: Clock,
) -> anyhow::Result<Production> {
    let preview = super::diagnose_database(path, completed_at)?;
    assemble(preview, started_at, completed_at, None).map_err(anyhow::Error::msg)
}

/// Finish a slow read using the actual completion clock. This public operation
/// can only retain observation authority, including when the read crosses 15:30.
pub fn finish_database_observation(
    production: Production,
    completed_at: Clock,
) -> Result<Production, String> {
    if production.candidate.is_some()
        || !production.receipt.preview.authority.contains("NotAdmitted")
    {
        return Err("database timing finalization accepts observations only".into());
    }
    let mut preview = production.receipt.preview;
    preview.reinspect(completed_at)?;
    assemble(preview, production.receipt.started_at, completed_at, None)
}

fn assemble(
    preview: Preview,
    started_at: Clock,
    completed_at: Clock,
    authority: Option<&QualifiedSource>,
) -> Result<Production, String> {
    if started_at.offset().local_minus_utc() != 28800
        || completed_at.offset().local_minus_utc() != 28800
        || completed_at < started_at
    {
        return Err("producer requires monotonic Shanghai +08:00 clocks".into());
    }
    let day = started_at.date_naive();
    let preparation_at = at(day, 15, 2);
    let dispatch_not_before = at(day, 15, 3);
    let preparation_deadline = at(day, 15, 4);
    let expires_at = at(day, 15, 30);
    let elapsed_ms = completed_at
        .signed_duration_since(started_at)
        .num_milliseconds();
    let source_status = if authority.is_none() {
        "ContractNotDelivered/ObservedOnly"
    } else if !preview.missing.is_empty()
        || preview
            .rows
            .iter()
            .any(|row| row.state == State::Unavailable)
    {
        "IndependentSourceInvalid"
    } else {
        "InternalQualifiedSource"
    };
    let mut missing = if authority.is_none() {
        vec![
            "ContractNotDelivered: real account/lot ownership, T+1 sellability, reservations, independently finalized close, 15:00 lifecycle/status, source-carried board/quantity and original/current fees".into(),
            "Observed snapshots and paper lots cannot mint real-account dispatch authority".into(),
        ]
    } else {
        vec![]
    };
    let state = if completed_at >= expires_at || preview.state == State::Expired {
        ProductionState::Expired
    } else if started_at < preparation_at
        || started_at >= preparation_deadline
        || completed_at.date_naive() != day
        || crate::calendar::verified_a_share_trading_day(day) != Ok(true)
    {
        missing.push("start must be on a verified trading day at 15:02 <= start <15:04".into());
        ProductionState::OutsidePreparationWindow
    } else if elapsed_ms > MAX_STARTUP_MS || completed_at > preparation_deadline {
        missing
            .push("15:03 fast-card startup/read budget not met (<=120000ms and by 15:04)".into());
        ProductionState::StartupBudgetExceeded
    } else if authority.is_none()
        || !preview.missing.is_empty()
        || preview
            .rows
            .iter()
            .any(|row| row.state == State::Unavailable)
    {
        ProductionState::NotReady
    } else if preview.state == State::Candidate {
        ProductionState::Ready
    } else if preview.state == State::NoSuggestion {
        ProductionState::NoSuggestion
    } else {
        ProductionState::NotReady
    };
    let candidate = if state == ProductionState::Ready {
        Some(DispatchCandidate::from_qualified_preview(
            &preview,
            completed_at,
            dispatch_not_before,
            expires_at,
        )?)
    } else {
        None
    };
    let candidate_count = usize::from(candidate.is_some());
    Ok(Production {
        receipt: ProductionReceipt {
            schema: VERSION,
            state,
            started_at,
            completed_at,
            elapsed_ms,
            dispatch_not_before,
            expires_at,
            source_input_sha256: preview.input_hash.clone(),
            source_status,
            missing,
            dispatch_candidate_count: candidate_count,
            delivery:
                "NotAttempted: producer has no sink; receipt is an observation, not a durable job",
            preview,
        },
        candidate,
    })
}

/// Opaque borrowed request for the existing single counted owner. There is no
/// constructor, Deserialize or Clone. The occurrence remains stable when fresh
/// evidence changes; the source hash binds immutable content separately. The
/// owner must atomically adjudicate that identity and enforce the expiry for any
/// resumed delivery. Never register this as T-21's paper execution confirmation.
pub struct PreparedDispatch {
    business_date: chrono::NaiveDate,
    occurrence_identity: String,
    source_canonical: Vec<u8>,
    source_sha256: String,
    content: String,
    prepared_at: Clock,
    dispatch_not_before: Clock,
    account_captured_at: Clock,
    account_observed_at: Clock,
    expires_at: Clock,
}
impl PreparedDispatch {
    pub fn business_date(&self) -> chrono::NaiveDate {
        self.business_date
    }
    pub fn occurrence_identity(&self) -> &str {
        &self.occurrence_identity
    }
    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }
    pub fn source_canonical(&self) -> &[u8] {
        &self.source_canonical
    }
    pub fn content(&self) -> &str {
        &self.content
    }
    pub fn expires_at(&self) -> Clock {
        self.expires_at
    }
    pub fn retry_authorized(&self) -> bool {
        false
    }
    /// The counted owner calls this again at admission and before any resumed
    /// physical attempt. Holding a previously prepared borrow does not freeze
    /// wall time, keep an account snapshot fresh or extend the daily window.
    pub fn validate_consumption_at(&self, now: Clock) -> Result<(), DispatchBlock> {
        if now.offset().local_minus_utc() != 28800 || now < self.prepared_at {
            return Err(DispatchBlock::ClockReversed);
        }
        if now >= self.expires_at {
            return Err(DispatchBlock::Expired);
        }
        if now < self.dispatch_not_before {
            return Err(DispatchBlock::BeforeWindow);
        }
        if [self.account_captured_at, self.account_observed_at]
            .iter()
            .any(|time| {
                *time > now
                    || now.signed_duration_since(*time) > Duration::milliseconds(ACCOUNT_FRESH_MS)
            })
        {
            return Err(DispatchBlock::AccountStale);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerReceipt {
    Accepted,
    Duplicate,
    Unknown,
    Rejected,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchBlock {
    BeforeWindow,
    Expired,
    AccountStale,
    ClockReversed,
    AlreadyConsumed,
    UnknownRequiresOwnerReconcile,
}
enum Consumption {
    Pending,
    Prepared,
    Terminal(OwnerReceipt),
    Expired,
    Stale,
}

/// Kept in memory by the owner consumer. A serialized report cannot reconstruct
/// this capability; durable recovery belongs to the existing counted owner.
pub struct DispatchCandidate {
    request: PreparedDispatch,
    inspected_at: Clock,
    consumption: Consumption,
}
impl DispatchCandidate {
    fn from_qualified_preview(
        preview: &Preview,
        now: Clock,
        dispatch_not_before: Clock,
        expires_at: Clock,
    ) -> Result<Self, String> {
        let account = preview.account_snapshot.as_ref().ok_or("account missing")?;
        if account.account_ref.is_empty() || preview.state != State::Candidate {
            return Err("independent qualified real account candidate required".into());
        }
        let date = now.date_naive();
        let occurrence_identity = format!(
            "afterhours-sell-reminder:v1:{date}:{}",
            hash(&account.account_ref)
        );
        let content = preview.markdown();
        let canonical = serde_json::json!({
            "schema": VERSION, "business_date": date, "occurrence": occurrence_identity,
            "preview": preview, "rendered_content": content,
            "dispatch_not_before": dispatch_not_before, "expires_at": expires_at,
        });
        let source_sha256 = hash(&canonical);
        let source_canonical = serde_json::to_vec(&canonical).map_err(|error| error.to_string())?;
        Ok(Self {
            request: PreparedDispatch {
                business_date: date,
                occurrence_identity,
                source_canonical,
                source_sha256,
                content,
                prepared_at: now,
                dispatch_not_before,
                account_captured_at: account.captured_at,
                account_observed_at: account.observed_at,
                expires_at,
            },
            inspected_at: now,
            consumption: Consumption::Pending,
        })
    }

    /// Call at the point of owner consumption, not just when the bin prepared
    /// content. Once expired/stale/terminal it never becomes eligible again.
    pub fn prepare_dispatch(&mut self, now: Clock) -> Result<&PreparedDispatch, DispatchBlock> {
        if now.offset().local_minus_utc() != 28800 || now < self.inspected_at {
            return Err(DispatchBlock::ClockReversed);
        }
        self.inspected_at = now;
        match self.consumption {
            Consumption::Expired => return Err(DispatchBlock::Expired),
            Consumption::Stale => return Err(DispatchBlock::AccountStale),
            Consumption::Terminal(OwnerReceipt::Unknown) => {
                return Err(DispatchBlock::UnknownRequiresOwnerReconcile)
            }
            Consumption::Terminal(_) | Consumption::Prepared => {
                return Err(DispatchBlock::AlreadyConsumed)
            }
            Consumption::Pending => {}
        }
        if let Err(block) = self.request.validate_consumption_at(now) {
            if block == DispatchBlock::Expired {
                self.consumption = Consumption::Expired;
            } else if block == DispatchBlock::AccountStale {
                self.consumption = Consumption::Stale;
            }
            return Err(block);
        }
        self.consumption = Consumption::Prepared;
        Ok(&self.request)
    }

    /// Every result, including Unknown and explicit rejection, closes this local
    /// attempt. Unknown is reconciled by the durable owner, never blindly resent
    /// by the independent scheduled producer.
    pub fn record_owner_receipt(&mut self, receipt: OwnerReceipt) -> Result<(), String> {
        if !matches!(self.consumption, Consumption::Prepared) {
            return Err("owner receipt requires exactly one prepared attempt".into());
        }
        self.consumption = Consumption::Terminal(receipt);
        Ok(())
    }
}

/// Read the close portion from a real gateway capability without claiming that
/// daily-bar admission supplies real sellability, lifecycle, quantity or fees.
#[derive(Debug, Serialize)]
pub struct AdmittedCloseObservation {
    pub instrument: String,
    pub price_date: chrono::NaiveDate,
    pub close_micro_cny: i64,
    pub provider: String,
    pub source: String,
    pub source_at: Option<String>,
    pub observed_at: String,
    pub batch_id: String,
    pub authority: &'static str,
}
pub fn observe_admitted_close(
    batch: &crate::data_gateway::AdmittedDailyBars,
    as_of: Clock,
) -> Result<AdmittedCloseObservation, String> {
    if as_of.offset().local_minus_utc() != 28800
        || as_of < at(as_of.date_naive(), 15, 0)
        || crate::calendar::verified_a_share_trading_day(as_of.date_naive()) != Ok(true)
    {
        return Err(
            "close observation requires completed current verified Shanghai session".into(),
        );
    }
    let evidence = batch.evidence();
    let observed_at = chrono::DateTime::parse_from_rfc3339(&evidence.observed_at)
        .map_err(|_| "close observation timestamp invalid")?;
    let source_at = evidence
        .source_at
        .as_ref()
        .ok_or("close source timestamp missing")
        .and_then(|value| {
            chrono::DateTime::parse_from_rfc3339(value)
                .map_err(|_| "close source timestamp invalid")
        })?;
    if source_at > observed_at
        || observed_at > as_of
        || source_at < at(as_of.date_naive(), 15, 0)
        || evidence.source.is_empty()
        || evidence.batch_id.is_empty()
    {
        return Err(
            "close evidence must cover 15:00 with nonfuture source/observation timestamps".into(),
        );
    }
    let mut exact = batch
        .records()
        .iter()
        .filter(|bar| bar.date == as_of.date_naive());
    let bar = exact
        .next()
        .ok_or("exact current close price_date missing")?;
    if exact.next().is_some()
        || !bar.settled
        || bar.adjust != crate::data_provider::AdjustType::None
        || !bar.close.is_finite()
        || bar.close <= 0.0
    {
        return Err("one settled unadjusted positive exact close required".into());
    }
    let close_micro_cny = crate::trading::paper_ledger::Money::from_cny(bar.close)
        .map_err(|error| error.to_string())?
        .micros();
    Ok(AdmittedCloseObservation {
        instrument: batch.target_code().to_owned(),
        price_date: bar.date,
        close_micro_cny,
        provider: format!("{:?}", evidence.provider),
        source: evidence.source.clone(),
        source_at: evidence.source_at.clone(),
        observed_at: evidence.observed_at.clone(),
        batch_id: evidence.batch_id.clone(),
        authority: "QualifiedDailyCloseOnly/NotRealSellDispatchAuthority",
    })
}

#[cfg(test)]
#[path = "sell_reminder_producer_tests.rs"]
mod tests;
