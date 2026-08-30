# Magic Dependency Removal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove every `magic-market-data-rs` Cargo dependency and direct Rust crate reference, delete the in-repository provider host, and make gRPC the only production market-data transport.

**Architecture:** Start from the already-compiling `--no-default-features` path, promote its local domain types to the sole implementation, and collapse every dual local/gRPC gateway onto the gRPC branch. Delete provider-only server, probes, tests, feature gates, build-time revision attestation, and lockfile packages; retain provider-neutral admission and stable wire/provider identities.

**Tech Stack:** Rust 2021, Cargo, Tokio, tonic/prost, repository shell regression gates.

**Spec:** `docs/superpowers/specs/2026-08-30-magic-dependency-removal-design.md`

## Global Constraints

- `Cargo.toml`, `Cargo.lock`, and `cargo tree --locked` must contain no `magic-*` crate or `magic-market-data-rs.git` source.
- Rust source must contain no direct `magic_*_rs`, `magic_market_core`, `magic_market_router`, or `magic_market_composition` path.
- Production market-data calls must use gRPC and fail closed; no local provider fallback, empty success, default value, fake data, or silent opt-out is allowed.
- Stable database and wire identities such as `"magic-tdx"` remain unchanged.
- Existing evidence, freshness, canonical identity, selection, decision, trading, and audit semantics remain unchanged.
- Preserve the pre-existing `src/data_gateway/grpc_source.rs` `no_current_reports` mapping/test, `config/selection/selection_activation.v1.json`, and root-level untracked files.
- Preserve concurrent `src/data_gateway/consensus.rs` and `src/grpc_client/errors.rs` fixes. The concurrent `Cargo.toml`/`Cargo.lock` revision bump to `48ae41bf4eb9682466d4ae2c776edabf997a5888` is superseded only because Task 1 removes those dependencies entirely.
- Stage only paths explicitly changed by this migration; never reset, checkout, or bulk-format over user work.

---

### Task 1: Remove the Cargo and build-time dependency graph

**Files:**

- Create: `scripts/check-no-magic-dependencies.sh`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `build.rs`
- Modify: `src/data_gateway/benchmark.rs`
- Modify: `tests/test_coverage_thresholds.rs`
- Delete: `build_support/magic_tdx_lock.rs`
- Delete: `tests/magic_market_release_revision.rs`
- Delete: `tests/magic_tdx_lock_contract.rs`

**Interfaces:**

- Consumes: the current default dependency graph and build script.
- Produces: a manifest/build layer that no longer knows about the upstream repository, plus a reusable static acceptance gate.

- [ ] **Step 1: Add the failing repository gate**

Create `scripts/check-no-magic-dependencies.sh` with these exact checks:

```bash
#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

scope="${1:-all}"
failed=0

reject() {
  local label="$1"
  local pattern="$2"
  shift 2
  if rg -n "$pattern" "$@"; then
    printf 'forbidden %s remains\n' "$label" >&2
    failed=1
  fi
}

if [[ "$scope" == "manifest" || "$scope" == "all" ]]; then
  reject "Cargo dependency" '(^|[^[:alnum:]_])magic-(tdx-rs|market-core|market-router|market-composition|eastmoney-rs|ths-rs|sina-rs|cninfo-rs|tencent-rs|cls-rs|jin10-rs|thepaper-rs|exchange-rs|baidu-rs)([^[:alnum:]_]|$)' Cargo.toml Cargo.lock
  reject "upstream Git source" 'magic-market-data-rs\.git' Cargo.toml Cargo.lock build.rs build_support tests
  reject "gateway feature" 'magic-gateway' Cargo.toml
  reject "TDX lock attestation" 'MAGIC_TDX_DEPENDENCY_REVISION|locked_magic_tdx_revision|magic_tdx_lock' build.rs build_support tests src/data_gateway/benchmark.rs
fi

if [[ "$scope" == "source" || "$scope" == "all" ]]; then
  reject "upstream Rust path" '\b(magic_tdx_rs|magic_market_core|magic_market_router|magic_market_composition|magic_eastmoney_rs|magic_ths_rs|magic_sina_rs|magic_cninfo_rs|magic_tencent_rs|magic_cls_rs|magic_jin10_rs|magic_thepaper_rs|magic_exchange_rs|magic_baidu_rs)\b' src tests build.rs build_support
  reject "gateway cfg" 'cfg\([^\n]*feature[[:space:]]*=[[:space:]]*"magic-gateway"' src tests build.rs
fi

if [[ "$scope" == "targets" || "$scope" == "all" ]]; then
  for path in src/grpc_server src/bin/grpc_market_server.rs src/bin/friday_full_replay.rs src/bin/hbars_probe.rs src/bin/rq_probe.rs src/bin/selection_live_probe.rs src/bin/t0_lib_probe.rs src/bin/t0_minute_probe.rs src/bin/t0_replay.rs src/bin/tdx_5min_probe.rs src/bin/tdx_raw_probe.rs src/bin/tdx_server_probe.rs src/bin/tencent_quote_probe.rs src/bin/virtual_pnl.rs
  do
    if [[ -e "$path" ]]; then
      printf 'provider-only target remains: %s\n' "$path" >&2
      failed=1
    fi
  done
fi

exit "$failed"
```

- [ ] **Step 2: Run the manifest scope and confirm it is red**

Run `bash scripts/check-no-magic-dependencies.sh manifest`.

Expected: exit `1`, with matches in `Cargo.toml`, `Cargo.lock`, `build.rs`, build support, and revision tests.

- [ ] **Step 3: Remove manifest features and dependencies**

Delete all 14 dependency rows and the complete `[features]` table from `Cargo.toml`. Do not replace them with empty features, path dependencies, patches, or vendored crates.

- [ ] **Step 4: Remove build-time lock revision ownership**

In `build.rs`, delete the `magic_tdx_lock` module declaration, the `Cargo.lock` read, the `MAGIC_TDX_DEPENDENCY_REVISION` environment injection, and `locked_magic_tdx_revision`. Keep protobuf merge/generation and its `rerun-if-changed` declarations.

Delete `build_support/magic_tdx_lock.rs` and both revision-enforcement integration tests. In `tests/test_coverage_thresholds.rs`, rename the synthetic fixture path from `build_support/magic_tdx_lock.rs` to `build_support/sample.rs`; the test still verifies build-support coverage authority without referencing deleted production code.

- [ ] **Step 5: Decouple benchmark compilation from the deleted lock revision**

Replace the unconditional build environment lookup with this transitional local constant so the feature-off library compiles until Task 4 deletes the local benchmark adapter:

```rust
const TDX_DEPENDENCY_REVISION: &str = "remote-grpc-contract-v1";
```

Delete benchmark tests that parse `Cargo.lock` or assert a resolved Git revision. Retain canonical request, admission, batch identity, and gRPC response evidence tests.

- [ ] **Step 6: Regenerate and verify the lockfile**

Run:

```bash
cargo check --offline --lib
bash scripts/check-no-magic-dependencies.sh manifest
cargo tree --locked | rg 'magic-|magic-market-data-rs' && exit 1 || true
```

Expected: all commands exit `0`; the final search prints nothing.

- [ ] **Step 7: Commit the dependency-graph removal**

```bash
git add Cargo.toml Cargo.lock build.rs src/data_gateway/benchmark.rs tests/test_coverage_thresholds.rs scripts/check-no-magic-dependencies.sh
git add -u build_support/magic_tdx_lock.rs tests/magic_market_release_revision.rs tests/magic_tdx_lock_contract.rs
git commit -m "refactor: remove magic dependency graph"
```

---

### Task 2: Promote local compatibility types to the market domain

**Files:**

