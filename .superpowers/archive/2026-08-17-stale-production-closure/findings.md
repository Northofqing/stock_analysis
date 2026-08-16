# Findings

## 2026-08-03 P2-F test/runtime findings

- The monitor unit binary is now green in both default-thread and serial
  execution (`509 passed, 0 failed, 4 ignored`). The earlier parallel-only
  failures were not flaky business assertions: secure descriptor validation
  observed shared `data/test` ancestor link-count changes from sibling test
  namespaces. One serial domain for all such mutators is required; weakening
  the link-count check would hide a real path-rebinding risk.
- `monitor --test` had an independent startup-order bug. `main` set
  `STOCK_ENV_MODE=test` and `V10_DRY_RUN_PUSH=1`, then called BR-144 audit
  preflight before creating `DURABLE_DELIVERY_TEST_CODE`. The exact integration
  suite proved the resulting fail-closed error. The candidate now installs or
  validates a unique path-safe TEST_CODE before preflight; all 19 integration
  cases pass and production continues to reject test dry-run authority.
- The ignored candidate must not be confused with the broad root dependency
  target. Candidate `Cargo.toml`/lock still resolve only two sibling-path Magic
  packages and Polars 0.46 plus qmt-parser-induced 0.52. It has no unified
  `src/data_gateway` cutover, and roughly 77 Rust files still reference the old
  provider layer. BR-164 is therefore a real remaining implementation stage,
  not documentation cleanup.
- Current Gate-A documents have drifted beyond their previously frozen hashes,
  P2 destination hashes/checker coverage are incomplete, and the ignored-test
  acceptance text does not match the actual workspace inventory. These must be
  refrozen and independently reviewed before candidate materialization.
- The post-repair locked full-workspace serial test exits zero. This proves the
  candidate is internally compile/test coherent, but does not make it a Gate-B
  artifact because `target/br203-candidate` is ignored and unreachable from
  Git. Gate A refreeze and a reachable clean commit remain mandatory.
- All-target/all-feature Gate C must run outside the managed filesystem sandbox
  because 34 deterministic HTTP/TCP fixture tests bind loopback listeners. A
  sandbox `Operation not permitted` is infrastructure evidence, not a product
  assertion failure; the same exact argv outside the sandbox passes. This does
  not authorize live external transport or weaken any data-source failure.
- The actual workspace ignore inventory is sixteen exact names: six
  parent-invoked process helpers plus ten opt-in live/external integrations.
  Release tooling must whitelist these names exactly instead of claiming that
  the notify child is the only ignored test.

## 2026-08-03 BR-164/BR-203 recovery re-anchor

- `Cargo.toml` already declares root Polars `0.54`; the current lock resolves
  the Polars implementation family to `0.54.4`. The old 0.46/0.52/qmt-parser
  diagnosis belonged to an isolated stale candidate and is not current-tree
  evidence.
- The complete shared worktree passes locked library/monitor checks and the
  15-case unified-data architecture suite, but fixed parent `96da674` does not
  contain that broad provider/caller closure. Therefore those green commands
  cannot prove a narrow Cargo-only BR-203 child commit.
- Independent architecture review requires BR-164 to be a separately owned,
  compile-green predecessor. BR-203 must then use a docs-only exact re-anchor
  before counted-delivery recovery begins.
- The physically reliable offline metadata probe is
  `cargo metadata --locked --offline --format-version 1 --no-deps`. Complete
  dependency identity must be proved by structural TOML parsing plus accepted
  whole-file hashes; full offline metadata can fail only because a non-host
  target crate is absent from the local cache.

## 2026-08-02 BR-196/BR-201/BR-202 formal gate findings

- Second-round formal reviews are already disproving parts of all three repair
  reports before final grading. BR-196's claimed 36 current real chains still
  overcounts at least eleven retained shapes with no non-test upstream caller.
  BR-201 still has candidate contradictions in closed reason codes, Claimed
  projection ownership/takeover, fixed `old+1` recovery generation and BR-086
  audit insertion. BR-202 still has candidate gaps in raw CI invocation and
  isolated artifact retention/cleanup. These remain provisional until each
  reviewer returns exact C/I/M evidence, but no corresponding Gate may advance.
- BR-196's final second-round grade is C2/I2/M0. Exact additional no-caller
  examples are T-08, I-09 SectorTop/SectorAnomaly, R-03, T-14/T-15, BR-033/
  BR-034, S-02 and N-01/N-02. More importantly, typed token acquisition inside
  an unreachable wrapper remains dead code, so a registry↔manifest bijection
  is not independent reachability evidence. The repaired model must either
  provide an actually enforceable source-root/runtime evidence boundary or
  conservatively disable every unproved shape with an exact startup banner.

- The shared worktree is intentionally broad: the current tracked diff spans
  231 paths with 40,329 insertions and 55,196 deletions, plus untracked staged
  migration artifacts. `git diff --check` passes. This is not authority to
  reset, delete, or overwrite unrelated user work; parallel changes remain
  partitioned by design path and business-rule row ownership.

- BR-196's implemented registry cannot yet be accepted as a closed production
  template manifest. Direct caller tracing supports at most 36 registered
  presentation chains with real production callers: the typed
  `I-01-intraday-market` wrapper has no upstream caller while a different
  board-flow shape is unregistered, and `T-17-etf-closing-call-auction` has no
  producer and is explicitly disabled. Other false-active descriptors and two
  missing production shapes remain to be reconciled. Passing renderer/dry-run
  tests do not prove lifecycle truth.
- Independent root tracing reproduced those two BR-196 facts. The only
  non-test market-view producer at `main.rs` renders
  `render_board_flow_market_view` and calls raw
  `push_governor_v3(PushKind::IntradayMarket)`; `push_intraday_market` itself
  has no upstream call. `dispatch_etf_closing_call_auction` likewise has no
  call outside its definition, while startup/runtime comments name the missing
  ETF-auction producer. Registry membership and macro token acquisition are
  therefore insufficient Active evidence for either shape.
- Current strict Clippy has one concrete BR-196 implementation blocker at
  `br196_test_delivery.rs:278`: `iter().any()` must become direct slice
  `.contains(...)`. This is a mechanical Gate B repair, not grounds to bypass
  the RED Gate A or to suppress `clippy::manual_contains`.
- The 2026-08-02 production push-log directory and event-bus JSONL file are
  both absent. This independently confirms the design/reviewer statement that
  BR-196 and BR-201 currently lack same-day production delivery evidence; no
  test/dry-run artifact can satisfy CLAUDE Completion Rule layer 4.
- BR-201's fresh formal review returned C0/I6/M0 after independently
  reproducing hashes. The six Important contradictions are canonical-record
  item count, missing
  five-second quote gates, invalid JSONL/SQLite recovery inference and state,
  incoherent reconciliation takeover CAS, the BR-134 fixed-20/manual-confirmed
  policy conflict, and a non-recomputable ordered-commit preimage. Gate B stays
  closed.
- BR-202 formal review is RED at C0/I4/M0. Its current design has one
  self-counting evidence command, an impossible same-commit self-SHA
  attestation, an exact wrapper that hashes an inventory it never creates, and
  no deterministic/hash-bound proof for the report-missing zero-instrumented
  set. The stale report omits 35 of 408 current Rust sources, so absence from a
  report cannot be silently treated as zero-instrumented authority.
- The repaired BR-202 design and unique business-rule row are byte-identical
  and pass scoped diff checks. This is not acceptance: the newly specified
  compiler/show/hash proof, isolated wrapper and attestation sequence are under
  a fresh independent formal review and remain absent from implementation.
- BR-201's second repair updated both its design and BR-134/BR-201 rows. Root
  confirmed each row occurs once and the stale contradiction phrases are
  absent. The implementer-reported rule hashes are exactly reproducible over
  the row bytes with the line terminator removed: BR-201 `b19039dd...3c87b`
  and BR-134 `46e5a120...4aed`. A raw `rg | shasum` includes the newline and is
  therefore a different preimage; the reviewer must still confirm the design
  names the no-newline preimage explicitly.
- The full-inventory stale coverage math independently reproduces as
  147,898/187,949 core lines with a 30,654-line deficit to 95%; it remains a
  diagnostic until the Gate A design and same-source-SHA tooling are accepted.
- Current coverage implementation confirms BR-202 is still Gate A only:
  `check_thresholds.py` has a 15-prefix `CORE_PREFIXES` heuristic, accepts the
  first `data` run without exact cardinality/schema/sum reconciliation, and has
  no complete source registry, zero-instrumented proof, dep-info/show binding,
  fixed-SHA wrapper or artifact hashes. Its test file contains only three basic
  prefix/threshold cases. Gate B must replace this shallow seam rather than
  treating the repaired design text as implemented.
- Business-rule compliance independently exposes the same implementation gap:
  BR-202 names a wrapper that does not yet exist and its current test lacks the
  rule citation; BR-196's existing allowlist file lacks its rule citation.
  These are blocking Gate C facts, while the checker's 124 other messages are
  historical warnings rather than the three hard failures.

## 2026-08-01 independent review status

- Two independent review turns failed at the Codex response backend with HTTP
  403 before producing evidence. This is review-infrastructure failure, not
  evidence that BR-192 or BR-194 passed or failed their technical contracts.
- The safe response is to retain both Gates as open, retry with fresh reviewer
  instances, and continue only read-only/design work on independent paths.
- BR-193's current Gate A failure is substantive: rollback approvals cannot be
  pre-created, recovery CLI success cannot depend on post-migration admitted
  rows, BR-194/push/durable-delivery cannot be prerequisites, and scheduler
  ownership plus AC/log contracts are not yet implementable as written.
- The current exact-staged BR-192 design contains the five enabled seam
  evidence blocks and explicitly says enabled metadata is not Gate D evidence.
  Its index hashes and diff check are internally consistent, so the remaining
  Gate A uncertainty is independent reviewer acceptance rather than index drift.
- Direct BR-193 source inspection confirms the reviewer findings are textually
  present in §13: lines 3917-3918 pre-list rollback approval artifacts, lines
  4018-4022 freeze an unimplementable borrowed `CadenceOwner` signature, lines
  4036-4050 defer the CLI contract while reusing Gate-D output, and lines
  4064-4067 depend on push/BR-194/durable delivery outside BR-193 scope.
- The §13.3 claim that 47 failures share one root cause has no pasted command
  output and conflicts with the current planning evidence that most failures
  remain uninvestigated. The revision must classify actual failures instead of
  preserving a convenient aggregate claim.
- BR-194 production-evidence check for 2026-08-01 found no ReviewLhb or
  ReviewProviderTopN push-log file and no matching event-bus entry. Production
  call sites do exist for R-04 and R-09, but Gate D replay evidence is missing;
  this must remain explicitly pending even if Gate B review becomes green.

## 2026-08-01 continuation recovery

- Current branch is `feat/event-scoped-selection-shadow` at `b4aeee6`; the
  worktree contains broad ongoing migration changes and must be treated as the
  authoritative state rather than reconstructed from prior reports.
- `AGENTS.md` 2.3 still explicitly requires alert plus manual confirmation for
  adjacent valid-value changes above +/-20%, which directly conflicts with the
  requested dynamic board/listing/corporate-action treatment. This is a Gate A
  design conflict, not a safe one-line threshold deletion.
- BR-192, BR-193 and BR-194 occupy separable paths and can be progressed in
  parallel while that design conflict is resolved.
- The prior threshold-removal spec is marked superseded solely because the
  current repository red line still freezes +/-20%; the active lifecycle spec
  therefore treats legitimate large moves as real facts but still blocks them
  from computation pending a manual confirmation.
- The released upstream Core already models source-backed `Board`, `is_st`,
  `listed_on`, and `PriceLimitRule`, with missing fields kept absent. That is
  the correct typed seam for a dynamic regime, but incomplete source metadata
  cannot be replaced by code-prefix inference or a fixed local percentage.
- Current released upstream adapters do not populate a source-backed
  `PriceLimitRule`: Magic TDX, Tencent and Sina all construct it with both
  fields absent. Magic TDX can provide listing date and a name-derived ST flag,
  but its board value is explicitly derived from exchange/code and the batch is
  marked unavailable for complete security metadata. Therefore a design that
  claims current Magic TDX alone proves the exact historical price-limit regime
  would be false.
- The unresolved design choice is whether a structurally valid bar with
  unavailable regime evidence is retained as an unpromoted raw fact, rejected
  entirely, or admitted to non-order computations. The user has been asked to
  choose; recommended behavior is raw retention plus fail-closed promotion.

## 2026-07-29 BR-192 post-delivery recovery finding

- Existing production sequencing commits the 86,400-second Global cooldown
  when the sink reports acceptance, but commits the strict-review schedule only
  after the delivered transition is appended. A post-acceptance audit failure
  therefore produces a user-visible delivery that the task state does not own.
- Reacquiring provider data is not a valid recovery: it changes observation,
  batch and decision identities and can be blocked by the already committed
  cooldown. Recovery must be a persistence-only reconcile of the original
  binding and confirmed sink receipt.
- A daily cap based only on `PushOutcome::Pushed` is not a physical-send cap:
  sink acceptance followed by audit failure returns `SinkError` and currently
  would not consume the budget. BR-192 must count or reserve the physical send
  at acceptance and make that count durable across reconcile.
- If the sink may have accepted but its receipt cannot be persisted, the state
  is uncertain delivery. Automatic retry would risk a duplicate financial
  notification, so the safe terminal behavior is fail-closed/manual recovery.

## 2026-07-29 Provider Top-N composition audit

- The dedicated composition constructor is non-forgeable at its public seam:
  it is zero-argument, constructs the production `EastmoneyClient` internally,
  exposes no caller-owned transport or generic registration, and Router remains
  Core-only.
- The exact composition tests now cover unsupported metrics with zero provider
  calls, wrong source identity, midnight rollover, clock failure and every
  public Eastmoney error class.
- Independent documentation audit found a blocking release-evidence gap: the
  recorded real probe does not print acquisition start or post-response
  `observed_at +08:00`, so it cannot prove the required post-15:35 capture
  gate.
- The Eastmoney integration page and root README still omit the new Top-N
  contract/evidence link, and the design status still says Gate A while the
  evidence claims admission.
- Core capability values and the generic validator remain publicly
  constructible, but no public composition injection path exists. Documentation
  must call them validation inputs rather than admission authority; only the
  concrete composition binding owns admission.
- The first Provider Top-N batch identity was insufficient: source/date/second
  precision observation time does not distinguish VolumeRatio from
  MainNetInflow, nor two different same-second responses. Release identity must
  also bind the request metric/limit/filter and normalized response content.
- Direct `EastmoneyClient::provider_top_n_rankings` live success is not evidence
  that the zero-argument production composition route succeeds. Gate D needs a
  bounded live probe that constructs `EastmoneyProviderTopNRankingRouter::new()`
  and routes both admitted metrics through Core revalidation and FailoverChain.

## 2026-07-29 business-first closure findings

- `cargo run --bin monitor -- --test` completed with exit 0 in an isolated
  `TEST_CODE` database. Production writes and external push/network effects
  remained disabled by BR-051; all eight template pushes were audited as
  dry-run outcomes.
- `cargo run --bin monitor -- --review` completed with exit 0 and
  `failed=0`: two real deliveries, two explicit no-data outcomes, one
  time-window wait and three explicitly disabled capabilities. This proves the
  review dispatcher is operational, but does not prove the disabled
  capabilities or every degraded component are complete.
- CFFEX delivery is not a downstream routing omission. The immutable upstream
  revision `660902ff93a07f18367dc16879cf67732accd25a`, upstream remote `main`
  and the local upstream HEAD all expose
  `calendar_capabilities().futures_delivery == false`; the production method
  returns `Unsupported`. The recorded official-HTTPS live admission is
  `failed_transport`. HTTP diagnostic output cannot be admitted as production
  evidence and an unavailable result cannot be relabelled verified-empty.
