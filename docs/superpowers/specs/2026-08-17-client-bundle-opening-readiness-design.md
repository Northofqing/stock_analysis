# Client-Bundle Opening Readiness Design

Date: 2026-08-17
Target session: 2026-08-18 A-share pre-open/open
Status: Gate A design selected under the user's standing project authorization
Rules: AGENTS 2.1, 2.2, 2.3, 2.4, 2.7, 2.8, 2.10; BR-091, BR-103,
BR-112, BR-113, BR-114, BR-116, BR-159, BR-164, BR-168, BR-188, BR-213,
BR-216, BR-217, BR-218, BR-220, BR-221, BR-223, BR-225, BR-226, BR-227,
BR-231.

## 1. Outcome

The production monitor must be able to acquire the real public-market inputs
needed by the 09:00-09:30 push chain through the authenticated `client-bundle`
contract, while retaining the current local gRPC service as a reversible
fallback until the remote canaries pass.

Success means:

1. the bundle is loaded without logging or committing its private key or token;
2. startup verifies remote TLS identity, health, and the required admitted
   runtime capabilities before the data path is declared ready;
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
Because the delivered ExternalV1 contract is incomplete, the configured bundle
is an additional authenticated client, not a wholesale replacement for that
bridge. Dispatch is closed and operation-specific: only fixture-proven external
operations may use the bundle; every other operation remains on the local
normalized service. A configured external operation never silently falls back
after TLS/auth/schema/evidence failure. If the bundle is absent, all existing
local operations keep their current path and the externally owned identity/news
readiness checks report unavailable explicitly.

### 5.2 Transport and auth

The tonic channel uses a CA certificate plus client identity and pins the TLS
domain from the bundle. The Bearer token is held in a zeroizing value and added
to request metadata by the client instance; it is never copied into an
environment variable, URL, request payload, log or debug output.

### 5.3 Contract profiles

`ContractProfile::LocalBridgeV1` retains local short schemas and JSON-array
records. `ContractProfile::ExternalV1` is a closed allow-list built only from
contracts actually delivered by the upstream. The 2026-08-17 bundle proves the
`SecurityMetadata` and `InstrumentNews` request contracts; it does not deliver
request/record schema labels for the other opening operations, its Proto ends at
operation 55 while the companion document claims 60, and the referenced
`grpc-derived-products.md` is missing. Those operations therefore remain
explicitly unsupported in the direct external profile. Unknown or undelivered
operation/schema/version fails before I/O or interpretation; local Rust types
must not be used to guess an ExternalV1 payload contract.

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

### 5.7 Startup readiness

Before the monitor declares the bundle path ready, a read-only preflight checks:

1. bundle parse and TLS/auth connection;
2. health `live && ready`;
3. every opening-required capability has at least one
   `repository_admission=ADMITTED && runtime_available=true` provider;
4. real canaries are required only where the bundle supplies a frozen request
   and response contract. Capability-only evidence for quote,
   news/announcement or board membership is reported separately and cannot
   authorize direct parsing until the upstream supplies the missing fixtures and
   schema labels.

A failed preflight is visible and fail-closed. The current release remains
running until the new binary and data path have passed these checks.

## 6. Failure modes

| Failure | Required behavior |
| --- | --- |
| bundle missing/escapes directory/bad protocol | fail before network; no secret text |
| TLS/auth/health failure | typed unavailable/unauthenticated; bounded retry only where contract permits |
| capability unavailable or unadmitted | startup blocker for its dependent push; never enable diagnostics silently |
| schema/version unknown or absent from delivered contract | block direct operation before I/O/interpretation; never infer it from local types |
| admission not ADMITTED | reject for production |
| response complete=false | accept only explicitly modeled field-level projections such as identity; otherwise reject |
| provider/source/batch missing or conflicting | invalid evidence; no compatibility fill |
| source time missing | keep absent; consumers requiring freshness reject |
| authenticated remote clock leads the consumer | preserve the original timestamp and admit at most 2 seconds of positive clock skew; reject larger future evidence and retain the ordinary 30-second observation / one-trading-day source freshness budgets |
| one board membership request contains multiple stocks | invalid request; caller performs independent per-stock calls |
| remote outage after startup | current attempt fails and timer retains retry eligibility under BR-116 |
| local fallback evidence missing | internal error; do not weaken client validation |
| real account evidence stale | account-dependent advice remains conservative; do not fabricate from public data |

## 7. Old module disposition

| Module | Adopt/reject | Reason |
| --- | --- | --- |
| `src/grpc_client/client.rs` | adopt and deepen | single owner for channel, TLS, auth and contract profile |
| `src/grpc_client/envelope.rs` | adopt and split profile-specific request building | retains request ID and response validation |
| `src/data_gateway/grpc_source.rs` | adopt | single gateway normalization boundary and existing retry bridge |
| `src/data_gateway/grpc_source/convert.rs` | adopt | normalize external records and enforce evidence conflicts |
| `src/grpc_server/delegate.rs` | adopt/fix | local fallback must preserve original gateway evidence |
| `src/grpc_server/handlers.rs` | adopt/fix | remove invented compatibility evidence |
| `src/data_gateway/board_runtime.rs` | adopt/fix | bridge the blocking membership consumer |
| `src/data_gateway/market_capabilities.rs` | adopt/fix | bridge the narrow security identity capability |
| monitor push renderers/governance/sinks | unchanged | data correction belongs below business rendering |
| direct library provider fallback in no-feature monitor | reject | not compiled and would bypass the selected transport |

## 8. Validation and release evidence

Targeted red/green tests cover bundle path containment, TLS config construction,
auth secrecy, external request schemas, unadmitted rejection, remote record
shape, identity partial projection, board single-code evidence and blocking
bridge use.

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

Live read-only evidence must additionally show the bundle preflight passes,
opening canaries have admissible records, one monitor process is running, the
source/batch-empty errors are absent, and the push/audit chain records real
delivery outcomes. A weekend/off-session stale quote is not opening evidence;
the 5-second quote check must be repeated in the 2026-08-18 live window.

## 9. Deployment and rollback

1. Preserve the current release binaries and process metadata.
2. Build and preflight the new binaries without starting a second monitor.
3. Stop the old monitor gracefully only after Gate C and bundle canaries pass.
4. Start one monitor with the explicit bundle path and verify startup/read-only
   acquisition before the push windows.
5. If the external profile fails, stop the new process and restore the previous
   release plus local gRPC service. Do not delete audit, push or market evidence.
6. Code rollback is `git revert <scoped-commit>`; configuration rollback removes
   only `GRPC_MARKET_CLIENT_BUNDLE` and restores the prior explicit local address.