- Create: `src/market_domain/{mod,bars,evidence,instrument,lifecycle,market,provider_id,ranking,record,tdx,value}.rs`
- Modify: `src/lib.rs`
- Modify: every Rust file returned by `rg -l 'crate::magic_compat|stock_analysis::magic_compat' src tests`
- Delete: `src/magic_compat/**`

**Interfaces:**

- Consumes: feature-off mirror types currently exported by `crate::magic_compat`.
- Produces: the same public fields, enums, serde forms, and constructors under `crate::market_domain`, with no upstream re-export branch.

- [ ] **Step 1: Write a red serialization ownership test**

Add to the future `src/market_domain/mod.rs` test module:

```rust
#[test]
fn provider_id_wire_names_are_stable() {
    let cases = [
        ("Tdx", ProviderId::Tdx),
        ("Tencent", ProviderId::Tencent),
        ("Eastmoney", ProviderId::Eastmoney),
        ("Sina", ProviderId::Sina),
        ("Custom", ProviderId::Custom),
    ];
    for (expected, provider) in cases {
        assert_eq!(format!("{provider:?}"), expected);
        assert_eq!(serde_json::to_string(&provider).unwrap(), format!("\"{expected}\""));
    }
}
```

Run `cargo test --offline --lib market_domain::tests::provider_id_wire_names_are_stable`; expected initial failure because `market_domain` does not exist.

- [ ] **Step 2: Move the local implementations without redesigning them**

Move the eleven feature-off implementation files from `src/magic_compat/` to `src/market_domain/`. In the new `mod.rs`, export only local modules and types:

```rust
pub mod bars;
pub mod evidence;
pub mod instrument;
pub mod lifecycle;
pub mod market;
pub mod provider_id;
pub mod ranking;
pub mod record;
pub mod tdx;
pub mod value;

pub use bars::{Adjustment, Bar, BarInterval};
pub use evidence::{EvidenceTimestamp, NonEmptyText, SourceEvidence};
pub use instrument::{AssetClass, CoreError, Exchange, InstrumentId};
pub use lifecycle::{CorporateActionCategory, CorporateActionStatus, CorporateActionTerms, UnverifiedSourceUnit};
pub use market::{FinancialLine, FinancialStatement, LimitPoolEntry, LimitPoolKind, MarketStatistics, StatementKind};
pub use provider_id::ProviderId;
pub use ranking::{DragonTigerSide, FxPair, GlobalIndexCode, MarketRankingKind, MarketRankingUnit};
pub use record::{DataBatch, FlowInterval, IsoDate, NorthboundChannel, Provenance, QualityReport};
pub use tdx::SecurityBar;
pub use value::{FiniteNumber, Money, PositiveU32, Price, Quantity, Ratio, RatioUnit};
```

Delete all upstream re-exports and comparison-only tests guarded by `magic-gateway`.

- [ ] **Step 3: Update imports and public module ownership**

Replace `pub mod magic_compat;` with `pub mod market_domain;` in `src/lib.rs`. Replace only Rust paths `crate::magic_compat`/`stock_analysis::magic_compat`; do not replace wire strings such as `magic-market-core.MarketDataProvider.bars.v0.2.0`.

- [ ] **Step 4: Verify domain compatibility**

```bash
cargo test --offline --lib market_domain::
cargo check --offline --lib
rg -n 'crate::magic_compat|stock_analysis::magic_compat|pub mod magic_compat' src tests && exit 1 || true
```

- [ ] **Step 5: Commit the domain ownership migration**

```bash
git add src/market_domain src/lib.rs src tests
git add -u src/magic_compat
git commit -m "refactor: own market domain types locally"
```

---

### Task 3: Make gRPC the mandatory transport seam

**Files:**

- Modify: `src/data_gateway/grpc_source.rs`
- Modify: `src/review/catalyst_review.rs`
- Modify: `src/bin/monitor/main.rs`
- Modify: gateway call sites returned by `rg -l 'bridge_for\(' src/data_gateway --glob '*.rs'`

**Interfaces:**