- The remaining macOS GlobalSchema owner failure is caused by SQLite opening
  `/dev/fd/<database-fd>` in WAL mode: the VFS derives a nonexistent
  `/dev/fd/<database-fd>-wal`. The descriptor pool already uses the correct
  retained-parent route. The owner inspection and rehearsal path now reuse
  that route and revalidate the exact inode before and after opening.
- All 13 downstream Magic dependencies in `Cargo.toml` and their lockfile
  packages resolve to the same `=0.2.0` source revision
  `660902ff93a07f18367dc16879cf67732accd25a`. The direct Polars graph is
  singular at 0.54 and `qmt-parser` is absent.
- Authoritative GitHub evidence confirms upstream PR
  `Northofqing/magic-market-data-rs#1` merged into `main` at merge commit
  `13b0172b436b43616d1f3969314dbb83e6d2facd` with all reported CI, audit and
  coverage checks successful. The upstream repository's current
  `origin/main` is `660902ff93a07f18367dc16879cf67732accd25a`, exactly the
  immutable revision consumed downstream.
- The downstream repository has no verified PR/merge evidence yet. The first
  GitHub PR-list query failed at the API connection boundary, so the local
  branch and dirty worktree must not be described as merged.
- The rewritten README already contains the required Gateway matrix, separates
  Tencent/Sina identity from Magic TDX lifecycle evidence, scopes
  `DATABASE_PATH` versus `STOCK_DB`, describes QMT only as an unimplemented
  broker boundary, and labels CFFEX plus the other unsupported capabilities
  honestly. Active configuration/source searches no longer find
  `opportunity_scan_interval_min` or deleted TOML filenames.
- BR-181's design document still labels revision 3 as “implementation
  prohibited until a fresh independent review” even though its implementation
  is present. Gate A review evidence/status must be reconciled before this
  slice can count as release-ready.
- The BR-181/BR-164 static deletion checks currently show: zero active stale
  runtime-input keys/filenames, exactly 13 identical Magic revision pins, zero
  RustDX references, and no `qmt-parser` dependency/reference outside the
  architecture regression that forbids it. The surviving `data_provider`
  files are limited to chip distribution, consensus, halt/limit status,
  data-only news items and the service facade; surviving search providers are
  only the unified general-web boundary and module index. Their business
  ownership still requires the BR-164 audit rather than assuming every
  remaining filename is obsolete.
- The normal monitor still has a business-availability defect in its paper
  exit loop. `PAPER_ENGINE_LAST_RUN` advances only after a fully successful
  batch, while a single pre-close paper position with an unavailable/stale
  quote returns from `rebuild_open_positions` and aborts all other positions.
  The outer 30-second monitor loop therefore retries immediately and emits the
  BR-134 warning repeatedly. After close, missing quotes are already isolated
  per position, which demonstrates the intended partial-failure seam. Any
  repair must preserve paper-only scope and may not fabricate an execution
  quote.
- R-02/R-05/R-06 cannot be truthfully re-enabled from existing data. R-02
  lacks one review-date/batch-complete market overview (three indices,
  turnover, full breadth and limit identities); R-05 lacks append-only
  signal→confirmed-delivery→execution/non-execution→settlement lineage; R-06
  additionally lacks evidence-bound, versioned classification. The current
  partial market snapshot, generic paper/execution rows and renderer-only
  failure attribution are not substitutes. Disabled paths now return before
  partial acquisition and name the exact missing contract.
- R-08 global-market degradation has two verified causes. The fixed Magic Sina
  index packets contain exactly four value fields and expose no provider
  `source_at`; their observation time is local post-response time and cannot
  substitute. USD/CNY does expose provider time, but live evidence shows the
  packet is roughly minute-level, so a 5-second realtime gate correctly marks
  it stale. The consumer then mislabels these realtime snapshot contracts as
  “overnight” facts. The safe correction is a typed “latest completed global
  session/daily close” upstream contract (or a genuinely ≤5s timestamped
  provider if the product is renamed realtime), not a weaker freshness gate.
- The macOS retained-parent SQLite route fixed the original
  `BEGIN IMMEDIATE CannotOpen(14)` failure. The exact owner regression now
  reaches receipt reconciliation and fails on an audit namespace container
  mutation in its TEST_CODE fixture. This is stronger evidence that the VFS
  route is fixed, but the owner gate remains red until the deeper marker
  lifecycle failure is resolved without weakening production attestation.

## Upstream status

- Branch: `feat/unified-data-release`
- Worktree:
  `target/magic_market_unified_work`
- Upstream PR: `https://github.com/Northofqing/magic-market-data-rs/pull/1`
- Functional gates already observed passing: workspace check, strict clippy,
  full tests, fmt, compliance, docs links, and diff check.
- Latest coverage artifact:
  `target/magic_market_unified_work/target/coverage/coverage.json`
- Overall coverage: 82.85% (passes 80%).
- Critical coverage: 95.17% (passes 95%).
- Largest deterministic opportunities are core calendar/global/policy/research/
  signals validation, router contract branches, CNInfo pagination validation,
  and TDX service facade/error branches.
- Core calendar/global/policy already use path-based test modules, so their
  currently uncovered serde/accessor/bounds branches can be covered without
  adding forbidden inline source test bodies.
- Existing research and signals integration fixtures already construct valid
  source evidence, so semantic bypass tests can be extended cheaply and remain
  isolated from production/network paths.
- Exact uncovered core signal branches include invalid turnover units/values,
  disclosure seat identity and side/rank uniqueness, source evidence date
  separators, and checked disclosure/request deserialization.
- Research coverage still needs the explicit non-PDF content-type serde branch;
  the newly added header/size/EOF tests cover the constructor failures.
- Deeper TDX inspection corrected the initial diagnosis: the normalized adapter
  and README explicitly verify Beijing market `2` for quotes/bars/minute/trades/
  books. The defect is the service-local market mapper, which still rejects
  Beijing and contradicts the released adapter capability. Beijing security
  metadata remains explicitly unsupported because that separate list packet is
  not live-verified.
- Existing coverage profiles show the router is now much smaller than the
  earlier aggregate: remaining misses are 68 adapter lines, 4 discovery lines,
  and 6 intelligence lines. Most are explicit duplicate/order/date/evidence
  rejection branches and can be exercised with existing fixture providers.
- Router test fixtures already cover most provider families. The remaining
  adapter gaps are primarily ordering of validation (for example, an oversized
  batch masks duplicate-rank checks), missing batch `source_at`, requests
  without an optional trading date, and canonical ordering tie-breakers.

## Constraints

- Do not lower coverage thresholds or exclude critical files.
- Do not add inline source test bodies; use path-based/integration tests.
- Do not exercise slow/unbounded network paths merely to inflate coverage.
- Production data failures remain explicit and must never fall back to
  fabricated values.
- CFFEX local live validation is blocked outside the code path: the system
  resolver returns a `198.18.x.x` fake IP, and direct connections to all five
  public IPs returned by Google DoH are also closed after TLS ClientHello.
  `curl` and `ureq` fail identically. No HTTP downgrade or synthetic calendar
  is permitted; the repository's existing GitHub Actions live diagnostic is
  the next independent network gate.

## Downstream pending scope

- Pinned all 15 Magic crates to upstream release `0.2.0` at immutable revision
  `4f2730b6c37267f49f21aea9172f3062346cc06f`.
- Integrated the released CFFEX official-notice delivery contract through a
  downstream Gateway and R-08 without a direct URL, parser or date formula.
- The upstream source is a delivery notice, so a one-day advance reminder is
  possible only when CFFEX has already published that notice. Absence remains
  an explicit retryable state.
- CFFEX coverage must not be described as SHFE/DCE/CZCE/INE/GFEX coverage.
- Audit and migrate all direct financial/news acquisition code.
- Delete all replaced legacy source code, dependencies, configuration, and
  RustDX references.
- Preserve explicit unsupported behavior for capabilities the upstream does not
  provide, such as live broker-account ownership data.
- The released global-news surface is the common
  `magic_market_core::NewsProvider::global_news(PositiveU32)` contract over
  `NewsItem`; all four clients return complete `DataBatch` evidence and reject
  unavailable/empty protocol responses instead of fabricating news.
- Verified upstream provenance sources are `eastmoney-web`, `cls-v1`,
  `jin10-flash-v1`, and `thepaper-finance-v1`; the initial design label
  `eastmoney-global-news` was corrected before implementation.
- The existing downstream governed aggregator still constructs events from
  local `SearchResult` providers (Jin10, WallStreetCN, CLS, Sina flash, Weibo,
  Gelonghui and KcbDaily). These are the BR-166 replacement targets; generic
  user-authorized web search remains a separate non-authoritative capability.
- The governed production aggregator now has exactly four unified Gateway
  feeds and no longer imports any local financial `SearchResult` provider.
  Legacy provider modules may still have other callers through `SearchService`;
  those call sites must be traced before deleting the modules themselves.

## Upstream remote-live findings

- The exchange job did not reach CFFEX. The combined
  `magic-exchange-rs` live example stopped first on an SSE official endpoint
  `Authentication(403)`, so this run is not valid CFFEX evidence.
- CNInfo instrument announcements were admitted with real records, but
  whole-market discovery rejected the pagination contract because `hasMore`
  remained true on the declared final page. Diagnosis found two production
  implementations: the newer BR-027 `MarketAnnouncements` path already models
  CNInfo's `totalpages=total/30` integer quotient and separately derives the
  ceiling request-page count, while the older `AnnouncementDiscovery` path
  treats `totalpages` conventionally. The repair is consolidation onto the
  verified implementation, not an empty fallback or ignored evidence.
- The Eastmoney job is a workflow input bug: the example explicitly required
  `MAGIC_EASTMONEY_DRAGON_DATE=YYYY-MM-DD`, but the dispatched workflow did not
  set it. The adapter was not the failing component in that job.
- The old CNInfo whole-market discovery contract has now been removed rather
  than wrapped. `MarketAnnouncements` is the only released whole-market API;
  its source identity and verified pagination semantics remain strict.
- The exact SSE request that returned 403 from the GitHub-hosted runner returns
  HTTP 200 with the expected JSON media type from the local live network using
  the library's current headers. This isolates the observed failure to the
  remote execution path/IP policy unless a later independent run disproves it;
  changing source semantics or inventing a fallback is not justified.
- The exchange live example already supports a dedicated
  `MAGIC_EXCHANGE_LIVE_OPERATION=cffex-delivery` mode. CI must invoke that mode
  independently so an unrelated SSE transport rejection cannot suppress CFFEX
  evidence. The combined exchange probe must still report its own failure.
- The downstream BR-164 gate currently reports 55 violations. They split into
  three actionable groups: legacy HTTP/host-owned providers and analyzers;
  direct Magic TDX imports in `data_provider`/`selection`; and the obsolete
  `search_service/providers` financial-news tree. Existing unified Gateway
  coverage already handles global news, Sina instrument news, dragon tiger,
  CFFEX delivery, review and event-calendar slices, but the remaining groups
  still require caller migration before deletion.
- The direct Magic TDX files are not all equivalent dead code. The legacy
  fallback provider is called by closing valuation, paper risk and monitor T0;
  the selection adapter is the event-scoped selection/outcome source. They
  therefore cannot be deleted blindly. Their acquisition boundary must move
  under `src/data_gateway` and consume the released typed Magic contracts
  before the old exports can be removed.
- The first authoritative coverage rerun after the CNInfo/exchange workflow
  repair failed only because a TDX pool regression bound a loopback TCP socket
  under coverage instrumentation, where the execution sandbox returned
  `Operation not permitted`. The production invariant is a pure generation and
  reservation-accounting transition, so it was extracted into
  `ConnectionPool::settle_return_state`. The regression now exercises that
  state deterministically without weakening or skipping the test; production
  socket close/queue behavior remains outside the pure transition.
- The next coverage attempt reached all 287 TDX library tests and then found
  two external connection regressions with the same hidden assumption:
  `tests/connection.rs` bound `127.0.0.1:0` before it exercised any production
  code. `TcpConnection` now keeps its public interface but owns a private
  `ConnectionStream` seam. The production adapter is still `TcpStream`; a
  memory adapter deterministically verifies connector error mapping, read/write
  timeout configuration, send/receive, peer state and shutdown. The
  loopback-only fixture was removed only after those equivalent production
  branches were covered.
- The third coverage attempt passed all 289 TDX library tests and then found
  the only remaining repository loopback fixture in the THS concrete HTTPS
  test. This established a cross-provider test-architecture root cause rather
  than another product defect. A whole-workspace audit found exactly that one
  remaining listener. THS now follows the already-proved CNInfo pattern:
  production `execute` and deterministic tests share
  `collect_transport_result`, mapping in-memory ureq 200/403 responses and a
  typed invalid-URL transport error. A second whole-workspace audit returns no
  listener/bind fixtures.
- The repository-level transport seam repair is now Gate-D evidenced rather
  than merely targeted: the complete coverage-instrumented workspace passed,
  with 82.12% overall and 95.20% critical coverage. A separate latest-code
  full test run also passed, so the memory seams did not conceal a normal-build
  failure.
- Upstream candidate `cc8d26d` was pushed and its latest combined exchange
  remote job still stops at the SSE official endpoint with
  `Authentication(403)`. The browser-equivalent header repair therefore did
  not change GitHub-hosted runner access; this remains explicit remote-path
  evidence, not permission to report the exchange job as healthy.
- The new independent CFFEX job did reach the dedicated provider and advertise
  `futures_delivery=true`, then failed before any response with
  `Network is unreachable (os error 101)` for the official HTTPS notice page.
  Combined with the earlier local TLS failure, there is still no independent
  live CFFEX record evidence. This is a transport-path blocker, not a parser
  defect and not grounds for formula fallback.
- Downstream BR-167 has a typed `EconomicCalendarGateway` over the released
  Jin10 economic-release contract. The remaining consumer defect is
  `SearchService::search_macro_news`, which still calls legacy WallStreetCN,
  CLS and Jin10 clients directly and labels historical releases as a future
  calendar. It must be changed to four independent `GlobalNewsGateway`
  results plus the economic release Gateway, with each unavailable source
  rendered explicitly.
- `SearchService::fetch_financial_calendar` currently converts every Jin10
  error to `Vec::new()`, violating explicit failure semantics (2.1/2.2).
  `search_macro_news` repeats the same direct acquisition independently,
  deduplicates formatted strings, and describes the result as “未来48h” even
  though the released upstream contract is latest economic releases. Both
  legacy surfaces are BR-167 deletion targets after the new renderer passes.
- `fetch_financial_calendar` has no production or test callers outside its own
  definition, so it can be deleted in the BR-167 slice once the macro report
  consumes `EconomicCalendarGateway`. The report's second-stage generic search
  loop is a separate user-authorized search capability and remains temporarily;
  it must not act as a financial-provider fallback for any failed Gateway
  section.
- `GatewayBatch` and `GatewayError` already expose the complete renderer seam:
  records/evidence for `Available` or `VerifiedEmpty`, plus public
  `reason_code`, `retryable`, and `Display` for failures. No new transport
  abstraction is needed; the macro formatter can be a pure deep module over
  these typed outcomes.
- The second macro-news search loop is not yet safely generic: the shared
  `providers` vector includes direct Eastmoney, WallStreetCN, CLS, CNInfo,
  SSE/SZSE and Xueqiu adapters before SerpAPI/Bocha/Tavily. Without an explicit
  provider capability, it can silently use a superseded financial adapter
  after a Gateway failure. BR-167 must identify user-authorized general web
  search through the `SearchProvider` interface rather than provider-name
  string matching.
