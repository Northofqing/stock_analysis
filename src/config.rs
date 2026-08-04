//! Registered business rules: BR-056, BR-138, BR-160, BR-181.
//! 启动配置 — `load_all()` 单次读取 strategy.toml 与 chain.toml。
//!
//! 公告关键词是生产推送选择合同，缺失或格式错误时按 BR-138 显式不可用；
//! 产业链规则只来自成功激活的内存快照，不从消费路径重读或回退。

use serde::{Deserialize, Serialize};

/// v17.7 earnings classification config, loaded from `[v17_7_earnings]` in strategy.toml.
#[derive(Debug, Clone, Deserialize)]
pub struct EarningsConfig {
    #[serde(default = "default_earnings_metric")]
    pub metric: String,
    #[serde(default = "default_beat_threshold")]
    pub beat_threshold_pct: f64,
    #[serde(default = "default_miss_threshold")]
    pub miss_threshold_pct: f64,
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
}

fn default_earnings_metric() -> String {
    "eps".to_string()
}
fn default_beat_threshold() -> f64 {
    10.0
}
fn default_miss_threshold() -> f64 {
    -10.0
}
fn default_poll_interval() -> u64 {
    900
}

impl Default for EarningsConfig {
    fn default() -> Self {
        Self {
            metric: default_earnings_metric(),
            beat_threshold_pct: default_beat_threshold(),
            miss_threshold_pct: default_miss_threshold(),
            poll_interval_secs: default_poll_interval(),
        }
    }
}

impl EarningsConfig {
    /// Validate that thresholds are correctly signed.
    pub fn validate(&self) -> Result<(), String> {
        if !self.beat_threshold_pct.is_finite() || self.beat_threshold_pct <= 0.0 {
            return Err(format!(
                "beat_threshold_pct must be finite and > 0, got {}",
                self.beat_threshold_pct
            ));
        }
        if !self.miss_threshold_pct.is_finite() || self.miss_threshold_pct >= 0.0 {
            return Err(format!(
                "miss_threshold_pct must be finite and < 0, got {}",
                self.miss_threshold_pct
            ));
        }
        Ok(())
    }
}
use std::sync::{Arc, LazyLock, RwLock};

// ── 产业链规则 ──

#[derive(Debug, Clone, Deserialize)]
pub struct ChainRuleConfig {
    pub chain: String,
    pub logic: String,
    pub board_keyword: String,
    pub keywords: Vec<String>,
    /// 优先级 (0-100)，越大越优先匹配。具体规则应高于宽泛规则。toml 缺失时默认 0。
    #[serde(default)]
    pub priority: u32,
    /// 大类分组，如 "AI硬件"、"半导体"、"新能源"。toml 缺失时默认空。
    #[serde(default)]
    pub category: String,
    /// 是否为通用规则：当仅命中该类规则时，可触发 AI 二次分类验证。
    #[serde(default)]
    pub generic: bool,
    /// 是否启用：false 时 chain_mapper 在规则加载时跳过该 entry。
    /// BR-006: 基于真实胜率 (0%) 关停某些主题, 防止它们继续产生低质推送。
    /// toml 缺失时默认 true (向后兼容)。
    #[serde(default = "default_chain_rule_enabled")]
    pub enabled: bool,
}

fn default_chain_rule_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChainRulesFile {
    pub rules: Vec<ChainRuleConfig>,
}

// ── 排除板块 ──

#[derive(Debug, Clone, Deserialize)]
pub struct ExclusionBoardConfig {
    pub name: String,
    pub reason: String,
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExclusionFile {
    pub boards: Vec<ExclusionBoardConfig>,
}

/// BR-160 versioned A-10 deterministic clustering contract.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ChainIntelligenceConfig {
    pub calculation_version: String,
    pub taxonomy_version: String,
    pub min_members: usize,
    pub excluded_board_names: Vec<String>,
}

