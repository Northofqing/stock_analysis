//! 热加载配置 — toml 文件读取 + 默认值 fallback。
//!
//! SIGHUP 信号触发 reload。toml 缺失或格式错误 → 用代码默认值，不崩溃。

use serde::Deserialize;
use std::sync::{LazyLock, RwLock};

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
}

fn default_screener_interval() -> u64 { 30 }
fn default_opp_interval() -> u64 { 60 }
fn default_news_window_start_hour() -> u8 { 8 }
fn default_news_window_end_hour() -> u8 { 22 }
fn default_topic_search_intent_count() -> u8 { 6 }
fn default_topic_search_timeout_sec() -> u64 { 10 }
fn default_topic_mmr_relevance_weight() -> f32 { 0.72 }
fn default_topic_mmr_diversity_penalty() -> f32 { 2.2 }
fn default_topic_mmr_history_penalty() -> f32 { 1.4 }
fn default_topic_history_window_hours() -> u64 { 72 }
fn default_topic_history_memory_size() -> usize { 160 }
fn default_topic_history_db_limit() -> usize { 400 }
fn default_dq_quote_stale_sec() -> u64 { 5 }
fn default_dq_position_stale_sec() -> u64 { 30 }
fn default_dq_nav_stale_sec() -> u64 { 24 * 3600 }
fn default_dq_daily_stale_sec() -> u64 { 24 * 3600 }
fn default_opportunity_min_confidence() -> u8 { 55 }

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

fn default_true() -> bool { true }
fn default_veto_mode() -> String { "dry_run".to_string() }

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

fn default_air_refuel_entry_mode() -> String { "confirm".to_string() }
fn default_air_refuel_confirm_lots() -> u32 { 10 }
fn default_air_refuel_pilot_lots() -> u32 { 3 }

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

fn default_factor_action_normal() -> String { "normal".to_string() }
fn default_down_weight_scale() -> f64 { 0.5 }

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
            dq_quote_stale_sec: 5,
            dq_position_stale_sec: 30,
            dq_nav_stale_sec: 24 * 3600,
            dq_daily_stale_sec: 24 * 3600,
            opportunity_min_confidence: 55,
            opportunity_require_cross_source: false,
            live_veto: LiveVetoConfig::default(),
            position_sizing: PositionSizingConfig::default(),
            factor_feedback: FactorFeedbackConfig::default(),
            air_refuel: AirRefuelConfig::default(),
        }
    }
}

// ── 全局配置缓存 ──

static CHAIN_RULES: RwLock<Option<Vec<ChainRuleConfig>>> = RwLock::new(None);
static EXCLUSION_BOARDS: RwLock<Option<Vec<ExclusionBoardConfig>>> = RwLock::new(None);
static ANNOUNCE_KEYWORDS: RwLock<Option<AnnounceKeywordsFile>> = RwLock::new(None);
static MONITOR_CONFIG: LazyLock<RwLock<MonitorConfig>> = LazyLock::new(|| {
    RwLock::new(MonitorConfig::default())
});

/// 尝试加载所有 toml 配置。失败不崩溃，保留旧值。
pub fn load_all() {
    if let Ok(s) = std::fs::read_to_string("config/chain_rules.toml") {
        if let Ok(c) = toml::from_str::<ChainRulesFile>(&s) {
            *CHAIN_RULES.write().unwrap() = Some(c.rules);
        }
    }
    if let Ok(s) = std::fs::read_to_string("config/exclusion.toml") {
        if let Ok(c) = toml::from_str::<ExclusionFile>(&s) {
            *EXCLUSION_BOARDS.write().unwrap() = Some(c.boards);
        }
    }
    if let Ok(s) = std::fs::read_to_string("config/announce_keywords.toml") {
        if let Ok(c) = toml::from_str::<AnnounceKeywordsFile>(&s) {
            *ANNOUNCE_KEYWORDS.write().unwrap() = Some(c);
        }
    }
    if let Ok(s) = std::fs::read_to_string("config/monitor.toml") {
        if let Ok(c) = toml::from_str::<MonitorConfig>(&s) {
            *MONITOR_CONFIG.write().unwrap() = c;
        }
    }
}

/// 获取产业链规则（优先 toml，fallback 调用方提供的默认值）
pub fn get_chain_rules() -> Option<Vec<ChainRuleConfig>> {
    CHAIN_RULES.read().unwrap().clone()
}

/// 获取排除板块配置
pub fn get_exclusion_boards() -> Option<Vec<ExclusionBoardConfig>> {
    EXCLUSION_BOARDS.read().unwrap().clone()
}

/// 获取公告关键词配置
pub fn get_announce_keywords() -> Option<AnnounceKeywordsFile> {
    ANNOUNCE_KEYWORDS.read().unwrap().clone()
}

/// 获取监控定时器配置
pub fn get_monitor_config() -> MonitorConfig {
    MONITOR_CONFIG.read().unwrap().clone()
}

/// 获取 VetoChain 否决链配置
pub fn get_veto_config() -> LiveVetoConfig {
    MONITOR_CONFIG.read().unwrap().live_veto.clone()
}

/// 获取动态仓位配置
pub fn get_position_sizing_config() -> PositionSizingConfig {
    MONITOR_CONFIG.read().unwrap().position_sizing.clone()
}
