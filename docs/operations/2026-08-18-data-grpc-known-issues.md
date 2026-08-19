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

- Status: `FIXED`; the corrected P-01 consumer and BR-238 opening gate both use
  the exact `LimitPools` route.
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
- Historical scoped evidence: client/converter/delegate P-01 tests and the
  no-feature consumer tests passed 2/2 and the full-record preservation test
  passed 1/1. The 2026-08-18 P-01 compensation used this route and produced
  one durable Feishu `Accepted` receipt; the same-day second run performed no
  provider acquisition and no second sink call. That evidence proves the P-01
  occurrence only; it does not prove the BR-238 static opening route set.
- Acceptance: focused client/server/consumer tests pass; server and monitor are
  rebuilt with their production feature sets; an authenticated live RPC returns
  an admitted exact-date batch; a P-01-equivalent delivery has a real receipt
  and immutable audit join. The opening report must itself name exact
  `LimitPools`, then fresh Gate C and independent review must pass.
- Corrective evidence: fresh Gate C passed; rebuilt isolated and production
  probes both exited zero and named `LimitPools`; the corrected monitor repeated
  the same gate before resident producers started. No `UpperLimitPoolReview`
  compatibility payload was accepted as the required route.

### GRPC-20260818-002 — filtered quote batch was reported as complete

- Status: `IN_PROGRESS`; implementation regression is fixed, next live market
  window evidence remains required.
- Affected path: position quote batch, DataMode Quote, intraday valuation, and
  any consumer requiring an exact requested-code set.
- Confirmed: during the protected 2026-08-18 opening diagnostic; the exact
  account-linked observation time is intentionally omitted.
- Evidence: a private exact-set position request produced a proper strict
  subset after provider admission, while the RPC envelope still said
  `complete=true`. The account-linked request identity, audit locator, set
  cardinality, and security identities remain only in the protected acquisition
  audit and are intentionally omitted here.
- Failure mode: provider records that fail the five-second gate can be excluded
  inside the server, but the remaining batch is still packed as complete.
- Safety behavior: the client exact-code check rejects any proper subset;
  missing quotes are not filled from cache, previous close, or another batch.
- Required correction: the server must not advertise complete when any
  requested identity is excluded. It must either obtain an exact complete batch
  from one admitted provider or return a typed partial/stale error carrying the
  real excluded identities and evidence.
- Fix evidence: `data_gateway::market_data::tests` passed 30/30. A stale,
  future, missing, duplicate, out-of-order, or evidence-conflicting member now
  rejects the provider attempt instead of returning a fresh subset as complete.
- Acceptance: a deterministic `TEST_CODE` exact-set regression catches a proper
  subset; a private live request is either set-equal or an explicit
  non-complete failure, without publishing account cardinality.

### DATA-20260818-001 — opening-auction realtime quotes did not meet 5 seconds

- Status: `RECOVERED` for an issue-scoped public diagnostic instrument; the
  private position batch remains
  blocked by `GRPC-20260818-002` and per-record freshness.
- Affected path: OpeningLive readiness, A-02/P-05, DataMode Quote, intraday
  valuation.
- Evidence: during the opening diagnostic, one upstream row with a zero current
  price failed the positive-price gate and a later request failed `quote_stale`.
  A subsequent independent Tencent request admitted a fresh record. Exact
  request/audit locators, timestamps, and instrument identity remain in the
  protected acquisition audit and are intentionally omitted here.
- Safety behavior: zero, future, and older-than-five-second prices remain
  rejected under red lines 2.3/2.4. `observed_at` must not replace `source_at`.
- Follow-up: continue live checks across representative public diagnostic
  instruments; isolate
  provider-specific timestamp semantics without widening the five-second gate.

### DATA-20260818-002 — GlobalNews provider routes are degraded

- Status: `DEGRADED`; the admitted two-provider quorum is currently CLS and
  ThePaper. Eastmoney and Jin10 remain intermittent.
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
  - The 2026-08-18 19:49 production cutover admitted independent CLS and
    ThePaper batches and explicitly excluded Eastmoney and Jin10. The opening
    probe reported a two-provider GlobalNews quorum; no failed provider was
    relabelled or filled from another batch. A later review invalidated the
    overall opening result because a separate mandatory route was not exact, so
    this observation supports only the independent news batches.
- Safety behavior: a failing provider stays typed and excluded. One provider's
  batch must not be relabelled as another provider, and no URL/field allowlist
  is widened without a source-contract change and regression fixture.
