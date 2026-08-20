# Client-Bundle Opening Readiness Design

Date: 2026-08-17
Target session: 2026-08-18 A-share pre-open/open
Status: Gate A design selected under the user's standing project authorization
Rules: AGENTS 2.1, 2.2, 2.3, 2.4, 2.7, 2.8, 2.10; BR-091, BR-103,
BR-112, BR-113, BR-114, BR-116, BR-159, BR-164, BR-168, BR-188, BR-213,
BR-216, BR-217, BR-218, BR-220, BR-221, BR-223, BR-225, BR-226, BR-227,
BR-231, BR-236, BR-238.

## 1. Outcome

The production monitor must be able to acquire the real public-market inputs
needed by the 09:00-09:30 push chain through the authenticated `client-bundle`
contract, while retaining the current local gRPC service as a reversible
fallback until the remote canaries pass.

Success means:

1. the bundle is loaded without logging or committing its private key or token;
2. startup verifies remote TLS identity, health, and the required static
   admitted runtime capabilities before static producers start, while live
   capabilities remain a separate background gate;
3. external request schemas and canonical record schemas are translated at one
   boundary, not in individual push renderers;
4. provider, acquisition authority, source time, observation time and batch ID
   survive the boundary;
5. incomplete fields remain absent and unadmitted diagnostics never become
   production push facts;
6. the 09:00 P-01, 09:10 quote preflight, 09:20-09:25 P-02/A-02/P-05 and the
   concurrent news/announcement/board-index loops either receive admissible real
   data or emit a typed blocker and retain retry eligibility.

This work does not fabricate a fresh broker account snapshot. BR-103 remains a
separate real-account boundary: account-dependent advice stays conservative
until same-batch broker account/trade watermark evidence exists. Public-data and
source-only pushes must not be suppressed merely because that separate evidence
is unavailable.

## 2. Reproduced facts

### 2.1 Runtime failures

The existing single production monitor and local data service were inspected;
a second monitor was not started because the process has no proven singleton
lock and duplicate instances could duplicate delivery.

Observed failures:

- position-chain warmup: `requested=7 assigned=0 verified_empty=0 failed=7`;
  five failures were `source 空 (服务端未回填证据链)` and two were cancelled;
- no-feature monitor: `SecurityIdentity` reported
  `library transport disabled: DATA_GATEWAY_GRPC=1 required`;
- the news L2 concept index called `memberships_blocking`, which bypassed the
  existing asynchronous gRPC branch and failed by the same no-feature boundary;
- selection-v2 recovery tick repeatedly reported
  `production_database_connection_unverified`; current source declares this
  unreleased capability disabled, so a rebuilt binary must not treat it as an
  opening readiness dependency;
- the real account snapshot was stale and cannot lawfully be refreshed from
  public market data.

### 2.2 Code boundary evidence

Reproducible command:

```bash
rg -n 'fn pack|pack\(|pack_ev\(|memberships_blocking|security_identities' \
  src/grpc_server/delegate.rs src/data_gateway/board_runtime.rs \
  src/data_gateway/market_capabilities.rs src/data_gateway/grpc_source.rs
```

Relevant result:

```text
src/grpc_server/delegate.rs:144:fn pack(...)
src/grpc_server/delegate.rs:150:source: String::new()
src/grpc_server/delegate.rs:797:async fn fetch_board_constituents(...)
src/grpc_server/delegate.rs:830:pack(records, source_at)
src/data_gateway/board_runtime.rs:286:pub fn memberships_blocking(...)
src/data_gateway/market_capabilities.rs:464:pub async fn security_identities(...)
```

`pack()` clears provider/source/batch identity. The response handler then
invented a compatibility provider and a new batch ID while forwarding the empty
source. The client correctly rejected that response. This is the root cause of
the five deterministic warmup failures; weakening `evidence_of()` would only
hide it and is prohibited.

### 2.3 External contract evidence

Read-only bundle probes established:

- mutual TLS 1.3 succeeds and verifies `magic-market.local`;
- `GetHealth`: `live=true`, `ready=true`, `state=ready`;
- `GetCapabilities`: 98 provider-operation rows, 81 admitted and runtime
  available; these are not 98 operations;
- live reflection exposes operations 0..60, while the checked-in bundle proto
  ends at operation 55;
