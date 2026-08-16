# Progress log

## 2026-08-03 BR-203 P2-F executable recovery

- Reconstructed the ignored `target/br203-candidate` compile foundation while
  keeping the product source tree frozen. This candidate is not a Git worktree
  or accepted commit and remains non-release evidence.
- Closed the typed counted-binding compatibility path and the 20-template E2E
  classification: five non-counted templates reach isolated dry-run delivery;
  fifteen counted templates without an admitted producer are explicitly
  `DisabledNoProducer` before the generic governor.
- Diagnosed default-thread monitor failures as legitimate BR-192 ancestor
  link-count revalidation colliding with concurrent sibling creation/removal
  below `data/test`. All physical namespace/environment mutators now share the
  existing `cooldown_memo` serial domain; the security check was not relaxed.
- Candidate evidence: `cargo fmt --all -- --check` PASS; strict workspace
  all-target/all-feature Clippy PASS; `cargo test --locked --bin monitor`
  `509 passed, 0 failed, 4 ignored`; the same suite with
  `--test-threads=1` has the identical result.
- The first full-workspace serial run exposed a real CLI defect: `monitor
  --test` selected Test mode before BR-144 preflight but did not install
  `DURABLE_DELIVERY_TEST_CODE`. P2-H13 now requires a path-safe per-invocation
  code before audit binding, separately from P2-H12's test-runtime serialization.
  The repaired exact integration suite passes
  `19/19`. After quarantining the candidate-only production-shaped SQLite
  artifact to a recoverable `/private/tmp` directory, the locked library suite
  passes `2032/0/5` and the complete locked workspace serial run exits zero.
  The latest `cargo fmt --all -- --check` and strict workspace
  all-target/all-feature Clippy both pass after the CLI repair.
- Re-ran the exact Gate-C command
  `cargo test --locked --workspace --all-targets --all-features -- --test-threads=1`.
  The sandboxed attempt failed only because loopback fixtures could not bind
  local ports (`Operation not permitted`); the required unsandboxed rerun
  exited zero. Library result is `2032 passed / 0 failed / 5 ignored`, monitor
  is `509 / 0 / 4`, `monitor_help_isolation` is `19 / 0 / 0`, and every other
  integration/bench target completed. The process suite created no fixed
  production durable SQLite artifact in the candidate.
- Read-only dependency audit confirms BR-164 is still absent from the candidate:
  only two local Magic path dependencies exist; qmt-parser is present and is
  the sole Polars 0.52 introducer alongside root Polars 0.46; old provider
  callers, RustDX test text and the old BR-203 dependency checker remain.
- Remaining authority blockers are unchanged: refreeze/accept P0-A3 or P0-A4,
  materialize P2-F as a reachable clean commit, complete BR-164/P1/P3/P4, then
  rerun final Gate C/D, live commands, authenticated delivery, cleanup and PR.

## 2026-08-03 BR-203 P0-A3 continuation

- Re-read repository pre-flight rules and restored the persistent production
  closure plan before editing.
- Reused two independent reviewers in parallel: one owns BR-164/BR-203 commit
  ordering and one owns exact argv/test-filter realizability. A third read-only
  reviewer is partitioning the broad BR-164 worktree into compile-closed,
  reversible commits.
- Confirmed current HEAD is `96da674`; P0-M0/P0-A1/P0-A2 exist, while P0-A3
  remains uncommitted and Gate A remains RED.
- Confirmed Polars is already 0.54/0.54.4 and qmt-parser is absent from the
  current target. The active blocker is commit closure and review authority,
  not a TOML version correction.
- Product source remains frozen while the three-document P0-A3 correction is
  prepared. No user worktree cleanup/reset was performed.

## 2026-08-02 BR-196/BR-201 and coverage convergence

- Latest formal Gate-A results remain RED: BR-196 C1/I4/M1 and BR-202
  C2/I2/M0. Their fifth scoped repairs are running in parallel, while a fresh
  independent BR-201 reviewer recomputes the repaired design. No product-code
  implementation has been authorized from these RED designs.
- BR-196's current blockers are the missing BR citation on its target allowlist,
  false-active VirtualWatch state, an omitted direct health-webhook
  presentation, non-reproducible evidence/source binding and incomplete public
  API migration inventory.
- BR-202's current blockers are an incomplete compilation/test input manifest,
  non-terminal publication durability, missing exported object/executable bytes
  and a circular behavior denominator that cannot detect an omitted decision.
- The first BR-201 v5 reviewer completion message accidentally answered the
  user's concurrency question instead of returning a formal verdict. It was
  rejected as review evidence and the reviewer was immediately reopened with a
  mandatory C/I/M verdict contract; Gate A remains RED/pending.
- The latest in-flight Rule-2.10 checker now fails only three hard paths, all
  BR-201 future Gate-B files (`br201_paper_exit_store.rs`, the rollback verifier
  and its integration test), plus 132 historical/non-blocking citation
  warnings. BR-196's config citation and BR-202's current-doc-only row no longer
  contribute hard errors. This remains a Gate-A blocker for BR-201, not a pass.
- BR-201 v5 formal review is RED C2/I5/M0. The fifth repair now owns Git
  tracking/spec-only registration, missing total account reasons, the public
  `paper_trade::simulate` bypass, legacy UTC/timezone/idempotency normalization,
  coherent DualRead/V1Primary eligibility and rollback from an exact clean
  deployed source tree. Previously verified hash/debounce/supersession facts are
  retained but are not enough to cross Gate A.
- BR-201's fifth docs/rule repair is complete: its exact design is staged,
  current Rule 2.10 passes with 198 rules/130 historical warnings and no BR-201
  hard error, while shared business rules remain unstaged. A fresh v6 reviewer
  is independently rechecking the repaired reason totality, private execution
  authority, legacy time/cutover and detached rollback contracts.
- BR-201 v6 formal review is RED C1/I7/M0. Its sixth repair is now confined to
  the deployed rollback trust root, admission terminology, nested private
  SQLite owner, takeover transitions, nonoverlapping reason/result mapping,
  atomic Confirmed-audit sequencing, Rule 2.3 PR evidence and bounded current
  API commands. Gate B remains closed.
- BR-202's fifth docs/rule repair is complete. The exact design file is now
  staged and `git ls-files --error-unmatch`, staged/unstaged whitespace checks
  pass; the shared `business_rules.md` remains MM and was not broadly staged.
  A fresh v6 independent reviewer is active. No Cargo, production-evidence or
  Gate-A success claim was inherited from the repairer.
- BR-196's fifth docs/rule/config-citation repair is complete. The exact design
  is staged and passes tracking/whitespace checks; shared business rules and the
  empty allowlist config were not broadly staged. A fresh v5 independent formal
  reviewer is active. The known manual-contains Clippy debt, absent banners and
  absent 2026-08-02 production evidence remain explicit blockers.
- BR-196 v5 formal review is RED C1/I2/M0. Its sixth repair is confined to the
  exact PR-tracked source/config/rule preimage, a path-bounded full public API
  audit beyond push/dispatch functions, and refreshed compliance output bound
  to checker/rule/config bytes. Gate B remains closed.
- While the three design/review lanes remained docs-only, the shared product
  state was recompiled with `cargo check --locked --bin monitor`; it passed in
  8.00s. This is a build-regression checkpoint only, not Gate-A, runtime-data,
  delivery or release evidence.
- The first BR-202 v6 reviewer turn failed at the external agent service with
  HTTP 403 before reviewing files. It was recorded as infrastructure failure
  and immediately retriggered; it provides no Gate-A evidence.
- The v6 retry hit the same external 403. A new v7 reviewer was started with an
  isolated, self-contained context to avoid carrying the oversized history;
  the two failed turns still provide zero review evidence.
- BR-202 v7 formal review is RED C1/I4/M1. The sixth repair now owns portable
  archive/terminal semantics across CI permission loss, linker/SDK/host-tool
  input closure, a concrete D/M extractor CLI, tracked-only invocation rows,
  complete top-20 evidence and index-ready BR registration. Gate B remains
  closed.
- BR-201 v7 formal review is RED C1/I7/M1. A seventh docs-only repair now owns
  the immutable race-free rollback bootstrap, closed private Admission handoff,
  reason-specific nullable provenance, exhaustive symbol/path/schema inventory,
  durable legacy alias inputs, deterministic proposal ordering and one minimal
  shared-registry GFM repair. Shared BR-201/BR-134 rows remain unstaged until all
  concurrent business-rule writers finish; Gate B remains closed.
- While these reviews remained docs-only, the root baseline also passed
  `cargo fmt --all --check`, `cargo check --locked --bin monitor` and
  `cargo test --locked --bin monitor --no-run`. These checks prove formatting
  and buildability only, not runtime delivery or same-day production evidence.

- Fresh independent reviews are still active and have already found new
  candidate Important defects in every repaired design. BR-196's 36-chain
  count appears to retain at least eleven callerless shapes; BR-201 has
  reason/state/generation/audit-atomicity candidates; BR-202 has CI and
  artifact-lifecycle candidates. All three remain at Gate A pending final
  evidence and rework.
- BR-196 formal review is final RED C2/I2/M0. At least eleven additional
  retained shapes have no non-test upstream caller; the proposed registry
  bijection cannot prove reachability; no complete startup no-producer banner
  contract exists; and Gate C still has the allowlist BR citation failure. A
  third docs/business-rule repair is active with no Gate B edits authorized.

- Re-audited the shared worktree before continuing parallel edits. It remains
  an intentionally broad migration (231 tracked paths in the current diff plus
  untracked design/source/test artifacts); no cleanup/reset is authorized.
  `git diff --check` passes. Active agents are confined to separate BR-196,
  BR-201 and BR-202 design paths, with only BR-201 authorized to touch the
  BR-134/BR-201 business-rule rows.
- Re-ran the current shared-state binary compilation while the three Gate A
  repairs were docs-only: `cargo check --locked --bin monitor` passed in 0.82s.
  This is a compile regression check only and does not override any RED Gate A
  or prove production delivery.
- Re-ran `cargo test --locked --bin monitor br196_ -- --test-threads=1`:
  21/21 pass. These tests still encode the lifecycle/cardinality model that
  production-caller evidence has invalidated, so they are regression-only and
  must be revised after BR-196 Gate A is accepted; a green self-consistency
  suite is not lifecycle truth.
- `cargo fmt --all -- --check` passes against the current shared code state.
  This is an interim formatting gate; it must be repeated after all accepted
  Gate B changes converge.
- BR-196 Gate-A repair has provisionally reconciled the caller graph to 36
  existing proved chains, 14 Disabled legacy declarations and two newly
  registered real inline shapes, for 38 proposed production descriptors. The
  exact design/row and independent review are still pending.
- Synchronized the repaired BR-196 rule into `docs/business_rules.md` after one
  non-mutating failed long-line match. The corrected exact replacement leaves
  one BR-196 row, removes the stale 50-descriptor/A48/A50 authority claims, and
  passes scoped `git diff --check`.
- Interim strict Clippy is RED with one observed error:
  `src/bin/monitor/br196_test_delivery.rs:278` uses `iter().any()` where
  `GOVERNANCE_SMOKE_IDENTITIES.contains(...)` is required by
  `clippy::manual_contains`. The repair is deferred until BR-196 Gate A passes;
  no lint allow will be added.
- Rechecked current-date production evidence for 2026-08-02: both
  `data/push_log/2026-08-02/` and `data/event_bus/2026-08-02.jsonl` are absent.
  BR-196/BR-201 therefore have no current-date production Gate-D evidence;
  tests and dry-run logs remain ineligible substitutes.

- A production-caller audit superseded the previously claimed BR-196 manifest
  cardinalities: at most 36 registered shapes have proved production callers;
  two initially counted shapes lack their claimed production owner, the other
  false-active descriptors must be disabled, and two real production shapes are absent.
  A design-only Gate A repair is recomputing shape/cardinality contracts before
  formal re-review; the earlier passing dry-run is retained only as regression
  evidence.
- BR-201's prior six Important and one Minor findings were remediated in the
  design, but fresh formal review returned C0/I6/M0 on six new contradictions:
  canonical-record cardinality, the two required five-second quote gates,
  JSONL durability recovery, crashed-reconciler takeover, BR-134 fixed-20
  semantics and release-manifest preimage. A docs/business-rule repair is now
  active; implementation remains prohibited until a later review returns C0/I0.
- BR-202 formal Gate A review returned C0/I4/M0. The four blocking defects are
  an evidence command that self-counts, impossible same-commit self-SHA
  attestation, missing inventory generation in the wrapper, and an unfrozen
  zero-instrumented exception that omits 35/408 current Rust sources. A
  design-only repair is active in parallel.
- Independently inspected the current coverage checker and tests. The current
  code still uses 15 raw core prefixes and three basic regression cases; it has
  none of the proposed complete inventory, exact report schema/sum checks,
  zero-instrumented proof, compiler/show/hash reconciliation or isolated
  fixed-SHA wrapper. This confirms implementation has not silently outrun Gate
  A and defines the later Gate B change surface.
- BR-202's docs/business-rule repair now specifies a non-self-counting debt
  probe, source-SHA-bound dep-info/show/zero proof, artifact generation before
  hashing, fail-closed empty bootstrap and a physically possible source commit
  followed by docs-only attestation. Scoped uniqueness/byte-equality and diff
  checks pass; a fresh independent reviewer is active.
- BR-201's second docs/business-rule repair now claims exact 26-field audit
  records, two five-second quote gates, repeat-sync JSONL recovery, same-phase
  reconciler takeover, manual-confirmed/N-A >20% admission and a typed ordered
  commit preimage. A fresh independent reviewer is recomputing all hashes and
  transitions; Gate B remains closed.
- Independently reproduced the claimed BR-201 and BR-134 rule-line hashes by
  hashing exact matched row bytes after removing the line terminator. This
  resolves the apparent mismatch with newline-inclusive `rg | shasum`, while
  leaving formal review to verify that the preimage is documented unambiguously.
- The old coverage checker baseline suite passes 3/3 with
  `cargo test --locked --test test_coverage_thresholds -- --test-threads=1`.
  Because those tests cover only prefix inclusion/basic threshold behavior,
  the pass is not evidence for the proposed BR-202 contracts.
- `bash tools/compliance/lib/check_business_rules.sh` is currently RED with 3
  blocking errors and 124 historical warnings. Blocking debt is exact: the
  coverage test lacks a BR-202 citation, the BR-202 isolated wrapper does not
  exist yet, and the BR-196 allowlist config lacks a BR-196 citation. These are
  deferred to their accepted Gate B implementations; both formal reviewers
  received the evidence.

- BR-196 formatting and focused acceptance tests pass 20/20. The production
  registry/closed manifest, six-governance-smoke validator, pinned target
  authority, production-target deny, ephemeral batch permit, spawn-time clock
  recheck, receipt audit and layered summary are present.
- The BR-196 quiet-hour repair is complete: invocation-scoped typed smoke
  authority uses a fixed verified Shanghai daytime instant only for the exact
  six governance tuples, while the ordinary governor remains quiet-hour
  denied. Focused tests pass 21/21 and `monitor_help_isolation` passes 26/26.
  The bounded `--test --push-dry-run` command exits zero with A48/D13/R3/T64,
  rendered=48, smoke=6/6 and zero process/batch/receipt activity.
- Live BR-196 Gate D remains externally blocked because no separately reviewed
  non-production Feishu conversation is configured. The current real target is
  hash-only classified as `production_deny`; no live message was sent.
- The earlier BR-201 Gate-A review was RED with 0 Critical, 6 Important and 1
  Minor. Those findings are now represented as remediated design debt pending
  the fresh formal review above.
- Added the Gate-D coverage closure design and marked the 2026-07-18 plan
  historical. The honest planning baseline includes `src/broker.rs` and
  `src/calendar.rs`: global 149,200/189,647 (78.67%) and core
  121,404/155,308 (78.17%), leaving at least 2,518 global and 26,139 core
  covered lines. BR-202 registration and independent Gate-A review are still
  blocking.

## 2026-08-01 R-08/A-10 closure and runtime-gap audit

- Closed all five Important findings from the independent R-08/A-10 review:
  verified-empty terminal replay, exact CFFEX reminder-date projection and
  canonical ordering, truthful missing CNInfo category rendering, public-only
  R-08 transition lineage, and schema-v4 A-10 source-batch delivery binding.
- Focused evidence passes: R-08 26/26, BR-160 source-batch envelope/durability
  3/3, monitor governor lineage 1/1, CFFEX formal-admission 1/1 and startup
  reconciliation 1/1. `cargo fmt --check` and `git diff --check` pass.
- Added the missing BR-199 citation to the CFFEX gateway. The business-rule
  checker now exits zero; its remaining output is historical warning-only.
- BR-201 Gate A was revised after four Important objections. It now specifies
  fixed `+08:00` fail-closed calendar authority, an unforgeable engine permit,
  lazy risk-context loading, pre-provider and pre-side-effect revalidation,
  11:30/15:00 TOCTOU behavior and explicit legacy/BR-154 disposition. Fresh
  independent acceptance is still running; implementation has not crossed the
  Gate A boundary.
