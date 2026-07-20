# Monitor Notification Liveness Repair Implementation Plan

**Upstream debt** (from git log and current callers):

- PR #5 restored independent intraday scheduling but did not repair the startup notification
  bootstrap cycle. No deletion severed the AccountMode/DataMode producers: current multiline-aware
  tracing shows `evaluate_account_mode_hook -> push_account_mode_change` and
  `evaluate_data_mode_hook -> push_data_mode_change` in `main.rs`.
- The pre-existing AccountMode audit can contain `pushed=0`; those rows are delivery debt and must
  be retried by original row ID, not replaced or treated as acknowledged.

**Rename impact**:

- No identifier is renamed. `PortfolioMetrics` is nevertheless a source-breaking public shape
  change: its three numeric fields become `Option`, so every downstream constructor/comparison must
  use `complete`, `incomplete`, or explicit option handling.
- `BannerCtx` removes production `Default` and adds `account_metrics_complete`; all monitor,
  v14-adapter, renderer, and test constructors must state whether all three account facts exist.
- `push_account_mode_change` changes from `Result<bool, String>` to a typed delivery result; the
  main hook and all tests must distinguish no-change, pushed, deduped, denied, and sink failure.

**Production evidence** (DONE criteria):

- Push artifact: `grep -lE '账户模式变更|数据模式变更' data/push_log/${DATE}/*.md | head -3`
  must find at least one post-restart state notification without printing its content.
- L7 delivery: `sqlite3 data/push_analytics.db "SELECT COUNT(*) FROM push_analytics WHERE substr(ts,1,10)='${DATE}' AND template_id IN ('account_mode','data_mode') AND pushed=1;"`
  must return at least `1`.
- Immutable delivery audit: `grep -c '"event_type":"push.delivery.audit"' data/event_audit/${DATE}.jsonl`
  must be non-zero.
- If a producer remains unavailable, startup/runtime logs must contain the explicit
  `[AccountMode-hook][BR-108]` or `[DataMode-hook][BR-116]` retry marker; silence is failure.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore truthful, audited operational notifications when account facts or market data are unavailable, and repair the live Eastmoney announcement detail protocol without adding fallback data.

**Architecture:** Make account metrics optional at the domain and banner boundaries, retain conservative risk modes, and keep paper/trading context fail-closed unless every required account metric is present. Model DataMode alerts as data-source-down events whose notification state advances only after confirmed delivery, reuse pending AccountMode audit rows on retry, and strictly consume the current Eastmoney detail endpoint.

**Tech Stack:** Rust, Tokio, Diesel/SQLite, reqwest, serde, existing v14 L4/L5/L7 push stack, cargo llvm-cov.

---

## File map

- `src/risk/account_mode.rs`: truthful optional account metric model and conservative evaluation.
- `src/bin/monitor/push_templates.rs`: optional banner rendering, fallible paper context, and mode-notification result contracts.
- `src/bin/monitor/main.rs`: assemble explicit incomplete state, retry mode notifications, and keep paper work fail-closed.
- `src/database/account_snapshot.rs`: reuse immutable real-account capture/observation timestamps as the only eligible account freshness evidence.
- `src/push_l5/governance.rs`: keep clock failures explicit instead of substituting epoch zero.
- `src/bin/monitor/v14_adapter.rs`: correct DataMode event payload/profile and AccountMode status eligibility.
- `src/database/account_mode_log.rs`: reuse the existing immutable pending audit row through its public row contract.
- `src/data_provider/announcement.rs`: current official detail endpoint and strict response parser.
- `tests/monitor_help_isolation.rs`: process-level fail-closed assertion follows the new explicit
  AccountMode startup boundary after the banner bootstrap repair.
- `docs/business_rules.md`: BR-105/108/113/116 registration (completed before code).
- `docs/superpowers/specs/2026-07-20-monitor-notification-liveness-design.md`: Gate A design.

### Task 1: Represent missing account facts without numeric sentinels

**Files:**
- Modify: `src/risk/account_mode.rs`
- Modify: `src/bin/monitor/push_templates.rs`
- Modify: `src/bin/monitor/main.rs`

- [ ] **Step 1: Write the failing account-domain test**

Add one public-behavior test proving absent metrics select `ReduceOnly` and remain absent:

```rust
#[test]
fn incomplete_metrics_are_explicit_and_conservative() {
    let metrics = PortfolioMetrics::incomplete();
    assert!(!metrics.is_complete());
    let result = evaluate(&metrics, Some(AccountMode::Normal), &ModeThresholds::default());
    assert_eq!(result.mode, AccountMode::ReduceOnly);
    assert_eq!(metrics.today_pnl_pct, None);
    assert_eq!(metrics.consecutive_stop_loss_n, None);
    assert_eq!(metrics.total_pos_cheng, None);
}
```

- [ ] **Step 2: Run RED**

```bash
cargo test --lib risk::account_mode::tests::incomplete_metrics_are_explicit_and_conservative -- --exact
```

Expected: compile failure because the optional fields and constructors do not exist.

- [ ] **Step 3: Implement the optional metric contract**

Use this shape and pattern-match before all numeric comparisons:

```rust
#[derive(Clone, Debug, Default)]
pub struct PortfolioMetrics {
    pub today_pnl_pct: Option<f64>,
    pub consecutive_stop_loss_n: Option<u32>,
    pub total_pos_cheng: Option<u8>,
}

impl PortfolioMetrics {
    pub fn complete(today_pnl_pct: f64, consecutive_stop_loss_n: u32, total_pos_cheng: u8) -> Self {
        Self {
            today_pnl_pct: Some(today_pnl_pct),
            consecutive_stop_loss_n: Some(consecutive_stop_loss_n),
            total_pos_cheng: Some(total_pos_cheng),
        }
    }

    pub fn incomplete() -> Self {
        Self::default()
    }

    pub fn is_complete(&self) -> bool {
        self.today_pnl_pct.is_some()
            && self.consecutive_stop_loss_n.is_some()
            && self.total_pos_cheng.is_some()
    }
}
```

In `evaluate`, return the registered incomplete-data result before unwrapping the tuple. Update all complete fixtures/callers to `PortfolioMetrics::complete(...)`.

- [ ] **Step 4: Write the failing banner and BR-134 tests**

```rust
#[test]
fn incomplete_banner_renders_missing_account_facts() {
    let banner = BannerCtx {
        account_mode: AccountMode::ReduceOnly,
        total_pos: None,
        today_pnl: None,
        account_metrics_complete: false,
        data_mode: DataMode::Unsafe,
        data_missing_note: Some("账户指标缺失".to_string()),
    };
    let text = banner.render();
    assert!(text.contains("仓位缺失"));
    assert!(text.contains("日盈亏缺失"));
}

#[test]
fn br134_incomplete_banner_cannot_create_paper_risk_context() {
    let banner = BannerCtx {
        account_mode: AccountMode::ReduceOnly,
        total_pos: None,
        today_pnl: None,
        account_metrics_complete: false,
        data_mode: DataMode::Unsafe,
        data_missing_note: Some("账户指标缺失".to_string()),
    };
    assert!(paper_risk_context_from_banner(&banner).is_err());
}
```

- [ ] **Step 5: Run RED**

```bash
cargo test --bin monitor incomplete_banner -- --nocapture
```

Expected: compile failure because banner numeric fields are not optional and paper conversion is infallible.

- [ ] **Step 6: Implement optional banner rendering and fallible paper conversion**

Change `BannerCtx.total_pos` and `today_pnl` to `Option`. Render each independently:

```rust
let position = self.total_pos.map_or_else(|| "仓位缺失".to_string(), |v| format!("仓位{v}成"));
let pnl = self.today_pnl.map_or_else(|| "日盈亏缺失".to_string(), |v| format!("日盈亏{v:+.1}%"));
```

Return `Result<PaperRiskContext, String>` from `paper_risk_context_from_banner`; reject unless both displayed account fields are present and the banner records that all three underlying metrics (including consecutive losses) were complete in the same evaluation. Propagate the error from D-01 and return `false` before the post-close paper loop. Add `current_paper_risk_context_for` in `main.rs` so the v16.3 background task logs one explicit BR-134 refusal path.

Build complete banners with `Some(...)` and `account_metrics_complete=true`; incomplete or partial
banners retain each real `Some(...)` fact but set `account_metrics_complete=false`. Account-mode
audit arguments use the metric options directly and set `data_complete=metrics.is_complete()`.

