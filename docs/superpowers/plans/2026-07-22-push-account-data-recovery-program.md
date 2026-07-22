# Push, Account Display, and Data Diagnostics Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore non-duplicating audited delivery, add user-confirmed full-position closing valuation without pretending it is a real account, and replace startup DataMode noise with truthful capability diagnostics.

**Architecture:** Three deep modules are implemented in parallel: delivery-audit health/settlement, user snapshot/closing valuation, and capability diagnostics. They expose small typed interfaces to a fourth, serial monitor-integration plan; only that integration stage edits the shared monitor entrypoints and message governor.

**Tech Stack:** Rust 2021, Tokio, Diesel/SQLite, serde/serde_json, chrono, SHA-256, reqwest/rustdx-complete, existing L4-L7 push governance.

---

## Approved inputs and constraints

- Written design: `docs/superpowers/specs/2026-07-22-user-position-valuation-push-recovery-design.md`.
- Registered rules: BR-143 through BR-149 plus AGENTS 2.1/2.2/2.3/2.4/2.7/2.8/2.10.
- No real broker integration in this program.
- User updates are complete snapshots; latest stable `(effective_at, confirmed_at, snapshot_id)` wins.
- Local closing valuation remains display-only; Frozen and incomplete action facts remain unchanged.
- Existing `data/event_audit/*.jsonl`, user snapshots, valuation runs, and incidents are append-only evidence and are never rewritten/deleted by rollback.
- `stock_position` is a simulated projection and `stock_daily` does not preserve `adjust`; neither may be used as the new authoritative input.

## Deep-module interfaces and file ownership

| Owner | Deep module/interface | Owned files during parallel phase | Shared files explicitly excluded |
| --- | --- | --- | --- |
| Audit worker | `event::{preflight_runtime_delivery_audit, runtime_delivery_audit_health, settle_delivery}` | `src/event/push_record.rs`, `src/event/dispatcher.rs`, `src/event/delivery_settlement.rs`, `src/event/mod.rs` | all `src/bin/monitor/*` |
| Valuation worker | `save_user_position_snapshot`, `latest_user_position_snapshot`, `calculate_closing_valuation`, `save_closing_valuation`, `latest_persisted_valuation_view` | new portfolio/database modules, `src/database/mod.rs`, `src/portfolio/mod.rs`, importer binary | all `src/bin/monitor/*`, `notify.rs` |
| Capability worker | `CapabilityTracker::snapshot` and `evaluate_diagnostics` | `src/monitor/data_mode.rs`, optional new `src/monitor/capability_health.rs`, library provider markers | `main.rs`, `market_data.rs`, `push_templates.rs`, `notify.rs`, `v14_adapter.rs` |
| Integrator | production startup, probe scheduler, governed sensitive message, rendering | `src/bin/monitor/main.rs`, `notify.rs`, `push_templates.rs`, `v14_adapter.rs`, `market_data.rs`, new bin-monitor helpers, process tests | parallel modules after their interfaces are accepted |

The module deletion test passes: deleting any of the three parallel modules would force chain compatibility/settlement, snapshot/valuation invariants, or capability-state logic back into multiple callers. Database and network details remain behind adapters; renderers receive persisted views and never query or calculate.

## Four-angle architecture challenge

| Angle | Objection | Resolution / acceptance proof |
| --- | --- | --- |
| Data safety | A user snapshot or adjusted/unknown close could be mistaken for broker evidence. | Separate table/type names, no `stock_position` fallback, RustDX `AdjustType::None` check, action conversion rejects display facts, BR-146/147/149 tests. |
| Failure/recovery | Audit could fail after physical delivery, causing duplicate sends. | Typed `PhysicallyDeliveredAuditFailed`, identity commit after sink acceptance, AuditDegraded before next sink, process test with sink call count 1. |
| Compatibility | Relaxing empty code could admit malformed v2 or rewrite history. | Exact legacy double-empty normalization only after hash verification; byte-for-byte prefix test; authoritative v2 parser unchanged. |
| Operability/integration | Green library tests could remain disconnected from monitor. | Serial integration plan, release binary smoke, multiline call-chain grep, production canary/evidence or explicit `disabled=no_producer`; no completion claim without CLAUDE.md four layers. |

## Milestones and dependency map

| Milestone | Effort | Owner | Depends on | Done criteria |
| --- | --- | --- | --- | --- |
| M0 Gate A registration | 1–2h | Main | approved design | BR-143..149 and four implementation plans committed; no contradiction/placeholders |
| M1 Audit core | 4–8h | Audit worker | M0 | legacy chain loads; preflight health works; settlement tests prove no resend |
| M2 Snapshot/valuation core | 6–12h | Valuation worker | M0 | atomic complete snapshots and persisted partial valuation pass focused tests |
| M3 Capability diagnostics core | 6–12h | Capability worker | M0 | five states, Warming suppression, provider/error evidence pass focused tests |
| M4 Serial monitor integration | 8–16h | Main | M1, M2, M3 | startup preflight precedes sinks; quote probe independent; valuation message governed/redacted |
| M5 Gates C/D and canary | 8–16h | Main + independent reviewer | M4 | all mandatory commands/evidence pass; PR remains draft until production evidence exists |

