# AGENTS.md — Repository-wide Mandatory Rules (Highest Priority)

> **Scope**: entire repository (live A-share trading system)
> **Force levels**: **MUST** (violation = block) / **SHOULD** (strongly recommended) / **MAY** (optional)
> **This document is the highest constraint for all development, review, and deployment activities. Data red lines take priority over all processes.**

---

## 0. General Principles

- **MUST** Every data red line in Part 2 must be a Definition of Done (DoD) checkpoint for every stage of the development flow (Part 1); the two are inseparable.
- **MUST** All process outputs must be tracked in Git via PR; the PR checklist must be fully ticked before merge.
- **MUST** In any rule conflict, the priority is: data safety > fund safety > process compliance > development efficiency.
- **MAY** Emergency situations may use the "Controlled Exception Path" (see Part 3), but must leave audit trails and conduct post-mortems.

---

## Part 1: Development Flow

Execute in order. If the previous step has not reached its DoD, do not proceed. Each step's DoD serves as its merge gate.

### 1.1 Flow Steps and Completion Criteria

| Step | Description | DoD (must satisfy before merge) |
|---|---|---|
| 0. Pre-flight | Read `AGENTS.md` + `docs/ENGINEERING_RULES_V2.md` + `.github/copilot-instructions.md` + `CLAUDE.md`. Resolve conflicts by precedence. | Pre-flight plan output (impacted paths, triggered rule IDs, validation, rollback) |
| 1. Architecture | Document design (data flow, failure modes, rollback, old module relations) in `docs/`. | Design doc exists and reviewable; blocking objections = 0 |
| 2. Implementation | Code + explicit failure handling. No mock data in production paths. | `cargo fmt --check` + `cargo clippy -D warnings` + `cargo test` all pass; failure paths exercisable |
| 3. Compliance | All compliance checks pass. | `bash tools/compliance/check.sh` passes (data freshness, fake impl, design contradiction, BR registration) |
| 4. Release | Unit coverage ≥ 80%, core trading/data links ≥ 95%, regression + live data validation pass, audit fields traceable. | Coverage report + auditor sign-off |

### 1.2 Gate Progression (Gate A → B → C → D)

- **Gate A (Design Ready)**: Design intent is explicit and traceable.
- **Gate B (Implementation Ready)**: Implementation + explicit failure handling.
- **Gate C (Compliance Ready)**: All compliance checks pass (blocking).
- **Gate D (Release Ready)**: Tests/coverage/evidence complete.

If a gate fails, fix and retry from the failed gate. Do not skip.

### 1.3 Pre-flight Output (REQUIRED before any code change)

Before editing any code/file, the agent must output a short pre-flight plan containing:

1. **Impacted paths/modules**
2. **Triggered rule IDs** (e.g., 2.1 / 2.4 / 2.6 / 2.8 / 2.9 / 2.10)
3. **Validation commands** to run
4. **Rollback plan**

If pre-flight is missing, the task is **not allowed to proceed**.

---

## Part 2: Data Red Lines (MUST, Blocking)

### 2.1 Data Source

- Production paths **MUST NOT** use mock data.
- Position/trade/net value data **MUST** come from real accounts; fabrication is prohibited.
- Data source failures **MUST** be explicit errors, **MUST NOT** be downgraded to fake data fallbacks.

### 2.2 Missing Data

- Missing data fields **MUST** be left blank or logged as warnings; **MUST NOT** be silently filled.

### 2.3 Bad Data Validation

Before data enters computation, the following **MUST** be validated:

- Price > 0
- Adjacent valid-value change > ±20%: alert + manual confirmation
- Time continuity: gaps/duplicates return error
- Split/dividend consistency: series continuity, jumps within expectation

Bad data is treated as a failure: explicit error, not silent computation with bad data.

### 2.4 Data Freshness

- Realtime quotes: 5 seconds
- Position/cash: 30 seconds
- Net value: same trading day (stale after midnight)
- Daily/historical: 1 trading day

#### 2.4.1 Freshness Gate

- Tables like `stock_daily` **MUST** have `MAX(date)` no more than 1 trading day behind (excluding holidays).
- **MUST** be checked by `tools/compliance/lib/check_data_freshness.sh`.
- **MUST** fix on FAIL: run `bash tools/one_shot/backfill_daily.sh`.
- A freshness FAIL is a strict merge blocker.

### 2.5 Test vs Live Isolation

- Test code **MUST** use `TEST_CODE` prefix.
- Production **MUST REJECT** `TEST_CODE` orders; test environment **MUST REJECT** real-symbol orders.
- Test accounts and live accounts **MUST** be physically isolated.

### 2.6 Order Safety (MUST, Blocking)

- Single order amount ≤ available cash, **AND** ≤ 1,000,000 RMB.
- Single order quantity > 0 **AND** a multiple of 100 shares.
- Order price **MUST** be within the daily limit-up/limit-down range.
- The same business order ID within 60 seconds **MUST** be rejected (idempotency).
- Single order ≥ 500,000 RMB **MUST** require secondary confirmation.

