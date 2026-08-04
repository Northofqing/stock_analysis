# Production closure task plan

## Goal

Complete the unified `magic-market-data-rs` cutover, keep every production
failure explicit, validate the two operator commands against real delivery
paths, remove superseded configuration/documentation, and produce the Gate D
evidence required by `AGENTS.md`.

## Current execution authority — 2026-08-03 BR-192/BR-194 recovery

- [completed] Reconstruct fixed baseline
  `9307b6785420c32b57fe210f9c9b870d83e4a52d`, tracked WIP
  `2a4d1b929507fadadb082c2a803d5fea50cf6dd8` and untracked object
  `1389098b395a8894578259463923d58ab580a8b6` without touching production data.
- [in_progress] Close Gate A design/manifest. `P0-M0`, `P0-A1` and `P0-A2`
  are materialized through HEAD `96da674`. Current root Polars is `0.54`, the
  lock resolves the implementation family to `0.54.4`, and qmt-parser is
  absent; those bytes are the final BR-164 target, not authority for the
  historical predecessor. Independent review proved that the direct child of
  `P0-A2` must be one docs-only `P0-A3`; the known-red, uncommitted whole-Cargo
  proposal is rejected and must never be materialized as an intermediate
  commit. P0-A3 accepts only the minimal compile-green P2-F foundation. A
  separately reviewed compile-green BR-164 dependency-identity prerequisite
  then precedes P1 and preserves legacy dependencies until their callers are
  retired. `P0-A3` is committed as `4cf1573`, but post-materialization review
  caught one descendant-parent defect: the manifest still pointed P2-F at
  historical `96da674`. A two-document P0-A4 correction now requires P2-F to
  be the direct child of the accepted Gate-A authority HEAD; fresh independent
  review and a direct-child docs-only commit are in progress. Fresh
  implementation reviews run only after reachable candidate Git objects and
  exact argv/test filters are frozen.
- [in_progress] Gate B P2-F counted/shared-path compile foundation. The
  ignored isolated candidate now passes monitor tests in both default-thread
  and serial modes (`509/0/4`), strict workspace Clippy, and the 19-case
  monitor CLI isolation suite. It is not a reachable Git commit and therefore
  is implementation evidence only, not an accepted Gate-B artifact. The final
  locked full-workspace serial rerun passes after repairing `monitor --test`
  TEST_CODE installation before delivery-audit preflight; the post-repair
  format check and strict all-target/all-feature Clippy also pass. The exact
  locked all-target/all-feature Gate-C command now exits zero outside the
  filesystem sandbox, where deterministic loopback fixtures can bind local
  ports; only the manifest-authorized sixteen ignored tests remain. The latest
  static review corrected stale selection-target identities and candidate
  reachability wording; the strengthened 19-case CLI isolation suite is green,
  while fresh independent Gate-A C0/I0/M0 and a real reachable P2-F commit are
  still required.
- [pending] Separate Gate-A/B BR-164 dependency-identity prerequisite, then
  Gate B P1 BR-159/BR-162/Provider-Top-N real-source prerequisites.
- [pending] Gate B P3 atomic R-04/R-09 SourceOnly producer cutover.
- [pending] Gate B P4 exact-count/coverage/bounded-startup validation tooling.
- [pending] Gate C/D full serial fmt/clippy/tests/compliance/80-95 coverage,
  release build, bounded normal startup and authenticated R-04/R-09 replay/join.
- [pending] Cleanup superseded configuration/docs, rewrite README from final
  executable evidence, create the mandatory PR evidence and merge only after
  every release gate is green.

No new production source staging is authorized until the repaired P0-A3
receives `C0/I0/M0`. Existing broad worktree source bytes remain preserved but
are not accepted commits. This section is the current recovery checkpoint;
older BR-192 checkpoints below are retained as historical audit evidence rather
than current authority.

## Historical baseline (superseded as release evidence)

- The broad root worktree previously resolved all downstream Magic crates to
  `magic-market-data-rs` `=0.2.0` at immutable revision
  `5f1ce93656a55854c844065390520cd4aecd9a14`; the current isolated P2 candidate
  does not. It still has only two sibling path Magic packages, qmt-parser, and
  Polars 0.46/0.52, so BR-164 remains pending.
- `cargo run --bin monitor -- --review` most recently exited zero. R-04 and
  R-09 reused durable delivered outcomes without provider or sink calls; A-10
  rebuilt a real chain batch and obtained a validated Feishu receipt.
- The old `cargo run --bin monitor -- --test` 40-template result is no longer
  release evidence. BR-196 now defines the closed 64-family/58-kind lifecycle
  and requires either an explicitly authorized non-production Feishu receipt
  run or the dedicated zero-transport `--test --push-dry-run` acceptance.
- R-03 remains unavailable because no real broker position batch is joined to
  a same-batch trade-sync watermark. Local projections are not source evidence.
