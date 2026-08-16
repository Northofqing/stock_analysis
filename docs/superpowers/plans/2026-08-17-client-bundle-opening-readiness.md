# Client-Bundle Opening Readiness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the 2026-08-18 pre-open/open monitor consume authenticated real `client-bundle` data with complete evidence and verified push readiness.

**Architecture:** Keep gateway and push consumers stable. Add an external-v1 transport/contract profile inside `grpc_client`, normalize canonical records once in `grpc_source`, and retain the local plaintext service as a reversible fallback until live canaries pass. Missing, stale, partial, conflicting, unsupported and unadmitted facts remain fail-closed.

**Tech Stack:** Rust, Tokio, tonic 0.14 TLS, Prost, Serde JSON, zeroize, existing SQLite/Diesel audit and monitor delivery stack.

---

## File map

- Create `src/grpc_client/bundle.rs`: validate bundle metadata and secret-file boundaries.
- Create `src/grpc_client/external_v1.rs`: external schema/parameter mapping.
- Create `src/bin/grpc_bundle_probe.rs`: secret-safe live health/capability/schema canary.
- Modify `Cargo.toml`, `src/grpc_client/{mod,auth,client,envelope}.rs`: TLS, per-client auth and contract profiles.
- Modify `src/data_gateway/grpc_source{.rs,/convert.rs}`: profile selection and canonical normalization.
- Modify `src/data_gateway/{market_capabilities,board_runtime}.rs`: opening-critical no-feature bridges.
- Modify `src/grpc_server/{delegate,handlers}.rs`: preserve local fallback evidence.
- Modify `docs/business_rules.md`: BR-231 citations/status and pre-existing duplicate-ID repair.

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

### Task 6: Add a secret-safe live readiness probe

**Files:** `src/bin/grpc_bundle_probe.rs`

- [ ] Test capability evaluation: an operation is capability-ready only if at
  least one row is admitted and runtime available; diagnostic MoneyFlows cannot
  satisfy production readiness. Keep capability-ready separate from
  contract-ready so missing schemas cannot be treated as usable data.
- [ ] Implement `--bundle <dir> --opening`. Output only health, capability readiness, admission/completeness/count, canonical schema name/version and sorted JSON field names—never values or auth metadata.
- [ ] Exit nonzero for bad health, missing capability, missing delivered
  contract/fixture, unknown schema or bad evidence.
- [ ] Verify unit/help tests, then run:

```bash
cargo run --release --bin grpc_bundle_probe -- \
  --bundle /Users/zhangzhen/Desktop/Quant/stock_analysis/client-bundle --opening
```

Off-session quote staleness is an explicit blocker and must be rechecked in the live opening window.

### Task 7: Integrate monitor readiness without changing push semantics

**Files:** `src/bin/monitor/main.rs`, existing integration/process tests

- [ ] Add a failing startup test: failed external readiness prints one `opening_data_ready=false reason_code=...` banner and does not start opening producers.
- [ ] Run health/capability readiness before producer warmup. Post-start failures retain BR-116 retry eligibility. Renderer/governance/sink modules never receive credentials.
- [ ] Verify call paths with multiline-aware search:

```bash
rg -nA4 'GRPC_MARKET_CLIENT_BUNDLE|opening_data_ready|security_identities_async|board_constituents\(' \
  src/bin/monitor src/data_gateway src/grpc_client
```

- [ ] Run `grpc_bridge_e2e`, `monitor_help_isolation` and a release no-default-features monitor build.

### Task 8: Gate C/D and single-instance cutover

**Files:** changed sources/docs; private `.env` at cutover only

- [ ] Add BR-231 citations to changed active paths. Reassign the pre-existing duplicate rows without changing semantics: SignalTracker BR-224 → BR-232; review BR-225 → BR-233; update their exact code citations.
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

- [ ] Preserve current binary hashes/PIDs, set only the private bundle path, stop the old monitor gracefully and start exactly one new monitor after canaries pass.
- [ ] Verify opening producers, governed delivery and JSONL audit outcomes; confirm no credentials, mock data, empty evidence, duplicate monitor or selection-v2 recovery flood.
- [ ] On any blocker, stop the new monitor and restore the preserved release plus explicit local address. Never delete audit/data evidence. Code rollback is `git revert <scoped-commit>`.

## Self-review

- Every selected design requirement maps to Tasks 1-8.
- No placeholder or unspecified error-handling step remains.
- Names are consistent: `ClientBundleConfig`, `ContractProfile`, `build_external_query_request`, `security_identities`, `GRPC_MARKET_CLIENT_BUNDLE`.
- Public-market readiness remains separate from unavailable BR-103 broker account/trade watermark evidence.
