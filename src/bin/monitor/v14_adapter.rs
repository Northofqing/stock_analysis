//! Registered business rules: BR-005, BR-048.
//! v14_adapter.rs — v14.2 七层架构与 v13 推送链路的桥接层 (b011 修复版)
//!
//! 严格按 `docs/architecture/v14.2-push-architecture.md` v14.2 §3.4 + b-009 R-4 落地.
//!
//! b011 P0-1/P0-2/P1-2 修复后的职责 (两段式, 取代旧 v14_dispatch 单函数):
//!   1. `v14_gate`   — 投递前闸门: L4 dedup (真实 (kind,code)+冷却窗口) + L5 governance.
//!      Deny 也写 L7 留痕 (sink="none", pushed=false).
//!   2. `v14_record_delivery` — 投递后记录: 把**真实**投递结果 (成功与否 + 实际通道)
//!      写入 L7 push_analytics. sink_name 不再是入口硬编码字面量.
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
use stock_analysis::push_l4::{Dispatcher, ReserveOutcome};
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

static V14_STACK: OnceLock<Result<V14Stack, String>> = OnceLock::new();

/// 构造（或获取）全局 v14.2 stack 单例
pub fn v14_stack() -> Result<&'static V14Stack, String> {
    let initialized = V14_STACK.get_or_init(|| {
        let test_mode = stock_analysis::risk::env_guard::current_env()
            == stock_analysis::risk::env_guard::TradingEnv::Test;
        let store_path = if test_mode {
            std::path::PathBuf::from("data/test/push_analytics.db")
        } else {
            std::path::PathBuf::from("data/push_analytics.db")
        };
        let parent = store_path
            .parent()
            .ok_or_else(|| format!("L7 store path has no parent: {}", store_path.display()))?;
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create L7 directory {}: {error}", parent.display()))?;
        let store = SqliteStore::open(&store_path).map_err(|error| {
            format!("open persistent L7 store {}: {error}", store_path.display())
        })?;
        log::info!("[v14.2][BR-113] SqliteStore={}", store_path.display());
        Ok(V14Stack {
            dispatcher: std::sync::RwLock::new(Dispatcher::new()),
            governance: GovernanceEngine::new(),
            store: Mutex::new(store),
        })
    });
    initialized.as_ref().map_err(Clone::clone)
}

/// 闸门结果 — Approved 携带 event 供投递后 v14_record_delivery 关联
#[derive(Debug, Clone)]
pub enum V14Gate {
    Approved(Box<SignalEvent>),
    Deduped,
    Denied(String),
}

/// 投递前闸门: L4 dedup + L5 governance (b011 P0-2)
///
/// `code`: 票级冷却键; None 时 PerTicket 类 kind 的冷却归模板层 memo, L4 不拦.
/// `sub_kind`: v17.6 §5.1 — DailyReport 子段 (FactorIC/SectorTier/CapitalVerify) 推送时
/// 传入 Some("FactorIC") 让 dedup key 加上第三元组, 实现 per-sub_kind 隔离.
/// None 时 sub_kind="" (向后兼容, 不破坏现有 caller).
pub fn v14_gate(kind: PushKind, code: Option<&str>) -> V14Gate {
    v14_gate_with_sub_kind(kind, code, None, None)
}

