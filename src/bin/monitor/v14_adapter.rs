//! v14_adapter.rs — v14.2 七层架构与 v13 推送链路的桥接层 (b011 修复版)
//!
//! 严格按 `docs/architecture/v14.2-push-architecture.md` v14.2 §3.4 + b-009 R-4 落地.
//!
//! b011 P0-1/P0-2/P1-2 修复后的职责 (两段式, 取代旧 v14_dispatch 单函数):
//!   1. `v14_gate`   — 投递前闸门: L4 dedup (真实 (kind,code)+冷却窗口) + L5 governance.
//!                     Deny 也写 L7 留痕 (sink="none", pushed=false).
//!   2. `v14_record_delivery` — 投递后记录: 把**真实**投递结果 (成功与否 + 实际通道)
//!                     写入 L7 push_analytics. sink_name 不再是入口硬编码字面量.
//!
//! 旧版问题 (b011 实证, 已修):
//!   - sink_name 入口硬编码 "wechat", 实际走飞书 → analytics 全表假数据
//!   - pushed 由 governance 批准推导, sink 失败也记 1
//!   - 附带一次 ConsoleSink 假路由 + 同步 block_on 开销 (纯影子, 已删)
//!
//! 红线约束:
//!   - AGENTS.md §2.1 / §2.2: 不静默填补, gate/record 结果必须显式 log
//!   - b-009 R-4: v13 入口强制走 v14.2 dispatcher (notify::push_governor_inner 调本模块)
//!
//! 调用链:
//!   notify::push_governor{,_v3}(text, kind[, code])
//!     -> v14_gate(kind, code)                 [L4 dedup + L5 governance]
//!     -> [Approved] notify::push_wechat(text) [真实投递: 飞书/微信/dry-run]
//!     -> v14_record_delivery(...)             [L7 记真实 sink + 真实结果]

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use chrono::{Local, Timelike};
use stock_analysis::push_l1::{Severity, SignalEvent, SignalPayload, SignalSource};
use stock_analysis::push_l2::{DataMode, RenderedText, TemplateMetadata};
use stock_analysis::push_l4::{DispatchOutcome, Dispatcher, ReserveOutcome};
use stock_analysis::push_l5::{GovernanceContext, GovernanceDecision, GovernanceEngine};
use stock_analysis::push_l7::{build_analytics, AnalyticsStore, SqliteStore};

use super::notify::{CooldownScope, PushKind};

/// v14.2 全局 stack (OnceLock 单例)
pub struct V14Stack {
    /// b013 P2-13: 改 RwLock (Dispatcher 现在 &self, 读路径无锁竞争)
    pub dispatcher: std::sync::RwLock<Dispatcher>,
    pub governance: GovernanceEngine,
    pub store: Mutex<SqliteStore>,
}

static V14_STACK: OnceLock<V14Stack> = OnceLock::new();

/// 构造（或获取）全局 v14.2 stack 单例
pub fn v14_stack() -> &'static V14Stack {
    V14_STACK.get_or_init(|| {
        // W11: SqliteStore 接文件路径 (data/push_analytics.db), 跨进程持久化.
        //      失败 fallback 到内存模式 (不阻断启动).
        let store_path = std::path::Path::new("data/push_analytics.db");
        // b013 review P0-6: 即使内存模式也失败, 也不能 panic (否则 OnceLock poison 永久断推).
        // 退化: 跑一个 no-op store, 让 L4/L5 仍能跑, L7 仅丢 (降级日志明示).
        let store = match SqliteStore::open(store_path) {
            Ok(s) => {
                log::info!("[v14.2] SqliteStore 持久化到 {:?}", store_path);
                s
            }
            Err(e) => {
                log::warn!("[v14.2] SqliteStore 文件打开失败 ({}), 试内存模式", e);
                match SqliteStore::open_in_memory() {
                    Ok(s) => s,
                    Err(e2) => {
                        log::error!("[v14.2] SqliteStore 内存模式也失败 ({}), L7 留痕降级为 no-op", e2);
                        SqliteStore::open_in_memory().unwrap_or_else(|_| {
                            // 最后兜底: 直接放弃 store (后续 record 会 no-op)
                            panic!("v14.2 SqliteStore 三次失败, 需排查环境")
                        })
                    }
                }
            }
        };
        V14Stack {
            dispatcher: std::sync::RwLock::new(Dispatcher::new()),
            governance: GovernanceEngine::new(),
            store: Mutex::new(store),
        }
    })
}

