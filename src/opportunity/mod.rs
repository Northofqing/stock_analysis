//! Opportunity Context — 产业链挖掘 + 机会发现。
//!
//! BR-174 已将正式事件级选股迁移到 `selection` schema-v2。这里仅保留仍有
//! 独立生产职责的 opportunity 子模块；旧候选扫描、盘后 Top-N 生成和 legacy
//! outcome owner 已删除，禁止回退到模糊板块匹配或无证据候选。

pub mod auction_agent; // v10 P0.2: 09:25 竞价 Agent
pub mod bom_kb; // 修复 P0-2: BOM 弹性节点 + KB
pub mod candidate_panel; // v11-P0-5+ Commit A: 候选筛选台模型 + 多源合并去重
pub mod candidate_state; // v12 PR3-3.4: 影子候选零推送
pub mod chain_mapper;
pub mod discover;
pub mod event_extractor;
pub mod hit_case; // v10 P0.1 BC-3: 5 边界 hit CASE 逻辑
pub mod impact;
pub mod launch_gate; // 修复 P0-3: 上线门槛
pub mod real_alpha;
pub mod scheduler; // 修复 v9.1 §1.3: 调度器
pub mod score; // 修复 P0-1: dual_score 评分模型
pub mod virtual_reason; // v10 P0.2 BR-016: VirtualReason 枚举 + 主理由优先级
pub mod winrate; // 修复 P1-2: winrate 二元化 // v10 P0.3 BC-1: real_alpha + A/B/C 置信度 + 5 要素信封