- R-08 remains unavailable on a first execution because the pinned Magic
  release formally reports the mandatory CFFEX delivery-calendar capability as
  `Unsupported`. Diagnostics are not admitted as production evidence.
- BR-199/BR-200 replay now accepts a verified-empty R-08 terminal, projects
  only the exact reminder date in canonical order, preserves missing CNInfo
  category as missing, and records the public-component source/rule lineage.
  The focused R-08 suite passes 26/26.
- BR-160 source-batch delivery audit schema v4 now retains A-10 business date,
  observed/as-of time, batch ID and content hash. The focused envelope,
  durability and monitor-governor tests pass 4/4.
- The business-rule compliance hard failure for the CFFEX BR-199 path is fixed;
  the rule checker passes with historical non-blocking warnings.
- BR-196's latest formal Gate A review is RED C1/I4/M1. Rule 2.10 currently
  rejects the uncited target allowlist; VirtualWatch is falsely Active; the
  direct health-webhook presentation is missing; the evidence commands and
  source binding are incomplete; and the public API migration inventory omits
  changed/deleted entrypoints. A fifth design/rule/config-citation repair is
  active; no Gate B implementation is authorized.
- BR-201's third formal Gate A review is RED C0/I9/M0. Hashes, 26-field record
  shape, double quote-freshness boundary and unique BR rows are correct, but
  the design still has non-deterministic debounce/session audit ordering, a
  missing manual-confirmation reason, unspecified basis-point reconciliation,
  contradictory BR-086 atomic audit semantics, unsafe concurrent Claimed and
  repeated-boot recovery, an open joined-fact cardinality, a rollback-authority
  contradiction, and no real provider for the proposed account batch. Gate B
  remains closed pending a third repair and another independent review.
- BR-202's latest formal Gate A review is RED C2/I2/M0. The fixed-source
  derivation omits integration-test inputs; the publication order can leave a
  verifier-valid bundle after parent-fsync failure; exact object/executable
  bytes are not exported; and the behavior-completeness equations have no
  independent production-decision denominator. A fifth design/rule repair is
  active; no Gate B implementation is authorized.

## Active closure sequence

> 2026-08-03 recovery note: the pre-merge stash has been reconstructed in a
> disposable clone and passes library check plus the first exact BR-192 test.
> The current active sub-step remains proof of the full pure BR-192/BR-194
> recovery boundary; no production-tree edit is authorized until all exact
> focused tests and the frozen checker are evaluated.

> 2026-08-03 Gate A update: focused evidence is green on the reconstructed
> snapshot and a dedicated incomplete-commit recovery design now exists.
> Independent C0/I0/M0 review plus the exact R-04/R-09 predecessor closure are
> still required before implementation.

> 2026-08-03 recovery review update: two independent axes returned RED
> `C2/I6/M0`. The design has been repaired for the additive BR-159 schema,
> forward-compatible counted rollback, atomic SourceOnly producer cutover,
> current `5f1ce936...` authority, exact non-zero test counts, semantic hunk
> exclusion, serial full tests, coverage/release build and causal Gate-D
> replay/join. An exact source-object/hunk hash manifest and fresh independent
> C0/I0/M0 review remain required; Gate B is still closed.

1. [completed] Close the five independent-review findings in R-08/A-10 and run
   their focused regression suites.
2. [in_progress] Close BR-192 Gate A against fixed
   `HEAD=b4aeee68d2c0259cc968914b3d39e3a89a18a496`. The repair freezes all
   15 counted kinds as `DisabledNoProducer` and adds one schema-v5-to-v6
   migration while preserving BR-194. Exact staged design/plan/rule identities
   pass whitespace and Rule-2.10 checks; independent review infrastructure is
   currently failing before sampling with HTTP 403, so Gate A remains open.
3. [pending] After BR-192 receives independent C0/I0, implement its Gate B
   contract test-first, then pass focused/full Gate C. Current worktree is still
   schema v5 and does not contain the new retry-cycle authorities.
4. [pending] Repair and close BR-196 Gate A/B/C against the complete actual
   production caller graph. Preserve typed target and one-shot transport safety
   while replacing self-attestation with independent reachability evidence and
   reclassifying every unproved producer.
5. [pending] Repair and close BR-201 Gate A/B/C, then implement the fail-closed
   Asia/Shanghai paper-engine session permit, lazy risk-context acquisition and
   pre-side-effect TOCTOU revalidation.
6. [pending] Repair and close BR-202 Gate A/B/C/D and generate fresh coverage
   evidence. Overall coverage must be at least 80% and core trading/data links
   at least 95%; prior reports are not reusable.
7. [pending] Run the serialized release gates after those changes converge:
   `cargo fmt --all -- --check`, workspace strict Clippy, full workspace tests,
   and `bash tools/compliance/check.sh`.
8. [pending] Re-run `monitor --review`, `monitor --test`, and a bounded normal
   monitor startup after the final code/config state is fixed.
