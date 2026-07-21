# Announcement Relevance Gate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop low-value market-wide company announcements from becoming immediate notifications while preserving real, actionable holding/watchlist facts and all BR-137 safety/audit gates.

**Architecture:** Parse announcement keywords from an explicit combined-config section, classify lifecycle-only notices with a deterministic pure predicate, and pass the real portfolio/watch universe into the normalized announcement router. Classified but relevance-filtered provider identities remain owned by the normalized route so legacy delivery cannot bypass the filter.

**Review corrections:** The shared production provider holds one exact keyword snapshot through
transport, detail selection, and assembly. Each announcement route rebuilds its audience from a
verified broker position batch no older than 30 seconds plus independently loaded explicit watch
codes; local `stock_position.updated_at` is never accepted as source provenance. Until that broker
batch exists, the unavailable position component is explicitly excluded without blocking the
independent watch set. The router returns a typed disposition per handled identity; only `Pushed`
may feed D-01/I-02, while filtered/failed outcomes block legacy fallback without downstream effects.
Configuration, provider, audience, and name-resolution failures isolate only the announcement
sub-chain; unrelated outer-loop scheduling, reset, persistence, and banner refresh still run. The
production loop uses a phase coordinator as its actual branch seam: Pending/Failed watch readiness
disables only the Announcement phase, while every unrelated phase must enter exactly once or emit an
explicit outer-tick contract failure.

**Tech Stack:** Rust, Tokio, Serde/TOML, `HashSet`, existing BR-137 source-fact governance, SQLite-backed L7/audit pipeline.

---

## File map

- `docs/business_rules.md`: register BR-138 before filter/dedup code changes.
- `config/chain.toml`: place announcement lists under `[announce_keywords]`.
- `src/config.rs`: parse and validate the typed combined announcement section.
- `src/data_provider/announcement.rs`: deterministic lifecycle-value predicate.
- `src/bin/monitor/v17_sources.rs`: explicit audience gate and handled-identity ownership.
- `src/bin/monitor/main.rs`: pass the real portfolio/watch universe and fail closed when config is unavailable.
- `docs/superpowers/specs/2026-07-21-announcement-relevance-design.md`: design and rollback contract.

### Task 1: Register BR-138 and repair the typed configuration seam

**Files:**
- Modify: `docs/business_rules.md`
- Modify: `config/chain.toml`
- Modify: `src/config.rs`
- Modify: `src/data_provider/announcement.rs`
- Test: `src/config.rs`
- Test: `src/data_provider/announcement.rs`

- [ ] **Step 1: Register BR-138 before implementation**

Add a business-rule row that states: immediate company announcements require a real holding/watch
code; procedural creditor notices and completed reduction-plan notices are local-only; filtered
provider identities remain handled to block legacy fallback; `[announce_keywords]` is required and a
parse failure is explicit.

- [ ] **Step 2: Write the failing combined-config test**

```rust
#[test]
fn br138_repository_chain_config_exposes_announcement_section() {
    let content = include_str!("../config/chain.toml");
    let parsed = parse_chain_combined(content).expect("valid combined chain config");
    assert!(parsed.announce_keywords.emergency.contains(&"立案调查".to_string()));
    assert!(parsed.announce_keywords.important.contains(&"股东减持".to_string()));
    assert!(parsed.announce_keywords.positive.contains(&"中标".to_string()));
}
```

- [ ] **Step 3: Run the test and verify RED**

Run: `cargo test --lib br138_repository_chain_config_exposes_announcement_section -- --test-threads=1`
Expected: compile failure because `parse_chain_combined`/the typed section does not exist, or parse
failure because the keys are attached to the final `[[rules]]` item.

- [ ] **Step 4: Implement the typed section and fail-closed loader**

```rust
#[derive(Debug, Clone, Deserialize)]
struct CombinedChainConfig {
    rules: Vec<ChainRuleConfig>,
    announce_keywords: AnnounceKeywordsFile,
    boards: Vec<ExclusionBoardConfig>,
}

fn parse_chain_combined(content: &str) -> Result<CombinedChainConfig, toml::de::Error> {
    toml::from_str(content)
}
```