impl ChainIntelligenceConfig {
    pub fn validate(&self) -> Result<(), String> {
        for (field, value) in [
            ("calculation_version", self.calculation_version.as_str()),
            ("taxonomy_version", self.taxonomy_version.as_str()),
        ] {
            if value.trim().is_empty() || value.chars().any(char::is_control) {
                return Err(format!(
                    "BR-160 chain_intelligence.{field} must be non-empty canonical text"
                ));
            }
        }
        if !(3..=100).contains(&self.min_members) {
            return Err(format!(
                "BR-160 chain_intelligence.min_members must be within 3..=100, got {}",
                self.min_members
            ));
        }
        let mut names = std::collections::BTreeSet::new();
        for name in &self.excluded_board_names {
            let trimmed = name.trim();
            if trimmed.is_empty()
                || trimmed.chars().any(char::is_control)
                || !names.insert(trimmed.to_owned())
            {
                return Err(format!(
                    "BR-160 chain_intelligence.excluded_board_names contains invalid/duplicate value {name:?}"
                ));
            }
        }
        Ok(())
    }
}

// ── 公告关键词 ──

#[derive(Debug, Clone, Deserialize)]
pub struct AnnounceKeywordsFile {
    pub emergency: Vec<String>,
    pub important: Vec<String>,
    pub positive: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CombinedChainConfig {
    rules: Vec<ChainRuleConfig>,
    chain_intelligence: ChainIntelligenceConfig,
    announce_keywords: AnnounceKeywordsFile,
    boards: Vec<ExclusionBoardConfig>,
}

fn parse_chain_combined(content: &str) -> Result<CombinedChainConfig, toml::de::Error> {
    toml::from_str(content)
}

// ── 监控定时器配置 ──

#[derive(Debug, Clone, Deserialize)]
pub struct MonitorConfig {
    /// v17.4 §5.3.2 (D 方案): 选股推荐最低置信度, 低于该值静默 (info log 出声)
    /// Threshold-Proof (红线 2.9): 与 docs/v17.x/v17.4-news-and-review.md §5.3.2/§6 互为引用
    #[serde(default = "default_screener_min_score")]
    pub screener_min_score: u8,
    /// v17.4 §5.1 (BR-033): 新闻 critical 即时推强度阈值 (默认 80, 与 spec §6 互引)
    #[serde(default = "default_news_critical_score_threshold")]
    pub news_critical_score_threshold: u8,
    /// v17.4 §5.1 (BR-033): critical 每日上限 (防刷屏, 默认 20, 超限 warn 出声)
    #[serde(default = "default_news_max_critical_per_day")]
    pub news_max_critical_per_day: u32,
    #[serde(default = "default_news_window_start_hour")]
    pub news_window_start_hour: u8,
    #[serde(default = "default_news_window_end_hour")]
    pub news_window_end_hour: u8,
    #[serde(default = "default_topic_search_intent_count")]
    pub topic_search_intent_count: u8,
    #[serde(default = "default_topic_search_timeout_sec")]
    pub topic_search_timeout_sec: u64,
    #[serde(default = "default_topic_mmr_relevance_weight")]
    pub topic_mmr_relevance_weight: f32,
    #[serde(default = "default_topic_mmr_diversity_penalty")]
    pub topic_mmr_diversity_penalty: f32,
    #[serde(default = "default_topic_mmr_history_penalty")]
    pub topic_mmr_history_penalty: f32,
    #[serde(default = "default_topic_history_window_hours")]
    pub topic_history_window_hours: u64,
    #[serde(default = "default_topic_history_memory_size")]
    pub topic_history_memory_size: usize,
    #[serde(default = "default_topic_history_db_limit")]
    pub topic_history_db_limit: usize,
    /// 主题/Web 搜索新闻的新鲜度窗口（天）：超过该阈值且能解析出发布日期的旧闻被丢弃（AGENTS.md §2.4）
    #[serde(default = "default_topic_news_max_age_days")]
    pub topic_news_max_age_days: i64,
    #[serde(default = "default_dq_quote_stale_sec")]
    pub dq_quote_stale_sec: u64,
    #[serde(default = "default_dq_position_stale_sec")]
    pub dq_position_stale_sec: u64,
    #[serde(default = "default_dq_nav_stale_sec")]
    pub dq_nav_stale_sec: u64,
    #[serde(default = "default_dq_daily_stale_sec")]
    pub dq_daily_stale_sec: u64,
    /// 修复 v9.1 §0 NS3: dual_score.event_risk_score 推送阈值
    /// 实际推送的最低 event_risk_score, 默认 75
    /// 60-74 入候选池 (供复盘), 75+ 实时推送, <60 不推
    #[serde(default = "default_opportunity_push_threshold")]
    pub opportunity_push_threshold: u8,
    /// 修复 v9.1: 启用 v9.1 dual_score 评分门 (替代 ad-hoc score_hit_confidence)
    /// false = 用 legacy score_hit_confidence (默认, 向后兼容)
    /// true = 用 dual_score.event_risk_score (新评分模型, 更严谨)
    #[serde(default)]
    pub opportunity_use_dual_score: bool,
    /// VetoChain 否决链配置 (可选 section [live_veto])
    #[serde(default)]
    pub live_veto: LiveVetoConfig,
    /// 动态仓位配置 (可选 section [position_sizing])
    #[serde(default)]
    pub position_sizing: PositionSizingConfig,
    /// IC 反馈到排序评分配置（可选 section [factor_feedback]）
    #[serde(default)]
    pub factor_feedback: FactorFeedbackConfig,
    /// 空中加油执行配置（可选 section [air_refuel]）
    #[serde(default)]
    pub air_refuel: AirRefuelConfig,
    /// v17.7 earnings classification config.
    #[serde(default)]
    pub v17_7_earnings: EarningsConfig,
}

fn default_screener_min_score() -> u8 {
    75
}
fn default_news_critical_score_threshold() -> u8 {
    80
}
fn default_news_max_critical_per_day() -> u32 {
    20
}
fn default_news_window_start_hour() -> u8 {
    8
}
fn default_news_window_end_hour() -> u8 {
    22
}
fn default_topic_search_intent_count() -> u8 {
    6
}
fn default_topic_search_timeout_sec() -> u64 {
    10
}
fn default_topic_mmr_relevance_weight() -> f32 {
    0.72
}
fn default_topic_mmr_diversity_penalty() -> f32 {
    2.2
}
fn default_topic_mmr_history_penalty() -> f32 {
    1.4
}
fn default_topic_history_window_hours() -> u64 {
    72
}
fn default_topic_history_memory_size() -> usize {
    160
}
fn default_topic_history_db_limit() -> usize {
    400
}
fn default_topic_news_max_age_days() -> i64 {
    7
}
fn default_dq_quote_stale_sec() -> u64 {
    5
}
fn default_dq_position_stale_sec() -> u64 {
    30
}
fn default_dq_nav_stale_sec() -> u64 {
    24 * 3600
}
fn default_dq_daily_stale_sec() -> u64 {
    24 * 3600
}
fn default_opportunity_push_threshold() -> u8 {
    75
}

// ── 实时否决链配置 (VetoChain) ──

/// VetoChain 配置，作为 `config/strategy.toml` 的 `[live_veto]` section。
#[derive(Debug, Clone, Deserialize)]
pub struct LiveVetoConfig {
    /// 总开关
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 运行模式: "dry_run" | "live"
    #[serde(default = "default_veto_mode")]
    pub mode: String,
    /// 乖离率拦截
    #[serde(default = "default_true")]
    pub bias_rate_enabled: bool,
    /// 空头排列拦截
    #[serde(default = "default_true")]
    pub bearish_alignment_enabled: bool,
    /// 主力资金拦截
    #[serde(default = "default_true")]
    pub main_flow_enabled: bool,
    /// 基本面恶化拦截
    #[serde(default = "default_true")]
    pub fundamental_enabled: bool,
}

fn default_true() -> bool {
    true
}
fn default_veto_mode() -> String {
    "dry_run".to_string()
}

impl Default for LiveVetoConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: "dry_run".to_string(),
            bias_rate_enabled: true,
            bearish_alignment_enabled: true,
            main_flow_enabled: true,
            fundamental_enabled: true,
        }
    }
}

