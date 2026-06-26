//! 依赖注入容器 — 逐步替代分散的全局单例。
//!
//! AppContext 在 main.rs 初始化后注入到各模块。
//! 渐进迁移策略：新增代码通过 AppContext 访问服务；
//! 旧代码仍通过全局 OnceCell 访问，逐步迁移。

use std::sync::Arc;
use anyhow::Result;
use stock_analysis::analyzer::GeminiAnalyzer;
use stock_analysis::config;
use stock_analysis::pipeline::PipelineConfig;

/// 应用级依赖容器。
pub struct AppContext {
    pub ai_analyzer: Option<Arc<GeminiAnalyzer>>,
    pub pipeline_config: PipelineConfig,
}

impl AppContext {
    /// 初始化应用上下文。数据库和搜索服务仍通过全局单例访问（渐进迁移）。
    pub fn init(pipeline_config: PipelineConfig) -> Result<Self> {
        let ai_analyzer = std::env::var("GEMINI_API_KEY").ok()
            .filter(|k| !k.is_empty())
            .map(|_| Arc::new(GeminiAnalyzer::from_env()));

        Ok(Self {
            ai_analyzer,
            pipeline_config,
        })
    }

    /// 从环境变量和 CLI 参数构建默认 PipelineConfig
    pub fn default_pipeline_config(max_workers: usize) -> PipelineConfig {
        let monitor_cfg = config::get_monitor_config();
        PipelineConfig {
            max_workers,
            dry_run: false,
            send_notification: true,
            single_notify: false,
            dq_quote_stale_sec: monitor_cfg.dq_quote_stale_sec,
            dq_position_stale_sec: monitor_cfg.dq_position_stale_sec,
            dq_nav_stale_sec: monitor_cfg.dq_nav_stale_sec,
            dq_daily_stale_sec: monitor_cfg.dq_daily_stale_sec,
        }
    }
}