- `SearchProvider` already has a capability pattern (`supports_topic_search`)
  with provider overrides. The compatible repair is a second default-false
  capability; only SerpAPI, Bocha and Tavily opt in. This keeps every existing
  direct financial adapter excluded without name lists or downcasts.
- The legacy `jin10.rs` calendar is cleanly separable from its still-used flash
  path: one event struct, one parser, `fetch_calendar`,
  `fetch_calendar_entries`, CDN URL literals and calendar-only tests. No flash
  type or caller depends on those symbols, so BR-167 can delete them without
  retaining a compatibility alias.
- The latest Eastmoney remote gate uses the requested 2026-07-24 dates and
  reaches the real provider, but reports ten independent failures. Three board
  flow families and popularity receive HTTP 302; the Dragon Tiger seat
  identity collapses distinct rows with the same date/side/seat name; three
  limit-pool probes request only three rows and then correctly fail the
  complete-batch contract because the source totals are 11/25/116; one limit
  pool request hit a transient TLS resource error; and global news returns the
  official `global.eastmoney.com` article host, which the current allow-list
  rejects. These are now separate protocol/model/probe/transport defects, not
  one generic “Eastmoney unavailable” condition.
- The upstream checks job fails only when compiling the CNInfo example with
  `--all-targets`: its included `market_announcements_probe_tests.rs` fixture
  omits the newly required `Announcement.instrument_name`. The ordinary local
  test command did not compile this example target, so the repaired release
  evidence must include `cargo test --workspace --all-targets`.
- The cargo-deny failure has two roots. `magic-sina-rs` is the sole owner of
  `scraper 0.24`, which brings the unmaintained `fxhash` advisory
  RUSTSEC-2025-0057 and four rejected MPL parser crates. Separately, ureq/rustls
  legitimately uses `webpki-roots` under the permissive
  `CDLA-Permissive-2.0` license, which the current policy omits.
- Eastmoney's two Dragon Tiger seat endpoints do not provide a unique broker
  seat identifier, and the same display label (commonly `机构专用`) can occupy
  multiple ranked rows for one entry and side. Rejecting by label or even by
  identical displayed amounts loses real source rows. The stable identity is
  the already-validated entry ID plus side plus source-order rank; entry/date/
  trade-ID/cardinality checks remain atomic.
- The verified Eastmoney rolling global-news page legitimately links to both
  `finance.eastmoney.com` and `global.eastmoney.com`. Canonicalization must
  retain the exact admitted source host and exact `/a/<digits>.html` path;
  suffix lookalikes remain rejected.
- The limit-pool production contract correctly rejects partial batches when
  source `total` exceeds the caller limit. The release probe was wrong because
  it reused a three-row display limit. A 1000-row request (the core contract
  maximum) is required only for the live completeness gate; production caller
  limits and atomic semantics are unchanged.
- Redirects remain disabled in the Eastmoney HTTPS agent. Exposing a bounded
  response `Location` on 3xx is sufficient to diagnose endpoint drift without
  following an unvalidated target or converting a transport failure into
  success. PDF/HTML requests also now close their probe accounting on every
  mapped failure path.
- `qmt-parser` had no production consumer. Its only enum dependency was
  `stock_code_map::market_of`, and that function was exercised only by two
  integration tests. The dependency, enum adapter and tests are deleted while
  the dependency guard prevents reintroduction. String-only QMT symbol
  conversion remains because it has no parser/runtime dependency.
- `cargo tree -i qmt-parser` now reports no matching package. Polars resolves
  exactly once, as direct `polars 0.54.4 -> stock_analysis`; the former
  qmt-owned Polars 0.52 graph is gone.
- The apparent “new stock over 20%” persistence defect was not in the unified
  historical-bars Gateway. That Gateway already passed `max_gap_for(code)`;
  `DatabaseManager::save_kline_data` then ran a second, obsolete validator with
  a hard-coded 20% ceiling. This duplicate rejected valid STAR-market
  20.0052%/20.0917% tick-rounded moves after upstream admission.
- The unique BR-092 validator can safely distinguish ordinary board limits,
  but its IPO/ex-rights exceptions are currently test-only registries:
  repository-wide searches find no production `mark_ipo` or `mark_ex_rights`
  caller. A real lifecycle source must populate these facts before the
  exception can be used in production; code prefix or local observation time
  is not sufficient evidence.
- Candidate assembly had two independent production defects: it used
  `portfolio::get_all_codes()` as the held-position exclusion and therefore
  incorrectly removed explicit watchlist candidates, and it projected no
  market statistics so P-03 could never receive a real volume ratio. The
  correct seam joins exact ordered quote and Tencent statistics batches,
  retains both evidence objects, and filters only confirmed positions.
- Activating the single Polars 0.54 graph exposed two deterministic migration
  requirements: temporal streaming also needs the `strings` feature in 0.54.4,
  and `LazyFrame::drop` now accepts a `Selector`. The explicit feature and sole
  `by_name` migration compile and pass strict workspace Clippy.
- SearchService no longer owns financial/news provider protocols. The remaining
  BR-164 count fell from 55 to 42, and deleting the obsolete
  `src/bin/test_em_fetch.rs` direct Eastmoney diagnostic reduced it to 41.

## 2026-07-28 current release findings

- BR-174 cannot reuse `selection_candidates` as a schema-v2 projection:
  current candidates have foreign keys to the lossy v1 inbox/run graph. The
  valid cutover is physical isolation: v1 candidates/outcomes finish only
  legacy T0/D1, while receipted v2 admitted `selection_samples` are the new
  shadow-visibility read model.
- SQLite integrity/durability is a Gate B prerequisite, not an implementation
  detail. The shared production pool currently sets `synchronous=NORMAL` and
  does not enable/read back foreign keys on every connection; v2 FK and
  Prepared/stage/Committed/receipt claims are not valid until the pool enforces
  `foreign_keys=1` and `synchronous=2`.
- Notification simhash disposition cannot be part of the authoritative source
  ingress hash because ingress must commit before notification projection.
  The v2 inbox stores the deterministic projection ID but leaves
  Delivered/DedupSuppressed ownership to notification audit.
- A recovery envelope is required before Prepared audit because payload/run
  hashes alone cannot reconstruct provider records after a crash. Recovery
  must drain envelope-only runs by `enveloped_at,stage_run_id` separately from
  manifested runs by `staged_at,stage_run_id`; otherwise the earliest crash
  window is permanently orphaned.
- SQLite receipt triggers can enforce row cardinality and hash relationships
  but cannot authenticate the external JSONL audit. Schema-v2 authoritative
  reads therefore require a fail-closed verified read model that validates the
  full audit chain and every receipt before startup/read snapshots; a DB-only
  receipt is not authority.
- An `Available` feed row is not proof of no loss unless its exact
  `record_count` distinct source-fact attempts are present and bound to that
  feed/batch. Both the ingress stage transaction and receipt trigger must
  enforce that count; VerifiedEmpty/Unavailable require zero children.
- Prospective board evidence needs a registered immutable generation window.
  BR-177 binds each source fact to one config activation and one
  Asia/Shanghai market date; crossing that boundary becomes an explicit
  no-provider-call `prospective_window_closed`, not freshness reclassification
  or retry-count expiry.

- Current upstream coverage at immutable revision
  `b2b68df78156df1d67824e5c44c0cb01b752f55a` is 86.97% overall and 95.11%
  critical under the repository's unchanged checker. This supersedes the
  earlier coverage figures recorded during intermediate revisions.
- CFFEX futures-delivery support is not releasable on current evidence. The
  official host fails DNS resolution in all bounded transport probes, so the
  capability must remain false and the production trait must return typed
  `Unsupported`. Formula calendars, HTTP downgrade and locally inferred dates
  remain prohibited.
- The formal selection implementation is narrower than the historical
  `opportunity` design: it accepts only exact direct-mentioned securities with
  complete market evidence. This is safe but does not satisfy the requested
  news-to-industry-chain discovery quality by itself.
- The formal selection database has candidates, feature snapshots, outcomes
  and visibility, but hard-rejected securities are not yet queryable as
  per-security training/backtest samples. Rejection facts currently survive
  only in the immutable audit and event-level completion reason.
- Formal outcomes and reports expose T0/D1 only. The old
  `opportunity/news_outcome.rs` contains D1/D3/D5 concepts, but it is coupled to
  the superseded d01 JSONL path and is not evidence that formal selection has
  D3/D5 coverage.
- A high-quality BR-174 closure therefore needs one formal path: traceable
  Magic chain-membership evidence for related candidates, fixed-version
  features, queryable rejected rows, D1/D3/D5 outcome settlement, and
  selected-plus-rejected backtest/calibration. It must not restore direct HTTP
  clients or use `default()` after missing money-flow evidence.
## 2026-07-29 — CFETS 对 R-08 全球隔夜批次的适用性复核

- 固定上游 `magic-market-data-rs@660902ff` 的 `magic-cfets-rs` 当前正式能力是参考利率（如 Shibor/LPR）与其既有外汇族；它没有“美股三大指数已完成交易时段/日收盘”的统一批次契约。
- `magic-market-core::global` 仍只有实时 `GlobalIndexQuote` / `FxQuote` 形态；因此 CFETS 不能补齐 R-08 所需的三指数 + USD/CNY 同一完成时段证据，也不能用本地 `observed_at` 替代缺失的 provider `source_at`。
- 结论：R-08 继续显式降级是正确行为；要解除必须先在上游发布 typed completed-session/daily-close contract，再更新下游固定 revision，不能通过放宽 2.4 freshness 或跨源拼批实现。

## 2026-07-29 — AGENTS 2.3 相邻日值门复核

- 当前活动实现并未简单把大涨股票当坏票丢弃：`src/monitor/data_quality.rs` 保留 `MAX_UNCONFIRMED_ADJACENT_DAILY_CHANGE_PCT=20.0`，超过阈值生成 BR-171 待人工确认，确认账本位于 `src/database/daily_change_confirmation.rs`。
- `docs/business_rules.md` 的 BR-092/147/156/171 已明确：该门只决定数据准入，不属于选股质量或收益过滤；真实大涨经绑定 code/date/prices/provider/batch/operator 的不可变确认后可以进入计算。
- 仍需在最终 Gate C 检查过时的“删除相邻阈值”文档与活动 AGENTS/BR-171 是否构成设计矛盾；不得按旧文档恢复无确认直接放行。

## 2026-07-29 — BR-189 GlobalSchema owner sidecar 代码审查

- 新顺序符合设计：取得 exclusive maintenance authority 与初始无 sidecar 门后，descriptor-bound 读写连接先物化并固定 exact WAL/SHM；此后才创建 `LockedSelectionAuditSession` 和 `BEGIN IMMEDIATE`，因此 owner 自己的合法 SQLite sidecar 创建不会落在审计 namespace mutation window 内。
- `VerifiedSelectionSchemaSnapshot::consume_authority` 在同一 transaction/audit session 内复核 catalog、PRAGMA、integrity、audit、主库身份及 exact sidecar，随后 commit transaction 并 finish audit；只有外层 drop connection、descriptor-relative exact unlink、目录 `sync`、最终 sidecar 全缺失之后才 `PreparedSelectionSchemaInspection::issue` capability。
- `PinnedNamespace::validate_unchanged` 只比较目录 device/inode，sidecar create/unlink 不会因合法 mtime/ctime 变化被误判；audit 自身仍维护更严格的 namespace mutation marker。
- 待精确测试确认的风险点：空/只读 WAL 库是否稳定物化两枚 sidecar；以及三类启动前未知 sidecar 是否均被保留并拒绝。若测试失败，回到 BR-189 Gate B 修复，不加 marker refresh 或 test-only 分支。

## 2026-07-29 — BR-187 盘中形态接入复核

- 新的盘中形态 Gateway 使用 Magic TDX `get_t0_evidence_batch`，绑定同一代码、批次、来源时间与观察时间，并按 2.4 在读取及缓存命中时重新执行五秒实时门；消费者在缺失/不一致时显式失败，没有恢复旧 HTTP 采集。
- 集成审查发现一个 Gate B 缺口：`magic_tdx_t0::validate_settled_daily` 仍直接产生 `daily_change_over_20pct`，没有查询 BR-171 的 exact 手工确认账本。BR-187 会把它正确投影为 `manual_confirmation_required`，但目前即使已经完成同一证据对的人工确认，也无法重新准入。必须补齐该确认闭环；不得直接删除阈值，因为 AGENTS 2.3 要求“告警 + 人工确认”。

## 2026-07-29 — 最终 legacy 文本/直连静态复核

- 活动依赖与生产代码中未发现 `qmt-parser` 或双 Polars 版本；README 仅保留“QMT 是未接入的执行边界，不含 qmt-parser”这一明确声明。
- “RustDX 全部删除”仍有文档残留：`docs/emquant-api-integration-plan-调研-2026-06-05.md` 和 `docs/superpowers/specs/2026-07-17-repository-history-and-gate-remediation-design.md` 仍多次描述旧 RustDX 架构。它们不是生产调用，但与用户要求的“不要保留任何 rustdx 相关内容”不一致；最终 cleanup 必须决定删除/归档净化，并让静态 guard 覆盖文档，而不能仅搜索 `src/`。
- 非 Gateway 的 `reqwest` 命中大多属于 LLM、通知 sink、Magiclaw daemon 或测试 fixture，不是金融/新闻采集；`src/bin/monitor/blocking_market_data.rs` 仍需按调用语义复核，确保它没有绕开统一金融数据 Gateway。
- `src/bin/monitor/blocking_market_data.rs` 复核完成：生产部分只是通用 `spawn_blocking` 生命周期边界，不创建 HTTP client；唯一 `reqwest::blocking::Client` 位于 `#[cfg(test)]` runtime-drop 回归。实际生产调用仍指向 `market_data`/Magic Gateway 装配函数，因此该命中不是金融源直连残留。

## 2026-08-01 — BR-192 focused regression root causes

- R-08 rendered holdings intentionally preserve an explicit blank broker-position
  placeholder before user-confirmed holdings when the broker batch is unavailable.
  The failing test incorrectly assumed the first element was the confirmed holding;
  production ordering already complied with 2.2 and required no change.
- R-04 moved to the BR-194 source-only counted gate. Its static BR-192 test still
  asserted the older generic counted function name; the production dispatcher was
  already correct.
- `v14_gate` and `v14_gate_counted_binding` both used the internal
  `CombinedAccount` marker, so the generic gate accidentally satisfied the counted
  admission check. The module now distinguishes generic combined-account context
  from explicitly bound counted combined-account context without changing the
  public interface or governance data source.
- The BR-192 registry row intentionally contains literal pipes inside frozen
  contract prose. The old §2.10 parser incorrectly assumed field 5 was always the
  path column. It now reconstructs intent fields and reads the final non-empty
  field as code paths; the frozen worktree and staged BR-192 row remain byte
  identical at SHA-256 `9dde6d41e24d265ab1f102ec103166ff1ab90d9493864f49251f32de15525c11`.
- BR-193 remains a Gate A draft. Marking the registry status `spec-only` is the
  truthful state; adding BR-193 citations to five future adoption targets would
  falsely claim behavior that is not implemented.

## 2026-08-01 — full lib failure topology

- The current full library suite is not release-ready: 44 tests fail.
- Many selection failures are secondary: after the first selection audit
  namespace failure, the process-wide audit mutex is poisoned and later tests
  report `audit_lock_failed` instead of exercising their intended behavior.
  The first failure must be repaired before recounting the remaining set.
- Two global-schema tests fail before the selection cascade and therefore remain
  independent blockers.