// ── 动态仓位配置 (PositionSizing) ──

/// 动态仓位配置，作为 `config/strategy.toml` 的 `[position_sizing]` section。
#[derive(Debug, Clone, Deserialize)]
pub struct PositionSizingConfig {
    /// 是否启用动态仓位 (false = 回退到旧 position_shares)
    #[serde(default = "default_true")]
    pub use_dynamic: bool,
}

impl Default for PositionSizingConfig {
    fn default() -> Self {
        Self { use_dynamic: true }
    }
}

// ── 因子 IC 反馈配置（仅影响排序/展示，不影响买入触发） ──

/// 因子反馈配置，作为 `config/strategy.toml` 的 `[factor_feedback]` section。
///
/// action 取值：
/// - normal: 保持原值
/// - disable: 维度禁用（权重=0）
/// - invert: 维度反转（score -> 100-score）
/// - down_weight: 维度降权（乘以 down_weight_scale）
#[derive(Debug, Clone, Deserialize)]
pub struct FactorFeedbackConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_factor_action_normal")]
    pub technical_action: String,
    #[serde(default = "default_factor_action_normal")]
    pub quality_action: String,
    #[serde(default = "default_factor_action_normal")]
    pub valuation_action: String,
    #[serde(default = "default_factor_action_normal")]
    pub flow_action: String,
    #[serde(default = "default_factor_action_normal")]
    pub growth_action: String,
    #[serde(default = "default_down_weight_scale")]
    pub down_weight_scale: f64,
}

