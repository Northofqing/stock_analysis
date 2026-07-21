# Terminal Monitor Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every explicit monitor CLI command execute truthfully without `MONITOR_ENABLED`, while making event JSONL initialization ready and fallible before background consumption starts.

**Architecture:** Treat `MONITOR_ENABLED` as a bare-service lifecycle gate evaluated before runtime components. Change `JsonlWriter::spawn` into an async ready boundary that performs directory and retention initialization before returning one consumer task, then propagate initialization failure through monitor exit status.

**Tech Stack:** Rust, Tokio broadcast/tasks/fs, process integration tests, existing event JSONL and monitor CLI infrastructure.

---

## File Map

- Modify `tests/monitor_help_isolation.rs`: process regression for an unset service switch.
- Modify `src/bin/monitor/main.rs`: pure bare-service predicate, early enable gate, and awaited writer initialization.
- Modify `src/event/jsonl_writer.rs`: ready initialization followed by one consumer task.
- Modify `docs/business_rules.md`: BR-141 is already registered by the design commit.
- Create `docs/superpowers/specs/2026-07-21-terminal-monitor-lifecycle-design.md`: already committed design authority.

### Task 1: Lock the false-success regression

**Files:**
- Modify: `tests/monitor_help_isolation.rs`
- Modify: `src/bin/monitor/main.rs` tests

- [x] **Step 1: Add the failing process test**

Add a test that creates an isolated working directory and database path, removes
`MONITOR_ENABLED`, and executes:

```rust
let output = Command::new(env!("CARGO_BIN_EXE_monitor"))
    .args(["--test", "--review"])
    .current_dir(&root)
    .env("DATABASE_PATH", &database_path)
    .env("MAGICLAW_DB_PATH", &database_path)
    .env("STOCK_LIST", "TEST_CODE_000001")
    .env("STOCK_ENV_MODE", "test")
    .env("V10_DRY_RUN_PUSH", "1")
    .env("STOCK_ANALYSIS_QUIET_HOUR_OVERRIDE", "1")
    .env_remove("MONITOR_ENABLED")
    .env_remove("ALERT_WEBHOOK_URL")
    .env_remove("WECHAT_WEBHOOK")
    .output()
    .expect("run isolated strict review without service switch");
```

Require status 2, output containing `[复盘] --review 终端模式启动`, and output
not containing either `[jsonl_writer] fatal error` or `background task failed`.
Remove the isolated directory after assertions.

- [x] **Step 2: Add the pure gate test before its implementation**

In monitor's unit-test module, assert the intended distinction:

```rust
#[test]
fn br141_only_bare_monitor_requires_service_enablement() {
    assert!(service_enablement_required(&["monitor".to_string()]));
    for argument in ["--test", "--review", "--history", "--unknown"] {
        assert!(!service_enablement_required(&[
            "monitor".to_string(),
            argument.to_string(),
        ]));
    }
}
```

- [x] **Step 3: Run RED**

Run:

```bash
cargo test --test monitor_help_isolation review_command_runs_without_service_enablement -- --exact
cargo test --bin monitor br141_only_bare_monitor_requires_service_enablement -- --exact
```

Expected: the process test reports status 0 instead of 2, and the unit test does
not compile because `service_enablement_required` does not exist.

- [x] **Step 4: Commit RED tests**

```bash
git add tests/monitor_help_isolation.rs src/bin/monitor/main.rs
git commit -m "test: expose terminal monitor false success"
```

### Task 2: Make the JSONL writer ready before returning

**Files:**
- Modify: `src/event/jsonl_writer.rs`
- Modify: `src/bin/monitor/main.rs`

- [x] **Step 1: Change `JsonlWriter::spawn` to an async ready boundary**

Use this contract:

```rust
pub async fn spawn(
    receiver: broadcast::Receiver<EventEnvelope>,
    base_dir: PathBuf,
    retention_days: u32,
) -> Result<JoinHandle<()>, JsonlError> {
    let writer = Self {
        base_dir,
        retention_days,
    };
    fs::create_dir_all(&writer.base_dir).await?;
    Self::cleanup_expired(&writer.base_dir, writer.retention_days).await?;
    Ok(tokio::spawn(async move {
        if let Err(error) = writer.consume(receiver).await {
            log::error!("[jsonl_writer] fatal error: {error}");
        }
    }))
}
```

Rename the existing `run` loop to `consume` and remove directory creation and
retention cleanup from that loop. Do not change record format, replay filtering,
append behavior, or retention duration.

