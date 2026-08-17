# Client-Bundle Opening Readiness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the 2026-08-18 pre-open/open monitor consume authenticated real `client-bundle` data with complete evidence and verified push readiness.

**Architecture:** Keep gateway and push renderers stable. Add an external-v1 transport/contract profile inside `grpc_client`, normalize canonical records once in `grpc_source`, and retain the local plaintext service for contracts the bundle does not freeze. A blocking static/auth/contract phase starts public/static producers; a separate background live phase observes RealtimeQuotes, OrderBooks and T0Evidence without delaying P-01. Every live consumer still revalidates exact five-second evidence at its own clock. Missing, stale, partial, conflicting, unsupported and unadmitted facts remain fail-closed.

**Tech Stack:** Rust, Tokio, tonic 0.14 TLS, Prost, Serde JSON, zeroize, existing SQLite/Diesel audit and monitor delivery stack.

---

## File map

- Create `src/grpc_client/bundle.rs`: validate bundle metadata and secret-file boundaries.
- Create `src/grpc_client/external_v1.rs`: external schema/parameter mapping.
- Create `src/bin/grpc_bundle_probe.rs`: secret-safe live health/capability/schema canary.
- Modify `Cargo.toml`, `src/grpc_client/{mod,auth,client,envelope}.rs`: TLS, per-client auth and contract profiles.
- Modify `src/data_gateway/grpc_source{.rs,/convert.rs}`: profile selection and canonical normalization.
- Modify `src/data_gateway/{global_news,market_data}.rs`: provider-bound news requests and strict live quote admission.
- Modify `src/data_gateway/{market_capabilities,board_runtime}.rs`: opening-critical no-feature bridges.
- Modify `src/grpc_server/{delegate,handlers}.rs`: preserve local fallback evidence.
- Create `src/bin/monitor/process_lease.rs` (TO BE BUILT at Gate B): process-lifetime cross-process production monitor exclusion.
- Modify `src/bin/monitor/{main,market_data}.rs`: two-phase readiness, mode banners and consumer/DataMode gates.
- Modify `docs/business_rules.md`: BR-238 citations/status and pre-existing duplicate-ID repair.

### Task 1: Freeze and repair local evidence propagation

**Files:** `src/grpc_server/delegate.rs`, `src/grpc_server/handlers.rs`, `tests/grpc_bridge_e2e.rs`

- [ ] Add a failing unit test that creates a `GatewayBatch<BoardMembershipRecord>` with:

```rust
let evidence = BatchEvidence {
    provider: ProviderId::Tdx,
    source: "TEST_CODE_tdx-board-memberships".into(),
    source_at: Some("2026-08-17T09:00:00+08:00".into()),
    observed_at: "2026-08-17T09:00:01+08:00".into(),
    batch_id: "TEST_CODE_membership_batch".into(),
};
```

Assert a new `pack_board_membership_batch` returns the same provider/source/batch.

- [ ] Verify RED:

```bash
cargo test --lib grpc_server::delegate::tests::board_membership_pack_preserves_original_evidence -- --exact
```

- [ ] Implement exact-one-code membership acquisition. Reject `codes.len() != 1`, call `memberships` once, and pack with `pack_ev`; never aggregate independent batches or invent a batch ID.
- [ ] Make the migrated handler copy `result.provider/source/batch_id` and reject any missing field. Do not use `tdx-dev` or request-time batch IDs for this path.
- [ ] Verify GREEN:

```bash
cargo test --lib grpc_server::delegate::tests::board_membership_pack_preserves_original_evidence -- --exact
cargo test --test grpc_bridge_e2e -- --test-threads=1
```

### Task 2: Parse client-bundle without leaking secrets

**Files:** `src/grpc_client/bundle.rs`, `src/grpc_client/mod.rs`

- [ ] Add failing tests for protocol 2, `../` path escape, non-regular/empty files and error text containing no token/key bytes.
- [ ] Verify RED: `cargo test --lib grpc_client::bundle::tests -- --test-threads=1`.
- [ ] Implement:

```rust
pub struct ClientBundleConfig {
    pub endpoint_uri: String,
    pub tls_server_name: String,
    pub ca_pem: Vec<u8>,
    pub certificate_pem: Vec<u8>,
    pub private_key_pem: Zeroizing<Vec<u8>>,
    pub bearer_token: Zeroizing<String>,
}

pub fn load(path: &Path) -> Result<ClientBundleConfig, BundleError>;
```

Canonicalize the root and every declared file; require protocol v1, files under the root, an HTTPS endpoint and non-empty TLS/token values. Errors name only field roles.
- [ ] Verify GREEN and scan: `cargo test --lib grpc_client::bundle::tests -- --test-threads=1` plus `rg -n 'Bearer |client-key' src/grpc_client` (no values).

### Task 3: Add the authenticated external-v1 client profile

**Files:** `Cargo.toml`, `src/grpc_client/{auth,client,envelope,external_v1}.rs`

- [ ] Add failing tests:

```rust
let request = build_external_query_request(
    Operation::SecurityMetadata,
    json!({"codes":["600396"]}),
).unwrap();
assert_eq!(request.payload.unwrap().schema, "magic.market.security_metadata.request");
assert!(!request.allow_unadmitted);
```

Repeat for `InstrumentNews`. Add negative tests proving `RealtimeQuotes`, board
membership and `UpperLimitPoolReview` fail as an undelivered ExternalV1 schema
before I/O; do not infer their wire contracts from local types.
- [ ] Verify RED: `cargo test --lib grpc_client::external_v1::tests -- --test-threads=1`.
- [ ] Enable tonic ring TLS and implement `GrpcMarketClient::connect_client_bundle(&Path)` with bundle CA, client identity and TLS domain.
- [ ] Store `ContractProfile::{LocalBridgeV1,ExternalV1}`, instance-owned zeroizing Bearer auth and `grpc-mtls:<tls_server_name>` acquisition authority. No secret-containing type derives `Debug`.
- [ ] Add a closed external schema/parameter mapping containing only delivered
  and fixture-proven contracts (`SecurityMetadata`, `InstrumentNews`). Unknown
  or contract-incomplete operations fail before I/O; production always sets
  `allow_unadmitted=false`.
- [ ] For external responses only, fill absent local field 11 with the authenticated acquisition authority. Never alter provider, batch ID or timestamps. Local empty source remains empty and is rejected.
- [ ] Verify: `cargo test --lib grpc_client:: -- --test-threads=1` and `cargo check --no-default-features --bin monitor`.

### Task 4: Normalize external canonical records

**Files:** `src/data_gateway/grpc_source/convert.rs`

- [ ] Add failing tests for local one-payload JSON arrays versus external one-object-per-payload records, unknown schema/version, and record/envelope provider or batch conflicts.
- [ ] Add a sanitized external SecurityMetadata test with nullable listing/limit fields and `complete=false`; require a successful narrow identity projection and a failed full metadata projection.
- [ ] Update `parse_records` to validate all payload schemas/versions and flatten exactly one supported shape. Reject mixed shapes.
- [ ] Replace RFC3339-only parsing with the shared strict evidence timestamp parser; preserve raw evidence and never replace source time with observed time.
- [ ] Implement:

```rust
pub fn security_identities(
    q: &QueryResult,
) -> Result<GatewayBatch<MarketSecurityIdentity>, GatewayError>;
```

Read only `instrument.code`, non-empty `name`, `is_st` and immutable evidence; reject wrong requested sets and all evidence conflicts.
- [ ] Verify: `cargo test --lib data_gateway::grpc_source::convert::tests -- --test-threads=1`.

### Task 5: Wire no-feature security identity and blocking membership

**Files:** `src/data_gateway/grpc_source.rs`, `src/data_gateway/market_capabilities.rs`, `src/data_gateway/board_runtime.rs`

- [ ] Extend the source inventory tests to require `SecurityIdentity` and the blocking membership bridge. Verify the tests fail first.
- [ ] Cache the local normalized connection selected by `GRPC_MARKET_ADDR` and,
  when configured, a separate authenticated client selected by
  `GRPC_MARKET_CLIENT_BUNDLE`. Dispatch only fixture-proven external operations
  to the latter; never replace all local operations with an incomplete external
  contract. Startup logs only profile/authority, never secret paths/values.