// ── 空中加油执行配置 ──

/// 空中加油执行配置，作为 `config/strategy.toml` 的 `[air_refuel]` section。
///
/// entry_mode 取值：
/// - confirm: 次日早盘确认弱转强后再记录虚拟观察仓（默认）
/// - pilot: 整盘日尾盘/竞价先潜伏记录虚拟观察仓
#[derive(Debug, Clone, Deserialize)]
pub struct AirRefuelConfig {
    #[serde(default = "default_air_refuel_entry_mode")]
    pub entry_mode: String,
    #[serde(default = "default_air_refuel_confirm_lots")]
    pub confirm_lots: u32,
    #[serde(default = "default_air_refuel_pilot_lots")]
    pub pilot_lots: u32,
    #[serde(default = "default_true")]
    pub next_day_review_enabled: bool,
}

fn default_air_refuel_entry_mode() -> String {
    "confirm".to_string()
}
fn default_air_refuel_confirm_lots() -> u32 {
    10
}
fn default_air_refuel_pilot_lots() -> u32 {
    3
}

impl Default for AirRefuelConfig {
    fn default() -> Self {
        Self {
            entry_mode: "confirm".to_string(),
            confirm_lots: 10,
            pilot_lots: 3,
            next_day_review_enabled: true,
        }
    }
}

fn default_factor_action_normal() -> String {
    "normal".to_string()
}
fn default_down_weight_scale() -> f64 {
    0.5
}

impl Default for FactorFeedbackConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            technical_action: "normal".to_string(),
            quality_action: "normal".to_string(),
            valuation_action: "normal".to_string(),
            flow_action: "normal".to_string(),
            growth_action: "normal".to_string(),
            down_weight_scale: 0.5,
        }
    }
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            screener_min_score: 75,
            news_critical_score_threshold: 80,
            news_max_critical_per_day: 20,
            news_window_start_hour: 8,
            news_window_end_hour: 22,
            topic_search_intent_count: 6,
            topic_search_timeout_sec: 10,
            topic_mmr_relevance_weight: 0.72,
            topic_mmr_diversity_penalty: 2.2,
            topic_mmr_history_penalty: 1.4,
            topic_history_window_hours: 72,
            topic_history_memory_size: 160,
            topic_history_db_limit: 400,
            topic_news_max_age_days: 7,
            dq_quote_stale_sec: 5,
            dq_position_stale_sec: 30,
            dq_nav_stale_sec: 24 * 3600,
            dq_daily_stale_sec: 24 * 3600,
            opportunity_push_threshold: 75,
            opportunity_use_dual_score: false,
            live_veto: LiveVetoConfig::default(),
            position_sizing: PositionSizingConfig::default(),
            factor_feedback: FactorFeedbackConfig::default(),
            air_refuel: AirRefuelConfig::default(),
            v17_7_earnings: EarningsConfig::default(),
        }
    }
}