- Consumes: `bridge_for(op) -> Result<Option<Arc<GrpcSource>>, GatewayError>` and environment-controlled fallback.
- Produces: `bridge_for(op) -> Result<Arc<GrpcSource>, GatewayError>`, always configured from `GRPC_MARKET_ADDR`/client bundle and never selecting a local library.

- [ ] **Step 1: Replace old mode tests with red mandatory-transport tests**

Keep the existing user-added `no_current_reports_survives_reason_code_static` test. Replace old disabled/default-library/keep-local assertions with:

```rust
#[test]
fn bridge_exists_without_legacy_mode_environment() {
    let _env = test_grpc_env_guard();
    std::env::remove_var("DATA_GATEWAY_GRPC");
    std::env::remove_var("DATA_GATEWAY_GRPC_DISABLED");
    std::env::remove_var("GRPC_MARKET_ADDR");
    reset_bridge();
    assert!(bridge_for("RealtimeQuotes").is_ok());
}

#[test]
fn startup_banner_is_grpc_only() {
    let _env = test_grpc_env_guard();
    let banner = startup_banner();
    assert!(banner.contains("数据源模式 = grpc"), "{banner}");
    assert!(!banner.contains("library"), "{banner}");
    assert!(!banner.contains("保持本地"), "{banner}");
}

#[test]
fn every_declared_operation_is_remote() {
    assert!(KEEP_LOCAL_OPS.is_empty());
    assert!(HOOKED_OPS.contains(&"StrongStockReasons"));
    assert!(HOOKED_OPS.contains(&"ChainBatch"));
}
```

Run the three tests and confirm they fail against the old optional bridge.

- [ ] **Step 2: Collapse bridge construction to one remote path**

Change `bridge_for` to return `Result<Arc<GrpcSource>, GatewayError>`. Retain the existing `SOURCE` cache and lazy connection, but delete both legacy environment checks and all `Ok(None)` returns. Remove `DATA_GATEWAY_GRPC` and `DATA_GATEWAY_GRPC_DISABLED` from the environment guard, banner, readiness language, chain-batch logic, and tests. Set `KEEP_LOCAL_OPS` to `&[]`, add `StrongStockReasons` and `ChainBatch` to `HOOKED_OPS`, and make `fetch_chain_batch_grpc` always query the remote source.

- [ ] **Step 3: Update gateway call sites**

Replace optional branches such as:

```rust
if let Some(source) = super::grpc_source::bridge_for("RealtimeQuotes")? {
    return source.realtime_quotes(codes);
}
```

with mandatory calls:

```rust
return super::grpc_source::bridge_for("RealtimeQuotes")?.realtime_quotes(codes);
```

Delete the now-unreachable local-library branch and its `"DATA_GATEWAY_GRPC=1 required"` error. Preserve admission/audit code that runs after DTO conversion.

- [ ] **Step 4: Remove runtime mode selection from callers**

In `src/review/catalyst_review.rs`, always use the gRPC chain batch and remove the local `build_for_date` fallback. In `src/bin/monitor/main.rs`, update startup text to state the gRPC-only provider host and remove instructions for enabling `DATA_GATEWAY_GRPC`.

- [ ] **Step 5: Verify mandatory transport semantics**

```bash
cargo test --offline --lib data_gateway::grpc_source::tests::bridge_exists_without_legacy_mode_environment
cargo test --offline --lib data_gateway::grpc_source::tests::startup_banner_is_grpc_only
cargo test --offline --lib data_gateway::grpc_source::tests::every_declared_operation_is_remote
cargo test --offline --lib data_gateway::grpc_source::tests::no_current_reports_survives_reason_code_static
rg -n 'DATA_GATEWAY_GRPC(_DISABLED)?|library transport disabled' src tests && exit 1 || true
```

- [ ] **Step 6: Commit the mandatory seam**

Stage `src/data_gateway/grpc_source.rs` deliberately so the pre-existing `no_current_reports` change remains included, but leave `config/selection/selection_activation.v1.json` unstaged:

```bash
git add src/data_gateway/grpc_source.rs src/data_gateway src/review/catalyst_review.rs src/bin/monitor/main.rs
git commit -m "refactor: require remote market data transport"
```

---

### Task 4: Delete provider branches from general gateways

**Files:**

- Modify: `src/data_gateway/{benchmark,block_trade,board,board_runtime,capital,chain_intelligence,company,consensus,dragon_tiger,economic_calendar,event_calendar,futures_delivery,global_market,global_news,historical_bars,index,market_capabilities,market_data,outcome_daily_bars,research,review,security_lifecycle,sina_instrument_news}.rs`

**Interfaces:**

- Consumes: dual gRPC/provider gateway modules.
- Produces: the same business-facing request/result APIs backed only by `GrpcSource`, with local evidence admission retained.

- [ ] **Step 1: Establish the red source-path gate**

Run `bash scripts/check-no-magic-dependencies.sh source`.

Expected: exit `1` with upstream crate paths and `magic-gateway` cfg matches in the listed gateways.

- [ ] **Step 2: Remove provider-only imports and implementations**

In each listed file:

1. Keep public request/result/evidence types and gRPC conversion/admission functions.
2. Delete imports from all `magic_*` crates.
3. Delete provider client constructors, router/source functions, `spawn_blocking` provider calls, provider canaries, and tests that instantiate upstream errors or clients.
4. Delete every `#[cfg(feature = "magic-gateway")]` item.
5. Remove `#[cfg(not(feature = "magic-gateway"))]` from the surviving remote/error-neutral item.
6. Remove constants and helpers made unused by deleting provider transport.

Do not delete stable provider identity strings, gRPC DTO parsing, canonical hashes, freshness checks, evidence checks, or `GatewayError` classification.

- [ ] **Step 3: Verify the general gateway slice**

```bash
cargo check --offline --lib
cargo test --offline --lib data_gateway::grpc_source::
cargo test --offline --lib data_gateway::market_data::
cargo test --offline --lib data_gateway::historical_bars::
cargo test --offline --lib data_gateway::review::
```

Expected: all commands exit `0`; warnings caused by deleted provider branches are fixed in touched files rather than suppressed.

- [ ] **Step 4: Commit the general gateway cleanup**

```bash
git add src/data_gateway
git commit -m "refactor: remove local provider gateway branches"
```

---

### Task 5: Remove TDX-specific local modules while preserving T0 domain data

**Files:**

- Create: `src/data_gateway/t0_evidence.rs`
- Modify: `src/data_gateway/intraday_shape.rs`
- Modify: `src/data_gateway/grpc_source.rs`
- Modify: `src/data_gateway/mod.rs`
- Modify: callers returned by `rg -l 'magic_tdx(_t0|_selection)?' src tests --glob '*.rs'`
- Delete: `src/data_gateway/magic_tdx.rs`
- Delete: `src/data_gateway/magic_tdx_selection.rs`
- Delete: `src/data_gateway/magic_tdx_t0.rs`
- Delete: `tests/br192_t0_counted_binding.rs`
- Modify: `tests/unified_data_architecture.rs`

**Interfaces:**

- Consumes: T0 DTO/domain types mixed with TDX transport code.
- Produces: provider-neutral `T0Batch`, `T0Evidence`, `T0Quote`, daily/five-minute bars, rejection, freshness, and completeness validation under `data_gateway::t0_evidence`.

- [ ] **Step 1: Add a red gRPC T0 round-trip test using provider-neutral names**

Add a test to the future `src/data_gateway/t0_evidence.rs` that constructs one valid `T0Batch`, serializes it, deserializes it, and asserts equality of `batch_id`, `source_at`, quote price, five bid/ask levels, and `time_untrustworthy`. Run `cargo test --offline --lib data_gateway::t0_evidence::tests::t0_batch_wire_round_trip`; expected initial failure because the module does not exist.

- [ ] **Step 2: Extract only provider-neutral T0 types and validation**