- Event delivery test isolation, chain-fact fixture persistence, activation
  artifact hashes, outcome recovery bounds, persistence interface assertions,
  and request-evidence schema tests also show independent symptoms and require
  exact isolated reproduction.

## 2026-08-01 — current `--review` source-only root cause

- A fresh full workspace run completed successfully after the earlier 44-failure
  topology was repaired: library 2313 passed/7 ignored and monitor 523 passed/4
  ignored; all integration targets exited zero. The older RED count above is
  retained as history, not current release evidence.
- The exact isolated `cargo run --bin monitor -- --test` command exits zero and
  completes the TEST_CODE v70 end-to-end path.
- The exact `cargo run --bin monitor -- --review` command still exits two. Its
  first R-04 failure was the upstream canonical `unix-ms:<epoch>` observation
  format; the parser now preserves the raw value in immutable evidence and
  strictly decodes it for the durable UTC timestamp. Focused valid/invalid tests
  pass and the next real run advanced beyond preparation.
- The remaining R-04 failure occurs after a complete five-row DragonTiger
  Gateway batch is prepared and before Launch/L5/durable admission. The exact
  `review_audit` reason is `counted_source_only_binding_invalid`: preparation
  accepts canonical `unix-ms:<epoch>` and preserves those provider bytes, while
  `CountedDeliveryBinding::validate_r04_source_only` reparses the same field as
  RFC3339 only. The typed origin and canonical evidence therefore disagree by
  parser contract even though they describe the same instant. This is the
  current root cause; the earlier DataMode hypothesis is disproven because the
  request never reaches L5.
- R-04 currently writes an empty dispatcher error after any non-pushed
  `PushOutcome`; the typed denial reason survives in `ReviewTaskOutcome` but is
  discarded by `log_dispatcher_attempt`. Observability must retain the exact
  Denied/Deduped/SinkError reason.
- R-09 is independently fail-closed on a historical/weekend effective review
  date (`provider_top_n_current_date_only`) before provider access. It is not the
  same R-04 DataMode defect and must not be repaired with cached or inferred data.

## 2026-08-02 — BR-202 second independent Gate A findings

- Inventory closure is no longer the blocker: the reviewer independently
  reproduced 36 directory classifications, 29 root classifications, 16
  top-level bin classifications, 408 uniquely classified Rust files, 12
  non-self historical-plan matches and byte-identical BR-202 registry text.
- The design's single-entrypoint claim is contradicted by the current CI and
  README, which still teach the raw coverage gate; both must be explicit Gate B
  migration targets and the wrapper must be the only admitted entrypoint.
- A temporary worktree is not safe merely because it is isolated. The contract
  must export authoritative evidence before cleanup, install success/failure
  traps, preserve failure diagnostics, define cleanup ordering, and fail if the
  worktree cannot be removed without sacrificing the exported evidence.
- Five hand-written capability categories cannot prove behavior completeness.
  A closed, machine-enumerated behavior-cluster registry is required so every
  registered cluster has either production evidence or the exact disabled
  banner, with omissions and duplicates rejected.
- An attestation that checks only Git ancestry/path shape can accept stale or
  fabricated JSON. Verification must parse and independently recompute source
  SHA, inventory counts, report/artifact hashes, isolation facts and capability
  evidence against the exported artifacts.
- Phrases such as "any configured store" or generic broker credentials leave a
  bypass surface. The isolation contract needs frozen environment-variable,
  credential-mode and production-store registries with default-deny handling of
  any unregistered resource.
- Strict JSON integer parsing must freeze a concrete representation and maximum
  (rather than saying only "exact integer range") so boundary tests are
  deterministic and portable.

## 2026-08-02 — BR-201 third independent Gate A findings

- A single decision record cannot be typed as `DeferredDebounce` or `Admission`
  before debounce is read. The design needs one deterministic observation/read
  phase followed by exactly one terminal decision append, or an explicitly
  separate non-decision trace record; it cannot promise both premature logging
  and exactly one decision row.
- Any explicit failure reason used by admission must be a member of the frozen
  terminal-reason registry. `manual_confirmation_required` currently is not.
- Basis points are not self-defining reconciliation evidence: decimal input,
  scaling, tie-breaking, allowed residue and sum-boundary behavior must be
  frozen in integer arithmetic and tested at both signs and half-way cases.
- BR-086 cannot be both a preconfirmed input and a row atomically committed with
  the order. The design must distinguish reserved identities from confirmed
  records and define mandatory rejection audits for every rejected order.
- JSONL `Claimed` recovery needs exclusive ownership, generation and durable
  prior-owner death/handoff proof plus lock-held state re-read. A second recovery
  observer must converge on the winner rather than treating a zero-row CAS as an
  undefined failure.
- Exact `new_generation == old+1` is not crash-resilient. The persisted attempt
  must bind the winning generation/owner, while later verified owners may
  advance monotonically after proving the prior generation dead; otherwise a
  crash between lock acquisition and CAS permanently wedges recovery.
- Joined-order facts require a closed `fact_kind` enum and exact per-order row
  cardinality/ordering before a non-empty join hash is independently computable.
- Rollback authority must be based on the deployed artifact and a new signed,
  parent-bound rollback attestation. A local rebuild cannot certify the running
  binary, especially after a deep revert.
- A new account batch contract has no production value until a named real
  provider/adapter emits it from source-fresh position/cash evidence. Rejecting
  every existing source while requiring a positive canary makes the design
  impossible to release.

## 2026-08-02 — BR-196 complete-presentation audit findings

- Forced replay is a live outward presentation outside the descriptor registry:
  `main.rs` routes it, `src/event/replay.rs` assembles distinct text, and the
  real sink calls raw `notify::push_wechat`. An exhaustive template/presentation
  inventory must include this behavior; descriptor enumeration alone cannot.
- Static evidence must include executable commands, pasted output, every hop to
  the gateway and a reviewed source-tree identity. A placeholder search command
  plus truncated chains is not reproducible authority even if a reviewer can
  reconstruct the correct 26/24 descriptor split independently.
- API visibility and wrapper replacement are compatibility changes even when no
  `PushKind` variant or stable storage identity is renamed. Gate A must record
  downstream callers, shims/migration and rollback rather than assert no change.
- Literal pipes inside Markdown rule text must be escaped or replaced. Otherwise
  the canonical business-rule row is structurally invalid despite a substring
  match and Rule 2.10 tooling may read the wrong Code column.

## 2026-08-02 — BR-202 realizability findings

- The wrapper cannot change the exit behavior of a separately invoked
  `cargo llvm-cov`; it can instead be the only process that creates a release
  context, independently verified artifact bundle and PASS attestation. Raw
  coverage remains diagnostic and must not be described as release authority.
- A failure journal opened after successful verify cannot preserve earlier
  export/fsync failures. The persistent diagnostic target/fd must be prepared
  and synced before the disposable run root exists, or preflight must fail
  without creating disposable state.
- A partition of source/test files is not a partition of business behavior. One
  file can implement many descriptors and one behavior spans many files. The
  cluster authority must be generated from closed business registries plus a
  reviewed residual set, with a mega-cluster and omitted behaviors rejected.
- Coverage JSON does not prove which source-built binary/profile produced it.
  Release evidence must bind covered object/build identities, LLVM mapping,
  profraw/profdata, Cargo lock/metadata, features/targets and toolchain versions
  to the fixed source tree, then independently verify the report derivation.
- Offline isolation needs a credential-free, content-addressed dependency
  source: a source-controlled vendor tree or a release-hashed read-only cache
  whose exact Cargo.lock/source/config identities are verified. Empty isolated
  Cargo state cannot resolve current Git dependencies.
- Moving CI consumers from standalone `coverage.json` to a nested verified
  bundle is an artifact API migration. Bundle layout, artifact name, consumers,
  compatibility alias/deprecation and rollback must be explicit.

## 2026-08-02 — BR-201 identity and compatibility findings

- Changing the hash/domain of an idempotency key is a safety migration even if
  it describes the same business intent. During cutover, both legacy and V1
  identities must be queried/reserved under one transaction and one fence; any
  existing or unresolved old/new row makes the other form ineligible. Rollback
  needs the same dual protection until all relevant windows and pending orders
  are durably closed.
- Typed failure contracts need a distinct closed reason for every account parse,
  sign, zero, overflow, asset-identity, aggregate and basis-point mismatch. A
  broad `partial` label or free-form diagnostic cannot satisfy a closed terminal
  registry.
- A signature proves who signed bytes, not that a revert is the exact inverse.
  Rollback authority must recompute ordered parent chains, inverse patches/trees,
  allowed paths and final tree equivalence, rejecting any extra change.
- Conflicting business-rule MUSTs require explicit scoped supersession. For
  four-rule exits, BR-201 may replace eager context and attempt-all semantics
  while preserving complete per-item audit and stopping new side effects on
  expired authority; all other BR-134 callers remain unchanged.
- A persistent singleton must define genesis: schema/version, exact null matrix,
  atomic seed, first-due rule and restart/migration behavior. Treating all missing
  state as corruption without a bootstrap path makes fresh deployment unusable.
- Keeping a Rust function name while changing its arguments is still a breaking
  API change. A versioned new permit entry and an old source-compatible shim that
  always fails closed (or a fully proven caller cutover plus privatization) are
  required to prevent legacy bypass.

## 2026-08-02 — Latest BR-196 formal Gate-A findings

- Rule 2.10 is a current Gate-A blocker, not deferred Gate-B debt: the
  BR-196 target allowlist configuration needs an explicit BR-196 citation while
  preserving all values and behavior.
- VirtualWatch has an empty post-close producer and therefore cannot be Active;
  lifecycle and matrix counts must reflect 25 active descriptors unless a real
  producer is independently proved.
- The health-failure webhook is a direct outward presentation path with its own
  JSON transport and must be inventoried or explicitly and rigorously scoped
  outside the claimed closed set.
- Broad source searches with thousands of hits do not reproduce presentation
  classification. Evidence must be bounded, include complete gateway hops and
  bind every authority source file to the reviewed tree.
- Public API impact includes deleted policy entrypoints and changed signatures
  across post-session review, R-08, R-04 and candidate-trigger paths; exact
  callers, migration, compatibility and rollback must be recorded.

## 2026-08-02 — Latest BR-202 formal Gate-A findings

- A fixed-source coverage derivation must bind every compilation and execution
  input, including `tests/**`; a clean production tree with dirty integration
  tests is not a fixed-source run.
- Publishing an attested bundle before the parent-directory fsync can leave an
  apparently authoritative bundle after a durability failure. Authority needs
  a separately durable post-publish terminal bound to bundle hash, path, source
  identity and run identity.
- Exact covered objects and executable bytes must survive disposable build-root
  cleanup in a content-addressed evidence store so an independent verifier can
  rerun mapping and coverage derivation.
- Registered authorities plus declared residuals are circular: an omitted
  decision can be absent from both sets. The denominator must be generated from
  an independent, closed production-decision inventory and map every identity
  exactly once to authority or residual evidence.

## 2026-08-02 — Root BR-201 repaired-design spot check

- The repaired document remains honest about current source state: production
  still has one old `paper_engine::run_once(PaperRiskContext)` caller, the old
  side-effecting public definitions still exist, and `run_once_guarded_v1` is
  explicitly marked TO BE BUILT. These facts prevent the design review from
  being mistaken for implementation evidence.
- The repaired contract now explicitly preserves source-compatible fail-closed
  shims, permanent legacy/V1 dual guards, monotonic cutover state, a narrowly
  scoped BR-134 supersession and a Disabled capability until a unique real
  account-evaluation provider exists. Formal acceptance still depends on the
  fresh independent reviewer reproducing those properties and finding zero
  Critical/Important defects.

## 2026-08-02 — Root BR-196 in-flight repair spot check

- The in-flight repair now classifies VirtualWatch/pilot as Disabled and adds
  the independently reachable raw health-webhook presentation. Its frozen
  target arithmetic is Shape 28/26/14/68, Family 27/25/14/66 and PushKind
  24/24/12/60; replay and health are additive typed identities rather than
  aliases for an existing business kind.
- The API-disposition section is still being expanded. At this snapshot it
  records the main notify/template removals and signature changes but has not
  yet reached the repair's promised complete classifier/news/webhook/API
  inventory. Treat these bytes as an in-flight checkpoint, not a reviewable
  final Gate-A candidate.

## 2026-08-02 — Root BR-202 in-flight repair spot check

- The repaired coverage authority now binds integration tests and other build
  inputs, retains exact content-addressed object/executable bytes, separates a
  durable post-publish terminal from the candidate bundle and keeps failed
  final-path bundles quarantined rather than apparently authoritative.
- The behavior claim is now deliberately narrowed to independently extracted
  non-test production decision/entry sites that reach a closed observable sink,
  plus every Rule-2.10 operator. A compiler/MIR/rustdoc/source-AST denominator
  `D`, total site-owner mapping `M`, closed sink/operator registry and unresolved
  class failures are intended to make an omission fail even if authority and
  residual registries shrink together. This is a stronger logical contract but
  remains an in-flight Gate-A candidate until feasibility and completeness are
  independently reviewed.

## 2026-08-02 — In-flight Rule-2.10 checkpoint

- `bash tools/compliance/lib/check_business_rules.sh` currently exits 1 with
  exactly three hard errors, all from BR-201 registering not-yet-created Gate-B
  paths. It also reports 132 citation warnings. The BR-196 allowlist citation
  hard error is fixed without changing non-comment configuration semantics, and
  BR-202 now registers only its two existing Gate-A documents until each future
  implementation path joins the row in the same staged slice.

## 2026-08-02 — BR-201 v5 formal Gate-A findings

- The formal design itself is untracked and the BR-201 Rule-2.10 row registers
  three nonexistent future implementation paths. Gate A needs a tracked design
  plus a current spec-only Code cell; future paths join the row and cite BR-201
  in the same staged implementation slice.
- The closed account-rejection registry lacks unique reasons for unsupported
  schema version, negative stop-loss count, noncanonical position order and
  zero/multiple proposed-exit joins, contradicting its total/one-to-one claim.
- Public `paper_trade::simulate` and publicly constructible `PaperSignal` leave
  a BR-201 bypass surface. The guarded exit needs a private, unforgeable
  execution authority while unrelated BR-134 simulate callers retain their
  existing contract.
- Legacy IDs use host-local date while SQLite reservations use naive UTC
  `CURRENT_TIMESTAMP`. Without frozen legacy timezone/timestamp interpretation,
  restart/cutover can bypass the 60-second dual guard.
- The state machine calls only `DualReadV1Write` enable-eligible, then requires
  a one-way transition to `V1PrimaryDualGuard`; successful cutover would disable
  itself. Canary and steady-state eligibility must be explicit and coherent.
- Operational rollback verifies an intended base but runs `git revert` on the
  caller's current HEAD. It must work in an exact clean detached tree rooted at
  the verified deployed source commit and check every parent/inverse/tree step
  before producing rollback authority.

## 2026-08-02 — BR-202 v7 formal Gate-A findings

- A terminal that binds runner-local canonical path/device/inode and internal
  read-only modes cannot be consumed after `actions/upload-artifact`: download
  changes the path/inode and the action normalizes file modes. Release evidence
  needs a canonical archive plus detached, path-independent terminal while
  retaining local durability facts only in the local journal.
- Fixed inputs cover Rust/Cargo/LLVM but not the actual linker, macOS SDK,
  system/dynamic libraries, Git, Python, shell and utilities. Every executable
  and observed read must be manifest-bound or the run must fail closed.
- The compiler/source D/M denominator has no exact executable command, flags,
  target/cfg/features, normalized output or machine marker. Desired equations
  alone cannot satisfy a machine-checkable acceptance criterion.