// ── 全局配置缓存 ──
// review #14: 原 RwLock<Option<Vec<T>>> + .read().clone() 热路径触发 RwLock read + 整 Vec clone.
// 改 ArcSwap: 内部类型是 T (不是 Arc<T>), ArcSwap::load_full() 自动返回 Arc<T> 共享引用.
// store() / from() 都要求 Arc<T>, 但内部 T 是普通值, ArcSwap 内部会做 Arc wrap.
type ChainRulesSwap = arc_swap::ArcSwap<Option<Vec<ChainRuleConfig>>>;
type ExclusionBoardsSwap = arc_swap::ArcSwap<Option<Vec<ExclusionBoardConfig>>>;
type AnnounceKeywordsSwap = arc_swap::ArcSwap<Option<AnnounceKeywordsFile>>;
type ChainIntelligenceSwap = arc_swap::ArcSwap<Option<ChainIntelligenceConfig>>;
type MonitorConfigSwap = arc_swap::ArcSwap<MonitorConfig>;

static CHAIN_RULES: LazyLock<ChainRulesSwap> =
    LazyLock::new(|| ChainRulesSwap::from(Arc::new(None)));
static EXCLUSION_BOARDS: LazyLock<ExclusionBoardsSwap> =
    LazyLock::new(|| ExclusionBoardsSwap::from(Arc::new(None)));
static ANNOUNCE_KEYWORDS: LazyLock<AnnounceKeywordsSwap> =
    LazyLock::new(|| AnnounceKeywordsSwap::from(Arc::new(None)));
static CHAIN_INTELLIGENCE: LazyLock<ChainIntelligenceSwap> =
    LazyLock::new(|| ChainIntelligenceSwap::from(Arc::new(None)));
static MONITOR_CONFIG: LazyLock<MonitorConfigSwap> =
    LazyLock::new(|| MonitorConfigSwap::from(Arc::new(MonitorConfig::default())));

// 当前仅保留有生产消费者的账户模式阈值。
static RISK_CONFIG: LazyLock<RwLock<RiskConfig>> =
    LazyLock::new(|| RwLock::new(RiskConfig::default()));

#[cfg(test)]
static CHAIN_RULES_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) struct ChainRulesTestGuard {
    previous: Option<Vec<ChainRuleConfig>>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl Drop for ChainRulesTestGuard {
    fn drop(&mut self) {
        CHAIN_RULES.store(Arc::new(self.previous.take()));
    }
}

#[cfg(test)]
pub(crate) fn replace_chain_rules_for_test(
    rules: Option<Vec<ChainRuleConfig>>,
) -> ChainRulesTestGuard {
    let lock = CHAIN_RULES_TEST_LOCK
        .lock()
        .expect("chain-rule test snapshot lock poisoned");
    let previous = (*CHAIN_RULES.load_full()).clone();
    CHAIN_RULES.store(Arc::new(rules));
    ChainRulesTestGuard {
        previous,
        _lock: lock,
    }
}

/// `strategy.toml` 中仍由生产路径消费的风险配置。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RiskConfig {
    /// v12 PR1: 账户模式三态判定阈值 (BR-021)
    #[serde(default)]
    pub account_mode: AccountModeConfig,
}

/// v12 PR1-1.4: 账户模式阈值配置 (对齐 `risk::account_mode::thresholds` const fallback)
///
/// 缺 toml 段时 serde(default) 走 Default 实现, 对应 code-level const.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountModeConfig {
    /// 当日累计亏损 ≤ 此值触发 ReduceOnly (默认 -1.5)
    #[serde(default = "default_daily_loss_pct")]
    pub daily_loss_pct: f64,
    /// 当日累计亏损 ≤ 此值触发 Frozen (默认 -2.0)
    #[serde(default = "default_circuit_breaker_pct")]
    pub circuit_breaker_pct: f64,
    /// 连续止损笔数 ≥ 此值触发 ReduceOnly (默认 3)
    #[serde(default = "default_consecutive_n")]
    pub consecutive_stop_loss_n: u32,
    /// 总仓位 > 此值触发 Frozen (默认 8 成)
    #[serde(default = "default_position_overload")]
    pub position_overload_cheng: u8,
}