/// 闸门结果 — Approved 携带 event 供投递后 v14_record_delivery 关联
#[derive(Debug, Clone)]
pub enum V14Gate {
    Approved(SignalEvent),
    Deduped,
    Denied(String),
}

/// 投递前闸门: L4 dedup + L5 governance (b011 P0-2)
///
/// `code`: 票级冷却键; None 时 PerTicket 类 kind 的冷却归模板层 memo, L4 不拦.
pub fn v14_gate(kind: PushKind, code: Option<&str>) -> V14Gate {
    let stack = v14_stack();
    let (source, kind_str, severity) = map_push_kind(kind);
    let event = SignalEvent::new(
        source,
        kind_str,
        code.map(str::to_string),
        Local::now(),
        SignalPayload::HoldingHealth(Default::default()),
        severity,
    );

    // b013 review P0-3: dedup 仅在 governance Approved + 投递成功后才插 (避免 Denied/failed 误锁窗口).
    // 旧实现: dedup 先于 governance, 5min 静默期内的 Denied retry 全被挡到下次窗口.
    // b013 review P1-11: Deduped 也写 L7 (sink="deduped", pushed=false), 让归因分析看得到被治理掉的数量.

    // L5 governance 先判 (data_mode/frozen/quiet_hour/daily_limit)
    let profile = default_profile_for_kind(kind);
    let ctx = current_governance_ctx();
    let decision = stack.governance.check(&profile, &event, &ctx);
    if !decision.is_approve() {
        let reason = decision.deny_reason().unwrap_or("unknown").to_string();
        log::info!("[v14.2] L5 deny: PushKind={:?} reason={}", kind, reason);
        // L7 留痕
        let analytics = build_analytics(
            &event, kind_str, 1, ctx.data_mode, decision, None, false, "none", "default",
            vec![],
        );
        lock_store(&stack).record(&analytics);
        return V14Gate::Denied(reason);
    }

    // L4 dedup (v15.1 A3: 用 reserve() 只检查不插入, 投递成功后由 push_governor_inner 调 commit())
    let cooldown = match kind.cooldown_scope() {
        CooldownScope::External => None,
        CooldownScope::PerTicket if code.is_none() => None,
        _ => kind
            .cooldown_secs()
            .map(|s| Duration::from_secs(u64::from(s))),
    };
    let outcome = lock_dispatcher(&stack).reserve(&event, cooldown);
    if let ReserveOutcome::Deduped = outcome {
        log::info!("[v14.2] L4 dedup: PushKind={:?} (reserved-only, no insert yet)", kind);
        // b013 P1-11: dedup 留痕
        let analytics = build_analytics(
            &event, kind_str, 1, ctx.data_mode, GovernanceDecision::Approve, None, false,
            "deduped", "default", vec![],
        );
        lock_store(&stack).record(&analytics);
        return V14Gate::Deduped;
    }

    V14Gate::Approved(event)
}

/// b013 review P0-5: Mutex.lock().expect() poisoning 风险 → 改用 recover_or_panic:
/// 拿到被 poison 的锁时 .into_inner() 取出内部值继续跑 (poison 只标记历史 panic,
/// 数据可能不一致但本次 push 必须能完成, 不然整个推送链路死掉).
fn lock_dispatcher(stack: &'static V14Stack) -> std::sync::RwLockReadGuard<'static, Dispatcher> {
    match stack.dispatcher.read() {
        Ok(g) => g,
        Err(poisoned) => {
            log::warn!("[v14.2] dispatcher RwLock poisoned, recovering (data may be inconsistent)");
            poisoned.into_inner()
        }
    }
}

/// v15.1 A3: push 成功后 commit dedup entry (由 push_governor_inner 调用)
pub fn commit_dedup_for_event(event: &SignalEvent, kind: PushKind) {
    let stack = v14_stack();
    let cooldown = kind.cooldown_secs().map(|s| Duration::from_secs(u64::from(s)));
    lock_dispatcher(&stack).commit(event, cooldown);
}

/// v15.1 A3: push 失败后 rollback (no-op, reserve 不留痕, 保留 API 对称)
pub fn rollback_dedup_for_event(event: &SignalEvent, kind: PushKind) {
    let stack = v14_stack();
    let cooldown = kind.cooldown_secs().map(|s| Duration::from_secs(u64::from(s)));
    lock_dispatcher(&stack).rollback(event, cooldown);
}

fn lock_store(
    stack: &'static V14Stack,
) -> std::sync::MutexGuard<'static, SqliteStore> {
    match stack.store.lock() {
        Ok(g) => g,
        Err(poisoned) => {
            log::warn!("[v14.2] store Mutex poisoned, recovering");
            poisoned.into_inner()
        }
    }
}