```text
M0 ──┬──> M1 Audit core ────────────┐
     ├──> M2 Snapshot/valuation ────┼──> M4 serial integration ──> M5 Gates C/D
     └──> M3 Capability diagnostics ┘
```

Critical path is the audit core followed by serial integration: ordinary sinks remain disabled until audit preflight is Healthy.

## Execution sequence

- [x] **Step 1: Approve the written design**

Expected: the design status records user approval on 2026-07-22.

- [x] **Step 2: Register business rules before code**

Expected: `rg -n 'BR-14[3-9]' docs/business_rules.md` returns seven registered rules.

- [ ] **Step 3: Execute the three parallel plans with exclusive file ownership**

Plans:

```text
docs/superpowers/plans/2026-07-22-delivery-audit-recovery-implementation.md
docs/superpowers/plans/2026-07-22-user-position-closing-valuation-implementation.md
docs/superpowers/plans/2026-07-22-capability-diagnostics-implementation.md
```

Expected: each worker commits only its owned files and reports focused RED/GREEN evidence.

- [ ] **Step 4: Independently review each parallel result before integration**

The reviewer brief must begin with the CLAUDE.md independence paragraph and rerun claimed tests. Critical/Important findings block M4.

- [ ] **Step 5: Execute the serial monitor integration plan**

Plan:

```text
docs/superpowers/plans/2026-07-22-monitor-recovery-integration-implementation.md
```

Expected: multiline grep shows imports and call sites from `main.rs`/`notify.rs`/`push_templates.rs` into all three new interfaces.

- [ ] **Step 6: Run mandatory Gate C commands**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
cargo build --release --bin monitor
```

Expected: every command exits 0. A freshness failure is fixed with `bash tools/one_shot/backfill_daily.sh` and the whole Gate C sequence is rerun.

- [ ] **Step 7: Run Gate D coverage and isolated live-path evidence**

```bash
cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
V10_DRY_RUN_PUSH=1 ./target/release/monitor --test
```

Expected: global coverage is at least 80%, changed core paths at least 95%, and the isolated smoke shows audit preflight, capability Warming/attempt, and the new governed path without exposing holdings or amounts.

- [ ] **Step 8: Perform production canaries in release order**

```text
1. Audit full-chain preflight + isolated filesystem canary
2. Runtime I-01 blocking regression
3. Closing valuation shadow run with latest snapshot hash/coverage only
4. Governed valuation message canary
5. Capability diagnostics canary
```

Expected: no ordinary sink is invoked when audit is degraded; no user-position values appear in delivery audit or aggregate logs; no completion claim is made if a real producer has no evidence.

## Risk and rollback matrix

| Risk | Impact | Mitigation | Rollback |
| --- | --- | --- | --- |
| Legacy parser too permissive | Critical | exact double-empty matrix + v2 strict regression | revert audit-core commit; sinks remain blocked |
| Physical send duplicated | Critical | sink counter test + identity commit on acceptance | revert integration while preserving committed identities/incidents |
| Adjusted/old close used | Critical | `AdjustType::None`, settled date, strict K-line validation | disable valuation scheduler; preserve stored run |
| Sensitive values leak to logs | Critical | dedicated sensitivity policy + negative grep tests | disable valuation dispatch; retain controlled DB facts |
| Warming accidentally authorizes actions | Critical | governance maps Warming to Down/Unsafe; BR-134 regression | revert diagnostic projection only |
| Shared-file merge conflict | Medium | main agent owns all shared monitor files | abort integration commit; parallel commits remain independently testable |

## PR evidence template

```markdown
### Refs
- spec: `docs/superpowers/specs/2026-07-22-user-position-valuation-push-recovery-design.md`
- plans: the four `2026-07-22-*-implementation.md` documents

### Data-Redlines
- [2.1] no simulated position or unknown-adjust close fallback
- [2.2] missing account/price/capability facts remain explicit
- [2.3] strict K-line and price validation before valuation
- [2.4] Quote 5s independent from position 30s; daily close is latest completed day
- [2.7] startup audit preflight and tamper-resistant append remain fail-closed
- [2.8] import/save/probe/push paths perform real target operations
- [2.10] BR-143..149 registered before code

### OldModules
| module | adopt/reject | reason |
| --- | --- | --- |
| `event::AuditDispatcher` | adopt/deepen | preserve lock/hash/fsync implementation; add exact compatibility and health |
| `stock_position` | reject | simulated/order projection, not user-confirmed snapshot |
| `stock_daily` | reject for valuation facts | persisted rows lack adjustment provenance |
| `RustdxProvider` | adopt | real strict unadjusted K-line source |
| existing governance DataMode | adopt | keep action semantics; diagnostics are orthogonal |

### Threshold-Proof
- no config threshold changed; existing 5s/30s/1-trading-day red lines remain authoritative

### Business-Rules
- BR-143, BR-144, BR-145, BR-146, BR-147, BR-148, BR-149

### Rollback
- `git revert <scoped-commit-sha>` in reverse integration order; never delete evidence
```

## Status rule

Until M5 passes with independent review and required production evidence, report this program as **In Progress**, never “Done/100%/收官”.