fn default_daily_loss_pct() -> f64 {
    -1.5
}
fn default_circuit_breaker_pct() -> f64 {
    -2.0
}
fn default_consecutive_n() -> u32 {
    3
}
fn default_position_overload() -> u8 {
    8
}

impl Default for AccountModeConfig {
    fn default() -> Self {
        Self {
            daily_loss_pct: default_daily_loss_pct(),
            circuit_breaker_pct: default_circuit_breaker_pct(),
            consecutive_stop_loss_n: default_consecutive_n(),
            position_overload_cheng: default_position_overload(),
        }
    }
}

impl AccountModeConfig {
    /// 转 `risk::account_mode::ModeThresholds` (PR1-1.3 评估用)
    pub fn to_thresholds(&self) -> crate::risk::account_mode::ModeThresholds {
        crate::risk::account_mode::ModeThresholds {
            daily_loss_pct: self.daily_loss_pct,
            circuit_breaker_pct: self.circuit_breaker_pct,
            consecutive_stop_loss_n: self.consecutive_stop_loss_n,
            position_overload_cheng: self.position_overload_cheng,
        }
    }
}

/// 读取仍由生产路径使用的风险配置。
pub fn get_risk_config() -> RiskConfig {
    RISK_CONFIG.read().unwrap().clone()
}

/// 加载 strategy.toml (整合 risk + monitor + opportunity)
///
/// v12: 3 文件 → 1 文件整合. 解析出 RiskConfig + MonitorConfig 两个子 struct.
fn parse_strategy_projections(
    content: &str,
) -> (
    Result<RiskConfig, toml::de::Error>,
    Result<MonitorConfig, toml::de::Error>,
) {
    (
        toml::from_str::<RiskConfig>(content),
        toml::from_str::<MonitorConfig>(content),
    )
}

fn parse_strategy_toml(content: &str) {
    let (risk, monitor) = parse_strategy_projections(content);
    match risk {
        Ok(config) => *RISK_CONFIG.write().unwrap() = config,
        Err(error) => log::warn!(
            "[v12-config][BR-181] strategy.toml RiskConfig projection rejected; retaining previous/default value: {error}"
        ),
    }
    match monitor {
        Ok(config) => {
            // review #14: ArcSwap 原子替换 (lock-free for readers).
            MONITOR_CONFIG.store(Arc::new(config));
        }
        Err(error) => log::warn!(
            "[v12-config][BR-181] strategy.toml MonitorConfig projection rejected; retaining previous/default value: {error}"
        ),
    }
}

/// 加载 strategy.toml；读取失败时保留各投影先前值（首次为默认值）。
fn load_strategy_config() {
    match std::fs::read_to_string("config/strategy.toml") {
        Ok(content) => {
            log::debug!(
                "[v12-config] 加载 config/strategy.toml ({} bytes)",
                content.len()
            );
            parse_strategy_toml(&content);
        }
        Err(e) => log::warn!(
            "[v12-config][BR-181] config/strategy.toml 读取失败: {} (保留各投影先前值/默认值)",
            e
        ),
    }
}

