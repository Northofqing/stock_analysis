# Findings & Decisions

## Requirements
- User wants all stock_analysis data acquisition aligned with magic-market-data-rs, explicitly including financial and news data.
- Production data must be real; upstream contracts without provider implementations remain Unsupported/Unavailable.
- Both repositories must converge on one maintainable crate version and preserve source time, batch/provenance and freshness evidence.
- For news sources not yet implemented in magic-market-data-rs, preserve sources with unique value and reliable provider publication time by moving their acquisition upstream; remove duplicate, chronically stale or incomplete-evidence sources.

## Research Findings
- Current dependency graph resolves exactly one Polars release,
  `polars 0.54.4`; the former qmt-parser-owned duplicate graph is absent.
  All thirteen direct Magic dependencies are pinned to the same immutable
  `0.2.0` revision `b2b68df78156df1d67824e5c44c0cb01b752f55a`.
- 2026-07-27 current-state reconciliation: all downstream Magic crates resolve
  to one immutable published revision
  `b2b68df78156df1d67824e5c44c0cb01b752f55a`, version `0.2.0`.
- NewsAI is no longer a disabled evidence-less consumer: its opt-in Shadow
  producer accepts only sealed admitted news/market evidence, binds one real
  upstream model receipt, and writes an immutable hash-chain assessment without
  push, prediction publication or order side effects.
- All Gateway security requests now use BR-173 canonical identity resolution.
  Provider-specific BSE support remains narrower where live capability evidence
  proves only `920xxx`; unsupported `921xxx–929xxx` values fail explicitly.
- Magic TDX provenance currently emits Unix-second `observed_at` values for
  lifecycle batches. Treating those as RFC3339-only was a downstream parser
  defect; the fixed parser accepts exact declared Unix seconds/fractions without
  changing any freshness or manual-confirmation policy.
- Daily freshness now passes with `MAX(stock_daily.date)=2026-07-27`. The
  688548 and 688690 histories remain behind at 2026-07-24 because their exact
  provider batches contain adjacent changes just above 20% and correctly await
  explicit BR-171 operator confirmation.
- The strict review path now completes successfully with real Gateway batches:
  R-03, A-10 and R-08 delivered; A-01 correctly reported no exact target-day
  T+1 observation instead of replaying an old record; R-04 remained an
  expected pre-21:00 wait. This supersedes older findings that described the
  review dispatcher as a mixed legacy path.
- The remaining local `src/data_provider` files are mostly domain records,
  caches and analytics facades over Gateways. Their stale comments still claim
  direct Sina/Eastmoney/Tencent acquisition and must be corrected before the
  architecture evidence is trustworthy.
- `chain_registry.rs` remains a substantive red-line risk: production callers
  can fill industry-chain labels from a manually maintained static table. It
  needs a separate Gate-A decision to replace it with admitted Magic board
  evidence or retire the enrichment; a comment-only rename is insufficient.
- The static registry has exactly three remaining production consumers:
  `position_tracker::query_chain_exposure`, `portfolio::store::get_positions`
  and `DatabaseManager::backfill_chain_name`. All three currently lose source
  batch/time evidence. The unified `BoardDataGateway::memberships` already
  delegates to Magic TDX `BoardMembershipProvider`, so the replacement should
  prefetch a verified membership batch and persist its provenance rather than
  perform a synchronous static lookup during reads.
- BR-085 and BR-123 still explicitly allow the static registry, while the final
  Magic-only architecture and Data Redlines 2.1/2.7 prohibit that source. The
  business rules must be revised before implementation. `stock_position`
  currently stores only `chain_name`; it cannot prove which provider batch
  assigned the value. A provenance-bearing assignment record (and a direct
  link from the current position value) is required for traceability.
- BR-170 is the next free business-rule ID. Migration must also clear any
  existing `stock_position.chain_name` that cannot be linked to a verified
  assignment; otherwise deleting the static registry would leave its old
  values silently active in SQLite.
- CFFEX delivery remains a genuine upstream/external live-admission blocker:
  the current `magic-exchange-rs` capability reports unsupported after the
  official HTTPS probe failed. Formula, stale cache and insecure HTTP are not
  acceptable substitutes.
