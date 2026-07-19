//! Registered business rules: BR-056.
//! 热加载配置 — toml 文件读取 + 默认值 fallback。
//!
//! SIGHUP 信号触发 reload。toml 缺失或格式错误 → 用代码默认值，不崩溃。

use serde::{Deserialize, Serialize};

/// v17.7 earnings classification config, loaded from [v17_7_sources.earnings] in strategy.toml.
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

// ── 公告关键词 ──

#[derive(Debug, Clone, Deserialize)]
pub struct AnnounceKeywordsFile {
    pub emergency: Vec<String>,
    pub important: Vec<String>,
    pub positive: Vec<String>,
}

// ── 监控定时器配置 ──

#[derive(Debug, Clone, Deserialize)]
pub struct MonitorConfig {
    #[serde(default = "default_screener_interval")]
    pub screener_interval_min: u64,
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
    #[serde(default = "default_opp_interval")]
    pub opportunity_scan_interval_min: u64,
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
    /// 产业链命中最小置信度（0-100），低于该值仅观察不参与机会推荐
    #[serde(default = "default_opportunity_min_confidence")]
    pub opportunity_min_confidence: u8,
    /// 是否强制要求快讯+Web双源共振
    #[serde(default)]
    pub opportunity_require_cross_source: bool,
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

fn default_screener_interval() -> u64 {
    30
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
fn default_opp_interval() -> u64 {
    60
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
fn default_opportunity_min_confidence() -> u8 {
    55
}
fn default_opportunity_push_threshold() -> u8 {
    75
}

// ── 实时否决链配置 (VetoChain) ──

/// VetoChain 配置，作为 `config/monitor.toml` 的 `[live_veto]` section。
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

/// 动态仓位配置，作为 `config/monitor.toml` 的 `[position_sizing]` section。
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

/// 因子反馈配置，作为 `config/monitor.toml` 的 `[factor_feedback]` section。
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

/// 空中加油执行配置，作为 `config/monitor.toml` 的 `[air_refuel]` section。
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
            screener_interval_min: 30,
            screener_min_score: 75,
            news_critical_score_threshold: 80,
            news_max_critical_per_day: 20,
            opportunity_scan_interval_min: 60,
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
            opportunity_min_confidence: 55,
            opportunity_require_cross_source: false,
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
type MonitorConfigSwap = arc_swap::ArcSwap<MonitorConfig>;

static CHAIN_RULES: LazyLock<ChainRulesSwap> =
    LazyLock::new(|| ChainRulesSwap::from(Arc::new(None)));
static EXCLUSION_BOARDS: LazyLock<ExclusionBoardsSwap> =
    LazyLock::new(|| ExclusionBoardsSwap::from(Arc::new(None)));
static ANNOUNCE_KEYWORDS: LazyLock<AnnounceKeywordsSwap> =
    LazyLock::new(|| AnnounceKeywordsSwap::from(Arc::new(None)));
static MONITOR_CONFIG: LazyLock<MonitorConfigSwap> =
    LazyLock::new(|| MonitorConfigSwap::from(Arc::new(MonitorConfig::default())));

// 修复 P3.1: 集中风险/费用常量
static RISK_CONFIG: LazyLock<RwLock<RiskConfig>> =
    LazyLock::new(|| RwLock::new(RiskConfig::default()));

/// 修复 P3.1: 集中风险/费用常量
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RiskConfig {
    pub trading: TradingConfig,
    pub slippage: SlippageConfig,
    pub performance: PerformanceConfig,
    pub regime: RegimeConfig,
    pub exposure: ExposureConfig,
    pub alert: AlertConfig,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradingConfig {
    pub commission_rate: f64,
    pub stamp_tax_rate: f64,
    pub slippage_rate: f64,
    pub min_commission: f64,
    pub lot_size: u64,
}
impl Default for TradingConfig {
    fn default() -> Self {
        Self {
            commission_rate: 0.0003,
            stamp_tax_rate: 0.001,
            slippage_rate: 0.001,
            min_commission: 5.0,
            lot_size: 100,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlippageConfig {
    pub dynamic_enabled: bool,
    pub alpha: f64,
    pub adv_days: u32,
}
impl Default for SlippageConfig {
    fn default() -> Self {
        Self {
            dynamic_enabled: false,
            alpha: 0.1,
            adv_days: 20,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceConfig {
    pub risk_free_rate: f64,
    pub trading_days_year: u32,
    pub sharpe_window: u32,
    pub sortino_min_period: u32,
}
impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            risk_free_rate: 0.03,
            trading_days_year: 252,
            sharpe_window: 60,
            sortino_min_period: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegimeConfig {
    pub window_days: u32,
    pub bull_threshold: f64,
    pub bear_threshold: f64,
    pub index_plunge_atr_mult: f64,
    pub flow_outflow_threshold: f64,
    pub flow_lookback_min: u32,
}
impl Default for RegimeConfig {
    fn default() -> Self {
        Self {
            window_days: 20,
            bull_threshold: 0.03,
            bear_threshold: -0.03,
            index_plunge_atr_mult: 2.0,
            flow_outflow_threshold: 0.5,
            flow_lookback_min: 15,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExposureConfig {
    pub single_stock_max: f64,
    pub single_sector_max: f64,
    pub cash_floor: f64,
    pub stop_loss_default: f64,
}
impl Default for ExposureConfig {
    fn default() -> Self {
        Self {
            single_stock_max: 0.10,
            single_sector_max: 0.40,
            cash_floor: 0.15,
            stop_loss_default: -0.10,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertConfig {
    pub min_importance_score: u8,
    pub min_emergency_score: u8,
    pub index_plunge_window_min: u32,
    pub stale_data_max_age_sec: u64,
}
impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            min_importance_score: 70,
            min_emergency_score: 85,
            index_plunge_window_min: 5,
            stale_data_max_age_sec: 30,
        }
    }
}

/// 修复 P3.1: 读取集中风险配置
pub fn get_risk_config() -> RiskConfig {
    RISK_CONFIG.read().unwrap().clone()
}

/// 加载 strategy.toml (整合 risk + monitor + opportunity)
///
/// v12: 3 文件 → 1 文件整合. 解析出 RiskConfig + MonitorConfig 两个子 struct.
fn parse_strategy_toml(content: &str) {
    if let Ok(c) = toml::from_str::<RiskConfig>(content) {
        *RISK_CONFIG.write().unwrap() = c;
    }
    if let Ok(c) = toml::from_str::<MonitorConfig>(content) {
        // review #14: ArcSwap 原子替换 (lock-free for readers).
        MONITOR_CONFIG.store(Arc::new(c));
    }
}

/// 加载 strategy.toml. 失败不崩溃, 保留 const fallback.
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
            "[v12-config] config/strategy.toml 读取失败: {} (用 const fallback)",
            e
        ),
    }
}

/// 加载 chain.toml (整合 chain_rules + announce_keywords + exclusion)
///
/// 3 文件 → 1 文件. 用 toml::from_str 独立 parse 三种 schema.
fn load_chain_combined() {
    let content = match std::fs::read_to_string("config/chain.toml") {
        Ok(c) => {
            log::debug!("[v12-config] 加载 config/chain.toml ({} bytes)", c.len());
            c
        }
        Err(e) => {
            log::warn!(
                "[v12-config] config/chain.toml 读取失败: {} (用 const fallback)",
                e
            );
            return;
        }
    };
    if let Ok(c) = toml::from_str::<ChainRulesFile>(&content) {
        // review #14: ArcSwap store 是 atomic 替换, 不阻塞读.
        CHAIN_RULES.store(Arc::new(Some(c.rules)));
    }
    if let Ok(c) = toml::from_str::<AnnounceKeywordsFile>(&content) {
        ANNOUNCE_KEYWORDS.store(Arc::new(Some(c)));
    }
    if let Ok(c) = toml::from_str::<ExclusionFile>(&content) {
        EXCLUSION_BOARDS.store(Arc::new(Some(c.boards)));
    }
}

/// 兼容老 API: 加载 risk 配置 (内部调 load_strategy_config)
pub fn load_risk_config() {
    load_strategy_config();
}

/// 尝试加载所有 toml 配置。失败不崩溃，保留旧值。
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