- Acceptance: each restored provider independently produces an admitted batch
  with provider/source/count/evidence identity preserved.

### GRPC-20260818-003 — client error mapping may expose unsafe server detail

- Status: `FIXED`; the replacement uses closed typed fields and redacts all
  unclassified free-form detail.
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
- Rejected historical evidence: the first implementation normalized and
  truncated `Status.message()` and screened a short marker list. That cannot
  establish safety for cookie values, alternate credential labels, arbitrary
  payloads, or future upstream prose, and its old tests/Gate-C run must not be
  cited as acceptance.
- Required correction: use a closed safe diagnostic vocabulary or a structured
  server-owned detail field; unclassified free-form status text is redacted.
  Typed code/reason/retryability remain the only program-control inputs.
- Remaining acceptance: focused tests cover bearer/private-key/certificate,
  API-key, cookie, request-payload and arbitrary-text cases; formatting,
  strict Clippy, full tests and compliance pass on the final source; rebuilt
  binaries retain only admitted canonical detail in authenticated evidence.
- Corrective evidence: focused secret/admission regressions and fresh Gate C
  passed; rebuilt authenticated probes and the corrected monitor emitted only
  the admitted canonical reason vocabulary for degraded routes.

### GRPC-20260818-004 — InstrumentNews range loses the caller's instant upper bound

- Status: `OPEN`; P-01 adds a consumer-side fail-closed guard, while the RPC
  contract remains to be corrected.
- Affected path: P-01 per-head InstrumentNews acquisition and any caller that
  requires an instant upper bound within the current local date.
- Confirmed: 2026-08-18 code-path inspection of
  `SinaInstrumentNewsGateway::instrument_news_in_range` and its gRPC bridge.
- Failure mode: the bridge converts `(to-from)` to an integer `from_days`, and
  the server chooses its own `Local::now().date_naive()` as the end date. The
  ExternalV1 converter proves only that records fall in the inclusive local
  date range, so it cannot prove that a same-day `published_at` is no later
  than the caller's captured `observed_at` instant.
- Safety behavior: P-01 retains the original admitted record/batch evidence and
  rejects the entire binding when any `published_at > captured observed_at`;
  it does not silently filter the row, rewrite the timestamp, or substitute a
  cached headline.
- Required correction: make the RPC request carry the caller's explicit start
  and end instants (or a separately frozen exact end-date/end-time contract),
  and validate the same bounds on both server and client without using server
  wall time as request identity.
- Acceptance: fixed-clock round-trip tests prove an exact upper-bound record is
  accepted, a record one nanosecond later is rejected, supported timestamp
  encodings compare by instant, and the original evidence is unchanged.

### GRPC-20260818-005 — latest client bundle documentation and proto are inconsistent

- Status: `OPEN`; non-blocking for the current P-01 route.
- Confirmed: 2026-08-18 local read-only inspection of the user-supplied latest
  `client-bundle`.
- Evidence: `grpc-external-api.md` says 60 read-only RPC families are in the v1
  proto and assigns operations 56--60 to `IndexQuotes`, `IntradayShape`,
  `T0Evidence`, `OutcomeDailyBars`, and `UpperLimitPoolReview`. The delivered
  `market.proto` enum and `MarketDataService` still end at operation/RPC 55
  `InstrumentNews`, and the referenced `grpc-derived-products.md` is absent.
- Current scope: P-01 uses delivered `LimitPools` operation 44 and
  `InstrumentNews` operation 55, so this packaging mismatch does not authorize
  delaying or replacing either P-01 source.
- Safety behavior: clients must not infer or code-generate the missing five RPC
  contracts from prose, and must not treat a document version timestamp as a
  wire contract.
- Required correction: publish one self-consistent bundle whose proto contains
  every advertised method/enum value and whose referenced derived-product
  contract is included; bind all artifacts to one release manifest/hash set.
- Acceptance: clean-directory code generation exposes exactly 60 methods,
  derived request/record schemas are present, and document/proto counts and
  hashes agree.

### DATA-20260818-003 — HistoricalBars had no verified batch for an issue-scoped instrument

- Status: `FIXED`.
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
- Fix evidence: the provider now over-fetches before close, validates the raw
  batch, removes only the exact forming current-session bar, and returns the
  exact settled count. The focused regression and historical module 11/11
  tests passed. The 2026-08-18 resident runtime admitted the exact settled TDX
  range for each independently validated valuation request; account-linked
  instrument identities are intentionally omitted.