/// 加载 chain.toml (整合 chain_rules + announce_keywords + exclusion)
///
/// 3 文件 → 1 文件. BR-138 要求一次性解析真实嵌套结构，禁止公告段失败后
/// 静默保留更宽泛的编译期词表。
fn load_chain_combined() {
    let content = match std::fs::read_to_string("config/chain.toml") {
        Ok(c) => {
            log::debug!("[v12-config] 加载 config/chain.toml ({} bytes)", c.len());
            c
        }
        Err(e) => {
            ANNOUNCE_KEYWORDS.store(Arc::new(None));
            CHAIN_INTELLIGENCE.store(Arc::new(None));
            log::error!(
                "[v12-config][BR-138][BR-181] config/chain.toml 读取失败: {e}; \
                 announce/intelligence unavailable, rules/boards retain previous snapshot \
                 (unavailable on first startup)"
            );
            return;
        }
    };
    let combined = match parse_chain_combined(&content) {
        Ok(combined) => combined,
        Err(error) => {
            ANNOUNCE_KEYWORDS.store(Arc::new(None));
            CHAIN_INTELLIGENCE.store(Arc::new(None));
            log::error!(
                "[v12-config][BR-138][BR-181] config/chain.toml 结构非法: {error}; \
                 announce/intelligence unavailable, rules/boards retain previous snapshot \
                 (unavailable on first startup)"
            );
            return;
        }
    };
    if let Err(error) = combined.chain_intelligence.validate() {
        ANNOUNCE_KEYWORDS.store(Arc::new(None));
        CHAIN_INTELLIGENCE.store(Arc::new(None));
        log::error!(
            "[v12-config][BR-160][BR-181] config/chain.toml 合同非法: {error}; \
             announce/intelligence unavailable, rules/boards retain previous snapshot \
             (unavailable on first startup)"
        );
        return;
    }
    // review #14: ArcSwap store 是 atomic 替换, 不阻塞读.
    activate_chain_rules_snapshot(combined.rules);
    ANNOUNCE_KEYWORDS.store(Arc::new(Some(combined.announce_keywords)));
    EXCLUSION_BOARDS.store(Arc::new(Some(combined.boards)));
    CHAIN_INTELLIGENCE.store(Arc::new(Some(combined.chain_intelligence)));
}

/// Activates the typed chain-rule projection already parsed by the startup
/// configuration owner. It does not read files or provide fallback data.
///
/// This narrow activation seam is public so integration tests can install an
/// explicit in-memory snapshot without relying on repository files at runtime.
#[doc(hidden)]
pub fn activate_chain_rules_snapshot(rules: Vec<ChainRuleConfig>) {
    CHAIN_RULES.store(Arc::new(Some(rules)));
}

/// 兼容老 API: 加载 risk 配置 (内部调 load_strategy_config)。
#[deprecated(
    since = "0.1.0",
    note = "use load_all(); this compatibility wrapper may be removed in a future release"
)]
pub fn load_risk_config() {
    load_strategy_config();
}

/// 启动时加载所有 runtime TOML 配置。失败语义见 BR-181。
///
/// v12 整合: 2 个文件 (strategy.toml + chain.toml) 替代原 6 个
pub fn load_all() {
    load_strategy_config();
    load_chain_combined();
}

/// 获取产业链规则 (review #14: ArcSwap 引用, 0 clone).
/// 返回 Arc<Vec<...>> 让调用方共享同一份内存. 热路径 (chain_mapper) 用 .as_slice() 或 .iter().
pub fn get_chain_rules() -> Option<Arc<Vec<ChainRuleConfig>>> {
    (*CHAIN_RULES.load_full()).clone().map(Arc::new)
}

/// 获取排除板块配置 (review #14: ArcSwap 引用, 0 clone).
pub fn get_exclusion_boards() -> Option<Arc<Vec<ExclusionBoardConfig>>> {
    (*EXCLUSION_BOARDS.load_full()).clone().map(Arc::new)
}

/// 获取公告关键词配置 (review #14: ArcSwap 引用, 0 clone).
pub fn get_announce_keywords() -> Option<Arc<AnnounceKeywordsFile>> {
    (*ANNOUNCE_KEYWORDS.load_full()).clone().map(Arc::new)
}

/// Returns the versioned BR-160 clustering contract. Absence is explicit and
/// disables A-10; no compile-time threshold fallback is permitted.
pub fn get_chain_intelligence_config() -> Option<Arc<ChainIntelligenceConfig>> {
    (*CHAIN_INTELLIGENCE.load_full()).clone().map(Arc::new)
}

/// BR-138: 公告生产循环只在显式加载完整关键词合同时运行。
pub fn announcement_keywords_available() -> bool {
    ANNOUNCE_KEYWORDS.load().is_some()
}

/// 获取监控定时器配置
// review #14: get_monitor_config 改返回 Arc<MonitorConfig>, 调用方共享同一份内存,
// 改 6 字段 String clone (200B alloc) 为 0 alloc. 调用方通过 .as_ref() 拿 &MonitorConfig.
/// 获取 MonitorConfig (Arc 引用, 0 clone).
pub fn get_monitor_config() -> Arc<MonitorConfig> {
    MONITOR_CONFIG.load_full()
}

