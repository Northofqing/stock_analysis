//! v17.6 §5.1: DailyReport 子段路由器
//!
//! 把原 PushKind 中的 3 个 low-priority variants (FactorIC / SectorTier / CapitalVerify)
//! 收纳进 `DailyReportSubKind` 体系, 走统一 DailyReport 主路径 + title prefix 区分.
//!
//! ## 设计 (Path D 一致 — audit/helper/env 模式, 不重写 L1 EventBus)
//!
//! - 3 个 variant 在 `PushKind` enum 仍保留 (向后兼容, 现有 9 callsite 不破)
//! - `PushKind::daily_report_sub_kind()` 返回 Option<DailyReportSubKind> 标识子段归属
//! - 本模块 3 个公开函数 (route_factor_ic / route_sector_tier / route_capital_verify)
//!   内部走 `push_governor_v3(PushKind::DailyReport, ...)`, 但 title 加 `[SubKind]` prefix
//! - 启动时 audit log 打印 3 sub_kind + legacy_kind 映射表 (默认值"出声")
//!
//! ## 冷却
//!
//! - DailyReport 主路径 24h (`PushKind::DailyReport::cooldown_secs` = 86400)
//! - sub_kind 独立窗口 (`DailyReportSubKind::cooldown_secs`) 默认 None (共享主路径)
//!   SectorTier/CapitalVerify override 30min, 仍受 L4 dedup (sub_kind + date) 控制
//!
//! ## 回滚
//!
//! 改回旧路径: 把 `route_*` 调用换回 `notify::push_governor(&text, PushKind::FactorIC)` 等.
//! `PushKind` 三个 variants 仍存在, 不破坏 type system.

use crate::notify::{push_governor_v3, DailyReportSubKind, PushKind, PushOutcome};
use chrono::Local;
use stock_analysis::push_l4::dispatcher::sub_kind_dedup_key;

/// 启动时 audit log — 打印 3 sub_kind + legacy 映射 + 冷却配置.
///
/// 设计: 默认出声 (v15.x 4 铁律), 让运维一眼看清路由表面.
pub fn init_audit() {
    log::info!(
        "[v17.6 §5.1] daily_report_router init: 3 sub_kinds = {:?}, \
         legacy_kind = {:?}, master cooldown = 24h",
        [
            DailyReportSubKind::FactorIC,
            DailyReportSubKind::SectorTier,
            DailyReportSubKind::CapitalVerify,
        ],
        [
            DailyReportSubKind::FactorIC.legacy_kind(),
            DailyReportSubKind::SectorTier.legacy_kind(),
            DailyReportSubKind::CapitalVerify.legacy_kind(),
        ]
    );
}

/// 给 title 加 sub_kind prefix, 避免与 DailyReport 主推送混在一起时丢失语义.
///
/// 例如: "[FactorIC] 因子 IC 排行 Top10" 而非直接"因子 IC 排行 Top10".
/// DailyReport 主推送无 prefix, 仍按 v12 §14.2 R-01 渲染.
fn with_sub_kind_prefix(sub_kind: DailyReportSubKind, text: &str) -> String {
    format!("[{}] {}", sub_kind.label(), text)
}

/// 内部统一调度: 走 push_governor_v3(PushKind::DailyReport) + sub_kind prefix + audit log.
async fn route(sub_kind: DailyReportSubKind, text: &str) -> PushOutcome {
    let prefixed = with_sub_kind_prefix(sub_kind, text);
    // v17.6 §5.1: 计算 sub_kind-aware dedup key (audit-only — 当前 dispatcher 走的是
    // (kind, code) 旧键, 此 key 留在日志供后续 L4 hash 接入时直接接上).
    let today = Local::now().format("%Y-%m-%d").to_string();
    let dedup_key = sub_kind_dedup_key("daily_report", Some(sub_kind.label()), &today);
    log::info!(
        "[v17.6 §5.1] daily_report_router::{} dispatch: len={} dedup_key={}",
        sub_kind.label(),
        prefixed.chars().count(),
        dedup_key
    );
    push_governor_v3(&prefixed, PushKind::DailyReport, None).await
}

/// 公开: FactorIC (因子 IC 排行) → DailyReport 主路径 + [FactorIC] prefix
pub async fn route_factor_ic(text: &str) -> PushOutcome {
    route(DailyReportSubKind::FactorIC, text).await
}