Use a single `ModeEvaluation`, created with one `Local::now().time()` sample, for banner assembly,
audit planning, persistence, rendering, and delivery. `push_account_mode_change` accepts that
evaluation and must not sample the clock or call `evaluate` again. Add an 08:30:59 deterministic
test proving persisted mode, banner mode, and paper context agree. Validate persisted `pushed`
strictly as `0` or `1`; reject all other values.

Load only `real_account_snapshot` as account freshness evidence and apply its exact 30-second gate.
Require explicit available daily PnL and position ratio. Do not treat ledger date/write time as a
broker timestamp. Until a same-batch broker trade-sync watermark exists, reject complete metrics
and keep the conservative state notification path active.

- [ ] **Step 7: Run focused GREEN and commit**

```bash
cargo fmt --all
cargo test --lib risk::account_mode
cargo test --bin monitor incomplete_banner
cargo test --bin monitor br134
git add src/risk/account_mode.rs src/bin/monitor/main.rs src/bin/monitor/push_templates.rs
git commit -m "fix: preserve missing account facts in monitor state"
```

Expected: focused tests pass; no account numeric fallback remains.

### Task 2: Retry AccountMode notifications and bootstrap a conservative banner

**Files:**
- Modify: `src/bin/monitor/main.rs`
- Modify: `src/bin/monitor/push_templates.rs`
- Test: existing monitor unit tests in those files

- [ ] **Step 1: Write a failing pure planning test**

Extract a pure helper whose observable decision is one of `NoChange`, `Insert`, or
`ReusePending(i64)`. Add:

```rust
#[test]
fn unchanged_unpushed_account_mode_reuses_the_pending_audit_row() {
    let latest = stock_analysis::database::account_mode_log::AccountModeLogRow {
        id: 41,
        ts: "TEST_CODE_2026-07-20 11:00:00".to_string(),
        prev_mode: "Normal".to_string(),
        new_mode: "ReduceOnly".to_string(),
        trigger_reason: "TEST_CODE_account facts unavailable".to_string(),
        today_pnl_pct: None,
        consecutive_n: None,
        total_pos_cheng: None,
        data_complete: 0,
        pushed: 0,
        push_attempted_at: None,
    };
    let plan = account_mode_notification_plan(AccountMode::ReduceOnly, Some(&latest)).unwrap();
    assert_eq!(plan, AccountModeNotificationPlan::ReusePending(41));
}
```

Construct the row with `TEST_CODE` only in free-text fields; all numeric facts remain `None`.

- [ ] **Step 2: Run RED**

```bash
cargo test --bin monitor push_templates::tests::br116_failed_account_notification_reuses_pending_audit_row -- --exact
```

Expected: compile failure because the notification plan does not exist.

- [ ] **Step 3: Implement pending-row reuse**

Change `push_account_mode_change` to accept `Option<&AccountModeLogRow>`. Parse both persisted mode labels strictly. The plan is:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccountModeNotificationPlan {
    NoChange,
    Insert,
    ReusePending(i64),
}

