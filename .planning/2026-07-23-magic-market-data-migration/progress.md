# Progress Log

## Session: 2026-07-27 reconciliation

### Current Status
- **Phase:** 4 — final downstream cleanup and release-gate preparation

### Actions Taken
- Preserved the exact admitted `GlobalNewsRecord + BatchEvidence` batches from
  the same four unified Gateway acquisitions alongside the lossy
  `MarketEvent` projection. Event simhash dedup no longer destroys the evidence
  required by BR-172.
- Added immutable NewsAI assessment persistence and a hash-chain audit boundary.
  The opt-in Shadow producer now consumes the exact admitted news batch plus
  sealed quote/daily-bar evidence, performs one real model call with a bound
  upstream receipt, and writes only the immutable assessment audit.
- Extended the real OpenAI-compatible LLM HTTP seam with upstream response
  model/ID plus exact system, user and raw-response hashes. Final shared-tree
  validation is waiting for the independent sealed-market slice to leave its
  temporary TDD RED state.
- Completed BR-173 canonical A-share identity across every Gateway. Current BSE
  canonical listed-stock codes are `92xxxx`; old `43/83/87/88` values require
  official historical alias evidence and are rejected rather than guessed.
- Re-audited the pinned upstream CFFEX implementation. It has a typed diagnostic
  parser and downstream Gateway, but `calendar_capabilities().futures_delivery`
  remains false because both Rustls and native-TLS official-site probes failed.
  Production therefore remains explicitly Unsupported; a formula-derived
  delivery date is still prohibited.
- Tightened BR-172 Gate A so a real model receipt must bind the exact versioned
  system prompt, normalized user prompt, mandatory upstream response ID and raw
  response bytes before an assessment can be accepted.
- Reconciled the persistent plan against the current source tree and real
  runtime evidence instead of carrying forward stale 2026-07-25 gaps.
- Confirmed every Magic dependency is pinned to published revision
  `b2b68df78156df1d67824e5c44c0cb01b752f55a` at version `0.2.0`.
- Fixed the lifecycle evidence parser to accept the exact timestamp formats
  actually declared by Magic TDX (RFC3339 or Unix seconds with optional exact
  fractional seconds). All 15 lifecycle tests pass.
- Fixed `confirm_daily_change` so review mode initializes the acquisition-audit
  database before asking the Gateway for pending BR-171 evidence. Its five CLI
  tests and strict Clippy pass.
- Re-ran the two previously blocked histories. Both now reach the intended
  fail-closed manual confirmation boundary, with exact evidence tokens:
  688548 (`38.74→46.49`, `20.005162622612%`) and 688690
  (`26.18→31.44`, `20.09167303285%`). No confirmation was fabricated.
- Backfilled 31 of 33 configured securities through 2026-07-27. The repository
  freshness script passes at one trading day behind; the two BR-171 histories
  remain individually incomplete until explicit operator confirmation.
- Confirmed the strict review path exits successfully and delivers R-03,
  A-10 and R-08 with truthful missing-field rendering.
- Confirmed old acquisition modules have been deleted and a source-level
  `unified_data_architecture` regression gate exists.
- Replaced stale direct-Sina startup text and source comments with the actual
  `GlobalNewsGateway` / `SinaInstrumentNewsGateway` ownership, and corrected
  BR-066 to prohibit consumer-owned news protocols.
- Traced the static industry-chain registry to three production consumers and
  confirmed the real Magic TDX board-membership Gateway is available. The
  replacement needs provenance-bearing persistence; deleting only the fallback
  would stop false enrichment but would not satisfy the requested completion.
- Compared three replacement shapes and selected the already user-approved
  Magic-TDX-primary design: asynchronous verified prefetch plus transactional
  provenance persistence. Per-read network access and NULL-only retirement were
  rejected because they respectively harm risk-path availability and fail the
  requested enrichment outcome.
- Reserved BR-170 for the deterministic, provenance-bearing position-chain
  assignment and legacy static-value retirement rule.
