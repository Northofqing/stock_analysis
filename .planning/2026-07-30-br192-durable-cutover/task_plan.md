# Task Plan: BR-192 Durable Counted-Delivery Cutover

## Goal

Finish the evidence-bound BR-192 delivery coordinator and migrate every active counted
producer without synthetic dispatch-time identity, then pass the repository Gate B/C/D
checks and the required monitor runs.

## Current Phase

Phase 4 — Gate B integration and independent repair.

## Phases

### Phase 1: Gate A and upstream release
- [x] Freeze the BR-192 design and business rule.
- [x] Independent exact-byte Gate A review: 0 Critical / 0 Important.
- [x] Merge and pin upstream `magic-market-data-rs` revision
  `d7dfa3140919525f3280bed87136602a78fa17ad`.
- **Status:** complete

### Phase 2: Durable coordinator
- [x] Implement SQLite state machine, immutable append chain, generations, claims,
  receipts, uncertainty/manual boundaries and schedule hydration.
- [ ] Re-close production/test physical runtime isolation for fixed push logs, immutable
  audit files, and the SQLite main/WAL/SHM set after independent path-rebinding review.
- [x] Pass both focused runtime namespace tests.
- [x] Pass all 28 durable coordinator tests after the final two root-cause fixes.
- **Status:** reopened at Gate B

### Phase 3: Explicit producer binding
- [x] Add `CountedDeliveryBinding`; reject generic counted pushes with
  `counted_binding_required`.
- [x] Migrate R-09 Provider Top-N.
- [x] Migrate R-04 ReviewLhb using real GatewayBatch evidence.
- [x] Migrate R-08 EventCalendar using four ordered real GatewayBatch identities.
- [x] Retire residual R-02 acquisition and record capability-unavailable.
- [x] Generalize BR-140 hydration mapping to registered review task labels.
- [x] Remove the legacy synthetic R-08 test producer.
- [x] Resolve remaining counted producers: migrate only with real stable evidence;
  otherwise record explicit disabled/unavailable state.
- [x] Restore T0Advice with a complete Magic TDX evidence batch, real account snapshot
  binding and stable durable decision identity.
- [x] Restore PaperTrade from the immutable terminal row plus exact order-audit
  hash-chain receipt; fail closed on absent or ambiguous evidence.
- **Status:** complete

### Phase 4: Gate B integration
- [x] Preserve admitted financial/consensus batch evidence through normalized source events.
- [x] Classify the remaining upstream capability gaps and sequence honest follow-up slices.
- [x] Reject production dry-run before opening the durable store and physically isolate
  TEST_CODE runtime namespaces.
- [x] Persist and acknowledge only exact schedule hydrations applied to the current
  business date; preserve foreign-date Pending state across restart.
- [ ] Reject lexical path traversal, symlink/hard-link aliases and post-open path
  replacement across fixed push-log, immutable-audit and SQLite main/WAL/SHM namespaces.
- [ ] Preserve a retained, exact SQLite SHM proof when SQLite reuses a process-shared SHM
  mapping for a second coordinator.
- [ ] Validate effective ownership and non-group/world-writable modes for every retained
  namespace directory and file; durably sync newly created parent entries.
- [ ] Focused explicit-binding and producer tests.
- [ ] `cargo check --bin monitor`.
- [ ] Independent Gate B review and repair.
- **Status:** in progress

### Phase 5: Gate C/D and delivery
- [ ] fmt, strict Clippy, full tests and compliance.
- [ ] Coverage ≥80% overall and ≥95% critical.
- [ ] `monitor --test`, `monitor --review`, bounded normal monitor live evidence.
- [ ] README/config cleanup, mandatory PR evidence, merge to master.
- **Status:** pending

## Remaining Work

1. Fix and independently re-review the reopened isolation tracks:
   - file sinks/event audit first fresh review was `2 Critical + 7 Important`; its
     second fresh review remains RED at `1 Critical + 5 Important`, with the retained
     audit-dispatcher identity, non-retryable uncertainty and exact terminal verifier
     repairs active;
   - SQLite coordinator/fixtures first fresh review was `3 Critical + 4 Important`;
     the third independent review remains RED at `3 Critical + 5 Important`.
     Bootstrap must bind WAL/SHM before schema commits, capability/sidecar checks must
     precede main-file creation, tests must never read production DB/WAL/SHM bytes,
     exact manifest-absolute TEST_CODE paths and cross-process/OFD/ack evidence remain
     incomplete. The repair track is active.