- stock_analysis currently depends directly only on the path crate `magic-tdx-rs`; most financial/news call sites still use local providers.
- magic-market-data-rs now contains broader crates than the earlier audit: `magic-eastmoney-rs`, `magic-cls-rs`, `magic-cninfo-rs`, `magic-iwencai-rs`, `magic-ths-rs`, `magic-sina-rs`, provider contracts and `magic-market-router`.
- All public provider crates except `magic-market-analysis` are aligned at version `0.2.0`; `magic-market-analysis` remains `0.1.0`.
- Confirmed provider implementations from source:
  - Magic TDX: security metadata, daily/minute bars, quotes, trades and order books (sync/async client variants).
  - Tencent/Sina/Baidu: market-data alternatives; Sina additionally implements financial statements and ETF option data.
  - Eastmoney: research reports, instrument news, dragon-tiger data, limit pools, board/fund flows, margin, block trades, holder counts, lockups, dividends and popularity.
  - CLS: global news; CNInfo: announcements and investor questions.
  - THS: consensus, strong-stock reasons, limit pools and popularity; iWencai: semantic search.
  - EMQuant: quotes/bars/money flow with several explicitly unsupported capabilities.
- Explicit `Unsupported` paths exist and must survive translation into stock_analysis; notably CLS instrument-specific news is not implemented, and some period/range/capability combinations are rejected.
- Upstream documentation states:
  - CLS real probe passed on 2026-07-23 with five complete telegrams and a two-request serial load probe.
  - iWencai requires an operator-supplied authorized `MAGIC_IWENCAI_API_KEY`; without it, semantic search must remain unavailable.
  - Eastmoney public-web capabilities are intended for post-close research/backfill/cross-validation, not a five-second realtime SLA; its 15:35 post-close Top10 semantic is explicitly not yet verified.
  - THS implements only the limit-up pool among its limit-pool variants; broken-board, limit-down and previous-limit pools remain unsupported there.
  - CNInfo and CLS do not run schedulers, caches or persistence; stock_analysis must own those concerns.
- magic-market-data-rs already carries a stock_analysis handoff document that assigns realtime Quote/Kline/OrderBook to market providers and keeps account, AI, push and trading decisions outside the provider repository.
- stock_analysis has substantial duplicated acquisition code: local Eastmoney/Sina/Tencent/Baostock market providers; local financials/consensus/industry/money-flow modules; local announcement providers; and many independent news/search providers.
- Production `monitor --review` still bypasses the planned Gateway entirely:
  `run_review_only` enters `dispatch_post_session_review`, but R-03 calls the
  local industry endpoint plus `fallback::fetch_kline_with_fallback`, A-01
  calls `DataFetcherManager::get_daily_data`, A-10 uses the same manager for
  security names, R-04 issues a direct Eastmoney datacenter request, and R-08
  calls the local announcement provider plus Yahoo. R-02/R-05/R-06 are
  hard-disabled by `review_preflight`. The old startup banner therefore
  describes the real active path; it is not stale logging.
- The current feature worktree contains no compiled `src/data_gateway` module.
  The approved Gateway exists only as design commit `8fe06b0`; downstream
  implementation and call-site cutover have not started.
- `stock_analysis::data_provider::KlineData` currently mixes bar facts with valuation, financial, consensus and industry enrichment. A direct type replacement would be too broad; the gateway must translate market batches separately from enrichment batches.
- Router type aliases alone are not provider evidence; each selected domain still needs implementation and live-probe verification.
- Current event-selection Gate D found the duplicate chain configuration and nested-runtime call sites; those fixes remain unmerged on the feature branch.
- The upstream workspace now passes fmt, strict all-feature clippy, full all-target
  tests, compliance and docs-link checks in the isolated release worktree.
- The latest bounded TDX live probe exposed a release blocker: the raw server
  returns Friday transaction cache on Saturday, and the normalized current
  trade adapters currently admit it. Normalized current minute/trade requests
  need a non-trading-session admission gate before the upstream baseline can
  be published.
- The TDX release blocker is now repaired in the isolated upstream worktree:
  normalized current minute/trade requests reject weekends and off-session
  windows before transport while raw cached packets remain explicitly
  diagnostic/unadmitted. `magic-tdx-rs` has 278 passing library tests, two
  passing live-probe tests, strict clippy, and a real weekend probe ending in
  `live_probe_status=passed`.
- The main repository `monitor --test` reaches its E2E completion after local
  audit/database isolation fixes, but repeated runs still leak recommendation
  output outside the test namespace and accumulate duplicate fixture trades.
  Those are BR-051/BR-136 blockers, not acceptable release evidence.
