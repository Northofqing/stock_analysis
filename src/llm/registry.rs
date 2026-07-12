//! LlmRegistry — 业务侧入口, 按 role 选 provider, role 缺失时按 fallback 链选

use super::providers::{DeepSeekProvider, MiniMaxProvider, OpenAiCompatProvider};
use super::LlmProvider;
use std::collections::HashMap;
use std::sync::Arc;

/// Role = 业务任务. 业务用 `select("ticker_extraction")` 拿 provider, 不关心是哪个模型.
///
/// 一个 role 可指定 多个 provider (逗号分隔), 按顺序尝试, 第一个可用即用.
#[derive(Debug, Clone)]
pub struct LlmRole {
    /// role 名, e.g. "ticker_extraction", "deep_analysis"
    pub name: String,
    /// 候选 provider 列表 (按优先级), 空时全局 fallback
    pub candidates: Vec<String>,
}

/// 注册表 — 启动时 from_env 加载, 运行时 select(role) 取 provider
pub struct LlmRegistry {
    providers: HashMap<String, Arc<dyn LlmProvider>>,
    /// role → 候选 provider 列表 (按顺序)
    roles: HashMap<String, Vec<String>>,
    /// role 找不到时的全局 fallback 链
    default_fallback: Vec<String>,
}

impl LlmRegistry {
    /// 从 env 加载所有 provider + role 配置
    ///
    /// env 约定:
    /// - 各个 provider 的 env (DEEPSEEK_API_KEY / MiniMax_API_KEY / OPENAI_COMPAT_API_KEY)
    /// - `LLM_ROLE_<NAME>=<provider1>,<provider2>,...` 例如 `LLM_ROLE_TICKER=minimax,deepseek`
    /// - `LLM_DEFAULT_FALLBACK=deepseek,minimax,openai_compat`
    pub fn from_env() -> Self {
        let mut providers: HashMap<String, Arc<dyn LlmProvider>> = HashMap::new();

        if let Some(p) = DeepSeekProvider::from_env() {
            providers.insert(p.name().to_string(), Arc::new(p));
        }
        if let Some(p) = MiniMaxProvider::from_env() {
            providers.insert(p.name().to_string(), Arc::new(p));
        }
        if let Some(p) = OpenAiCompatProvider::from_env() {
            providers.insert(p.name().to_string(), Arc::new(p));
        }

        let mut roles: HashMap<String, Vec<String>> = HashMap::new();
        for (k, v) in std::env::vars() {
            if let Some(role_name) = k.strip_prefix("LLM_ROLE_") {
                let role = role_name.to_lowercase();
                let candidates: Vec<String> = v
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if !candidates.is_empty() {
                    roles.insert(role, candidates);
                }
            }
        }

        let default_fallback = std::env::var("LLM_DEFAULT_FALLBACK")
            .unwrap_or_else(|_| "deepseek,minimax,openai_compat".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        log::info!(
            "[LLM] 加载 {} 个 provider: {:?}",
            providers.len(),
            providers.keys().collect::<Vec<_>>()
        );
        log::info!(
            "[LLM] {} 个 role 配置, fallback={:?}",
            roles.len(),
            default_fallback
        );

        Self {
            providers,
            roles,
            default_fallback,
        }
    }

    /// 按 role 选 provider, 返回第一个可用的. 无可用 → None, 业务降级.
    pub fn select(&self, role: &str) -> Option<Arc<dyn LlmProvider>> {
        // 1. role 候选
        if let Some(candidates) = self.roles.get(role) {
            for name in candidates {
                if let Some(p) = self.providers.get(name) {
                    return Some(p.clone());
                }
            }
        }
        // 2. 全局 fallback
        for name in &self.default_fallback {
            if let Some(p) = self.providers.get(name) {
                return Some(p.clone());
            }
        }
        log::warn!(
            "[LLM] role={} 无可用 provider (env 未配置 / 全部失败)",
            role
        );
        None
    }

    /// 已加载的 provider 列表 (调试用)
    pub fn available_providers(&self) -> Vec<String> {
        self.providers.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_loads_what_is_configured() {
        // 不 mock env, 至少验证不 panic + 返回空 registry 当全无配置时
        let r = LlmRegistry::from_env();
        // 至少 available_providers 不 panic
        let _ = r.available_providers();
    }

    #[test]
    fn select_returns_none_when_empty() {
        let r = LlmRegistry {
            providers: HashMap::new(),
            roles: HashMap::new(),
            default_fallback: vec![],
        };
        assert!(r.select("anything").is_none());
    }
}