- `MoneyFlows`, `MarketRankings` and `MarketBreadth` are diagnostics and cannot
  support production push decisions;
- `SecurityMetadata` returns one canonical payload per record, not a local JSON
  array, may be `complete=false` because listing/limit metadata is absent, and
  still carries admitted source-backed name/ST identity fields;
- the upstream response has no local extension field `source=11`.

Sanitized real response shape:

```text
operation=SecurityMetadata admission=ADMITTED provider=Tencent
batch_id=tencent-web:<immutable-id>:security-metadata
observed_at=<provider evidence timestamp>
source_at=<explicit offset timestamp>
record.schema=magic.market.security_metadata
record.data={instrument,name,board,is_st,listed_on:null,price_limit:{...},
             status,source_at,observed_at,provider,batch_id}
```

No credential value is part of this document or the recorded command evidence.

### 2.4 Gate-redesign evidence

The first readiness implementation exposed three independent correctness
failures. These are Gate A findings, not release evidence.

Reproducible command:

```bash
rg -n 'external_opening_readiness|OPENING_REQUIRED_EXTERNAL_CAPABILITIES|with_capacity\(8\)|off_session_static_quote_eligible|off_session_quote_keepalive_loop|global_news_async\(\)|fetch_global_news\(\)' \
  src/bin/monitor/main.rs src/data_gateway/market_data.rs \
  src/data_gateway/global_news.rs src/data_gateway/grpc_source.rs \
  src/grpc_server/delegate.rs
```

Relevant result:

```text
src/bin/monitor/main.rs:4384:external_opening_readiness().await
src/bin/monitor/main.rs:4819:off_session_quote_keepalive_loop()
src/data_gateway/grpc_source.rs:226:const OPENING_REQUIRED_EXTERNAL_CAPABILITIES: &[Operation] = &[
src/data_gateway/grpc_source.rs:231:Operation::Announcements,
src/data_gateway/grpc_source.rs:232:Operation::MarketAnnouncements,
src/data_gateway/grpc_source.rs:233:Operation::BoardConstituents,
src/data_gateway/grpc_source.rs:234:Operation::BoardMemberships,
src/data_gateway/grpc_source.rs:235:Operation::LimitPools,
src/data_gateway/grpc_source.rs:237:Operation::UpperLimitPoolReview,
src/data_gateway/grpc_source.rs:519:let mut routes = Vec::with_capacity(8);
src/data_gateway/market_data.rs:783:fn off_session_static_quote_eligible(...)
src/data_gateway/market_data.rs:908:if off_session_static_quote_eligible(...)
src/data_gateway/global_news.rs:115:bridge.global_news_async().await
src/data_gateway/grpc_source.rs:559:.global_news_async()
src/grpc_server/delegate.rs:240:Operation::GlobalNews => fetch_global_news().await
src/grpc_server/delegate.rs:522:.global_news(GlobalNewsProvider::Eastmoney, 20)
```

The synchronous readiness loop occurs before strategy registration and all
producer loops, so a quote that cannot be fresh before its live publication
window can suppress the 09:00--09:15 P-01 public/static window. Separately,
BR-236 currently admits a same-day off-session static quote through the
`RealtimeFiveSecond` path and the keepalive caller marks `Capability::Quote`;
this converts a non-realtime fact into realtime authority. Finally, the gRPC
GlobalNews branch discards the caller's provider and limit, while the server
always requests Eastmoney/20. Four registered feed attempts can therefore
consume the same Eastmoney batch under four logical names unless response
evidence is checked against each registration. The capability list also treats
both members of three alias pairs as mandatory and the eight-route matrix omits
`T0Evidence`, so capability discovery can both false-red on a valid alias and
false-green without a core live route.

The selected correction is a deeper two-phase readiness module: one small
interface returns typed static-startup and live-session reports, while contract
selection, provider binding, current-time freshness and mode behavior remain
inside the module. Callers do not receive a general-purpose Boolean that can
substitute for their own evidence checks.

## 3. Opening dependency map