/// v17.6 §5.1: v14_gate 的 sub_kind-aware 版本. daily_report_router 三个公开函数调用.
pub fn v14_gate_with_sub_kind(
    kind: PushKind,
    code: Option<&str>,
    sub_kind: Option<&str>,
    cooldown_override_secs: Option<u32>,
) -> V14Gate {
    let stack = match v14_stack() {
        Ok(stack) => stack,
        Err(error) => {
            log::error!("[v14.2][BR-113] {error}");
            return V14Gate::Denied("analytics_store_unavailable".to_string());
        }
    };
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
    let mut ctx = match current_governance_ctx() {
        Ok(ctx) => ctx,
        Err(error) => {
            log::error!("[v14.2][BR-113] {error}");
            return V14Gate::Denied("governance_context_unavailable".to_string());
        }
    };
    if profile.max_per_user_per_day.is_some() {
        let count_result = {
            let store = match lock_store(stack) {
                Ok(store) => store,
                Err(error) => {
                    log::error!("[v14.2][BR-113] {error}");
                    return V14Gate::Denied("analytics_store_lock_unavailable".to_string());
                }
            };
            store.count_today_pushed_for_user_and_template(
                "default",
                kind_str,
                ctx.now.date_naive(),
            )
        };
        match count_result {
            Ok(count) => match u32::try_from(count) {
                Ok(count) => ctx.today_pushed_count = count,
                Err(_) => {
                    return deny_for_unavailable_daily_count(
                        stack,
                        &event,
                        kind_str,
                        &ctx,
                        format!("invalid daily count {count}"),
                    );
                }
            },
            Err(error) => {
                return deny_for_unavailable_daily_count(
                    stack,
                    &event,
                    kind_str,
                    &ctx,
                    error.to_string(),
                );
            }
        }
    }
    let decision = stack.governance.check(&profile, &event, &ctx);
    if !decision.is_approve() {
        let reason = decision.deny_reason().unwrap_or("unknown").to_string();
        log::info!("[v14.2] L5 deny: PushKind={:?} reason={}", kind, reason);
        // L7 留痕
        let analytics = build_analytics(
            &event,
            kind_str,
            1,
            ctx.data_mode,
            decision,
            None,
            false,
            "none",
            "default",
            vec![],
        );
        if let Err(error) = lock_store(stack)
            .and_then(|store| store.record(&analytics).map_err(|error| error.to_string()))
        {
            log::error!("[v14.2][BR-113] L5 deny audit failed: {error}");
            return V14Gate::Denied("analytics_audit_unavailable".to_string());
        }
        return V14Gate::Denied(reason);
    }

    // L4 dedup (v15.1 A3: 用 reserve() 只检查不插入, 投递成功后由 push_governor_inner 调 commit())
    // v17.6 §5.1: sub_kind 参与 dedup key (per-sub_kind 独立窗口)
    let cooldown = dedup_cooldown(kind, code.is_some(), cooldown_override_secs);
    let outcome = match lock_dispatcher(stack) {
        Ok(dispatcher) => dispatcher.reserve(&event, cooldown, sub_kind),
        Err(error) => {
            log::error!("[v14.2][BR-113] {error}");
            return V14Gate::Denied("dedup_lock_unavailable".to_string());
        }
    };
    if let ReserveOutcome::Deduped = outcome {
        log::info!(
            "[v14.2] L4 dedup: PushKind={:?} sub_kind={:?} (reserved-only, no insert yet)",
            kind,
            sub_kind
        );
        // b013 P1-11: dedup 留痕
        let analytics = build_analytics(
            &event,
            kind_str,
            1,
            ctx.data_mode,
            GovernanceDecision::Approve,
            None,
            false,
            "deduped",
            "default",
            vec![],
        );
        if let Err(error) = lock_store(stack)
            .and_then(|store| store.record(&analytics).map_err(|error| error.to_string()))
        {
            log::error!("[v14.2][BR-113] L4 dedup audit failed: {error}");
            return V14Gate::Denied("analytics_audit_unavailable".to_string());
        }
        return V14Gate::Deduped;
    }

    V14Gate::Approved(Box::new(event))
}

fn lock_dispatcher(
    stack: &'static V14Stack,
) -> Result<std::sync::RwLockReadGuard<'static, Dispatcher>, String> {
    stack
        .dispatcher
        .read()
        .map_err(|_| "dispatcher RwLock poisoned; governance rejected".to_string())
}

fn dedup_cooldown(
    kind: PushKind,
    has_code: bool,
    cooldown_override_secs: Option<u32>,
) -> Option<Duration> {
    match kind.cooldown_scope() {
        CooldownScope::External => None,
        CooldownScope::PerTicket if !has_code => None,
        _ => cooldown_override_secs
            .or_else(|| kind.cooldown_secs())
            .map(|secs| Duration::from_secs(u64::from(secs))),
    }
}

/// v15.1 A3: push 成功后 commit dedup entry (由 push_governor_inner 调用)
pub fn commit_dedup_for_event(
    event: &SignalEvent,
    kind: PushKind,
    sub_kind: Option<&str>,
    cooldown_override_secs: Option<u32>,
) -> Result<(), String> {
    let stack = v14_stack()?;
    let cooldown = dedup_cooldown(kind, event.code.is_some(), cooldown_override_secs);
    lock_dispatcher(stack)?.commit(event, cooldown, sub_kind);
    Ok(())
}