2. Add provider-free authorized retry with typed budget/cooldown/business-date deferral,
   concurrency fencing and no retry for uncertain outcomes. The read-only audit found
   2 Critical + 2 Important gaps.
3. Run the focused Gate B tests, then the repository Gate C/D commands in one serialized
   Cargo queue.
4. Capture `monitor --test`, `monitor --review` and bounded normal-mode evidence.
5. Complete README/config cleanup, PR evidence and merge; retain the nine upstream
   capability gaps as explicit unavailable states until their strong contracts exist.

## Errors Encountered

| Error | Attempt | Resolution |
| --- | --- | --- |
| Durable coordinator focused suite passed 24/28 | 1 | Fixed two test SQL defects, foreign-attempt readiness and audit identity collision. |
| Durable coordinator focused suite passed 26/28 | 2 | Use SQLite `rowid` in the append-order assertion and namespace the manual accepted-audit identity; rerun active. |
| Runtime test with `--exact` selected zero tests | 1 | Re-ran by unique substring without `--exact`; both namespace tests passed. |
| R-04/R-08 hydration unsupported because mapping recognized only R-09 | 1 | Map only registered `ReviewTask::ALL` labels and retain canonical task-identity validation. |
| T0 complete batch identity was frozen before actual provider completion | 1 | Freeze after completion and hash requested/source/observed times, typed instruments, normalized records and explicit rejections. Five focused suites pass 33/33. |
| Twelve active counted producer paths still use the generic governor | 1 | Prioritize PaperTrade and real-position summary binding; reclassify board flow; explicitly disable remaining paths until immutable evidence exists. |
| HoldingPlan lacked admissible account/quote/decision evidence | 1 | Fail closed before position/quote acquisition and remove the unbound production wrappers; preserve pure renderers only. |
| PaperTrade integration tests failed monitor compilation with eight removed/gated daily-bar helper references and one duplicate status-type mismatch | 1 | Re-open Gate B: retire unsupported legacy review acquisitions in `main.rs` and unify/convert the PaperTrade terminal status at the binding boundary, then rerun. |
| Independent Gate B review found production `V10_DRY_RUN_PUSH=1` can synthesize an authoritative accepted receipt in the production durable namespace | 1 | Critical: physically restrict dry-run acceptance to TEST_CODE/test runtime and reject it before any production durable reservation or authoritative result. |
| Independent Gate B review found applied BR-140 schedule hydration is not durably acknowledged/audited | 1 | Important: persist an idempotent Applied acknowledgement tied to exact transition identity/hash so restart cannot reapply it. |
| Retry-authorized `RejectedDurable` is stranded in a long-lived ready runtime until restart | 1 | Important: let the same exact envelope re-enter the coordinator's authorized retry transition without provider reacquisition or bypassing startup reconciliation. |
| Hydration caller acknowledges all queued transition IDs although the schedule applies only its current business date | 1 | Critical: return and persist only exact applied transition identities; foreign-date transitions remain Pending for their own schedule date. |
| Production/test isolation re-review found runtime namespace reuse and test-store path aliasing | 1 | Bound immutable runtime namespace into the cache; exact lexical path plus canonical-parent and Unix dev+ino checks now reject traversal, symlink and hardlink aliases. Focused re-review is pending. |
| Fresh file-sink review found incomplete per-append chain verification and no counted `push.delivery.audit` | 1 | Gate B reopened: remove cursor checkpointing, restore complete-chain verification on every append, and join artifact/attempt/receipt to the existing durable event audit before Accepted. |
| Fresh SQLite review found four autocommit writes, a test-path contract mismatch and incomplete zero-touch proof | 1 | Gate B reopened: migrate every state write to explicit immediate transactions, make rollback validation explicit, align exact TEST_CODE path shape, and strengthen fixture/snapshot evidence. |
| Provider-free retry review found long-lived authorized rejections stranded and typed deferrals collapsed to `false` | 1 | Schedule a TDD slice after physical isolation: typed retry admission/deferral, one-attempt-per-cycle runner, concurrency fence, zero provider calls and no retry for Uncertain. |