| Window / family | Current producer | Required public data | Required external capability |
| --- | --- | --- | --- |
| 09:00 P-01 | `dispatch_preopen_news_hot_daily` | persisted chain/news rows plus missing head-stock identity | `SecurityMetadata` identity subset |
| 09:00 P-03 | candidate dispatcher | existing candidate evidence | no new external fact unless a retry refresh is requested |
| 09:10 preflight | `fetch_realtime_quotes` | exact requested live quotes | `RealtimeQuotes` |
| 09:20-09:25 P-02 | auction scanner | source-backed limit pool and admitted candidate facts | `LimitPools`/`UpperLimitPoolReview`; diagnostics must not replace them |
| 09:20-09:25 A-02/P-05 | candidate reload and push | candidate evidence plus exact live quotes | `RealtimeQuotes`, identity where display-only |
| 09:30 P-05 valuation | virtual observation | exact live quotes | `RealtimeQuotes` |
| 120-second news loop | news/announcement owners | news, announcements, security identities | `GlobalNews`, `MarketAnnouncements`/`Announcements`, `SecurityMetadata` |
| L2 concept index | `memberships_blocking` | exact stock-to-board memberships | `BoardMemberships` or normalized `BoardConstituents` |

An RPC being present is insufficient. Startup readiness uses the intersection of
repository admission and runtime availability for each required capability.

## 4. Alternatives considered

### A. Authenticated remote transport with one normalization boundary — selected

`GrpcMarketClient` gains an explicit client-bundle profile. The profile loads
mTLS material and Bearer auth, maps local operation parameters to the frozen
external request schema, and supplies a truthful acquisition authority such as
`grpc-mtls:magic-market.local`. `grpc_source` then normalizes upstream canonical
records into existing gateway types and preserves upstream provider/batch/time
facts.

Advantages: uses the requested remote source, keeps push code unchanged, and
concentrates schema/version checks. It also permits a local-server fallback by
leaving the existing plaintext profile intact.

### B. Turn `grpc_market_server` into a second proxy process — rejected for this slice

This would isolate external translation in the server but adds another process
mode, recursion prevention, remote health propagation and deployment ownership
under an opening deadline. It remains a future topology option after one full
trading-day observation.

### C. Re-enable all `magic-gateway` dependencies in the monitor — rollback only

This can restore local provider access but does not solve the `client-bundle`
TLS/schema contract and reintroduces provider libraries into the consumer
process. It is retained only as an emergency rollback after explicit evidence;
it is not the target architecture.

## 5. Selected design

### 5.1 Configuration and secret boundary

New opt-in environment variable:

```text
GRPC_MARKET_CLIENT_BUNDLE=/absolute/path/to/client-bundle
```

When present together with `DATA_GATEWAY_GRPC=1`, the bridge loads
`connection.json` and resolves `ca`, `certificate`, `private_key` and
`bearer_token` relative to the canonical bundle directory. It validates protocol
version 1, an HTTPS endpoint, a non-empty TLS server name, regular files and
non-empty token. Paths may not escape the bundle directory. Errors identify only
the field/file role, never secret content.

`GRPC_MARKET_ADDR` continues to identify the normalized local plaintext bridge.
The configured bundle is an additional authenticated client, not a wholesale
replacement for that bridge. Dispatch is closed and operation-specific: only
fixture-proven external operations may use the bundle; every other operation
remains on the local normalized service. A configured external operation never
silently falls back after TLS/auth/schema/evidence failure. If the bundle is
absent, all existing local operations keep their current path and the externally
owned identity/news readiness checks report unavailable explicitly.

### 5.2 Transport and auth

The tonic channel uses a CA certificate plus client identity and pins the TLS
domain from the bundle. The Bearer token is held in a zeroizing value and added
to request metadata by the client instance; it is never copied into an
environment variable, URL, request payload, log or debug output.

### 5.3 Contract profiles

`ContractProfile::LocalBridgeV1` retains local short schemas and JSON-array
records. `ContractProfile::ExternalV1` is a closed allow-list built only from
contracts actually delivered by the upstream. The 2026-08-19.1 bundle proves
60 RPC declarations, supplies `grpc-derived-products.md`, and freezes the
`SecurityMetadata` v1 plus `GlobalNews` v2 and `InstrumentNews` v2 request and
record contracts. Those three operations may use the direct external profile;
all other operations remain explicitly unsupported there and continue through
their previously admitted LocalBridgeV1 routes. Unknown or undelivered
operation/schema/version fails before I/O or interpretation; local Rust types
must not be used to guess an ExternalV1 payload contract.