- Wrote the Gate-A design
  `docs/superpowers/specs/2026-07-27-position-chain-magic-tdx-design.md`.
  Revised BR-085/BR-123 and registered BR-170 before implementation; the
  contract selects from complete Magic TDX memberships and persists an
  append-only assignment linked to the current position projection.
- Self-reviewed the BR-170 design for placeholders, contradictions and
  ambiguity, then wrote the executable TDD plan
  `docs/superpowers/plans/2026-07-27-position-chain-magic-tdx.md`.
- Corrected the TDD skill path from the absent user-level location to the
  repository-owned `.agents/skills/tdd/SKILL.md`. Adopted its deep-module
  guidance: callers and tests will use one `PositionChainGateway` interface;
  normalization, selection, hashing and persistence details stay behind it.
- Identified the remaining cleanup boundary: stale startup/source comments,
  dead configuration review, the production static industry-chain registry,
  NewsAI evidence restoration, full gates/coverage/live evidence and PR merge.

### Remaining External Blocker
- CFFEX futures delivery cannot be enabled until the upstream provider passes a
  real official-source live-admission gate. The downstream Gateway remains
  explicitly unsupported by design.

### Validation Notes
- The first `cargo fmt --all -- --check` requested two `log::info!` calls be
  collapsed to one line. `cargo fmt --all` applied the mechanical correction;
  no semantic or architecture failure was observed.
- `cargo fmt --all -- --check`: PASS.
- `git diff --check`: PASS.
- `cargo test --test unified_data_architecture -- --nocapture`: 5/5 PASS,
  including legacy acquisition, QMT parser, Jin10 calendar and direct-host
  ownership guards.

## Session: 2026-07-23

### Current Status
- **Phase:** 3 - Upstream Provider Slices
- **Started:** 2026-07-23

### Actions Taken
- Created isolated migration plan and preserved the previous planning task.
- Read brainstorming, planning-with-files and codebase-design instructions.
- Confirmed stock_analysis only directly imports magic-tdx-rs today.
- Confirmed magic-market-data-rs has added real public-intelligence provider crates, but capability-by-capability verification is still in progress.
- Completed source-level capability matrix for market, financial, research, capital, news, announcement and signal providers.
- Confirmed the new public-intelligence crates are still untracked upstream work and therefore are not yet safe production dependencies.
- Mapped the principal duplicated acquisition modules in stock_analysis and identified the mixed market/enrichment `KlineData` model as a migration boundary.
- Paused implementation at the brainstorming design gate.
- User approved staged domain migration and selected the source policy: migrate unique high-evidence news sources into Magic, remove duplicate/stale sources.
- User approved architecture section 1: Magic owns acquisition/contracts/evidence/routing; stock_analysis owns scheduling/persistence/decisions and accesses providers only through four deep data-gateway modules.
- User approved design section 2: complete-batch routing, evidence-preserving downstream joins, shadow dual-read migration and the five-stage cutover order.
- User approved design section 3: typed failures, strict degradation boundaries, async-only gateway execution and batch-level log governance.
- User approved design section 4: upstream/downstream gates, live probes, monitor validation and old-module disposition.
- Wrote and self-reviewed the overall design in isolated branch `feat/magic-data-gateway-design`.
- Committed the design as `8fe06b0` without touching the dirty event-selection worktree.
- User confirmed the written design and authorized progression to implementation planning.
- Wrote and self-reviewed the Slice 0 implementation plan in isolated upstream branch `docs/magic-data-slice-0-plan`.
- Added explicit upstream business-rule registration, uniform `0.2.0` version enforcement, all-feature preflight, 80%/95% coverage, bounded real probes, release packaging and complete PR evidence.
- Committed the Slice 0 plan as `b758ee309bf63909a904222b0c07b88bf2df609d`.
- User reaffirmed that legacy financial/news acquisition must be deleted after
  verified domain cutover; recorded as a mandatory Slice 4 cleanup gate.
- Imported the user-confirmed 2026-07-24 15:01 live screenshot as immutable
  `user_position_snapshot` batch 4 (7 positions) and
  `user_account_summary` batch 4. Recomputed market value is exactly
  CNY 56,514.00 and SQLite integrity is `ok`.
