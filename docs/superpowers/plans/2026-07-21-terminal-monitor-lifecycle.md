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

This initial ready-boundary step was superseded by Task 5's propagated consume
result. The final contract is:

```rust
pub async fn spawn(
    receiver: broadcast::Receiver<EventEnvelope>,
    base_dir: PathBuf,
    retention_days: u32,
) -> Result<JoinHandle<Result<(), JsonlError>>, JsonlError> {
    let writer = Self {
        base_dir,
        retention_days,
    };
    fs::create_dir_all(&writer.base_dir).await?;
    Self::cleanup_expired(&writer.base_dir, writer.retention_days).await?;
    Ok(tokio::spawn(async move { writer.consume(receiver).await }))
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

Final evidence (2026-07-22 Asia/Shanghai): `cargo fmt --all -- --check`, strict
workspace/all-target/all-feature Clippy, the serial full workspace test suite,
compliance, release build and `git diff --check` all exited 0. The library suite
passed 1,797 tests with four explicit ignores; the monitor suite passed 413
tests with one process-helper ignore; all remaining targets passed. Production
freshness validation was read-only and `stock_daily` was current through
2026-07-20 (one completed trading day behind). The deterministic serial coverage
report measured 87,565/108,638 = 80.60% globally and 34,104/35,868 = 95.08%
across the registered core files.

- [x] **Step 2: Run the release canary without the switch**

Run:

```bash
env -u MONITOR_ENABLED ./target/release/monitor --test --review
```

Expected: exit 2 after the strict-review marker because isolated test data has no
real account evidence; no real sink is contacted; output contains neither
`jsonl_writer fatal error` nor `background task failed`.

Final evidence (2026-07-22 09:01 Asia/Shanghai): the release canary ran with an
empty inherited environment, `MONITOR_ENABLED` absent, a fresh isolated test
database and no notification credentials. It entered the strict `--review`
path, failed closed with exit 2 because real account evidence was absent, and
drained the JSONL writer normally without a fatal/background-task marker.

- [ ] **Step 3: Review, PR and merge**

The PR must include Refs, Data-Redlines, OldModules, Threshold-Proof,
Business-Rules, Validation and Rollback. Merge without bypassing required checks,
fetch GitHub `master`, and verify the merge commit. Do not restart the cancelled
48-hour monitor.

### Task 5: Close independent-review lifecycle findings

**Files:**
- Modify: `src/event/jsonl_writer.rs`
- Modify: `src/bin/monitor/main.rs`
- Modify: `tests/monitor_help_isolation.rs`

- [x] **Step 1: Isolate inherited test paths and add RED process cases**

The review process test must remove `EVENT_AUDIT_DIR` and `PUSH_LOG_DIR`. Add a
writer-startup failure process case by creating a regular `data` file under the
isolated root and requiring exit 2 with `event_bus.jsonl initialization failed`.
Add a corrupt `data/test/event_bus/YYYY-MM-DD.jsonl`, run test history for that
date without `MONITOR_ENABLED`, and require exit 1.

- [x] **Step 2: Add RED writer failure tests**

Cover: parent path is a regular file (ready initialization fails); unreadable
base directory makes retention cleanup fail on Unix; replacing the initialized
base directory with a regular file makes the next envelope write fail; and a
capacity-one bus with two publications before the consumer starts reports lag.

- [x] **Step 3: Propagate consume and shutdown results**

Return `JoinHandle<Result<(), JsonlError>>`; use `?` for envelope writes and
convert `RecvError::Lagged` into a terminal `JsonlError`. Add one monitor lifecycle
helper that shuts down the bus, awaits exactly one handle, and converts writer or
join failure to exit 2. The long-running service must select on that handle and
stop immediately if it fails or completes unexpectedly before bus shutdown.

- [x] **Step 4: Remove deep exits and fix history status**

Make strict review return `Result<(), String>` instead of exiting internally.
Route terminal completion through the lifecycle helper. Return exit 1 for either
history query/statistics error, while successful empty history remains exit 0.

- [x] **Step 5: Run focused and full gates again**

Run all writer tests and `monitor_help_isolation`, then repeat fmt, clippy, full
workspace tests, compliance, coverage threshold check, release build and the
unset-switch canary. Request a second independent review; merge only with zero
Critical and zero Important findings.

Gate evidence (2026-07-21 23:44 Asia/Shanghai): writer failure tests 7/7,
BR-141 monitor unit tests 2/2 and process isolation tests 13/13 passed. `cargo
fmt --check`, `cargo clippy --workspace --all-targets --all-features -- -D
warnings`, the full workspace/all-target/all-feature test command, compliance
and `cargo build --release --bin monitor` exited 0. Freshness was 2026-07-20,
one trading day behind. Global line coverage was 86,528/107,512 = 80.48%; core
coverage was 33,434/35,085 = 95.29%. An isolated release canary with
`MONITOR_ENABLED` absent entered the strict review marker, failed closed with
exit 2 for unavailable real account evidence, closed the JSONL receiver
normally, and used no configured external notification sink. Independent
review result is recorded before this checkbox is completed.

### Task 6: Close second-review concurrency and mode findings

**Files:**
- Modify: `src/event/bus.rs`
- Modify: `src/event/cli.rs`
- Modify: `src/bin/monitor/main.rs`
- Modify: `tests/monitor_help_isolation.rs`

- [x] **Step 1: Replace the unsafe sender slot**

Use `RwLock<Option<broadcast::Sender<_>>>` so publish/subscribe/count cannot race
shutdown. Remove the manual `unsafe impl Send/Sync`; a publish that observes the
closed slot must return `Rejected(ShuttingDown)`.

- [x] **Step 2: Test concurrent shutdown**

Run publishers and shutdown on separate threads behind a barrier. Require no
panic, no invalid result, all post-close publishes rejected, and one idempotent
shutdown path.

- [x] **Step 3: Close explicit-mode fallthrough**

Keep registered push/backfill flags known to the event parser, reject
`--v13-diag` without `--test` before runtime initialization, and add a final
guard so no unhandled explicit argument can enter the bare service loop.

- [x] **Step 4: Repeat review and all Gate D evidence**

Run focused tests and all repository gates again, then request independent
review of the final commit range. Critical and Important findings must both be
zero before PR creation.

- [x] **Step 5: Quiesce producers and bound writer drain**

Own every long-running spawned task handle. Remove nested detached notification
producers, cancel and await owned producers before bus close, and give writer
drain a ten-second timeout that aborts and reports exit 2. Unit-test timeout and
all unexpected writer completion classifications.

- [x] **Step 6: Enforce process isolation and audit ownership**

All monitor process tests must construct commands through one helper that clears
`EVENT_AUDIT_DIR` and `PUSH_LOG_DIR`. Document that JSONL is a non-authoritative
replay projection after the existing hash-chained, synced delivery audit; it is
not a replacement for the red-line 2.7 owner.

### Task 7: Close final-review audit and truthful-handler findings

**Files:**
- Modify: `src/event/dispatcher.rs`
- Modify: `src/bin/monitor/main.rs`
- Modify: `src/bin/monitor/dryrun_report.rs`
- Modify: `src/opportunity/news_outcome.rs`
- Modify: `tests/monitor_help_isolation.rs`

- [x] **Step 1: Make the authoritative delivery audit cross-process safe**

Namespace every audit override under test/prod. Hold a per-year OS lock across
full-chain revalidation, append, flush and `sync_data`; reject incomplete tails.
Prove four independent process writers produce one valid chain.

- [x] **Step 2: Extract and exercise the lifecycle supervisor**

Use one injectable state machine for main-loop completion, signal result and
writer completion. Prove producer cancellation precedes bus close/writer drain,
runtime writer failure is terminal, and unexpected main-loop completion fails.

- [x] **Step 3: Make restored one-shot handlers truthful**

Strictly parse and normalize outcome-backfill dates, propagate source/K-line/
write failures, aggregate push-dry-run source/build errors, and keep production
assemblers out of TEST_CODE E2E. Cover malformed/path-traversal dates, corrupt
input, every registered handler marker and exit-code mapping in isolated
processes.

- [x] **Step 4: Clear inherited runtime paths and credentials in process tests**

The shared command builder clears database, audit, dispatcher, review, push,
environment-mode and notification/Magiclaw overrides before each test applies
its explicit isolated values.

- [x] **Step 5: Repeat all gates and obtain a clean independent review**

Repeat focused tests, full workspace tests, compliance, coverage, release build
and canary after these changes. Do not create the PR until independent review
reports zero Critical and zero Important findings.

### Task 8: Complete the authoritative delivery-audit contract

**Files:**
- Modify: `docs/business_rules.md`
- Modify: `docs/superpowers/specs/2026-07-21-terminal-monitor-lifecycle-design.md`
- Modify: `src/event/envelope.rs`
- Modify: `src/event/push_record.rs`
- Modify: `src/event/dispatcher.rs`
- Modify: `src/event/mod.rs`

- [x] **Step 1: Register BR-142 before implementation**

Specify the required audit fields, identity redaction, domain-separated hashes,
fail-closed validation and legacy read-only compatibility in the stable rule
table and design before editing production code.

- [x] **Step 2: Add RED contract and compatibility tests**

Require complete structured delivery fields, redacted identities, v2 chain
domains, and a legacy-parent-to-v2 append. Prove malformed fields and unknown
domains are rejected before append.

- [x] **Step 3: Implement the v2 authoritative audit schema**

Build a redacted authoritative event at the persistence seam, validate the
structured fields through `PushRecord`, and domain-separate identity and record
hashing while retaining legacy verification only for existing rows.

- [x] **Step 4: Repeat Gate C/D, canary and independent review**

Run every mandatory gate on the final tree. Zero Critical and Important
findings are required before PR creation and merge.

### Task 9: Close BR-142 independent-review findings

- [x] **Step 1: Refine the rule before implementation**

Register the closed v2 payload, independently domain-separated subject hash,
recomputable identity hash, validated legacy prefix and one-way legacy-to-v2
transition.

- [x] **Step 2: Add RED downgrade and schema-injection tests**

Prove `legacy -> v2 -> legacy` is rejected, malformed legacy envelopes fail,
unknown v2 payload fields fail, and identity/subject hashes cannot drift.

- [x] **Step 3: Implement and repeat independent review**

Validate every historical row with its schema, enforce a one-way chain upgrade,
and persist only closed, recomputable v2 delivery records.

### Task 10: Make the coverage gate deterministic

- [x] **Step 1: Record the failure evidence and decision**

Default-parallel instrumented runs failed in two different tests that share
process-global environment/database state; the same focused tests and the full
single-thread suite passed. Preserve complete workspace coverage while aligning
the coverage runner with the existing serial release-test gate.

- [x] **Step 2: Serialize CI coverage tests**

Pass `-- --test-threads=1` through `cargo llvm-cov` in the mandatory command and
workflow. Do not add excludes or change thresholds.

- [x] **Step 3: Regenerate and enforce the report**

Require global line coverage >=80% and registered core coverage >=95%, then
retain the JSON report as the CI artifact.

Evidence: the serialized instrumented run completed without the global-state
flakes seen under default parallelism. Threshold enforcement passed at 80.60%
global and 95.08% core coverage; no file exclusions or threshold changes were
introduced. Independent BR-142 review reported READY with zero Critical and zero
Important findings before final release-gate repetition.