- The tracked invocation inventory requires two files that `git ls-files`
  rejects (`docs/handoffs/HANDOFF_2026-07-18_REPOSITORY_SAFETY_CLOSURE.md` and
  `findings.md`). Required inventory rows must be generated from tracked paths;
  untracked diagnostics cannot be release authority.
- The design is staged but the BR-202 rule row is not in the index, violating
  its own two-blob Gate-A precondition. Its status text also still says the
  staged design is untracked.
- Ranking arithmetic is correct, but a display of 12 rows cannot substantiate a
  claimed top-20 sum without the generating command or complete machine
  artifact.

## 2026-08-02 — BR-201 v6 formal Gate-A findings

- A rollback verifier loaded from the dirty caller worktree cannot choose the
  source/base that establishes its own trust. The immutable deployed signed
  artifact/receipt and pinned key must select the base before any checked-out
  worktree verifier participates.
- Initial session Admission is intentionally before lazy account acquisition,
  so account schema/count/order/join failures occur after that Admission but
  before proposal/order authorization. Current wording claims both orders.
- Private execution authority and final SQLite transaction ownership must share
  one non-bypassable module; assigning them to paper engine, paper trade and a
  database module simultaneously contradicts the privacy contract.
- The closed projection decoder omits legal same-state ownership takeovers for
  `Claimed` and `AppendAckPending`.
- `business_order_id_duplicate` overlaps the exact 60-second reason and has no
  condition; `business_order_identity_alias_conflict` is also missing from the
  result matrix. Closed reason encoding is not total outside account failures.
- A Confirmed BR-086 record cannot be an acknowledged prerequisite when the
  accepted transaction is defined to insert and read it back atomically.
- The mandatory PR Data-Redlines list omits Rule 2.3 despite adjacent-change,
  continuity and split/dividend admission gates.
- Current `run_once`/`simulate` caller and alias counts are assertions without
  the bounded multiline commands and exact output required for reproducible
  Gate-A evidence.

## 2026-08-02 — BR-196 v5 formal Gate-A findings

- Three manifest source files, the compiled allowlist configuration and the
  current BR-196 rule row are absent from the index/HEAD preimage. A scoped
  revert cannot reproduce authority bytes that were never tracked, and a whole
  untracked config is not a comment-only change relative to Git.
- A scan limited to function names beginning `push_` or `dispatch_` is only a
  presentation-function sub-audit. It misses public enum variants, removed news
  APIs/types and changed public struct fields, including
  `ReviewProviderTopN`, `record_news_recommendation`, news aggregator API
  replacements, `T0Kind`/`T0Style`/`EventHolding` and `T0AdviceParams`.
- The pasted Rule-2.10 result reports an old three-error state; the current
  checker exits zero. Compliance evidence must bind the checker, business-rule,
  design and config bytes and reproduce current output.

## 2026-08-02 — BR-201 v7 formal Gate-A findings

- The rollback trust root is still not self-contained: ambient `PATH` tools and
  caller-relative object paths remain substitutable between verification and
  execution. A single absolute root-owned, byte-pinned executable must perform
  the complete bootstrap with sanitized environment and descriptor-bound object
  handoff. HEAD/index/porcelain comparisons are insufficient; every caller path
  needs a canonical raw-byte/mode/link/absence manifest before and after.
- The current guarded signature cannot prove it consumes the exact durable
  Admission selected by the scheduler. One private high-level tick operation or
  a non-forgeable read-back-bound Admission capability must close that seam.
- `FailedAccountContext` currently requires an all-present provenance tuple for
  failures whose identity/authority/timestamp may be missing or malformed. A
  reason-specific nullable matrix plus immutable raw response/error evidence is
  required to preserve missingness without fabrication.
- The proposed symbol/path inventory omits multiple private capabilities,
  rollback types and the `br201-evidence` build target; the SQLite inventory also
  leaves reservation/audit/adjacency/BR-086/generation authorities unnamed.
- Compatibility aliases depend on free-form legacy code/reason bytes that are
  not signed or persisted before alias construction. The exact projection must
  be frozen across restart and cutover.
- Proposal ordinals affect the unique BR-086 key but the BINARY-sorted set has
  no exact field order, encoding, tie-break or duplicate contract.
- The shared BR-201 row is deliberately absent from the index until concurrent
  rule writers finish. Worktree canonical equality and a passing Rule-2.10 check
  cannot substitute for the required staged-slice equality.

## 2026-08-02 — Focused monitor failure root causes

- `config/br196_non_production_feishu_targets.toml` hashes to
  `e351650d70e0716eae3895a8092908c8b6facaea1a9d405da514cbeadacd16ba`,
  while `src/bin/monitor/br196_transport.rs` pins
  `5da7d08f213ad83816cb57acc8853924c7d7c09dc0188a37c36b12caa5b7db4b`.
  The loader correctly fails closed before parsing. The design must first freeze
  the final config bytes; Gate B then updates the pin in the same reviewed slice.
- R-08 now dispatches through
  `notify::push_r08_presented_source_only_with_binding`, which validates the
  presentation token and delegates to a private
  `push_r08_source_only_with_binding`. The unit test and BR-194 checker still
  require the obsolete public helper. The correct repair is to validate the
  public wrapper, its token/delegation, and the private counted core—not to
  reopen the private helper as public.
- Strict Clippy separately flags `manual_contains` in
  `br196_test_delivery.rs:278`; use the direct `.contains(...)` expression only
  after BR-196 Gate A is independently accepted.

## 2026-08-02 — BR-202 v7 formal Gate-A findings

- New coverage design cannot chain from unverified BR-196/BR-201 or use
  dirty-worktree inventory as authority. Independent HEAD/worktree counts are
  `391/35/26` and `408/36/29` respectively.
- Stable Rust 1.95 rejects `rustc -Zunpretty=expanded`; stable rustdoc JSON also
  requires unstable options. A success AC that forbids the enabling toolchain
  has no physically possible path.
- A critical append-only JSONL journal must satisfy the repository's complete
  lock, existing-chain/tail validation, batch serialization, held-lock append,
  sync, fail-closed, isolation and >=5-year retention contract. Append/fsync
  alone is insufficient.
- The real shared index changes six paths and 32 business-rule row lines, so an
  asserted BR-202-only two-blob index is false. Path-scoped reading cannot turn
  it into true release evidence.

## 2026-08-02 — BR-201 latest realizability findings

- Formal review is `REJECT C2/I3/M0`. The frozen SQLite open-attempt and
  delivery schemas are each materially smaller than the state machines that
  consume them, so AC-08/AC-10 have no implementable success path.
- The identifier golden file is not exhaustive: its suffix-limited extractor
  misses three named public/private types while still allowing
  `unclassified=0`.
- The root-owned rollback bootstrap is only prose. It is absent from the
  proposed tracked build/install surface and conflicts with private-bin type
  ownership, so rollback cannot be reproduced from reviewed Git content.
- Fourteen defined acceptance checks cannot satisfy an exact thirteen-marker
  assertion. This must be corrected at Gate A, not worked around in code.

## 2026-08-02 — gate-order finding

- BR-192 remains the earliest open batch. Its current five-enabled-producer
  catalog appears to describe dirty-worktree/later-batch seams rather than the
  bounded `HEAD` preimage. Under the repository no-spec-chaining rule, this
  blocks BR-196/201/202 regardless of their local documentation progress.

## 2026-08-02 — BR-192 formal metadata findings

- Independent result: `REJECT C0/I2/M0`. The current catalog correction does
  not close its own earlier I1 because all five enabled seams are absent from
  fixed HEAD; several names are stale even relative to the worktree.
- Fixed HEAD's durable schema is already v5. A plan that validates v5 without
  migration cannot install BR-192 objects into an existing production v5
  database. The next additive authority must be v6 (or an equivalent explicit
  accepted redesign), with v5 upgrade and preservation tests.
- Reviewer reproduced the staged design/plan identities, BR-192 row hash,
  whitespace checks and Rule-2.10 pass. None of those facts substitutes for
  the rejected architecture or later Gate B/C/D/live evidence.

## 2026-08-02 — BR-192 repair-candidate findings

- Fixed HEAD proves no accepted production counted producer among the 15-kind
  durable closure. The safe first catalog is therefore 15 disabled rows; a
  later worktree wrapper or enum mapping is not producer authority.
- Existing deployed schema-v5 databases require a real v5-to-v6 step. Merely
  validating v5 would make new BR-192 tables permanently absent for upgrades,
  even if fresh database tests passed.
- With every producer disabled, Gate D can prove exact disabled banners and
  zero acquisition/sink effects, but cannot honestly produce a real retry
  receipt. The verifier retains its `require_count in 1..=256` safety contract;
  no zero-count bypass was introduced.
- Plan Task 9 originally contradicted that safe catalog by requiring an active
  `ReviewProviderTopN` counted producer. The staged repair now makes the current
  check require the exact disabled row and reserves activation for a later
  producer-specific Gate-C rule plus a fresh Gate-A C0/I0 review.
- Current worktree implementation remains on durable schema v5 and contains no
  BR-192 retry-cycle/authorization/evidence authority types or v5-to-v6
  migration. Existing `br192_*` tests mainly cover earlier physical-isolation
  and BR-194 durable-delivery work; they are not evidence that the new Gate-B
  contract is implemented.
- Four consecutive fresh independent review starts have failed before sampling
  with external HTTP 403. They provide zero acceptance credit, so the strict
  no-gate-chaining rule still blocks BR-192 code changes and every later batch.

## 2026-08-02 — BR-192 exact review repair findings

- Independent exact-identity reviews subsequently completed and rejected the
  all-disabled candidate at C1/I6/M1 and C0/I3/M0. The hard issues were stale
  frozen evidence without an expiry terminal, speculative retry infrastructure
  with no consumer, incomplete caller enforcement/evidence authorities, a
  nonexistent migration-test rename, conflicting v6/newer-version criteria,
  incoherent Task-1 RED/GREEN commands and an inaccurate new-file action.
- The selected corrective architecture enables exactly one real Gate-B target:
  `push_templates::dispatch_r09_provider_top_n_outcome` backed by
  `CapitalDataGateway::provider_top_n_pair`; all other counted kinds remain
  disabled. Worktree implementations are candidate code only.
- R-09 freezes source business date plus the next Shanghai midnight expiry.
  Expired discovery/admission/manual authorization converges to one audited
  retained `ExpiredFreshness`, clears active authority and makes zero calls;
  operator authorization cannot revive it.
- An unforgeable kind/seam-bound permit plus a syntax-aware all-15 caller audit
  now covers generic governor/dispatch/durable entrypoints and every counted-
  specific loader before acquisition. Normative evidence paths are exact
  push-log pending/commit JSON plus exact event-bus JSONL event types.
- Fixed HEAD's `br194_schema_v5_migration_matrix_is_repeatable_and_rejects_newer_versions`
  identifier is preserved and extended; v6 is repeatable and v7 is the exact
  newer-version rejection fixture.

## 2026-08-02 — BR-192 latest exact-review findings

- Fresh exact reviews of the next authority identities returned `C0/I4/M0`
  and `C1/I3/M1`; neither grants Gate-A acceptance.
- The permit prose did not freeze a concrete owner/API/consumption boundary,
  so fixed-HEAD's public cloneable binding constructor could still bypass the
  catalog. The repaired contract gives the bin-local catalog sole constructor
  ownership, consumes a non-cloneable/non-serializable permit in
  `CountedDeliveryBinding::new_permitted`, persists an exact attestation and
  removes raw envelope delivery from production visibility.
- Expired rows could previously disappear behind `expires_at > now` without a
  terminal event. The repaired contract requires a complete active-expiry
  snapshot/drain before Reserved and candidate work, with pre-start Reserved
  release and acknowledged-start/consumed-ownership uncertainty semantics.
- Retry provenance now fails closed before automatic/manual authorization for
  v5/NULL envelopes, non-R-09 seams and all fourteen disabled kinds; no legacy
  row is inferred or promoted during migration.
- Task 1 now begins with the counted-cutover test file and a genuine failing
  panic sentinel; the eight contract tests also fail deliberately until their
  bodies are replaced. Empty test bodies are forbidden.
- Fixed-HEAD caller evidence now enumerates all fifteen kinds and all seven
  generic/durable entrypoints with supported commands. The clean-baseline
  dependency delta pins all five Magic crates to one Git revision and exact
  package version, including `Cargo.lock`.

## 2026-08-02 BR-192 expiry/clock/total-order repair

- Preliminary independent review returned C3/I2/M0: an unauthorised manual
  target could expire without a persistable audit, freshness used PAM time
  rather than a private current clock, and a Pending expiry could be stranded
  by later start acknowledgement. The denial-opacity and rollback-catalog
  wording were also contradictory.
- The repaired contract adds nullable authorization for the closed
  `ManualTargetExpiredBeforeAuthorization` terminal, exact persisted
  `freshness_observed_at`, and a private production clock with no caller-time
  parameters on freshness-bearing coordinator operations.
- A Pending `SinkAttemptStarted` is already conservative authority because its
  immutable append may precede SQLite acknowledgement. Four exact triggers now
  impose a total order: start/ownership first routes expiry to uncertainty;
  expiry first blocks all later start/ack/ownership writes.
- Winning ownership before midnight is not sufficient to send after midnight.
  The sole sink-capable method performs a final private-clock gate at the call
  linearization point. Expiry consumes the single-use permit with zero external
  call and atomically persists `FreshnessExpiredBeforeExternalCall` plus the
  Pending expiry authority.
- Rollback now preserves the exact 15-row catalog bytes/hash and enabled R-09
  identity; only the periodic retry runner may be disabled. A catalog mutation
  cannot be used as rollback while retained authority exists.
- Scoped `git diff --check` and Rule-2.10 pass. Rule-2.10 still reports 131
  repository-wide historical warnings but zero BR-192 hard errors.

## 2026-08-02 — BR-192 final-pre-call expiry findings

- Final pre-call expiry is a definite zero-external-call state, but fixed v5
  `delivery_attempts` cannot encode a new physical state without rebuilding a
  highly referenced table. The repair retains the base `AttemptInFlight` row
  as compatibility history and derives the effective terminal from appended
  expiry, revoked fence, released reservations, terminal schedule, cleared
  current pointers and zero result.
- The no-call authority needs two transactions: Transaction A atomically
  commits companion, terminal ownership, Pending expiry and exact cycle
  recount; after immutable append/ack, Transaction B atomically revokes send
  authority and terminalizes decision/binding/schedule/reservations.
- Eight triggers were insufficient because a sink result could be inserted
  after a partial or committed expiry triple. A ninth reverse trigger plus
  result-absence rechecks on ownership and expiry writes closes every result-
  first/between/after interleaving.
- `FreshnessExpiredBeforeExternalCall` contributes zero calls but is not
  ambiguous. Cycle evidence must be recalculated from exact
  `TerminalRecorded`+authoritative-result joins and become `Confirmed(n)`, not
  blindly `Confirmed(0)` or permanently `Indeterminate`.
- R-09 non-empty/finite/provenance validation makes Rule 2.3 applicable;
  provider-verified empty is typed `Failed` with zero binding/sink, never
  `NoData`.

## 2026-08-02 — BR-192 cross-rule/state precheck findings

- BR-194 SourceOnly forbids R-09 from accepting `BannerCtx`, account mode or a
  broker snapshot. The only admissible target signature is
  `dispatch_r09_provider_top_n_outcome(business_date, observed_at)`.
- BR-198 is a whole-release invariant, not a five-crate BR-192 subset: exactly
  14 direct Magic dependencies and 15 lockfile packages must resolve to
  `=0.2.0` at revision `5f1ce93656a55854c844065390520cd4aecd9a14`.