- Preserved the unexplained CNY 3,605.14 account-display difference in ignored
  local evidence and deliberately did not write `real_account_snapshot`.
- Traced the production `--review` dispatcher and confirmed every active
  external-data review task still uses local acquisition code. This is the
  immediate downstream cutover target after the upstream release baseline.

### Test Results
| Test | Expected | Actual | Status |
|------|----------|--------|--------|
| Current selection config uniqueness | No duplicate chain ID | 2 targeted tests passed in clean tree | PASS |
| Current nested-runtime regression | CurrentThread path awaits without block_on | 2 targeted tests passed in clean tree | PASS |

### Errors
| Error | Resolution |
|-------|------------|
| Root compile failed in uncommitted `magic_tdx_t0.rs` | Used clean validation tree; unrelated user draft preserved |
| SIGINT exposed R-03 timer polling after runtime shutdown | Root cause recorded; migration/adjacent repair design must use cancellable async access |

## Session: 2026-07-24 continuation

### Current Status
- **Phase:** 3 — upstream release baseline and current-session freshness gate

### Actions Taken
- Re-read repository process rules and the approved unified Gateway design.
- Independently confirmed the downstream Gateway has not been implemented and
  `--review`, financial/news acquisition and NewsAI still use old or disabled
  paths.
- Completed upstream fmt, strict clippy, full tests, compliance and docs checks.
- Ran the real TDX probe and found cached Friday current-trade payloads admitted
  on Saturday; returned the upstream slice to Gate A instead of publishing it.
- Fixed local `monitor --test` audit/database namespace and repeatable ledger
  seeding defects; the command now exits zero, while remaining log/trade
  isolation defects are recorded as open rather than hidden.
- Added a process-level repeat-run regression for recommendation/trade
  persistence. It first failed because recommendations used the production
  namespace, then passed after environment-scoped recommendation paths and a
  strict versioned trade fixture were implemented.
- Bound `MAGICLAW_DB_PATH` to the selected isolated database in test mode.
  A repository `.env` containing production database, sink database and real
  watch-list defaults is now ignored by the isolated E2E path with a visible
  BR-051 startup decision.

### Gate State
- Gate A: active for TDX current-session admission and downstream slice mapping.
- Gate B: upstream Provider set implemented; freshness blocker open.
- Gate C: upstream static gates pass before the latest freshness repair.
- Gate D: blocked by failed live probe and incomplete downstream production cutover.

### Focused Test Evidence
- `cargo test --test monitor_help_isolation bare_test_alias_reaches_the_final_completion_marker -- --nocapture`
  - RED: production `data/d01_recommendations` was created.
  - GREEN: two consecutive E2E runs passed; only the test recommendation
    namespace exists and `TEST_CODE_TRADE_V2` count remains exactly two.
- `cargo test --test monitor_help_isolation test_mode_ignores_the_repository_dotenv_production_database_default -- --nocapture`
  - RED: required Magiclaw isolation decision was absent.
  - GREEN: production dotenv database/watch/sink defaults stayed outside test
    state and the process reported the isolated sink binding.

## Session: 2026-07-25 continuation

### Current Status
- **Phase:** 3 — upstream release baseline, final gates and all-market signal completion

### Actions Taken
- Restored the persistent plan and current working-tree evidence.
- Confirmed the upstream TDX current-session admission repair is implemented:
  normalized current minute/trade operations now fail closed before transport
  on weekends and off-session windows; explicit historical-date requests keep
  their historical route.
- Confirmed the downstream A-01/R-03 Gateway slice completed its focused
  design/BR registration and entered TDD RED with every production caller
  enumerated.
- Confirmed the R-04 all-market Dragon-Tiger audit found source `TRADE_ID`;
  distinct same-day reasons must be retained and exact duplicates alone may
  be collapsed.