9. [pending] Complete the mandatory PR evidence, independent review, merge, and
   release handoff.
10. [pending] After BR-196/BR-201 stabilize, perform the separately designed
   cleanup: remove the unused `PUSH_VERBOSE` semantics and dead configuration
   only through their Rule 2.9/2.10 Gate A, mark or archive superseded provider
   documents without deleting audit history, and recalibrate README runtime and
   template-count claims from final executable evidence.

## Capability work that is not complete

- R-03: connect a real broker snapshot owner and exact trade-sync watermark.
- R-08 first delivery: upstream must expose an admitted official CFFEX calendar
  batch; no inferred or manually fabricated calendar is allowed.
- R-02: complete-market ranking capability is not admitted. R-09 Provider Top-N
  is intentionally narrower and cannot be relabelled as R-02.
- R-05: no source-to-delivery-to-execution-to-settlement linked evidence chain.
- R-06: no evidence-bound classified failure source.
- A-01: an exact T+1 observation may legitimately be `no_data`; this is not a
  production error when the required observation does not exist.
- Selection-v2 activation and the larger v18/v19 architecture remain disabled
  until their authoritative owner/recovery/read-model gates are complete.

## Release blockers

- BR-192 is the earliest open gate. Three subsequent read-only prechecks found
  cross-rule `C2/I5/M0`, state-machine `C0/I1/M0`, and executable Gate-A
  `C0/I4/M1`; the previously staged identities are obsolete. The working
  authority now repairs the R-09 SourceOnly signature, BR-198 14-direct/
  15-lock dependency closure, BR-200 occurrence mapping, retry-only expiry,
  capture-time boundaries, deferred terminal-result bijection, BR-202 Gate-D
  wrapper authority and missing exact test bodies. Fresh validation and two
  independent C0/I0 reviews are still required. BR-192 Gate B and all later
  implementation remain sequence-blocked.
- BR-201's latest realizability review is RED C2/I3/M0: its open-attempt and
  delivery schemas cannot encode the required recovery/AckPending state
  machines, the identifier inventory is incomplete, the rollback bootstrap is
  not a tracked buildable authority, and 14 ACs contradict the exact 13-marker
  predicate.
- BR-202's latest formal review is RED C3/I2/M0: it chains from unverified
  batches, has no stable-Rust success path, lacks a compliant critical audit
  chain, assumes a false two-blob index, and omits required PR evidence.
- Interim strict Clippy is RED: `br196_test_delivery.rs:278` triggers
  `clippy::manual_contains`. Fix it with the direct `.contains(...)` form only
  after BR-196 Gate A is accepted; do not suppress the lint.
- The most recent business-rule checker is RED with eight blocking errors and
  125 non-blocking historical warnings: one BR-196 config citation and seven
  BR-202 missing implementation/citation paths. The omitted BR-202 entrypoint
  test path was additionally a Gate A registration defect. These are not
  authority to implement before the corresponding Gate A acceptance.
- BR-202 Gate A is RED C1/I4/M1: its local path/device/inode and read-only mode
  authority cannot survive the CI upload/download boundary; host linker/SDK/
  tools are outside the fixed-input closure; the D/M denominator has no exact
  executable interface; tracked invocation inventory requires untracked files;
  the BR row is not in the index; and ranking evidence omits its generator/full
  top-20 output.
- BR-196 Gate A is RED C1/I2/M0: three new authority source files, the compiled
  allowlist and current BR row are not yet in the Git preimage; the advertised
  full public-API audit only covers `push_`/`dispatch_` functions and misses
  enum/struct/news/T0 changes; and pasted Rule-2.10 output is stale/unbound.
- BR-201 Gate A is RED C1/I7/M0. Rollback still lets a mutable caller-worktree
  script choose its own trust base; initial Admission/account-terminal ordering
  is contradictory; private authority and final transaction owner are assigned
  to different modules; takeover transitions are absent from the closed set;
  two order reasons are unmapped/overlapping; a Confirmed audit is incorrectly
  required before its atomic transaction; PR evidence omits Rule 2.3; and
  current API/caller counts lack reproducible commands/output.
- BR-201 Gate A seventh independent review is RED C1/I7/M1. The rollback
  bootstrap still executes ambient caller-controlled tooling and does not prove
  raw caller-worktree bytes unchanged; Admission has no closed private handoff;
  invalid/missing account provenance is not representable without fabrication;
  proposed symbols/paths and SQLite schema objects are incomplete; compatibility
  aliases do not durably bind legacy code/reason bytes; proposal ordinal sorting
  lacks an exact total key; and the shared BR-201/BR-134 rows remain intentionally
  unstaged until all concurrent business-rule edits finish. One unrelated GFM
  row also contains an unescaped inline pipe.
- Any freshness, fake-implementation, business-rule, design-contradiction,
  test, Clippy, or coverage failure.
