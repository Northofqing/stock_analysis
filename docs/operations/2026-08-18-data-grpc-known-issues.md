# 2026-08-18 Data and gRPC Known-Issue Ledger

This is the live operational ledger for provider-data failures and gRPC
contract/implementation defects observed during the opening-push recovery.
It records evidence; it does not authorize fallback data, relax freshness, or
replace the append-only acquisition and delivery audits.

## Status vocabulary

- `OPEN`: reproducible defect with no validated correction.
- `IN_PROGRESS`: correction is being implemented but has not passed the stated
  acceptance check.
- `DEGRADED`: the system remains usable through an admitted independent route,
  while the named route stays unavailable.
- `RECOVERED`: a live probe recovered, but no code correction explains or
  prevents recurrence.
- `FIXED`: regression test, required gates, rebuilt binary, and live evidence
  all passed.

Never change an old observation to make the incident appear healthy. Append a
new dated evidence note and then change the status.

## Active issues

### GRPC-20260818-001 — P-01 `LimitPools` was not wired end to end

- Status: `IN_PROGRESS`
- Affected path: P-01/P-02 upper-limit-pool acquisition and opening push.
- First confirmed: 2026-08-18 09:11--09:22 +08:00.
- Symptom: the no-default-feature monitor selected the gRPC data path, but
  `ReviewDataGateway::current_upper_limit_pool` returned
  `library transport disabled: DATA_GATEWAY_GRPC=1 required`.
- Root cause: `LimitPools` existed as an operation name, but the monitor lacked
  the full client method and the server exposed a flattened chain view rather
  than the complete P-01 `LimitPoolEntry` contract.
- Required contract: exact request
  `{"kind":"Upper","trading_date":"YYYY-MM-DD","limit":200}`; response
  preserves all `LimitPoolEntry` fields plus original record and batch
  `provider/source/source_at/observed_at/batch_id` evidence. Missing, mixed,
  truncated, wrong-date, duplicate, or over-limit data fails explicitly.
- Current work: client/converter, server delegate, synchronous consumer seam,
  documentation, and focused tests are being completed in parallel.
- Acceptance: focused client/server/consumer tests pass; server and monitor are
  rebuilt with their production feature sets; an authenticated live RPC returns
  an admitted exact-date batch; a P-01-equivalent delivery has a real receipt
  and immutable audit join.

### GRPC-20260818-002 — filtered quote batch was reported as complete

- Status: `OPEN`
- Affected path: position quote batch, DataMode Quote, intraday valuation, and
  any consumer requiring an exact requested-code set.
- Confirmed: 2026-08-18 09:36:55 +08:00.
- Evidence: request ID `quote-positions-20260818T0933` asked for the seven
  attested position codes. Acquisition audit ID `399694`, request hash
  `1e097356ba48b573cfc78c59bd6d8d8bdd778c4fc088421a13207cf045b60a13`,
  records Tencent `available/accepted_count=6`, while the RPC envelope said
  `complete=true`.
- Failure mode: provider records that fail the five-second gate can be excluded
  inside the server, but the remaining batch is still packed as complete.
- Safety behavior: the client exact-code check rejects the six-of-seven batch;
  missing quotes are not filled from cache, previous close, or another batch.
- Required correction: the server must not advertise complete when any
  requested identity is excluded. It must either obtain an exact complete batch
  from one admitted provider or return a typed partial/stale error carrying the
  real excluded identities and evidence.
- Acceptance: deterministic seven-code regression test catches 6/7; live
  seven-code request is either exact 7/7 or an explicit non-complete failure.

### DATA-20260818-001 — opening-auction realtime quotes did not meet 5 seconds

- Status: `RECOVERED` for a single liquid code; full position batch remains
  blocked by `GRPC-20260818-002` and per-record freshness.
- Affected path: OpeningLive readiness, A-02/P-05, DataMode Quote, intraday
  valuation.
- Evidence:
  - 09:16: the upstream Sina row had a zero current price and was rejected by
    the positive-price gate.
  - 09:27: a real RPC for `000813` returned `quote_stale`; provider source time
    was 09:25:00 and admission time was 09:27:10.
  - 09:30: request ID `quote-diag-20260818T092945` succeeded through Tencent;
    audit ID `399561` records one admitted record with source time 09:30:15.