GlobalNews v2 accepts one closed routing `provider` plus a positive bounded
`limit`. The adapter maps the provider to `QueryRequest.preferred_provider` and
emits the delivered business payload exactly as `{"limit":N}`; it must never
invent a `provider` JSON field. The routing value is one of the four closed
`GlobalNewsProvider::wire_name()` values.
InstrumentNews v2 accepts exactly one canonical instrument, `start`, `end`,
positive bounded `limit`, and caller-captured RFC3339 `captured_through`; when
the date range is present, `end` must equal that instant's Asia/Shanghai date.
`captured_through` is captured once by the caller and is used unchanged for the
request and consumer-side upper-bound check. It is never reconstructed from a
response, database time, or a second wall-clock read.

`allow_unadmitted` defaults to `false`. Only a separately named diagnostic
probe may set it to true; production gateway calls cannot.

### 5.4 Evidence model

The following fields remain upstream facts and are never generated locally:

- selected provider;
- upstream batch ID;
- source/observation times;
- admission, completeness and field status;
- canonical record schema/version.

The upstream v1 envelope lacks `source`. For the external profile only, the
client adds the authenticated acquisition authority
`grpc-mtls:<tls_server_name>` to the local `QueryResult.source`. This describes
the route that was actually authenticated; it does not impersonate a provider
or provider source time. The local profile continues to reject an empty source.

Remote records may repeat provider/batch/time fields. Any conflict with the
envelope fails the whole batch. Fractional Unix evidence timestamps use the
existing shared strict evidence parser; observation time is never substituted
for missing source time.

### 5.5 Narrow identity projection

The remote `SecurityMetadata` capability may be incomplete by design. The
existing full `MarketSecurityMetadata` type requires listing and price-limit
facts, so the remote response must not be forced into that type. A new gRPC
bridge method projects only the already-existing `MarketSecurityIdentity`
subset: code, source-backed name, ST flag and immutable evidence. Null listing
or limit fields remain absent and are not touched.

### 5.6 Board membership and local evidence fix

The synchronous `memberships_blocking` entry uses the same gRPC bridge and audit
contract as the async entry. The local server's membership delegate accepts one
canonical stock code per request and returns that one gateway batch with its
original evidence via `pack_ev`. Multi-code aggregation is rejected because it
would otherwise require inventing a synthetic cross-batch identity.

The response handler stops inventing `tdx-dev` and a new batch ID. Empty
provider/source/batch values are an internal error. This fixes the local
fallback while preserving the stricter external path.

### 5.6.1 P-01 LocalBridge LimitPools full-record contract

The synchronous BR-213 P-01 consumer remains the owner of the exact-date
upper-limit-pool interface. When `DATA_GATEWAY_GRPC=1`, it calls LocalBridgeV1
`LimitPools` through the bridge's runtime-safe blocking seam and audits that
routed result. A configured bridge error is returned unchanged; it never falls
back to a library provider, cache, empty batch, `UpperLimitPoolReview`, or A-10
chain projection.

The LocalBridgeV1 request has exactly three keys and is:

```json
{"kind":"Upper","trading_date":"YYYY-MM-DD","limit":200}
```

The date is caller-owned and the whole-pool bound remains 200. A `date` alias,
missing or extra key, non-`Upper` kind, malformed date, or any other limit is an
invalid request. The server calls a separate library-only `ReviewDataGateway`
seam so it cannot re-enter the consumer transport selector. That seam performs
the existing Eastmoney/Tonghuashun router acquisition and BR-159 audit. The
server must not derive this operation from `VisibleChainBatch`: its current
flattened records omit price, change, optional provider fields and record
evidence and therefore cannot authorize P-01.

Every response record is one canonical JSON object with this closed field set:

```text
kind, instrument, trading_date, price, change, volume, turnover,
sealed_amount, first_seal_at, last_seal_at, break_count, streak,
industry, board_name, seal_state, reseal_count, reason, evidence
```

`instrument`, `price`, `change`, and `evidence` retain their complete typed
Magic representation; every listed key is present, optional fields serialize as
explicit `null`, and values are never synthesized. The envelope carries the
selected real provider plus its source, source time, observation time and batch
ID. Each record evidence must bind exactly to that provider, batch ID, source
time and observation time.