- A reproducible baseline scan on the current main worktree finds 39 Rust files
  containing direct URLs for Eastmoney/CNInfo/CLS/Jin10/WallStreetCN/Xueqiu/
  Sina/Tencent/Baidu/THS/iWencai data. There are 55 Rust files referencing a
  direct HTTP client. Infrastructure-only notification/LLM endpoints must be
  classified separately, but this proves the financial/news cutover is far from
  complete and gives the final architecture check a concrete zero-target.
- AI/content audit: NewsAI has no production caller and is explicitly disabled;
  the deep analyzer still flattens local tool output into strings without
  provider/batch evidence, and strict `--review` does not invoke that deep-AI
  path. Existing Magic providers can cover financial statements, THS
  consensus, Eastmoney research/LHB, CLS/Jin10/ThePaper and per-security
  announcements, but full-market announcement discovery, instrument news,
  official policy sources and some report-body/PDF semantics remain upstream
  gaps. PushKind/template IDs must stay stable during the cutover to preserve
  dedup and audit continuity.
- The approved Slice 0 plan is saved in the upstream isolated worktree at
  `docs/superpowers/plans/2026-07-23-magic-market-data-slice-0-baseline.md`
  and committed as `b758ee309bf63909a904222b0c07b88bf2df609d`.
- Slice 0 uses the Magic repository's BR-009 through BR-011 for Provider
  capability admission, public request bounds/pacing and duplicate identity.
  The stock repository's BR-158/BR-159 remain a Slice 1 Gateway obligation.
- Final-state requirement: once a downstream domain is connected through the
  Magic gateway and passes real parity/shadow evidence, delete the old direct
  acquisition implementation, dependency/configuration surface and fallback.
  No legacy financial/news HTTP path may remain merely for compatibility.
- Real Eastmoney Dragon-Tiger schema evidence includes a source `TRADE_ID`.
  A security can legitimately have multiple records on one trading date for
  distinct `EXPLANATION`/`TRADE_ID` values. The old `code:date` identity is
  therefore a data-loss bug; all-market discovery must preserve the source
  trade identity, collapse only exact duplicates, and fail on conflicting
  duplicate identities.
- The downstream A-01/R-03 slice has completed Gate A. It must migrate all
  four A-01 callers (v13 diagnostics, dry-run, daily and noon) and both R-03
  callers (legacy v12 and strict dispatcher), not only the visible review
  dispatcher. Empty real batches must keep provider provenance instead of
  being replaced with a synthetic/default batch.
- Current upstream hard capability gaps, distinguished from downstream wiring:
  1. no admitted instrument-specific news provider (`Eastmoney`, CLS, Jin10
     and ThePaper all explicitly reject instrument-news promotion);
  2. no full-market announcement discovery request/provider, only
     per-instrument CNInfo/SSE/SZSE queries;
  3. no production provider for the existing board-membership, market-ranking
     and concept-hit contracts, and no board-constituent discovery contract;
  4. no admitted full-market Dragon-Tiger discovery contract yet (the current
     provider contract is per instrument/date; implementation is in progress);
  5. Eastmoney public fund-flow is callable but capability admission remains
     false, and its generic board flow is not the exact 15:35 post-close Top10
     contract;
  6. research metadata and PDF URL exist, but report-body/PDF downloading is
     explicitly disabled;
  7. no typed official-policy source for NDRC/MIIT/Gov.cn;
  8. no global-index/FX/economic-calendar or full-market corporate-event
     discovery family required by R-08;
  9. no futures-contract delivery calendar needed for advance delivery-day
     reminders (ETF option contracts are a different supported family).
- These upstream gaps do not explain all unfinished work. Financial statements,
  THS consensus, Eastmoney research metadata, all four Eastmoney limit pools,
  per-instrument announcements, per-instrument Dragon-Tiger entries/seats,
  global CLS/Jin10/ThePaper news, HKEX northbound daily Top10, and several
  capital-event families already have real providers; the remaining obstacle
  for those families is stock_analysis Gateway/caller migration and legacy
  deletion.
- The isolated release worktree already implements the canonical
  `HistoricalBars<Bar = magic_market_core::Bar>` contract directly on
  `TdxSmartClient`. A downstream `TdxCoreBarsProvider` that re-normalizes
  `SecurityBar` would create two independent validation/evidence definitions
  and would fail once the upstream release is merged. The Gateway must register
  `TdxSmartClient` directly and keep all normalization in magic-market-data-rs.