- The `--test` audit confirmed that 40 renderer previews are not the production
  template closed set. LimitBoards, six normalized source cards and the real
  R-08 renderer are omitted; six smoke paths discard PushOutcome. BR-196 Gate A
  is being repaired in parallel before production code changes.
- The existing coverage snapshot is a real Gate-D blocker and is stale relative
  to the current worktree. It reports 149,200/189,647 global lines (78.67%, at
  least 2,518 additional covered lines needed for 80%) and 67,101/84,179
  registered core lines (79.71%, at least 12,870 needed for 95%). The core gap
  is systemic, concentrated in `data_gateway` and `database`; thresholds and
  denominators will not be weakened.
- Broadened the core coverage registry to include the omitted production
  `auth`, monitor binary, durable-delivery, market-analyzer, monitor, portfolio
  and selection paths. Against the stale snapshot the honest core result is
  121,025/154,838 = 78.16%; this remains diagnostic until coverage is regenerated.

## 2026-08-01 root continuation recovery

- Re-read the repository-wide mandatory instructions and restored the persistent
  unified-migration plan before changing product code.
- Confirmed the worktree is intentionally large and dirty; unrelated user
  changes will be preserved and no destructive Git cleanup will be used.
- The active closure order is BR-192 metadata review, BR-193 cadence/acquisition
  Gate B, BR-194 durable-delivery Gate B, then dynamic price-limit Gate A and
  complete Gate C/D runtime evidence.
- Fixed-20% behavior remains frozen at Gate A because current `AGENTS.md` 2.3
  mandates manual confirmation while the user has explicitly requested a
  market-regime-aware rule. No implementation change will cross that unresolved
  design/authority boundary.

## 2026-07-29 BR-192 recovery-state Gate A repair

- Parallel independent re-review returned 0 Critical / 2 Important, so BR-192
  is not yet design-ready.
- The missing cases are both post-sink: a real sink may accept a message before
  delivery-audit or task-transition persistence fails, while current cooldown,
  schedule and daily-budget accounting do not share one recoverable identity.
- Gate A repair is in progress. The required design records physical sink
  acceptance durably, consumes the user-visible daily budget at physical
  acceptance, reconciles only the original binding/receipt/decision identity
  with zero provider and sink calls, and makes an unpersisted receipt an
  explicit manual-recovery state that is never auto-retried.
- Three independent work streams are active: upstream collision-safe batch
  identity, downstream BR-192 design repair, and production-composition probe
  review. Full upstream gates remain intentionally paused until these blockers
  converge.

## 2026-07-29 Provider Top-N release closure

- Independent review rejected the first concrete Top-N route because public
  Core metadata could be forged and a direct Eastmoney dependency would
  violate Router provider neutrality.
- Moved the concrete binding into the new upstream
  `magic-market-composition` crate. Its public constructor is zero-argument and
  creates the production `EastmoneyClient` internally; Core is acquisition-only
  again, Router remains provider-neutral, and no caller-owned transport or
  generic registration method is exposed.
- Added the missing deterministic unsupported-metric regression. It asserts
  `FailureKind::Unsupported` and zero provider calls for an unadmitted metric.
- Upstream formatting passes and the exact composition suite passes 12/12
  (6 private contract/failure tests plus 6 public integration tests) under
  `--locked --offline`.
- The first probe-evidence formatter compile attempt failed because
  `magic-eastmoney-rs` did not expose the `time` crate to examples. The fix is
  a dev-only exact `time` dependency for diagnostic timestamp rendering; no
  production Provider dependency or clock semantics are changed.
- The immediate locked rerun correctly refused the stale lockfile after that
  manifest change. Regenerate the lockfile offline once, then restore all
  subsequent `--locked --offline` gates.
- Regenerated the lockfile offline and the Eastmoney live-probe example test
  passes under `--locked --offline`.
- The complete upstream workspace/all-target/all-feature test suite passed
  after the composition change.
- The first live rerun failed only inside the restricted sandbox at DNS
  resolution; the required unsandboxed rerun then passed both metrics with
  real data. Volume-ratio started at `22:25:22.507854+08:00` and completed at
  `22:25:25+08:00`; main-net inflow started at
  `22:25:25.629507+08:00` and completed at `22:25:29+08:00`. Both returned
  20/20 rows from a provider-declared 5,542-security universe, retained
  `source_at=None`, and ended with `live_probe_status=admitted`.
- Updated the evidence record, Eastmoney integration guide, root README,
  design status/terminology and crate README so the 15:35 gate and the
  composition ownership boundary are auditable.
- Final static rerun currently passes formatting, locked/offline metadata,
  compliance and documentation links. The attempted
  `tools/diff/check.sh` command was an obsolete path and exited 127; locate
  and run the repository's actual diff-hygiene command instead of treating
  this as a product failure.
- Final composition/full workspace/coverage/live gates and the exact-byte
  independent re-review are still pending; no upstream merge SHA has been
  claimed.

## 2026-07-29 business-first continuation

- Passed the isolated business command `cargo run --bin monitor -- --test`
  with exit 0 and no production side effects.
- Passed the real review command `cargo run --bin monitor -- --review` with
  `attempted=8 delivered=2 no_data=2 waiting=1 disabled=3 failed=0`.
- Revalidated the unified dependency baseline: all 13 Magic crates are pinned
  to the same immutable upstream revision and the removed `qmt-parser` has not
  re-entered the graph.
- Verified that CFFEX futures-delivery cannot be enabled honestly at the
  current upstream release: the typed capability is false and official HTTPS
  admission remains failed transport. Kept the downstream state explicit
  rather than fabricating an empty calendar.
- Integrated the macOS GlobalSchema/WAL route repair from the independent
  slice. Targeted compilation and the exact former failing owner regression
  remain to be run after the non-overlapping parallel edits finish.
- Verified upstream release state through GitHub: PR #1 is merged and all
  recorded checks are green; local and remote upstream `main` both resolve to
  the downstream-pinned `660902ff...` revision. A separate downstream PR
  evidence query hit an API connection failure and remains pending.
- Audited README/config truthfulness against BR-181. The intended README
  contracts and stale-key deletion are present; the BR-181 design status still
  requires reconciliation with its independent-review evidence.
- Passed the non-Cargo static cleanup probes: 13/13 identical Magic pins, no
  active stale config keys or deleted runtime TOML references, no RustDX
  references, and no qmt-parser dependency. Left remaining provider facades
  for the architecture ownership audit instead of deleting by filename alone.
- Traced the repeated BR-134 warning in normal mode to the exact control flow:
  one unavailable pre-close paper quote aborts the whole exit batch and the
  success-only debounce never advances. Logged it as a remaining
  business-availability repair rather than silencing the warning.
- Completed the R-02/R-05/R-06 capability audit and removed R-02's
  post-disable partial fetch. All three remain explicitly Disabled with precise
  evidence-contract reasons; no generic trade or partial market data was
  relabelled as a verified review result.
- Applied canonical rustfmt to the one reported GlobalSchema formatting hunk;
  `cargo fmt --all -- --check` now passes.
- Completed the R-08 global-market evidence diagnosis. Kept index/FX
  fail-closed because the fixed upstream index payload lacks provider time and
  the FX payload is minute-level while the current contract is realtime.
- Passed all 8 CFFEX Gateway tests, including formal Provider admission and
  malformed/partial evidence rejection.
- Passed the BR-140 review preflight test plus the R-02 zero-acquisition and
  R-05 authoritative-lineage disabled regressions.
- Re-ran the exact macOS GlobalSchema owner regression: the former SQLite
  `CannotOpen(14)` no longer occurs; a deeper TEST_CODE audit-container
  mutation assertion is now the active red loop.
- Passed all 15 `tests/unified_data_architecture.rs` regressions. The checked
  static ownership/deletion rules currently find no BR-164/167/175 violation.
  A separate structural audit still identified unsupported-but-consumed
  intraday-shape, board and market-ranking facades; those are feature gaps, not
  proof that legacy acquisition remains.
- Passed the required production data-freshness gate:
  `stock_daily` latest date is 2026-07-28, exactly one trading day behind the
  2026-07-29 check and within rule 2.4.

## 2026-07-28

- Re-froze BR-174/176/177/178 Gate A at design SHA-256
  `177034487fdc5684b48802cc2bdfad4244ea9fe0ab416fcba9b9c7088aa5d8d3`
  and business-rule SHA-256
  `3e273bdfdd522e198583d8d5a1f3421d0bd45bfcfaa745cc5fbe8787c1ed0d91`;
  two independent reviewers of those exact bytes returned zero blockers.
- Integrated the canonical board proposal/artifact parser, fixed Magic TDX
  runtime constructor, config activation binding and non-forgeable board
  evidence API. Focused tests pass: board 5/5, runtime 7/7, activation 5/5 and
  schema 13/13.
- Added all eight permanent run-kind v2 audit phases while preserving historic
  phase parsing, plus exact audit lookup available only from the held OS-lock
  session. Audit tests pass 20/20 and strict library Clippy passes.
- Exported `selection_v2_repository` into the real compile graph and fixed its
  representation-only large-enum Clippy finding. Root-session
  `cargo check --lib` passes.
- Independent repository review blocked Gate B on transaction choreography:
  the earlier skeleton combined envelope and stage, used unlocked generic
  audit proofs, lacked generation/outcome staging, recovery execution and the
  verified read model. Parallel TDD slices now address the independent
  envelope transaction, pool-wide FULL/FK connection contract and typed
  generation/outcome contract; monitor remains disconnected until recovery
  tests pass.
- Completed BR-174 Gate A at frozen design hash
  `a18dcd69c563f2df1e2ab104972cb8844eaa47c15fc17d034d6475f54d0eb6a5`
  and business-rule hash
  `3627f8526920264a258c75017ff9bbe9859c2ae194937bc3f330f8a3694fe7e6`;
  three independent read-only reviews reported zero blockers.
- Added the evidence-preserving four-feed raw global-news acquisition seam.
  Its focused suite passes 4/4 and proves Available/VerifiedEmpty/Unavailable
  terminals remain distinct before notification simhash.
- Added and exported the canonical schema-v2 hash/type contract. Project-level
  `cargo check --lib`, strict `cargo clippy --lib -- -D warnings`, and all 7
  schema-v2 tests pass.
- Added and exported deterministic source-ingress preparation. Its 3/3 tests
  cover complete/empty/unavailable feeds, first-source authority, replay,
  conflict, and same-day/stale/future admission.
- Consolidated strict provider-board binding/admission into the sole public
  `data_gateway::board` module, kept directory/flow acquisition private, and
  added the exact Magic TDX `board_constituents(limit=10000)` boundary. All 8
  focused board tests pass; the checked-in registry remains explicitly
  direct-only because no live-proven binding artifact may be fabricated.
- The first SQLite-v2 tracer passed its 10 local tests but failed independent
  design review: direct SQL could obtain config, generation and D3 receipts
  without required lineage. Gate B was reopened and the exact bypasses are now
  mandatory negative regressions; the skeleton is not counted complete.
- Continued Gate B in parallel: append-only SQLite v2 DDL/guards, exact Magic
  TDX provider-board admission, and deterministic raw-news ingress preparation
  are isolated to non-overlapping files. None is counted complete until it is
  exported and passes project-level tests.

## 2026-07-25

- Restored the current `/goal` and repository-wide AGENTS.md constraints.
- Confirmed the remaining upstream release blocker is critical coverage:
  93.78% versus the required 95%; overall coverage is already 82.11%.
- Persisted the remaining two-repository plan and validation requirements.
- Completed the mandatory pre-flight reads of
  `docs/ENGINEERING_RULES_V2.md`, `.github/copilot-instructions.md`, and
  `CLAUDE.md`; no precedence conflict changes the current Gate D plan.
- Next action: add deterministic upstream contract/failure tests and run
  targeted tests before regenerating full coverage.
- Added deterministic core calendar/global/policy/research validation tests.
- First targeted gate stopped at formatting differences; no test result was
  claimed. The next attempt applies rustfmt before rerunning tests.
- Applied rustfmt and passed all `magic-market-core` tests.
- Added and passed research content-type plus dragon-tiger turnover, evidence,
  identity, duplicate-rank, and checked-deserialization tests (9 signals tests,
  6 research-document tests).
- Added TDX unexpected-identity and facade coverage. The targeted run exposed a
  production-contract defect: `TdxService::quotes` did not reject a Beijing
  instrument before SmartClient failover. The regression test is retained
  while the preflight is diagnosed.
- Corrected the diagnosis after tracing the adapter and live-evidence comments:
  Beijing quote market `2` is supported. The actual inconsistency is the
  service-local mapper rejecting Beijing while the normalized adapter supports
  it; the next patch aligns that mapper and keeps security metadata unsupported.
- Aligned the service-local mapper with BR-007 (`Beijing -> market 2`). A
  separate attempt to cover raw facade methods proved non-deterministic because
  SmartClient can connect/fail over and legitimately return live data; that
  coverage-only test will be removed rather than accepting either outcome.
- Added isolated Router tests for optional/missing/invalid evidence dates,
  dragon-tiger limits and duplicates, post-close rank/instrument/name/order
  faults, northbound provenance/cardinality, and previously masked
  announcement/economic/policy duplicates/order.
- Targeted Router coverage now reports 1,599/1,687 lines (94.78%); remaining
  misses are mostly constructor-invariant/unreachable defensive branches.
- Added source-identity accessor coverage for calendar/global/policy/research
  records and corrected the oversized-PDF fixture to reach the size gate. The
  first compile used a non-existent provider variant; it was corrected to the
  canonical `StateCouncil` identity before rerunning.
- Passed the complete `magic-market-core` package test suite after those
  additions.
- Regenerated full upstream workspace coverage: overall 82.59% passed, critical
  94.82% remained 31 lines below the release gate.
- Added deterministic Eastmoney transport tests for default PDF/HTML forwarding,
  exact response-limit preflights, absent HTML media evidence, poisoned limiter
  and request-probe locks, and unbalanced probe completion.
- Passed all targeted Eastmoney transport tests. Package coverage for
  `transport.rs` increased from 245/382 to 304/382 covered production lines
  (+59), which is sufficient mathematically; the authoritative workspace
  coverage report is being regenerated before claiming Gate D coverage.
- Authoritative workspace coverage passed: overall 82.76%, critical 95.17%.
- Passed upstream formatting, strict workspace Clippy, compliance, docs-link,
  and diff checks.
- The explicit full workspace test then exposed a real TDX heartbeat/pool race:
  `close_all` zeroed `active` while a heartbeat guard was alive, so guard drop
  underflowed and poisoned the mutex. Returned to Gate B instead of accepting
  the earlier coverage-run success.
- Registered BR-029 and added the reviewable TDX pool close-race design before
  implementation. Pool generations now invalidate pre-close guards, active
  reservations survive close until return, failed connect/handshake
  reservations are released, and counter contradictions no longer subtract
  unchecked in `Drop`.
- Passed five deterministic pool tests including the active-guard/close race,
  and reran the previously aborting adapter test successfully (67.61s).
- Revalidated TDX strict Clippy, formatting, BR-029 compliance, documentation,
  and diff hygiene.
- Full upstream workspace tests now pass after the pool repair, including 287
  TDX library tests and all doc tests. The former heartbeat guard-drop abort did
  not recur.
- Authoritative upstream coverage now passes at 82.85% overall and 95.17%
  critical. CNInfo whole-market announcements, Eastmoney limit pool and
  whole-market dragon tiger, Sina instrument news, Magic TDX board membership,
  SSE/SZSE dragon tiger, and HKEX northbound live probes returned admitted,
  source-evidenced records.
- CFFEX is the only open live probe. The dedicated probe and `curl` both fail
  at TLS initialization. Google DoH returned five official public IPs; direct
  TLS 1.2 attempts to every IP were also closed after ClientHello. This is an
  external network path failure, not a parser-only inference. The existing
  GitHub Actions remote live workflow will be used as an independent gate; no
  HTTP downgrade or fabricated delivery calendar will be introduced.
- Fixed downstream private-Git resolution with project-local Git CLI fetching.
  Cargo locked all 15 Magic dependencies to upstream release `0.2.0` at
  `4f2730b6`; `cargo check --lib` passed.
- Superseded the stale futures-delivery gap conclusion: the released upstream
  includes `CffexClient` and the typed official-notice delivery contract.
- Registered BR-165, added the evidence-preserving CFFEX delivery Gateway, and
  integrated it as the sixth independent R-08 component. The consumer only
  reminds when the official delivery date is exactly the next calendar day;
  an unpublished notice remains explicit unavailable/waiting.
- Passed four CFFEX Gateway tests, nine R-08 tests, and
  `cargo check --bin monitor`.
- Restored the downstream migration plan after context compaction and re-read
  the repository engineering rules before continuing Gate B.