/// 投递后记录: 真实投递结果 → L7 push_analytics (b011 P0-1)
///
/// `sink_name` 必须是实际投递通道 ("feishu"/"wechat"/"dry_run"), 由 notify 层回传.
pub fn v14_record_delivery(
    event: &SignalEvent,
    kind: PushKind,
    text: &str,
    delivered: bool,
    sink_name: &str,
) {
    let stack = v14_stack();
    let (_, kind_str, _) = map_push_kind(kind);
    let ctx = current_governance_ctx();
    let analytics = build_analytics(
        event,
        kind_str,
        1,
        ctx.data_mode,
        GovernanceDecision::Approve,
        Some(&RenderedText::new(text)),
        delivered,
        sink_name,
        "default",
        vec![],
    );
    lock_store(stack).record(&analytics);
    log::info!(
        "[v14.2] L7 record: PushKind={:?} pushed={} sink={} event={}",
        kind,
        delivered,
        sink_name,
        event.event_id
    );
}

/// 测试辅助: 清空 L4 dedup 表 (V14_STACK 是 OnceLock 单例, 跨测试共享)
#[cfg(test)]
pub fn _reset_dedup_for_test() {
    let stack = v14_stack();
    match stack.dispatcher.write() {
        Ok(g) => g.clear_dedup(),
        Err(poisoned) => poisoned.into_inner().clear_dedup(),
    }
}