- The adjacent upstream `main` is still older than the isolated release
  worktree, so downstream compilation against `../magic-market-data-rs` is not
  authoritative until the upstream release PR is merged. Temporary validation
  may point an isolated copy at
  `target/magic_market_unified_work`; committed manifests must continue to use
  the adjacent repository path.
- The legacy Sina instrument-news endpoint is no longer a valid production
  contract. `feed.mix.sina.com.cn` with the historical page id returns business
  status 11 ("list/page not registered"); the only responding global-feed page
  ignores the requested stock code. Treating those rows as instrument news
  would violate source identity and audit red lines. The replacement must use a
  request-scoped official Sina company-news page (or another independently
  verified official instrument endpoint) and must fail closed unless every row
  carries provider time, canonical URL and request-bound instrument identity.
- A-10 is not an isolated loader migration. Its input `chain_daily` is written
  only when the live summary pipeline first calls the legacy blocking
  `MarketAnalyzer::get_limit_up_stocks` and then
  `pipeline::chain_analysis::run_chain_analysis`. The latter still acquires
  concepts, board discovery/constituents, LHB, research/search and news through
  local HTTP/tool code. This explains the repeated stale `chain_daily as_of`
  review failure: `monitor --review` consumes the cache but does not own a
  same-date refresh. A correct A-10 slice must migrate and schedule the
  chain-production batch, not merely replace the final stock-name lookup.
- The production deep-analysis path still calls the old K-line fallback plus
  six local tools (`financials`, `research`, `news`, `sector`, `chip`,
  `fund_flow`) and flattens their output to untyped strings. Although missing
  tools are labelled, successful inputs carry no immutable provider/batch
  evidence into the AI report. The unified AI slice therefore needs a
  Gateway-created evidence inventory and must remove these acquisition tools
  after equivalent upstream capabilities are wired.
- R-08 still performs direct local Eastmoney announcement acquisition and
  Yahoo global-index/FX calls inside the review path. Per-instrument
  announcements can migrate now, but the current report semantics also require
  full-market announcement/corporate-event discovery and global macro
  contracts that remain genuine upstream gaps.
- On 2026-07-25 the user reported that `cargo run --bin monitor -- --review`
  still visibly uses legacy acquisition. This matches the recorded plan state:
  Phase 4 production call-site cutover is still pending. The presence of an
  untracked `src/data_gateway/` directory is not integration evidence; every
  active R-03/R-04/R-08/A-01/A-10 caller must be traced from
  `dispatch_post_session_review` to its concrete provider before the symptom
  can be considered fixed.
- The current worktree has partially advanced beyond the plan text:
  `src/lib.rs` exports `data_gateway`, R-03 calls
  `ReviewDataGateway::r03_upper_limit_pool`, and A-01 calls
  `ReviewDataGateway::a01_daily_bars`. Cargo resolves four Magic crates at
  `0.2.0` from `target/magic_market_unified_work`. Therefore the user-visible
  legacy behavior is a mixed-path cutover, not a total failure to link Magic.
- `dispatch_post_session_review` concurrently owns five active tasks:
  R-03, R-04, R-08, A-10 and A-01. Only R-03 and A-01 have source-level
  Gateway assertions today. R-08 still exposes local announcement types and
  R-02 (currently preflight-disabled) still calls the local market snapshot.
  R-04/A-10/R-08 remain the primary suspects for legacy runtime logs.
- Current source has advanced again: all five active strict-review tasks now
  enter `data_gateway` (`ReviewDataGateway`, `DragonTigerGateway`,
  `EventCalendarGateway`, `ChainIntelligenceGateway`). The user-visible
  "old acquisition" symptom is now repository-wide rather than a strict
  review-dispatcher defect: ten production call sites still invoke legacy
  `data_provider` acquisition names in the normal monitor, opportunity,
  multi-timeframe, earnings and market-overview paths.
- The existing BR-164 architecture test catches external hosts and Magic
  provider imports outside `data_gateway`, but it does not reject calls to
  legacy `data_provider::{financials,money_flow,intraday_kline,north_flow,
  announcement}` acquisition APIs. Those APIs were converted to explicit
  unsupported stubs, so the test can pass while production still calls old
  entry points and repeatedly fails. A source-level acquisition-call guard is
  required.
- `cargo run --bin monitor -- --review` was unable to start during diagnosis
  because concurrent `cargo test`, two `cargo check` processes and Polars
  compilation held the shared artifact lock. No process was killed; the
  queued diagnostic invocation was cancelled after the lock remained held.