- Registered BR-166 and added the reviewable global-news Gateway design. The
  active implementation slice replaces the governed news aggregator's direct
  Jin10/CLS/Eastmoney and legacy flash providers with the released typed
  Eastmoney/CLS/Jin10/The Paper clients; provider identity and timestamp
  contracts are being verified against the pinned upstream source before code
  admission is implemented.
- Implemented `GlobalNewsGateway` for all four released providers with
  provider-specific source/time admission, future/order/identity/evidence
  rejection, BR-159 durable acquisition audit and blocking-client isolation.
- Passed all three deterministic global-news Gateway tests, including all four
  source contracts and rejection of future/out-of-order records and invalid
  limits.
- Replaced the production seven-provider legacy flash registry with four
  `UnifiedGlobalNewsFeed` instances (Eastmoney/CLS/Jin10/The Paper), deleted the
  old `SearchResult` feed adapters and unimplemented poller shells, and made
  all registered feeds execute concurrently with ordered per-source outcomes.
- The MarketEvent projection preserves real publication/observation evidence,
  uses stable identities, and intentionally keeps unknown impact at
  Neutral/strength 0 instead of inventing sentiment.
- Passed 37 aggregator tests, including deterministic parallel polling and
  non-invented impact projection, and passed `cargo check --bin monitor`.
- Re-ran the BR-164 architecture gate. It remains intentionally RED with 55
  direct financial/news source violations, down from the prior 59; the
  production global-news registrations are no longer on the violation list.
- The enabled upstream remote live workflow completed with TDX/CLS/Sina/Baidu/
  Tencent/THS passing and exchange/CNInfo/Eastmoney failing. The PR remains
  unmerged while the three failing job logs are classified.
- Classified the three remote failures: Eastmoney is a missing workflow env
  input; exchange stopped on SSE 403 before CFFEX; CNInfo instrument retrieval
  passed but whole-market pagination rejected contradictory `hasMore` evidence.
  Upstream stays at Gate D pending targeted fixes and a new run.
- Traced the CNInfo failure to duplicate production implementations. The
  verified `MarketAnnouncements` path already models CNInfo's quotient
  `totalpages` field and derives the final request page by ceiling division;
  the older `AnnouncementDiscovery` path still treats `totalpages` as a
  conventional final page. Gate B is reopened to consolidate the old trait
  onto the verified path, not to relax the atomic pagination rule.
- Amended the CNInfo design with explicit old-module disposition and deleted
  the obsolete discovery request/trait, CNInfo mapper/config path, and Router
  adapter. Renamed the capability to `market_announcements`; targeted Core,
  CNInfo and Router test suites pass.
- Reproduced the exact SSE announcement request locally with the same
  endpoint, query and headers: HTTP 200, JSON media type, 5,806 bytes. The prior
  GitHub-hosted 403 is therefore recorded as a remote-path failure, not parser
  evidence.
- Updated SSE public requests to use browser-equivalent static headers without
  cookies or credentials, added exact header tests, fixed the Eastmoney
  workflow's missing discovery date, and added an independent CFFEX delivery
  live job. Exchange targeted tests, full strict Clippy, full workspace tests,
  formatting, compliance, docs links and diff hygiene all pass.
- Replaced the coverage-only TDX loopback-socket regression with a deterministic
  test of the extracted generation/accounting transition. Targeted strict
  Clippy and the close-with-active-reservation regression pass; the
  authoritative workspace coverage report is being regenerated.
- The regenerated coverage run progressed further and exposed two remaining
  loopback-bound TDX connection fixtures. Added the documented private stream
  seam without changing `TcpConnection`'s public interface; deterministic
  connector/timeout/I/O tests, the public preflight test, formatting and strict
  package Clippy now pass. A fresh authoritative coverage run is required.
- The third coverage run passed all TDX tests and exposed the only remaining
  loopback fixture in THS. Added a repository-level transport-test design,
  reused the CNInfo-style ureq result seam in THS, and verified in-memory
  200/403/transport-error mapping. THS strict Clippy and its targeted test pass;
  a whole-workspace source audit now finds no listener/bind fixture.
- The fourth authoritative upstream coverage run completed successfully:
  overall 29,177/35,528 = 82.12% (required 80%) and critical
  15,457/16,237 = 95.20% (required 95%). The latest workspace formatting,
  strict Clippy, all-feature locked tests, compliance, documentation links and
  diff hygiene also pass.
- Re-ran the downstream BR-164 architecture test and captured the complete 55
  remaining violations for the next migration batches; no violation was
  suppressed or allow-listed.
- Pushed upstream hardening candidate
  `cc8d26dd60f3dc22f9356fdfeed86f6723cf8b84` and dispatched the real-data
  workflow. Coverage passes remotely; audit/check and three live jobs still
  require root-cause repair before merge.
- Added and targeted-tested the BR-167 `EconomicCalendarGateway`. Inspection
  confirms the macro-news consumer still uses legacy direct clients and
  mislabels latest releases as a future event window; the next tracer bullet
  migrates this consumer and deletes its obsolete calendar surface.
- Traced the macro consumer end to end: `fetch_financial_calendar` hides
  provider errors as an empty vector, while `search_macro_news` independently
  fetches WallStreetCN/CLS/Jin10 and performs display-string deduplication.
  The replacement seam will consume typed Gateway batches and make each
  provider failure visible without fallback.
- Confirmed the obsolete public calendar helper has zero callers. The planned
  tracer bullet will therefore migrate `search_macro_news`, remove the helper
  and its exported legacy event type, while leaving generic user-authorized
  search visibly separate from provider-specific Gateway outcomes.
- Identified the minimal BR-167 renderer interface: one pure function consumes
  the four typed global-news outcomes and one typed economic-release outcome,
  returning report sections. Existing Gateway types already carry every
  explicit status/error field required by 2.2.
- Re-read the BR-167 Gate A design before the consumer change. It requires
  importance filtering only after complete admission, a 15-row display cap,
  latest-release terminology, and explicit unavailable/verified-empty
  rendering; the tracer-bullet test will assert those observable behaviors.
- BR-167 renderer RED confirmed: the exact targeted test fails with unresolved
  `render_gateway_sections`. No production behavior was changed before this
  failure was observed.
- BR-167 renderer GREEN: implemented one pure interface over four independent
  global-news Gateway outcomes plus the economic-release outcome. The exact
  test passes and proves success, verified-empty, retryable failure,
  importance filtering, source evidence, and latest-release terminology.
- While tracing the integration, found a second fallback path: the generic
  macro search loop iterates a mixed vector whose first entries are legacy
  financial-source adapters. The slice will add an explicit
  `supports_general_web_search` interface capability and restrict this loop to
  SerpAPI/Bocha/Tavily, preventing legacy source fallback by construction.
- The general-search capability RED is confirmed: SerpAPI, Eastmoney and CLS
  have no such interface method today. This proves the mixed vector cannot be
  filtered by capability before the implementation change.
- General-search capability GREEN: `SearchProvider` now defaults false and
  only SerpAPI/Bocha/Tavily opt in. The exact test proves Eastmoney and CLS
  remain excluded.
- Passed all 56 `search_service` tests after production integration. The BR-164
  architecture gate remains intentionally RED with the same 55 source/import
  violations because legacy provider files still exist; no allow-list or
  suppression was added.
- Audited the legacy Jin10 file and isolated the calendar-only symbols and
  tests. The flash path remains a separate unfinished BR-166 caller migration;
  only the now-unreferenced calendar protocol will be deleted in this slice.
- BR-167 deletion guard RED confirmed on `Jin10CalendarEvent`. Source ranges
  and the sole re-export are identified; the next change deletes those exact
  calendar-only artifacts.
- Deleted the complete legacy Jin10 calendar protocol and its re-export without
  touching the still-used flash path. The BR-167 source guard passes and all
  54 search-service tests pass after deletion.
- Classified the latest independent CFFEX remote failure: the provider starts
  with its typed delivery capability, then the GitHub runner reports network
  unreachable to the official CFFEX HTTPS notice path. The other three log
  reads hit a transient GitHub API connection failure and will be retried
  sequentially with narrower output.
- Classified the latest Eastmoney remote failures into deterministic repair
  slices: fix the live-probe completeness request, strengthen Dragon Tiger seat
  business identity, admit the verified official global-news host, and
  diagnose redirect targets before changing transport behavior. The isolated
  TLS resource error remains retryable evidence and will not be converted into
  fabricated success.
- Retrieved the correct upstream checks and cargo-deny jobs. The check repair
  is a missing optional CNInfo fixture field plus an `--all-targets` regression
  run. The security repair is to delete Sina's unnecessary `scraper` chain and
  explicitly admit only the permissive CDLA root-certificate license; no
  advisory ignore is planned.
- Sina parser dependency guard RED was observed against the existing manifest.
  The provider now uses a bounded BR-025 parser, the complete nine-test
  instrument-news suite is GREEN, `scraper` is absent from the crate manifest,
  and the CNInfo all-target fixture explicitly preserves the missing
  `instrument_name` value as `None`.
- The GitHub-equivalent `cargo test --workspace --all-targets --locked` now
  passes across the full upstream workspace, including the CNInfo example, and
  strict all-target Clippy passes for both changed crates. Local `cargo deny`
  could not run because that optional cargo subcommand is not installed; the
  repository CI action remains the authoritative audit runner and will be
  rerun after the complete repair commit.
- Added release-gate TDD coverage for Eastmoney. RED was observed for the
  missing redirect mapper and complete-pool probe helper, while the old seat
  and global-news semantics were proven incompatible with real source output.
  GREEN now preserves repeated institutional seats by source side/rank,
  admits only exact `finance.eastmoney.com` and `global.eastmoney.com` article
  hosts, requests the complete 1000-row limit-pool source page in the live
  gate, and reports bounded `Location` diagnostics without following redirects.
  All 151 Eastmoney library tests, its live-probe example test, formatting and
  strict all-target package Clippy pass.
- Confirmed the Sina dependency removal in the resolved lockfile:
  `scraper`, `fxhash`, `cssparser`, `cssparser-macros`, `selectors`, and
  `dtoa-short` are all absent. The next step is the complete upstream Gate C
  rerun before committing and dispatching the authoritative remote audit/live
  workflows.
- Completed the current upstream local release gates after the remote-gate
  repair: `cargo fmt --check`, strict workspace all-target/all-feature Clippy,
  GitHub-equivalent locked all-target/all-feature tests, compliance, docs links
  and diff hygiene all pass. Fresh pinned `cargo-llvm-cov 0.8.7` evidence is
  29,363/35,813 = 81.99% overall and 15,446/16,215 = 95.26% critical, both
  above the unchanged 80%/95% thresholds.
- Deleted the unused `qmt-parser` dependency, its `Market` enum adapter and its
  two test-only enum assertions. The remaining QMT helpers are dependency-free
  string conversions. The architecture dependency guard and all seven code
  mapping tests pass.
- Verified the dependency graph: qmt-parser is absent and Polars has a single
  owner/version (`polars 0.54.4 -> stock_analysis`). Added the explicit
  `strings` feature required by temporal streaming and migrated the sole
  `LazyFrame::drop` call to the 0.54 selector API. Formatting and strict
  workspace all-target/all-feature Clippy pass.
- Removed the no-caller legacy policy push wrapper/test left behind by the
  SearchService financial-source deletion, then removed the obsolete direct
  Eastmoney `test_em_fetch` binary. The exact BR-164 gate now has 41 remaining
  violations; it remains intentionally RED until all domain migrations finish.
- Upstream PR #1 passed its authoritative GitHub gates and merged. Downstream
  now pins all Magic crates to released `0.2.0` revision
  `13b0172b436b43616d1f3969314dbb83e6d2facd`; no mixed Magic revision remains.
- Deleted the superseded `DataProvider`/`DataFetcherManager` interface and the
  legacy Magic TDX bridge. `cargo check --all-targets`, strict workspace
  all-target/all-feature Clippy and the four-test BR-164 architecture deletion
  suite pass.
- The first downstream full workspace/all-feature test run completed
  1,835/1,852 non-ignored library tests and exposed 13 contract/fixture
  regressions. They are being repaired by module without restoring local
  `updated_at` as broker source evidence or weakening missing-data gates.
- Rechecked the resolved dependency graph: QMT is absent, Polars is exclusively
  `0.54.4`, and all Magic packages resolve to the single upstream merge
  revision.
- Repaired all 13 regressions from the first downstream full run and reran
  their exact tests successfully. The next complete run passed all 1,848
  non-ignored library tests; the `monitor` binary passed 440 tests and exposed
  six further contract regressions grouped into BR-165 delivery rendering,
  candidate-panel fixtures and real-raw wrapper behavior. Those groups are
  under independent root-cause repair; no production freshness, provenance or
  missing-data gate is being relaxed.
- Repaired the six second-run monitor regressions without restoring legacy
  acquisition. BR-165 now explicitly renders `NotProvided` instead of
  inventing cash settlement; the obsolete synchronous candidate wrapper and
  its raw fallback fixtures are deleted; and the production D-01/P-03 path now
  awaits exact realtime-quote and Tencent market-statistics Gateway batches,
  retains both evidence records, excludes only confirmed positions (not the
  watchlist), consumes real `volume_ratio`, and keeps unsupported money-flow
  heat absent.
- Removed the contradictory database-layer fixed-20% K-line validator.
  Database writes and repository reads now share the board-aware BR-092
  validator: main-board 10.5%, STAR/ChiNext 20.5%, Beijing 30.5%, with
  source-backed IPO/ex-rights exceptions only. Exact database, repository,
  backfill, quality and IPO registered/unregistered tests pass.
- Re-ran the real unified-Gateway backfill for 688548 and 688690. Both accepted
  complete 90-row Baidu PAE batches after Magic TDX transport attempts were
  explicitly rejected as truncated, wrote through 2026-07-24, and recorded
  immutable `HistoricalDailyBars` acquisition evidence. `stock_daily` now
  contains 100 rows for each code and the freshness gate passes at one trading
  day of lag.
- Recorded the remaining lifecycle-evidence gap rather than fabricating a
  release exception: production has no trusted caller of `mark_ipo` or
  `mark_ex_rights`; unknown IPO/ex-rights jumps therefore continue to fail
  closed until a typed upstream lifecycle batch is admitted.
- Reproduced the isolated monitor gate: 17/20 process tests passed and three
  failed from one root cause. The v70 TEST_CODE trade was passed to the
  production HistoricalDailyBars Gateway, which correctly rejected the
  non-six-digit identity before the final marker. BR-051 already requires
  isolated E2E to avoid external market calls, so the test-only review branch
  now explicitly skips post-exit daily-bar enrichment while production
  `--review` remains unchanged and fail-closed.

## 2026-07-28

- Registered BR-174/BR-176 and wrote the Gate A selection-evidence closure
  design. The revised physical cutover keeps `selection_candidates` and its v1
  foreign-key graph legacy-only; schema-v2 admitted visibility is derived from
  receipted `selection_samples`, while v2 settlement reads only v2 samples.
- Added design-level contracts for separate receipted source ingress,
  evidenced Available/VerifiedEmpty/Unavailable feed attempts, zero-result run
  manifests, exact domain-separated hash preimages/golden vectors, per-binding
  audit hashes, immutable complete relation barriers, runtime Production/Test
  store modes, and T0/D1/D3/D5 dual-cohort settlement.
- Independent schema review found that production pooled SQLite connections
  currently lack pool-wide `foreign_keys=ON` and use
  `synchronous=NORMAL`. Gate B now requires every connection to read back
  `foreign_keys=1` and `synchronous=2` before v2 migration. Formal Gate A
  re-review is running; no implementation is allowed until objections are
  zero.
- Two formal Gate A reviews have now completed against successive revisions.
  Their objections were folded into the design as BR-177/BR-178 and exact
  contracts for config activation/effective time, envelope-only recovery
  ordering, per-feed no-loss receipt checks, complete v1 graph quiesce/write
  denial, receipt-to-external-audit verification, v2 due/report ordering and
  limits, executable zero-delivery evidence and rollback-switch preflight.
  A fresh independent zero-objection review is still required before Gate B
  implementation starts.

- BR-174/176/177/178 Gate A is now complete on frozen design SHA
  `177034487fdc5684b48802cc2bdfad4244ea9fe0ab416fcba9b9c7088aa5d8d3`
  and business-rule SHA
  `3e273bdfdd522e198583d8d5a1f3421d0bd45bfcfaa745cc5fbe8787c1ed0d91`.
  Two independent final reviews of those exact bytes reported zero blockers.
  The final closure includes record-level provider hashing, fixed no-follow
  evidence paths, main/WAL/SHM snapshot binding, runtime writer-freeze,
  exact-one canonical canary payloads and post-generation SQLite snapshot
  chronology.
- Gate B started with non-overlapping parallel slices. Added the raw global
  news collector that fixes the four registered Gateway identities/order,
  retains Available/VerifiedEmpty/Unavailable evidence before notification
  simhash, fails invalid limits before provider calls and drops raw provider
  messages at the typed boundary. Its four focused library tests pass.