- Any attempt to replace missing broker/CFFEX evidence with local state,
  inference, mock data, or a diagnostic probe.
- Missing PR evidence fields required by `AGENTS.md` Part 3.
- Live BR-196 Feishu Gate-D evidence cannot run against the current target: its
  domain-separated identity is release-pinned as `production_deny`, and no
  distinct reviewed non-production conversation is configured. The code must
  remain fail-closed until such a target is explicitly provisioned and hashed.

## Rollback

Before production execution, discard an unmerged recovery branch normally.
After any production schema open or counted reservation, never blindly revert
the atomic slices. Stop/freeze new producers, preserve the schema-v3-aware
authority, reconcile all dates, manually resolve every Uncertain state, prove
zero active/pending authority, then deploy a forward-compatible producer-
disabled build that retains every BR-159/schema-v3 table, trigger, reader,
immutable record and audit file. Never downgrade schema or delete live data.

## Current BR-192 repair checkpoint (2026-08-02)

- [x] Record exact C1/I6/M1 and C0/I3/M0 reviewer findings.
- [x] Record exact C0/I4/M0 and C1/I3/M1 reviewer findings.
- [x] Select one-consumer architecture: R-09 enabled, 14 counted kinds disabled.
- [x] Repair design/plan/rule for expiry, permit/caller enforcement, evidence
      paths, fixed-HEAD migration-test identity, v7 rejection and test/file map.
- [x] Freeze the concrete non-forgeable permit API, current-catalog retry
      provenance, active expiry drain, genuine RED bodies, first Gate-B file
      action and the complete BR-198 14-direct/15-lock dependency pin.
- [x] Repair preliminary C3/I2/M0 expiry/clock/total-order findings.
- [x] Pass scoped diff and Rule-2.10 checks for the repaired authority triple.
- [ ] Stage exact design/plan/BR-192 row and compute new identities.
- [ ] Obtain fresh parallel independent C0/I0 Gate-A verdict.
- [ ] Only then begin BR-192 Gate B implementation.

## Current BR-192 final-expiry repair checkpoint (2026-08-02)

- [x] Record the latest independent C1/I7/M0 and C1/I3/M1 findings.
- [x] Repair the final pre-call two-transaction effective terminal, reverse
      result trigger, exact cycle recount, R-09 Rule-2.3 semantics, fixed-HEAD
      inventories, clock/error ownership, root constant and task ownership.
- [x] Pass scoped staged whitespace and Rule-2.10 checks; Rule-2.10 retains 131
      historical warnings and reports zero hard errors.
- [x] Stage the exact authority triple and record the identities above.
- [ ] Obtain two fresh independent C0/I0 verdicts for those exact objects.
- [ ] Only then begin BR-192 Gate B implementation.

## Current BR-192 cross-rule/state repair checkpoint (2026-08-02)

- [x] Record the three independent precheck verdicts: C2/I5/M0, C0/I1/M0,
      and C0/I4/M1.
- [x] Remove R-09 banner/account coupling and freeze the SourceOnly signature.
- [x] Reconcile BR-198 to the exact 14-direct/15-lock immutable dependency
      closure and add its executable release-revision test.
- [x] Freeze the BR-200 occurrence-state mapping, ordered rule IDs, capture
      boundaries and initial-versus-retry expiry semantics.
- [x] Specify deferred terminal-result bijection constraints, reverse triggers
      and mutation/race tests.
- [x] Route BR-192 Gate D exclusively through the BR-202 isolated wrapper and
      add the previously missing exact test bodies.
- [x] Re-audit every exact test command against a unique plan declaration and
      remove stale metadata wording: 245 unique commands, 249 declaration-
      shaped names, zero missing and zero duplicate declarations. The four
      non-command names are three parent-invoked ignored child helpers and one
      quoted fixed-HEAD BR-194 evidence snippet, not standalone Gate-B tests.
- [x] Pass scoped whitespace, Rule-2.10 and dependency-closure validation.
- [ ] Freeze the repaired authority objects and obtain two fresh independent
      C0/I0 Gate-A verdicts.
- [ ] Only then begin BR-192 Gate B implementation.

The latest three read-only prechecks are RED and supersede the prior candidate:
state `C1/I0/M0`, cross-rule `C1/I1/M0`, executable `C1/I2/M0`. Gate B stays
closed while the docs-only repair normalizes terminal-result pointer-first
ordering, keeps BR-198 rollback on the admitted `5f1ce936...` dependency,
removes the BR-192/BR-202 progression cycle, gives the fixed-HEAD-absent
dependency test explicit create/commit ownership, and supplies executable
BR-198/BR-200 prerequisite plans.

## Errors encountered

| Error | Attempt | Resolution |
| --- | --- | --- |

