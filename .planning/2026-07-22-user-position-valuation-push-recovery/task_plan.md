# Task Plan: user position valuation and push recovery

## Goal

Restore truthful, non-duplicating notification delivery; accept atomic user-confirmed full
position snapshots; calculate auditable closing-price valuations without claiming broker account
facts; and deliver clear account/data-status messages through small, parallel, gated PRs.

## Current Phase

Phase 3 — parallel implementation after Gate A plan registration.

## Phases

### Phase 1: Approved design and written review
- [x] Diagnose delivery-audit, runtime, account-metrics, DataMode, and message failures.
- [x] Confirm no real broker-account integration in this scope.
- [x] Confirm user updates are complete position snapshots and take precedence for local valuation.
- [x] Define closing-price valuation and strict non-action boundary.
- [x] Write the design document and perform an inline spec self-review.
- [x] Obtain user review of the written design artifact.
- **Status:** completed

### Phase 2: Implementation plans and Gate A registration
- [x] Invoke `writing-plans` after written-spec approval.
- [x] Produce small-commit plans for Audit core, Snapshot/Valuation core, Capability diagnostics, and serial Monitor integration.
- [x] Re-read the current business-rule registry and allocate BR-143 through BR-149 before code.
- [x] Define focused RED/GREEN commands, ownership boundaries, integration order, PR evidence, and rollback.
- **Status:** completed

### Phase 3: Parallel implementation
- [ ] Audit core: legacy compatibility, preflight seam, strict-v2 regression tests.
- [ ] Snapshot/Valuation core: schema, atomic repository, formulas, partial coverage, persistence.
- [ ] Capability core: five-state diagnostics and independent probe seam.
- [ ] Keep shared-file ownership exclusive; main agent performs cross-stream integration.
- **Status:** in_progress

### Phase 4: Integration and mandatory Gates
- [ ] Integrate notification outcome semantics and production wiring.
- [ ] Run focused tests, format, strict Clippy, full workspace tests, compliance, coverage, and release build.
- [ ] Run isolated E2E and privacy checks.
- [ ] Obtain independent code/spec review with zero blocking objections.
- **Status:** pending

### Phase 5: Canary, PRs, and delivery
- [ ] Prove audit canary before enabling ordinary pushes.
- [ ] Run shadow valuation against the latest user snapshot and verify coverage without exposing values.
- [ ] Enable valuation/Banner/DataMode changes in the documented order.
- [ ] Complete PR evidence, rollback proof, and production outcome verification.
- **Status:** pending

## Decisions Made
| Decision | Rationale |
|----------|-----------|
| Do not integrate a real broker account | Explicit user scope; estimated holdings must not become action evidence. |
| Latest confirmed complete user snapshot is authoritative for local valuation | User requires later updates to replace prior local holdings. |
| Keep display facts separate from action metrics | Prevent estimated closing P&L from clearing Frozen or authorizing orders. |
| Use real validated unadjusted closes | Current market value must use tradable close; missing values stay missing. |
| Preserve partial independent valuations with coverage | One bad symbol must not erase unrelated complete facts, but totals must remain qualified. |
| Repair immutable-prefix reading without rewriting JSONL | Existing hashes and five-year evidence cannot be edited or deleted. |
| Pause ordinary pushes while audit is unhealthy | User approved temporary suppression and 2.7 requires fail-closed delivery evidence. |
| Use three parallel ownership streams plus serial integration | Maximizes throughput without concurrent edits to shared monitor files. |

## Errors Encountered
| Error | Resolution |
|-------|------------|
| Initial symptom probe used a nonexistent `stock_position.position_amount` column | Read the real schema and reran with `status='open' AND quantity>0`; production was untouched. |
| User-provided Tokio panic was not captured in the private long-running monitor log | Traced the exact reqwest source string and adjacent async-to-blocking I-01 path; preserve backtrace capture as a regression requirement. |