- Re-ran the current upstream locked/offline workspace coverage gate at clean
  revision `b2b68df78156df1d67824e5c44c0cb01b752f55a`. The unchanged checker
  reports 37,630/43,268 = 86.97% overall and 18,751/19,716 = 95.11% critical,
  passing the required 80%/95% thresholds. Coverage instrumentation was
  cleaned after the report was written to
  `/private/tmp/magic-market-data-rs-coverage-2026-07-28.json`.
- Completed bounded upstream live evidence for WallStreetCN, SSE, SZSE, HKEX,
  CNInfo and Eastmoney. The CFFEX provider remains fail-closed and advertises
  `futures_delivery=false`: Rustls, native TLS and curl all failed DNS
  resolution for the official HTTPS host, so no record was fabricated,
  inferred or admitted.
- Cut A-01 review history over to `HistoricalBarsGateway`, preserving provider,
  source as-of time, observation time and immutable batch identity. Removed
  duplicate local routing and validation from that caller.
- Cut Bocha, Tavily and SerpAPI over to the typed BR-175
  `GeneralWebResearchGateway`. Search output carries ResearchOnly evidence and
  cannot become a financial, trading, policy or formal-selection fact. Deleted
  the old provider-local HTTP implementations.
- Removed duplicate Magic TDX realtime ownership from the T0-specific gateway;
  `MarketDataGateway` is now the single owner of general realtime quotes.
- Passed the unified architecture tests, focused Gateway/search tests,
  formatting, strict Clippy and the complete downstream compliance script.
- Completed a read-only formal-selection audit. The current safe path includes
  direct-mentioned securities, 21-day daily bars, realtime/five-minute
  price-volume snapshots, deterministic hard rejection, immutable audit and
  T0/D1 outcomes. Remaining Gate A work is traceable industry/chain expansion,
  queryable rejected samples, D3/D5 outcomes and a formal backtest containing
  both selected and rejected samples. The legacy `opportunity` acquisition
  path is not an admissible shortcut because it substitutes default money-flow
  values after failures.
- Upstream typed-cardinality PR #3 merged at
  `660902ff93a07f18367dc16879cf67732accd25a`. All 13 downstream Magic manifest
  and lockfile entries now resolve to that exact `=0.2.0` Git identity; the
  standalone release-revision guard and the 11-test BR-164/167/170/171/175
  acquisition-ownership suite pass.
- Imported the user-attested 2026-07-28 23:00 live account screenshot through
  the three production one-shot importers. The append-only position snapshot
  contains exactly seven items, the immutable real-account snapshot preserves
  screenshot SHA-256
  `08117ccb690142b404cda4b84134c827c84eb0824fc2dc95d223bc62884f51e0`,
  the account summary matches the same totals, and SQLite `integrity_check`
  returns `ok`.
- The mandatory 2026-07-29 freshness preflight initially failed with
  `stock_daily` at 2026-07-27. The repository backfill path admitted and wrote
  31/33 requested symbols through the unified Gateway to 2026-07-28; 688548
  and 688690 remain explicitly blocked at 2026-07-24 because no matching
  BR-171 manual daily-change confirmation exists. The global freshness gate
  now passes at one trading day of lag; the two symbol-specific gaps remain
  open and were not auto-confirmed.
- Completed the typed outcome-attempt slice. Outcome attempt/row contracts are
  v3, the payload is v2, ExpectedWait/Settled/Error use an exact typed state
  matrix, and every adaptive provider attempt remains hash-bound even when a
  later attempt fails after an earlier success. Schema tests pass 2/2,
  outcome tests 11/11, and all 15 Gateway cases have passing evidence.
- Completed the storage-free process-bootstrap slice. One zero-argument
  library facade owns the sole real `args_os` read, installs one opaque
  non-Clone proof, rejects repeated initialization, and strictly separates
  production from TEST_CODE symbols. Help/version/invalid/operational-unready
  child-process tests pass 4/4 and prove the fixed production DB, WAL/SHM,
  Magiclaw DB, audit file and audit lock are unchanged. Operational startup
  remains intentionally fail-closed until the private global schema owner and
  exact STSA/1 migration are complete.
- Completed a read-only README/config audit. It found five blocking
  documentation/implementation mismatches, seven target-file legacy config
  references, eleven additional stale source comments/logs and three safe
  dead-config/code removals. It also produced the mandatory keep-list:
  compliance design contracts, active chain config sections, role/search
  environment keys and all thirteen pinned Magic crates. No cleanup edit is
  allowed to weaken those active contracts.
- Completed the first shared `GlobalSchemaVersionOwner` primitive at source
  SHA-256
  `735b92edd9386a6a42603ac463b1acba74d79c75978be718564b821da4542d7c`.
  Its twelve exact tests pass and the final independent security review reports
  zero Critical/Important findings after closing namespace ABA, main/WAL/SHM
  pinning, symlink/hardlink/FIFO handling and exec descriptor inheritance.
- Selection-v2 database evidence now totals 108 passing tests: 72 exact schema
  catalog/identity cases, 25 repository cases and 11 migration CLI cases.
  Production migration remains hard-disabled. A read-only backup assessment
  of the real `0/0` database proved that enabling it now would violate the
  frozen design because exclusive global authority, the complete whole-app
  generation registry, production outcome-claim ownership and global
  migration/audit/rollback receipts are still absent.
## 2026-07-29 — 当前并行收口进度

- 已确认 `IntradayShapeGateway` 采用既有 `MagicTdxGateway::get_t0_evidence_batch` 的同源同批纯投影方案；实现由并行任务按 BR-187 推进。
- 板块消费者切换按真实能力推进：资金流只消费 Magic Eastmoney 排名批次，证券→板块关系只消费 Magic TDX membership；不伪装全市场涨幅排序或无证据成分行情。
- GlobalSchema macOS 原 `/dev/fd/<dbfd>` 打开失败已消除；当前继续修复生产 owner 自身 WAL/SHM 改变 namespace marker 的事务生命周期问题（BR-189），拒绝 test-only 绕过。
- README 全球市场能力行已纠正：Sina 全球指数当前缺 provider `source_at`，不能宣称完成时段/隔夜批次；R-08 保持显式降级。
- 当前工作树是长期迁移累计态：199 个 tracked 文件有变更，另有大量 Gate A/B 设计与新模块未跟踪。必须在三路实现合流后重新跑全量门禁并审查变更边界，不能以先前局部通过结果宣称完成。
- GitHub 当前没有这轮迁移对应的 open/draft PR；最新列表仅有既往已合并 PR #1–#13。活动分支 `feat/event-scoped-selection-shadow` 相对远端 `stock_analysis/master` 的 merge-base 之后有 22 个本地 commit，且仍有上述大规模未提交变更。最终 PR/merge 尚未开始，不能把旧 PR 合并记录当作本轮证据。
- `target/` 已通过 `cargo clean` 删除 7.6 GiB 可重建产物，保留源码、数据库与审计证据；清理前磁盘仅余约 12 GiB，必须控制串行编译/coverage 的峰值并在 Gate D 前复核空间。
- 清理后的复核显示数据卷可用约 18 GiB（96% 已用），`target/` 当前重建至约 564 MiB；工作树已达到 346 个变更/未跟踪条目，合入前仍必须完成逐项范围审查与 PR 证据。

## 2026-07-29 — Provider TopN 上游 Gate D

- `magic-market-data-rs` 隔离工作树 `/tmp/magic-market-rank-topn` 已完成
  Provider TopN Core 契约、Eastmoney 真实实现以及 provider-locked composition
  路由；生产构造器只接受 `EastmoneyClient`，不开放注入或伪造 provider
  metadata 的入口。
- 两类真实线上探针均通过：`VolumeRatio` 返回 20/5542，`MainNet` 返回
  20/5542；探针同时记录请求开始、批次/首条观测时间并明确保留
  `source_at=None`，没有伪造数据源时间。
- 最终二次 `cargo test --workspace --all-targets --all-features --locked
  --offline` 已全量通过；此前 `cargo fmt --all -- --check`、workspace
  严格 Clippy、compliance、文档链接与 `git diff --check` 也已通过。
- 当前仍在执行 Gate D 覆盖率、最终独立审查与上游 PR/CI/合并；在取得
  不可变 merge SHA 前，下游 BR-192 不允许写入临时分支 SHA。
- 第一次最终独立审查发现并阻断了一个生产信任缝隙：
  `EastmoneyProviderTopNRankingRouter::new(Arc<EastmoneyClient>)` 仍可接受由
  公开 fixture transport 构造的 client。根因是 client 类型未编码 transport
  真实性。已按 RED→GREEN 将公开接口收窄为零参数生产构造，client 在
  composition 内部创建；fixture provider/clock 仅保留在路径式内部测试。
- 同一轮审查发现 composition 纳入 critical coverage 后 checker fixture
  清单未同步、critical 源文件存在 inline tests。现已迁移到路径式内部测试，
  并更新 coverage checker 的 80%/95% 精确边界数据。composition 13 项测试和
  coverage checker 13 项测试均通过；首次无效 coverage 构建已主动终止，待
  修复后完整重跑。
- 修复后的 workspace 全量测试、严格 Clippy、fmt、compliance、docs links
  和 diff check 均已通过；三路精确字节复审结论为 0 Critical /
  0 Important。
- 首次修复后 coverage 重建因数据卷仅余 160 MiB 而以 `ENOSPC` 失败。只用
  `cargo clean` 清理两个上游工作树的可重建 `target/` 产物，未触碰源码、
  数据库或审计文件；数据卷可用空间恢复至约 21 GiB，coverage 将从干净构建
  重新执行。
- 干净全工作区 coverage 首轮所有测试通过并生成报告：overall
  `45935/52122 = 88.13%` 已过 80%，critical
  `27027/28570 = 94.60%` 未过 95%。未降低阈值或排除文件；按 RED 证据只在
  Core/Eastmoney/composition 的 Provider TopN 外部/路径式测试补齐真实失败
  分支，定向结果分别为 9/9、9/9、17/17。
- 覆盖率重跑期间的独立复审发现两个更高优先级 Gate B/D 阻塞，因此主动终止
  过期 coverage：Eastmoney `batch_id` 未绑定 metric/request/content，秒级同批
  请求会碰撞；线上探针也只证明了 Eastmoney trait，未经过零参数 production
  composition router。当前按 RED→GREEN 分两路修复，完成前不生成最终 coverage。
- 下游 BR-192 Gate A 文档已按独立审查修正为真实零参数 composition API，
  并补齐 immutable SHA/14-crate 原子 repin、非证券 delivery subject、
  binding/transition 时序、`--test` 零网络、周末/历史日期、retryability、双批
  原子失败与日预算。BR-192 自身静态错误已清零，正在做修订后独立复审；
  全仓 business-rule gate 仍被其他累计工作树的 21 项错误阻塞。
- batch identity 的两个碰撞 RED 用例现已 GREEN：同一观测时间的不同指标和
  同指标但不同规范化响应均生成不同 ID。实现使用 `ring` SHA-256，绑定
  kind/date/limit/filter/content/observed_at；Core 9/9、Eastmoney 12/12、
  composition 21/21 定向测试通过。
- 首次受限网络 production-composition 探针以两项
  `all registered market-data sources were exhausted` 显式失败且没有承认空批；
  获准真实网络重跑后，零参数 composition route 在
  `23:44:29/23:44:33+08:00` 分别承认 VolumeRatio/MainNetInflow
  `20/5542`，`source_at=None`，`failures=0`。
- 独立探针审查的 1 个 Important 已修复：旧 direct-provider 运行不再被称为
  最终 admission，失败日志将配置期身份标为 expected_provider/source。
  evidence 已改为仅凭实际 composition live 输出准入。
- BR-192 文档修订已补齐 TO BE BUILT delivery coordinator、耐久物理接收
  receipt、日预算 reserve/commit、uncertain-delivery 人工恢复、原 identity
  零 provider/sink reconcile 和 crash matrix；仍需新的独立 reviewer 对精确
  修订字节返回 0 Critical / 0 Important。

## 2026-07-31 — BR-194 Gate B→C→D 收口 (claude session)

**上下文**: plan file `/Users/zhangzhen/.claude/plans/structured-plotting-harp.md`；互斥分析 `.planning/2026-07-31-br194-gate-b-fix4/br192-mutex-analysis.md`。

### 已完成 (Step 0/1/2/3/4/5/6/7/8/9/10/12)

- **Step 0**: 写 BR-194↔BR-192 互斥矩阵 doc（0 冲突，6 条 BR-192 实施不变量）。
- **Step 1**: `cargo fmt --all`；修了 schema.rs:962 `verify_outbox_predecessor_self_fk` 多行签名、tests.rs:2166 `assert_eq!` 多行。
- **Step 2**: `cargo clippy --workspace --all-targets --all-features -- -D warnings` 修了 9 个 clippy error（durable_delivery_runtime.rs type_complexity、review_batch.rs bool_assert_comparison、v14_adapter.rs field_reassign_with_default、dispatcher.rs needless_borrows、tests.rs type_complexity → 引入 MemoryAppendRecord 结构、coordinator.rs enum_variant_names/items_after_test_module 两处 `#[allow]`、durable_delivery_runtime.rs type_complexity 三处 type alias）。
- **Step 3**: `bash tools/compliance/lib/check_br194_review_dependency.sh` 输出 `BR-194 review dependency static contract: PASS`。
- **Step 4**: `bash tools/compliance/check.sh` BR-194 部分 PASS；非 BR-194 部分（§2.10 BR-132/137/138/139 未登记、check_br174_legacy_callers 缺 rg）属 pre-existing 与 BR-194 无关。已 `brew install ripgrep` 解除环境依赖。
- **Step 5**: `cargo test --lib durable_delivery::tests::br194_` 全绿。原 `br194_schema_v5_migration_matrix_is_repeatable_and_rejects_newer_versions` 因 v4→v5 migration INSERT...SELECT 在 deferred FK 仍 per-row 检查、SQLite ALTER TABLE RENAME 不更新 self-FK target 两点而失败。修复：v4→v5 拆成两阶段（无 FK staging → rename → 再以 `REFERENCES immutable_audit_outbox` 重建），`PRAGMA defer_foreign_keys=ON` 在事务中保持。
- **Step 6/7/8**: replay 13 + review_batch 10 + notify 3 个 BR-194 精确测试全 PASS。
- **Step 9**: v14_adapter + calendar BR-194 测试 PASS。
- **Step 10**: `cargo test --test monitor_help_isolation br194_` 3 个 process 测试 PASS（`br194_test_review_blocks_r04_r09_provider_and_sink_before_account_gate`、`br194_terminal_replay_cli_rejects_ordinal_override_before_database_open`、`br194_terminal_replay_cli_rejects_duplicates_and_nontrading_dates_before_database_open`）。
- **Step 12**: `cargo build --release --bin monitor` 0 退出；`target/release/monitor --help` 含 `--br194-audited-terminal-replay` 入口。

### 未完成 (Step 11/13/14/15/16)

- **Step 11** 全工作区 sweep：2263 passed / 47 failed / 7 ignored。47 个失败集中在 `database::global_schema_v1::tests` 和 `database::selection_v2_repository::tests`，**均未触碰的 pre-existing 代码**（`git diff HEAD -- src/database/global_schema_v1.rs` 空 diff）。断言失败点 `global_schema_v1.rs:3771` 是 `SelectionAuthorityContradiction` 不匹配，与 BR-194 schema 改动无因果。需独立修复会话。
- **Step 13** `--br194-audited-terminal-replay --business-date ... --task R-04/R-09`：当前环境 `data/durable_delivery.sqlite3` 不存在（push log 自 2026-07-12 断流 19 天），CLI 正确返回 `terminal_replay_identity_invalid: expected one decision, observed 0`。需生产环境有真实 R-04/R-09 delivery 才能产出 spec §8.3 期望的精确 stdout。
- **Step 14** `python3 tools/release/verify_br194_review_join.py`：依赖 Step 13 真实 delivery 才能精确打印 spec §8.3 的 `BR194_JOIN` 行；当前环境只能验证脚本可执行。
- **Step 15** `cargo llvm-cov` + `python3 tools/coverage/check_thresholds.py`：未跑。
- **Step 16** PR + commit：未发出。

### 已知 pre-existing 失败

`database::global_schema_v1::tests::v2_audit_with_absent_database_half_fails_closed_as_contradictory`、`missing_audit_returns_database_half_only_and_never_authoritative_absent`、`selection_v2_repository::tests::outcome_persistence_owner_*` 等 47 个，断言 `SelectionAuthorityContradiction` 不匹配实际 error。修复需独立 PR 范围。

## 2026-08-01 — 47 pre-existing 失败根因调查 (claude session)

**范围**: §13.3 描述的 47 个 lib-test 失败根因分析（不动实现）。