| Combined findings/progress append used a heading that exists only in `progress.md` | 1 | Located the actual EOF anchors with `rg`/`tail`, then appended each planning record under its own exact existing section |
| BR-196 business-rule long-line patch did not match the exact existing text | 1 | Re-read the authoritative line, applied one exact replacement, then verified the row count is 1, stale count phrases are absent and `git diff --check` passes |
| Business-rule compliance check failed | 1 | Classified the 3 blocking errors as pending BR-196/BR-202 Gate B implementation; retained 124 historical warnings separately and notified both formal reviewers |
| BR-202 second formal Gate A review returned C0/I5/M1 | 1 | Kept Gate B closed and opened a third docs-only repair covering wrapper entrypoints, cleanup/evidence retention, capability registry, semantic attestation, isolation allowlists and integer bounds |
| BR-201 third formal Gate A review returned C0/I9/M0 | 1 | Kept Gate B closed and opened a docs-only repair covering deterministic decision audit, closed reasons/rounding, BR-086 ownership, concurrent recovery, joined-fact cardinality, release rollback authority and a real account provider |
| BR-196 fresh formal Gate A review returned C0/I4/M0 | 1 | Kept Gate B closed; opened a docs-only repair to include replay, reproducible full-hop evidence/source binding, public API impact and valid GFM/Rule-2.10 registration |
| BR-202 fresh formal Gate A review returned C0/I7/M0 | 1 | Kept Gate B closed; opened a docs-only repair for realizable release entrypoints, pre-pinned failure evidence, behavior-authority enumeration, build/profile binding, offline dependencies, complete paths and artifact migration |
| BR-201 fresh formal Gate A review returned C1/I7/M0 | 1 | Kept Gate B closed; opened docs-only repair for legacy/V1 dual-key cutover, closed account reasons, exact inverse rollback proof, scoped BR supersession, debounce genesis, API compatibility, complete paths and honest Disabled evidence |
| BR-196 fifth formal Gate A review returned C1/I4/M1 | 1 | Kept Gate B closed; opened a scoped design/rule/config-citation repair for Rule 2.10, truthful lifecycle state, health-webhook inventory, bounded evidence/source binding and complete API migration |
| BR-202 fifth formal Gate A review returned C2/I2/M0 | 1 | Kept Gate B closed; opened a scoped design/rule repair for complete build/test inputs, durable post-publish authority, replayable object bytes and an independent production-decision denominator |
| BR-201 v5 reviewer returned the user's concurrency answer instead of a formal verdict | 1 | Reopened the same reviewer with an explicit completion contract requiring the remaining checks and a `VERDICT: GREEN\|RED; Critical=N Important=N Minor=N` result; no review credit was given |
| Recovery Gate-A review returned C2/I6/M0 | 1 | Kept Gate B closed; repaired schema/rollback, removed the generic R-04 intermediate state, added exact evidence/counts, semantic hunk admission, serial full gates, coverage/release build and audited replay/join; exact hunk manifest and rereview remain open |
| Combined progress/findings patch contained an invalid cross-file hunk | 1 | Reissued two independently anchored update hunks; no source/design content was lost |
| BR-201 v5 formal review returned C2/I5/M0 | 1 | Kept Gate B closed; opened a fifth scoped docs/rule repair for Git tracking, spec-only Rule 2.10 registration, total account reasons, an unforgeable private BR-201 simulate owner, frozen legacy time semantics, coherent cutover eligibility and clean detached-worktree rollback |
| BR-202 fifth repair completed with the design still untracked | 1 | Staged only the exact BR-202 design file, verified it with `git ls-files` and separate staged/unstaged whitespace checks, left shared business-rules staging untouched, and started a fresh v6 formal reviewer |
| BR-202 v6 reviewer startup failed with external HTTP 403 | 1 | Classified as reviewer infrastructure failure, gave it zero review credit, and immediately retriggered the same independent formal-review brief |
| BR-202 v6 reviewer retry also failed with external HTTP 403 | 2 | Retired that agent instance and started a fresh v7 reviewer with an isolated no-history context and a self-contained formal-review brief |
| BR-196 fifth repair completed with the design initially untracked | 1 | Staged only the exact BR-196 design, verified tracking and separate whitespace scopes, preserved the empty/untracked allowlist and shared business-rule index state, then started a fresh v5 formal reviewer |
| BR-201 fifth repair completed after C2/I5/M0 | 1 | Preserved spec-only Rule-2.10 registration, staged only the exact design, verified the current checker passes without BR-201 hard errors, left shared business rules unstaged, and started a fresh v6 reviewer |
| BR-202 v7 formal review returned C1/I4/M1 | 1 | Kept Gate B closed; opened a sixth docs/rule repair for a portable archive+detached terminal, complete host execution inputs, an exact executable D/M interface, tracked-only invocation inventory, complete ranking evidence and index-ready row wording |
| BR-201 v6 formal review returned C1/I7/M0 | 1 | Kept Gate B closed; opened a sixth docs/rule repair for an immutable deployed rollback trust root, unambiguous initial-admission ordering, a same-module nested private transaction owner, complete takeover transitions/reason mappings, atomic audit sequencing, Rule 2.3 PR evidence and reproducible current API commands |
| BR-196 v5 formal review returned C1/I2/M0 | 1 | Kept Gate B closed; opened a sixth docs/rule repair for an exact tracked source preimage, a bounded full public-item/variant/field/re-export audit and current manifest-bound Rule-2.10 evidence |
| BR-201 v7 formal review returned C1/I7/M1 | 1 | Kept Gate B closed; opened a seventh docs-only repair for a single immutable race-free rollback bootstrap, closed Admission capability, reason-specific provenance, exhaustive API/schema inventories, durable alias inputs, deterministic proposal ordering and minimal GFM repair; shared rules remain unstaged until concurrent writers finish |
| BR-201 v7 process-evidence patch missed a `progress.md` heading | 1 | Confirmed the failed patch wrote nothing, reread exact anchors with `rg`/`sed`, split the update into bounded patches, and reran scoped plus staged whitespace checks successfully |
| Exact focused test command used an incomplete module path and selected zero tests | 1 | Re-ran both failures with their full module paths and confirmed each executes exactly one test and fails at the known assertion |
| `cargo test --locked --bin monitor -- --test-threads=1` is RED | 1 | Recorded the baseline as 562 passed, 2 failed, 4 ignored; isolated the BR-196 allowlist hash drift and stale R-08 public-wrapper contract before implementation |
| `check_br194_review_dependency.sh` is RED on the removed public helper marker | 1 | Traced the actual dispatcher → presented public wrapper → private source-only helper chain; Gate-B fix must validate both layers rather than restoring the obsolete public API |
| Three fresh parallel agent attempts failed with external HTTP 403 | 2 | Assigned zero review/repair credit, retained Gate A as unaccepted, and continued bounded local diagnosis; fresh independent C0/I0 review remains required when agent service recovers |
| BR-202 seventh formal Gate-A review returned C3/I2/M0 | 1 | Parked Gate B; recorded no-spec-on-unverified-gate chaining, impossible stable `-Z`/rustdoc-json success path, incomplete critical audit contract, unsatisfied combined-index staging premise, and incomplete PR evidence. BR-202 cannot progress before earlier gates close |
| Combined BR-192/BR-202 wording patch missed a wrapped design anchor | 1 | Verified the failed patch wrote nothing, reread the exact wrapped paragraph, split to bounded exact anchors and removed the circular gate order without touching production code |
| Three-way BR-192 precheck returned C1/I0/M0, C1/I1/M0 and C1/I2/M0 | 1 | Kept Gate B closed; split transaction-order, BR-198, BR-200, BR-202-order and dependency-test ownership repairs into bounded docs-only slices |
| Read-only `rg` verification accidentally contained shell backticks | 1 | The shell attempted to execute the backticked test path and printed permission denied; no file was changed by that attempt. Future searches use literal-safe patterns |
| Combined planning-log patch missed a wrapped `progress.md` anchor | 1 | Confirmed the failed patch wrote nothing, reread exact file tails, and split the task/progress/findings updates into bounded patches |
| Independent BR-198 Gate-A review returned C2/I8/M1 | 1 | Kept all production gates closed; removed the impossible standalone BR-198 Gate-C prerequisite, folded its date/capture/dependency contract into BR-192 Task 8, and made independently accepted BR-200-with-R09-disabled the sole cross-rule Gate-C prerequisite |
| BR-200 implementation-state audit found only 6 substitute tests and 21/23 planned names missing | 1 | Classified the worktree code as an unsafe partial prototype, retained Gate B closed, and narrowed the BR-200 Gate-A slice to a typed read-only generic API plus real R-04/R-08 consumers while R-09 remains disabled |
| Combined BR-192 design/plan patch missed a wrapped self-contained-baseline anchor | 1 | Confirmed the failed patch wrote nothing, reread exact ranges, and applied smaller exact `apply_patch` edits for the dependency sequence and Shanghai capture window |
| BR-198 scan used obsolete guessed filenames | 1 | Located the tracked supporting contract as `2026-08-01-r09-settled-closed-day-review{,-design}.md`, reran the scan on those exact paths and confirmed the forward-only rollback text is present |
| Rule-2.10 check rejected future rollback artifacts in the BR-200 Code cell | 1 | Removed the not-yet-created patch/verifier paths from the registry Code cell while retaining them as Gate-B deliverables in design/plan; rerun passed with 198 rules and zero hard errors |
| Literal search for a backticked PushKind name used double-quoted shell text | 1 | The shell attempted an unintended command substitution and changed no files; switched subsequent searches to single-quoted/literal-safe patterns and retained the incident in the planning log |
| BR-198 formal review returned C0/I1/M0 | 1 | Unified the sole completion-window field to `capture_completed_at` in BR-192 design/plan/rule; `request_completed_at` is now absent from the authority set |
| BR-192 replacement review returned C0/I2/M0 | 1 | Replaced pre-commit HEAD verification with a fully staged `write-tree`/`commit-tree` candidate whose tree must equal the final commit, and expanded patched-tree verification to the complete BR-198/BR-200/schema/catalog/revision/recovery/startup matrix |
| BR-200 formal review returned C1/I3/M0 | 1 | Kept production code closed; narrowed independent BR-200 to live R-04 only, assigned R-08/R-09 typed disabled capabilities pending BR-199/BR-192, and opened a docs-only repair for a closed capability map, additive checker preservation and reproducible fixed-HEAD evidence |
| Literal stale-semantics scan again used a backticked token inside double-quoted shell text | 2 | Command substitution failed with `command not found` and changed no files; all subsequent shell regexes use single-quoted literal-safe patterns |
| BR-200 R-04-only formal review returned C0/I5/M0 | 1 | Kept Gate B closed and opened a bounded docs-only repair for candidate-tree rollback verification, executable checker-prefix protection, bidirectional test manifest equality, R-08 typed Unsupported authority separation, and R-04-specific rule vectors |
| BR-200 second reviews returned C0/I2/M0 and C0/I5/M0 | 1 | Kept Gate B closed; added the exact R-09 banner and Gate-D evidence, then repaired the remaining R-08 future-profile, verifier-owned digest, and actual Cargo registration/exact-one-test gaps |
| Two combined BR-200 design/plan patches missed wrapped Markdown anchors | 1 | Confirmed each failed patch was atomic, reread the exact ranges, and applied bounded `apply_patch` edits without production-code changes |
| Local BR-200 audit used zsh-reserved lowercase `commands` | 1 | The shell rejected the assignment before the audit ran; no file changed. Re-ran with task-scoped `br200_command_file` and retained uppercase non-reserved names in the implementation plan |