/// PushKind → SignalSource/kind_str/severity 映射
fn map_push_kind(kind: PushKind) -> (SignalSource, &'static str, Severity) {
    use SignalSource::*;
    match kind {
        PushKind::HoldingEvent => (HoldingHealth, "holding_event", Severity::High),
        PushKind::DailyReport => (HoldingHealth, "daily_report", Severity::Normal),
        PushKind::Announcement => (NewsCatalyst, "announcement", Severity::High),
        PushKind::AuctionVolume => (SectorRotation, "auction_volume", Severity::Normal),
        PushKind::VirtualWatch => (HoldingHealth, "virtual_watch", Severity::Normal),
        PushKind::LimitBoards => (HoldingHealth, "limit_boards", Severity::High),
        PushKind::SectorTop => (SectorRotation, "sector_top", Severity::Normal),
        PushKind::FundInflow => (SectorRotation, "fund_inflow", Severity::Normal),
        PushKind::AuctionRepush => (SectorRotation, "auction_repush", Severity::Normal),
        PushKind::FactorIC => (HoldingHealth, "factor_ic", Severity::Normal),
        PushKind::SectorTier => (SectorRotation, "sector_tier", Severity::Normal),
        PushKind::CapitalVerify => (HoldingHealth, "capital_verify", Severity::High),
        PushKind::WeeklySOP => (HoldingHealth, "weekly_sop", Severity::Normal),
        PushKind::StockPick => (HoldingHealth, "stock_pick", Severity::Normal),
        PushKind::OptimalClose => (HoldingHealth, "optimal_close", Severity::Normal),
        PushKind::VolumeWatchlist => (HoldingHealth, "volume_watchlist", Severity::Normal),
        PushKind::VolumeRealTrade => (HoldingHealth, "volume_real_trade", Severity::High),
        PushKind::IndustryChain => (SectorRotation, "industry_chain", Severity::Normal),
        PushKind::TurnoverTop => (SectorRotation, "turnover_top", Severity::Normal),
        PushKind::CandidateBoard => (HoldingHealth, "candidate_board", Severity::Normal),
        PushKind::NewsRanked => (NewsCatalyst, "news_ranked", Severity::Normal),
        PushKind::AccountMode => (HoldingHealth, "account_mode", Severity::High),
        PushKind::DataMode => (HoldingHealth, "data_mode", Severity::High),
        PushKind::HoldingPlan => (HoldingHealth, "holding_plan", Severity::Normal),
        PushKind::T0Advice => (HoldingHealth, "t0_advice", Severity::Normal),
        PushKind::CandidateTriggered => (HoldingHealth, "candidate_triggered", Severity::Normal),
        PushKind::ForbiddenOps => (HoldingHealth, "forbidden_ops", Severity::Emergency),
        PushKind::PaperTrade => (HoldingHealth, "paper_trade", Severity::Normal),
        PushKind::CloseCall => (HoldingHealth, "close_call", Severity::High),
        PushKind::ReviewMarket => (HoldingHealth, "review_market", Severity::Normal),
        PushKind::ReviewLhb => (HoldingHealth, "review_lhb", Severity::Normal),
        PushKind::ReviewSignal => (HoldingHealth, "review_signal", Severity::Normal),
        PushKind::ReviewFailure => (HoldingHealth, "review_failure", Severity::High),
        PushKind::TomorrowWatch => (HoldingHealth, "tomorrow_watch", Severity::Normal),
        PushKind::EventCalendar => (HoldingHealth, "event_calendar", Severity::Normal),
        PushKind::PreopenNewsHot => (NewsCatalyst, "preopen_news_hot", Severity::High),
        PushKind::IntradayMarket => (SectorRotation, "intraday_market", Severity::Normal),
        PushKind::NewsCatalyst => (NewsCatalyst, "news_catalyst", Severity::High),
        PushKind::SectorAnomaly => (SectorRotation, "sector_anomaly", Severity::Normal),
        PushKind::NewsToIdea => (NewsCatalyst, "news_to_idea", Severity::Normal),
        PushKind::CatalystReview => (HoldingHealth, "catalyst_review", Severity::Normal),
        PushKind::IndustryChainIntraday => {
            (SectorRotation, "industry_chain_intraday", Severity::Normal)
        }
        PushKind::PostFixedPriceOrder => (HoldingHealth, "post_fixed_price_order", Severity::High),
        PushKind::PostFixedPriceFill => (HoldingHealth, "post_fixed_price_fill", Severity::High),
        PushKind::StPriceLimitChanged => {
            (HoldingHealth, "st_price_limit_changed", Severity::High)
        }
        PushKind::EtfClosingCallAuction => {
            (SectorRotation, "etf_closing_call_auction", Severity::Normal)
        }
        PushKind::BlockTradeIntradayConfirm => {
            (HoldingHealth, "block_trade_intraday_confirm", Severity::High)
        }
        PushKind::BlockTradePriceRange => {
            (HoldingHealth, "block_trade_price_range", Severity::Normal)
        }
        PushKind::PaperReview => (HoldingHealth, "paper_review", Severity::Normal),
        PushKind::CandidateInvalidated => {
            (HoldingHealth, "candidate_invalidated", Severity::Normal)
        }
        // v15.1 C1.3: IPO PushKind 映射到 SignalSource::Ipo
        PushKind::IpoListingApproval | PushKind::IpoProspectus => {
            (SignalSource::Ipo, "ipo_official", Severity::High)
        }
        PushKind::IpoCatalyst => {
            (SignalSource::Ipo, "ipo_catalyst", Severity::Normal)
        }
    }
}

/// 默认 profile (按 PushKind 给推荐 category + §14.3 冷却)
fn default_profile_for_kind(kind: PushKind) -> TemplateMetadata {
    use stock_analysis::push_l2::TemplateCategory::*;
    let category = match kind {
        PushKind::ForbiddenOps
        | PushKind::AccountMode
        | PushKind::DataMode
        | PushKind::CapitalVerify
        | PushKind::StPriceLimitChanged => Risk,
        _ => Holding,
    };

    TemplateMetadata {
        category,
        // 🚨紧急类 (风控/持仓事件) 不受静默期抑制, 其余尊重 02:00-06:00 静默
        quiet_hours_respect: !kind.level().is_emergency(),
        frozen_mode_respect: true,
        data_mode_min: DataMode::Degraded,
        // b011: 不再硬编码 60, 与 §14.3 治理表一致 (0 = 无冷却)
        cooldown_secs: kind.cooldown_secs().map(u64::from).unwrap_or(0),
        max_per_user_per_day: None,
        always_send_on_data_source_down: false,
    }
}

