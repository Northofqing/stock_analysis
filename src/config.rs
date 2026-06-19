//! 热加载配置 — toml 文件读取 + 默认值 fallback。
//!
//! SIGHUP 信号触发 reload。toml 缺失或格式错误 → 用代码默认值，不崩溃。

use serde::Deserialize;
use std::sync::RwLock;

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
}

fn default_screener_interval() -> u64 { 30 }
fn default_opp_interval() -> u64 { 60 }
fn default_news_window_start_hour() -> u8 { 8 }
fn default_news_window_end_hour() -> u8 { 22 }

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            screener_interval_min: 30,
            opportunity_scan_interval_min: 60,
            news_window_start_hour: 8,
            news_window_end_hour: 22,
        }
    }
}

// ── 全局配置缓存 ──

static CHAIN_RULES: RwLock<Option<Vec<ChainRuleConfig>>> = RwLock::new(None);
static EXCLUSION_BOARDS: RwLock<Option<Vec<ExclusionBoardConfig>>> = RwLock::new(None);
static ANNOUNCE_KEYWORDS: RwLock<Option<AnnounceKeywordsFile>> = RwLock::new(None);
static MONITOR_CONFIG: RwLock<MonitorConfig> = RwLock::new(MonitorConfig {
    screener_interval_min: 30,
    opportunity_scan_interval_min: 60,
    news_window_start_hour: 8,
    news_window_end_hour: 22,
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