Change `chain.toml` to open `[announce_keywords]` before `emergency/important/positive`, and parse the
combined value once. Publish all three ArcSwap values only on success; log an error and clear the
announcement section on failure. Expose `announcement_keywords_available() -> bool` for the monitor
loop. Enforce the same check inside the public production `fetch_announcements` boundary so R-08 and
future callers cannot bypass the fail-closed contract; keep injected lower-level transport helpers for
isolated protocol tests only.

- [ ] **Step 5: Run focused tests and commit**

Run: `cargo test --lib br138_repository_chain_config_exposes_announcement_section -- --test-threads=1`
Run: `cargo test --lib br138_public_fetch_rejects_unloaded_keyword_contract_before_transport -- --test-threads=1`
Expected: both focused tests pass.

```bash
git add docs/business_rules.md config/chain.toml src/config.rs src/data_provider/announcement.rs
git commit -m "fix: load announcement keywords from typed config"
```

### Task 2: Add deterministic lifecycle-value filtering

**Files:**
- Modify: `src/data_provider/announcement.rs`
- Test: `src/data_provider/announcement.rs`

- [ ] **Step 1: Write two failing tests from the observed failure classes**

```rust
#[test]
fn br138_procedural_capital_reduction_notice_is_local_only() {
    assert!(!announcement_title_is_immediately_actionable(
        "关于注销部分回购股份并减少注册资本通知债权人的公告"
    ));
}

#[test]
fn br138_completed_reduction_plan_is_local_only_but_new_plan_remains_actionable() {
    assert!(!announcement_title_is_immediately_actionable(
        "持股5%以上股东减持计划期限届满暨实施情况的公告"
    ));
    assert!(announcement_title_is_immediately_actionable(
        "控股股东拟减持股份的预披露公告"
    ));
}
```

- [ ] **Step 2: Run the tests and verify RED**

Run: `cargo test --lib br138_ -- --test-threads=1`
Expected: compile failure because the predicate is not defined.

- [ ] **Step 3: Implement the narrow pure predicate**

```rust
pub fn announcement_title_is_immediately_actionable(title: &str) -> bool {
    let creditor_procedure = title.contains("通知债权人")
        && (title.contains("减少注册资本") || title.contains("注销") && title.contains("回购"));
    let reduction_completed = title.contains("减持")
        && ["期限届满", "时间届满", "实施完毕", "实施完成"]
            .iter()
            .any(|marker| title.contains(marker));
    !creditor_procedure && !reduction_completed
}
```

- [ ] **Step 4: Run focused tests and commit**

Run: `cargo test --lib br138_ -- --test-threads=1`
Expected: both exclusion tests pass and the new-plan control remains true.

```bash
git add src/data_provider/announcement.rs
git commit -m "fix: suppress completed announcement lifecycle notices"
```

### Task 3: Gate normalized announcements by the real monitored universe

**Files:**
- Modify: `src/bin/monitor/v17_sources.rs`
- Modify: `src/bin/monitor/main.rs`
- Test: `src/bin/monitor/v17_sources.rs`
- Test: `src/bin/monitor/main.rs`

- [ ] **Step 1: Write failing router tests**

```rust
#[tokio::test]
async fn br138_off_universe_announcement_is_handled_without_push() {
    let eligible = HashSet::from(["TEST_CODE_ALLOWED".to_string()]);
    let report = route_announcements(&[test_important_announcement(
        "TEST_CODE_EXTERNAL", "TEST_CODE_OTHER", "重大监管问询公告"
    )], &eligible).await;
    assert_eq!(report.source.pushed, 0);
    assert_eq!(report.source.skipped, 1);
    assert_eq!(report.disposition("TEST_CODE_EXTERNAL"),
               Some(AnnouncementDisposition::FilteredAudience));
}

#[tokio::test]
async fn br138_eligible_actionable_announcement_still_reaches_governance() {
    let eligible = HashSet::from(["TEST_CODE_ALLOWED".to_string()]);
    let report = route_announcements(&[test_important_announcement(
        "TEST_CODE_ALLOWED_EXTERNAL", "TEST_CODE_ALLOWED", "重大监管问询公告"
    )], &eligible).await;
    assert_eq!(report.source.classified, 1);
    assert_eq!(report.disposition("TEST_CODE_ALLOWED_EXTERNAL"),
               Some(AnnouncementDisposition::Pushed));
}
```