## Current BR-200 second-review repair checkpoint (2026-08-02)

- [x] Add the exact R-09 `disabled=no_producer` startup banner contract.
- [x] Add production push-log/event-bus and cross-version debt Gate-D evidence.
- [x] Define closed BR-200-baseline and complete BR-199-enabled R-08 checker profiles.
- [x] Replace caller-supplied append digest with verifier-owned literals and both mutation-matrix PASS proofs.
- [x] Require canonical manifest ↔ source declaration ↔ Cargo registration equality and exact-one-test execution.
- [x] Pass scoped command/declaration/document consistency and Rule-2.10 checks.
- [x] Freeze the exact staged authority objects at business/design/plan SHA-256
      `36a56278…` / `257464e1…` / `2e8822f6…`.
- [ ] Obtain two fresh C0/I0 verdicts for those exact staged objects.
- [ ] Only then begin BR-200 Gate B implementation.

The exact-hash reviews returned RED C0/I8 and C0/I5, superseding the checkpoint
above. Gate B remains closed while one final bounded Gate-A repair addresses the
accepted BR-194 base, stable status/row ownership, startup banner call site,
26th runtime test, immutable candidate verifier execution, fail-closed shell,
causal Gate-D evidence, baseline coverage and variable rule-vector authority.

## Current BR-200 final Gate-A repair (2026-08-02)