- Safety behavior: zero, future, and older-than-five-second prices remain
  rejected under red lines 2.3/2.4. `observed_at` must not replace `source_at`.
- Follow-up: continue live checks across liquid and illiquid holdings; isolate
  provider-specific timestamp semantics without widening the five-second gate.

### DATA-20260818-002 — GlobalNews provider routes are degraded

- Status: `DEGRADED`; CLS and Jin10 have formed the two-provider admitted
  quorum. ThePaper remains unavailable; Eastmoney has been intermittent.
- Affected path: provider diversity and news/AI industry-chain inputs.
- Evidence:
  - ThePaper repeatedly returned
    `native The Paper row unexpectedly has an external link` before a verified
    batch could form.
  - Eastmoney has intermittently rejected a `bond.eastmoney.com` article host,
    but later formed an admitted batch; the historical rejected audit did not
    retain enough message detail to attribute every old attempt.
  - Jin10 initially rejected a row missing `vip_level`, then produced repeated
    admitted batches. It is therefore recovered, not proven fixed.
- Safety behavior: a failing provider stays typed and excluded. One provider's
  batch must not be relabelled as another provider, and no URL/field allowlist
  is widened without a source-contract change and regression fixture.
- Acceptance: each restored provider independently produces an admitted batch
  with provider/source/count/evidence identity preserved.

### GRPC-20260818-003 — client error mapping drops safe server detail

- Status: `OPEN`
- Affected path: diagnosis and audit explainability, not admission authority.
- Symptom: tonic `Status.message()` contains the first provider/contract
  rejection, but `GrpcError` retains only the coarse typed taxonomy. Historical
  `invalid_evidence` audit rows therefore cannot identify the exact predicate.
- Required correction: preserve a bounded, secret-safe diagnostic message for
  logs and operator evidence while keeping program branching exclusively on the
  typed error code and retryability.
- Acceptance: regression tests prove useful status detail survives, bearer
  tokens, private keys, certificate contents, and request payload secrets never
  appear in `Display`, `Debug`, logs, or audits.

### DATA-20260818-003 — HistoricalBars has no verified batch for `000813`

- Status: `OPEN`
- Affected path: intraday valuation fallback and portions of post-session
  review that require daily bars.
- Confirmed: monitor runtime on 2026-08-18 returned typed
  `no_verified_batch` for the same real instrument while realtime acquisition
  was also unavailable.
- Safety behavior: no synthetic daily bar and no unverified cached value is
  substituted.
- Required diagnosis: run the production HistoricalBars RPC with an exact date
  range, retain provider attempt taxonomy, and determine whether the first
  rejection is transport, parser, completeness, continuity, or one-trading-day
  freshness.
- Acceptance: exact requested dates and record continuity pass with original
  batch evidence, or the route remains explicitly unavailable.

### MONITOR-20260818-001 — P-01 scheduler is unreachable before open

- Status: `OPEN`; Gate A correction is BR-241.
- Affected path: resident P-01 09:00--09:15 owner.
- Confirmed: 2026-08-18 read-only call-chain inspection.
- Evidence: `market_loop` first blocks in `while !is_market_active()` at
  `src/bin/monitor/main.rs:8155`; the P-01 state and due branch are initialized
  only afterward at lines 8298--8343. The branch requires a closed session and
  `09:00 <= now < 09:15`, so its requirements cannot coexist. Its volatile
  `preopen_pushed` state is also assigned from `preopen_ok && candidate_ok`,
  coupling an accepted P-01 to independent P-03 failure.
- Safety behavior: do not start a second monitor and do not use grouped
  `--push` as compensation; outside the pre-open window it can dispatch A-01
  and A-10 instead of P-01.
- Required correction: install a P-01-only resident owner before the
  market-active wait, use durable Global `BusinessDateOnce`, separate P-03,
  and add an exclusive production compensation command that acquires the same
  monitor lease.
- Acceptance: boundary/restart tests pass; a controlled single-owner release
  produces one real P-01 Accepted receipt; repeated compensation and resident
  restart produce zero additional Feishu messages.

### DATA-20260818-004 — P-01 persisted inputs are stale and one dependency has no producer