- [ ] **Step 2: Run the tests and verify RED**

Run: `cargo test --bin monitor br138_ -- --test-threads=1`
Expected: compile failure because the router lacks the eligible-universe parameter and handled field.

- [ ] **Step 3: Implement audience/lifecycle ownership**

Return `AnnouncementDisposition` by external ID. Increment `skipped` and record
`FilteredLifecycle` before normal classification for a retained local-only lifecycle row; record
`FilteredAudience` when the code is absent from `eligible_codes`; never call
`push_normalized_event` for either. Record `Pushed` only for confirmed delivery and `Failed` for any
other normalized outcome.

```rust
pub async fn route_announcements(
    announcements: &[Announcement],
    eligible_codes: &HashSet<String>,
) -> AnnouncementSourceRouteReport
```

In `news_monitor_loop`, require `announcement_keywords_available()`, pass the independently loaded
watch audience plus only a verified broker-position component, and consume the route's typed
per-identity disposition. Every disposition suppresses legacy fallback, but append to the downstream
trigger set only for `Pushed`. Do not apply this gate to policy or critical flash producers.

Add a RED test proving a fresh local `stock_position.updated_at` cannot authorize a holding, and a
RED test proving lifecycle/off-universe dispositions cannot make the outer-loop important-event gate
true. Keep positive controls for explicit watch membership and normalized `Pushed` outcomes.

Add a provider RED test proving the creditor-notice example survives assembly as `Skip` local
evidence and a loop-isolation RED test proving watch-load failure does not prevent unrelated outer
work. Use at most one outer-loop-owned background watch load, never await it while unfinished, run
policy/critical flash first, and continue holding-derived earnings/L2 when the watch result is absent.
Add consumer RED tests proving the provider-retained `Skip` row cannot enter NewsMonitor,
NewsAggregator, either R-08 path, or event-calendar rendering. Log the first failure immediately and
expose per-disposition aggregate counts for live canary evidence.

- [ ] **Step 4: Update all existing router tests/callers and run monitor tests**

Run: `cargo test --bin monitor -- --test-threads=1`
Expected: all monitor tests pass, including repeated-announcement dedup and BR-137 source facts.

- [ ] **Step 5: Commit**

```bash
git add src/bin/monitor/v17_sources.rs src/bin/monitor/main.rs
git commit -m "fix: gate immediate announcements by relevance"
```

### Task 4: Release gates, review, merge, and production restart

**Files:**
- Modify: PR evidence only

- [ ] **Step 1: Run all mandatory gates**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
git diff --check
```

Expected: every command exits zero; global line coverage is at least 80% and core line coverage is at
least 95%.

- [ ] **Step 2: Push, create a complete PR, and obtain independent approval**

The PR must include Refs, Data-Redlines, OldModules, Threshold-Proof, Business-Rules, validation, and
rollback fields. No account snapshot, raw log, notification target, provider body, or security list is
included.

- [ ] **Step 3: Merge to master and restart production**

After approval, merge the PR, fast-forward local master, rebuild release, terminate only the verified
old monitor PID, start one managed monitor instance, and verify the loaded binary inode matches the
master artifact.

- [ ] **Step 4: Verify the live selection contract**

From a new local-log line baseline, record aggregate counts only. Required evidence: no panic/fatal,
no `banner unavailable`, BR-138 typed disposition counts are visible, handled low-value events do not reach a
Pushed outcome, and eligible real source facts can still reach normal governance.