Client admission requires `complete=true`, no more than 200 records, unique
canonical instrument codes, `kind=Upper`, every record trading date and the
envelope source date equal to the requested date, and exact record/envelope
evidence binding. A complete provider-backed zero-record result is
`VerifiedEmpty`. Missing/truncated/extra record fields, partial quality, count
overflow, duplicate instruments, wrong kind/date, or evidence conflict is
typed non-retryable `invalid_evidence`; a malformed request date is typed
non-retryable `invalid_request`; transport/provider failures retain their
source classification and retryability.

### 5.7 Two-phase opening readiness

Readiness is split at the real dependency seam. Static/auth/contract readiness
is a startup prerequisite. Live-session readiness is a background observation
and never delays a public/static producer window. The reports share evidence
validation but have different scheduling and failure effects.

#### 5.7.1 Capability discovery uses semantic OR families

Bundle health must be `live && ready`. Capability rows remain discovery
evidence only and require
`repository_admission=ADMITTED && runtime_available=true`. Aliases for the same
semantic dependency form an OR family, not multiple mandatory operations:

| Semantic dependency | Phase | Admitted runtime operation family |
| --- | --- | --- |
| announcements | static | `Announcements` OR `MarketAnnouncements` |
| board membership | static | `BoardConstituents` OR `BoardMemberships` |
| upper-limit review | static | `UpperLimitPoolReview` OR `LimitPools` |
| realtime quotes | live | `RealtimeQuotes` |
| order books | live | `OrderBooks` |
| T0 evidence | live | `T0Evidence` |
| security identity | static | `SecurityMetadata` |
| instrument news | static | `InstrumentNews` |
| global news | static | `GlobalNews` |

At least one row in every required semantic family must pass. Requiring both
aliases would reject a valid deployment; accepting a diagnostic-only or
unadmitted row would fabricate authority. Discovery never replaces the exact
route canary selected below.

#### 5.7.2 Static startup phase

Before any producer is started, the monitor must acquire its production process
lease, parse the bundle, verify TLS/auth and health, evaluate the static
semantic capability families, and execute these exact static routes. The three
live semantic families are evaluated by the background phase and cannot turn a
static startup check red:

| Stable route name | Profile | Empty policy |
| --- | --- | --- |
| `SecurityMetadata` | ExternalV1 | exact canary identity required |
| `InstrumentNews` | ExternalV1 | bounded verified-empty allowed |
| `GlobalNews-Eastmoney` | ExternalV1 | bounded verified-empty allowed |
| `GlobalNews-CLS` | ExternalV1 | bounded verified-empty allowed |
| `GlobalNews-Jin10` | ExternalV1 | bounded verified-empty allowed |
| `GlobalNews-ThePaper` | ExternalV1 | bounded verified-empty allowed |
| `Announcements` | LocalBridgeV1 | bounded verified-empty allowed |
| `BoardConstituents` | LocalBridgeV1 | exact canary membership required |
| `LimitPools` | LocalBridgeV1 | bounded verified-empty allowed; full `LimitPoolEntry` contract required |

Each GlobalNews canary sends the exact registered provider plus a positive
bounded limit. The server must consume both parameters. The returned
`BatchEvidence.provider` and `source` must equal that provider registration and
the record count must not exceed the requested limit. A batch from one provider
must not satisfy another provider's route; a mismatch is non-retryable invalid
evidence for that attempt and is retained in the audit disposition.

When `GRPC_MARKET_CLIENT_BUNDLE` is configured, all four GlobalNews routes use
the authenticated ExternalV1 v2 profile. TLS, authentication, capability,
schema, provider, record or evidence failure on that configured profile is
terminal for that provider attempt and never falls back to the local server,
library transport, cache or another provider. With no bundle configured, the
pre-existing LocalBridgeV1 GlobalNews route remains available under its own
explicit evidence and readiness contract; it cannot impersonate ExternalV1.

The four news canaries form one redundant source family, not four serial startup
locks. Startup attempts all four and requires at least two independently
identified, fully validated providers. A failed provider remains an explicit
typed route failure and is excluded; its records are never accepted, relabelled
or replaced. Fewer than two verified providers blocks startup. This preserves
cross-source corroboration while preventing one provider protocol change from
suppressing every public-news producer.

