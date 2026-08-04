# Progress Log

## 2026-07-30

- Restored the active BR-192 goal and mandatory AGENTS pre-flight/gates.
- Aligned all downstream Magic crate revision pins to upstream merge
  `d7dfa3140919525f3280bed87136602a78fa17ad`; focused revision/config tests passed.
- Implemented and statically checked the durable coordinator, runtime startup reconciliation,
  exact-byte append chain and schedule hydration.
- Runtime physical-isolation tests both executed and passed:
  `br192_each_test_guard_owns_one_stable_physical_runtime_namespace` and
  `br192_test_code_cannot_escape_its_physical_namespace`.
- Added explicit counted binding and made every generic counted push fail closed.
- Completed static R-09, R-04 and R-08 producer migrations; Cargo integration remains pending.
- Retired the residual R-02 full-market snapshot producer instead of relabelling partial data.
- Generalized scheduler hydration to registered BR-140 task labels.
- Durable coordinator validation is green: the 27-test full rerun plus the corrected
  append-history exact test prove all 28 cases pass. The final assertion now reflects the
  three legitimate immutable events in each generation: Reserved, Attempting and terminal.
- T0Advice audit found the existing production loop always hits
  `counted_binding_required`, never advances its timer and reacquires data every 30 seconds.
  The migration now preserves the real account snapshot and complete Magic TDX evidence
  through a stable T0 decision binding; five focused suites pass 33/33.
- Completed a read-only counted-caller inventory: twelve reachable production-semantic
  callers remain on the generic governor, plus one already disabled path.
- Started three parallel, non-overlapping follow-ups: PaperTrade terminal-receipt binding,
  fail-closed removal/reclassification of unsupported main-loop callers, and superseding
  misleading v12 E2E documentation.
- PaperTrade now binds the immutable terminal row, plan ID, typed instrument and exact
  order-audit chain receipt; its static contract is 4/4 and library BR-192 tests are 2/2.
- Eight unsupported main-loop counted producers now fail closed before private acquisition;
  board-flow uses the existing non-counted `IntradayMarket` semantics.
- The real/paper position summary and close-review report are explicitly unavailable because
  they lack the <=30s account capture and complete immutable price/account evidence required
  by data-redline 2.4.
- Deleted the synthetic legacy "20 templates" E2E producer, the unbound real/paper position
  summary renderer and unused old R-08 DTOs. HoldingPlan now fails closed before acquiring
  positions or quotes, and its unbound production wrappers are gone; focused renderers remain.
- Removed the deleted daily-report router's generic sub-kind wrappers while preserving
  explicit durable sub-kind mappings and gates. The expanded static BR-192 guard passes 6/6.
- PaperTrade now rejects future or older-than-five-second realtime quotes before business-ID
  reservation or audit writes, revalidates the quote in its terminal binding, and explicitly
  isolates settled-daily execution instead of re-timestamping a daily close as realtime.
  Its two static contracts pass 8/8; root Cargo validation remains.
- Root Cargo validation for the PaperTrade BR-192 slice passes 4/4 library tests, including
  five-second/future freshness, settled-daily isolation, stable terminal binding and the exact
  order-audit receipt.
- The first integration-test attempt correctly failed Gate B: monitor compilation found eight
  legacy daily-bar helper references and one duplicate PaperTrade status-type mismatch. Two
  non-overlapping repairs are in progress; no validation result has been fabricated.
- Completed a read-only BR-164 residual audit. Provider transports are fully cut over, while a
  P0 financial/consensus evidence-projection loss and nine explicit upstream capability gaps
  remain before the overall unified-data goal can be called complete.
- Retired the remaining monitor `--test` synthetic counted paths, indirect all-template sweep,
  old daily-bar helper references and fabricated LHB fixture. Root re-ran the two focused
  static suites: 2/2 monitor cleanup and 6/6 production fail-closed guards pass.
- Completed the pinned-upstream capability audit. None of the nine fail-closed feature gaps
  has a release-ready strong upstream contract that downstream merely forgot to wire; each
  now has an exact upstream/downstream evidence location and a minimal honest follow-up slice.