- Status: `OPEN`; Gate A source correction is BR-241.
- Affected path: P-01 completed-day source composition and rendering.
- Confirmed: 2026-08-18 production DB and caller inspection.
- Evidence: `chain_daily` latest date was 2026-08-14 and
  `board_rotation_daily` latest date was 2026-07-16. Both loaders independently
  select `MAX(date)`. Repository search found `save_board_rotations` callers
  only in DAO tests, so the rotation table has no production writer. Current
  P-01 also does not consume the exact LocalBridge `LimitPools` batch.
- Safety behavior: do not mix latest dates, relabel
  `OpeningStatic-UpperLimitPoolReview`, invent a rotation producer, synthesize
  names/news, or treat verified-empty news as a headline.
- Required correction: for P-01 business date D, resolve
  `evidence_date=prev_trading_day(D)`; acquire exact
  `LimitPools {Upper,evidence_date,200}`; derive/persist the exact chain solely
  from that batch; resolve the exact top-head set through SecurityIdentity; and
  query each head through `SinaInstrumentNewsGateway::instrument_news_in_range`
  over `[evidence_date,D]`. All evidence and exclusions enter one canonical
  binding. `board_rotation_daily` is retired only from P-01; other callers stay
  unchanged.
- Acceptance: Tuesday-to-Monday and Monday-to-Friday tests pass; exact request,
  chain receipt, identity exact-set, every per-head news batch, rendered hash,
  and delivery decision join to the same P-01 occurrence.

### DELIVERY-20260818-001 — P-01 has no durable typed receipt or exact join

- Status: `OPEN`; Gate A durable correction is BR-241.
- Affected path: `PreopenNewsHot` governance, sink authority, deduplication,
  compensation, and audit.
- Confirmed: 2026-08-18 code and production-audit inspection.
- Evidence: monitor `PushKind::PreopenNewsHot` is absent from durable
  `PushKind` and `durable_kind_and_sub_kind_with_override`; the current
  dispatcher reaches generic boolean `deliver_and_record`. The day's only
  confirmed Feishu delivery was `data_mode_v1`; there was no P-01 dispatcher
  attempt, `preopen_news_hot_v1` Accepted receipt, durable decision/attempt, or
  exact audit join.
- Safety behavior: DataMode, BR-196 test delivery, another PushKind, a local
  log, transport handshake, or boolean success cannot prove P-01. An uncertain
  remote result must not be blindly resent.
- Required correction: add durable `PreopenNewsHot/preopen_news_hot_v1` as
  Global BusinessDateOnce and daily-budget-exempt; perform generic
  `inspect_business_date_once_claim` before providers; create the complete
  P-01 `CountedDeliveryBinding`; and deliver only through existing
  `notify::push_counted_with_binding`. Late compensation must say
  `盘前热点补发` and `依据前一交易日`, not impersonate the 09:00 card.
- Acceptance: typed Feishu Accepted includes non-empty local/platform IDs;
  receipt hash, sink-result hash, decision, attempt, immutable audit, source
  binding, render hash, and committed artifact pass exact join; crash recovery
  and same-day repeat cause no second sink call.

## Runtime facts that are not open defects

- The notification outlet is functional: event-audit record
  `04376bb334283fd312bc37355df58677dd8f58306c897d7c08f3479ca689c2a3`
  records a confirmed Feishu `data_mode_v1` delivery at 2026-08-18 09:11:46
  +08:00. This does not prove P-01 or the other business dispatchers.
- `magic-*` dependencies remain in the root package because the
  `grpc_market_server` still owns the real provider-library transports. The
  production monitor is built with `--no-default-features` and must consume
  those sources through RPC. Physical dependency removal requires a server /
  monitor crate split or complete external-contract migration.
- R-03 still requires a real account/cash observation no older than 30 seconds.
  The user-attested screenshot is immutable account evidence, not a live-action
  freshness grant.
- R-08 remains unavailable until an admitted official CFFEX evidence route is
  published; it must not be represented as verified empty.

## Update procedure

For each new failure:

1. Reproduce with the narrowest production-equivalent RPC or consumer call.
2. Record local time, request ID, operation, provider, typed reason code,
   retryability, audit ID/hash, and affected consumer. Do not paste credentials
   or full sensitive payloads.
3. Classify it as provider data (`DATA-*`) or local contract/implementation
   (`GRPC-*`). If uncertain, say so and keep the evidence boundary explicit.
4. Add the safe next action and an acceptance check. Do not mark `FIXED` from a
   listener/banner, process exit code, or a single transport handshake.
5. Append recovery/fix evidence instead of deleting the original observation.