- [x] Make design/plan status stable and externally accepted.
- [x] Make accepted BR-194 Gate C a pinned Gate-B prerequisite.
- [x] Replace fixed five-element evidence storage with validated ordered rules.
- [x] Add `main.rs` ownership and the 26th exact-once process test contract.
- [x] Replace `bash -lc`/weak summaries with closed argv and non-ignored proof.
- [x] Require candidate/HEAD-tree verifier execution and fail-closed pipelines.
- [x] Replace BR-202 coverage dependency with repository baseline coverage.
- [x] Define phased causal repeated-review evidence ownership.
- [ ] Rewrite and hash the stable canonical BR-200 registry row.
- [ ] Run scoped audits and fresh C0/I0 exact-object reviews.
- [ ] Keep Gate B blocked until both Gate A and pinned BR-194 Gate C pass.
# Current BR-200 Gate-A repair (2026-08-02)

- [x] Repair BR-194/BR-199 checker for the real R-08 two-layer public/private route.
- [ ] Replace BR-200 fixed-HEAD checker-prefix binding with the independently accepted BR-194 Gate-C checker blob/boundary.
- [ ] Fix Task-4 staged allowlist ordering.
- [ ] Add explicit implementation, test, stage, commit, and committed-tree verification for `verify_br200_repeated_review.py`.
- [ ] Re-freeze exact design/plan/BR-200 row hashes and obtain fresh independent C0/I0 review.
- [ ] Keep BR-200 Gate B blocked until literal accepted `BR194_GATE_C_SHA` exists and revalidates cleanly.

## Current BR-194 master-baseline Gate-C repair (2026-08-02)