- BR-200 durable occurrences need distinct fail-closed semantics: delivered
  with hydration is terminal reuse; delivered without hydration and
  nonterminal states are retryable reconciliation; rejected/uncertain are
  nonretryable terminal failures; corrupt or ambiguous rows are typed invariant
  failures; absence alone permits the normal provider path.
- `terminal_sink_result_identity` must be a database-enforced bijection, not an
  application convention: unique deferred reference, one-time assignment,
  immutability and an authoritative non-late exact attempt/decision/fence
  reverse join are all required.
- An exact `cargo test ... --exact` command is evidence only when a matching
  test declaration exists and the run selects exactly one test. Every planned
  command must be mechanically compared with declarations before review.
- BR-202 owns Gate-D release evidence. Raw `cargo llvm-cov` remains diagnostic
  only; BR-192 may mint no release PASS outside
  `tools/coverage/run_isolated_gate.sh`.
- Requiring BR-202 Gate A before BR-192 Gate B creates a circular gate because
  CLAUDE.md forbids the later BR-202 batch from progressing before BR-192 Gate
  C. The non-circular contract is same-slice BR-202 registration/citation for
  BR-192 Gate-B paths, followed by BR-202 Gate A/B/C after BR-192 Gate C and
  exclusive BR-202-wrapper authority at Gate D.
- The repaired plan contains 241 unique exact test command targets and 243
  unique in-plan `br192_*` declarations; the only external declaration is the
  tracked Magic release-revision test. A mechanical comparison reports zero
  missing and zero duplicate declarations.
- `br192_magic_market_release_revision_is_one_atomic_identity` independently
  executes exactly one test and proves all 14 direct/15 lockfile packages are
  version 0.2.0 at the one pinned release revision.

## 2026-08-02 — BR-192 three-way RED precheck findings

- The prior same-slice BR-202 registration formulation was still circular
  because the current BR-202 Code cell owns only its two docs. BR-192 Gate-B
  source may carry literal future `BR-202` citations, but this batch cannot
  mutate or claim the current Code cell. After BR-192 Gate C, a fresh BR-202
  Gate-A object may register those already-accepted paths and later build the
  isolated wrapper.
- The v6 reverse result trigger requires a matching `TerminalRecorded`
  ownership pointer before authoritative/non-late result insertion. A positive
  result-first recipe cannot execute; result-first belongs only in a negative
  immediate-rejection test.
- Revision `660902...` cannot be the BR-198 rollback target while enabled R09
  is retained because it predates the admitted Provider Top-N API. Rollback
  must preserve the full `5f1ce936...` 14-direct/15-lock identity and revert
  only closed-day dispatch behavior.
- A required exact test absent from fixed HEAD must be owned as `Create`, named
  in its task file list and included in the atomic commit recipe; an untracked
  worktree test cannot be treated as inherited release authority.
- A prerequisite design cannot satisfy a separate Gate-C dependency without a
  machine-executable plan containing exact paths, RED declarations, focused
  and Gate-C commands, PR evidence fields and an actionable rollback.

## 2026-08-02 — Cross-rule dependency-cycle resolution

- BR-198 cannot be independently implemented from fixed HEAD: the R-09
  producer, capital gateway, BR-200 occurrence seam and release-identity test
  are absent and explicitly owned by BR-192 Task 8. Requiring BR-198 Gate C
  before BR-192 Gate B is therefore a real dependency cycle, not missing test
  evidence.
- The non-circular sequence is: independently land BR-200's generic read-only
  occurrence API with R-09 disabled; accept BR-192 Gate A incorporating BR-198;
  then atomically create the R-09 gateway/producer plus BR-198 date/capture and
  Magic dependency closure in BR-192 Task 8.
- BR-200's current unstaged code is not acceptable implementation evidence: it
  has string-downgraded errors, a writeful hydration side effect in the
  purported read-only seam, the wrong rule vector, only 2 of 23 planned exact
  names, and a red compliance checker. Six substitute tests passing do not
  satisfy Gate B.
- Shanghai freshness requires a fixed timezone authority and an acquisition
  interval, not merely matching calendar dates. The admitted invariant is
  `trusted_request_start <= provider_capture <= trusted_completion`, with
  complete raw provider timestamp bytes retained; host `TZ`, capture-before-
  start and capture-after-completion have explicit regression tests.

## 2026-08-02 — BR-200 / BR-194 checker fact check

- The current shared `check_br194_review_dependency.sh` does not require an
  R-09 provider implementation. Its R-09 assertions are limited to the closed
  `ReviewTask::dependency()` SourceOnly identity, dispatcher task membership,
  future-date policy markers and two BR-198 review-date tests. The provider
  contract assertions in the same checker are for existing R-04/R-08 paths.
- BR-200 can therefore remain an independent provider-free prerequisite while
  preserving every existing BR-194 R-09 identity assertion. Its checker work
  must be additive for typed R-04/R-08 occurrence preflight and must not remove
  or weaken the existing R-09 assertions.
- The worktree already contains later unaccepted R-09/BR-200-shaped code, so
  Gate-A realizability must continue to be judged against fixed
  `HEAD=b4aeee68d2c0259cc968914b3d39e3a89a18a496`, not dirty-worktree symbols.

## 2026-08-02 — Final Gate-A normalization facts

- Durable kind identity and production capability are separate authorities:
  `R09 -> ReviewProviderTopN` must remain mapped for BR-194 compatibility while
  BR-200 keeps `R09 -> DisabledNoProducer`; BR-192 must later switch capability,
  catalog permit and complete producer/gateway atomically.
- A rollback artifact is release evidence only when a normal Gate-B/Gate-C
  verifier proves its exact source SHA, one-file target, applied semantics,
  buildability and non-zero focused recovery tests in an isolated worktree.
- Rule 2.10 Code cells may list current implementation authority paths, not
  future files that Gate B has not created. Planned rollback artifacts remain
  mandatory in the design/plan file inventories until implementation exists.

## 2026-08-02 — Formal-review blocking facts

- A detached rollback verifier cannot validate uncommitted Task-8 files when
  called with the old branch HEAD. The safe pre-commit authority is a commit
  object made from the complete staged tree; the final implementation commit
  must have the identical tree and is verified again after commit.
- Fixed HEAD classifies R-08 as `LegacyAccountGate`, excludes its provider from
  the central dispatcher, and its old function reads verified positions,
  virtual holdings and Yahoo. BR-200 therefore cannot claim R-08 production
  SourceOnly reachability before BR-199 lands; direct helper tests are not
  production-route evidence.
- Shared compliance-checker edits must be additive against the accepted
  fixed-HEAD assertion inventory. Preserving only R-09 identity is insufficient
  if caller, partition, R-04 governance, dual-test isolation, unique-merge,
  replay, schema or mutation assertions can be deleted.

## 2026-08-02 — BR-200 narrowed-slice conclusion

- The independently realizable BR-200 slice has exactly one live consumer:
  R-04. R-08 cannot become SourceOnly until BR-199 atomically replaces its
  fixed-HEAD `LegacyAccountGate` path; R-09 cannot become live until BR-192
  atomically supplies capability, catalog permit and the complete producer.
- Durable-kind mapping is stable identity, not runtime permission. A separate
  closed capability map is required to prevent a mapped push kind from being
  mistaken for an enabled producer.
- A generic R-08 or `BusinessDateOnce` fixture may test read-only occurrence
  semantics, but it is not production-route evidence and must keep all external
  counters at zero.

## 2026-08-02 — BR-200 first R-04-only review findings

- A rollback verifier must receive the staged candidate commit/tree containing
  the implementation; using current HEAD before the task commit proves the
  wrong source.
- A no-deletion diff is not sufficient to protect a shell checker because an
  inserted early exit can make every retained assertion unreachable. The fixed
  checker bytes need an append boundary plus an execution/mutation self-test.
- Command-to-declaration lookup is only half a test-manifest proof. The reverse
  declaration-to-command set equality must reject an unexecuted 26th test.
- BR-200 does not own BR-199 lifecycle. The independent contract therefore
  marks R-08 typed Unsupported and leaves its durable identity untouched.
- BR-198 is R-09-specific evidence and must not appear in live R-04's authority
  vector; task-specific rule vectors avoid this reverse dependency.

## 2026-08-02 — BR-200 second-review findings

- A baseline checker must not permanently outlaw a separately registered
  future state. The safe form is a finite profile union: current typed
  Unsupported, or the complete independently accepted BR-199 SourceOnly
  authority set; any partial/mixed state is invalid.
- An append digest supplied by the caller proves nothing because the caller can
  bless arbitrary bytes. The release verifier must embed the reviewed prefix
  and append digests and validate the boundary plus both mutation matrices on
  the accepted and patched trees.
- Matching command names to `fn` text does not prove Rust registered or ran the
  tests. Gate B must compare Cargo `--list` output to the canonical manifest and
  require every strict exact command to report exactly one running test.

## 2026-08-02 — BR-200 final-hash review facts

- The append-only BR-194 checker makes accepted BR-194 Gate C a real execution
  prerequisite. Fixed HEAD is factual evidence only; it is not a valid BR-200
  Gate-B base while the inherited three-caller/partition/gate contract is RED.
- A no-producer startup banner requires a startup-owned call site and an exact
  runtime cardinality proof; review-preflight logging cannot substitute.
- Candidate verification must execute the verifier blob extracted from the
  candidate/HEAD object, not a mutable worktree script passed the candidate SHA.
- Real repeated-review evidence needs pre-first watermarks and one joined
  decision/occurrence identity across durable DB, push log, event bus and review
  audit. Same-day broad string counts are not causal evidence.
- A shared business-rule registry is owned row-by-row for this slice. Gate-A
  evidence and rollback must bind the exact BR-200 row, while unrelated dirty or
  staged rows remain outside the BR-200 PR.

## 2026-08-02 — BR-200 final repair decisions

- Fixed HEAD remains reproducible factual evidence, while accepted BR-194 Gate
  C is a separate pinned execution-base prerequisite. This prevents BR-200 from
  inheriting or hiding the still-red BR-194 implementation.
- `monitor --test` is isolation evidence only: live R-04 provider work must be
  blocked there. Authentic R-04 acquisition/reuse is proven only by the phased
  Gate-D verifier against production authorities.
- Exact-one test evidence now needs the requested qualified test line plus a
  `1 passed / 0 failed / 0 ignored / 0 measured` harness summary. `running 1
  test` alone is insufficient.
- Gate-D causality needs binary binding, pre-first watermarks, recomputed
  task/decision/occurrence/provider-evidence identities and a phase hash chain;
  caller-supplied logs and same-day aggregate counts are rejected.
- Current shared BR-194 checker is RED on missing
  `pub async fn push_r08_source_only_with_binding(`. The pinned BR-194 Gate-C
  prerequisite therefore does not yet exist; this is an actual upstream source/
  checker contract repair, not a BR-200 design-document issue.
# 2026-08-02 BR-194/BR-199 checker repair

- `check_br194_review_dependency.sh` was RED because it still searched for a
  public `push_r08_source_only_with_binding`, while production now correctly
  exposes `push_r08_presented_source_only_with_binding` and keeps the
  source-only implementation private.
- The checker now verifies both layers and their ordering independently: the
  public wrapper validates the EventCalendar presentation token before calling
  the private helper; the helper validates canonical R-08 source text before
  the SourceOnly gate/delivery path. Its mutation matrix covers both seams.
- Direct checker validation is GREEN: `BR-194 review dependency static
  contract: PASS`.
- BR-200 Gate A remains RED: its fixed-HEAD 659-line prefix is incompatible
  with mandatory BR-194 Gate-C checker fixes inside that prefix. BR-200 must
  bind the accepted BR-194 checker blob/boundary instead.

## 2026-08-02 — BR-194 clean-master baseline facts

- Commit `9307b67` cannot be treated as accepted Gate-C evidence solely from
  its commit message. A clean checkout fails compilation because
  `src/event/dispatcher.rs` references
  `COUNTED_DELIVERY_AUDIT_SCHEMA_VERSION` while the committed envelope exports
  only `DELIVERY_AUDIT_SCHEMA_VERSION`.
- The same clean checkout fails the dedicated BR-194 static checker before the
  six-reason repair is evaluated: the checker requires
  `fn prepare_review_lhb_delivery(`, but committed `push_templates.rs` exposes
  the R-04 outcome dispatch functions without that named seam.
- A focused-test failure after the bounded repair is currently environmental,
  not behavioral: rustc/clang reported `errno=28` across independent crates.
  Gate evidence must be rerun after artifact cleanup; this failure cannot be
  counted as a test pass or a logic regression.
- Cleanup authority was deliberately narrow: only reproducible Cargo artifacts
  in two abandoned worktrees were removed. The main dirty tree and all live
  data remain untouched.
- The frozen BR-194 design explicitly labels the R-04 SourceOnly entry,
  `prepare_review_lhb_delivery`, closed context source, and caller-wide banner
  removal as Gate-B work. The clean master commit includes the checker and
  design claims but omits `push_templates.rs`, `notify.rs`, `main.rs` and
  `event/envelope.rs` from its changed-file list; this is evidence of an
  incomplete commit boundary rather than a harmless stale checker marker.
- The later factual tree `b4aeee6` contains the missing R-04 prepare seam,
  SourceOnly notify entry and counted-audit schema v3 constant. A direct
  master-to-`b4aeee6` diff is very broad, so those files cannot be copied
  wholesale; ancestry and the minimal owning commits must be identified first.
- The counted-audit compiler failure is not safely repaired by adding one
  constant. The dirty implementation couples schema v3 to new envelope fields,
  canonical counted join hashing, exact rule IDs, `PushRecord` validation and
  persistence/publication entrypoints. Porting only the constant would compile
  one reference while leaving BR-192/2.7 audit validation internally false.
- Git ancestry confirms `9307b67` is an ancestor of `b4aeee6`; however the
  missing event and R-04 production changes are not in the intervening commits.
  They exist as broad uncommitted working-tree changes, which explains why the
  master commit message described behavior its committed tree did not contain.
- The current dirty R-04 implementation cannot be transplanted verbatim as
  BR-194: it already contains later BR-200 occurrence preflight and includes
  `BR-200` in the R-04 transition rule vector. The BR-194 restoration must stop
  at its frozen SourceOnly contract and keep later occurrence work blocked.
- The dedicated checker requires the full chain, not just a function name:
  provider/date/batch/ten-seat preparation, canonical binding/text recheck,
  Launch-before-v14-before-durable ordering, a closed SourceOnly governance
  context, and mutation detection for each seam. Weakening the checker to match
  old master would contradict the accepted BR-194 design and Rule 2.8.
## 2026-08-03 BR-194 continuation evidence

- The isolated `9307b67` candidate reproduced the same deterministic build
  failure after a complete dependency compile: `src/event/dispatcher.rs:501`
  references missing `COUNTED_DELIVERY_AUDIT_SCHEMA_VERSION`; exit 101.
- Independent read-only audit confirms the BR-194 checker is not stale. The
  commit omitted the production R-04/R-09 caller, SourceOnly notify entry,
  dispatcher partition, Magic Eastmoney gateway and exact process tests while
  committing only lower-layer review-batch/v14/durable pieces.
- A one-line constant cannot be admitted: master envelope/push-record remain
  schema v2, so the counted schema-v3 verifier would otherwise be unreachable
  and violate data-audit/fake-implementation rules 2.7/2.8.
- Current dirty-tree implementations contain later BR-197/198/199/200 behavior;
  whole-file copying would change the frozen BR-194 contract. The next safe
  evidence source is the pre-merge Git stash/unreachable object snapshot.