/// v15.1 A3: push 失败后 rollback (no-op, reserve 不留痕, 保留 API 对称)
pub fn rollback_dedup_for_event(
    event: &SignalEvent,
    kind: PushKind,
    sub_kind: Option<&str>,
    cooldown_override_secs: Option<u32>,
) -> Result<(), String> {
    let stack = v14_stack()?;
    let cooldown = dedup_cooldown(kind, event.code.is_some(), cooldown_override_secs);
    lock_dispatcher(stack)?.rollback(event, cooldown, sub_kind);
    Ok(())
}

fn lock_store(
    stack: &'static V14Stack,
) -> Result<std::sync::MutexGuard<'static, SqliteStore>, String> {
    stack
        .store
        .lock()
        .map_err(|_| "L7 store Mutex poisoned; analytics rejected".to_string())
}

fn deny_for_unavailable_daily_count(
    stack: &'static V14Stack,
    event: &SignalEvent,
    kind_str: &str,
    ctx: &GovernanceContext,
    error: String,
) -> V14Gate {
    log::error!(
        "[v14.2][BR-005] daily push count unavailable for {}: {}",
        kind_str,
        error
    );
    let reason = "daily_limit_count_unavailable".to_string();
    let analytics = build_analytics(
        event,
        kind_str,
        1,
        ctx.data_mode,
        GovernanceDecision::Deny(reason.clone()),
        None,
        false,
        "none",
        "default",
        vec![error],
    );
    match lock_store(stack)
        .and_then(|store| store.record(&analytics).map_err(|error| error.to_string()))
    {
        Ok(()) => V14Gate::Denied(reason),
        Err(record_error) => {
            log::error!("[v14.2][BR-113] daily-count failure audit failed: {record_error}");
            V14Gate::Denied("analytics_audit_unavailable".to_string())
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
) -> Result<(), String> {
    let stack = v14_stack()?;
    let (_, kind_str, _) = map_push_kind(kind);
    let ctx = current_governance_ctx()?;
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
    lock_store(stack)?
        .record(&analytics)
        .map_err(|error| error.to_string())?;
    log::info!(
        "[v14.2] L7 record: PushKind={:?} pushed={} sink={} event={}",
        kind,
        delivered,
        sink_name,
        event.event_id
    );
    Ok(())
}

/// 测试辅助: 清空 L4 dedup 表 (V14_STACK 是 OnceLock 单例, 跨测试共享)
#[cfg(test)]
pub fn _reset_dedup_for_test() {
    let mut banner = crate::LATEST_BANNER
        .lock()
        .expect("test banner lock must be available");
    *banner = Some(crate::push_templates::BannerCtx::default());
    drop(banner);
    let stack = v14_stack().expect("test L7 store must initialize");
    match stack.dispatcher.write() {
        Ok(g) => g.clear_dedup(),
        Err(poisoned) => poisoned.into_inner().clear_dedup(),
    }
}

/// 测试辅助: 模拟 push 成功后调 dispatcher.commit() 插入 dedup entry.
/// v15.1 A3 后 reserve() 不占位, 必须 commit() 才会让后续 reserve 看到 Deduped.
/// v17.6 §5.1: sub_kind 参与 dedup key, 测试场景默认 None.
#[cfg(test)]
pub fn _commit_dedup_for_test(kind: PushKind, code: Option<&str>) {
    let stack = v14_stack().expect("test L7 store must initialize");
    let (source, kind_str, severity) = map_push_kind(kind);
    let event = SignalEvent::new(
        source,
        kind_str,
        code.map(str::to_string),
        Local::now(),
        SignalPayload::HoldingHealth(Default::default()),
        severity,
    );
    let cooldown = match kind.cooldown_scope() {
        CooldownScope::External => None,
        CooldownScope::PerTicket if code.is_none() => None,
        _ => kind
            .cooldown_secs()
            .map(|s| Duration::from_secs(u64::from(s))),
    };
    match stack.dispatcher.write() {
        Ok(g) => g.commit(&event, cooldown, None),
        Err(poisoned) => poisoned.into_inner().commit(&event, cooldown, None),
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
        PushKind::StPriceLimitChanged => (HoldingHealth, "st_price_limit_changed", Severity::High),
        PushKind::EtfClosingCallAuction => {
            (SectorRotation, "etf_closing_call_auction", Severity::Normal)
        }
        PushKind::BlockTradeIntradayConfirm => (
            HoldingHealth,
            "block_trade_intraday_confirm",
            Severity::High,
        ),
        PushKind::BlockTradePriceRange => {
            (HoldingHealth, "block_trade_price_range", Severity::Normal)
        }
        PushKind::PaperReview => (HoldingHealth, "paper_review", Severity::Normal),
        // v17.4 能力1 (BR-082)
        PushKind::NewsFlashCritical => (NewsCatalyst, "news_flash_critical", Severity::High),
        PushKind::NewsFlashAggregated => (NewsCatalyst, "news_flash_aggregated", Severity::Normal),
        PushKind::CandidateInvalidated => {
            (HoldingHealth, "candidate_invalidated", Severity::Normal)
        }
        // v15.1 C1.3: IPO PushKind 映射到 SignalSource::Ipo
        PushKind::IpoListingApproval | PushKind::IpoProspectus => {
            (SignalSource::Ipo, "ipo_official", Severity::High)
        }
        PushKind::IpoCatalyst => (SignalSource::Ipo, "ipo_catalyst", Severity::Normal),
        // v15.3 D5.2: 4 路源 SignalSource 映射
        PushKind::PolicyHit => (SignalSource::Policy, "policy_hit", Severity::High),
        PushKind::EarningsBeat => (SignalSource::Earnings, "earnings_beat", Severity::High),
        PushKind::EarningsMiss => (SignalSource::Earnings, "earnings_miss", Severity::High),
        PushKind::AnalystUpgrade => (
            SignalSource::AnalystView,
            "analyst_upgrade",
            Severity::Normal,
        ),
        PushKind::MarketActionAlert => (
            SignalSource::MarketAction,
            "market_action_alert",
            Severity::High,
        ),
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
        // v17.1 治本: frozen_mode_respect 改为 false (L5 governance 不再 Deny Frozen)
        // Frozen 状态保留在 ctx.is_frozen, 模板自行渲染 ⚠️ 警告
        // 4 铁律: 通知层保持出声, 仓位风险控制在 broker 下单层
        frozen_mode_respect: false,
        data_mode_min: DataMode::Degraded,
        // b011: 不再硬编码 60, 与 §14.3 治理表一致 (0 = 无冷却)
        cooldown_secs: kind.cooldown_secs().map(u64::from).unwrap_or(0),
        max_per_user_per_day: matches!(kind, PushKind::CandidateBoard).then_some(5),
        always_send_on_data_source_down: false,
    }
}

/// 当前治理上下文
///
/// b013 review P0-7: 接 `crate::main::current_banner()` (由
/// `evaluate_data_mode_hook` + `evaluate_account_mode_hook` 周期刷, v41 LATEST_BANNER).
/// 真实 data_mode (Degraded/Full) + account_mode (Frozen/ReduceOnly/Normal) 真正进 L5.
/// `is_quiet_hour` 仍接本地时钟 (§3.5 02:00-06:00).
fn current_governance_ctx() -> Result<GovernanceContext, String> {
    let now = Local::now();
    // b013 P0-7: 直接读 LATEST_BANNER (main.rs 顶层 pub static, 由
    // evaluate_account_mode_hook + evaluate_data_mode_hook 周期刷)
    let banner = crate::LATEST_BANNER
        .lock()
        .map_err(|_| "governance banner lock poisoned".to_string())?
        .clone()
        .ok_or_else(|| "governance banner unavailable".to_string())?;
    Ok(GovernanceContext {
        data_mode: match banner.data_mode {
            crate::push_templates::DataMode::Full => DataMode::Full,
            crate::push_templates::DataMode::Degraded => DataMode::Degraded,
            _ => DataMode::Down,
        },
        is_quiet_hour: match std::env::var("STOCK_ANALYSIS_QUIET_HOUR_OVERRIDE")
            .ok()
            .as_deref()
        {
            // 显式 override (仅测试/运维用): "0"/"false" 强制非静默, "1"/"true" 强制静默
            // 默认 (未设): 墙钟 02:00-06:00 静默, 行为不变 (v15.x 静默默认值需显式声明才生效)
            // 背景: e2e_t01/t02 断言"推送成功", 凌晨 02-06 点跑必挂 (墙钟依赖),
            //   测试内设 override=0 变时间无关 (2026-07-16 诊断)
            Some("0") | Some("false") => false,
            Some("1") | Some("true") => true,
            _ => (2..6).contains(&now.hour()),
        },
        // b013 P0-7: Frozen 模式经 banner 进来, 治理能真正拦
        is_frozen: matches!(
            banner.account_mode,
            crate::push_templates::AccountMode::Frozen
        ),
        now,
        // BR-005: 有日上限的模板由 v14_gate 在 L5 检查前从 L7 注入真实成功数。
        today_pushed_count: 0,
    })
}

// ============================================================================
// 单元测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn isolated_stack() -> &'static V14Stack {
        Box::leak(Box::new(V14Stack {
            dispatcher: std::sync::RwLock::new(Dispatcher::new()),
            governance: GovernanceEngine::new(),
            store: Mutex::new(SqliteStore::open_in_memory().expect("test L7 store")),
        }))
    }

    #[test]
    fn br113_poisoned_dispatcher_lock_is_rejected() {
        let stack = isolated_stack();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = stack.dispatcher.write().expect("fresh dispatcher lock");
            panic!("TEST_CODE poison dispatcher");
        }));
        assert!(lock_dispatcher(stack).is_err());
    }

    #[test]
    fn br113_poisoned_store_lock_is_rejected() {
        let stack = isolated_stack();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = stack.store.lock().expect("fresh store lock");
            panic!("TEST_CODE poison store");
        }));
        assert!(lock_store(stack).is_err());
    }

    #[test]
    fn v14_stack_init() {
        // v14_stack 是 OnceLock 单例, 跨测试共享, 仅验证可访问.
        let _stack = v14_stack().unwrap();
    }

    #[test]
    fn br005_candidate_board_profile_has_daily_limit_five() {
        assert_eq!(
            default_profile_for_kind(PushKind::CandidateBoard).max_per_user_per_day,
            Some(5)
        );
        assert_eq!(
            default_profile_for_kind(PushKind::HoldingEvent).max_per_user_per_day,
            None
        );
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn gate_no_cooldown_kind_always_approves() {
        _reset_dedup_for_test();
        // HoldingEvent: cooldown=None + Emergency level (quiet_hours_respect=false)
        // → 与时钟无关, 连续两次都应 Approved
        let g1 = v14_gate(PushKind::HoldingEvent, Some("TEST_CODE_600519"));
        let g2 = v14_gate(PushKind::HoldingEvent, Some("TEST_CODE_600519"));
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
            // v15.1 A3: reserve 不占位, 模拟 push 成功后 commit, 第二次 reserve 才返 Deduped
            _commit_dedup_for_test(PushKind::FactorIC, None);
            let second = v14_gate(PushKind::FactorIC, None);
            assert!(matches!(second, V14Gate::Deduped), "second: {:?}", second);
        } else {
            assert!(matches!(first, V14Gate::Denied(_)), "first: {:?}", first);
        }
    }

    #[test]
    #[serial_test::serial(cooldown_memo)]
    fn daily_report_sub_kind_commit_uses_same_key_and_override_window() {
        _reset_dedup_for_test();
        let event = SignalEvent::new(
            SignalSource::HoldingHealth,
            "daily_report",
            None,
            Local::now(),
            SignalPayload::HoldingHealth(Default::default()),
            Severity::Normal,
        );
        let override_secs = Some(1_800);

        assert_eq!(
            dedup_cooldown(PushKind::DailyReport, false, override_secs),
            Some(Duration::from_secs(1_800))
        );
        commit_dedup_for_event(
            &event,
            PushKind::DailyReport,
            Some("SectorTier"),
            override_secs,
        )
        .unwrap();

        assert_eq!(
            lock_dispatcher(v14_stack().unwrap()).unwrap().reserve(
                &event,
                Some(Duration::from_secs(1_800)),
                Some("SectorTier"),
            ),
            ReserveOutcome::Deduped
        );
        assert_eq!(
            lock_dispatcher(v14_stack().unwrap()).unwrap().reserve(
                &event,
                Some(Duration::from_secs(1_800)),
                Some("CapitalVerify"),
            ),
            ReserveOutcome::Reserved
        );
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
        let a1 = v14_gate(PushKind::HoldingPlan, Some("TEST_CODE_000001"));
        if matches!(a1, V14Gate::Approved(_)) {
            // v15.1 A3: 模拟 push 成功后 commit
            _commit_dedup_for_test(PushKind::HoldingPlan, Some("TEST_CODE_000001"));
            assert!(matches!(
                v14_gate(PushKind::HoldingPlan, Some("TEST_CODE_000001")),
                V14Gate::Deduped
            ));
            // 不同 code 不受影响 (v59 F1 语义保持)
            assert!(matches!(
                v14_gate(PushKind::HoldingPlan, Some("TEST_CODE_000002")),
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