- Completed the Gate-A draft for the next A-10 vertical slice:
  `docs/superpowers/specs/2026-07-25-chain-intelligence-gateway-a10-design.md`.
  It moves same-date chain production under the existing post-session owner,
  replaces the evidence-poor `chain_daily` cache with immutable typed batches,
  and records the missing upstream board catalog/constituent contracts.

### Test Results
| Test | Expected | Actual | Status |
|------|----------|--------|--------|
| `cargo test -p magic-tdx-rs --lib` | Current-session gates and existing adapter contract pass | 278/278 PASS | PASS |
| Magic TDX live-probe tests | Diagnostic raw cache stays unadmitted and normalized path fails closed | 2/2 PASS | PASS |
| Magic TDX strict clippy | No warnings across targets/features | PASS | PASS |
| Real Magic TDX weekend live probe | Cached raw trades diagnostic only; normalized minute/trades rejected | `live_probe_status=passed` | PASS |

### Capability Gap Audit
- Re-audited current Core/Router traits, Provider implementations, READMEs and
  explicit `Unsupported` paths.
- Separated nine genuine upstream gaps from already implemented families that
  are merely not wired through stock_analysis yet.

### 2026-07-25 Validation Update
- Re-ran the complete locked/offline `magic-tdx-rs` package suite against the
  isolated release worktree after the current-session repair.
- Result: 278 library tests, 3 adapter integration tests, 3 capability tests,
  fuzz/golden/protocol/service suites and doc tests all passed.
- Re-ran strict all-target/all-feature Clippy with `-D warnings`; it passed.
- Re-ran the bounded real TDX probe on Saturday 2026-07-25. Daily bars carried
  source date 2026-07-24; raw cached minute/trade packets were labelled
  `diagnostic_unadmitted_weekend`; normalized current minute/trade and
  pagination requests failed closed; the probe ended
  `live_probe_status=passed`.
- Reviewed the first downstream Gateway implementation and found it duplicated
  the upstream `SecurityBar -> core::Bar` normalization. The release worktree
  already exposes `TdxSmartClient: HistoricalBars<Bar = core::Bar>`, so the
  duplicate downstream adapter was rejected and is being removed before GREEN.
- Confirmed the A-01/R-03 migration must be compiled against the release
  worktree first, then restored to the adjacent published `magic-market-data-rs`
  path after the upstream branch is merged.
- Added the BR-159 acquisition-audit persistence boundary:
  `data_acquisition_audit` and its immutable SHA-256 chain retain capability,
  provider/source, request hash, provider/local time, batch ID, typed outcome
  and aggregate counters. Append uses an SQLite `IMMEDIATE` transaction,
  validates the already startup-verified chain tail, and reports provider
  outcome transitions without an O(n) scan per request.
- Focused acquisition-audit tests first passed 3/3 for atomic append, mandatory
  success batch ID and tamper detection. A fourth tail-tamper regression is
  queued behind the concurrent full-workspace Cargo lock.

### 2026-07-25 Review-Path Investigation
- User reproduced that `monitor --review` still emits legacy acquisition
  behavior.
- Re-established Gate A and the mandatory diagnostic loop before editing
  production code.
- Current leading fact: Phase 4 caller migration is pending; next step is a
  deterministic call-path audit that fails while any active review task
  references a legacy loader or direct HTTP provider.
- Call-path audit update: A-01 and R-03 are wired to the new Gateway in the
  dirty worktree and Cargo resolves Magic `0.2.0` crates from the isolated
  upstream release tree. The runtime is currently mixed: R-04/R-08/A-10 still
  need exact tracing and regression coverage.
- Exact tracing completed: R-04 calls the local LHB loader; R-08 calls local
  announcement and Yahoo loaders; A-10 reads the local chain cache. This
  reproduces the user's symptom deterministically at source level.

### 2026-07-25 A-01/R-03 Gateway Audit Gate
- Completed the A-01/R-03 public Gateway cutover contract against the isolated
  upstream release worktree.
- Every admitted, verified-empty or rejected acquisition now attempts an
  immutable BR-159 database append before the Gateway returns. Audit failure
  fails the Gateway call closed instead of silently losing evidence.
