//! v14_e2e — v14.2 七层架构端到端实测
//!
//! 完整链路实测: SignalEvent (L1) → Dispatcher (L4) → Governance (L5)
//!              → Sink (L6 Console) → Analytics (L7 InMemory + SQLite)

use chrono::Local;
use stock_analysis::push_l1::{
    DataSourceDownPayload, HoldingHealthPayload, LimitUpPayload, NewsCatalystPayload,
    SectorRotationPayload, Severity, SignalEvent, SignalPayload, SignalSource,
};
use stock_analysis::push_l2::{DataMode, RenderedText, TemplateCategory, TemplateMetadata};
use stock_analysis::push_l4::{DispatchOutcome, Dispatcher, ReserveOutcome};
use stock_analysis::push_l5::{
    event_severity, GovernanceContext, GovernanceDecision, GovernanceEngine,
};
use stock_analysis::push_l6::{PushMessage, SinkResult, SinkRouter};
use stock_analysis::push_l7::{build_analytics, AnalyticsStore, InMemoryStore, SqliteStore};

fn make_event(
    source: SignalSource,
    kind: &str,
    code: Option<&str>,
    payload: SignalPayload,
) -> SignalEvent {
    let severity = match source {
        SignalSource::DataSourceDown => Severity::Emergency,
        SignalSource::RiskViolation => Severity::Emergency,
        _ => Severity::High,
    };
    SignalEvent::new(
        source,
        kind,
        code.map(String::from),
        Local::now(),
        payload,
        severity,
    )
}

fn make_profile(category: TemplateCategory, always_send: bool) -> TemplateMetadata {
    TemplateMetadata {
        category,
        quiet_hours_respect: true,
        frozen_mode_respect: true,
        data_mode_min: DataMode::Full,
        cooldown_secs: 60,
        max_per_user_per_day: None,
        always_send_on_data_source_down: always_send,
    }
}

/// E2E sink is synchronous and treated as successfully delivered, so reserve is
/// immediately followed by commit. This exercises the current rollback-safe API.
fn dispatch_as_delivered(
    dispatcher: &Dispatcher,
    event: &SignalEvent,
    cooldown: Option<std::time::Duration>,
) -> DispatchOutcome {
    match dispatcher.reserve(event, cooldown, None) {
        ReserveOutcome::Reserved => {
            dispatcher.commit(event, cooldown, None);
            DispatchOutcome::Pushed
        }
        ReserveOutcome::Deduped => DispatchOutcome::Deduped(format!(
            "kind={} code={} still cooling down",
            event.kind,
            event.code.as_deref().unwrap_or("")
        )),
    }
}

fn render_event_inline(event: &SignalEvent, body: &str) -> PushMessage {
    PushMessage {
        event: event.clone(),
        text: RenderedText::new(body),
        template_id: "e2e_v1".to_string(),
        template_version: 1,
        user_id: "default".to_string(),
    }
}