/// 获取 VetoChain 否决链配置 (review #14: 走 Arc 引用, 不再 deep clone 整个 LiveVetoConfig).
pub fn get_veto_config() -> Arc<LiveVetoConfig> {
    Arc::new(MONITOR_CONFIG.load_full().live_veto.clone())
}

/// 获取动态仓位配置.
pub fn get_position_sizing_config() -> Arc<PositionSizingConfig> {
    Arc::new(MONITOR_CONFIG.load_full().position_sizing.clone())
}

#[cfg(test)]
mod chain_combined_tests {
    use super::*;

    #[test]
    fn br138_repository_chain_config_exposes_announcement_section() {
        let content = include_str!("../config/chain.toml");
        let parsed = parse_chain_combined(content).expect("valid combined chain config");
        assert!(parsed
            .announce_keywords
            .emergency
            .contains(&"立案调查".to_string()));
        assert!(parsed
            .announce_keywords
            .important
            .contains(&"股东减持".to_string()));
        assert!(parsed
            .announce_keywords
            .positive
            .contains(&"中标".to_string()));
        parsed
            .chain_intelligence
            .validate()
            .expect("valid BR-160 chain intelligence contract");
        assert_eq!(
            parsed.chain_intelligence.calculation_version,
            "chain-intelligence-v2"
        );
        assert_eq!(parsed.chain_intelligence.min_members, 3);
        assert!(parse_chain_combined("rules = []\nboards = []\n").is_err());
    }

    #[test]
    fn br155_repository_chain_ids_are_unique() {
        let content = include_str!("../config/chain.toml");
        let parsed = parse_chain_combined(content).expect("valid combined chain config");
        let mut seen = std::collections::HashSet::new();
        for rule in parsed.rules {
            let chain_id = rule.chain.trim();
            assert!(
                seen.insert(chain_id.to_owned()),
                "duplicate production chain ID: {chain_id}"
            );
        }
    }
}

#[cfg(test)]
mod strategy_config_tests {
    use super::*;

    #[test]
    fn strategy_projections_parse_independently() {
        let (risk, monitor) = parse_strategy_projections(
            r#"
news_window_start_hour = 17
[account_mode]
daily_loss_pct = "invalid"
"#,
        );
        assert!(risk.is_err());
        assert_eq!(
            monitor
                .expect("monitor projection must ignore malformed risk-only fields")
                .news_window_start_hour,
            17
        );

        let (risk, monitor) = parse_strategy_projections(
            r#"
news_window_start_hour = "invalid"
[account_mode]
daily_loss_pct = -1.25
"#,
        );
        assert_eq!(
            risk.expect("risk projection must ignore malformed monitor-only fields")
                .account_mode
                .daily_loss_pct,
            -1.25
        );
        assert!(monitor.is_err());
    }

    #[test]
    fn live_veto_missing_mode_defaults_to_dry_run_but_repository_selects_live() {
        let missing_mode: MonitorConfig = toml::from_str(
            r#"
[live_veto]
enabled = true
"#,
        )
        .expect("missing mode uses fail-safe default");
        assert_eq!(missing_mode.live_veto.mode, "dry_run");

        let repository: MonitorConfig = toml::from_str(include_str!("../config/strategy.toml"))
            .expect("repository strategy config must parse");
        assert_eq!(repository.live_veto.mode, "live");
    }

    #[test]
    fn v17_7_earnings_direct_table_parses_non_default_values() {
        let parsed: MonitorConfig = toml::from_str(
            r#"
[v17_7_earnings]
metric = "revenue"
beat_threshold_pct = 12.5
miss_threshold_pct = -7.25
poll_interval_secs = 321
"#,
        )
        .expect("direct v17_7_earnings table must parse");

        assert_eq!(parsed.v17_7_earnings.metric, "revenue");
        assert_eq!(parsed.v17_7_earnings.beat_threshold_pct, 12.5);
        assert_eq!(parsed.v17_7_earnings.miss_threshold_pct, -7.25);
        assert_eq!(parsed.v17_7_earnings.poll_interval_secs, 321);
        parsed
            .v17_7_earnings
            .validate()
            .expect("non-default values satisfy the classifier contract");
    }
}