- Unreachable stash commit `2a4d1b929507fadadb082c2a803d5fea50cf6dd8`
  is timestamped 2026-08-01 08:26 +08 and contains both missing seams:
  `COUNTED_DELIVERY_AUDIT_SCHEMA_VERSION = 3`, complete envelope/event/
  push-record support, `prepare_review_lhb_delivery`, and
  `push_counted_source_only_with_binding`.
- A scoped `git grep` found no BR-199/BR-200 marker in those recovered event,
  monitor, gateway or process-test paths. Its index parent is unchanged for
  the same paths, confirming the missing implementation lived in the unstaged
  worktree captured by the stash, consistent with the incomplete commit root
  cause.
- The stash working-tree commit changes the expected tracked integration files
  (`Cargo*`, monitor main/notify/templates, event envelope/mod/push-record,
  `src/lib.rs`, process tests), but does not itself contain `src/data_gateway`
  or `src/event/durable_delivery_append.rs`. Those were untracked at stash
  time and must be inspected from the stash's third parent `1389098...`, not
  inferred or recreated.
- Stash parent `1389098b395a8894578259463923d58ab580a8b6` contains the missing
  untracked production modules, including `src/data_gateway/dragon_tiger.rs`,
  `src/data_gateway/mod.rs`, and `src/event/durable_delivery_append.rs`, plus
  the BR-192 integration/process tests. Scoped search found DragonTiger and
  immutable-append authorities and no BR-197/198/199/200 marker in that tree.
- The stash must therefore be treated as two evidence components: tracked
  working-tree state from `2a4d1b...` plus exact untracked modules from its
  third parent. Reading only the merge tree would silently omit required code.
- A disposable local clone at `/private/tmp/br194-stash-audit.SGsvRr/repo`
  accepted `git stash apply --index 2a4d1b...` cleanly on exact parent
  `9307b67`, restoring both tracked and third-parent untracked content. This
  proves the Git object is a reconstructible pre-merge workspace, not merely a
  loose collection of unrelated blobs. The production repository and its
  databases were not modified.
- The complete reconstructed pre-merge snapshot passes `cargo check --lib`
  against the shared build target (exit 0, 44.57s). This falsifies API-drift as
  the primary cause and confirms the omitted worktree was internally
  compile-coherent at the library boundary.
- Exact test
  `event::envelope::tests::br192_counted_delivery_v3_binds_attempt_artifact_result_and_receipt`
  passes on the reconstructed snapshot (1/1; 2,316 filtered). This proves the
  recovered envelope v3 implementation binds the counted audit fields as the
  missing dispatcher verifier expects.
- Exact push-record test also passes. The persistence-before-success test did
  not reach business logic in the `/private/tmp` clone: audit namespace
  attestation intentionally rejects the world-writable `/private/tmp` ancestor
  (mode 1777) at `dispatcher.rs:1255`. The fixture roots itself under
  `CARGO_MANIFEST_DIR/data/test`, so this is an expected environment-boundary
  rejection, not evidence of a BR-192 semantic failure.
- Re-running from another `/private/tmp` path would repeat the same failure.
  The next test clone must live under the user-owned repository `target/`
  hierarchy so the exact audit-directory ownership contract can be exercised.
- The same persistence-before-success test passes from a reconstructed clone
  under the user-owned repository `target/` hierarchy (1/1, 0.26s test body).
  This confirms the prior failure was solely the deliberate `/private/tmp`
  ancestor rejection and that all three selected BR-192 semantic tests pass.
- The frozen `tools/compliance/lib/check_br194_review_dependency.sh` passes on
  the reconstructed snapshot without modification. This directly proves the
  missing production caller/gateway/SourceOnly/test structure matches the
  design/checker that commit `9307b67` claimed to have shipped.
- The snapshot contains 33 `br194_` tests across monitor/event code plus three
  process-isolation tests in `tests/monitor_help_isolation.rs`; those runtime
  tests remain to be executed before treating the snapshot as a Gate-B repair.
- `cargo test --bin monitor br194_ -- --nocapture` passes 31/31 on the
  reconstructed snapshot (495 filtered, 0 failed). This exercises the R-04
  canonical envelope/source-only gate and terminal replay/audit state machine,
  not just static source markers.
- `cargo test --test monitor_help_isolation br194_ -- --nocapture` passes all
  three process tests (3/3, 21 filtered). This proves `--test --review` blocks
  R-04/R-09 provider and sink before account gates, and malformed terminal
  replay CLI input is rejected before database access.
- Recovered snapshot focused evidence is now: library check PASS, three exact
  BR-192 tests PASS, frozen BR-194 checker PASS, monitor BR-194 31/31 PASS,
  process BR-194 3/3 PASS. Extraction to a minimal clean commit remains the
  next task; the full 213-file stash is not itself an acceptable patch.
- R-04 recovery is not just `dragon_tiger.rs`: the snapshot gateway imports
  shared acquisition hashing/audit and `BatchEvidence/GatewayBatch/GatewayError`
  from the broad `review.rs`. That broad module also owns A-01/R-03, historical
  bars and THS dependencies, so copying it wholesale would violate the narrow
  BR-194 recovery boundary. A small shared evidence/audit seam must be designed
  or the already-accepted upstream gateway prerequisite must be recovered as a
  separately reviewable predecessor slice.
- R-09 is also a true production prerequisite in `push_templates.rs` (canonical
  provider Top-N pair, envelope and counted delivery), so a BR-194-only patch
  cannot truthfully omit its gateway/dependency path while claiming the frozen
  SourceOnly partition is complete.
- The frozen BR-194 design itself remains labelled “Gate A correction after
  independent RED; awaiting re-review,” while its source text explicitly says
  R-04/R-09 provider contracts are prerequisites/adopted old modules rather
  than changes owned by BR-194. Therefore the recovery must first establish a
  separately reviewable predecessor slice for counted delivery + unified
  R-04/R-09 gateways, then apply the BR-194 dependency partition. Treating the
  broad stash as one BR-194 implementation would contradict its own scope.
- Existing BR-162 and BR-194 rules already register the exact R-04 aggregation,
  exact buy-five/sell-five evidence, SourceOnly partition, stable merge and
  failure semantics. Recovery documentation can reference these rows; new
  filtering/sorting semantics must not be invented during extraction.
- Relevant tracked diffs confirm why whole-file recovery is unsafe:
  `main.rs` is +4,298/-5,444, `notify.rs` +3,583/-382 and
  `push_templates.rs` +5,077/-3,414, while the event v3 files are comparatively
  bounded. The monitor files contain the wider unified cutover, so BR-194 must
  be extracted by owned symbols/hunks after its gateway predecessor is made
  explicit.
- The authoritative main worktree remains on `b4aeee68` with extensive staged,
  unstaged and untracked user/project work; `master` remains `9307b67`. Direct
  checkout/reset/merge there would overwrite unrelated work and is prohibited.
  Recovery implementation must stay in the clean isolated clone/branch until
  a reviewed PR boundary exists.
- `check_business_rules.sh` passes on the current worktree (198 rules) with 134
  historical warnings; the new recovery design introduces no blocking Rule
  2.10 failure. Those warnings are not Gate C success and remain separate debt.
- Snapshot R-04/R-09 shared audit is not self-contained: it writes immutable
  acquisition evidence through `DatabaseManager::record_data_acquisition`, so
  P1 also requires the BR-159 acquisition-audit schema/module. The current
  broad `review.rs` couples these shared types to A-01/R-03, THS and historical
  bars; this confirms the exact predecessor closure must be enumerated before
  Gate B.
- The recovered 2026-08-01 snapshot pins Magic revision `d7dfa314...`, whereas
  the authoritative current plan pins a later immutable release. Recovery may
  reuse code semantics but must resolve dependencies to the final single
  release; copying the snapshot Cargo manifest/lockfile would regress the
  unified-version requirement.
- Codebase-design assessment: the shared evidence/error/audit logic is an
  internal deep module, not a new external provider port. Its interface should
  expose `BatchEvidence`, `GatewayBatch<T>`, `GatewayError`, canonical request
  hashing and audited outcome closure while hiding `DatabaseManager` and hash-
  chain persistence. This seam earns locality across R-04/R-09 and the wider
  Gateway family; tests should cross the same interface.
- `CapitalDataGateway` already acts as a deep module: R-09 callers use one
  `provider_top_n_pair(date)` interface while routing, atomic pair validation,
  audit and provider construction remain implementation details. Recovery
  should adopt that module as a predecessor rather than expose its private
  provider/router seams. The exact module dependency closure still needs the
  parallel audit result.
- Parallel dependency audit refined that choice: importing the historical
  `capital.rs` would still compile unrelated fund-flow and HKEX northbound code
  and pull `magic-exchange-rs`. The narrow recovery should instead extract the
  already-tested provider Top-N facts/request/atomic pair/router/admission/audit
  into `provider_top_n.rs` with one `pair(date)` interface. This preserves
  depth while enforcing the recovery scope.
- The audit also proved ordering ownership: BR-162/BR-192 predecessor R-04 must
  first use generic `push_counted_with_binding`; only BR-194 P3 may switch it to
  `push_counted_source_only_with_binding`. Applying the stash call site early
  would mix layers and contradict the predecessor tests.
- Clean `9307b67` already contains durable coordinator/runtime/review-batch/
  v14 lower layers. Recovery must add the missing immutable append adapter,
  `rusqlite/functions`, R-09 reconciliation/hydration producer and then the
  narrow BR-194 caller/partition transition; it must not duplicate committed
  durable code.
- The recovery design has no whitespace errors (`git diff --check` clean) and
  explicitly remains implementation-prohibited pending independent review.
## 2026-08-03 Gate A validation evidence

- The latest BR-192/BR-194 recovery design has no whitespace/error-marker defects under `git diff --check`.
- The business-rule checker exits successfully with 198 registered rules. It also emits 134 historical active-path citation warnings, so a later full compliance run must preserve those warnings as explicit evidence rather than claiming a warning-free gate.
- The complete design now fixes P1/P2/P3 path ownership, the final Magic revision, negative marker exclusions, focused/full/runtime commands, and reverse-order rollback. The remaining Gate-A condition is the independent C0/I0/M0 review.
## 2026-08-03 Gate A review defects

- The recovery design currently contradicts itself about schema work: BR-159 audit schema/trigger initialization is a database migration even when it is additive and idempotent.
- Counted-delivery rollback is a state transition, not merely reverse Git history. Reverting authority while physical delivery or reservations remain unresolved can orphan a sent delivery or permit duplicate execution.
- Existing durable tests explicitly exercise prior-date pending reconciliation, `UncertainManualReview`, reservation release/uncertainty, and zero-local-pending summaries; the recovery rollback must name these states and all-date convergence rather than invent a Git-only shortcut.
- Frozen BR-192 rollback text explicitly says `Never git revert` the atomic runtime commit and forbids deletion of database/WAL/SHM/authorization/reservation/attempt/receipt/disposition/push-log/audit records. The recovery design must inherit this retained-reader/forward-disable model.
## 2026-08-03 authority/version clarification

- `5f1ce93656a55854c844065390520cd4aecd9a14` is not an unregistered threshold/version change in the current tree: it is the exact revision in the active BR-192 rule, 14 direct manifest dependencies, 15 lockfile packages, and `tests/magic_market_release_revision.rs`. Historical `d7dfa314...` recovery bytes are source evidence only.
- A real BR-194 review-join verifier already exists; omitting it from Gate D would permit a false-green when replay or receipt/audit joins are broken.
- `CLAUDE.md` forbids treating uncited command summaries as reproducible design evidence. The reconstructed-stash claim needs an exact evidence block, including the safe repository-owned test path because the `/private/tmp` rejection was environmental and expected.
- `docs/ENGINEERING_RULES_V2.md` fixes the full commands: serial all-target/all-feature tests, llvm-cov JSON plus threshold checker, and release monitor build.
## 2026-08-03 recovery evidence rerun

- The reconstructed stash still compiles as a library from the repository-owned safe clone with shared build artifacts. This confirms the recovery object remains reproducible after the design review; it does not prove current production readiness.
- The exact 31+3 BR-194 count is reproducible and non-zero. Future focused gates must assert those counts rather than accepting a filter command's exit status alone.
## 2026-08-03 Gate-D causal proof

- The checked-in replay command deliberately cannot obtain a real sink. It reuses production validation/terminal classification and fails before sink ownership if a resend would be required.
- The independent verifier binds fixed repository authority paths and joins provider binding, durable occurrence, sink result, push log, delivery audit, hydration, replay start/completion audits, and pre/post watermarks.
## 2026-08-03 immutable append manifest count

- The correct focused acceptance count for `event::durable_delivery_append::tests::br192_` in the recovered tree is exactly nine passing parent tests. The ignored child name is invoked by those parents and is not a tenth direct passing test.
## 2026-08-03 P1 baseline counts

- The recoverable P1 source behavior has a concrete 4+6+3 passing baseline. Refactoring shared evidence into `admission.rs` may add tests, but cannot reduce or rename away the three Provider Top-N validation scenarios or six DragonTiger aggregation/evidence scenarios without an accepted design change.
## 2026-08-03 post-remediation status

- The docs-only remediation introduces no whitespace or business-rule gate failure. It remains Gate-A draft solely because exact hunk identity and fresh independent review are still pending.
## 2026-08-03 historical blob contamination

- The recovery snapshot is not semantically pure even when later BR-199/200 markers are absent: `push_templates.rs` contains BR-160 A-10 behavior. Exact hunk extraction is mandatory; an entire-file copy would violate the design's exclusions.
## 2026-08-03 immutable authority correction

- The earlier statement that active authority had already accepted `5f1ce936...` was too strong. The current dirty worktree is consistent internally, but the fixed HEAD is not. A recovery design may specify the target amendment; it cannot cite uncommitted Cargo/rule/test bytes as already accepted authority.
## 2026-08-03 candidate versus index authority

- The recovery target cannot cite `git write-tree` yet because the shared business-rules file has concurrent staged and unstaged changes. Exact blob review is possible, but final PR staging must revalidate that the accepted row bytes—not an older index version—are present.
## 2026-08-03 recovery classification correction

- “Recover” has three distinct operations that the manifest must label: exact immutable hunk copy, extraction+refactor into a deeper seam, and newly implemented accepted target behavior. Treating all three as byte recovery would make scope and tests unverifiable.
- The notification finalization seam is not deep in the baseline: durable code reaches a broad writer/delivery helper chain. Gate A must either admit and hash that complete chain or redesign a smaller dependency seam before implementation.
## 2026-08-03 notify counted-authority closure expansion