### 单 test 跑 vs namespace 内跑

逐个单独跑 `cargo test --lib database::global_schema_v1::tests::v2_audit_with_absent_database_half_fails_closed_as_contradictory` 与 `cargo test --lib database::selection_v2_repository::tests::outcome_persistence_owner_commits_once_and_exactly_replays`：**两个都 PASS**。

跑整个 `database::global_schema_v1::tests` namespace（`--test-threads=1`）：24 pass / 2 fail / 2 ignored。两个失败都是同一个 `SelectionAuthorityContradiction` 不匹配断言（`global_schema_v1.rs:3771` / `global_schema_v1.rs:3740`）。

### 根因（global_schema_v1 namespace 失败）

`TestFixture::new(label, application_id, user_version)` 在 `src/database/global_schema_v1.rs:3445-3465` 创建 fixture：

```rust
let database = root.join("stock_analysis.db");
let connection = Connection::open(&database).expect("create test database");
// ...
connection.pragma_update(None, "user_version", user_version).expect(...);
drop(connection);
```

fixture **始终**用 `Connection::open` 创建空 SQLite 文件，fixture Drop 时清理。

测试 `v2_audit_with_absent_database_half_fails_closed_as_contradictory` 用 `TestFixture::new("selection-audit-v2-db-absent", 0, 0)` 创 fixture，但测试**名字**假设 database half 不存在。实际 database 文件**存在**（空 catalog、user_version=0），且 `pinned_audit_writer()` 后 audit record 也存在。两个 half 都 present → `inspect_selection_with_audit_for_test` 不走 contradiction 分支 → 返回其它 error（具体是 `Ok(diagnostic)` 或非 `SelectionAuthorityContradiction` error）→ 测试断言 `matches!(error, SelectionAuthorityContradiction)` 失败。

### 修复方向（database context owner 独立 PR）

任选其一（不在 BR-193 scope）：

A. 让 `TestFixture::new` 支持 "absent database" 模式（新增 `TestFixture::absent(label)`，不调 `Connection::open`，仅创 root 目录）。改 `v2_audit_with_absent_database_half_fails_closed_as_contradictory` 与 `missing_audit_returns_database_half_only_and_never_authoritative_absent` 用新 fixture。

B. 让 `inspect_selection_with_audit_for_test` 增加 contradiction 检测：当 database 文件存在但 catalog 完全空，且 audit evidence 存在时，返回 `SelectionAuthorityContradiction`。

### 剩余 45 个 selection_v2_repository 失败

单独跑 PASS、namespace 内跑 FAIL（与 global_schema_v1 同模式）。根因未深入调查；可能是 process-local OnceLock、temp 文件竞争、或 wall clock 影响。需独立 PR 由 database context owner 调查（不能在 BR-193 scope 内静默修复、削弱断言或 `#[ignore]`）。
## 2026-08-01 BR-192 metadata correction

- Fresh reviewer result was RED C0/I1/M0 because counted PushKinds without a
  real producer had no closed exact startup banner contract.
- Amended and exact-staged only the BR-192 design, plan, and BR-192 business
  rule row. The catalog now covers all 15 `PushKind::ALL` values exactly once,
  freezes five enabled target seams and ten disabled reason codes, validates
  before acquisition/sink, and defines executable static/runtime tests.
- Staged identities awaiting fresh independent review: design blob
  `1577ca552239340143ece07cfb03415e621dfea3`, plan blob
  `5a6e230a9b06b299eb66ca095b56234c10cfd845`, BR-192 row SHA-256
  `9dde6d41e24d265ab1f102ec103166ff1ab90d9493864f49251f32de15525c11`.
- BR-193 Gate B remains blocked by its own current-spec fresh Gate A review;
  existing scheduler tests 10/10 and cadence journal tests 3/3 pass, but only
  cadence receipt persistence exists.
## 2026-08-01 BR-194 minimal Gate B correction

- Removed out-of-contract `terminal_replay_classification_failed`; infra
  classification failure now maps to the frozen
  `terminal_replay_evidence_unavailable` reason.
- Release verifier now checks all historical completion reasons; compliance
  mutation logic locks the exact six-reason set; focused BR-194 tests,
  `cargo fmt --all -- --check`, monitor check and focused clippy passed in the
  implementer run.
- Fresh independent Gate B review is in progress. Full workspace Gate C/D,
  coverage and production replay evidence remain pending.
## 2026-08-01 continuation: independent review infrastructure

- BR-193 fresh Gate A review returned RED `C3/I4/M1`; production implementation
  remains blocked. A read-only revision-proposal agent is preparing alternatives
  that address every finding before any spec edit.
- BR-192 fourth independent review failed before sampling with backend HTTP 403.
  The same reviewer was retriggered against the unchanged staged identities; no
  Gate was bypassed.
- BR-194 first independent review also failed before sampling with backend HTTP
  403. A new independent reviewer instance was dispatched against the current
  five-file Gate B diff. Gate B remains unapproved until an independent report
  reaches `C0/I0`.
- Root independently rechecked the unchanged BR-192 staged identities:
  design `44fd3a8c...`, plan `fd3dd424...`, and exact BR row
  `9dde6d41...`; all match the review brief and `git diff --cached --check`
  passes. This is dispatcher evidence only and does not replace fresh review.
- BR-192 independent review failed a third time at the same backend HTTP 403,
  including a fresh alternate-model instance. Further identical retries are
  paused under the three-strike protocol; Gate A remains open.
- Root BR-194 pre-review validation is green: static checker PASS, monitor
  terminal replay tests `14/14`, library BR-194 tests `3/3`, and process
  isolation tests `3/3`. These results do not replace fresh Gate B review.
- Root also confirmed `cargo fmt --all -- --check`, `git diff --check`,
  `cargo check --bin monitor`, and `cargo clippy --bin monitor -- -D warnings`
  all exit 0 against the current worktree.

## 2026-08-01 BR-192 focused RED-to-GREEN repair

- Reproduced all three deterministic failures independently before editing.
- Updated the R-08 observable binding assertion to preserve the explicit missing
  broker placeholder and verify the following user-confirmed holding.
- Updated the R-04 static assertion to require only the source-only counted seam.
- Added an internal `CountedCombinedAccount` governance-context marker so generic
  counted calls fail closed while explicit counted bindings retain the same account
  governance context.
- The three exact regression tests now pass individually; full BR-192/BR-194
  regression and Gate C checks are running next.
- Full BR-192 monitor regression: 67 passed, 0 failed, 3 ignored.
- BR-194 regression: monitor 32/32, library 3/3, process isolation 3/3;
  independent Gate B review returned GREEN C0/I0/M0.
- `cargo fmt --all -- --check`, `git diff --check`, `cargo check --bin monitor`,
  and `cargo clippy --bin monitor -- -D warnings` pass after the repair.
- Full compliance is still RED only at §2.10: five BR-193 active-path citation
  errors and one unparseable BR-192 code-path entry. All other compliance
  checks in that run passed. Two independent read-only audits are resolving the
  registration root causes before any metadata edit.
- Independent audits confirmed the §2.10 roots. BR-193 is now truthfully marked
  `spec-only`; no future target source file received a false BR-193 citation.
- The business-rule parser now preserves literal pipes in intent text and reads
  the final code-path column. BR-192 worktree/index row hashes remain identical.
- `bash tools/compliance/check.sh` now exits 0 with all blocking checks PASS.

## 2026-08-01 full library regression

- Ran `cargo test --lib -- --test-threads=1` against the current worktree.
- Result: 2266 passed, 44 failed, 7 ignored (exit 101).
- The failures are being split by first-cause ownership: two global-schema
  fixture/sidecar failures; a selection audit namespace failure followed by
  process-mutex poison cascades; and independent event, chain, activation,
  outcome, persistence, and schema fixture/interface regressions.
- No assertion was weakened or ignored. Three independent read-only diagnosis
  tasks are reproducing the first failures before any Gate B repair.

## 2026-08-01 runtime command validation and R-04 diagnosis

- A later serialized full-workspace run is current GREEN: library 2313 passed
  with 7 ignored, monitor 523 passed with 4 ignored, and all other workspace
  targets exited zero.
- `cargo run --bin monitor -- --test` exits zero in an isolated TEST_CODE
  namespace and completes the v70 E2E marker.
- `cargo run --bin monitor -- --review` remains RED with exit two. The R-04
  `unix-ms` provenance parser defect is fixed test-first; the next run proves
  provider acquisition and canonical preparation succeed.
- Root-cause tracing now places the remaining R-04 failure before BR-194 L5:
  preparation accepts and preserves canonical `unix-ms:<epoch>`, while the
  durable binding revalidator accepts only RFC3339. The exact review audit
  reports `counted_source_only_binding_invalid`; the earlier DataMode hypothesis
  was rejected before any governance change. A shared strict parser and a
  prepare-to-durable regression are the active repair.
- R-09 remains an explicit current-date-only provider capability failure on a
  weekend effective review date. Account-bound tasks remain separately blocked
  by the missing verified broker batch plus same-batch trade-sync watermark.

## 2026-08-01 BR-199 R-08 public SourceOnly Gate B

- R-08 is now a real `SourceOnly` review task and executes in the public-source
  phase before account-gated outcomes.
- Production R-08 reads only CNInfo announcements, the mandatory CFFEX official
  delivery batch, Sina indices and Sina FX; account, local portfolio and virtual
  holding inputs are absent from its dispatcher, binding and renderer.
- The reminder date uses the next trading day rather than the next calendar day.
- A dedicated fixed-kind delivery entry validates exact canonical provider,
  batch, projection, transition, origin and rendered-text bindings again before
  Launch/L5, durable admission or sink access.
- Focused evidence is green: BR-199 monitor tests `10/10`, BR-194 monitor tests
  `32/32`, futures-delivery library tests `8/8`, monitor suite `538/538`, focused
  static compliance PASS, and `cargo clippy --bin monitor -- -D warnings` PASS.
- The first full-workspace run exposed one unrelated stale BR-174 frozen-hash
  fixture (`2315` passed, `1` failed, `7` ignored); its schema and nested
  evidence match the frozen design, so the deterministic golden is being
  corrected before Gate C is rerun. BR-199 Gate C/D and real review evidence
  remain pending and are not claimed complete.

## 2026-08-01 business-runtime closure evidence

- BR-200 adds a read-only durable terminal preflight keyed by the exact review
  occurrence. R-04 and R-09 now hydrate the original delivered snapshot without
  provider acquisition, durable admission, or another physical sink call.
- BR-160 A-10 delivery now uses a narrow `SourceBatchEvidence` gate that binds
  the business date, `chain-batch:` identifier and lower-case SHA-256 content
  hash. It does not synthesize or require account metrics, and its L7 payload
  retains the exact source-batch identity.
- A real `cargo run --bin monitor -- --review` run for effective review date
  2026-07-31 exited zero: R-04 delivered by terminal reuse, R-09 delivered by
  terminal reuse, and A-10 rebuilt 98 Magic TDX metadata records plus 1,899
  memberships and obtained a validated Feishu receipt. R-03 remained an
  explicit missing-broker-evidence failure and R-08 remained an explicit
  CFFEX-Unsupported failure.
- A subsequent `cargo run --bin monitor -- --test` exited zero in the isolated
  test database: 40/40 templates rendered and all three real Feishu validation
  batches returned validated receipts with zero skipped or failed batches.
- The process-isolation regression uses the real `MONITOR_AUTH_REQUIRED=0`
  switch instead of the nonexistent `MONITOR_OPERATOR_AUTH_REQUIRED`; its
  focused suite passes 8/8.
- These runtime results prove the business command paths, but Gate D remains
  open until the final merged worktree passes strict Clippy, full workspace
  tests, compliance and fresh 80%/95% coverage thresholds.

## 2026-08-01 BR-196 / BR-201 final closure work

- Historical note superseded on 2026-08-02: an earlier review accepted BR-196
  at C0/I0 and Gate B added a 64-family/58-kind projection, but fresh complete
  production-presentation review invalidated that acceptance. The registry
  still contains 50 descriptors and focused tests remain useful regression
  evidence; neither proves the complete current presentation inventory.
- BR-196 is not complete: 38 production presentation callers still bypass the
  token gateway; the exact six governance-smoke outcomes are not yet a closed
  prerequisite; and the current direct test transport still lacks the accepted
  opt-in, non-production target allowlist, one-shot permit and Command-spawn
  TOCTOU checks.
- BR-201's latest design revision closes the unsafe reconciler takeover by
  requiring an exclusive process-owner lock plus an explicit durable handoff or
  a persisted prior-owner-death proof. Live `Collecting`/`Sealed` attempts are
  not eligible for freezing based on age, PID or retry count. Exact Gate-D
  canary commands and current-code evidence are being finalized before a fresh
  independent Gate-A review.
- Cleanup audit confirms production source, config and Cargo metadata contain
  no RustDX or qmt-parser dependency; all 14 Magic crates use `=0.2.0` at one
  revision. Remaining cleanup is historical provider prose, unused
  `PUSH_VERBOSE` semantics, four dead configuration fields and README runtime
  claims. Threshold/config removals require a separate Rule 2.9/2.10 Gate A;
  historical incident/audit evidence must be preserved or explicitly marked
  superseded rather than silently deleted.
- The current release Feishu identity was resolved without spawning a process
  or network call and only its domain-separated hash was stored in BR-196's
  `production_deny` manifest. No distinct test conversation exists, so the
  non-production allowlist stays empty and bare live `--test` remains correctly
  fail-closed; it must not relabel the production destination to make Gate D
  green.
- The latest available coverage report is stale but quantifies the lower bound:
  global 149,200/189,647 (78.67%, 2,518 lines short) and core
  121,025/154,838 (78.16%, 26,072 lines short). A new report is mandatory after
  BR-196/BR-201 because their files change both coverage and the denominator.

## 2026-08-02 BR-202 second formal Gate A review

- Independent review returned `C0/I5/M1`; Gate B remains closed.
- The reviewer independently confirmed the repaired inventory facts: 36
  directory entries, 29 root files, 16 top-level bins, all 408 Rust sources
  uniquely classified, 12 non-self historical-plan grep matches, and one
  byte-identical BR-202 rule row in the design and business-rule registry.
- Six design defects remain: `.github/workflows/coverage.yml` and README still
  invoke the raw gate; the isolated wrapper lacks trap/export/cleanup ordering;
  the five capability categories are not a complete machine-enforced behavior
  registry; attestation checks Git shape but does not recompute the JSON claims;
  production stores, credentials and environment variables are not frozen into
  closed deny/allow sets; and the strict integer parser lacks an exact maximum.
- Current implementation/compliance evidence is still RED: the BR-202 test file
  lacks its rule citation, the declared isolated wrapper does not exist, no
  current-date Gate-D push/event evidence exists, and no fresh full coverage run
  has been produced.

## 2026-08-02 — Gate baseline and focused failure isolation

- `cargo fmt --all --check`, `cargo check --locked --bin monitor`, and
  `cargo test --locked --bin monitor --no-run` pass.
- Full monitor tests are RED: 568 total, 562 passed, 2 failed, 4 ignored.
  The failures are the BR-196 allowlist release hash assertion and the R-08
  counted-binding dispatcher contract assertion.
- Both failures reproduce as one-test focused runs with the full module path.
  A prior abbreviated `--exact` command selected zero tests and is not counted
  as evidence.
- Rule 2.10 currently passes, while the dedicated BR-194 checker fails because
  it still searches for the removed public
  `push_r08_source_only_with_binding` marker.
- `docs/business_rules.md` plus the three BR-196/201/202 designs now form a
  whitespace-clean staged Gate-A candidate. Independent reviewers remain
  unavailable because fresh agent starts return HTTP 403; no Gate-A acceptance
  is claimed.

## 2026-08-02 — BR-202 seventh formal Gate-A review

- Formal verdict: `REJECT C3/I2/M0`; Gate B remains closed.
- HEAD inventory is `391 Rust files / 35 directories / 26 roots`, while the
  design's `408/36/29` figures came from the dirty worktree and chain through
  unfinished BR-196/BR-201. CLAUDE's no-spec-on-unverified-gate rule blocks
  BR-202 until prior gates close.
- The frozen extractor has no successful path on stable Rust 1.95 because both
  `-Zunpretty=expanded` and rustdoc JSON need unstable support while the design
  forbids bootstrap/a second compiler.
- The critical `target/coverage` lifecycle JSONL lacks cross-process locking,
  full-chain/tail validation, hash chaining and five-year retention.
- The actual combined index cannot satisfy the design's BR-202-only two-blob
  premise. The PR template also omits a spec section and red lines 2.2/2.8/2.9.
- No fresh Gate-D behavior evidence or same-covered-binary Disabled banner
  exists; no release-readiness claim was made.

## 2026-08-02 BR-201 third formal Gate A review

- Independent review returned `C0/I9/M0`; Gate B remains closed.
- The reviewer reproduced all 23 current-state evidence commands and all four
  frozen hashes, and confirmed the 26-field first-record/hash-chain contract,
  double quote-freshness boundary, and unique BR-134/BR-201 rows.