- [ ] Add `security_identities_async(codes)` and route `MarketCapabilitiesGateway::security_identities` through it before feature-gated library code.
- [ ] Add a synchronous wrapper around `board_constituents_async` using the existing safe bridge runtime; make `memberships_blocking` use the same audited branch as async membership.
- [ ] Verify:

```bash
cargo test --no-default-features --lib data_gateway::grpc_source -- --test-threads=1
cargo test --no-default-features --lib data_gateway::board_runtime -- --test-threads=1
cargo test --no-default-features --lib data_gateway::market_capabilities -- --test-threads=1
```

### Task 6: Close the four-provider GlobalNews bridge contract and quorum

**Files:** `src/data_gateway/global_news.rs`, `src/data_gateway/grpc_source.rs`, `src/grpc_server/delegate.rs`, `src/news/aggregator/raw_v2.rs`

- [ ] Add a failing test in which the LocalBridgeV1 server always returns an
  Eastmoney batch while the caller independently requests Eastmoney, CLS, Jin10
  and ThePaper. Only Eastmoney may be Available; the other three must be typed
  non-retryable `invalid_evidence`, not duplicate AI input.
- [ ] Add failing round-trip tests proving the caller's stable provider wire
  name and positive bounded `limit` reach the server, and that a response count
  greater than the limit is rejected.
- [ ] Give `GlobalNewsProvider` one public closed wire-name parser/formatter.
  Pass `{provider,limit}` through `GlobalNewsGateway` → `GrpcSource` → delegate;
  reject missing/unknown providers instead of defaulting to Eastmoney/20.
- [ ] Validate every returned batch against the requested registration's exact
  provider ID and source string before classifying Available or VerifiedEmpty.
  Preserve mismatching evidence in the typed audit disposition.
- [ ] Attempt all four providers but require at least two distinct verified
  batches for static readiness. Preserve every failed provider as an explicit
  typed outcome; never relabel, fill or consume it.
- [ ] Verify:

```bash
cargo test --lib data_gateway::global_news::tests -- --test-threads=1
cargo test --lib news::aggregator::raw_v2::tests -- --test-threads=1
cargo test --lib grpc_server::delegate::tests -- --test-threads=1
```

### Task 7: Add a secret-safe static readiness probe

**Files:** `src/bin/grpc_bundle_probe.rs`

- [ ] Test capability evaluation: a semantic capability family is ready when at
  least one alias row is admitted and runtime available. Explicitly prove
  `Announcements|MarketAnnouncements`,
  `BoardConstituents|BoardMemberships` and
  `UpperLimitPoolReview|LimitPools` are OR families. Diagnostic MoneyFlows
  cannot satisfy production readiness. Keep capability-ready separate from
  contract-ready so missing schemas cannot be treated as usable data.
- [ ] Implement `--bundle <dir> --opening`. Output only health, capability readiness, admission/completeness/count, canonical schema name/version and sorted JSON field names—never values or auth metadata.
- [ ] Reuse the production static-readiness module to exercise all nine static
  attempts, including four independently provider-bound GlobalNews routes and
  their two-of-four quorum. The
  probe may report stable route names and evidence presence/counts but must not
  print record values, URLs, titles, token material or private paths.
- [ ] Exit nonzero for bad health, missing capability, missing delivered
  contract/fixture, unknown schema or bad evidence.
- [ ] Verify unit/help tests, then run:

```bash
cargo run --release --bin grpc_bundle_probe -- \
  --bundle /Users/zhangzhen/Desktop/Quant/stock_analysis/client-bundle --opening
```

This read-only probe proves static auth/contract inputs. It must not turn an
off-session quote into live evidence or acquire the production monitor lease.

### Task 8: Integrate two-phase readiness and consumer-side live gates

**Files:** `src/data_gateway/grpc_source.rs`, `src/data_gateway/grpc_source/convert.rs`, `src/data_gateway/market_data.rs`, `src/bin/monitor/main.rs`, `src/bin/monitor/market_data.rs`, `src/bin/monitor/process_lease.rs`, existing integration/process tests

- [ ] Add failing ordering tests proving static failure starts zero producers,
  static success starts the 09:00--09:15 P-01 scheduler, and a missing/stale
  RealtimeQuotes, OrderBooks or T0Evidence canary leaves
  `opening_data_ready=false` without delaying P-01.