### MONITOR-20260818-001 — P-01 scheduler is unreachable before open

- Status: `FIXED`; BR-241 correction is implemented and live-proven.
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
- Fix evidence: P-01 has an independent resident owner plus exclusive
  compensation path. The 2026-08-18 compensation produced decision
  `efc5d2366154cbc883610535758a49d307e96b86f050ef56542bc3925146f98c`
  with one authoritative Feishu receipt; the second same-day compensation
  returned already-delivered with unchanged acquisition/attempt/sink counts.

### DATA-20260818-004 — P-01 persisted inputs are stale and one dependency has no producer

- Status: `FIXED` for the P-01 path; `board_rotation_daily` remains unrelated
  legacy debt for any other consumer.
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

- Status: `FIXED`; BR-241 durable correction is implemented and live-proven.
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
- Live evidence: decision state is `Delivered`; claim count is one; the exact
  attempt and sink result are `Accepted`; local/platform message IDs,
  accepted-at, delivery-audit reference, immutable disposition append, and
  source/render binding join are present. Pending immutable audits are zero.

### DATA-20260818-005 — ExternalV1 transport and InstrumentNews routes are intermittent

- Status: `DEGRADED`.
- Confirmed: during a protected 2026-08-18 post-close diagnostic; exact
  account-linked times are intentionally omitted.
- Affected path: external SecurityMetadata/InstrumentNews startup and the
  bounded post-close account-news backfill.
- Evidence: a non-sandbox production monitor established the authenticated
  channel and admitted independent account-news batches, while some requested
  identities returned typed `FailedPrecondition/external_query_unavailable`.
  A subsequent production-equivalent probe reached SecurityMetadata for an
  affected issue-scoped identity but failed InstrumentNews; the next connection
  attempt failed before health with
  `bundle transport not ready`. Earlier sandboxed resident attempts failed the
  connection repeatedly while the identical unsandboxed probe succeeded.
- Safety behavior: failed instruments remain explicit failures; no cached,
  fabricated, or cross-instrument news is substituted. The resident continues
  with admitted independent sources and retains each acquisition audit.
- Next action: deploy `GRPC-20260818-003`, capture the first safe server detail
  for each failed instrument, and determine whether the cause is provider
  verified-empty handling, record evidence, rate limiting, or transport.
- Later evidence: the historical release run admitted and persisted only
  independent batches that passed validation. Affected issue-scoped identities
  failed on separate HTML-contract predicates before a batch formed and
  remained explicit non-retryable `external_query_unavailable` outcomes with
  zero substituted records. The mapping between predicates and private
  securities remains only in protected acquisition evidence.
- Acceptance: repeated exact-range calls for every privately requested identity
  produce either an admitted/verified-empty batch or the same stable typed
  rejection with actionable safe detail; the bounded backfill records a
  terminal outcome without publishing account cardinality or identity mapping.

### DATA-20260818-006 — Cninfo Announcements alternates between admitted and router rejection

- Status: `DEGRADED`.
- Confirmed: 2026-08-18 18:04--18:19 +08:00.
- Evidence: raw local RPC `announcements-diagnose-20260818-1807` returned an
  admitted complete Cninfo batch and the production startup later admitted 100
  records, but the resident NewsMonitor subsequently recorded
  `router_batch_rejected` with audit ID `404298`.
- Safety behavior: rejected batches are isolated from NewsMonitor and are not
  converted to empty success. Other admitted news sources continue.
- Later evidence: the 19:46 authenticated probe admitted 100 records;
  the first 19:47 monitor startup rejected a later batch and exited before
  producers; a direct 19:48 replay admitted 100 records; the historical 19:49
  restart reported opening readiness, but that overall result was later
  invalidated by a non-exact mandatory route. The first strict R-08 attempt again
  observed `router_batch_rejected` and a later acquisition recovered. This is
  provider/batch intermittency, not a fixed client-bundle contract defect.

### GRPC-20260818-006 — CFFEX FuturesDelivery capability is unpublished

- Status: `OPEN`.
- Confirmed: 2026-08-18 19:49 +08:00 production strict review.
- Affected path: R-08 event-calendar review.
- Evidence: the server returned typed `provider_unsupported` for the official
  CFFEX delivery component. R-08 therefore remained failed even when the
  Cninfo announcement batch recovered.
- Safety behavior: no empty calendar, inferred delivery date, or substitute
  provider is treated as official CFFEX evidence. The failure is retained in
  the review transition and acquisition audit.