- Nine Important defects remain: the audit type cannot be chosen before reading
  debounce; `manual_confirmation_required` is absent from the closed reason
  set; basis-point rounding/residue rules are missing; BR-086 requires both
  preconfirmed and atomic audit identities; JSONL `Claimed` recovery has no
  exclusive owner/generation/death-proof contract; exact `old+1` generation
  wedges after consecutive recovery crashes; joined-order fact kinds and row
  cardinality are open; rollback incorrectly treats a local rebuild as release
  authority and creates no new signed attestation; and no admitted real adapter
  can emit `Br134AccountEvaluationBatchV1`.
- There is no current-date BR-201 push/event evidence, implementation symbol or
  disabled banner. This truthfully proves Gate B has not begun, not readiness.

## 2026-08-02 BR-196 fresh formal Gate A review

- Formal verdict is `C0/I4/M0`; Gate B remains closed.
- The 50 declared descriptors independently reproduce as 26 Active and 24
  Disabled, but a third registry-external Active presentation exists: forced
  replay assembles in `src/event/replay.rs`, routes through `main.rs`, and sends
  through raw `notify::push_wechat`. Therefore the proposed 66-shape inventory
  is incomplete and any target matrices must include or explicitly govern it.
- The audit table stops several Active chains before their actual gateway and
  uses a non-executable `<symbol>` command without pasted output or a source
  revision/tree binding. It is not reproducible evidence under CLAUDE.md.
- Current public visibility/removal/replacement changes in notification and
  wrapper APIs contradict the design's no-API-change claim and require an
  explicit compatibility/migration/rollback disposition.
- The canonical BR-196 row includes literal `<Disabled|SpecOnly>`; the pipe
  splits a GFM table cell. Its Code/old-module paths also omit replay and the
  public API migration.

## 2026-08-02 BR-202 fresh formal Gate A review

- Formal verdict is `C0/I7/M0`; Gate B remains closed.
- Inventory and math remain valid planning facts: 408 Rust files, 36 directory
  classes, 29 roots, 16 bins, Core 398/GlobalOnly 10, global deficit 2,518 and
  provisional core deficit 30,654. They are not fresh Gate D evidence.
- Raw llvm-cov can remain diagnostic; an external binary cannot be forced to
  exit 2 by the wrapper. The enforceable boundary is that only the wrapper can
  mint verified release authority/PASS, reconciled with the engineering rules.
- Failure evidence needs a durable diagnostic root pinned before any temporary
  worktree/export. File membership is not behavior completeness; authoritative
  business registries and audited residual identities must generate clusters.
- Report hashes/counters are insufficient without covered binary/object,
  profraw/profdata, mapping/build-manifest and toolchain/source bindings.
- A new empty CARGO_HOME plus offline mode cannot build Git-pinned Magic crates;
  a verified vendor or read-only dependency snapshot is required.
- BR-202 omitted its entrypoint test path, and the standalone JSON to verified
  bundle upload is a real artifact/caller migration that must be versioned.

## 2026-08-02 BR-201 fresh formal Gate A review

- Formal verdict is `C1/I7/M0`; Gate B remains closed.
- Critical: current four-rule orders use legacy `plan_id` directly for the
  reservation, while the design proposed a domain-separated V1 intent hash.
  Without dual lookup/uniqueness and a cutover fence, the same business intent
  can cross the deployment boundary under two identities and evade the 60-second
  idempotency rule.
- Account decimal/fen/bps validation failure paths lack one-to-one closed reason
  codes. Signed rollback bytes do not yet prove that every named deep change was
  exactly reversed without piggyback edits.
- BR-134 still requires eager `PaperRiskContext` and attempting all exits while
  BR-201 requires lazy real account capture and stopping side effects after a
  permit expires. A scoped supersession is required rather than competing MUSTs.
- Persistent debounce lacks a first-deployment genesis/null/seed contract, and
  public `run_once(PaperRiskContext)` lacks a versioned permit entry plus a
  source-compatible fail-closed shim/caller migration.
- Rule 2.10 omits order safety, risk adapter, order audit, concrete storage/
  migration and rollback-verifier paths. Real provider/banner/current production
  evidence remains absent and cannot count as green.
- Previously repaired scheduler ordering, 27-field hashes, integer allocation,
  BR-086 atomicity, recovery generations and joined-fact contracts independently
  passed and must not regress.

## 2026-08-02 BR-201 latest realizability review

- Fresh independent verdict is `REJECT C2/I3/M0`; Gate B remains closed.
- The authoritative open-attempt schema cannot represent the required engine
  phase, reconciliation ownership/generation, snapshot and pending-terminal
  recovery state. The delivery schema likewise cannot represent attempt/claim
  identities or pending receipts for the required
  `Sending -> AckPending -> DeliveredAcked` transition.
- The claimed exhaustive identifier inventory omits
  `PaperExitSessionAudit`, `PaperExitAttemptReconciler` and
  `PaperExitEventOutbox`; its extractor cannot discover those suffixes.
- The external root-owned rollback bootstrap has no tracked source/package/
  installation path in the proposed Gate-B slice, and the design asks it to
  write a private-bin type it cannot own reproducibly.
- The design defines AC-01 through AC-14 but its exact marker check requires
  13 markers. This makes the current acceptance predicate self-contradictory.
- Independent checks still reproduced the staged canonical registry text and
  frozen hashes and confirmed no current implementation/evidence exists; these
  are planning integrity facts, not Gate-A acceptance.

## 2026-08-02 corrected closure order

- Repository rule `CLAUDE.md` section 2 forbids spec-on-unverified-gate
  chaining. BR-192 is the earliest unclosed batch, so BR-196, BR-201 and
  BR-202 cannot enter implementation before the preceding batch reaches Gate C.
- The active order is now BR-192 Gate A/B/C, BR-196 Gate A/B/C, BR-201
  Gate A/B/C, then BR-202 Gate A/B/C/D and live `--review`/`--test` evidence.
- BR-192's enabled counted-producer catalog is being independently re-audited
  against `HEAD`; dirty-worktree seams from later batches cannot be used as its
  Gate-A authority.

## 2026-08-02 BR-192 corrective metadata formal review

- Formal verdict is `REJECT C0/I2/M0`; Gate A remains open.
- Fixed `HEAD=b4aeee68d2c0259cc968914b3d39e3a89a18a496` has no admitted
  production counted producer for the five rows currently marked enabled:
  T0/Paper still use the generic governor/dispatcher, R-04/R-08 use generic
  `dispatch_outcome`, and no production R-09 dispatcher exists. Worktree-only
  later-gate seams are not valid current-code evidence.
- The second defect is independently blocking: fixed HEAD already has durable
  schema v5 and the accepted BR-194 v4-to-v5 migration/checker baseline, while
  BR-192 still proposes another shared v4-to-v5 migration. Existing v5
  databases could never acquire the new BR-192 objects under that plan.
- Repair is limited to the tracked BR-192 design/plan/row: rebuild the 15-kind
  catalog from HEAD, use 15 `DisabledNoProducer` rows for the first release,
  and redesign the additive migration as v5-to-v6 with fresh/v1..v5
  convergence and BR-194 v5 preservation. A new independent C0/I0 is required.

## 2026-08-02 BR-192 C0/I2 repair candidate

- Repaired the staged design/plan/BR-192 row against fixed HEAD: catalog is
  exactly 15 rows, all `DisabledNoProducer`, and records zero accepted current
  producers; rejected worktree seam output is explicitly non-authoritative.
- Rebased every identified schema/migration/test/checker/rollback contract on
  the real v5 baseline and one additive v5-to-v6 migration. Fresh/v1..v5 paths
  converge on v6 while preserving the accepted BR-194 v5 objects/rows/triggers.
- Current staged identities are design
  `20a87d2f88e71c7f4fa5705293dd4a62435085cf`, plan
  `d70bfb95ab34c7c9bde72e6457e49bf8cacac9cc`, and BR-192 row
  SHA-256 `9d763b136ece9d38901bdf4c1831c763532ed11c08bf8c0d96a5212b288a7212`.
- `git diff --check`, cached diff check and Rule-2.10 pass; the checker reports
  131 repository-wide historical warnings and no BR-192 hard error.
- Root review found and repaired a final Task-9 contradiction: the first
  catalog cannot simultaneously freeze all 15 kinds disabled and require an
  active `ReviewProviderTopN` producer. The cross-version check now requires
  that kind to remain disabled until a later producer-specific rule reaches
  Gate C and a fresh Gate-A C0/I0 review authorizes the catalog change.
- Three fresh independent reviewer starts failed with external HTTP 403 and
  receive zero acceptance credit. Gate A remains open until an actual C0/I0
  verdict arrives.

## 2026-08-02 BR-192 C1/I6/M1 + C0/I3/M0 repair

- Two exact independent reviews completed after the earlier reviewer-service
  failures and rejected the staged all-disabled identities. Their previous
  design/plan/row hashes are now obsolete and receive no Gate progression.
- Reworked the tracked Gate-A triple to one real R-09 producer consumer, 14
  disabled kinds, immutable freshness expiry/terminal audit, full permit/caller
  enforcement, exact production evidence authorities, preserved fixed-HEAD
  migration-test identity, coherent all-eight Task-1 tests and correct file
  create/modify ownership.
- No production code was changed. Next: run scoped whitespace/Rule-2.10 checks,
  stage only the repaired authority triple, compute fresh identities, and send
  those exact objects to parallel independent Gate-A reviewers. Gate B remains
  closed until C0/I0.

## 2026-08-02 BR-192 C0/I4/M0 + C1/I3/M1 repair

- Kept Gate B closed after both independent RED verdicts.
- Repaired the tracked design/plan/BR-192 row for the concrete permit API,
  exact persisted producer provenance, active expiry drain, pre-start Reserved
  terminalization, genuine RED tests, mandatory first Gate-B file action,
  exhaustive fixed-HEAD caller inventory and unified five-crate dependency
  pin.
- Corrected the deferred Task-8 file action and final integration staging list
  to include `Cargo.lock` and the counted-producer catalog module.
- No production Rust path was changed in this corrective Gate-A step. Next is
  scoped validation, exact staging/hash capture and fresh parallel C0/I0
  review.

## 2026-08-02 BR-192 C3/I2/M0 preliminary repair

- Kept Gate B closed and repaired only the tracked design/plan/BR-192 rule.
- Closed manual-before-authorization expiry persistence, replaced caller/PAM
  freshness with a private production clock, and specified the exact
  expiry/start/ownership SQLite total order.
- Added the final pre-external-call freshness gate and its zero-sink terminal
  authority so a claim obtained before midnight cannot send stale data after
  midnight.
- Removed denial-opacity and rollback-catalog contradictions; preserved the
  exact enabled R-09 catalog identity during rollback.
- `git diff --check`: PASS.
- `bash tools/compliance/lib/check_business_rules.sh`: PASS with 131 historical
  warnings and zero hard errors.
- Next: stage only the authority triple, capture fresh object identities, and
  run two fresh independent exact-object reviews. No production code changed.

## 2026-08-02 BR-192 final-pre-call terminal repair

- Two independent exact-object reviews returned C1/I7/M0 and C1/I3/M1; Gate B
  stayed closed.
- Repaired only the tracked design/plan/BR-192 row. The contract now uses a
  two-transaction no-call expiry terminal, nine exact ordering triggers,
  result-absence rechecks, exact `Confirmed(n)` recount, and a derived
  `ExpiredFreshnessBeforeSink` terminal compatible with the fixed v5 attempt
  table.
- Unified R-09 bad-data/verified-empty handling as typed `Failed`, preserved
  fixed-HEAD R09 enum/order/label/SourceOnly identity, reclassified cfg(test)
  callers, made the coordinator the sole production freshness-clock owner,
  unified expiry outcomes, completed the public constant manifest, and moved
  private catalog creation to Task 1.
- Scoped/staged whitespace and Rule-2.10 checks pass; Rule-2.10 reports 131
  historical warnings and zero hard errors.
- Exact staged identities: design
  `6ba1d63f569f5c4c79afb2b7fbbcbceb61b1b592`, plan
  `ce18d4e5657b1f58a4e5716d9f7d5272d45834f9`, BR index blob
  `fa282bac9c2282877e0291bd8218681ce809af59`, BR-192 row SHA-256
  `9a6e8ee81fb16b570b55891668b6545ce1ba59be096e7b37df915c38aec4b6d0`.
- Two fresh independent read-only reviews are active; no production Rust was
  changed in this Gate-A repair.

## 2026-08-02 BR-192 cross-rule/state/executable repair

- Three independent read-only prechecks rejected the prior candidate with
  `C2/I5/M0`, `C0/I1/M0`, and `C0/I4/M1`; those earlier staged identities no
  longer carry Gate-A authority.
- The docs-only repair removes `BannerCtx` and all account/broker coupling from
  R-09, reconciles BR-198 to 14 direct Magic dependencies and 15 lockfile
  packages at one immutable revision, and freezes the exact BR-200 durable
  occurrence outcome/retryability map plus ordered rule IDs.
- Retry expiry now applies only to retry discovery/admission/authorization;
  initial settled-date acquisition remains separately admissible. Dedicated
  capture-before-request, cross-Shanghai-midnight and invalid provider-time
  cases are included.
- The terminal-result relation now specifies a unique deferred foreign key,
  write-once/immutable ownership pointer, exact authoritative reverse join and
  race/mutation tests. The trigger catalog increases from nine to twelve.
- BR-192 Gate D now delegates release authority only to the BR-202 isolated
  wrapper. Gate B remains closed pending command/declaration audit, scoped
  validation and two fresh independent C0/I0 reviews.
- Removed a newly detected gate-order deadlock: BR-192 Gate-B paths must cite
  and register under BR-202 now, but BR-202 Gate A/B/C implementation starts
  only after BR-192 Gate C. Final BR-192 Gate D still waits for the BR-202
  isolated wrapper.
- Mechanical exact-test audit passes after the capability/rollback additions:
  BR-192 has 245 unique `cargo test ... --exact` targets, all with unique plan
  declarations; BR-200 has 25 such targets, all with unique declarations.
  BR-192's four declaration-shaped non-command names are three parent-invoked
  ignored child helpers and one quoted fixed-HEAD BR-194 evidence snippet.
- Scoped `git diff --check` passes. Rule 2.10 exits zero with 198 rules and 131
  historical warnings. The exact BR-198 dependency test executes one test and
  passes, proving the 14-direct/15-lock Magic closure at the pinned revision.

## 2026-08-02 BR-192 three-way precheck RED repair

- Fresh read-only prechecks returned state `C1/I0/M0`, cross-rule
  `C1/I1/M0`, and executable `C1/I2/M0`; BR-192 Gate B remains closed.
- The state blocker is a stale result-first positive recipe that contradicts
  the reverse trigger. The only allowed positive order is ownership pointer
  update, failpoint, authoritative result insert, bijection validation, commit;
  result-first remains a negative immediate-rejection test.
- BR-198 rollback incorrectly named revision `660902...`, which cannot compile
  the retained enabled R09 API. The repair keeps the full 14-direct/15-lock
  `5f1ce936...` dependency identity and rolls back only closed-day behavior.
- BR-192 now requires only literal future `BR-202` citations in Gate-B source.
  It neither mutates nor claims the current BR-202 Code cell; BR-202 Gate A and
  all later progression wait until BR-192 Gate C.
- The fixed-HEAD-absent `tests/magic_market_release_revision.rs` is now an
  explicit Task-8 Create path and is included in the atomic commit recipe.
- Separate executable BR-198 and BR-200 plans are being authored in parallel;
  neither untracked candidate has Gate-A or Gate-C acceptance credit.

## 2026-08-02 BR-192/BR-198/BR-200 dependency-cycle repair

- A fresh BR-198 review returned `C2/I8/M1`. The decisive blocker was physical:
  BR-198 required R-09 gateway/producer/preflight objects that only BR-192 Task
  8 creates, while BR-192 simultaneously required BR-198 Gate C first.
- BR-198 is now a supporting contract incorporated into BR-192 Task 8/Gate B.
  It owns no independent Gate B/C, commit or prerequisite. Its exact Shanghai
  date, 15:35, trusted request-start/capture/completion, raw timestamp and
  14-direct/15-lock requirements remain mandatory inside BR-192.
- BR-200 is narrowed to a generic typed read-only durable occurrence API plus
  real R-04/R-08 production consumers. R-09 remains disabled throughout its
  independent progression; BR-192 later consumes the accepted API.
- BR-192 now requires only BR-200 accepted Gate C before Gate B. Its new tests
  also prove host `TZ` cannot change Shanghai semantics and reject same-date
  provider capture before trusted request start or after completion.
- Scoped whitespace and Rule-2.10 checks pass; fresh independent exact-object
  reviews are running. No production code was accepted or modified by this
  Gate-A repair.

