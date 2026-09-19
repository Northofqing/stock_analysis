//! Closed Macro schedule. Adapters own effects and confirmations, never fallback rules.
use super::{render_gateway_sections, NativeOutcome};
use crate::data_gateway::{GeneralWebResearchBatch, GlobalNewsProvider};
use crate::grpc_client::client::macro_attempt::MacroQueryIdentity;
use crate::search_service::SearchResponse;
use futures::{stream::FuturesUnordered, FutureExt, StreamExt};
use std::collections::{BTreeMap, BTreeSet};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub(crate) enum QueryKey {
    Gateway(u8),
    Web { dimension: u8, candidate: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Route {
    Local,
    External,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Step {
    Prepare(Route),
    Health,
    Capabilities,
    Data { query: QueryKey, attempt: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RouteState {
    Unprepared,
    NeedsHealth,
    NeedsCapabilities,
    Ready,
    Rejected,
}

#[derive(Clone)]
pub(crate) enum QueryOutcome {
    Native(NativeOutcome),
    // Only Legacy custom SearchProvider registrations can construct this branch.
    LegacyCompat(SearchResponse),
}

#[derive(Clone)]
pub(crate) struct Candidate {
    pub(crate) ordinal: u32,
    pub(crate) provider: Option<crate::data_gateway::GeneralWebResearchProvider>,
    pub(crate) eligible: bool,
}

#[derive(Clone)]
pub(crate) struct Definition {
    pub(crate) observed_date: String,
    pub(crate) research_limit: usize,
    pub(crate) candidates: Vec<Candidate>,
    pub(crate) external_news: bool,
}

impl Definition {
    pub(crate) fn identity(&self, key: QueryKey) -> anyhow::Result<MacroQueryIdentity> {
        Ok(match key {
            QueryKey::Gateway(ordinal @ 1..=4) => MacroQueryIdentity::GlobalNews {
                provider: NEWS[usize::from(ordinal - 1)],
                limit: 20,
            },
            QueryKey::Gateway(5) => MacroQueryIdentity::EconomicCalendar,
            QueryKey::Web {
                dimension: dimension @ 1..=6,
                candidate,
            } => {
                let registered = self
                    .candidates
                    .iter()
                    .find(|entry| entry.ordinal == candidate)
                    .ok_or_else(|| {
                        anyhow::anyhow!("Macro candidate is not in the original definition")
                    })?;
                MacroQueryIdentity::SemanticSearch {
                    provider: registered.provider.ok_or_else(|| {
                        anyhow::anyhow!("custom Legacy provider has no native identity")
                    })?,
                    query: self.query(dimension),
                    limit: self.research_limit,
                }
            }
            _ => anyhow::bail!("Macro query key is outside its closed domain"),
        })
    }

    pub(crate) fn query(&self, dimension: u8) -> String {
        format!(
            "{}{}",
            self.observed_date,
            DIMENSIONS[usize::from(dimension - 1)].0
        )
    }
}

#[derive(Clone)]
pub(crate) struct QueryState {
    pub(crate) next_attempt: u32,
    pub(crate) retry_due: Option<i64>,
    pub(crate) terminal: Option<QueryOutcome>,
    pub(crate) terminal_version: Option<u64>,
    pub(crate) terminal_at: Option<i64>,
}

impl Default for QueryState {
    fn default() -> Self {
        Self {
            next_attempt: 1,
            retry_due: None,
            terminal: None,
            terminal_version: None,
            terminal_at: None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct Dimension {
    pub(crate) selected: Option<u32>,
    pub(crate) pace_due: i64,
}

#[derive(Clone)]
pub(crate) struct Snapshot {
    pub(crate) budget: BudgetMode,
    pub(crate) definition: Definition,
    pub(crate) local: RouteState,
    pub(crate) external: RouteState,
    pub(crate) queries: BTreeMap<QueryKey, QueryState>,
    pub(crate) dimensions: BTreeMap<u8, Dimension>,
    pub(crate) final_output: Option<String>,
}

impl Snapshot {
    pub(crate) fn terminal(&self, key: QueryKey) -> Option<&QueryOutcome> {
        self.queries.get(&key)?.terminal.as_ref()
    }

    pub(crate) fn gateway_due(&self) -> anyhow::Result<i64> {
        let mut anchor = None;
        for ordinal in 1..=5 {
            let query = self
                .queries
                .get(&QueryKey::Gateway(ordinal))
                .filter(|query| query.terminal.is_some())
                .ok_or_else(|| anyhow::anyhow!("Macro Gateway is not terminal"))?;
            let version = query
                .terminal_version
                .ok_or_else(|| anyhow::anyhow!("Macro terminal has no confirmation version"))?;
            let at = query
                .terminal_at
                .ok_or_else(|| anyhow::anyhow!("Macro terminal has no confirmation time"))?;
            if anchor.is_none_or(|(previous, _)| version > previous) {
                anchor = Some((version, at));
            }
        }
        anchor
            .unwrap()
            .1
            .checked_add(200_000)
            .ok_or_else(|| anyhow::anyhow!("Macro pace overflow"))
    }
}

pub(crate) struct Returned<Ticket, Material> {
    pub(crate) ticket: Ticket,
    pub(crate) material: Material,
}

pub(crate) enum RunEnd {
    Complete(String),
    BudgetExpired,
}

#[derive(Clone, Copy)]
pub(crate) enum BudgetMode {
    CallerOwned,
    DurableAbsolute { started_at: i64, deadline_at: i64 },
}

/// An Adapter may emit this only with no live/persisted unresolved effects.
#[derive(Debug, thiserror::Error)]
#[error("Macro durable budget expired without unresolved effects")]
pub(crate) struct BudgetExpired;

/// Every effect's begin and result are synchronous confirmations in this
/// Interface. Futures own their transport/ticket and cannot borrow the store.
pub(crate) trait MacroStepIo {
    type Ticket;
    type Material;
    type Call: Future<Output = anyhow::Result<Returned<Self::Ticket, Self::Material>>> + Unpin;
    type Wait: Future<Output = anyhow::Result<()>> + Unpin;

    fn open(&mut self) -> anyhow::Result<Snapshot>;
    fn checkpoint(&mut self) -> anyhow::Result<()>;
    fn now(&self) -> i64;
    fn candidate_eligible(&mut self, ordinal: u32) -> anyhow::Result<bool>;
    fn admit(&mut self, step: Step) -> anyhow::Result<Self::Call>;
    fn record(
        &mut self,
        step: Step,
        returned: Returned<Self::Ticket, Self::Material>,
    ) -> anyhow::Result<Snapshot>;
    fn settle(&mut self, query: QueryKey) -> anyhow::Result<Snapshot>;
    fn wait(&self, due: i64) -> Self::Wait;
    fn finish_dimension(
        &mut self,
        dimension: u8,
        selected: Option<u32>,
    ) -> anyhow::Result<Snapshot>;
    fn close(&mut self, end: RunEnd) -> anyhow::Result<String>;
}

const NEWS: [GlobalNewsProvider; 4] = [
    GlobalNewsProvider::Eastmoney,
    GlobalNewsProvider::Cailianpress,
    GlobalNewsProvider::Jin10,
    GlobalNewsProvider::ThePaper,
];
pub(crate) const DIMENSIONS: [(&str, &str); 6] = [
    ("A股 大盘 股市 最新动态", "### 🇨🇳 A股市场动态"),
    ("国际财经 地缘政治 最新消息", "### 🌍 国际财经 / 地缘政治"),
    ("美股 美联储 大宗商品 今日", "### 🇺🇸 美股 / 大宗商品"),
    ("中国 央行 财政 产业政策 重要新闻", "### 📋 宏观政策"),
    (
        "高盛 摩根 大摩 美银 JPMorgan 中国A股 市场观点 研报",
        "### 🏦 投行观点（高盛/摩根/美银）",
    ),
    (
        "证券时报 第一财经 21世纪经济报道 重要财经",
        "### 📰 财经媒体要闻",
    ),
];

pub(crate) fn web_lines(outcome: &QueryOutcome) -> Vec<String> {
    let records = match outcome {
        QueryOutcome::Native(NativeOutcome::Web(Ok(GeneralWebResearchBatch::Available {
            records,
            ..
        }))) => {
            return records
                .iter()
                .take(3)
                .map(|record| {
                    format!(
                        "- **{}** {}  \n  {}",
                        record.title,
                        record.published_at_raw.as_deref().unwrap_or(""),
                        record.snippet.chars().take(150).collect::<String>()
                    )
                })
                .collect();
        }
        QueryOutcome::LegacyCompat(response) if response.success => &response.results,
        _ => return Vec::new(),
    };
    records
        .iter()
        .filter(|record| record.evidence.is_research_only())
        .take(3)
        .map(|record| {
            format!(
                "- **{}** {}  \n  {}",
                record.title,
                record.published_date.as_deref().unwrap_or(""),
                record.snippet.chars().take(150).collect::<String>()
            )
        })
        .collect()
}

pub(crate) fn render(snapshot: &Snapshot) -> anyhow::Result<String> {
    let mut news = Vec::new();
    for (index, provider) in NEWS.into_iter().enumerate() {
        match snapshot.terminal(QueryKey::Gateway(index as u8 + 1)) {
            Some(QueryOutcome::Native(NativeOutcome::News(outcome))) => {
                news.push((provider, outcome.clone()))
            }
            _ => anyhow::bail!("Macro final lacks a native news terminal"),
        }
    }
    let economic = match snapshot.terminal(QueryKey::Gateway(5)) {
        Some(QueryOutcome::Native(NativeOutcome::Economic(outcome))) => outcome.clone(),
        _ => anyhow::bail!("Macro final lacks a native Economic terminal"),
    };
    let mut sections = render_gateway_sections(
        news.try_into()
            .map_err(|_| anyhow::anyhow!("Macro news cardinality"))?,
        economic,
    );
    for dimension in 1..=6 {
        let terminal = snapshot
            .dimensions
            .get(&dimension)
            .ok_or_else(|| anyhow::anyhow!("Macro dimension is not confirmed"))?;
        if let Some(candidate) = terminal.selected {
            let outcome = snapshot
                .terminal(QueryKey::Web {
                    dimension,
                    candidate,
                })
                .ok_or_else(|| anyhow::anyhow!("Macro selected query is not confirmed"))?;
            let lines = web_lines(outcome);
            if lines.is_empty() {
                anyhow::bail!("Macro selected query has no ResearchOnly lines");
            }
            sections.push(format!(
                "### 🔎 通用网页研究发现（ResearchOnly；不得作为金融事实）\n{}\n{}",
                DIMENSIONS[usize::from(dimension - 1)].1,
                lines.join("\n")
            ));
        }
    }
    Ok(format!(
        "## 📡 今日宏观 / 市场背景（{}）\n\n{}",
        snapshot.definition.observed_date,
        sections.join("\n\n")
    ))
}

enum Event<Ticket, Material> {
    Returned(Step, Returned<Ticket, Material>),
    RetryDue(QueryKey),
}

// Keeping the two closed future kinds generic preserves Legacy's Send future
// without imposing Send on the durable store owner. Neither variant borrows IO.
enum Pending<Call, Wait> {
    Call { step: Step, call: Call },
    Wait { key: QueryKey, wait: Wait },
}
impl<Ticket, Material, Call, Wait> Future for Pending<Call, Wait>
where
    Call: Future<Output = anyhow::Result<Returned<Ticket, Material>>> + Unpin,
    Wait: Future<Output = anyhow::Result<()>> + Unpin,
{
    type Output = anyhow::Result<Event<Ticket, Material>>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.get_mut() {
            Self::Call { step, call } => Pin::new(call)
                .poll(cx)
                .map(|result| result.map(|returned| Event::Returned(*step, returned))),
            Self::Wait { key, wait } => Pin::new(wait)
                .poll(cx)
                .map(|result| result.map(|()| Event::RetryDue(*key))),
        }
    }
}

fn route_step(route: Route, state: RouteState) -> Option<Step> {
    match state {
        RouteState::Unprepared => Some(Step::Prepare(route)),
        RouteState::NeedsHealth => Some(Step::Health),
        RouteState::NeedsCapabilities => Some(Step::Capabilities),
        RouteState::Ready | RouteState::Rejected => None,
    }
}

fn confirm<I: MacroStepIo>(
    io: &mut I,
    snapshot: &mut Snapshot,
    step: Step,
    returned: Returned<I::Ticket, I::Material>,
) -> anyhow::Result<()> {
    let next = io.record(step, returned)?;
    let new_terminals = next
        .queries
        .iter()
        .filter_map(|(key, query)| {
            (query.terminal.is_some() && snapshot.terminal(*key).is_none()).then_some(*key)
        })
        .collect::<Vec<_>>();
    *snapshot = next;
    for key in new_terminals {
        *snapshot = io.settle(key)?;
    }
    Ok(())
}

/// Execute only the requested keys; the same lane loop handles the five Gateway
/// keys and one current Web candidate. Retry timers replace their own lane.
async fn collect<I: MacroStepIo>(
    io: &mut I,
    snapshot: &mut Snapshot,
    keys: &[QueryKey],
) -> anyhow::Result<()> {
    let mut active = FuturesUnordered::<Pending<I::Call, I::Wait>>::new();
    let mut steps = BTreeSet::new();
    let mut waiting = BTreeSet::new();
    loop {
        // Drain every already-ready result before considering a new effect.
        while let Some(Some(event)) = active.next().now_or_never() {
            match event? {
                Event::Returned(step, returned) => {
                    steps.remove(&step);
                    confirm(io, snapshot, step, returned)?;
                }
                Event::RetryDue(key) => {
                    waiting.remove(&key);
                    io.checkpoint()?;
                }
            }
        }
        io.checkpoint()?;
        if keys.iter().all(|key| snapshot.terminal(*key).is_some()) {
            if !active.is_empty() {
                anyhow::bail!("Macro terminal set still has owned effects");
            }
            return Ok(());
        }
        let mut admitted = false;
        for key in keys {
            if snapshot.terminal(*key).is_some()
                || waiting.contains(key)
                || steps
                    .iter()
                    .any(|step| matches!(step, Step::Data { query, .. } if query == key))
            {
                continue;
            }
            let route = match key {
                QueryKey::Gateway(1..=4) if snapshot.definition.external_news => Route::External,
                _ => Route::Local,
            };
            let custom_legacy = matches!(key, QueryKey::Web { candidate, .. } if snapshot.definition.candidates.iter()
                .any(|entry| entry.ordinal == *candidate && entry.provider.is_none()));
            let state = if custom_legacy {
                RouteState::Ready
            } else if route == Route::External {
                snapshot.external
            } else {
                snapshot.local
            };
            let step = if let Some(step) = route_step(route, state) {
                step
            } else {
                if state == RouteState::Rejected {
                    anyhow::bail!("Macro rejected route lacks its logical terminal");
                }
                let query = snapshot.queries.get(key).cloned().unwrap_or_default();
                if let Some(due) = query.retry_due.filter(|due| *due > io.now()) {
                    if active.len() >= 5 {
                        break;
                    }
                    let wait = io.wait(due);
                    let key = *key;
                    waiting.insert(key);
                    active.push(Pending::Wait { key, wait });
                    admitted = true;
                    break;
                }
                Step::Data {
                    query: *key,
                    attempt: query.next_attempt,
                }
            };
            if steps.contains(&step) {
                continue;
            }
            if active.len() >= 5 {
                break;
            }
            let call = io.admit(step)?;
            steps.insert(step);
            active.push(Pending::Call { step, call });
            // Loop back to poll the just-admitted call before confirming another
            // begin. FuturesUnordered polls pending members without holding IO.
            admitted = true;
            break;
        }
        if admitted {
            continue;
        }
        let event = active
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("Macro schedule has no admissible effect"))??;
        match event {
            Event::Returned(step, returned) => {
                steps.remove(&step);
                confirm(io, snapshot, step, returned)?;
            }
            Event::RetryDue(key) => {
                waiting.remove(&key);
                io.checkpoint()?;
            }
        }
    }
}

pub(crate) async fn run<I: MacroStepIo>(io: &mut I) -> anyhow::Result<String> {
    match run_inner(io).await {
        Err(error) if error.downcast_ref::<BudgetExpired>().is_some() => {
            io.close(RunEnd::BudgetExpired)
        }
        result => result,
    }
}

async fn run_inner<I: MacroStepIo>(io: &mut I) -> anyhow::Result<String> {
    let mut snapshot = io.open()?;
    if let Some(output) = &snapshot.final_output {
        return Ok(output.clone());
    }
    let gateways = [
        QueryKey::Gateway(1),
        QueryKey::Gateway(2),
        QueryKey::Gateway(3),
        QueryKey::Gateway(4),
        QueryKey::Gateway(5),
    ];
    collect(io, &mut snapshot, &gateways).await?;
    io.wait(snapshot.gateway_due()?).await?;
    io.checkpoint()?;
    for dimension in 1..=6 {
        if !snapshot.dimensions.contains_key(&dimension) {
            let candidates = snapshot.definition.candidates.clone();
            let mut selected = None;
            for candidate in candidates {
                if !io.candidate_eligible(candidate.ordinal)? {
                    continue;
                }
                let key = QueryKey::Web {
                    dimension,
                    candidate: candidate.ordinal,
                };
                collect(io, &mut snapshot, &[key]).await?;
                if !web_lines(snapshot.terminal(key).unwrap()).is_empty() {
                    selected = Some(candidate.ordinal);
                    break;
                }
            }
            snapshot = io.finish_dimension(dimension, selected)?;
        }
        io.wait(snapshot.dimensions[&dimension].pace_due).await?;
        io.checkpoint()?;
    }
    let output = render(&snapshot)?;
    io.close(RunEnd::Complete(output))
}