- Reconciled the BR-192 business-rule row and authoritative design with implementation truth:
  the coordinator/cutover is present, Gate B/C/D remain pending, all fourteen Magic crates are
  pinned to `d7dfa3140919525f3280bed87136602a78fa17ad`, deleted router paths are gone, and
  missing process/operations evidence is explicitly listed. BR-192-specific rule checks are
  clean; the repository-wide business-rule script still reports 21 unrelated existing issues.
- Cleared all 21 repository-wide blocking business-rule registration errors without restoring
  retired code. `check_business_rules.sh` now passes with 188 registered rules, zero errors
  and 108 non-blocking historical warnings; `git diff --check` passes.
- Independent Gate B review discovered a Critical test/live isolation flaw in the legacy dry-
  run authoritative sink seam. It is recorded as a release blocker; no production run will be
  attempted until the seam is physically TEST_CODE-scoped and regression tested.
- Completed the P0 financial/consensus evidence projection. Focused suites pass:
  source events 11/11, company financials 3/3, service 5/5 and monitor v17 sources 22/22;
  formatting and diff checks pass. Full monitor integration still must be rerun.
- Corrected the preliminary review interpretation of manual resolution before any edit:
  immutable authorization-before-CAS is intentional retry evidence. The actionable review
  set is now one Critical dry-run isolation defect and one Important hydration persistence
  defect, both assigned in parallel.
- Gate B review found a second Important: retry-authorized R-04/R-08 durable rejections cannot
  progress in an already-ready long-lived runtime and remain stranded until restart.
- Retracted a tentative pre-sink retry-projection Critical after confirming the existing
  transaction already synchronizes the decision retry flag. The repair owner was stopped
  before changing that correct behavior.
- Hydration persistence work uncovered a cross-date Critical before committing an unsafe ack:
  the caller must pass back the exact identities actually applied, not every queued hydration.
  The coordinator patch is retained while the main/review-batch caller change waits for the
  dry-run owner to release `main.rs`.
- Closed the hydration Critical with exact applied-transition evidence. Coordinator audit
  failure/restart idempotency, runtime restart/foreign-date handling, main caller ack failure
  and review-batch evidence tests pass 6/6; foreign dates remain Pending.
- Closed the static production dry-run seam and added immutable runtime namespace binding.
  The focused namespace behavior passes 1/1, the runtime module passes 8/8 and both process
  isolation tests pass 1/1.
- Hardened the test durable-store path from lexical and physical aliases. Lexical traversal
  and non-exact paths pass 3/3; canonical-parent symlink and Unix dev+ino hardlink isolation
  pass 3/3. The production DB/WAL/SHM test artifacts were removed.
- A second independent isolation re-review is active. The remaining known Gate B Important is
  provider-free retry for authorized RejectedDurable decisions with typed non-fatal deferral.
- Reopened physical isolation after the re-review found that open-time path validation alone
  cannot prove the later write target. Three non-overlapping repairs now retain and revalidate
  complete directory/file identities for push logs, immutable audit files, and SQLite
  main/WAL/SHM; no Cargo command is allowed while those shared-worktree edits are unstable.
- Immutable append no longer returns success before its common post-write validation and now
  uses an incremental verified cursor instead of reparsing the five-year JSONL on every append.
  Ownership/mode, parent-fsync and deterministic replacement regressions remain in progress.
- SQLite main/WAL/SHM descriptor attestation, alias rejection, leaf-swap detection and
  ownership/mode checks are statically present. The remaining static repair is an exact
  retained SHM proof for SQLite's same-process shared-SHM reuse.
- Root started the immutable-append Cargo test but aborted it during compilation before any
  test executed. A fresh independent SQLite review found a test fixture that could create and
  unconditionally delete the real `data/durable_delivery.sqlite3` set; executing it would
  violate 2.5/2.7 and could damage a concurrent monitor.
- The same review returned Gate B RED with seven Critical and four Important findings:
  runtime-CWD production rooting, pre-attestation DDL/WAL mutation, unsafe SQLite SHM fd
  duplication, missing post-operation validation, missing retained ancestor-chain proof,
  swallowed descriptor-enumeration errors, fd/proof lifetime leaks and incomplete adversarial
  tests. Fixture isolation and coordinator repairs are now separate parallel work tracks.