## 2026-08-02 BR-200 checker investigation

- Opened three authorized parallel lanes: BR-198 forward rollback repair,
  BR-192 raw-capture/rollback re-review, and BR-200/BR-194 checker fact check.
- Locally inspected the full shared BR-194/199/200 checker and current task/
  push-kind references. Confirmed that retaining R-09 enum, SourceOnly mapping,
  dispatcher membership and date-policy tests is compatible with BR-200
  keeping the actual R-09 provider disabled. No checker weakening is needed.
- Recorded the fact in `findings.md`; Gate A remains open and no production
  Rust path was edited.

## 2026-08-02 BR-192/198/200 final Gate-A normalization

- Independent prechecks converged on one provider-free BR-200 prerequisite:
  R-04/R-08 are `EnabledSourceOnly`, R-09 preserves durable identity and
  SourceOnly classification but remains typed `DisabledNoProducer` with zero
  provider/renderer/new-decision/sink calls.
- BR-198 now uses a checked-in forward patch that disables only BR-192 periodic
  retry discovery; it never reverts the atomic R-09/BR-200/schema/catalog work.
- BR-192 now owns a normal Gate-B/Gate-C forward-rollback verifier rather than
  relying on an incident-time `git apply --check` recipe.
- Scoped whitespace validation passes. The first Rule-2.10 run correctly
  rejected future artifacts listed as if they already existed; after limiting
  the BR-200 Code cell to current authority paths, the checker passes with 198
  rules and zero hard errors (historical warnings remain non-blocking here).
- Gate A remains open for exact command/declaration audit and fresh independent
  C0/I0 reviews; no production Rust path was edited in this normalization.

## 2026-08-02 Formal-review repair round

- BR-198 review found one field-name split; BR-192 now uses the BR-198-owned
  `capture_completed_at` name everywhere and no longer mentions
  `request_completed_at`.
- BR-192 review proved the initial rollback verifier call targeted pre-Task-8
  HEAD. Task 8 now freezes the fully staged tree as a `commit-tree` candidate,
  verifies that exact object, requires the real commit tree to match, and reruns
  the verifier on committed HEAD.
- The same verifier contract now includes all 12 BR-198 tests, seven BR-200 R09
  tests plus claim, schema/catalog/revision, retry recovery and startup-cycle
  tests after applying the one-file forward patch.
- BR-200 review found R-08 is still `LegacyAccountGate` at fixed HEAD and cannot
  be silently promoted. The corrective docs-only lane narrows this independent
  slice to live R-04; R-08/R-09 retain identity but fail closed until their own
  BR-199/BR-192 atomic releases.

## 2026-08-02 BR-200 R-04-only Gate-A repair completed

- The BR-200 design and plan now expose only R-04 as a live production
  consumer. R-08 is `DisabledNoProducer(Br199NotReleased)` and R-09 is
  `DisabledNoProducer(NoProducer)` before partition, with exact reason codes and
  zero provider/renderer/new-decision/sink work.
- The capability API is a closed nine-arm typed `Result`; `Option`, wildcard
  matching and identity-as-permission are forbidden. The shared BR-194 checker
  contract is additive against its fixed-HEAD SHA and rollback disables only
  R-04.
- Mechanical audit reports 25 exact commands matching 25 unique declarations,
  no stale live-R08 wording, no whitespace errors, and Rule 2.10 passes with 198
  rules plus 132 historical warnings. Fresh independent reviews remain open.

## 2026-08-02 Formal review status after R-04-only repair

- BR-198 passed independent review at C0/I0/M1. BR-192 passed two independent
  reviews at C0/I0; its remaining findings are staging/readability-only.
- BR-200 remained RED at C0/I5. Gate B stays closed while a bounded docs-only
  repair fixes candidate-tree rollback verification, byte-prefix checker
  preservation, a true command/declaration bijection, R-08/BR-199 authority
  separation and task-specific rule vectors.
- The repair now treats R-08 as typed Unsupported inside BR-200 rather than
  inventing a BR-199 enable transition. Only R-04 is enabled; R-09 remains
  typed disabled for the later BR-192 atomic slice.

## 2026-08-02 BR-200 second-review repair

- Two independent reviews remained RED at C0/I2 and C0/I5. The common
  blockers were a permanently baseline-only R-08 checker, a caller-supplied
  checker append digest, source-text-only test cardinality, and missing
  production/debt evidence. The R-09 startup banner and Gate-D evidence were
  repaired first.
- The current Gate-A documents now define two closed checker profiles:
  BR-200 baseline R-08 Unsupported and a complete separately accepted BR-199
  SourceOnly profile. Partial or mixed states fail and BR-200 still owns no
  R-08 transition.
- The forward verifier must own literal prefix/append digests and execute both
  mutation matrices before and after rollback patch application. The canonical
  25-test gate now compares source declarations, exact commands and Cargo
  registration, then requires each command to run exactly one test.
- Gate B remains closed pending scoped validation and fresh exact-object
  independent C0/I0 review. No production Rust path was edited in this repair.

## 2026-08-02 BR-200 final-hash review remained RED

- Exact staged hashes matched, but the Standards and realizability reviews
  returned C0/I8 and C0/I5. Gate B remains closed.
- Blocking themes are now bounded: stable Gate-A status/row-scoped registry
  authority, an accepted BR-194 Gate-C execution base, an owned exact-once R-09
  startup installation, ignored-test rejection, candidate-tree verifier bytes,
  fail-closed shell blocks, causal real-review evidence, accepted baseline
  coverage commands, and a variable-length validated rule vector.
- The next repair is docs/tests-command only. It will not absorb unaccepted
  BR-194 production changes into BR-200 and will not use BR-202 as authority.

## 2026-08-02 BR-200 final Gate-A repair in progress

- Three parallel read-only reviewers supplied realizability, shell/test, and
  causal Gate-D repair drafts. No production Rust path was changed.
- Design/plan now make BR-194 Gate C a literal accepted base prerequisite, own
  the exact-once R-09 startup call/test, use 26 exact tests, validate variable
  ordered rules, execute verifiers from committed trees, use fail-closed
  pipelines, and use repository baseline coverage rather than BR-202.
- The remaining Gate-A work is the canonical BR-200 row rewrite/hash, scoped
  validation, staging of exact authority objects, and fresh C0/I0 reviews.
- A concurrent read-only upstream probe ran the current
  `check_br194_review_dependency.sh`; it failed immediately because the checker
  still requires the removed `push_r08_source_only_with_binding` marker. BR-194
  Gate C remains RED and BR-200 Gate B remains correctly blocked.
# 2026-08-02 BR-200 prerequisite repair

- Repaired the BR-194/BR-199 compliance checker to match the production R-08
  public-presented/private-source-only call chain.
- `bash tools/compliance/lib/check_br194_review_dependency.sh`: PASS.
- Parallel realizability review found three BR-200 Gate-A blockers: Task-4
  staged-path order mismatch, missing implementation/commit lifecycle for the
  repeated-review verifier, and an impossible fixed-HEAD checker-prefix
  prerequisite. Gate A is still in progress; no BR-200 production Rust change
  has started.

## 2026-08-02 BR-194 master-baseline recovery

- Confirmed context recovery from the persisted plan and retained BR-200
  blocked behind literal BR-194 Gate C.
- The clean master baseline `9307b67` is not Gate-C reproducible: one event
  audit schema constant is referenced but absent, and the BR-194 checker
  expects an R-04 preparation seam missing from the committed source.
- The focused BR-194 test did not reach Rust assertions. Cargo failed with
  `No space left on device` while compiling dependencies in the isolated
  candidate target.
- Disk inspection found 6.9/7.1 GiB targets in two abandoned BR-194 worktrees.
  `cargo clean` removed 7.0 GiB from each; the active 21 GiB project target,
  source files and databases were preserved.
- Two independent read-only audits are running in parallel: R-04 SourceOnly
  implementation versus checker/spec, and the counted-delivery audit schema
  constant versus BR-160/BR-192 authority.
- Root tracing confirmed the frozen BR-194 spec marks the missing R-04
  SourceOnly/prepare/context work as Gate B, while commit `9307b67` changed the
  checker but did not include the required production entry files. The later
  `b4aeee6` tree has those symbols, but its broad diff is being decomposed by
  ownership before any code is transplanted.
## 2026-08-03 continuation

- Re-read the mandatory systematic-debugging and persistent-planning skills.
- Polled the previously running isolated candidate build; it completed with
  exit 101 on the missing counted-delivery schema constant, matching the clean
  baseline and BR-192 audit.
- Received the independent R-04 audit: `9307b67` has an incomplete commit
  boundary, not a stale checker. No source file was modified by the auditor.
- Next action: inspect the pre-merge Git snapshot read-only and recover only the
  pure BR-192/BR-194 slice before any production edit.
- Inspected the pre-merge stash object read-only. It contains the exact missing
  counted-delivery and R-04 production symbols and does not contain BR-199 or
  BR-200 in the scoped production/test paths. This is now the preferred
  recovery reference; no stash or worktree content has been applied yet.
- Scoped the tracked recovery file set. The gateway/exact-byte append modules
  are absent from the stash working tree because they were untracked; next
  inspection targets the stash untracked-files parent.
- Verified the third parent holds the exact missing gateway, immutable-append
  and BR-192 test files. No files have been restored yet; the next step is a
  disposable reconstruction/compile to prove that this snapshot is coherent.
- Created a disposable local clone and restored the complete pre-merge stash
  on its exact parent without conflicts. The reconstructed tree is ready for a
  shared-target `cargo check --lib`; no production path or database was used.
- `env CARGO_TARGET_DIR=<main-target> cargo check --lib` on the reconstructed
  snapshot completed successfully. Next: exact BR-192 tests and BR-194 checker.
- First exact BR-192 envelope test passed after a 1m36s shared-target build.
- Parallel follow-up auditor returned an unrelated context-status response
  instead of the requested extraction manifest; no evidence from that response
  is being used. Root continues from the authoritative Git snapshot.
- Exact push-record test passed. Exact persistence test exited 101 before its
  assertions because the disposable clone lived below world-writable
  `/private/tmp`; logged as an environment setup failure, not retried there.
- Reconstructed the snapshot under `target/br194-stash-audit-20260803` and
  reran the persistence test; it passed. All three exact BR-192 tests are now
  green on the recovered snapshot.
- Frozen BR-194 static checker passed on the recovered snapshot. Counted 33
  in-crate BR-194 tests and three process-isolation tests for the next focused
  validation stage.
- Focused monitor BR-194 suite passed 31/31. Next focused gate is the three
  `monitor_help_isolation` process tests.
- Process-isolation BR-194 suite passed 3/3. Root cause and source snapshot are
  proven; moving to minimal extraction on a clean `9307b67` base.
- Began dependency-boundary audit. R-04 and R-09 both depend on earlier unified
  gateway/counted-delivery slices that were likewise left uncommitted. Whole
  `review.rs` or whole 213-file stash import is rejected as an extraction plan.
- Re-read the BR-192/BR-194 frozen design headings and BR-162/194 rule rows.
  Gate A is still formally open; recovery will be split into predecessor
  authority/gateway slice and BR-194 orchestration slice before implementation.
- Measured diff scope and rechecked the authoritative worktree. Main remains a
  broad dirty branch and is preserved. Parallel audit now computes the exact
  R-04/R-09 gateway predecessor closure in the reconstructed clone.
- Added Gate A draft
  `docs/superpowers/specs/2026-08-03-br192-br194-incomplete-commit-recovery-design.md`.
  It freezes immutable recovery objects, splits source prerequisites / counted
  audit / BR-194 orchestration, forbids whole-stash import and later BR-160/
  197-200 content, and lists focused/full/runtime gates plus reverse rollback.
- Requested a fresh independent C/I/M review; Gate B remains prohibited.
- Rule-2.10 checker passed with existing warnings. Audited P1 imports and found
  the acquisition-audit database dependency plus the stale snapshot Magic
  revision; both are now explicit Gate A closure items.
- Updated the recovery design to pin authoritative Magic revision `5f1ce936...`
  and require BR-159 acquisition-audit persistence while rejecting accidental
  coupling to A-01/R-03/THS/historical bars.
- Applied the deep-module vocabulary to the shared Gateway seam: keep provider
  adapters private, make evidence/audit an internal deep module, and retain
  `CapitalDataGateway::provider_top_n_pair` as the R-09 interface. No code edit
  is authorized yet.
- Added an exact P1/P2/P3 module-and-hunk path matrix to the Gate A design. P1
  now defines `data_gateway/admission.rs` as an internal deep module and locks
  database/Cargo/lib edits to the acquisition-audit/provider closure.
- Integrated the parallel path-closure audit: replaced broad `capital.rs` with
  narrow `provider_top_n.rs`, excluded `magic-exchange-rs`, added
  `rusqlite/functions`, and fixed the generic-counted → SourceOnly transition to
  occur only in P3.
## 2026-08-03 Gate A static validation refresh

- `git diff --check -- docs/superpowers/specs/2026-08-03-br192-br194-incomplete-commit-recovery-design.md`: PASS.
- `bash tools/compliance/lib/check_business_rules.sh`: PASS (198 rules; 134 pre-existing citation warnings). The warnings remain separate follow-up debt and are not treated as a clean full Gate C result.
- Re-read the complete latest recovery design after the P1 path-closure refinement. Its status remains `Gate A draft`; no Gate B source edit has started.
- Scoped status confirms the recovery design and planning ledgers are the only new/modified planning artifacts in this stream; the broad user worktree remains untouched by recovery code.
## 2026-08-03 Gate A independent review blockers

- Independent review reported at least two Critical design defects; Gate B remains prohibited.
- C1: §2 says no database migration while P1 restores the BR-159 append-only acquisition-audit table/triggers; the design must explicitly classify, validate, and roll back this additive schema installation without deleting audit evidence.
- C2: simple P3 → P2 → P1 `git revert` is unsafe for counted delivery. Rollback must first freeze new reservations, reconcile pending attempts, manually dispose `Uncertain`, prove zero active/pending authority, and only then revert/disable producers and schema-v3 authority.
- A broad rollback search confirmed the durable layer has explicit all-date reconciliation, manual uncertainty, pending-inspection and active reservation states. The first narrowed spec search failed before reading files because zsh expanded a nonexistent glob; it produced no evidence and will be retried with literal paths.
- Literal-path review of the frozen BR-192 plan confirms rollback is forward-compatible, not a reverse commit sequence: preserve schema recognition, tables/triggers/indexes/outboxes/immutable records, never lower `user_version`, never launch an older binary, and let pending bytes continue only through exact idempotent reconciliation.
## 2026-08-03 Gate A C2/I5 remediation evidence

- Current authoritative `docs/business_rules.md` BR-192 row, `Cargo.toml`, `Cargo.lock`, the revision test, and newer BR-192 specs all already freeze Magic revision `5f1ce936...`; the review's revision contradiction came from the historical stash design at `d7dfa314...`, which this recovery explicitly supersedes.
- Located the checked-in positive join verifier at `tools/release/verify_br194_review_join.py`; Gate D will invoke it rather than relying on ordinary process exit.
- Confirmed hard evidence requirements: code-fact claims in the recovery design must include exact commands plus pasted output; full Gate C tests must be serial, and Gate D additionally requires llvm-cov threshold checks and a release monitor build.
## 2026-08-03 reproducible recovery evidence rerun

- Safe clone: `/Users/zhangzhen/Desktop/Quant/stock_analysis/target/br194-stash-audit-20260803`.
- `env CARGO_TARGET_DIR=/Users/zhangzhen/Desktop/Quant/stock_analysis/target cargo check --lib`: PASS; exact terminal output ended with `Finished dev profile [unoptimized + debuginfo] target(s) in 1m 00s`.
- Re-ran the three exact counted schema-v3 tests: each reported `1 passed; 0 failed`; envelope/push-record had 2316 filtered, persistence completed in 0.11s.
- Frozen BR-194 checker: `BR-194 review dependency static contract: PASS`.
- Monitor BR-194 filter: exactly `31 passed; 0 failed; 495 filtered out`.
- Process-isolation BR-194 filter: exactly `3 passed; 0 failed; 21 filtered out`.
## 2026-08-03 frozen Gate-D command recovery

- Recovered the exact frozen Gate-D sequence: normal authentic R-09/R-04 delivery and same-cycle hydration, then `--br194-audited-terminal-replay` once per task, then `tools/release/verify_br194_review_join.py --require-passed-replay 1` once per task.
- The replay success line must prove attempts=1, provider/resume/sink/delivery-audit-appends=0, and equal sink/delivery-audit watermarks. Ordinary `--review` output is insufficient.
## 2026-08-03 Gate-A remediation validation

- Latest recovery design remains clean under `git diff --check`.
- Exact immutable append filter rerun: `9 passed; 0 failed; 0 ignored; 2308 filtered out`; the nine parent tests exercise isolated child cases where required.
## 2026-08-03 P1 reproducible baseline