- Provider errors retain typed outcomes (`invalid_request`, `unavailable`,
  `partial`, `unsupported`), reason codes and retryability; a verified empty
  provider batch remains distinct from unavailability.
- The TDX path now consumes the upstream canonical `core::Bar` batch directly
  and routes the already-acquired batch without issuing a duplicate request.
- Validation evidence:
  - `cargo check --lib --locked --offline`: PASS against
    `target/magic_market_unified_work`;
  - `cargo test --lib data_gateway::review::tests --locked --offline`: 3/3 PASS;
  - `cargo test --lib database::data_acquisition_audit::tests --locked --offline`:
    4/4 PASS, including tail tamper rejection.
- Restored committed dependency paths to the adjacent upstream repository;
  final adjacent-path validation remains contingent on merging the upstream
  release baseline.

### 2026-07-25 R-04 Upstream Completion
- Completed the typed all-market Dragon-Tiger discovery and exact `TRADE_ID`
  seat query in Core, Eastmoney and Router.
- The identity rule preserves distinct same-stock/day reasons and folds only
  rows with the same side, seat name and all numeric facts.
- Real 2026-07-22 probe admitted five source records, each with five buy and
  five sell seats. Focused tests, strict Clippy, rustdoc, compliance, scoped
  formatting and `git diff --check` passed.
- Downstream R-04 Gateway aggregation and legacy-loader deletion remain open.

### 2026-07-25 Sina Instrument-News Upstream Completion
- Added the official Sina per-instrument news Provider with exact exchange/code
  identity, GB18030 decoding, bounded pagination, canonical URL identity,
  provider publication time, stable dedup/conflict rejection and
  provider-proven filtered-empty semantics.
- Real release probes admitted three complete records for both `600396.SH` and
  `000001.SZ`; Beijing/global remain explicit `Unsupported`.
- Package format, 44 tests, strict Clippy, docs links, compliance and
  `git diff --check` passed. Downstream exact-caller migration is in progress.

### 2026-07-25 R-08 Gate A
- Registered BR-161 and added
  `docs/superpowers/specs/2026-07-25-event-calendar-gateway-r08-design.md`.
- Confirmed R-08 needs two genuine upstream additions before its old acquisition
  can be fully deleted: full-market announcement discovery and a typed global
  index/FX snapshot. Existing per-instrument announcements must not be
  mislabeled as a full-market batch.
- Verified broker positions remain a separate real-account capability; a local
  projection/update time cannot become broker evidence.

### 2026-07-26 Legacy Acquisition Reproduction
- Built a deterministic source-level feedback loop for the reported symptom.
  It finds ten production legacy acquisition calls across DataFetchService,
  opportunity rescoring, multi-timeframe analysis, market overview, the
  announcement monitor and v17 earnings.
- Independently traced strict `--review`: R-03/A-01/R-04/R-08/A-10 now use
  unified Gateway entry points. The repository remains mixed because normal
  production paths are not covered by the current architecture guard.
- Confirmed all Magic crates are pinned to one `0.2.0` revision. The issue is
  incomplete downstream caller deletion, not a duplicate dependency version.
- Attempted a dry-run `--review`; it remained blocked on the shared Cargo
  artifact lock while unrelated test/check builds were active, so the queued
  invocation was cancelled without terminating those processes.

### 2026-07-25 A-10 Immutable Store Gate B
- Registered BR-160 before implementation and aligned the design with an
  authoritative visibility-receipt table.
- Added append-only `chain_intelligence_*` batch, input evidence, chain, member,
  rejection and visibility schemas. All tables reject UPDATE/DELETE.
- Staging validates the minimum three-member rule, stable input/chain/member
  ordinals, exact deterministic ordering, unique identities and lowercase
  content hashes before one SQLite `IMMEDIATE` transaction.
- A staged batch is not query-visible until a receipt binds its content hash to
  an authoritative audit-record hash. Repeated identical staging/publishing is
  idempotent; same identity with different content is `Conflict`.
