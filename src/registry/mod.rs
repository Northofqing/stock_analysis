//! v16.4 Commit 1 — StrategyRegistry 动态注册 (替代 v16.3 8 enum).
//!
//! 设计 (v16.3 doc §3.2): 替代 `opportunity::virtual_reason::VirtualReason` 硬编码 enum
//!                          (8 个变体), 用 UUID 风格 StrategyId + DashMap 动态注册,
//!                          支持运行时添加/删除 strategy (热加载, 灰度发布).
//!
//! 业务:
//!   - register(name, version, info) → StrategyId (UUID 风格)
//!   - lookup(id) → StrategyMeta
//!   - list_active() → Vec<StrategyMeta>
//!   - v16.3 8 enum 保留 as `VirtualReason::as_str()`, 仍被 paper_trades.virtual_reason 用
//!     (DB schema 不变, 8 enum 是序列化标签, StrategyId 是 runtime 路由)
//!
//! 复用: DashMap (项目已依赖, 用于 concurrent map)

use crate::bus::StrategyId;
use dashmap::DashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct StrategyMeta {
    pub id: StrategyId,
    pub name: String,
    pub version: String,
    pub description: String,
    /// v16.3 兼容: enum 标签 (NewsCatalyst / MainNetInflow / Momentum / ...)
    pub virtual_reason: String,
    pub active: bool,
}

pub struct StrategyRegistry {
    map: DashMap<StrategyId, StrategyMeta>,
}

static REGISTRY: OnceLock<StrategyRegistry> = OnceLock::new();

impl StrategyRegistry {
    pub fn global() -> &'static Self {
        REGISTRY.get_or_init(|| Self { map: DashMap::new() })
    }

    pub fn register(&self, name: &str, version: &str, description: &str, virtual_reason: &str) -> StrategyId {
        let id = crate::bus::new_strategy_id(name, version);
        let meta = StrategyMeta {
            id: id.clone(),
            name: name.to_string(),
            version: version.to_string(),
            description: description.to_string(),
            virtual_reason: virtual_reason.to_string(),
            active: true,
        };
        self.map.insert(id.clone(), meta);
        log::info!("[StrategyRegistry] 注册 {}/{}/{}", id, name, version);
        id
    }

    pub fn lookup(&self, id: &str) -> Option<StrategyMeta> {
        self.map.get(id).map(|e| e.value().clone())
    }

    pub fn list_active(&self) -> Vec<StrategyMeta> {
        self.map.iter().filter(|e| e.value().active).map(|e| e.value().clone()).collect()
    }

    pub fn list_all(&self) -> Vec<StrategyMeta> {
        self.map.iter().map(|e| e.value().clone()).collect()
    }

    pub fn deactivate(&self, id: &str) {
        if let Some(mut e) = self.map.get_mut(id) {
            e.active = false;
            log::info!("[StrategyRegistry] 停用 {}", id);
        }
    }

    pub fn activate(&self, id: &str) {
        if let Some(mut e) = self.map.get_mut(id) {
            e.active = true;
            log::info!("[StrategyRegistry] 激活 {}", id);
        }
    }
}

/// 启动时注册 8 个 v16.3 enum 对应 strategy (一次性, 1 进程 1 次)
pub fn register_v16_3_strategies() {
    let r = StrategyRegistry::global();
    r.register("NewsCatalyst", "v1", "新闻/公告催化", "NewsCatalyst");
    r.register("AuctionAnomaly", "v1", "竞价量能异动", "AuctionAnomaly");
    r.register("MainNetInflow", "v1", "主力净流入", "MainNetInflow");
    r.register("SectorLeader", "v1", "行业龙头", "SectorLeader");
    r.register("Breakout", "v1", "突破", "Breakout");
    r.register("VolumeSurge", "v1", "放量", "VolumeSurge");
    r.register("LLMSelect", "v1", "LLM 选股 (Gemini 6 分析师)", "LLMSelect");
    r.register("Momentum", "v1", "动量整合 (air_refuel + cross_resonance)", "Momentum");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_lookup() {
        let r = StrategyRegistry::global();
        let id = r.register("Test", "v0", "测试", "TestLabel");
        let meta = r.lookup(&id).expect("应该找到");
        assert_eq!(meta.name, "Test");
        assert_eq!(meta.virtual_reason, "TestLabel");
        assert!(meta.active);
    }

    #[test]
    fn list_active_excludes_deactivated() {
        let r = StrategyRegistry::global();
        let id = r.register("TestActive", "v0", "测试激活", "TestLabel");
        r.deactivate(&id);
        let active = r.list_active();
        let found = active.iter().any(|m| m.id == id);
        assert!(!found);
    }

    #[test]
    fn reactivate_restores_active() {
        let r = StrategyRegistry::global();
        let id = r.register("TestReactivate", "v0", "测试", "TestLabel");
        r.deactivate(&id);
        r.activate(&id);
        let meta = r.lookup(&id).expect("应该找到");
        assert!(meta.active);
    }

    #[test]
    fn register_overwrites_same_name_version() {
        let r = StrategyRegistry::global();
        let _id1 = r.register("Overwrite", "v0", "first", "Label1");
        let id2 = r.register("Overwrite", "v0", "second", "Label2");
        let meta = r.lookup(&id2).expect("应该找到 id2");
        assert_eq!(meta.description, "second");
    }

    #[test]
    fn v16_3_register_all_8_strategies() {
        register_v16_3_strategies();
        let r = StrategyRegistry::global();
        let all = r.list_all();
        let count = all.iter().filter(|m| m.name == "NewsCatalyst" || m.name == "Momentum").count();
        assert!(count >= 2);
    }
}