- Reconstructed BR-159 acquisition-audit filter: exactly `4 passed; 0 failed; 2313 filtered out`.
- Reconstructed BR-162 DragonTiger filter: exactly `6 passed; 0 failed; 2311 filtered out`.
- Historical capital-facade Provider Top-N filter: exactly `3 passed; 0 failed; 2314 filtered out`; P1 must migrate these three semantics into the narrow `provider_top_n` module and add three shared-admission failure tests.
## 2026-08-03 post-remediation static checks

- Scoped `git diff --check` across the recovery design and planning ledgers: PASS.
- Rule 2.10 checker after remediation: PASS, 198 rules and the same 134 historical warnings; no new blocking rule error.
## 2026-08-03 recovery-object semantic contamination check

- The historical `push_templates.rs` contains BR-160 A-10 content at lines 5941 and 14550, so whole-file recovery is conclusively forbidden.
- Allowed monitor paths contain the BR-194 R-04 dedicated SourceOnly helper/caller, plus an older R-08 combined-banner implementation. No searched BR-197/198/199/200 semantic API marker appeared, but marker absence remains non-authoritative.
## 2026-08-03 fixed-HEAD Magic revision correction

- `git show b4aeee68:Cargo.toml` and `git show 9307b67:Cargo.toml` both show `rusqlite` with only `chrono` and a sibling path dependency for `magic-tdx-rs`; neither fixed tree contains the 14 direct `5f1ce936...` pins.
- The current worktree's BR-192 row, Cargo files and revision test are candidate bytes, not committed authority. P1 must treat the release-revision closure as an atomic prerequisite amendment and obtain exact-blob review before Gate B.
## 2026-08-03 candidate authority blob inventory

- Working-tree Git blob IDs in order design/rules/Cargo/lock/revision-test: `f8ec9b5704f8a97a3bfdb31e64529183bf31e403`, `ec52754ace19f5e09341416abd37c4876963943e`, `17e4ff819323d1126f434875d4098681578243c8`, `24b2e7d0e4d912404213ad23a1abdf62792b5ad3`, `e79ac5a5d159e8cb534fd9778c5043dd65935f50`.
- All five are uncommitted candidates; `docs/business_rules.md` is both staged and further modified (`MM`), so Gate-A review must bind the working-tree blob explicitly and must not assume index equality.
## 2026-08-03 exact-hunk audit interim RED

- Segment hashing is reproducible from immutable objects, but the current closure is not complete.
- No immutable source object contains the final `5f1ce936...` Cargo/lock state; Cargo must be an accepted target amendment and lockfile regeneration, not an exact-copy recovery hunk.
- `admission.rs` and `provider_top_n.rs` are extraction/refactors with new fail-closed tests. Historical `dragon_tiger.rs` imports `super::review`, so even its whole-file blob needs a controlled seam adaptation.
- P2 notify closure is larger than designed: the committed durable runtime depends on the secure pinned push-log writer, eager binding, authoritative blocking delivery adapter and CLI receipt helper. Exact intervals/hashes are pending.
- Historical R-09 producer calls `CapitalDataGateway::provider_top_n_pair`; P3 must refactor it to `ProviderTopNDataGateway::pair`, not exact-copy the caller.
## 2026-08-03 notify dependency audit continuation

- Exact symbol search confirmed the baseline durable runtime calls the recovered secure push-log/eager-bind/authoritative-blocking APIs, so the P2 design's earlier narrow `notify` ownership is incomplete.
- Recovered `notify.rs` inspection confirms that authoritative counted delivery also depends on pending-byte persistence, schema-v3 audit publication, committed-marker verification, receipt joins, and the blocking CLI receipt helper. No production code was changed; the exact-hunk agent is computing immutable intervals and hashes before the Gate-A architecture choice.
- Re-read all mandatory repository companions (`docs/ENGINEERING_RULES_V2.md`, `.github/copilot-instructions.md`, `CLAUDE.md`) and reconfirmed Gate B is prohibited while the independent Gate-A review is RED. The broad dirty worktree is still preserved; recovery edits remain docs/planning only.
- Diff/definition audit narrowed the tracked WIP: its `notify.rs` contains many unrelated transport/token/daemon rewrites, so whole-file or broad tail admission is forbidden. Baseline helper definitions are sufficient for a smaller exact closure; remaining work is to hash the secure-writer, counted-interface, counted-finalizer and blocking-receipt intervals separately.
- Computed preliminary immutable SHA-256 hashes for four bounded `notify.rs` candidate segments and confirmed the two large authority segments are marker-clean with respect to excluded BR-160/197–200 behavior. Exact dependency closure and independent review remain open.
- P2 manifest work now has reproducible source object, full line range and SHA-256 identities for all three schema-v3 event files and the immutable append file. Two parallel agents were re-scoped to P1 Gateway and P3 producer manifests while the root agent closes P2.
- Added the tracked hunk-manifest draft and corrected the Gate-A design/working plan from impossible P1→P2 ordering to P2→P1→P3. The manifest binds P2 event files, bounded notify authority/test intervals and the P3 SourceOnly notify seam; P1/P3 producer rows remain pending parallel audit.
- Re-ran all P2-focused filters in the reconstructed clone: 3 envelope + 2 push-record + 3 persistence + 9 immutable-append + 24 monitor notify tests passed; the single ignored notify helper is exercised by a passing cross-process parent. Results are pasted into the manifest.
- Integrated the completed P3 immutable audit into the manifest: 11 production rows, seven in-process test rows and three process-isolation rows, including exact rejected enclosing ranges and two controlled test adaptations. Only P1 rows and fresh independent review remain before Gate B.
- Integrated the completed P1 audit into the hunk manifest, including the
  14-direct/15-lock target amendment identities, admission/Provider Top-N/
  DragonTiger ranges, BR-159 database authority and narrow glue. Corrected the
  historical admission capability/source pollution in the design and added a
  fourth attribution test requirement.
- Revalidated the four candidate blobs directly from the working tree:
  `ec52754a...`, `17e4ff81...`, `24b2e7d0...`, `e79ac5a5...`; all SHA-256
  values match the manifest. Scoped `git diff --check` passed and Rule 2.10
  passed with 198 rules plus the same 134 historical warnings.
- Froze the complete Gate-A review packet without changing its bytes: design
  Git blob/SHA-256 `261af572...` / `66f3823e...`; manifest
  `67a7686c...` / `5666a851...`; candidate rule/Cargo/lock/revision-test blobs
  remain `ec52754a...`, `17e4ff81...`, `24b2e7d0...`, `e79ac5a5...` with the
  exact SHA-256 values listed in the manifest. Fresh independent reviewers must
  name these identities; any drift invalidates their verdict.
- Executed the frozen revision-test candidate: it passes 1/1 in the broad
  current worktree. A fixed-baseline path audit then proved four files it reads
  do not exist at `9307b67` and are outside P1. Reported the resulting false-
  green/scope-expansion defect to all three fresh reviewers; the frozen packet
  is expected RED and Gate B remains closed.
- Recorded all three first-packet RED verdicts (`C2/I5/M0`, `C3/I1/M0`,
  `C1/I4/M0`) and kept production code closed.
- Repaired the Gate-A design/manifest only: rejected polluted rule/Cargo/lock/
  revision-test blobs; required a minimal `9307b67` delta; added BR-159 old-
  module disposition, explicit admission test rows, unconditional P3 test
  adaptations, exact-count verifier ownership, per-hunk splice/owner/test
  ledger and release-binary replay. No production source/database/provider/
  sink was changed or called.
- Next: materialize these exact docs in a temporary real Git commit, run scoped
  checks, then obtain fresh parallel `C0/I0/M0` review before Gate B.
- Ran three additional independent read-only closure audits. P2 was RED because
  13 required compile/runtime/test hunks were absent; P3 was RED because its
  SourceOnly tests, scheduler import and counted catalog were incomplete; the
  dependency/coverage review was RED because package identities and lock drift
  were not closed.
- Repaired only the Gate-A design and hunk manifest: added all verified P2/P3
  immutable ranges/hashes, moved the final cutover test to P3, separated M31
  from scheduler/R-09/R-04/catalog counts, froze V4/V5 test names, named all
  14 direct/15 lock Magic packages, assigned the dependency transition to
  BR-203 and added a fail-closed locked/offline checker contract.
- Recomputed fixed baseline identities: `Cargo.toml` blob
  `2118a3e490efe2d3416b2554559ca0347947c533`, SHA-256
  `521c3b24795288ddce453e714a74e23fe96afe348dfa49c5d68681f0fdf2adfa`;
  `Cargo.lock` blob `95481362e8061a1724cd1682d23b4e8a14f16377`, SHA-256
  `cd86df085943a710c17ec2cb5aceaef0acc0bde949443dce3fe802e99fbe74fd`.
- V3 remains Gate-A RED until the isolated generated target lock hashes/record
  whitelist and P2 compatibility-test closure arrive, are integrated, and a
  new real packet receives fresh independent `C0/I0/M0`. No production code,
  database, provider or sink was changed/called in this repair.
- Integrated the isolated Magic lock resolution into the Gate-A documents:
  exact target manifest/lock hashes, the 14-direct/15-lock Magic closure and
  every allowed package-record delta are now frozen. Locked/offline metadata
  resolution passed in the temporary baseline-derived repository; the main
  worktree was not modified by lock generation. Gate A now awaits only the
  fixed-baseline P2 compatibility goldens plus fresh packet review.
- Exported the P2 schema-v2 publication, parser-output and non-counted
  push-log artifact from a fixed-baseline mini probe with exact lengths and
  SHA-256 identities; no real provider or sink was called. The first attempt
  exposed an environmental ENOSPC during full monitor linking, so only the
  confirmed-unused recovery-audit target directory was cleaned (4.9 GiB of
  regenerable artifacts), after which the minimal probe succeeded.
- A fresh independent placeholder/realizability audit returned `C7/I5/M2`.
  Two parallel docs-only repair streams now cover the missing module/startup/
  P0/spy/exact-object contracts and the 95% coverage/bounded-startup evidence.
  Production source remains closed until the repaired immutable packet is
  independently `C0/I0/M0`.
- Executed the tracked-WIP schema-v2 mini probe and reproduced an actual
  parser-output byte regression caused by newly serialized `null` counted
  fields. Verified in the temporary clone that omitting absent schema-v3
  fields restores the fixed-baseline byte/hash exactly. This is now a required
  P2 compatibility hunk, not an allowed golden update.

## 2026-08-03 clean-lineage recovery proof

- Confirmed current worktree dependency state is the desired Polars 0.54
  family with no qmt-parser; corrected status reporting so historical parent
  versions are never presented as current blockers.
- Exported clean `96da674` to
  `/private/tmp/stock-analysis-head-check.Cd2KGl` and ran
  `cargo check --locked --lib`. It failed on five deterministic compile errors:
  four stale selection-audit callers and one missing counted schema constant.
- Ran `cargo check --locked --lib` in the isolated P2 candidate. The counted
  error disappeared and the same four selection caller errors remained,
  proving both problem slices independently.
- Collected three read-only parallel audit results. They confirm the current
  mixed worktree is green but cannot be committed atomically, the BR-164
  cutover must be partitioned by data domain, and the current Gate-A packet is
  RED until its implementation lineage is corrected.
- No production source, provider, database or sink was changed or invoked.
  Next action is a docs-only Gate-A rewrite followed by two fresh independent
  green reviews.
- Built two additional isolated dependency experiments. The current 0.54
  Cargo bytes over old source failed on 15 deterministic legacy/API errors and
  were rejected as a compile-foundation. The minimal historical-Cargo delta
  (`rusqlite/functions` plus direct same-path `magic-market-core`) passed
  locked/offline metadata and `cargo check --locked --offline --lib` with only
  one known warning. No repository production file was changed.

## 2026-08-03 P2-F evidence reconciliation

- Independent static review confirmed the candidate has exactly 19 CLI
  isolation tests and the full workspace has exactly the manifest-listed 16
  ignored tests, with all six ignored child helpers exercised by passing
  parents.
- Reconciled the manifest/design with the actual provisional selection targets:
  `outcome.rs` SHA-256 `13f3cbb8...` and `pipeline.rs` SHA-256 `bc345454...`.
  Removed the false claim that ignored `target/` blobs are already reachable.
- Strengthened the CLI fixed-production SQLite fingerprint with no-follow
  size/mode/mtime/ctime metadata, without reading or printing production DB
  contents. The exact CLI suite remains green: 19 passed, 0 failed/ignored.
- Narrowed the evidence claim to the exact SQLite trio and three named CLI
  regressions; event-audit/push-log namespace placement remains owned by the
  focused module tests. Updated candidate test identity to blob `1df27a0d...`,
  SHA-256 `a52f92c1...`.
- Restored P0-A3 business-ledger scope to only the BR-203 row. Rule 2.10 passes
  with 134 historical warnings and no blocking error; targeted diff check is
  clean. Fresh independent Gate-A review is still required before staging.
- Expanded the provisional P2-V0 structured verifier to hash every H11/H12/H13
  whole target and both raw inclusive `main.rs` hunks. The verifier compiles and
  the fail-closed wrapper passes; its exact mode/blob/SHA and runner-line hunk
  are now frozen in the manifest.
- Materialized P0-A3 as commit `4cf1573762e029b3fc90af91e8f7368322cdfabc`,
  direct child of `96da674...`, containing exactly the three authorized docs.
  Independent object review returned C0/I0/M0.
- The object review exposed a residual manifest defect: it still required P2-F
  to parent historical `96da674`, which would bypass P0-A3. Returned to Gate A
  and prepared P0-A4, limited to the two recovery docs, to require P2-F to be
  the direct child of the accepted Gate-A authority HEAD.

## 2026-08-17 M5 生产保真修复 + 阻碍项清单 (Task #76)

- 生产验证发现并修复 2 类缺陷 (commit 120b90d):
  1. BR-170 ×6「未知 provider Debug 名: tdx-dev」— convert.rs parse_provider 缺
     服务端 handlers.rs M1 fallback 值 "tdx-dev" 的映射 (语义即 Tdx)。修复后
     board-memberships 正常返回 provider=Tdx。
  2. delegate.rs 13 处网关类 map_err(format!) 折叠分类 (→ unknown/no_verified_batch/
     retryable=true → monitor 无界重试) → FetchFailure::from_gateway(e).with_message()
     保真传播 (provider/reason_code/retryable 直达)。fetch_consensus task error
     改 (String, GatewayError); fetch_board_ranking 非 GatewayError → unknown;
     fetch_research_reports task error 改 FetchFailure。
  3. GlobalIndices 网关钩子缺失 (服务端 delegate/桥方法已有) → zero-magic 生产
     构建 push_templates.rs us_indices 每天 fail-closed 报错。补 bridge_for 钩子,
     HOOKED_OPS 33→34 ops。
- 部署: server PID 73473 (00:24) + monitor PID 89949 (00:45, activation v39 生效)。
  banner 桥接 34 ops; 0 ERROR; BR-170 消失; BR-159 board-memberships 恢复为
  正确的 partial + provider=Tdx + retryable=false (周日无板块数据的正常业务态)。
- ⚠️ BR-183 踩坑 (记录): v38 生成后修改了 src/ (GlobalIndices 钩子) → hash 失配
  → activation_not_effective → selection-v2 全能力被禁。必须: 生成 activation
  → 落盘 → 此后不再改 src/ → 重启 monitor。

## 阻碍项 (未接完清单, 用户 2026-08-17 追问「还有哪些没接完」)

1. **14 个 magic-* git 依赖删除 (最终目标, 未完成)**: feature-flag 已让 monitor
   --no-default-features 零 magic 编译运行, 但 Cargo.toml 17 处声明仍在。
   删除依赖 = 本地 grpc_market_server 退役的前提 (server 是真实 magic 类型宿主,
   删依赖 server 无法构建)。阻塞于: 上游直连 10.211.55.3:50051 部署 (未来排期)。
2. **事件异动推送**: 价格/累计成交量/累计成交额已进生产事件流, 异动判定仍
   shadow (设计态, 非缺陷)。
3. **8 项诊断 op 数据合同未准入**: MoneyFlows/Auctions/FuturesDelivery/
   TechnicalBars/FundFlowSeries/PostCloseFlows/MarketRankings/MarketBreadth,
   UNADMITTED 诊断模式, 原因逐项见 client-bundle/grpc-external-api.md §10 表格。
4. **limit_pools/strong_stock_reasons (op 44/45)**: KEEP_LOCAL 设计保留,
   monitor/review 零引用 (数据消费经 ChainBatch op 61), 无生产影响。
5. **BR-159 board-memberships partial 观察项**: 周日无板块数据 → invalid_evidence
   是正常业务态; 若交易日持续 partial 需查服务端 BoardConstituents delegate。
6. **BR-178 GlobalSchema authority 失配刷屏 (每 60s)**: 预存问题 (memory:
   br178-recovery-flood-preexisting), 低优先待查。
