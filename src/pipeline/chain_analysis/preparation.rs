//! Fixed in-memory preparation of the actual chain report. This is not a durable checkpoint.

use anyhow::Result;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};

use super::{ChainCluster, PositionDiag};
use crate::analyzer::{AgentMode, GeminiAnalyzer};
use crate::data_gateway::{BatchEvidence, GatewayBatch};
use crate::market_data::TopStock;
use crate::search_service::SearchResult;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceStatus {
    Unknown,
    Available,
    VerifiedEmpty,
    Unavailable,
    NotRequested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelStage {
    SearchTerms,
    Deep,
    Simple,
    Overview,
    UnselectedCluster,
}

#[derive(Clone, Serialize, Deserialize)]
enum ModelOutcome {
    Returned(String),
    Failed(String),
    NotCalled(String),
}

/// Local analyzer invocation material, not remote wire data or routed model identity.
#[derive(Clone, Serialize, Deserialize)]
pub struct ModelCall {
    stage: ModelStage,
    concept: Option<String>,
    prompt: Option<String>,
    system: Option<String>,
    #[serde(with = "artifact_mode")]
    mode: Option<AgentMode>,
    provider_identity: Option<String>,
    model_identity: Option<String>,
    outcome: ModelOutcome,
}

impl ModelCall {
    fn not_called(stage: ModelStage, concept: Option<String>, reason: &str) -> Self {
        Self {
            stage,
            concept,
            prompt: None,
            system: None,
            mode: None,
            provider_identity: None,
            model_identity: None,
            outcome: ModelOutcome::NotCalled(reason.into()),
        }
    }
    pub fn stage(&self) -> ModelStage {
        self.stage
    }
    pub fn concept(&self) -> Option<&str> {
        self.concept.as_deref()
    }
    pub fn prompt(&self) -> Option<&str> {
        self.prompt.as_deref()
    }
    pub fn system(&self) -> Option<&str> {
        self.system.as_deref()
    }
    pub fn mode(&self) -> Option<AgentMode> {
        self.mode
    }
    pub fn response(&self) -> Option<&str> {
        if let ModelOutcome::Returned(text) = &self.outcome {
            Some(text)
        } else {
            None
        }
    }
    pub fn failure(&self) -> Option<&str> {
        if let ModelOutcome::Failed(error) = &self.outcome {
            Some(error)
        } else {
            None
        }
    }
    pub fn not_called_reason(&self) -> Option<&str> {
        if let ModelOutcome::NotCalled(reason) = &self.outcome {
            Some(reason)
        } else {
            None
        }
    }
    pub fn provider_identity(&self) -> Option<&str> {
        self.provider_identity.as_deref()
    }
    pub fn model_identity(&self) -> Option<&str> {
        self.model_identity.as_deref()
    }
}

impl std::fmt::Debug for ModelCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelCall")
            .field("stage", &self.stage)
            .field("called", &self.prompt.is_some())
            .field("succeeded", &self.response().is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchStage {
    Cluster,
    AfterMarket,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SearchObservation {
    stage: SearchStage,
    query: String,
    limit: usize,
    results: Vec<SearchResult>,
    source: SourceObservation,
}

impl SearchObservation {
    pub fn stage(&self) -> SearchStage {
        self.stage
    }
    pub fn query(&self) -> &str {
        &self.query
    }
    pub fn limit(&self) -> usize {
        self.limit
    }
    pub fn results(&self) -> &[SearchResult] {
        &self.results
    }
    pub fn source(&self) -> &SourceObservation {
        &self.source
    }
}

impl std::fmt::Debug for SearchObservation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchObservation")
            .field("stage", &self.stage)
            .field("result_count", &self.results.len())
            .field("status", &self.source.status)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SourceObservation {
    status: SourceStatus,
    source_at: Option<String>,
    batch_id: Option<String>,
    reason: Option<String>,
    provider: Option<crate::market_domain::ProviderId>,
    source: Option<String>,
    observed_at: Option<String>,
    request_date: Option<NaiveDate>,
    request_observed_at: Option<String>,
}

impl SourceObservation {
    pub fn unknown() -> Self {
        Self {
            status: SourceStatus::Unknown,
            source_at: None,
            batch_id: None,
            reason: None,
            provider: None,
            source: None,
            observed_at: None,
            request_date: None,
            request_observed_at: None,
        }
    }
    pub fn unavailable(reason: String) -> Self {
        Self {
            status: SourceStatus::Unavailable,
            reason: Some(reason),
            ..Self::unknown()
        }
    }
    pub(super) fn batch(status: SourceStatus, evidence: &BatchEvidence) -> Self {
        Self {
            status,
            source_at: evidence.source_at.clone(),
            batch_id: Some(evidence.batch_id.clone()),
            provider: Some(evidence.provider),
            source: Some(evidence.source.clone()),
            observed_at: Some(evidence.observed_at.clone()),
            ..Self::unknown()
        }
    }
    pub(super) fn requested(mut self, date: NaiveDate, observed_at: String) -> Self {
        self.request_date = Some(date);
        self.request_observed_at = Some(observed_at);
        self
    }
    pub fn status(&self) -> &SourceStatus {
        &self.status
    }
    pub fn source_at(&self) -> Option<&str> {
        self.source_at.as_deref()
    }
    pub fn batch_id(&self) -> Option<&str> {
        self.batch_id.as_deref()
    }
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
    pub fn provider(&self) -> Option<crate::market_domain::ProviderId> {
        self.provider
    }
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }
    pub fn observed_at(&self) -> Option<&str> {
        self.observed_at.as_deref()
    }
    pub fn request_date(&self) -> Option<NaiveDate> {
        self.request_date
    }
    pub fn request_observed_at(&self) -> Option<&str> {
        self.request_observed_at.as_deref()
    }
}

/// Only the database position fields consumed by this pipeline.
#[derive(Clone, Serialize, Deserialize)]
pub struct PositionInput {
    code: String,
    name: String,
    return_rate: Option<f64>,
}

impl PositionInput {
    pub fn new(code: String, name: String, return_rate: Option<f64>) -> Self {
        Self {
            code,
            name,
            return_rate,
        }
    }
    pub fn code(&self) -> &str {
        &self.code
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn return_rate(&self) -> Option<f64> {
        self.return_rate
    }
}

/// Owns the caller inputs and original report bytes; getters only lend immutable views.
#[derive(Clone)]
pub struct PreparedChainAnalysis {
    data: PreparedData,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreparedData {
    business_date: NaiveDate,
    limit_ups: Vec<TopStock>,
    limit_up_source: SourceObservation,
    macro_input: Option<String>,
    report: String,
    concepts: BTreeMap<String, Vec<String>>,
    clusters: Vec<ChainCluster>,
    isolated: Vec<TopStock>,
    positions: Vec<PositionInput>,
    position_concepts: BTreeMap<String, Vec<String>>,
    position_diags: Vec<PositionDiag>,
    candidate_sources: BTreeMap<String, SourceObservation>,
    board_evidence: Vec<SourceObservation>,
    board_directory: BTreeMap<String, String>,
    board_source: SourceObservation,
    candidate_board_codes: BTreeMap<String, String>,
    lhb_map: BTreeMap<String, f64>,
    lhb_source: SourceObservation,
    macro_context: String,
    macro_source: SourceObservation,
    macro_used_input: bool,
    model_calls: Vec<ModelCall>,
    cluster_news: BTreeMap<String, String>,
    cluster_news_sources: BTreeMap<String, SourceObservation>,
    after_market_context: String,
    after_market_source: SourceObservation,
    after_market_observed_at: Option<String>,
    search_observations: Vec<SearchObservation>,
    completed_stages: Vec<PreparationStage>,
    min_cluster_size: Option<usize>,
    concept_source: SourceObservation,
    positions_source: SourceObservation,
    position_concept_source: SourceObservation,
}

impl PreparedChainAnalysis {
    pub fn business_date(&self) -> NaiveDate {
        self.data.business_date
    }
    pub fn limit_ups(&self) -> &[TopStock] {
        &self.data.limit_ups
    }
    pub fn limit_up_source(&self) -> &SourceObservation {
        &self.data.limit_up_source
    }
    pub fn macro_input(&self) -> Option<&str> {
        self.data.macro_input.as_deref()
    }
    pub fn report(&self) -> &str {
        &self.data.report
    }
    pub fn concepts(&self) -> &BTreeMap<String, Vec<String>> {
        &self.data.concepts
    }
    pub fn clusters(&self) -> &[ChainCluster] {
        &self.data.clusters
    }
    pub fn isolated(&self) -> &[TopStock] {
        &self.data.isolated
    }
    pub fn positions(&self) -> &[PositionInput] {
        &self.data.positions
    }
    pub fn position_concepts(&self) -> &BTreeMap<String, Vec<String>> {
        &self.data.position_concepts
    }
    pub fn position_diags(&self) -> &[PositionDiag] {
        &self.data.position_diags
    }
    pub fn candidate_sources(&self) -> &BTreeMap<String, SourceObservation> {
        &self.data.candidate_sources
    }
    pub fn board_evidence(&self) -> &[SourceObservation] {
        &self.data.board_evidence
    }
    pub fn board_directory(&self) -> &BTreeMap<String, String> {
        &self.data.board_directory
    }
    pub fn board_source(&self) -> &SourceObservation {
        &self.data.board_source
    }
    pub fn candidate_board_codes(&self) -> &BTreeMap<String, String> {
        &self.data.candidate_board_codes
    }
    pub fn lhb_map(&self) -> &BTreeMap<String, f64> {
        &self.data.lhb_map
    }
    pub fn lhb_source(&self) -> &SourceObservation {
        &self.data.lhb_source
    }
    pub fn macro_context(&self) -> &str {
        &self.data.macro_context
    }
    pub fn macro_source(&self) -> &SourceObservation {
        &self.data.macro_source
    }
    pub fn macro_used_input(&self) -> bool {
        self.data.macro_used_input
    }
    pub fn model_calls(&self) -> &[ModelCall] {
        &self.data.model_calls
    }
    pub fn cluster_news(&self) -> &BTreeMap<String, String> {
        &self.data.cluster_news
    }
    pub fn cluster_news_sources(&self) -> &BTreeMap<String, SourceObservation> {
        &self.data.cluster_news_sources
    }
    pub fn after_market_context(&self) -> &str {
        &self.data.after_market_context
    }
    pub fn after_market_source(&self) -> &SourceObservation {
        &self.data.after_market_source
    }
    pub fn after_market_observed_at(&self) -> Option<&str> {
        self.data.after_market_observed_at.as_deref()
    }
    pub fn search_observations(&self) -> &[SearchObservation] {
        &self.data.search_observations
    }
    /// Returned pipeline stages, not transaction commits or persisted checkpoints.
    pub fn completed_stages(&self) -> &[PreparationStage] {
        &self.data.completed_stages
    }
    pub fn min_cluster_size(&self) -> Option<usize> {
        self.data.min_cluster_size
    }
    pub fn concept_source(&self) -> &SourceObservation {
        &self.data.concept_source
    }
    pub fn positions_source(&self) -> &SourceObservation {
        &self.data.positions_source
    }
    pub fn position_concept_source(&self) -> &SourceObservation {
        &self.data.position_concept_source
    }
}

const ARTIFACT_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
struct ArtifactRef<'a> {
    schema_version: u32,
    data: &'a PreparedData,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactOwned {
    schema_version: u32,
    data: PreparedData,
}

impl PreparedChainAnalysis {
    /// Confidential business material for protected storage, never for logs. This internal
    /// byte format is not Foundation canonical identity, Ready, a checkpoint or authentication.
    pub fn to_artifact_bytes(&self) -> Result<Vec<u8>> {
        validate_artifact_values(&self.data)?;
        let bytes = encode_artifact_data(&self.data)?;
        // serde_json's default float parser may not preserve every finite binary value.
        // Refuse an unstable encoding instead of silently accepting a changed value.
        let round_trip = decode_artifact_data(&bytes)?;
        if encode_artifact_data(&round_trip)? != bytes {
            anyhow::bail!("产业链准备 artifact 无法无损保存数值");
        }
        Ok(bytes)
    }

    /// Only accepts the exact compact deterministic bytes produced by this version.
    /// Re-encoding rejects omitted Option/default fields, duplicates (including map keys),
    /// unknown fields and alternative JSON representations. No external calls occur here.
    pub fn from_artifact_bytes(bytes: &[u8]) -> Result<Self> {
        let data = decode_artifact_data(bytes)?;
        if encode_artifact_data(&data)? != bytes {
            anyhow::bail!("产业链准备 artifact 不是受支持的确定性原字节");
        }
        Ok(Self { data })
    }
}

fn encode_artifact_data(data: &PreparedData) -> Result<Vec<u8>> {
    serde_json::to_vec(&ArtifactRef {
        schema_version: ARTIFACT_SCHEMA_VERSION,
        data,
    })
    .map_err(|_| anyhow::anyhow!("产业链准备 artifact 编码失败"))
}

fn decode_artifact_data(bytes: &[u8]) -> Result<PreparedData> {
    let artifact: ArtifactOwned = serde_json::from_slice(bytes)
        .map_err(|_| anyhow::anyhow!("产业链准备 artifact 输入无效"))?;
    if artifact.schema_version != ARTIFACT_SCHEMA_VERSION {
        anyhow::bail!("产业链准备 artifact 版本不受支持");
    }
    validate_artifact_values(&artifact.data)?;
    Ok(artifact.data)
}

fn validate_artifact_values(data: &PreparedData) -> Result<()> {
    fn stock_finite(stock: &TopStock) -> bool {
        stock.change_pct.is_finite()
            && stock.price.is_finite()
            && stock.volume_ratio.is_none_or(f64::is_finite)
            && stock.main_net_yi.is_none_or(f64::is_finite)
    }
    let stocks_finite = data
        .limit_ups
        .iter()
        .chain(&data.isolated)
        .chain(
            data.clusters
                .iter()
                .flat_map(|c| c.stocks.iter().chain(&c.candidates)),
        )
        .all(stock_finite);
    let cluster_values_finite = data.clusters.iter().all(|cluster| {
        cluster.score.as_ref().is_none_or(|score| {
            [
                score.logic_hardness,
                score.sentiment_position,
                score.fund_consensus,
                score.chip_health,
                score.falsify_prob,
            ]
            .into_iter()
            .all(f64::is_finite)
        }) && cluster.scenario.as_ref().is_none_or(|scenario| {
            [
                scenario.baseline_prob,
                scenario.bull_prob,
                scenario.bear_prob,
            ]
            .into_iter()
            .all(f64::is_finite)
        })
    });
    let other_values_finite = data
        .positions
        .iter()
        .all(|p| p.return_rate.is_none_or(f64::is_finite))
        && data
            .position_diags
            .iter()
            .all(|p| p.return_rate.is_none_or(f64::is_finite))
        && data.lhb_map.values().all(|value| value.is_finite())
        && data
            .search_observations
            .iter()
            .flat_map(|s| &s.results)
            .all(|result| result.relevance.is_finite());
    if !stocks_finite || !cluster_values_finite || !other_values_finite {
        anyhow::bail!("产业链准备 artifact 含不可保存的非有限数值");
    }
    if data
        .model_calls
        .iter()
        .any(|call| call.provider_identity.is_some() || call.model_identity.is_some())
    {
        anyhow::bail!("产业链准备 artifact 包含本版本未提供的模型身份");
    }
    Ok(())
}

mod artifact_mode {
    use crate::analyzer::AgentMode;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(
        mode: &Option<AgentMode>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        mode.map(|mode| match mode {
            AgentMode::Quick => "quick",
            AgentMode::Deep => "deep",
        })
        .serialize(serializer)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<AgentMode>, D::Error> {
        match Option::<String>::deserialize(deserializer)?.as_deref() {
            None => Ok(None),
            Some("quick") => Ok(Some(AgentMode::Quick)),
            Some("deep") => Ok(Some(AgentMode::Deep)),
            Some(_) => Err(serde::de::Error::custom("unsupported local model mode")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreparationStage {
    Concepts,
    ClusterWritesAndLifecycle,
    Candidates,
    Positions,
    PositionConcepts,
    DragonTiger,
    Macro,
    ModelsSearchAndReport,
}

/// Failure retains the facts observed before interruption. No rollback is implied.
/// The failed adapter may have performed effects before returning an error.
pub struct PreparationFailure {
    stage: PreparationStage,
    reason: String,
    observed: PreparedChainAnalysis,
}

impl PreparationFailure {
    pub fn business_date(&self) -> NaiveDate {
        self.observed.business_date()
    }
    pub fn stage(&self) -> PreparationStage {
        self.stage
    }
    pub fn reason(&self) -> &str {
        &self.reason
    }
    pub fn completed_stages(&self) -> &[PreparationStage] {
        self.observed.completed_stages()
    }
    pub fn failed_stage_may_have_effects(&self) -> bool {
        true
    }
    pub fn limit_ups(&self) -> &[TopStock] {
        self.observed.limit_ups()
    }
    pub fn macro_input(&self) -> Option<&str> {
        self.observed.macro_input()
    }
    pub fn concepts(&self) -> &BTreeMap<String, Vec<String>> {
        self.observed.concepts()
    }
    /// Lifecycle fields are only resolved once ClusterWritesAndLifecycle returned.
    pub fn clusters(&self) -> &[ChainCluster] {
        self.observed.clusters()
    }
    pub fn isolated(&self) -> &[TopStock] {
        self.observed.isolated()
    }
    pub fn candidate_sources(&self) -> &BTreeMap<String, SourceObservation> {
        self.observed.candidate_sources()
    }
    pub fn board_evidence(&self) -> &[SourceObservation] {
        self.observed.board_evidence()
    }
    pub fn board_directory(&self) -> &BTreeMap<String, String> {
        self.observed.board_directory()
    }
    pub fn board_source(&self) -> &SourceObservation {
        self.observed.board_source()
    }
    pub fn candidate_board_codes(&self) -> &BTreeMap<String, String> {
        self.observed.candidate_board_codes()
    }
    pub fn positions(&self) -> &[PositionInput] {
        self.observed.positions()
    }
    pub fn position_concepts(&self) -> &BTreeMap<String, Vec<String>> {
        self.observed.position_concepts()
    }
    pub fn position_diags(&self) -> &[PositionDiag] {
        self.observed.position_diags()
    }
    pub fn lhb_map(&self) -> &BTreeMap<String, f64> {
        self.observed.lhb_map()
    }
    pub fn lhb_source(&self) -> &SourceObservation {
        self.observed.lhb_source()
    }
}

impl std::fmt::Debug for PreparationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparationFailure")
            .field("business_date", &self.business_date())
            .field("stage", &self.stage)
            .field("returned_stage_count", &self.completed_stages().len())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for PreparationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "产业链准备阶段 {:?} 失败；原因保留在受保护观察中",
            self.stage
        )
    }
}
impl std::error::Error for PreparationFailure {}

fn observe_stage<T>(
    result: Result<T>,
    stage: PreparationStage,
    prepared: &mut PreparedChainAnalysis,
) -> Result<T> {
    match result {
        Ok(value) => {
            prepared.data.completed_stages.push(stage);
            Ok(value)
        }
        Err(error) => {
            log::warn!("[产业链] 核心准备阶段 {:?} 失败，停止后续分析", stage);
            Err(PreparationFailure {
                stage,
                reason: format!("{error:#}"),
                observed: prepared.clone(),
            }
            .into())
        }
    }
}

impl std::fmt::Debug for PreparedChainAnalysis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedChainAnalysis")
            .field("business_date", &self.data.business_date)
            .field("limit_up_count", &self.data.limit_ups.len())
            .field("report_bytes", &self.data.report.len())
            .finish_non_exhaustive()
    }
}

/// External concepts/cache I/O. Clustering and report construction remain in the module.
#[async_trait::async_trait(?Send)]
pub trait ChainPreparationIo {
    async fn concepts(&mut self, codes: &[String]) -> Result<HashMap<String, Vec<String>>>;
    // Default methods fail closed. A partial controlled adapter can never fall through to live I/O.
    fn min_cluster_size(&mut self) -> usize {
        panic!("cluster configuration I/O not supplied")
    }
    async fn persist_clusters(
        &mut self,
        _date: NaiveDate,
        _rows: &[(String, Vec<String>, i32)],
    ) -> Result<HashMap<String, i64>> {
        panic!("chain database I/O not supplied")
    }
    async fn board_codes(&mut self) -> Result<(HashMap<String, String>, Vec<BatchEvidence>)> {
        panic!("board I/O not supplied")
    }
    async fn candidates(
        &mut self,
        _board: &str,
        _excluded: &HashSet<String>,
    ) -> Result<GatewayBatch<TopStock>> {
        panic!("candidate I/O not supplied")
    }
    async fn positions(&mut self) -> Result<Vec<PositionInput>> {
        panic!("position database I/O not supplied")
    }
    async fn lhb(&mut self) -> Result<(HashMap<String, f64>, SourceObservation)> {
        panic!("dragon-tiger I/O not supplied")
    }
    async fn macro_search(&mut self) -> Result<String> {
        panic!("macro search I/O not supplied")
    }
    fn model_available(&mut self) -> bool {
        panic!("model configuration I/O not supplied")
    }
    async fn model(&self, _prompt: &str, _system: &str, _mode: AgentMode) -> Result<String> {
        panic!("model I/O not supplied")
    }
    fn search_available(&mut self) -> bool {
        panic!("search configuration I/O not supplied")
    }
    async fn search_topic(&mut self, _query: &str, _limit: usize) -> Result<Vec<SearchResult>> {
        panic!("topic search I/O not supplied")
    }
    fn local_now(&mut self) -> chrono::DateTime<chrono::FixedOffset> {
        panic!("clock I/O not supplied")
    }
}

struct ProductionIo {
    analyzer: Option<GeminiAnalyzer>,
}

#[async_trait::async_trait(?Send)]
impl ChainPreparationIo for ProductionIo {
    async fn concepts(&mut self, codes: &[String]) -> Result<HashMap<String, Vec<String>>> {
        super::fetch_concepts_cached(codes)
            .await
            .map_err(anyhow::Error::msg)
    }
    fn min_cluster_size(&mut self) -> usize {
        super::min_cluster_size()
    }
    async fn persist_clusters(
        &mut self,
        date: NaiveDate,
        rows: &[(String, Vec<String>, i32)],
    ) -> Result<HashMap<String, i64>> {
        let db = crate::database::DatabaseManager::try_get()
            .ok_or_else(|| anyhow::anyhow!("产业链主线数据库未初始化"))?;
        db.save_chain_clusters(&date.to_string(), rows)
            .map_err(anyhow::Error::msg)?;
        rows.iter()
            .map(|(concept, _, _)| {
                Ok((
                    concept.clone(),
                    db.get_chain_appearance_days_as_of_strict(concept, 10, date)
                        .map_err(anyhow::Error::msg)?,
                ))
            })
            .collect()
    }
    async fn board_codes(&mut self) -> Result<(HashMap<String, String>, Vec<BatchEvidence>)> {
        let batch = super::fetch_board_code_map()
            .await
            .map_err(anyhow::Error::msg)?;
        Ok((batch.codes, batch.evidence))
    }
    async fn candidates(
        &mut self,
        board: &str,
        excluded: &HashSet<String>,
    ) -> Result<GatewayBatch<TopStock>> {
        super::fetch_laggard_candidates(board, excluded)
            .await
            .map_err(anyhow::Error::msg)
    }
    async fn positions(&mut self) -> Result<Vec<PositionInput>> {
        let db = crate::database::DatabaseManager::try_get()
            .ok_or_else(|| anyhow::anyhow!("持仓主线诊断数据库未初始化"))?;
        Ok(db
            .get_all_open_positions()
            .map_err(|error| anyhow::anyhow!("持仓主线诊断查询失败: {error}"))?
            .into_iter()
            .map(|p| PositionInput::new(p.code, p.name, p.return_rate))
            .collect())
    }
    async fn lhb(&mut self) -> Result<(HashMap<String, f64>, SourceObservation)> {
        super::fetchers::fetch_lhb_observed()
            .await
            .map_err(anyhow::Error::msg)
    }
    async fn macro_search(&mut self) -> Result<String> {
        Ok(crate::search_service::get_search_service()
            .search_macro_news(3)
            .await)
    }
    fn model_available(&mut self) -> bool {
        let analyzer = self.analyzer.get_or_insert_with(GeminiAnalyzer::from_env);
        analyzer.is_available()
    }
    async fn model(&self, prompt: &str, system: &str, mode: AgentMode) -> Result<String> {
        self.analyzer
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("产业链模型未初始化"))?
            .call_api_mode(prompt, system, mode)
            .await
    }
    fn search_available(&mut self) -> bool {
        crate::search_service::get_search_service().is_available()
    }
    async fn search_topic(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        Ok(crate::search_service::get_search_service()
            .search_topic(query, limit)
            .await)
    }
    fn local_now(&mut self) -> chrono::DateTime<chrono::FixedOffset> {
        chrono::Local::now().fixed_offset()
    }
}

pub async fn prepare_chain_analysis(
    business_date: NaiveDate,
    limit_ups: Vec<TopStock>,
    macro_news: Option<String>,
) -> Result<PreparedChainAnalysis> {
    prepare_chain_analysis_with_io(
        business_date,
        limit_ups,
        macro_news,
        &mut ProductionIo { analyzer: None },
    )
    .await
}

pub async fn prepare_chain_analysis_with_io(
    business_date: NaiveDate,
    limit_ups: Vec<TopStock>,
    macro_news: Option<String>,
    io: &mut impl ChainPreparationIo,
) -> Result<PreparedChainAnalysis> {
    let mut prepared = PreparedChainAnalysis {
        data: PreparedData {
            business_date,
            limit_ups,
            limit_up_source: SourceObservation::unknown(),
            macro_input: macro_news,
            report: String::new(),
            concepts: BTreeMap::new(),
            clusters: Vec::new(),
            isolated: Vec::new(),
            positions: Vec::new(),
            position_concepts: BTreeMap::new(),
            position_diags: Vec::new(),
            candidate_sources: BTreeMap::new(),
            board_evidence: Vec::new(),
            board_directory: BTreeMap::new(),
            board_source: SourceObservation {
                status: SourceStatus::NotRequested,
                ..SourceObservation::unknown()
            },
            candidate_board_codes: BTreeMap::new(),
            lhb_map: BTreeMap::new(),
            lhb_source: SourceObservation::unknown(),
            macro_context: String::new(),
            macro_source: SourceObservation {
                status: SourceStatus::NotRequested,
                ..SourceObservation::unknown()
            },
            macro_used_input: false,
            model_calls: Vec::new(),
            cluster_news: BTreeMap::new(),
            cluster_news_sources: BTreeMap::new(),
            after_market_context: String::new(),
            after_market_source: SourceObservation {
                status: SourceStatus::NotRequested,
                ..SourceObservation::unknown()
            },
            after_market_observed_at: None,
            search_observations: Vec::new(),
            completed_stages: Vec::new(),
            min_cluster_size: None,
            concept_source: SourceObservation {
                status: SourceStatus::NotRequested,
                ..SourceObservation::unknown()
            },
            positions_source: SourceObservation {
                status: SourceStatus::NotRequested,
                ..SourceObservation::unknown()
            },
            position_concept_source: SourceObservation {
                status: SourceStatus::NotRequested,
                ..SourceObservation::unknown()
            },
        },
    };
    if prepared.data.limit_ups.is_empty() {
        prepared.data.report = format!(
            "# 产业链联动分析报告 {}\n\n涨停池批次成功返回 0 只，无可分析内容。\n",
            business_date.format("%Y-%m-%d")
        );
        return Ok(prepared);
    }

    // Concepts may read/write the cache. The owned observations are not durable checkpoints.
    let codes = prepared
        .data
        .limit_ups
        .iter()
        .map(|s| s.code.clone())
        .collect::<Vec<_>>();
    let concepts = observe_stage(
        io.concepts(&codes).await,
        PreparationStage::Concepts,
        &mut prepared,
    )?;
    prepared.data.concepts = concepts.clone().into_iter().collect();
    prepared.data.concept_source = SourceObservation::unknown();
    let min_size = io.min_cluster_size();
    prepared.data.min_cluster_size = Some(min_size);
    let (mut clusters, isolated) =
        super::cluster_by_concept(&prepared.data.limit_ups, &concepts, min_size);
    prepared.data.clusters = clusters.clone();
    prepared.data.isolated = isolated.clone();
    let rows = clusters
        .iter()
        .map(|c| {
            Ok((
                c.concept.clone(),
                c.stocks.iter().map(|s| s.code.clone()).collect(),
                i32::try_from(c.continuation_count)?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let lifecycle_result = io
        .persist_clusters(business_date, &rows)
        .await
        .and_then(|streaks| {
            for cluster in &mut clusters {
                cluster.streak_days = *streaks
                    .get(&cluster.concept)
                    .ok_or_else(|| anyhow::anyhow!("产业链生命周期结果缺失"))?;
            }
            Ok(())
        });
    prepared.data.clusters = clusters.clone();
    observe_stage(
        lifecycle_result,
        PreparationStage::ClusterWritesAndLifecycle,
        &mut prepared,
    )?;

    let candidates = prepare_candidates(io, &mut clusters, &prepared.data.limit_ups).await;
    let candidate_statuses = candidates.statuses;
    prepared.data.clusters = clusters.clone();
    prepared.data.candidate_sources = candidates.sources;
    prepared.data.board_evidence = candidates.board_evidence;
    prepared.data.board_directory = candidates.board_directory;
    prepared.data.board_source = candidates.board_source;
    prepared.data.candidate_board_codes = candidates.selected_boards;
    prepared
        .data
        .completed_stages
        .push(PreparationStage::Candidates);
    let positions = observe_stage(
        io.positions().await,
        PreparationStage::Positions,
        &mut prepared,
    )?;
    prepared.data.positions = positions.clone();
    prepared.data.positions_source = SourceObservation::unknown();
    let position_concepts = if positions.is_empty() {
        HashMap::new()
    } else {
        observe_stage(
            io.concepts(&positions.iter().map(|p| p.code.clone()).collect::<Vec<_>>())
                .await,
            PreparationStage::PositionConcepts,
            &mut prepared,
        )?
    };
    prepared.data.position_concepts = position_concepts.clone().into_iter().collect();
    if !positions.is_empty() {
        prepared.data.position_concept_source = SourceObservation::unknown();
    }
    let position_diags = positions
        .iter()
        .map(|p| PositionDiag {
            code: p.code.clone(),
            name: p.name.clone(),
            return_rate: p.return_rate,
            in_limit_pool: clusters
                .iter()
                .any(|c| c.stocks.iter().any(|s| s.code == p.code)),
            mainline: clusters
                .iter()
                .find(|c| {
                    c.stocks.iter().any(|s| s.code == p.code)
                        || position_concepts.get(&p.code).is_some_and(|tags| {
                            tags.iter()
                                .any(|t| t == &c.concept || c.aliases.contains(t))
                        })
                })
                .map(|c| (c.concept.clone(), c.streak_days)),
        })
        .collect::<Vec<_>>();
    prepared.data.position_diags = position_diags.clone();
    let (lhb_map, lhb_source) =
        observe_stage(io.lhb().await, PreparationStage::DragonTiger, &mut prepared)?;
    prepared.data.lhb_map = lhb_map.clone().into_iter().collect();
    prepared.data.lhb_source = lhb_source.clone();
    let macro_context = if let Some(text) = prepared
        .data
        .macro_input
        .as_ref()
        .filter(|text| !text.trim().is_empty())
    {
        prepared.data.macro_used_input = true;
        prepared.data.macro_source = SourceObservation::unknown();
        text.clone()
    } else {
        match tokio::time::timeout(std::time::Duration::from_secs(15), io.macro_search()).await {
            Ok(Ok(text)) => {
                // Upstream returns text without source status, even when internally degraded.
                prepared.data.macro_source = SourceObservation::unknown();
                text
            }
            Ok(Err(error)) => {
                log::warn!("[产业链] 宏观搜索失败，降级为空背景");
                prepared.data.macro_source = SourceObservation::unavailable(error.to_string());
                String::new()
            }
            Err(_) => {
                log::warn!("[产业链] 宏观搜索超时，降级为空背景");
                prepared.data.macro_source =
                    SourceObservation::unavailable("宏观新闻搜索超时（15秒）".into());
                String::new()
            }
        }
    };
    prepared.data.macro_context = macro_context.clone();
    prepared.data.completed_stages.push(PreparationStage::Macro);
    let rendered = render_with_io(
        io,
        &business_date.to_string(),
        &prepared.data.limit_ups,
        &concepts,
        &clusters,
        &candidate_statuses,
        &isolated,
        &position_diags,
        &lhb_map,
        &macro_context,
    )
    .await?;
    prepared.data.report = rendered.report;
    prepared.data.model_calls = rendered.model_calls;
    prepared.data.cluster_news = rendered.cluster_news;
    prepared.data.cluster_news_sources = rendered.cluster_news_sources;
    prepared.data.after_market_context = rendered.after_market;
    prepared.data.after_market_source = rendered.after_market_source;
    prepared.data.after_market_observed_at = rendered.after_market_observed_at;
    prepared.data.search_observations = rendered.searches;
    prepared
        .data
        .completed_stages
        .push(PreparationStage::ModelsSearchAndReport);
    prepared.data.concepts = concepts.into_iter().collect();
    prepared.data.clusters = clusters;
    prepared.data.isolated = isolated;
    prepared.data.positions = positions;
    prepared.data.position_concepts = position_concepts.into_iter().collect();
    prepared.data.position_diags = position_diags;
    prepared.data.lhb_map = lhb_map.into_iter().collect();
    prepared.data.lhb_source = lhb_source;
    prepared.data.macro_context = macro_context;
    Ok(prepared)
}

struct CandidatePreparation {
    statuses: super::CandidateSupplementStatuses,
    sources: BTreeMap<String, SourceObservation>,
    board_evidence: Vec<SourceObservation>,
    board_directory: BTreeMap<String, String>,
    board_source: SourceObservation,
    selected_boards: BTreeMap<String, String>,
}

async fn prepare_candidates(
    io: &mut impl ChainPreparationIo,
    clusters: &mut [ChainCluster],
    limit_ups: &[TopStock],
) -> CandidatePreparation {
    let mut statuses = super::CandidateSupplementStatuses::new();
    let mut sources = BTreeMap::new();
    let mut board_evidence = Vec::new();
    let mut board_directory = BTreeMap::new();
    let board_source;
    let mut selected_boards = BTreeMap::new();
    let limit = super::MAX_DEEP_ANALYSIS + super::MAX_SIMPLE_ANALYSIS;
    let excluded = limit_ups.iter().map(|s| s.code.clone()).collect();
    match io.board_codes().await {
        Ok((codes, evidence)) => {
            board_evidence = evidence
                .iter()
                .map(|e| SourceObservation::batch(SourceStatus::Available, e))
                .collect();
            board_directory = codes.clone().into_iter().collect();
            board_source = SourceObservation {
                status: SourceStatus::Available,
                ..SourceObservation::unknown()
            };
            for cluster in clusters.iter_mut().take(limit) {
                let result = match super::resolve_cluster_board_code(cluster, &codes) {
                    Ok(board) => {
                        selected_boards.insert(cluster.concept.clone(), board.to_owned());
                        io.candidates(board, &excluded)
                            .await
                            .map_err(|e| e.to_string())
                    }
                    Err(error) => Err(error.to_string()),
                };
                // Capture all five original fields before optional report policy rejects an
                // Available-empty batch. Never reconstruct acquisition evidence from its reason.
                let original_candidate_evidence = match &result {
                    Ok(GatewayBatch::Available { evidence, .. })
                    | Ok(GatewayBatch::VerifiedEmpty(evidence)) => Some(evidence.clone()),
                    Err(_) => None,
                };
                let status = super::commit_laggard_candidate_batch(cluster, result, &evidence);
                let mut source = candidate_observation(&status);
                if let Some(evidence) = original_candidate_evidence {
                    let reason = source.reason.take();
                    source = SourceObservation::batch(source.status, &evidence);
                    source.reason = reason;
                }
                sources.insert(cluster.concept.clone(), source);
                if matches!(status, super::CandidateSupplementStatus::Unavailable { .. }) {
                    log::warn!("[产业链] 补涨候选不可用，继续核心分析");
                }
                statuses.insert(cluster.concept.clone(), status);
            }
        }
        Err(error) => {
            log::warn!("[产业链] 补涨候选板块目录不可用，继续核心分析");
            board_source = SourceObservation::unavailable(error.to_string());
            for cluster in clusters.iter().take(limit) {
                statuses.insert(
                    cluster.concept.clone(),
                    super::CandidateSupplementStatus::Unavailable {
                        reason: format!("补涨候选板块目录不可用: {error}"),
                        board_evidence: Vec::new(),
                    },
                );
            }
        }
    }
    for cluster in clusters.iter().skip(limit) {
        statuses.insert(
            cluster.concept.clone(),
            super::CandidateSupplementStatus::NotRequested(
                "超出已登记的深度/简化分析数量上限".into(),
            ),
        );
    }
    for (concept, status) in &statuses {
        sources
            .entry(concept.clone())
            .or_insert_with(|| candidate_observation(status));
    }
    CandidatePreparation {
        statuses,
        sources,
        board_evidence,
        board_directory,
        board_source,
        selected_boards,
    }
}

fn candidate_observation(status: &super::CandidateSupplementStatus) -> SourceObservation {
    use super::CandidateSupplementStatus as C;
    match status {
        C::Available {
            candidate_evidence, ..
        } => SourceObservation::batch(SourceStatus::Available, candidate_evidence),
        C::VerifiedEmpty {
            candidate_evidence, ..
        } => SourceObservation::batch(SourceStatus::VerifiedEmpty, candidate_evidence),
        C::Unavailable { reason, .. } => SourceObservation::unavailable(reason.clone()),
        C::NotRequested(reason) => SourceObservation {
            status: SourceStatus::NotRequested,
            reason: Some(reason.clone()),
            ..SourceObservation::unknown()
        },
    }
}

/// The same prompt builders are used by production preparation and direct protocol regressions.
#[async_trait::async_trait(?Send)]
pub(super) trait ChainModel {
    async fn call_api_mode(&self, prompt: &str, system: &str, mode: AgentMode) -> Result<String>;
}

#[async_trait::async_trait(?Send)]
impl ChainModel for GeminiAnalyzer {
    async fn call_api_mode(&self, prompt: &str, system: &str, mode: AgentMode) -> Result<String> {
        GeminiAnalyzer::call_api_mode(self, prompt, system, mode).await
    }
}

struct IoModel<'a, I> {
    io: &'a I,
    observations: &'a RefCell<Vec<ModelCall>>,
    stage: ModelStage,
    concept: Option<&'a str>,
}

#[async_trait::async_trait(?Send)]
impl<I: ChainPreparationIo> ChainModel for IoModel<'_, I> {
    async fn call_api_mode(&self, prompt: &str, system: &str, mode: AgentMode) -> Result<String> {
        let result = self.io.model(prompt, system, mode).await;
        let outcome = match &result {
            Ok(text) => ModelOutcome::Returned(text.clone()),
            Err(error) => {
                log::warn!("[产业链] 模型阶段 {:?} 失败，保留缺失分析", self.stage);
                ModelOutcome::Failed(error.to_string())
            }
        };
        self.observations.borrow_mut().push(ModelCall {
            stage: self.stage,
            concept: self.concept.map(str::to_owned),
            prompt: Some(prompt.into()),
            system: Some(system.into()),
            mode: Some(mode),
            provider_identity: None,
            model_identity: None,
            outcome,
        });
        result
    }
}

struct RenderedPreparation {
    report: String,
    model_calls: Vec<ModelCall>,
    cluster_news: BTreeMap<String, String>,
    cluster_news_sources: BTreeMap<String, SourceObservation>,
    after_market: String,
    after_market_source: SourceObservation,
    after_market_observed_at: Option<String>,
    searches: Vec<SearchObservation>,
}

#[allow(clippy::too_many_arguments)]
async fn render_with_io(
    io: &mut impl ChainPreparationIo,
    date: &str,
    limit_ups: &[TopStock],
    concepts: &HashMap<String, Vec<String>>,
    clusters: &[ChainCluster],
    candidate_statuses: &super::CandidateSupplementStatuses,
    isolated: &[TopStock],
    positions: &[PositionDiag],
    lhb: &HashMap<String, f64>,
    macro_context: &str,
) -> Result<RenderedPreparation> {
    let available = io.model_available();
    if !available {
        log::warn!("[产业链] AI 模型未配置，仅输出聚类结果");
    }
    let observations = RefCell::new(Vec::new());
    let mut searches = Vec::new();
    let mut cluster_news = BTreeMap::new();
    let mut cluster_news_sources = BTreeMap::new();
    let mut sections = Vec::new();
    let (mut deep_count, mut simple_count) = (0, 0);
    for cluster in clusters {
        let count = cluster.stocks.len();
        let analysis =
            if count >= super::TIER1_MIN && available && deep_count < super::MAX_DEEP_ANALYSIS {
                deep_count += 1;
                let (news, source) =
                    cluster_news_with_io(io, cluster, concepts, &observations, &mut searches).await;
                cluster_news.insert(cluster.concept.clone(), news.clone());
                cluster_news_sources.insert(cluster.concept.clone(), source);
                super::analyze_cluster_deep(
                    &IoModel {
                        io: &*io,
                        observations: &observations,
                        stage: ModelStage::Deep,
                        concept: Some(&cluster.concept),
                    },
                    cluster,
                    super::DeepClusterAnalysisContext {
                        concepts,
                        lhb_map: lhb,
                        macro_news: macro_context,
                        cluster_news: &news,
                        date,
                        candidate_status: candidate_statuses.get(&cluster.concept),
                    },
                )
                .await
                .ok()
            } else if count >= super::TIER2_MIN
                && available
                && simple_count < super::MAX_SIMPLE_ANALYSIS
            {
                simple_count += 1;
                super::analyze_cluster_simple(
                    &IoModel {
                        io: &*io,
                        observations: &observations,
                        stage: ModelStage::Simple,
                        concept: Some(&cluster.concept),
                    },
                    cluster,
                    concepts,
                    lhb,
                    date,
                    candidate_statuses.get(&cluster.concept),
                )
                .await
                .ok()
            } else {
                observations.borrow_mut().push(ModelCall::not_called(
                    ModelStage::UnselectedCluster,
                    Some(cluster.concept.clone()),
                    if available {
                        "未达到分析分级或已超出数量上限"
                    } else {
                        "AI 模型未配置"
                    },
                ));
                None
            };
        sections.push((cluster.concept.clone(), analysis));
    }
    log::info!(
        "[产业链] LLM 分析: 深度={} 简化={} 仅聚类={}",
        deep_count,
        simple_count,
        clusters.len().saturating_sub(deep_count + simple_count)
    );
    let themes = clusters
        .iter()
        .take(5)
        .map(|c| c.concept.as_str())
        .collect::<Vec<_>>();
    let (after_market, after_market_source, after_market_observed_at) = if available {
        after_market_with_io(io, &themes, &mut searches).await
    } else {
        (
            String::new(),
            SourceObservation {
                status: SourceStatus::NotRequested,
                reason: Some("AI 模型未配置".into()),
                ..SourceObservation::unknown()
            },
            None,
        )
    };
    let overview = if available && !sections.is_empty() {
        super::synthesize_overview(
            &IoModel {
                io: &*io,
                observations: &observations,
                stage: ModelStage::Overview,
                concept: None,
            },
            clusters,
            &sections,
            positions,
            date,
            &after_market,
        )
        .await
    } else {
        observations.borrow_mut().push(ModelCall::not_called(
            ModelStage::Overview,
            None,
            if available {
                "无主线簇"
            } else {
                "AI 模型未配置"
            },
        ));
        None
    };
    let report = super::build_report(
        date,
        limit_ups,
        clusters,
        &sections,
        candidate_statuses,
        isolated,
        overview.as_deref(),
        &after_market,
        concepts,
        positions,
    );
    Ok(RenderedPreparation {
        report,
        model_calls: observations.into_inner(),
        cluster_news,
        cluster_news_sources,
        after_market,
        after_market_source,
        after_market_observed_at,
        searches,
    })
}

async fn cluster_news_with_io(
    io: &mut impl ChainPreparationIo,
    cluster: &ChainCluster,
    concepts: &HashMap<String, Vec<String>>,
    observations: &RefCell<Vec<ModelCall>>,
    searches: &mut Vec<SearchObservation>,
) -> (String, SourceObservation) {
    if !io.search_available() {
        observations.borrow_mut().push(ModelCall::not_called(
            ModelStage::SearchTerms,
            Some(cluster.concept.clone()),
            "新闻搜索未配置，未生成检索词",
        ));
        return (
            String::new(),
            SourceObservation::unavailable("新闻搜索未配置".into()),
        );
    }
    let (mut queries, prompt) = super::fetchers::build_cluster_query_context(cluster, concepts);
    if let Ok(text) = (IoModel {
        io: &*io,
        observations,
        stage: ModelStage::SearchTerms,
        concept: Some(&cluster.concept),
    })
    .call_api_mode(
        &prompt,
        "你是A股题材挖掘专家，只输出新闻搜索词，每行一条。",
        AgentMode::Quick,
    )
    .await
    {
        super::fetchers::append_generated_cluster_queries(&mut queries, &text);
    }
    let (mut seen, mut items) = (HashSet::new(), Vec::new());
    let first_search = searches.len();
    for query in queries {
        let results = observed_search(io, SearchStage::Cluster, query, 4, 15, searches).await;
        super::fetchers::append_cluster_news_items(&mut seen, &mut items, results);
        if items.len() >= 10 {
            break;
        }
    }
    (
        items.join("\n"),
        aggregate_search_source(&searches[first_search..]),
    )
}

async fn after_market_with_io(
    io: &mut impl ChainPreparationIo,
    themes: &[&str],
    searches: &mut Vec<SearchObservation>,
) -> (String, SourceObservation, Option<String>) {
    if !io.search_available() {
        return (
            String::new(),
            SourceObservation::unavailable("新闻搜索未配置".into()),
            None,
        );
    }
    let now = io.local_now();
    let today = now.format("%m月%d日").to_string();
    use chrono::Timelike;
    let time_label = if now.hour() >= 15 { "盘后" } else { "盘中" };
    let mut items = Vec::new();
    let first_search = searches.len();
    for theme in themes.iter().take(5) {
        if items.len() >= 10 {
            break;
        }
        let query = format!("{today} {theme} 最新 突发 催化");
        let results = observed_search(io, SearchStage::AfterMarket, query, 2, 8, searches).await;
        super::fetchers::append_after_market_items(&mut items, theme, results);
    }
    (
        super::fetchers::render_after_market_section(&today, time_label, &items),
        aggregate_search_source(&searches[first_search..]),
        Some(now.to_rfc3339()),
    )
}

async fn observed_search(
    io: &mut impl ChainPreparationIo,
    stage: SearchStage,
    query: String,
    limit: usize,
    timeout_seconds: u64,
    searches: &mut Vec<SearchObservation>,
) -> Vec<SearchResult> {
    let (results, source) = match tokio::time::timeout(
        std::time::Duration::from_secs(timeout_seconds),
        io.search_topic(&query, limit),
    )
    .await
    {
        Ok(Ok(results)) => {
            // Empty Vec from the old search interface is not evidence of verified emptiness.
            let status = if results.is_empty() {
                SourceStatus::Unknown
            } else {
                SourceStatus::Available
            };
            (
                results,
                SourceObservation {
                    status,
                    ..SourceObservation::unknown()
                },
            )
        }
        Ok(Err(error)) => {
            log::warn!("[产业链] 搜索阶段 {:?} 失败，降级为空背景", stage);
            (
                Vec::new(),
                SourceObservation::unavailable(error.to_string()),
            )
        }
        Err(_) => {
            log::warn!("[产业链] 搜索阶段 {:?} 超时，降级为空背景", stage);
            (
                Vec::new(),
                SourceObservation::unavailable(format!("新闻搜索超时（{timeout_seconds}秒）")),
            )
        }
    };
    searches.push(SearchObservation {
        stage,
        query,
        limit,
        results: results.clone(),
        source,
    });
    results
}

fn aggregate_search_source(searches: &[SearchObservation]) -> SourceObservation {
    if searches.is_empty() {
        return SourceObservation {
            status: SourceStatus::NotRequested,
            ..SourceObservation::unknown()
        };
    }
    if searches
        .iter()
        .all(|s| s.source.status == SourceStatus::Unavailable)
    {
        return SourceObservation::unavailable(
            "本阶段全部实际搜索不可用；逐次原因见搜索观察".into(),
        );
    }
    SourceObservation {
        status: if searches.iter().any(|s| !s.results.is_empty()) {
            SourceStatus::Available
        } else {
            SourceStatus::Unknown
        },
        ..SourceObservation::unknown()
    }
}