Move the data structs and validation functions consumed by `grpc_source.rs`, `intraday_shape.rs`, monitor, and tests into `t0_evidence.rs`. Rename public types mechanically:

```text
MagicTdxT0Batch -> T0Batch
MagicTdxT0Evidence -> T0Evidence
MagicTdxT0Quote -> T0Quote
MagicTdxT0DailyBar -> T0DailyBar
MagicTdxT0FiveMinuteBar -> T0FiveMinuteBar
MagicTdxT0Rejection -> T0Rejection
```

Keep serialized field names and values unchanged. Delete `TdxHqClient`, raw bar/quote conversion, cached client, connection, paging, and direct fetch functions.

- [ ] **Step 3: Delete provider-only selection and wrappers**

Delete `magic_tdx_selection.rs`; its only production-style caller is the provider-only `selection_live_probe`, removed in Task 6. Delete the `MagicTdxGateway` wrapper and make `intraday_shape` call the mandatory `T0Evidence` gRPC operation through `GrpcSource`. Remove source-inspection tests that require deleted local TDX fetch functions, while retaining T0 completeness/freshness behavior tests against `t0_evidence`.

- [ ] **Step 4: Verify T0 semantics**

```bash
cargo test --offline --lib data_gateway::t0_evidence::
cargo test --offline --lib data_gateway::intraday_shape::
cargo check --offline --lib
rg -n 'magic_tdx(_t0|_selection)?|MagicTdx' src tests --glob '*.rs' && exit 1 || true
```

Provider identity strings such as `"magic-tdx"` are not part of this identifier search.

- [ ] **Step 5: Commit the TDX module removal**

```bash
git add src/data_gateway src tests
git add -u tests/br192_t0_counted_binding.rs
git commit -m "refactor: remove local tdx transport modules"
```

---

### Task 6: Delete the production provider server and provider-only binaries

**Files:**

- Delete: `src/grpc_server/**`
- Delete: `src/bin/grpc_market_server.rs`
- Delete: `src/bin/{friday_full_replay,hbars_probe,rq_probe,selection_live_probe,t0_lib_probe,t0_minute_probe,t0_replay,tdx_5min_probe,tdx_raw_probe,tdx_server_probe,tencent_quote_probe,virtual_pnl}.rs`
- Create: `tests/support/mod.rs`
- Create: `tests/support/grpc_fixture.rs`
- Modify: `src/lib.rs`
- Modify: `src/bin/grpc_local_readiness_probe.rs`
- Modify: `tests/grpc_channel_e2e.rs`
- Modify: `tests/grpc_bridge_e2e.rs`
- Modify: tests returned by `rg -l 'grpc_server|grpc_market_server' tests src/bin --glob '*.rs'`

**Interfaces:**

- Consumes: the production in-process tonic server and provider-only executable targets.
- Produces: no production server target; client integration tests use a test-local tonic fixture built from generated server traits and canned provider-neutral responses.

- [ ] **Step 1: Run the target gate and confirm it is red**

Run `bash scripts/check-no-magic-dependencies.sh targets`.

Expected: exit `1` listing the server directory and provider-only binaries.

- [ ] **Step 2: Move only fixture behavior into integration tests**

Move the provider-neutral canned payloads from `src/grpc_server/fixture.rs`, the deterministic event hub from `src/grpc_server/events.rs`, and the fixture-only tonic service wiring used by `tests/grpc_channel_e2e.rs` into `tests/support/grpc_fixture.rs`. Export `start_fixture_server` and `FixtureServerGuard` from `tests/support/mod.rs`. Both integration tests import this support module. Do not move `delegate.rs`, provider clients, database initialization, production event polling, or production configuration into test support.

Change `tests/grpc_bridge_e2e.rs` from spawning `CARGO_BIN_EXE_grpc_market_server` to calling `support::start_fixture_server(0)`, then run existing client assertions against the returned loopback address.

- [ ] **Step 3: Delete production server ownership**