- [ ] Implement the nine stable static attempts from design §5.7.2, including
  separate `GlobalNews-Eastmoney`, `GlobalNews-CLS`, `GlobalNews-Jin10` and
  `GlobalNews-ThePaper` results. Require all five non-news routes plus at least
  two verified news providers. Emit `opening_static_ready` independently from
  `opening_data_ready` and list degraded providers without their record values.
- [ ] Run the three live routes (`RealtimeQuotes`, `OrderBooks`, `T0Evidence`)
  in a BR-116 background loop after static success. Outside a current live
  acquisition window report `pending_live_window`; never accept an off-session
  stale record to make the state green.
- [ ] Add injected-clock RED tests for exactly 5s accepted, 5s+1ns rejected,
  future+1ns rejected, network/consumer delay rejected, positive finite price,
  exact identity and record/envelope evidence conflicts. Apply the same
  consumer-time validation to realtime quote, order book and every T0 quote.
- [ ] Delete the BR-236 `RealtimeFiveSecond` off-session exception and the
  off-session Quote keepalive caller. Preserve official completed-session data
  only through the distinct typed `SettledClose` mode. Mark DataMode Quote
  success only after consumer-side live validation.
- [ ] Make identity/news positive clock skew remain the bounded contract in the
  design, but give live quote/order-book/T0 no positive future-time exception.
- [ ] Add mode tests: `--review` and `--test --review` do not run opening gates;
  `--test` and `--test --push-dry-run` use only TEST_CODE isolation and print
  not-applicable rather than opening-ready; production `--push` passes static
  readiness but does not wait indefinitely for live data.
- [ ] Acquire a mode-separated non-blocking cross-process monitor lease before
  any production provider/producer/sink call and hold it for process lifetime.
  A second delivery-capable production process must exit with
  `monitor_instance_already_running` and zero external calls. Probe/help/history
  remain read-only and lease-free.
- [ ] Renderer/governance/sink modules never receive credentials or a reusable
  readiness Boolean. Their live facts arrive only after the typed consumer gate.
- [ ] Verify call paths with multiline-aware search:

```bash
rg -nA4 'GRPC_MARKET_CLIENT_BUNDLE|opening_static_ready|opening_data_ready|T0Evidence|security_identities_async|board_constituents\(' \
  src/bin/monitor src/data_gateway src/grpc_client
```

- [ ] Run `grpc_bridge_e2e`, `monitor_help_isolation`, the cross-process lease
  test and a release no-default-features monitor build.

### Task 9: Gate C/D and single-instance cutover

**Files:** changed sources/docs; private `.env` at cutover only

- [ ] Add BR-238 citations to changed active paths. Reassign the pre-existing duplicate rows without changing semantics: SignalTracker BR-224 → BR-232; review BR-225 → BR-233; update their exact code citations.
- [ ] Run Gate C:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
```

- [ ] Run Gate D:

```bash
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

- [ ] Preserve current binary hashes/PIDs/listeners, set only the private bundle
  path, run the lease-free static probe, stop the old monitor gracefully, verify
  its PID/listener has gone, then start exactly one candidate monitor and prove
  exclusive lease ownership plus `opening_static_ready=true`.
- [ ] In the current live window verify all three live routes and exact
  consumer-side freshness before accepting `opening_data_ready=true`. Verify
  P-01 remains scheduled while live readiness is pending, at least two news
  provider batches retain their requested evidence, every degraded provider is
  explicit, and governed delivery/JSONL audit
  outcomes contain no credentials, mock data, empty evidence, duplicate monitor
  or selection-v2 recovery flood.
- [ ] On any blocker, stop the new monitor and restore the preserved release plus explicit local address. Never delete audit/data evidence. Code rollback is `git revert <scoped-commit>`.

## Self-review

- Every selected design requirement maps to Tasks 1-9.
- No placeholder or unspecified error-handling step remains.
- Names are consistent: `ClientBundleConfig`, `ContractProfile`, `build_external_query_request`, `security_identities`, `GRPC_MARKET_CLIENT_BUNDLE`.
- Public-market readiness remains separate from unavailable BR-103 broker account/trade watermark evidence.
- Gate C/D remain pending until Task 9 independently verifies their complete command and live-evidence sets.