All successful routes preserve provider, acquisition source, optional source
time, observation time and immutable batch ID. The five non-news routes remain
mandatory, so static readiness requires at least seven of nine attempts: all
five mandatory routes plus at least two of four GlobalNews routes. Success emits
`opening_static_ready=true routes=<passed>/9 global_news=<passed>/4` and starts
producer schedulers, including the 09:00--09:15 P-01 path. A mandatory failure
or an insufficient news quorum emits `opening_static_ready=false
reason_code=... retryable=...`, starts zero producers and retries only when the
classified failure is retryable. A deterministic bundle, auth, contract,
identity or evidence mismatch in a mandatory route exits before producer
startup.

#### 5.7.3 Live-session background phase

After static readiness succeeds, a background task canaries exactly three live
routes through LocalBridgeV1: `RealtimeQuotes`, `OrderBooks` and `T0Evidence`.
Before the current trading session supplies live data it reports
`opening_data_ready=false reason_code=pending_live_window retryable=true`; this
does not block P-01 or another producer whose declared inputs are all static.
It does keep every live-dependent computation fail-closed.

`opening_data_ready=true routes=3 route_names=RealtimeQuotes,OrderBooks,T0Evidence`
may be emitted only when all three canaries pass in the current live acquisition
window. It is an operational observation, not transferable evidence. Every
live consumer must still validate its own exact batch at its own consumption
clock before computation and before marking DataMode capability success:

- parse both RFC3339 and fractional-Unix evidence through the shared strict
  parser without rewriting the original strings;
- reject any future source timestamp, including positive sub-millisecond skew;
- accept age exactly five seconds and reject any age greater than five seconds
  without integer-millisecond truncation;
- require `source_at <= observed_at <= consumer_now` for live evidence;
- require positive finite quote prices and exact requested instrument identity;
- require record/envelope provider, source, batch and time fields to agree;
- repeat the same current-time check after RPC/network and downstream assembly
  delay, including each quote inside `T0Evidence`.

GRPC-20260818-002 corrects the realtime provider failover seam. Every provider
attempt must return exactly the requested canonical instrument set before it
can be classified successful. A stale, missing, duplicate, out-of-request,
out-of-order, or record/envelope evidence-mismatched quote fails the entire
provider attempt; the remaining subset must not be represented as a complete
batch. Stale and missing responses are typed retryable so routing continues to
the next registered provider. Structural, order and evidence conflicts are
typed non-retryable `invalid_evidence` for that response. If no provider returns
one complete admitted set, the request fails explicitly with
`no_verified_batch`. The freshness interval remains exactly `0..=5s`, with
future evidence and age `>5s` rejected without integer-millisecond truncation,
cache substitution, subset completion, or partial-as-complete promotion.

The live loop recomputes failure/success and emits state transitions; a prior
success is never a sticky permit. Route identity mismatch is deterministic and
non-retryable for that response. Transport/staleness remains retryable. A live
failure does not terminate the public/static producers, but no affected
consumer, DataMode update, T0 decision or order-book calculation may proceed.

Off-session, lunch and after-hours static prices are never valid
`RealtimeFiveSecond` evidence and never mark `Capability::Quote`. A caller that
needs an official completed-session close must use the distinct typed
`SettledClose` path with its trading-date/session proof. The old off-session
Quote keepalive is removed rather than renamed.

#### 5.7.4 Runtime modes

| Mode | Static phase | Live phase | Delivery meaning |
| --- | --- | --- | --- |
| bare production monitor | blocking before producers | background/current live window | normal governed delivery |
| production `--push` | blocking before selected dispatcher | no waiting; live-dependent dispatch fails closed at its consumer | terminal governed delivery |
| `--review` | not an opening dependency | not run | use the strict post-session dependency gate; never print opening-ready |
| `--test` | TEST_CODE deterministic adapter only | TEST_CODE deterministic adapter only | physically isolated test data/sink |
| `--test --push-dry-run` | same TEST_CODE isolation | same TEST_CODE isolation | render/audit only, zero external delivery |
| `--test --review` | not run | not run | verify strict review failure in test isolation |

Test and review skips are explicit banners such as
`opening_readiness=not_applicable mode=test`; they must not be represented as
`opening_data_ready=true`. A naked production `--push-dry-run` remains invalid;
the supported dry-run form is the documented isolated `--test --push-dry-run`.

#### 5.7.5 Single-monitor lease and cutover