fn account_mode_notification_plan(
    evaluated_mode: LibAM,
    latest: Option<&AccountModeLogRow>,
) -> Result<AccountModeNotificationPlan, String> {
    let Some(row) = latest else {
        return Ok(AccountModeNotificationPlan::Insert);
    };
    let persisted_mode = parse_account_mode_label(&row.new_mode)?;
    if row.pushed == 0 && persisted_mode == evaluated_mode {
        return Ok(AccountModeNotificationPlan::ReusePending(i64::from(row.id)));
    }
    if persisted_mode != evaluated_mode {
        return Ok(AccountModeNotificationPlan::Insert);
    }
    Ok(AccountModeNotificationPlan::NoChange)
}
```

For `ReusePending`, render from the persisted transition/reason and do not insert a second row. For `Insert`, persist metric options and `data_complete`. Mark exactly that row pushed only after real `Pushed`; failure leaves it at `pushed=0` for the next loop.

- [ ] **Step 4: Convert account assembly errors to explicit incomplete state**

In `evaluate_account_mode_hook`, retain the error log but continue with `PortfolioMetrics::incomplete()`. Read the latest row, evaluate through the existing rule, build/store a banner from optional metrics and real DataMode, then call the retry-aware orchestrator. The 08:30 reset is allowed only when `metrics.is_complete()`; incomplete facts preserve an existing Frozen state. Invalid mode labels, DB lookup failure, data-health failure, or lock failure still return `false`.

```rust
let metrics = match tokio::task::spawn_blocking(compute_account_mode_metrics_blocking).await {
    Ok(Ok(metrics)) => metrics,
    Ok(Err(error)) => {
        log::warn!("[AccountMode-hook] metrics unavailable, conservative state retained: {error}");
        PortfolioMetrics::incomplete()
    }
    Err(error) => {
        log::warn!("[AccountMode-hook] metrics task failed, conservative state retained: {error}");
        PortfolioMetrics::incomplete()
    }
};
let latest = match latest_account_mode_change() {
    Ok(latest) => latest,
    Err(error) => {
        log::error!("[AccountMode-hook] state lookup failed: {error}");
        return false;
    }
};
let previous = match latest
    .as_ref()
    .map(|row| parse_account_mode_label(&row.new_mode))
    .transpose()
{
    Ok(previous) => previous,
    Err(error) => {
        log::error!("[AccountMode-hook] invalid persisted state: {error}");
        return false;
    }
};
let evaluated = evaluate(&metrics, previous, &thresholds);
let data_health = match evaluated_data_health() {
    Ok(health) => health,
    Err(error) => {
        log::error!("[AccountMode-hook] data health unavailable: {error}");
        return false;
    }
};
let banner = build_banner(&metrics, evaluated.mode, &data_health);
if let Err(error) = store_banner(banner.clone()) {
    log::error!("[AccountMode-hook] banner store failed: {error}");
    return false;
}
match push_account_mode_change(&metrics, latest.as_ref(), Some(&banner)).await {
    Ok(_) => true,
    Err(error) => {
        log::error!("[AccountMode-hook] notification orchestration failed: {error}");
        false
    }
}
```

- [ ] **Step 5: Run focused GREEN and commit**

```bash
cargo fmt --all
cargo test --bin monitor account_mode -- --nocapture
git add src/bin/monitor/main.rs src/bin/monitor/push_templates.rs
git commit -m "fix: retry conservative account mode notifications"
```

Expected: pending row is reused and incomplete account state produces a truthful banner.

### Task 3: Make initial DataMode failure audible and delivery-confirmed

**Files:**
- Modify: `src/bin/monitor/push_templates.rs`
- Modify: `src/bin/monitor/main.rs`
- Modify: `src/bin/monitor/v14_adapter.rs`
- Modify if validation exposes reused test audit state: `src/event/dispatcher.rs`

- [ ] **Step 1: Write the failing initial-state test**

```rust
#[test]
fn initial_unsafe_data_mode_requires_a_status_delivery() {
    let input = DataHealthInput::default();
    let plan = data_mode_notification_plan(&input, None);
    assert!(matches!(plan, DataModeNotificationPlan::Dispatch { previous: None, current: LibDM::Unsafe, .. }));
}
```

Also assert an initial Full input returns `EstablishSilently`.

- [ ] **Step 2: Run RED**

```bash
cargo test --bin monitor push_templates::tests::initial_unsafe_data_mode_requires_a_status_delivery -- --exact
```

Expected: compile failure because the plan/result contract does not exist.

- [ ] **Step 3: Implement the initial transition and result contract**

Change `render_data_mode` to accept `Option<DataMode>` and render `None` as `未建立`. Introduce:

```rust
pub enum ModeDispatchResult {
    EstablishedSilently,
    Delivery(crate::notify::PushOutcome),
}