- [x] **Step 2: Update writer unit tests**

Every unit test must await initialization explicitly:

```rust
let handle = JsonlWriter::spawn(rx, dir.clone(), 7)
    .await
    .expect("initialize test JSONL writer");
```

Keep the existing assertions for one-envelope-per-line, replay exclusion and
retention cleanup.

- [x] **Step 3: Remove the nested monitor task**

Replace the outer `tokio::spawn` with one awaited initialization:

```rust
let _jsonl_writer_handle = match stock_analysis::event::JsonlWriter::spawn(
    bus.subscribe(),
    runtime_data_path(test_mode, "event_bus"),
    1_827,
)
.await
{
    Ok(handle) => handle,
    Err(error) => {
        log::error!("[event_bus.jsonl] initialization failed: {error}");
        log::logger().flush();
        std::process::exit(2);
    }
};
```

- [x] **Step 4: Run focused writer tests**

Run:

```bash
cargo test event::jsonl_writer::tests -- --test-threads=1
```

Expected: all JSONL append, replay-filter and cleanup tests pass.

- [x] **Step 5: Commit writer lifecycle**

```bash
git add src/event/jsonl_writer.rs src/bin/monitor/main.rs
git commit -m "fix: await event writer initialization"
```

### Task 3: Restrict service enablement to the bare monitor

**Files:**
- Modify: `src/bin/monitor/main.rs`

- [x] **Step 1: Implement the pure predicate**

```rust
fn service_enablement_required(args: &[String]) -> bool {
    args.len() == 1
}
```

- [x] **Step 2: Evaluate the gate before runtime initialization**

Immediately after test-mode setup and the side-effect-free help branch, add:

```rust
if service_enablement_required(&startup_args) && !check_enabled() {
    log::info!("[monitor] disabled: MONITOR_ENABLED is not true");
    return;
}
```

Delete the later unconditional `if !check_enabled() { return; }` after the
event writer has started. Explicit CLI arguments now continue into their normal
parser and failure path.

- [x] **Step 3: Run GREEN regression tests**

Run:

```bash
cargo test --test monitor_help_isolation review_command_runs_without_service_enablement -- --exact
cargo test --bin monitor br141_only_bare_monitor_requires_service_enablement -- --exact
cargo test event::jsonl_writer::tests -- --test-threads=1
```

Expected: all pass; the process test gets status 2 and no background writer
failure marker.

- [x] **Step 4: Commit gate fix**

```bash
git add src/bin/monitor/main.rs tests/monitor_help_isolation.rs
git commit -m "fix: run terminal commands outside service gate"
```

### Task 4: Release gates and isolated canary

**Files:**
- Modify: this plan only to record exact final evidence.

- [x] **Step 1: Run repository gates**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
STOCK_DB=/Users/zhangzhen/Desktop/Quant/stock_analysis/data/stock_analysis.db bash tools/compliance/check.sh
cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
git diff --check
```

Expected: every command exits 0; global coverage remains at least 80%, core at
least 95%, and no production database is changed by validation.

Evidence (2026-07-21): focused process tests 11/11 passed; JSONL writer tests
3/3 passed; fmt, clippy, full workspace/all-target/all-feature tests, compliance,
and release build exited 0. Production freshness was read-only and latest
`stock_daily` was 2026-07-20 (one trading day behind). Global line coverage was
86,295/107,267 = 80.45%; registered core coverage was 33,329/34,978 = 95.29%.

- [x] **Step 2: Run the release canary without the switch**

Run:

```bash
env -u MONITOR_ENABLED ./target/release/monitor --test --review
```

Expected: exit 2 after the strict-review marker because isolated test data has no
real account evidence; no real sink is contacted; output contains neither
`jsonl_writer fatal error` nor `background task failed`.

Evidence (2026-07-21 23:05 Asia/Shanghai): release canary entered the strict
`--review` path with `MONITOR_ENABLED` absent, used the isolated test database,
returned 2 for zero confirmed review delivery, and emitted neither writer-fatal
marker. Test mode kept external notification delivery in dry-run isolation.

- [ ] **Step 3: Review, PR and merge**

The PR must include Refs, Data-Redlines, OldModules, Threshold-Proof,
Business-Rules, Validation and Rollback. Merge without bypassing required checks,
fetch GitHub `master`, and verify the merge commit. Do not restart the cancelled
48-hour monitor.