Every production invocation capable of scheduling or delivering a push must
hold one non-blocking, cross-process exclusive monitor lease before network
preflight or producer initialization and retain its file descriptor for process
lifetime. Production and TEST_CODE use physically separate lease paths. A live
foreign owner yields a typed `monitor_instance_already_running` exit before any
provider, durable-delivery or sink call; a PID string is diagnostic and is not
the lock authority. Read-only `grpc_bundle_probe`, help and history inspection
do not take the delivery lease.

Cutover remains stop-then-start: build and probe the candidate without starting
a second monitor, gracefully stop the old process, verify its PID and listening
ownership are gone, then start the candidate and prove it owns the lease. The
current release remains running until static/auth/contract probes and Gate C are
ready for that controlled handoff.

The read-only bundle probe has a different failure-reporting duty from the
production startup gate. Transport, health, capability discovery and canonical
instrument construction remain hard prerequisites. After those prerequisites
pass, the probe must execute every independent static canary it can reach,
retain each secret-safe typed success or failure in stable route order, print a
single deterministic summary, and exit nonzero if any mandatory canary failed.
One provider or route failure must not hide later independent diagnostics. This
diagnostic aggregation grants no startup permit: production
`external_static_opening_readiness` remains fail-closed and returns no ready
report unless its existing BR-238 quorum and mandatory-route rules pass.
The probe may additionally issue the four closed ExternalV1 GlobalNews requests
directly to expose their structured gRPC detail. Those requests validate the
delivered request schema `magic.market.global_news.request@2` and response-record
schema `magic.market.news_item@2`, then run the same production projection. A
single direct GlobalNews failure is diagnostic only; readiness still uses the
existing two-of-four family quorum rather than promoting all four providers to
mandatory routes.

### 5.8 Upstream revision and immutable historical artifacts

The server-side fourteen-crate Magic runtime is pinned to revision
`75ee2a2bdd3b1ca2b01ce3afbb04aec416e7000e`, including the TDX provider-time
decode required by the five-second quote gate. Every newly admitted runtime
batch and provider registration must report that revision; older literals must
not label data acquired by this binary.

The checked-in board-binding registry is different: it is immutable audit
evidence captured at revision `5f1ce93656a55854c844065390520cd4aecd9a14`.
The file stays byte-identical. Its revision mismatch against the current
runtime is an explicit selection-activation blocker until the registry is
reacquired and audited; changing only its revision field would fabricate
evidence. Opening news and market-data readiness do not treat that historical
artifact as current provider authority.

## 6. Failure modes

| Failure | Required behavior |
| --- | --- |
| bundle missing/escapes directory/bad protocol | fail before network; no secret text |
| TLS/auth/health failure | typed unavailable/unauthenticated; bounded retry only where contract permits |
| capability alias family has no admitted runtime member | static startup blocker or live background failure according to §5.7; never require every alias and never enable diagnostics silently |
| schema/version unknown or absent from delivered contract | block direct operation before I/O/interpretation; never infer it from local types |
| admission not ADMITTED | reject for production |
| response complete=false | accept only explicitly modeled field-level projections such as identity; otherwise reject |
| provider/source/batch missing or conflicting | invalid evidence; no compatibility fill |
| source time missing | keep absent; consumers requiring freshness reject |
| authenticated identity/news clock leads the consumer | preserve the original timestamp and admit at most 2 seconds of positive clock skew only for the explicitly bounded identity/news contract; reject larger future evidence and retain its 30-second observation / one-trading-day source budgets |
| quote/order-book/T0 source time leads the consumer | reject any positive future duration; no clock-skew exception and no millisecond truncation |
| one gRPC quote provider returns stale/missing/duplicate/out-of-order/mismatched records | fail that whole provider attempt; stale/missing remain retryable for failover, structural/evidence conflicts are non-retryable; never promote a subset to complete |
| every gRPC quote provider lacks one complete exact requested set | explicit `no_verified_batch`; zero realtime computation or Quote/DataMode success mark |
| live route is stale, absent or fails after static startup | keep `opening_data_ready=false`, retain retry eligibility, and fail only its live-dependent consumers; do not suppress P-01 |
| off-session source returns same-day last trade | reject from Realtime/DataMode; only a separately requested typed SettledClose may consume it |
| one GlobalNews response identifies a different provider/source or exceeds limit | invalid evidence, non-retryable for that attempt; exclude it and do not classify it under the requested feed |
| fewer than two of four GlobalNews providers return verified evidence | static startup blocker with all four typed outcomes retained; never fabricate, relabel or consume a failed provider |
| one board membership request contains multiple stocks | invalid request; caller performs independent per-stock calls |
| remote outage after startup | current attempt fails and timer retains retry eligibility under BR-116 |
| LocalBridge `LimitPools` field/evidence/count/date conflict | non-retryable invalid evidence; do not fall back to library, `UpperLimitPoolReview` or A-10 chain projection |
| local fallback evidence missing | internal error; do not weaken client validation |
| real account evidence stale | account-dependent advice remains conservative; do not fabricate from public data |