- First-round repairs reached static stability, but two fresh independent reviews correctly
  kept Gate B RED. File sinks/event audit have 2 Critical and 7 Important findings: counted
  acceptance omits durable `push.delivery.audit`, the cursor skips complete-chain validation
  between checkpoints, the generic writer binds lazily, artifacts are not receipt/outcome
  joined, cursor/fixtures/tests/error classification and older `PUSH_LOG_DIR` specs remain
  incomplete. SQLite has 3 Critical and 4 Important findings: four state writes autocommit,
  TEST_CODE runtime passes an absolute path rejected by the exact relative config contract,
  zero-touch snapshots cannot detect in-place mutation, rollback/descriptor/test/target
  evidence is incomplete. Both repair tracks are active again; no Cargo command has run.
- A separate provider-free retry audit found 2 Critical and 2 Important gaps: long-lived
  ready runtimes never retry later authorized `RejectedDurable` decisions; budget/cooldown/
  claim deferrals collapse to `false` while summaries call them deliverable; manual retry
  authorization and concurrency/provider-zero/Uncertain evidence are absent. This is the next
  TDD slice after the physical isolation gate closes.
- The second independent file-sink/event-audit review remains RED at 1 Critical and
  5 Important. The counted path still delegates to the pathname-based legacy
  `AuditDispatcher`, uncertainty is exposed as retryable failure, the exact three-way
  terminal verifier and key failure/concurrency tests are absent, event tests do not own a
  TEST_CODE namespace, and counted audit lineage is hard-coded rather than bound to the real
  PushKind/template and BR-192 rule.
- The SQLite repair owner reports all prior transaction, zero-touch, descriptor-lifetime and
  adversarial-test findings statically addressed without Cargo. A third independent
  adversarial review is active; this is not yet accepted Gate B evidence.
- A fresh shell existence check confirms the production
  `data/durable_delivery.sqlite3{,-wal,-shm}` set is absent. No SQLite client was opened and
  no production data was read or mutated.
- The third independent SQLite review is RED at 3 Critical and 5 Important. It proved
  schema/policy writes can commit before WAL/SHM attestation, unsupported descriptor/OFD or
  malicious-sidecar failures can occur after main DB creation, and the test snapshot itself
  reads production main/WAL/SHM bytes. Exact manifest-absolute TEST_CODE binding,
  manifest-ancestor proof, cross-process OFD uniqueness, four-path ack failure injection and
  cross-process WAL/SHM tests are also incomplete. These findings are assigned to a dedicated
  repair track; Cargo remains blocked.
- README now accurately lists the independent BR-192 SQLite ledger as the third database,
  `.env.example` states that it cannot be overridden, Magic TDX integration separates
  Tencent/Sina identity from TDX lifecycle, and the root EMQuant document is explicitly
  marked historical/superseded. The active config-cleanup spec now records the released
  fourteen-crate `d7dfa314...` baseline. Static diff checks pass for this documentation slice.
- The file/event-audit implementer reports the retained-root, schema-v3 terminal join,
  non-retryable uncertainty and adversarial TEST_CODE coverage statically complete. Because
  that same lane repaired a directory-link-count defect, a fresh independent read-only
  C=0/I=0 review is now the acceptance gate; no self-review is accepted.
- The SQLite isolation implementer reports exact TEST_CODE path normalization, production
  zero-touch guards, pre-create capability probes, main/WAL/SHM attestation before
  transactional schema writes, owner-specific OFD markers and acknowledgement race/rollback
  coverage statically complete. A fresh independent adversarial review is active; no Cargo
  command will run until both physical-isolation reviews are green.
- The first provider-free retry Gate A review is RED at 4 Critical and 14 Important. The
  design/plan amendment must bind retry authorization to the frozen rejection disposition,
  persist append-only authorization and admission audit state, share a cycle-global sink
  attempt fence across Reserved and retry paths, and add cross-process/cancellation/backoff/
  TEST_CODE/Gate-D evidence before implementation may begin.
- A fresh zero-touch shell check confirms
  `data/durable_delivery.sqlite3{,-wal,-shm}` remains absent. No SQLite connection or Cargo
  process was started by that check.