- Validation evidence:
  - `cargo test --lib database::chain_intelligence::tests --locked --offline`:
    4/4 PASS;
  - `cargo clippy --lib --locked --offline -- -D warnings`: PASS against the
    isolated upstream release worktree.

### 2026-07-27 BR-170 Position-Chain Gate B (in progress)
- Added the pure Magic TDX board-membership assignment seam in
  `data_gateway::position_chain`.
- The first TDD cycle now passes: an Industry membership is selected before a
  Concept membership, while the complete accepted membership set and exact
  provider evidence remain in the assignment content hash.
- Validation evidence:
  - `cargo test --lib data_gateway::position_chain::tests::industry_membership_is_the_primary_position_chain -- --nocapture`:
    1/1 PASS.
- Conflict, verified-empty and malformed-evidence paths are the next focused
  TDD cycles before persistence is allowed to begin.
- Deterministic assignment is now complete: 10/10 focused tests pass, including
  Concept fallback, exact-duplicate folding, conflict rejection, source
  evidence validation and test/live symbol isolation.
- Append-only persistence is complete: 5/5 focused tests pass. Assignment
  insert and open-position linkage are one `IMMEDIATE` transaction; identical
  replay is idempotent, conflicting content is rejected, UPDATE/DELETE are
  blocked, missing positions roll back, and migration clears unlinked legacy
  chain names.
- BR-170 refresh orchestration is complete: exact codes are deduplicated and
  sorted, at most four Magic TDX membership calls run concurrently, outcomes
  return to stable code order, and one failed code does not suppress another
  code's assignment.
- `--backfill-chain-name` now calls the unified refresh and returns nonzero on
  any per-code failure. Its isolated terminal-handler test passes.
- Returned BR-170 to Gate A for one integration refinement: the existing
  candidate path requires a linked position before it can determine chain
  exposure, but the position does not exist until the order is accepted.
  Pre-flight now requires an async candidate assignment followed by one atomic
  order-audit/position/assignment/link transaction. No implementation or
  completion claim will use raw `chain_name` as a substitute.
- Completed the Gate-A refinement and returned BR-170 to Gate B. New candidates
  acquire a complete Magic TDX assignment before synchronous concentration
  sizing; `OpenPositionCmd` carries the complete assignment, and one SQLite
  transaction commits or rolls back the order audit/hash, position, immutable
  assignment and projection link.
- Added the long-running startup owner: all existing open-position codes are
  refreshed before any report scheduler, event consumer, news loop, market
  loop or paper engine starts. Failure to load the position set blocks startup;
  per-code provider failures stay explicit and isolated while successful codes
  commit.
- TDD note: the first source-order test falsely passed because its `split`
  matched the test's own marker. Replacing it with `rsplit_once` produced the
  intended RED (missing startup refresh), after which the production wiring
  made the same test GREEN.
- Focused validation:
  - `cargo test --bin monitor position_chain -- --nocapture`: 1/1 PASS.
  - `cargo test --lib pipeline::position_tracker -- --nocapture`: 10/10 PASS.
  - `cargo test --lib portfolio -- --nocapture`: 18/18 PASS.
  - `cargo test --lib br170_ -- --nocapture`: 2/2 PASS.
  - `cargo test --test unified_data_architecture br170 -- --nocapture`: 1/1 PASS.
  - `cargo check --bin monitor`: PASS.

### 2026-07-27 Final-Cutover Cleanup Audit
- Re-read the repository pre-flight documents and audited production source,
  startup text, `.env.example`, `config/`, the integration README and the
  architecture guard for superseded RustDX, BaoStock, local Sina/Tencent/
  Eastmoney K-line fallbacks and Yahoo acquisition.
- No production acquisition or stale startup/config reference remained. The
  only text match was the intentional R-08 regression-test name that forbids
  reintroducing legacy announcement/Yahoo acquisition.
- `cargo test --test unified_data_architecture -- --nocapture`: 6/6 PASS,
  covering Gateway-only financial/news acquisition, deleted legacy entry
  points, deleted QMT parser, deleted local LHB/news facades, deleted Jin10
  calendar protocol and deleted static position-chain fallback.