## 7. Old module disposition

| Module | Adopt/reject | Reason |
| --- | --- | --- |
| `src/grpc_client/client.rs` | adopt and deepen | single owner for channel, TLS, auth and contract profile |
| `src/grpc_client/envelope.rs` | adopt and split profile-specific request building | retains request ID and response validation |
| `src/data_gateway/grpc_source.rs` | adopt | single gateway normalization boundary and existing retry bridge |
| `src/data_gateway/grpc_source/convert.rs` | adopt | normalize external records and enforce evidence conflicts |
| current all-routes `external_opening_readiness` | replace/deepen | split into typed static-startup and live-session reports; remove transferable all-purpose Boolean authority |
| `src/data_gateway/global_news.rs` and server delegate | adopt/fix | carry exact provider/limit and validate returned provider/source before feed classification |
| `ReviewDataGateway::current_upper_limit_pool` | adopt/deepen | keep one synchronous P-01 interface; select the LocalBridge transport internally and expose a separate server-only library seam to prevent recursion |
| current flattened `LimitPools` chain view | reject/replace | omits full provider `LimitPoolEntry` facts and record evidence, so it cannot authorize P-01 |
| realtime `admit_quote_batch` | adopt/fix | exact-duration consumer gate remains the sole realtime admission path |
| `off_session_static_quote_eligible` and `off_session_quote_keepalive_loop` | reject/remove | they convert a static price into Realtime/Quote authority and violate §2.4 |
| `src/grpc_server/delegate.rs` | adopt/fix | local fallback must preserve original gateway evidence |
| `src/grpc_server/handlers.rs` | adopt/fix | remove invented compatibility evidence |
| `src/data_gateway/board_runtime.rs` | adopt/fix | bridge the blocking membership consumer |
| `src/data_gateway/market_capabilities.rs` | adopt/fix | bridge the narrow security identity capability |
| monitor push renderers/governance/sinks | unchanged | data correction belongs below business rendering |
| direct library provider fallback in no-feature monitor | reject | not compiled and would bypass the selected transport |

## 8. Validation and release evidence

Targeted red/green tests cover bundle path containment, TLS config construction,
auth secrecy, external request schemas, semantic capability OR families,
unadmitted rejection, remote record shape, identity partial projection, board
single-code evidence, all four provider-bound GlobalNews attempts, the two-of-four
news quorum, provider and limit round trips, T0 readiness, static-before-producer ordering,
live-background/P-01 independence, exact five-second/sub-millisecond consumer
checks, off-session realtime rejection, mode-specific skips and cross-process
monitor lease exclusion.

Mandatory commands:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

Live read-only evidence must additionally show the static bundle preflight
passes, at least two of four news providers retain distinct verified evidence
while every failed provider is explicit, the three
live canaries pass in the current live window, exactly one monitor owns the
lease, source/batch-empty errors are absent, and the push/audit chain records
real delivery outcomes. A weekend, lunch or after-hours static quote is not
opening evidence; the exact five-second quote/order-book/T0 consumer checks must
be repeated in the 2026-08-18 live window.

## 9. Deployment and rollback

1. Preserve the current release binaries and process metadata.
2. Build and run the read-only bundle/static probes without starting a second
   monitor; the probe does not acquire the delivery lease.
3. Stop the old monitor gracefully only after Gate C and static probes pass;
   verify its process/listener ownership is gone.
4. Start one monitor with the explicit bundle path, verify exclusive lease
   ownership and `opening_static_ready=true`, and let the live phase canary in
   the current session without delaying P-01.
5. If the external profile fails, stop the new process and restore the previous
   release plus local gRPC service. Do not delete audit, push or market evidence.
6. Code rollback is `git revert <scoped-commit>`; configuration rollback removes
   only `GRPC_MARKET_CLIENT_BUNDLE` and restores the prior explicit local address.
