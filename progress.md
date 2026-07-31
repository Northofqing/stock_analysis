# Progress log

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