impl ModeDispatchResult {
    pub fn is_confirmed(&self) -> bool {
        matches!(self, Self::EstablishedSilently | Self::Delivery(PushOutcome::Pushed))
    }
}
```

`push_data_mode_change` dispatches when the first mode is Degraded/Unsafe or a later mode changes, and calls `dispatch_outcome` so Denied/SinkError remain distinguishable.
DataMode uses no time-based cooldown: its last confirmed state is the exact dedup key. Add a rapid
`Full→Degraded→Unsafe` test proving both distinct transitions are delivered, plus a test proving a
coarse `Deduped` result is not confirmation.

- [ ] **Step 4: Write the failing governance event test**

```rust
#[test]
fn data_mode_alert_is_a_data_source_down_event_with_down_exemption() {
    let event = signal_event_for_kind(PushKind::DataMode, None);
    assert_eq!(event.source, SignalSource::DataSourceDown);
    assert!(matches!(event.payload, SignalPayload::DataSourceDown(_)));
    let profile = default_profile_for_kind(PushKind::DataMode);
    assert_eq!(profile.category, TemplateCategory::DataSource);
    assert!(profile.always_send_on_data_source_down);
}
```

- [ ] **Step 5: Run RED, implement v14 mapping, and keep failed state retryable**

```bash
cargo test --bin monitor v14_adapter::tests::data_mode_alert_is_a_data_source_down_event_with_down_exemption -- --exact
```

Build the v14 event through one helper. For DataMode use `SignalSource::DataSourceDown` and `SignalPayload::DataSourceDown(Default::default())`; other kinds keep their current payload. Set the DataMode category/exemption accordingly and set AccountMode `data_mode_min=Down` because it is a risk-state notification, not a market recommendation.

```rust
fn signal_payload_for_kind(kind: PushKind) -> SignalPayload {
    match kind {
        PushKind::DataMode => SignalPayload::DataSourceDown(Default::default()),
        _ => SignalPayload::HoldingHealth(Default::default()),
    }
}

// `map_push_kind` returns `SignalSource::DataSourceDown` for `PushKind::DataMode`.