- Acceptance: an official, exact-date CFFEX batch is admitted with original
  provider evidence, or the business rule is explicitly redesigned at Gate A
  to make the unpublished component a typed disabled task rather than a
  deliverable R-08 dependency.

### DATA-20260818-007 — R-03 account metrics are not within 30 seconds

- Status: `OPEN`.
- Confirmed: during the protected 2026-08-18 post-close strict review; exact
  account-linked observation time is intentionally omitted.
- Affected path: account-dependent R-03 review only.
- Evidence: an operator-provided historical account snapshot was retained in
  the protected append-only store without reproducing its time, holdings, or
  balances here, but the production
  account-action gate requires a real account capture no older than 30 seconds.
  R-03 returned typed `account_metrics_incomplete` before provider/render/sink.
- Safety behavior: the snapshot is not retimestamped and no stale balance is
  used as live account authority.
- Acceptance: an authenticated real-account connector supplies position and
  cash evidence within 30 seconds, or R-03 remains explicitly unavailable.

### GRPC-20260818-007 — synchronous bridge can wait forever after an inner RPC deadline

- Status: `FIXED` under BR-243; production library compile and focused
  behavioral tests pass. Full Gate C/D and the next live window remain pending.
- Confirmed: 2026-08-18 19:16 +08:00 from the 18:13 monitor process sample.
- Evidence: `/private/tmp/monitor-80482-20260818-1916.sample.txt` retained the
  monitor path in `GrpcSource::realtime_quotes -> std::thread::join ->
  pthread_join`; another worker retained the same bridge path for daily bars.
  The previous synchronous helper joined the complete future without an outer
  deadline, so mutex acquisition, connection, retry/backoff, RPC and conversion
  could collectively outlive the inner per-RPC timeout.
- Safety behavior: BR-243 adds one literal 20-second complete-future deadline
  inside the runtime that owns the future. Expiry drops the future there before
  the scoped helper thread is joined and returns typed retryable
  `grpc_bridge_sync_timeout`; it never becomes empty, cached or default data.
  Existing completed success and failure classifications remain unchanged.
- Validation evidence: fresh `cargo check --lib` exited 0 after making the
  timeout error generic over both `GatewayError` and
  `OutcomeTransportFailure`. Focused tests cover a pure synchronous caller, a
  Tokio-runtime caller, a future DropProbe, the literal production deadline and
  completed result/error preservation. Fresh
  `cargo test --lib data_gateway::grpc_source::tests::br243 -- --nocapture`
  passed 6/6 with zero failures after the shared test target became compilable;
  the sixth test pins the typed `OutcomeTransportFailure` timeout envelope.
- Acceptance: the focused BR-243 tests pass; formatting, clippy, full tests and
  compliance pass; then the next live window shows either a real admitted batch
  or typed `grpc_bridge_sync_timeout` within the total deadline, with no orphan
  bridge worker and no scheduler starvation.

## 2026-08-18 19:49 historical release-cutover observation (not acceptance)

- One release server owned `127.0.0.1:18082`; one release monitor owned the
  production delivery lease.
- The opening probe reported readiness and a two-provider GlobalNews quorum,
  but later review proved it substituted `UpperLimitPoolReview` for exact
  `LimitPools`. The overall opening result is invalid and must not be cited as
  Gate-C, BR-238, release, or merge acceptance.
- Closing valuation reached an explicit persisted terminal result; the earlier
  HistoricalBars join hang did not recur.
- The strict review delivered R-04, R-09, R-11, R-13, and A-10. Each decision
  has `Delivered`, an authoritative Feishu `Accepted` result, non-empty local
  and platform message IDs, `task_transition_payloads=Appended/Applied`, and
  `immutable_audit_outbox=Appended`.
- R-07 remained the registered `source_not_published` wait until 21:00. R-03
  and R-08 remained explicit failures described above; R-02/R-05/R-06/R-12
  remained registered disabled capabilities rather than fake successes.
- Next action: use the deployed bounded diagnostic detail to record the exact
  Cninfo router predicate on recurrence, then fix the provider contract or
  upstream record validation rather than relaxing the batch gate.
- Acceptance: repeated full-day announcement calls are admitted or verified
  empty with original provider/batch evidence, and the NewsMonitor poll no
  longer alternates to `router_batch_rejected`.

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
  The protected historical snapshot is immutable account evidence, not a
  live-action freshness grant; its time, holdings, balances, and identities
  must not be copied into documentation or PR evidence.
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