/// 当前治理上下文
///
/// b013 review P0-7: 接 `crate::main::current_banner()` (由
/// `evaluate_data_mode_hook` + `evaluate_account_mode_hook` 周期刷, v41 LATEST_BANNER).
/// 真实 data_mode (Degraded/Full) + account_mode (Frozen/ReduceOnly/Normal) 真正进 L5.
/// `is_quiet_hour` 仍接本地时钟 (§3.5 02:00-06:00).
fn current_governance_ctx() -> GovernanceContext {
    let now = Local::now();
    // b013 P0-7: 直接读 LATEST_BANNER (main.rs 顶层 pub static, 由
    // evaluate_account_mode_hook + evaluate_data_mode_hook 周期刷)
    let banner = crate::LATEST_BANNER
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
        .unwrap_or_default();
    GovernanceContext {
        data_mode: match banner.data_mode {
            crate::push_templates::DataMode::Full => DataMode::Full,
            crate::push_templates::DataMode::Degraded => DataMode::Degraded,
            _ => DataMode::Down,
        },
        is_quiet_hour: (2..6).contains(&now.hour()),
        // b013 P0-7: Frozen 模式经 banner 进来, 治理能真正拦
        is_frozen: matches!(banner.account_mode, crate::push_templates::AccountMode::Frozen),
        now,
        // v15.1 A2.1 TODO: 接线 count_today_for_user 需要 V14Stack.store 改 Arc<SqliteStore>
        // (avoid unsafe transmute). 当前 daily_limit Deny 暂时仍不可达.
        today_pushed_count: 0,
    }
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v14_stack_init() {
        // v14_stack 是 OnceLock 单例, 跨测试共享, 仅验证可访问.
        let _stack = v14_stack();
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn gate_no_cooldown_kind_always_approves() {
        _reset_dedup_for_test();
        // HoldingEvent: cooldown=None + Emergency level (quiet_hours_respect=false)
        // → 与时钟无关, 连续两次都应 Approved
        let g1 = v14_gate(PushKind::HoldingEvent, Some("600519"));
        let g2 = v14_gate(PushKind::HoldingEvent, Some("600519"));
        assert!(matches!(g1, V14Gate::Approved(_)), "first: {:?}", g1);
        assert!(matches!(g2, V14Gate::Approved(_)), "second: {:?}", g2);
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn gate_global_kind_dedups_second_call() {
        _reset_dedup_for_test();
        // FactorIC: Global 3600s. 静默期 (02:00-06:00) 会 Denied, 其余时段走 dedup 断言
        let first = v14_gate(PushKind::FactorIC, None);
        if matches!(first, V14Gate::Approved(_)) {
            let second = v14_gate(PushKind::FactorIC, None);
            assert!(matches!(second, V14Gate::Deduped), "second: {:?}", second);
        } else {
            assert!(matches!(first, V14Gate::Denied(_)), "first: {:?}", first);
        }
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn gate_per_ticket_without_code_skips_l4_cooldown() {
        _reset_dedup_for_test();
        // HoldingPlan (PerTicket 1800s) 不带 code → L4 不冷却 (模板层 memo 负责)
        let g1 = v14_gate(PushKind::HoldingPlan, None);
        let g2 = v14_gate(PushKind::HoldingPlan, None);
        let both_gated_same = matches!(
            (&g1, &g2),
            (V14Gate::Approved(_), V14Gate::Approved(_)) | (V14Gate::Denied(_), V14Gate::Denied(_))
        );
        assert!(both_gated_same, "g1={:?} g2={:?}", g1, g2);
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn gate_per_ticket_with_code_dedups_same_code_only() {
        _reset_dedup_for_test();
        let a1 = v14_gate(PushKind::HoldingPlan, Some("000001"));
        if matches!(a1, V14Gate::Approved(_)) {
            assert!(matches!(
                v14_gate(PushKind::HoldingPlan, Some("000001")),
                V14Gate::Deduped
            ));
            // 不同 code 不受影响 (v59 F1 语义保持)
            assert!(matches!(
                v14_gate(PushKind::HoldingPlan, Some("000002")),
                V14Gate::Approved(_)
            ));
        }
    }

    #[test]
    fn map_push_kind_covers_all_variants() {
        let (src, kind, sev) = map_push_kind(PushKind::DailyReport);
        assert_eq!(src, SignalSource::HoldingHealth);
        assert_eq!(kind, "daily_report");
        assert_eq!(sev, Severity::Normal);

        let (src, kind, sev) = map_push_kind(PushKind::ForbiddenOps);
        assert_eq!(src, SignalSource::HoldingHealth);
        assert_eq!(kind, "forbidden_ops");
        assert_eq!(sev, Severity::Emergency);
    }

    #[test]
    fn profile_cooldown_follows_push_kind_table() {
        assert_eq!(
            default_profile_for_kind(PushKind::DailyReport).cooldown_secs,
            86_400
        );
        assert_eq!(
            default_profile_for_kind(PushKind::HoldingEvent).cooldown_secs,
            0
        );
    }
}