- All fifteen Magic dependencies resolve to the same immutable `0.2.0`
  revision `4f2730b6...`; that merge commit contains remote main
  `73e17a9...`. Multiple Magic versions are not the cause of the symptom.
- Exact mixed-path evidence:
  - R-04 imports and invokes local
    `market_analyzer::lhb_review::fetch_recent_lhb`.
  - R-08 directly calls local
    `data_provider::announcement::fetch_announcements` and
    `data_provider::yahoo::fetch_overnight_data`.
  - A-10 reads `load_catalyst_review_snapshot_real`, which consumes the local
    chain/rotation cache rather than a Gateway batch.
  These are compile-time direct calls, so no runtime configuration can make
  the current `--review` invocation use Magic for those tasks.
- The initial A-01/R-03 Gateway slice retained evidence in memory and emitted
  structured logs, but BR-159 also requires a durable acquisition audit and
  provider-state transitions. Delivery/review audit cannot substitute for
  acquisition evidence. The new database boundary now supplies an immutable
  hash chain; the Gateway public methods still need to make audit persistence
  part of their success/failure contract before this slice can pass Gate D.
- A valid instrument-news source page whose records all fall outside the
  requested date range is a provider-proven empty result, not a transport or
  protocol failure. The new Sina provider must retain page provenance and
  return an empty complete batch for that case, while a missing/empty source
  datelist remains an explicit protocol failure.

## Technical Decisions
| Decision | Rationale |
|----------|-----------|
| Recommended end shape is one stock_analysis data gateway with domain methods | Deep interface centralizes translation, governance and source routing |
| Migrate in vertical slices rather than deleting all local providers at once | Each slice can prove parity and rollback independently |
| Upstream capability matrix is the design input | Prevents false claims that contracts are implemented |

## Issues Encountered
| Issue | Resolution |
|-------|------------|
| Scope spans independent data domains and two repositories | Decompose into slices; brainstorm and approve the first slice before implementation |
| The new magic Eastmoney/CLS/CNInfo/THS/iWencai/Baidu provider crates are currently untracked in the upstream working tree | Treat them as in-progress upstream Slice B, not as a released dependency; complete upstream gates and commit/version evidence before downstream production cutover |
| Both repositories have extensive unrelated/in-progress working-tree changes | Preserve them; use scoped commits/PRs and isolated validation trees instead of overwriting or bulk staging |
| The overall design is approved but downstream ReviewDataGateway does not exist | Treat every current direct review loader as unmigrated; implement the deep seam before deleting any source |

## Resources
- `stock_analysis/Cargo.toml`
- `stock_analysis/src/data_provider/`
- `magic-market-data-rs/crates/magic-market-core`
- `magic-market-data-rs/crates/magic-market-router`
- `magic-market-data-rs/docs/integrations/`
# 2026-07-27 BR-170 persistence notes

- `DatabaseManager::run_migrations` owns creation order; the new assignment
  table must be created only after `stock_position` exists because the current
  stock table is created inside that function, below the existing standalone
  schema initializers.
- `DatabaseManager::add_column_if_missing` is the established safe upgrade seam
  for `stock_position.chain_assignment_id`; Diesel models may omit the physical
  column while raw SQL owns the evidence link.
- `ChainIntelligenceStore` provides the local pattern for `IMMEDIATE`
  transactions, content-hash idempotency/conflict checks and immutable
  UPDATE/DELETE triggers.
- Magic TDX board membership provenance intentionally has `source_at=None`;
  BR-170 must preserve that absence and must not replace it with process time.
- The first BR-170 Task 5 design has an ownership contradiction for new
  candidates: `commit_position_chain_assignment` intentionally rejects an
  assignment when no open `stock_position` exists, while
  `position_tracker::track_position` currently tries to obtain the candidate's
  chain by reading an already-linked open position. A new candidate therefore
  cannot legally reach the sizing or open-order path.
- Resolving that contradiction requires the async analysis owner to acquire and
  derive a complete `PositionChainAssignment` before entering synchronous
  trading logic. `OpenPositionCmd` must carry the validated assignment, and
  the simulated execution database boundary must append the order audit,
  insert the position, append/idempotently verify the assignment, and link the
  projection in one transaction. Persisting a raw `chain_name` or inserting an
  unlinked assignment would break AGENTS 2.1/2.7 and BR-170.