- [x] Reproduce the accepted-claim mismatch on clean `master=9307b67`:
      `cargo build` is RED on a missing counted-audit schema constant and the
      dedicated checker is RED on a missing R-04 preparation seam.
- [x] Apply the bounded six-terminal-reason repair in
      `/private/tmp/stock-analysis-br194-master-impl` and pass
      `cargo fmt --all -- --check`.
- [x] Preserve BR-200 as blocked; its frozen checker prefix cannot be rebased
      until an independently accepted BR-194 Gate-C commit exists.
- [x] Diagnose the first focused-test attempt as an environment failure:
      isolated Cargo compilation exhausted disk space before test execution.
- [x] Recover about 14 GiB by running `cargo clean` only in the two abandoned
      BR-194 worktrees; source, databases and the active worktree were not
      removed.
- [ ] Finish root-cause audits for the missing event schema constant and the
      R-04 SourceOnly/checker mismatch before making another production edit.
- [ ] Add a failing regression for each confirmed root cause, implement only
      the accepted minimal fixes, then rerun focused BR-194 tests/checker.
- [ ] Pass full Gate C (`fmt`, strict Clippy, full tests, compliance) and record
      exact evidence before declaring BR-194 accepted or restarting BR-200.

## Current BR-192/BR-194 incomplete-commit recovery (2026-08-03)

- [x] Preserve the dirty user worktree and bind the fixed baseline plus tracked
      and untracked recovery objects.
- [x] Prove the fixed baseline is uncompilable because counted schema-v3
      authority is absent; correct implementation order to P2 → P1 → P3.
- [x] Enumerate exact P2 counted-authority ranges and rerun its non-zero focused
      test baseline.
- [x] Enumerate P3 R-04/R-09 production, replay, scheduler and test ranges,
      rejecting broad enclosing copies and later-rule contamination.
- [x] Enumerate P1 admission, Provider Top-N, DragonTiger, BR-159 database,
      dependency and glue authority; correct historical hard-coded audit
      attribution through a controlled, tested target adaptation.
- [x] Pass scoped whitespace and Rule-2.10 checks for the complete Gate-A
      candidate; Rule 2.10 reports 198 rules and 134 historical warnings.
- [x] Obtain first frozen-packet reviews; all three returned RED and identified
      polluted candidate authority, incomplete splice/test ownership and
      non-release validation commands.
- [x] Repair the design/manifest without production edits: reject the polluted
      candidates, define minimal baseline-derived target contracts and close
      BR-159/P3/test-count/replay/manifest traceability gaps.
- [ ] Materialize the repaired design/manifest in a real Git commit object and
      obtain fresh parallel independent C0/I0/M0 verdicts.
- [ ] Only after Gate A is green, implement and verify P2, then P1, then P3 in
      the isolated recovery branch.
- [ ] Complete Gate C/D, runtime/replay evidence, cleanup/README, PR review and
      merge to master.

## 2026-08-03 executable recovery re-anchor

- [x] Confirm the ambient target graph is already Polars `0.54`/`0.54.4`, all
      direct Magic crates are `0.2.0` at one revision, and `qmt-parser` is
      absent. Historical `HEAD=96da674` remains an extraction source only.
- [x] Export `HEAD=96da674` into an isolated clean tree and prove it is
      compile-red on exactly five missing selection-audit/counted-delivery
      symbols; do not claim a BR-164-only green predecessor from the mixed
      worktree.
- [x] Compile the isolated P2 candidate far enough to prove the counted schema
      closure removes the counted-delivery error and leaves only the four
      selection-audit caller incompatibilities.
- [ ] Amend the three BR-203 Gate-A documents to define one minimal compile
      foundation: strict selection-audit caller adaptation plus complete P2
      counted-delivery closure, followed by a separately reviewed BR-164
      dependency-identity prerequisite, P1 sources, and then separately owned
      BR-164 domain cutovers. The foundation must not resurrect
      caller-controlled production audit paths and must not adopt the stale
      whole-Cargo target.
- [x] Prove the executable dependency boundary in an isolated candidate:
      applying the current 0.54/no-qmt Cargo target directly to old source is
      RED with 15 missing-dependency/API errors, while the historical manifest
      plus only `rusqlite/functions`, same-path `magic-market-core`, complete
      P2 and strict caller adaptations is compile-green with one known
      dead-code warning to close before Gate B.
- [ ] Obtain two fresh independent `C0/I0/M0` reviews for the amended exact
      three-document packet, then commit only those documents.
- [ ] Implement the compile foundation from the accepted packet, validate it
      in a clean reconstruction, and commit it independently.
- [ ] Commit and validate the BR-164 dependency-identity prerequisite, stage
      P1/P3, execute BR-164 as small per-domain commits, then restore all
      review/test templates and counted Feishu delivery before lower-priority
      cleanup.
- [ ] Run full Gate C/D, bounded live `monitor`, README/config cleanup, PR
      evidence and merge only after the business path is proven.