- `cargo check --bin monitor`: PASS.
- One read-only investigation script initially failed to parse because of a
  malformed JavaScript object key. It performed no command and changed no
  files; the corrected calls completed normally.

### 2026-07-27 R-08 Global-Market Gateway
- Added the typed `GlobalMarketGateway` over the pinned Magic Sina global-index
  and foreign-exchange providers.
- US indices and USD/CNY are independent R-08 components. A failed index batch
  no longer erases a valid FX batch.
- The current Sina index packet has no provider `source_at`; it is therefore
  rejected explicitly instead of receiving a fabricated timestamp. Fresh
  USD/CNY packets retain provider source time and pass the five-second gate.
- Focused validation:
  - `cargo test --lib data_gateway::global_market::tests -- --nocapture`:
    3/3 PASS.
  - `cargo test --bin monitor tests_br140_r08_partial_components -- --nocapture`:
    5/5 PASS.
  - `cargo test --test unified_data_architecture -- --nocapture`: 6/6 PASS.
  - `cargo check --bin monitor`: PASS.
  - `cargo fmt --all -- --check`: PASS.
  - scoped `git diff --check`: PASS.

### 2026-07-27 Parallel Gate-B Completion Update
- Applied the user's parallelism policy: inspect dependency and write conflicts
  first, then run only disjoint file sets concurrently. Shared schema/modules,
  final Cargo gates and Git integration remain serialized.
- Search/opportunity evidence preservation completed:
  `FreshFlashFact` retains record and batch evidence; stale/future/mismatched
  facts fail closed; same-provider multiple batches do not count as
  cross-source confirmation. Focused search, push-gate, chain-evidence and
  service tests passed 20/20.
- Chain-analysis optional supplements now preserve
  Available/VerifiedEmpty/Unavailable/NotRequested with exact evidence and no
  longer erase a complete core batch. Focused module/fetcher/Gate-D tests
  passed 37/37; library check and strict library Clippy passed.
- BR-171 lifecycle and manual-admission internals passed focused verification:
  security lifecycle 13/13, append-only exact confirmation ledger 7/7 and
  HistoricalBars integration 2/2.
- R-02/R-05/R-06 were independently audited and remain explicitly Disabled:
  their required settled full-market, end-to-end closed-outcome and versioned
  failure-attribution batches do not exist. Enabling them would violate
  AGENTS 2.1/2.2/2.4/2.7/2.8.
- BR-171 static lifecycle-cache deletion guard was observed RED on
  `IPO_DATES`, then GREEN after removing the process-global IPO/ex-right
  writers, readers and their dead limit-status facade. Lifecycle facts may now
  enter daily-bar admission only through the evidence-bearing Gateway and
  immutable exact-confirmation ledger.

### 2026-07-27 BR-171 Operator and Cleanup Update
- Added the explicit `confirm_daily_change` operator CLI. Review mode is
  read-only and emits exact JSONL evidence plus a domain-separated SHA-256
  token. Confirmation requires explicit dates, token, database, operator and
  reason, reacquires the source/lifecycle batches, rejects changed evidence
  and appends only to the immutable confirmation ledger.
- `HistoricalBarsGateway::pending_daily_change_confirmations_async` exposes
  only exact pending queries and cannot construct an admitted daily-bar batch
  or write a confirmation.
- Removed the remaining process-global IPO/ex-right confirmation caches and
  documented the manual workflow in README and the integration guide.
- Reconciled R-02/R-05/R-06 and CFFEX with real capability state. They remain
  explicit Disabled/Unsupported outcomes; no partial snapshot, heuristic or
  formula is promoted to a complete source contract.
- Updated active runtime environment documentation and corrected the ignored
  Magic BarsRouter live test to expect the fixed upstream revision's
  unadjusted daily bars, strict provider labels and unique ascending dates.
- The chain-analysis supplement seam was deepened without behavior changes;
  focused tests passed 37/37 and strict library Clippy passed.