### 2.7 Audit Trail

- Critical data flows and every order **MUST** leave a trace: source, time, decision basis.
- Audit log **MUST** be tamper-resistant with retention ≥ 5 years.

### 2.8 Fake Implementation Ban (MUST, Blocking)

- Functions named `verify/save/notify/push/sync/update_result/reconcile` **MUST** actually operate on the target data source.
- Logging-only is treated as fake implementation, blocking merge.
- Check script: `tools/compliance/lib/check_fake_impl.sh`.

### 2.9 Design Contradiction Ban (MUST, Blocking)

- Modifying `config/*.toml` thresholds **MUST** reference a spec section in the PR.
- Modifying the spec **MUST** reference the config field in the PR.
- `threshold > clamp_max` **MUST** trigger CI FAIL.
- Check script: `tools/compliance/lib/check_design_contradiction.sh`.

### 2.10 Business Rule Documentation (MUST, Blocking)

- Logic involving dedup / mutex / filter / sort / limit **MUST** be registered in `docs/business_rules.md` first.
- PRs **MUST** include the corresponding rule ID (e.g., BR-001).
- Check script: `tools/compliance/lib/check_business_rules.sh`.

---

## Part 3: PR and Exception Handling

### 3.1 PR Required Fields (MUST)

PR description **MUST** include:

- `Refs: spec §X.X`
- `Data-Redlines: [2.1, 2.4, ...]`
- `OldModules:` (each: module | adopt/reject | reason)
- `Threshold-Proof:` (if threshold/config changed)
- `Business-Rules:` (if involved, list BR IDs)
- `Rollback:` (command/steps)

Missing any field → not merge-ready.

### 3.2 Root-Cause Rollback (MUST)

When validation fails, rollback by root cause (not always implementation only):

- Architecture/data-flow issue → return to Gate A
- Task decomposition/scope miss → return to Gate A/B (per situation)
- Implementation bug → return to Gate B
- Red line violation → Gate B fix + Gate A failure-mode recheck

### 3.3 Controlled Exception Path (MUST)

Exception allowed only with:

- Explicit approver
- Reason + risk statement
- Time limit
- Full audit trail
- Postmortem within 24 hours

**MUST NOT** use "emergency" to bypass hard safety red lines.

### 3.4 Output Style (SHOULD)

- Keep output concise, checklist-oriented
- Always reference rule IDs when explaining decisions
- Prefer small, verifiable changes over large batch edits

### 3.5 PR Evidence Examples (MUST)

```
### Refs
- spec: `docs/architecture/v13-push-templates.md §14.2 I-01`
- design: `docs/superpowers/specs/2026-07-06-v13-push-templates-design.md §3.2`

### Data-Redlines
- [2.1] No mock: render only consumes real sector_monitor / sector_score output
- [2.3] Bad data: score>±100 → debug_assert! triggers
- [2.4] Freshness: realtime quotes ≤ 5s

### OldModules
| module | adopt/reject | reason |
| --- | --- | --- |
| `news_monitor_loop` | adopt | reuse existing cluster output |
| `sector_rotation` | adopt | reuse existing score, do not change semantics |

### Validation
- `cargo build --bin monitor`: OK
- `cargo test --bin monitor`: 183/183 PASS
- `bash tools/compliance/check.sh`: PASS

### Rollback
\`\`\`bash
git revert <commit-sha>
cargo build --release
\`\`\`
```

---

## Part 4: Done Criteria (MUST)

A task is "Done" only if **ALL** true:

1. Required checks pass (`cargo fmt/clippy/test` + `tools/compliance/check.sh`).
2. Evidence fields are complete.
3. Failure paths are covered.
4. Tests for changed logic are included.
5. No rule conflicts remain unaddressed.

Otherwise, the status must be **"In Progress / Blocked"**.

### 4.1 Failure Handling (MUST)

If blocked by missing info/tools/permissions:

- Report the exact blocker.
- Provide a safe next action.
- Do not fabricate results.
- Do not bypass compliance.

---

## Part 5: Quick-Reference Rule Index

| Rule | Description | Check Script |
|---|---|---|
| 2.1 | No mock data in production | `check_fake_impl.sh` |
| 2.2 | Missing data explicit | manual review |
| 2.3 | Bad data validation | `cargo test` |
| 2.4 | Data freshness | `check_data_freshness.sh` |
| 2.5 | Test/live isolation | `env_guard.rs` |
| 2.6 | Order safety | `risk/limits.rs` |
| 2.7 | Audit trail | `database/` modules |
| 2.8 | No fake implementation | `check_fake_impl.sh` |
| 2.9 | No design contradiction | `check_design_contradiction.sh` |
| 2.10 | Business rule registration | `check_business_rules.sh` |

---

**Version**: v16.3 (2026-07-06)
**Supersedes**: Chinese original (preserved as git history)
**Compatibility**: All data red lines (Part 2) unchanged; only language/wording updated.