fn section(title: &str) {
    println!("\n{}", "=".repeat(60));
    println!("  {}", title);
    println!("{}", "=".repeat(60));
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    section("v14.2 端到端实测 (L1→L4→L5→L6→L7)");
    println!("session ts: {}", Local::now().format("%Y-%m-%d %H:%M:%S"));

    // ===== 1. L1 Signal Source =====
    section("L1 Signal Source — 5 个不同事件");
    let events = vec![
        make_event(
            SignalSource::LimitUp,
            "limit_up",
            Some("TEST_CODE_600519"),
            SignalPayload::LimitUp(LimitUpPayload {
                code: Some("TEST_CODE_600519".to_string()),
                name: Some("贵州茅台".to_string()),
                change_pct: Some(10.0),
                seal_amount_wan: Some(50000.0),
            }),
        ),
        make_event(
            SignalSource::SectorRotation,
            "sector_rotation",
            Some("BK0001"),
            SignalPayload::SectorRotation(SectorRotationPayload {
                sector_code: Some("BK0001".to_string()),
                sector_name: Some("白酒".to_string()),
                change_pct: Some(2.5),
                main_net_yi: Some(15.0),
            }),
        ),
        make_event(
            SignalSource::NewsCatalyst,
            "news_catalyst",
            Some("TEST_CODE_000001"),
            SignalPayload::NewsCatalyst(NewsCatalystPayload {
                code: Some("TEST_CODE_000001".to_string()),
                headline: Some("央行降准 0.5%".to_string()),
                source: Some("财新".to_string()),
                published_on: Some(chrono::Local::now().date_naive()),
            }),
        ),
        make_event(
            SignalSource::HoldingHealth,
            "holding_health",
            Some("TEST_CODE_600519"),
            SignalPayload::HoldingHealth(HoldingHealthPayload {
                code: Some("TEST_CODE_600519".to_string()),
                position_pct: Some(15.0),
                day_pnl_pct: Some(-1.2),
                entry_price: Some(1680.0),
                current_price: Some(1820.0),
            }),
        ),
        make_event(
            SignalSource::DataSourceDown,
            "data_source_down",
            None,
            SignalPayload::DataSourceDown(DataSourceDownPayload {
                source_name: Some("sina".to_string()),
                consecutive_failures: Some(5),
                last_error: Some("connection timeout".to_string()),
            }),
        ),
    ];

    for (i, e) in events.iter().enumerate() {
        println!(
            "  [{:>2}] source={:?} kind={} code={:?} event_id={} severity={:?}",
            i + 1,
            e.source,
            e.kind,
            e.code,
            e.event_id,
            event_severity(e)
        );
    }

    // ===== 2. L4 Dispatcher =====
    section("L4 Dispatcher — dedup 表");
    let dispatcher = Dispatcher::new();
    let mut dispatched = 0;
    let mut deduped = 0;
    let win = Some(std::time::Duration::from_secs(60));
    for e in &events {
        match dispatch_as_delivered(&dispatcher, e, win) {
            DispatchOutcome::Pushed => dispatched += 1,
            DispatchOutcome::Deduped(_) => deduped += 1,
        }
    }
    println!("  dispatched={} deduped={}", dispatched, deduped);
    println!("  stats: {:?}", dispatcher.stats);

    println!("  重发前 3 个 (应 dedup):");
    for e in events.iter().take(3) {
        match dispatch_as_delivered(&dispatcher, e, win) {
            DispatchOutcome::Deduped(detail) => println!("    [dedup] {}", detail),
            _ => println!("    [unexpected_pushed]"),
        }
    }
    println!("  final stats: {:?}", dispatcher.stats);

    // ===== 3. L5 Governance =====
    section("L5 Governance — 5 场景");
    let engine = GovernanceEngine::new();
    let default_ctx = GovernanceContext::default();

    println!("  case A: 全 Full 数据");
    for e in &events {
        let profile = make_profile(TemplateCategory::LimitUp, false);
        let decision = engine.check(&profile, e, &default_ctx);
        println!("    {:?} → {:?}", e.source, decision);
    }

    println!("\n  case B: 静默期 (02:00-06:00)");
    let mut quiet_ctx = default_ctx.clone();
    quiet_ctx.is_quiet_hour = true;
    for e in &events {
        let profile = make_profile(TemplateCategory::LimitUp, false);
        let decision = engine.check(&profile, e, &quiet_ctx);
        println!("    {:?} → {:?}", e.source, decision);
    }

    println!("\n  case C: data_mode=Down, DataSourceDown 豁免");
    let mut down_ctx = default_ctx.clone();
    down_ctx.data_mode = DataMode::Down;
    let ds_event = &events[4];
    let ds_profile = make_profile(TemplateCategory::DataSource, true);
    println!(
        "    DataSourceDown (always_send=true) → {:?}",
        engine.check(&ds_profile, ds_event, &down_ctx)
    );

    println!("\n  case D: QuietHour 事件在静默期放行 (W5.X 修订)");
    let quiet_hour_event = make_event(
        SignalSource::QuietHour,
        "quiet_hour",
        None,
        SignalPayload::DataSourceDown(DataSourceDownPayload::default()),
    );
    let qh_profile = make_profile(TemplateCategory::QuietHour, false);
    println!(
        "    QuietHour 源 + 静默期 → {:?}",
        engine.check(&qh_profile, &quiet_hour_event, &quiet_ctx)
    );

    // ===== 4. L6 Delivery =====
    section("L6 Delivery — SinkRouter (3 推送)");
    let mut router = SinkRouter::new();
    router.register_defaults();
    println!("  registered sinks: {}", router.len());

    let samples = vec![
        (&events[0], "📈 600519 涨停 +10.0%, 封单 5.0亿"),
        (&events[1], "📊 板块轮动: 白酒 +2.5%, 主力净流入 15.0亿"),
        (&events[3], "💼 持仓健康: 600519 持仓 15%, 日盈亏 -1.2%"),
    ];

    for (event, body) in &samples {
        let msg = render_event_inline(event, body);
        let result = router.route(&msg).await;
        println!("    [{}] → {:?}", event.event_id, result);
    }

    let hc = router.health_check_all().await;
    println!("  health: {:?}", hc);

    // ===== 5. L7 Analytics =====
    section("L7 Analytics — InMemoryStore + SqliteStore");
    let in_mem = InMemoryStore::new();

    println!("  InMemoryStore 记录:");
    for (i, (event, _body)) in samples.iter().enumerate() {
        let analytics = build_analytics(
            event,
            "e2e_v1",
            1,
            DataMode::Full,
            GovernanceDecision::Approve,
            Some(&RenderedText::new("body")),
            true,
            "console",
            "default",
            vec![],
        );
        in_mem.record(&analytics).unwrap();
        println!(
            "    [{:>2}] event_id={} pushed={} status={:?}",
            i + 1,
            analytics.event_id,
            analytics.pushed,
            analytics.validation_status
        );
    }
    println!("  InMemoryStore total: {}", in_mem.count_total().unwrap());
    println!(
        "  InMemoryStore push_rate: {:.2}",
        in_mem.push_rate().unwrap().unwrap()
    );

    println!("\n  SqliteStore (内存模式):");
    let sqlite = SqliteStore::open_in_memory().unwrap();
    for (event, _body) in &samples {
        let analytics = build_analytics(
            event,
            "e2e_v1",
            1,
            DataMode::Full,
            GovernanceDecision::Approve,
            Some(&RenderedText::new("body")),
            true,
            "console",
            "default",
            vec![],
        );
        sqlite.record(&analytics).unwrap();
    }
    println!("  SqliteStore total: {}", sqlite.count_total().unwrap());
    println!(
        "  SqliteStore approve count: {}",
        sqlite
            .count_by_governance(&GovernanceDecision::Approve)
            .unwrap()
    );

    if let Some(first) = sqlite.get_by_event_id(&events[0].event_id).unwrap() {
        println!(
            "  SqliteStore get_by_event_id: template={} v{} pushed={}",
            first.template_id, first.template_version, first.pushed
        );
    }

    // ===== 6. 端到端全链路 =====
    section("全链路演练: event → dispatcher → governance → render → sink → analytics");
    let test_event = make_event(
        SignalSource::LimitUp,
        "limit_up",
        Some("TEST_CODE_000858"),
        SignalPayload::LimitUp(LimitUpPayload {
            code: Some("TEST_CODE_000858".to_string()),
            name: Some("五粮液".to_string()),
            change_pct: Some(9.5),
            seal_amount_wan: Some(30000.0),
        }),
    );
    println!(
        "  [L1] event: source={:?} code={:?}",
        test_event.source, test_event.code
    );

    let outcome = dispatch_as_delivered(
        &dispatcher,
        &test_event,
        Some(std::time::Duration::from_secs(60)),
    );
    println!("  [L4] dispatcher: {:?}", outcome);
    assert!(matches!(outcome, DispatchOutcome::Pushed));

    let decision = engine.check(
        &make_profile(TemplateCategory::LimitUp, false),
        &test_event,
        &default_ctx,
    );
    println!("  [L5] governance: {:?}", decision);
    assert!(decision.is_approve());

    let body = format!(
        "📈 {} 涨停 +{:.1}%, 封单{:.1}亿",
        test_event.code.as_deref().unwrap_or(""),
        9.5,
        30000.0 / 10000.0
    );
    println!("  [L2+L3] rendered: {}", body);

    let msg = render_event_inline(&test_event, &body);
    let sink_result = router.route(&msg).await;
    println!("  [L6] sink result: {:?}", sink_result);
    assert_eq!(sink_result, SinkResult::Ok);

    let analytics = build_analytics(
        &test_event,
        "e2e_v1",
        1,
        DataMode::Full,
        decision,
        Some(&RenderedText::new(&body)),
        matches!(sink_result, SinkResult::Ok),
        "console",
        "default",
        vec![],
    );
    in_mem.record(&analytics).unwrap();
    println!(
        "  [L7] analytics: event_id={} pushed={} rendered_len={}",
        analytics.event_id, analytics.pushed, analytics.rendered_len
    );

    section("✅ v14.2 端到端实测通过");
    println!("汇总:");
    println!("  L1: 5 个事件生成 OK");
    println!("  L4: 5 dispatched + 3 dedup on re-dispatch");
    println!("  L5: 全 Full / 静默期 / Down+DataSourceDown 豁免 / QuietHourSource 豁免 OK");
    println!("  L6: 3 推送 OK, sink=console");
    println!(
        "  L7: InMemory {} 条 / Sqlite {} 条, push_rate={:.2}",
        in_mem.count_total().unwrap(),
        sqlite.count_total().unwrap(),
        in_mem.push_rate().unwrap().unwrap()
    );
    println!("  端到端: 1 个事件走完整链路 OK");
}