let category = match kind {
    PushKind::DataMode => TemplateCategory::DataSource,
    PushKind::AccountMode | PushKind::ForbiddenOps | PushKind::CapitalVerify
        | PushKind::StPriceLimitChanged => TemplateCategory::Risk,
    _ => TemplateCategory::Holding,
};
let data_mode_min = if matches!(kind, PushKind::AccountMode) {
    DataMode::Down
} else {
    DataMode::Degraded
};
let always_send_on_data_source_down = matches!(kind, PushKind::DataMode);
```

In `evaluate_data_mode_hook`, update `LATEST_DATA_MODE` only when `ModeDispatchResult::is_confirmed()` is true. Denied/SinkError leaves the prior mode unchanged for retry.

- [ ] **Step 6: Run focused GREEN and commit**

```bash
cargo fmt --all
cargo test --bin monitor data_mode -- --nocapture
cargo test --bin monitor v14_adapter -- --nocapture
git add src/bin/monitor/main.rs src/bin/monitor/push_templates.rs src/bin/monitor/v14_adapter.rs
git commit -m "fix: deliver and retry monitor state alerts"
```

Expected: initial Unsafe dispatch is eligible under Down governance and state advances only on confirmation.

If the delivery-confirmation test encounters a stale hash chain from a reused OS PID, make the
runtime-detected Cargo test process use a process-instance-unique BR-091 audit directory. Library
unit tests and binary/integration tests compile with different `cfg(test)` boundaries, so both must
use the same runtime detector. Production audit paths and fail-closed semantics remain unchanged.

### Task 4: Repair the strict Eastmoney announcement detail protocol

**Files:**
- Modify: `src/data_provider/announcement.rs`

- [ ] **Step 1: Write the failing current-protocol parser test**

```rust
#[test]
fn current_announcement_detail_protocol_requires_identity_and_notice_content() {
    let body = r#"{"success":true,"data":{"art_code":"TEST_CODE_ARTICLE","notice_content":"TEST_CODE_完整正文"}}"#;
    assert_eq!(
        parse_announcement_detail_http_response(200, Ok(body.to_string()), "TEST_CODE_ARTICLE").unwrap(),
        "TEST_CODE_完整正文"
    );
    assert!(parse_announcement_detail_http_response(200, Ok(body.replace("TEST_CODE_ARTICLE", "TEST_CODE_OTHER")), "TEST_CODE_ARTICLE").is_err());
    assert!(parse_announcement_detail_http_response(200, Ok(r#"{"success":true,"data":{"art_code":"TEST_CODE_ARTICLE","content":"old"}}"#.to_string()), "TEST_CODE_ARTICLE").is_err());
}
```

- [ ] **Step 2: Run RED**

```bash
cargo test --lib data_provider::announcement::tests::current_announcement_detail_protocol_requires_identity_and_notice_content -- --exact
```

Expected: current parser rejects `notice_content` because it still expects `content`.

- [ ] **Step 3: Implement the current strict protocol**

Add a separate production constant:

```rust
const ANNOUNCE_DETAIL_URL: &str = "https://np-cnotice-stock.eastmoney.com/api/content/ann";
```

Deserialize required `success`, `data.art_code`, and `data.notice_content`. Validate success, exact identity, and non-blank content. Detail requests use query parameters `art_code`, `client_source=web`, and `page_index=1`. Keep the list endpoint unchanged and keep all-or-nothing detail assembly.

- [ ] **Step 4: Update loopback transport test and run GREEN**

Use one loopback server with separate list/detail base URLs and assert the detail requests start with `/api/content/ann?art_code=`. Do not accept the old path or old `content` field.

```bash
cargo fmt --all
cargo test --lib data_provider::announcement -- --nocapture
git add src/data_provider/announcement.rs
git commit -m "fix: update Eastmoney announcement detail protocol"
```

Expected: strict parser and end-to-end loopback batch pass.

### Task 5: Prove Gates, merge, restart, and validate real delivery

**Files:**
- Modify only if a gate exposes a root-cause defect.
- Keep `/private/tmp/stock_analysis_monitor.log` outside Git with mode `0600`.

- [ ] **Step 1: Run Gate B and C**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
```

Expected: all commands exit 0.

- [ ] **Step 2: Run Gate D and release build**

```bash
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

Expected: global line coverage at least 80%, core at least 95%, release build exits 0.

- [ ] **Step 3: Run the mandatory independent five-step verifier**

Give a fresh verifier the required “do not trust implementer” brief and require these independent
checks, including production evidence and cross-version debt:

```bash
# 1. Module layer
cargo test --lib risk::account_mode::tests::
cargo test --lib data_provider::announcement::tests::
cargo build --lib

# 2. Multiline-aware caller trace
rg -n -A3 'push_(account|data)_mode_change\(' src/bin/monitor --glob '*.rs'

# 3. Release smoke through the production binary entry
cargo build --release --bin monitor
V10_DRY_RUN_PUSH=1 ./target/release/monitor --test 2>&1 | grep -E 'AccountMode-hook|DataMode-hook|announcement'

# 4. Post-restart production evidence (counts only)
DATE=$(date +%Y-%m-%d)
grep -lE '账户模式变更|数据模式变更' data/push_log/${DATE}/*.md | head -3 | wc -l
sqlite3 data/push_analytics.db "SELECT COUNT(*) FROM push_analytics WHERE substr(ts,1,10)='${DATE}' AND template_id IN ('account_mode','data_mode') AND pushed=1;"
grep -c '"event_type":"push.delivery.audit"' data/event_audit/${DATE}.jsonl

# 5. Cross-version debt
rg -n 'is_active_spec_target_|is_legacy_v17_' src/bin/monitor/notify.rs
rg -n -A3 'PushKind::(AccountMode|DataMode)|push_(account|data)_mode_change\(' src/bin/monitor --glob '*.rs'
```

Expected: module/build/smoke commands exit zero; both producers have live main callers; neither
state PushKind is hidden as legacy; post-restart state delivery and immutable audit counts are
non-zero. If the external source is unavailable during smoke, the explicit retry marker is required
and the live post-restart evidence remains the release gate.

- [ ] **Step 4: Review and merge through PR**

Push the branch, create a PR containing every AGENTS field, obtain independent Standards/Spec review, mark ready, merge into `master`, and verify local/remote master commit equality.

- [ ] **Step 5: Restart exactly one monitor process**

Terminate only the current release monitor PID after the merged master release build succeeds. Start:

```bash
umask 077
exec caffeinate -dimsu env RUST_LOG=info RUST_BACKTRACE=1 \
  ./target/release/monitor >> /private/tmp/stock_analysis_monitor.log 2>&1
```

- [ ] **Step 6: Validate without private output**

Use fixed counters and audit aggregates only. Prove:

- no panic/fatal/database-lock/audit failure;
- governance banner unavailable stops increasing after bootstrap;
- an initial account/data status attempt has `Pushed` plus L7 evidence; `Deduped` is not delivery
  confirmation for a newly observed mode;
- announcement detail no longer fails through the obsolete empty-body path and at least one complete batch succeeds when the source supplies valid data;
- private account values, securities, credentials, destination, message content, and raw log never enter Git or console output.

Continue the cumulative 48-hour observation after the restart; only a runtime-blocking regression opens another minimal repair PR.