Delete `src/grpc_server/**`, remove `pub mod grpc_server` from `src/lib.rs`, and delete `grpc_market_server.rs`. Delete the provider-only binaries listed above. Keep client-only probes such as `grpc_bundle_probe`, `grpc_local_readiness_probe`, and `gateway_quote_probe`; update `grpc_local_readiness_probe` to target `GRPC_MARKET_ADDR` instead of starting an in-process server.

- [ ] **Step 4: Verify target removal and client tests**

```bash
bash scripts/check-no-magic-dependencies.sh targets
cargo test --offline --test grpc_channel_e2e
cargo test --offline --test grpc_bridge_e2e
cargo check --offline --bins
```

Expected: all commands exit `0`; Cargo metadata contains no `grpc_market_server` or deleted provider probe target.

- [ ] **Step 5: Commit the server/target removal**

```bash
git add src/lib.rs src/bin tests
git add -u src/grpc_server
git commit -m "refactor: remove in-repository provider host"
```

---

### Task 7: Remove residual feature code, update active documentation, and run the full gate

**Files:**

- Modify: every path returned by `rg -l 'magic-gateway|magic_market_|magic_[a-z0-9_]+_rs' src tests build.rs build_support Cargo.toml`
- Modify: active architecture/readme/runbook files that instruct users to build or enable the local provider host
- Modify: `docs/superpowers/plans/2026-08-15-p4-migration.md`
- Modify: `docs/superpowers/specs/2026-08-23-provider-host-repository-split-design.md`

**Interfaces:**

- Consumes: the dependency-free client-only implementation from Tasks 1–6.
- Produces: a clean static gate, current operator documentation, and fresh verification evidence.

- [ ] **Step 1: Run the full static gate and use every match as a deletion checklist**

Run `bash scripts/check-no-magic-dependencies.sh all`.

Expected initially: exit `1` only if residual source paths/cfgs remain. Remove code made unreachable by this migration, unwrap surviving feature-off code, and fix imports. Do not suppress `unexpected_cfgs`, `unused`, or `dead_code` warnings to hide residue.

- [ ] **Step 2: Update active documentation**

Mark M5 deletion complete in active migration documents. Replace commands that build `grpc_market_server` or enable `magic-gateway`/`DATA_GATEWAY_GRPC` with external provider-host configuration via `GRPC_MARKET_ADDR` and the existing client bundle. Preserve historical provider identities.

- [ ] **Step 3: Format only migration-touched Rust files**

Run `rustfmt` through Cargo only after reviewing the touched-file list. If `cargo fmt --check` reports unrelated pre-existing formatting, format explicit migration paths instead of the entire dirty workspace.

- [ ] **Step 4: Run fresh verification**

```bash
bash scripts/check-no-magic-dependencies.sh all
cargo tree --locked | rg 'magic-|magic-market-data-rs' && exit 1 || true
cargo check --locked --offline --lib
cargo check --locked --offline --bins
cargo test --locked --offline --lib
cargo test --locked --offline --test grpc_channel_e2e
cargo test --locked --offline --test grpc_bridge_e2e
cargo test --locked --offline --test unified_data_architecture
git diff --check
```

Expected: every command exits `0`; static searches print nothing; tests report zero failures.

- [ ] **Step 5: Audit preserved user changes and scope**

```bash
git diff -- config/selection/selection_activation.v1.json
git diff -- src/data_gateway/grpc_source.rs
git status --short
```

Confirm the selection activation diff is unchanged from the pre-migration snapshot, `no_current_reports` and its test remain present, and root-level untracked files remain untouched.

- [ ] **Step 6: Commit final cleanup**

```bash
git add scripts Cargo.toml Cargo.lock build.rs src tests docs/superpowers/plans/2026-08-15-p4-migration.md docs/superpowers/specs/2026-08-23-provider-host-repository-split-design.md
git commit -m "refactor: complete grpc-only market data migration"
```

Do not stage `config/selection/selection_activation.v1.json` or unrelated root-level files.