/// 公开: SectorTier (v4 赛道分档) → DailyReport 主路径 + [SectorTier] prefix
pub async fn route_sector_tier(text: &str) -> PushOutcome {
    route(DailyReportSubKind::SectorTier, text).await
}

/// 公开: CapitalVerify (v4 资金验证) → DailyReport 主路径 + [CapitalVerify] prefix
pub async fn route_capital_verify(text: &str) -> PushOutcome {
    route(DailyReportSubKind::CapitalVerify, text).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 3 sub_kinds 枚举完整 + label 唯一
    #[test]
    fn sub_kind_label_unique_three() {
        let labels = [
            DailyReportSubKind::FactorIC.label(),
            DailyReportSubKind::SectorTier.label(),
            DailyReportSubKind::CapitalVerify.label(),
        ];
        assert_eq!(labels.len(), 3, "v17.6 §5.1 应 3 个 sub_kind");
        let mut sorted = labels.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 3, "label 应唯一: {:?}", labels);
    }

    /// legacy_kind 反向映射回原 PushKind variant
    #[test]
    fn sub_kind_legacy_kind_roundtrip() {
        assert_eq!(
            DailyReportSubKind::FactorIC.legacy_kind(),
            PushKind::FactorIC
        );
        assert_eq!(
            DailyReportSubKind::SectorTier.legacy_kind(),
            PushKind::SectorTier
        );
        assert_eq!(
            DailyReportSubKind::CapitalVerify.legacy_kind(),
            PushKind::CapitalVerify
        );
    }

    /// sub_kind 冷却: FactorIC None (跟随主 24h), SectorTier/CapitalVerify 30min
    #[test]
    fn sub_kind_cooldown_override() {
        assert_eq!(DailyReportSubKind::FactorIC.cooldown_secs(), None);
        assert_eq!(DailyReportSubKind::SectorTier.cooldown_secs(), Some(1800));
        assert_eq!(
            DailyReportSubKind::CapitalVerify.cooldown_secs(),
            Some(1800)
        );
    }

    /// stable_template_id 格式: daily_report_<kind>_v1
    #[test]
    fn sub_kind_stable_template_id_format() {
        assert_eq!(
            DailyReportSubKind::FactorIC.stable_template_id(),
            "daily_report_factoric_v1"
        );
        assert_eq!(
            DailyReportSubKind::SectorTier.stable_template_id(),
            "daily_report_sectortier_v1"
        );
        assert_eq!(
            DailyReportSubKind::CapitalVerify.stable_template_id(),
            "daily_report_capitalverify_v1"
        );
    }

    /// with_sub_kind_prefix 把 [SubKind] 前缀贴到 title 开头
    #[test]
    fn sub_kind_prefix_format() {
        let prefixed = with_sub_kind_prefix(DailyReportSubKind::FactorIC, "hello world");
        assert_eq!(prefixed, "[FactorIC] hello world");
        let prefixed2 = with_sub_kind_prefix(DailyReportSubKind::SectorTier, "赛道 A: 60分");
        assert_eq!(prefixed2, "[SectorTier] 赛道 A: 60分");
    }

    /// init_audit 默认出声 — 不 panic, 不需 env var
    #[test]
    fn init_audit_does_not_panic() {
        init_audit();
    }

    /// dedup_key 拼接规则: 有 sub_kind → "daily_report|sub_kind=<X>|date=<D>",
    /// 无 sub_kind → 仅 kind 字符串 (向后兼容原 L4 键空间)
    #[test]
    fn sub_kind_dedup_key_format() {
        use stock_analysis::push_l4::dispatcher::sub_kind_dedup_key;
        let k = sub_kind_dedup_key("daily_report", Some("FactorIC"), "2026-07-16");
        assert_eq!(k, "daily_report|sub_kind=FactorIC|date=2026-07-16");
        let k2 = sub_kind_dedup_key("daily_report", None, "2026-07-16");
        assert_eq!(k2, "daily_report", "None sub_kind 应 fallback 到纯 kind");
        let k3 = sub_kind_dedup_key("holding_event", Some("HoldingPlan"), "2026-07-16");
        assert_eq!(k3, "holding_event|sub_kind=HoldingPlan|date=2026-07-16");
    }
}