- The recovered durable runtime is not self-contained: `src/event/durable_delivery_runtime.rs` directly owns an `Arc<PinnedPushLogWriter>` and calls `eager_bind_push_log_capability`, `deliver_authoritative_blocking`, and `eager_bind_runtime_artifacts`.
- In the recovered `src/notify.rs`, the dependency closure spans the secure pinned-writer types and filesystem verification/registry/eager-binding implementation (beginning near `PushLogError`/`PinnedPushLogWriter`), the schema-v3 counted finalization adapter, exact pending/audit/commit joins, and the blocking Magiclaw/Feishu CLI receipt helper.
- Therefore P2 cannot truthfully admit only a narrow finalization hunk. Gate A must either (a) bind exact immutable intervals and their full symbol dependency closure, with semantic exclusions, or (b) redesign a smaller deep-module interface and treat the extraction as new implementation. Gate B remains blocked until that choice is reviewed at C0/I0/M0.
- Architecture trade-off: exact admission into `notify.rs` minimizes behavioral change because the reconstructed snapshot already compiles and its counted-delivery tests pass, but it is acceptable only if every admitted interval is semantically counted-authority code. Extracting a new physical-sink deep module improves locality but creates a larger unproven refactor. The default recovery choice should be the smallest complete exact closure unless the manifest proves semantic contamination inside those intervals.
- Fixed baseline `9307b67` already owns the generic Magiclaw/Feishu helper vocabulary (`MessageSendType`, transport resolution, target/bin/home resolution, `CliDeliveryReceipt`, receipt parser). The counted adapter need not import the tracked WIP's many unrelated rewrites of those helpers; it can depend on the baseline definitions plus a bounded blocking receipt wrapper.
- Fixed baseline has the durable `CountedDeliveryBinding` runtime and its tests but no `push_counted*` production entry/caller. This supports a staged recovery: P2 may add unreachable generic/counted authority interfaces and P3 may atomically add the dedicated SourceOnly caller, without an unsafe intermediate production route.
- Candidate immutable `notify.rs` segments from tracked WIP object `2a4d1b9...` now have reproducible SHA-256 identities: lines 1–22 `9a56aa3...` (BR-192 platform contract), 892–2008 `648412bf...` (pinned writer + secure save wrapper), 2340–2453 `03e02ffe...` (counted interfaces), and 2713–3519 `a7b4e069...` (authoritative adapter/finalization/blocking receipt). These are candidate boundaries pending line-completeness and dependency review, not yet accepted manifest rows.
- Marker scan of the secure-writer and authoritative-finalizer candidate intervals found no BR-160/A-10/R-03/R-04/R-08/R-09/selection/token/daemon behavior; the finalizer interval contains only BR-192 markers. This reduces semantic-contamination risk but does not replace call/dependency review.
## 2026-08-03 P2 immutable object manifest evidence

- Tracked WIP object `2a4d1b9...` is a Git stash commit whose first parent is the fixed baseline `9307b67...`; its full event-file blobs therefore represent exact baseline-plus-WIP candidates, not an unrelated branch snapshot.
- P2 exact full-file candidates and SHA-256 values are: `src/event/envelope.rs` lines 1–738 `117179dd...`, `src/event/mod.rs` lines 1–520 `51cfcc6a...`, and `src/event/push_record.rs` lines 1–968 `0f05bb39...` from `2a4d1b9...`; `src/event/durable_delivery_append.rs` lines 1–1841 `1e051097...` from untracked-object commit `1389098...`.
- Diff semantic scan of the three tracked event files found counted schema-v3/BR-192 additions, dispatcher/durable-append exports and their exact tests; no selection, BR-160 or BR-197–200 behavior surfaced. Full-file admission remains subject to the exact manifest review.
- `src/event/mod.rs` directly exports `DurableDeliveryImmutableAppend` and counted audit bind/publish functions; the untracked append file is a complete exact-byte append owner with nine direct BR-192 parent tests and isolated TEST_CODE child probes.
- Gate-order correction: the fixed first parent is already uncompilable because committed `event/dispatcher.rs` references the absent schema-v3 constant. P1 cannot honestly have a green compile gate before P2. The implementable sequence is P2 counted authority first, P1 source prerequisites second, and P3 callers last; P2/P1 remain unreachable from production until P3.
- Reconstructed P2 focused baselines are exact and non-zero: envelope 3/3, push-record 2/2, counted persistence 3/3, immutable append 9/9, and monitor notify BR-192 24 passed with one intentional isolated-child helper ignored. These counts are now frozen in the manifest so a zero-match cargo filter cannot false-green.
## 2026-08-03 P3 immutable manifest closure

- P3 production ranges are now bound for replay parsing, review-only/scheduler hydration, R-04 renderer/binding/SourceOnly dispatch, R-09 rendering/binding/producer refactor and the canonical BR-194 dispatcher. Broad `main.rs` and `push_templates.rs` copy units remain rejected.
- The historical R-09 producer must be an extract/refactor because it calls the forbidden broad `CapitalDataGateway`; the target uses `ProviderTopNDataGateway::pair(date)` without changing request/evidence/order/rendering.
- Two tests need controlled adaptation: an over-broad substring assertion could false-match the dedicated SourceOnly function, and one process test includes unrelated BR-183 DB/selection assertions. The manifest now classifies both as extraction, not exact recovery.

## 2026-08-03 P1 immutable manifest closure

- P1 now has exact immutable-object ranges and SHA-256 identities for the
  shared admission evidence, Provider Top-N, DragonTiger, BR-159 append-only
  acquisition audit and narrow module/database glue.
- Historical `review.rs` is not semantically neutral: it hard-codes both the
  batch capability (`review`) and failed-audit source (`review-data-gateway`).
  The target must accept the fixed capability/source from the admitted Gateway
  and test missing batch evidence attribution; exact-copying those two values
  would corrupt R-04/R-09 audit evidence.
- The four dependency/rule/revision-test files have exact current Git blob and
  SHA-256 identities, but remain target-amendment candidates until a fresh
  independent review binds them. Historical stash Cargo bytes are rejected.
- The manifest no longer has a pending P1/P3 source-range audit. Gate A remains
  open only for fresh C0/I0/M0 acceptance of the complete design, manifest and
  exact candidate blobs.

## 2026-08-03 revision-test scope contamination

- The frozen revision-test candidate passes in the broad dirty worktree, but
  that is not a valid clean-recovery proof: it reads `data_gateway/board.rs`,
  `data_gateway/outcome_daily_bars.rs`, `news/aggregator/raw_v2.rs` and the
  provider-board registry.
- All four paths are absent from fixed baseline `9307b67` and outside the P1
  manifest. Admitting the candidate would either make P1 fail on the clean
  branch or silently expand it with unrelated unified-gateway/selection work.
- The current P1-A4 blob must therefore be rejected. Its replacement should
  prove only the exact 14-direct/15-lock Cargo identity, repository, version,
  revision, absence of sibling paths/obsolete revisions, while the focused P1
  Gateway tests prove API closure.

## 2026-08-03 V3 recovery-closure audit

- Independent P2 review proved all eleven already listed immutable source
  hashes were correct but found thirteen omitted dependencies. The blocking
  omissions were five `ReviewProviderTopN` enum/match closure hunks, both
  generic counted-kind rejection guards, both real namespace-aware
  `save_push_log` calls, test-only writer/fixture hunks and the startup eager
  runtime-artifact bind. The final counted-cutover integration test belongs to
  P3, not P2, because it also requires producer/catalog removal.
- The fixed baseline already references monitor `PushKind::ReviewProviderTopN`
  from committed durable/L5 mappings while its notify enum lacks that variant.
  Therefore those five compile-closure hunks must land in P2 even though the
  R-09 producer and counted-catalog behavior remain P3-owned.
- Independent P3 review found three missing R-04 SourceOnly notify tests and
  their shared fixtures, a missing scheduler-test `sha2` import, and a missing
  counted-catalog increment. `cargo test --bin monitor br194_` closes exactly
  31 tests but does not cover scheduler, R-09, R-04 or catalog filters; those
  groups now have separate exact counts.
- The dependency closure is exact: fourteen direct Magic crates plus
  `magic-market-transport` in the lock, all `0.2.0` at
  `5f1ce93656a55854c844065390520cd4aecd9a14`. Vague
  “resolver-required deltas” are forbidden; an isolated baseline generation
  must freeze target hashes and every changed package record before staging.
- Coverage recovery must preserve all seven baseline core prefixes and add
  exactly `src/data_gateway/`, `src/bin/monitor/` and `src/review/`, retaining
  global/core floors 80/95 and failing closed for empty core input.
- The isolated Cargo 1.95.0 resolution is reproducible: target `Cargo.toml`
  SHA-256 is `11c3b3914089c29e0b10f0bdbc9be1e55ae65a2d77f6ae251624860ad052c877`,
  target `Cargo.lock` SHA-256 is
  `cb2460bc9872143891efdf5c2df8e17318c6cae5210d3c1861e68416626c1935`,
  and `cargo metadata --locked --offline --format-version 1` exits zero after
  environmental cache prewarming. The complete 34-added/eight-removed/seven-
  changed record whitelist is frozen in the recovery manifest; no broader
  resolver drift is authorized.
- A later independent closure audit found that the V3 packet still omitted the
  `durable_delivery_runtime` module declaration and the production startup
  `ensure_startup_reconciled` barrier, conflicted between exact whole event
  files and new golden tests, registered BR-203 after the first P2 source
  slice, left the BR-159 Code-cell choice open, lacked the T7 spy seam and did
  not provide a realizable path from measured 78.16% legacy-core coverage to
  the mandatory 95% core gate. Gate A therefore remains RED while those design
  defects are repaired; no production implementation has begun.
- Fixed-baseline compatibility fixtures are now measured rather than inferred:
  schema-v2 publication is 689 bytes with SHA-256
  `8de344f9fa9b80cbd114474f9299190a7e53a2d57553a4f649d2f4ef9f36bd33`,
  parsed `PushRecord` is 582 bytes with SHA-256
  `bd198be71b8cdc2e3f66b93ac3bc515f6bff453412639176dcb449c1beab6680`,
  and the non-counted push-log artifact is 73 bytes with SHA-256
  `41d03c80490b6c553aba19da0219db7ad3b69527f2bb24f80dfe9a52e496fb6d`.
  The probe used fixed TEST_CODE inputs and called no provider or sink.
- Running the same mini probe against the tracked WIP proved a real schema-v2
  regression: `PushRecord` grew from 582 to 788 bytes (SHA-256
  `e172add31aec501c80520b73e94f1d92d2ac01eed02753f1dc7e94c66118fc9d`)
  because eight schema-v3 `Option` fields serialized as trailing `null` values.
  A temporary compatibility experiment adding
  `skip_serializing_if = "Option::is_none"` to those eight fields restored the
  exact 582-byte baseline hash. P2 must therefore include that reviewed
  compatibility adaptation; the golden must not be updated to accept drift.
# 2026-08-03 BR-192/BR-194 first frozen packet formal RED

- Three fresh independent reviews rejected the first frozen Gate-A packet:
  clean-baseline review `C2/I5/M0`, exact-hunk review `C3/I1/M0`, and
  architecture review `C1/I4/M0`.
- Shared root cause: the packet froze broad dirty-worktree blobs rather than a
  minimal `9307b67`-derived recovery. The BR-192 row imported BR-198/200/202 and
  the forbidden broad `CapitalDataGateway`; Cargo changed unrelated
  dependencies, Polars and profiles; the lock delta was broad; the revision
  test read five absent/out-of-scope paths.
- The packet also lacked a real Git tree/commit identity, per-hunk owner/test/
  splice bindings, four explicit new admission-test rows, an unconditional
  P3-T5 classification, an exact P3-T6A target, and executable non-zero test
  count enforcement. It used the development runner for release replay and
  incorrectly treated plain `--test` as zero-sink isolation.
- The repaired design rejects all four former candidate blobs, specifies a
  minimal baseline-derived Cargo/lock/test transition, resolves the BR-159
  `review.rs` pointer, creates a two-phase raw-source/target-hunk manifest, adds
  the complete splice/owner/test ledger, uses the release binary for replay,
  and assigns isolation only to the three `--test --review` process tests.
- Gate B remains closed until the repaired two-document packet is committed as
  a real Git object and receives a fresh independent `C0/I0/M0` verdict.

## 2026-08-03 clean-lineage compile isolation

- The user was correct about the current target dependency graph: root
  `Cargo.toml` uses Polars `0.54`, `Cargo.lock` resolves Polars `0.54.4`, and
  `qmt-parser` is absent. References to Polars `0.46`/`0.52` describe the
  historical `96da674` parent, not the desired working-tree state.
- A clean `git archive 96da674` build is independently RED with five errors:
  two missing `SelectionAuditEnvironment` imports, two missing
  `SelectionAuditWriter::for_environment` calls, and one missing
  `COUNTED_DELIVERY_AUDIT_SCHEMA_VERSION`. This disproves the previously
  proposed `P0-A3 -> BR-164-only -> P0-A4 -> P2` sequence because no isolated
  BR-164-only green predecessor has been demonstrated.
- Commit `d43ce8a` deliberately hardened production selection audit creation by
  removing the caller-controlled environment/path constructor, but retained
  callers in `selection/outcome.rs` and `selection/pipeline.rs`. The repair
  must adapt those callers to the strict production constructor and fail
  explicitly; resurrecting `for_environment` would undo the safety fix.
- The isolated P2 candidate closes the counted-delivery compile error and then
  stops only on those four selection caller errors. Its source closure is
  useful, but its Cargo target is rejected because it retains old Polars/qmt
  and introduces a dual graph.
- Therefore the next executable Gate-A design must specify a minimal green
  compile foundation containing the strict selection caller adaptation and
  complete schema-v3 counted-delivery recovery. BR-164 domain migrations can
  then be applied as separately reversible commits from that green lineage.
- A constant-only patch is invalid: the historical parser accepts only schema
  v2, so defining schema version 3 without the complete envelope/persistence/
  verification closure would compile while remaining semantically unusable.
- The exact current 0.54/no-qmt Cargo target is not a valid predecessor for the
  old source tree. An isolated compile produced fourteen unresolved imports for
  dependencies already removed with later legacy modules plus one Polars 0.54
  `LazyFrame::drop(Selector)` API mismatch. Therefore dependency cleanup and
  source retirement must move together by domain; they cannot precede P2.
- A second isolated candidate retained the historical dependency graph and
  changed only `rusqlite` features from `chrono` to `chrono,functions`, added a
  direct same-path `magic-market-core = 0.2.0`, added the complete P2 source,
  and adapted both strict audit callers. Locked/offline metadata and library
  check passed. This is the first independently green recovery lineage.
- That predecessor intentionally still contains historical qmt and Polars
  0.46/0.52; it is an intermediate business-recovery commit, not the release
  target. BR-164 domain cutovers must remove their old callers/dependencies and
  end at the already prepared Polars 0.54.4-only/no-qmt target.
- The green check emitted one `dead_code` warning for
  `SelectionAuditWriter::for_test_code_pinned_root`, which later global-schema
  code consumes but the predecessor does not. P2-F must close this warning
  explicitly without making the API public or restoring caller-selected
  production paths; strict Clippy remains a blocking validation.

## 2026-08-03 candidate authority and isolation proof correction

- `git hash-object <file>` computes an object identity but does not make it
  reachable. Because `target/br203-candidate` is ignored and has no Git tree,
  all candidate target identities remain provisional until one real P2-F
  commit with parent `96da674...` materializes them.
- The candidate compile verifier already locks the actual strict-selection
  SHAs (`13f3cbb8...`, `bc345454...`); the recovery documents had stale values
  and were the inconsistent side of the contract.
- The CLI integration helper only snapshots fixed production
  `durable_delivery.sqlite3{,-wal,-shm}`. It cannot substantiate a claim about
  event-audit, push-log or immutable-audit roots. Those authorities require
  their own focused TEST_CODE namespace tests.
- For the SQLite trio, no-follow metadata including inode, device, length,
  mode, mtime and ctime detects creation/replacement/content mutation without
  opening or hashing real account data. Three runtime-crossing terminal
  regressions assert the before/after fingerprint; the full 19-case suite
  passes serially.
- A design table does not freeze code unless a machine verifier checks the same
  bytes. P2-V0B previously locked only Cargo/lock/selection; it now also locks
  all eight composed H11/H12/H13 targets and the two manifest-defined raw line
  ranges. The wrapper still passes and its own target identity is separately
  frozen for the future reachable commit.
- A source commit cannot name the historical pre-document parent after a
  docs-only authority commit is materialized. Doing so creates a sibling P2-F
  branch that omits the accepted rule/design tree. The non-circular contract is
  structural: P0-A4 is a reviewed docs-only direct child of P0-A3, then P2-F
  must be the direct child of the accepted P0-A4 authority HEAD; the exact
  relation is verified from Git objects after each commit.
