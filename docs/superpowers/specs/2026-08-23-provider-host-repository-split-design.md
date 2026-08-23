# Provider Host Repository Split Design

**Status:** Gate A candidate. The user approved the architecture, evidence
ownership, failure semantics, migration sequence and test strategy on
2026-08-23. This document does not claim Gate B, C or D completion.

## 1. Goal

Remove every live `magic-*` package and source reference from the
`stock_analysis` repository. Move public market/news provider acquisition into
an independently built and deployed `magic-market-provider` repository. Keep
`stock_analysis` responsible for evidence admission, freshness, trading
decisions, order safety and durable audit.

The end state is:

- `stock_analysis` source, `Cargo.toml`, `Cargo.lock` and Cargo dependency graph
  contain no `magic-*` package;
- `stock_analysis` cannot call a provider library or silently fall back to one;
- `provider-host` is the only module allowed to depend on `magic-*` adapters;
- the two repositories communicate only through a versioned, provider-neutral
  gRPC contract;
- data-source, freshness, validation and audit failures remain explicit; and
- a focused `stock_analysis` edit no longer recompiles provider implementations.

## 2. Current evidence

The design is based on reproducible checks against HEAD on 2026-08-23:

- `cargo metadata --no-deps --format-version 1` reports 82 Cargo targets;
- `rg -n '#\[(tokio::)?test\]' src tests` reports approximately 3,765 tests;
- `du -sh target` reports 41 GiB;
- a focused invocation of one library test spent 5m43s compiling the
  `stock_analysis` test target and 0.00s executing the test (`real 357.24s`);
- an unchanged repeat of the same invocation completed in `real 2.54s`;
- `cargo tree --no-default-features` contains no `magic-*` packages;
- the default Cargo graph still contains all fourteen direct `magic-*`
  dependencies plus their transitive provider packages;
- `src/data_gateway/grpc_source.rs::KEEP_LOCAL_OPS` still contains
  `strong_stock_reasons`; and
- `docs/superpowers/plans/2026-08-15-p4-migration.md` describes complete
  manifest removal as an M5 end state, while the implemented feature-flag
  approach retains provider-host and provider dependencies in the same package.

The current feature split therefore removes providers from the production
monitor build but does not remove them from this repository or from its default
development/test graph.

## 3. Scope

### In scope

- a new `magic-market-provider` repository;
- a provider-neutral `market-contract` package;
- migration of `grpc_market_server`, provider adapters and provider probes;
- a `MarketEvidencePort` seam in `stock_analysis`;
- completion of the `strong_stock_reasons` gRPC operation;
- removal of local provider/library fallbacks;
- removal of `magic_compat`, `magic-gateway`, all `magic-*` dependencies and
  provider-only targets from `stock_analysis`;
- cross-repository compatibility, shadow, production and rollback evidence;
- repository instructions and architecture documentation updates; and
- build-time measurement after provider removal.

### Non-goals

- changing signal, selection, ranking or order semantics;
- changing freshness windows or any portfolio/order threshold;
- introducing a generic event bus or a new transport in addition to gRPC;
- allowing provider DTOs to enter trading or decision code directly;
- changing the pinned `magic-market-data-rs` revision during the split;
- using mock/fake/default data in a production path; or
- weakening full Gate C/D validation to make development faster.

## 4. Options considered

### 4.1 Chosen: two repositories, contract package in provider repository

`magic-market-provider` is a Cargo workspace containing a small
`market-contract` package and the deep `provider-host` module. `stock_analysis`
pins only `market-contract`; it does not join the provider workspace.

This keeps the number of repositories small, gives the gRPC seam one canonical
contract source and removes provider compilation from every `stock_analysis`
workspace command.

### 4.2 Rejected: third repository for the contract

A neutral contract repository gives clean ownership but adds another release,
compatibility and CI workflow without a current independent consumer. It may be
reconsidered only if a second non-provider contract producer appears.

### 4.3 Rejected: copy proto sources into both repositories

Vendoring two writable copies is initially simple but makes contract drift a
normal state. Descriptor hashes can detect drift after it occurs; they do not
provide one owner. This is rejected for a live trading data seam.

## 5. Target architecture

```text
Third-party market/news providers
               |
               v
+-----------------------------------------------+
| magic-market-provider repository              |
|                                               |
| provider-host module                          |
| - magic-* adapters                            |
| - provider routing                            |
| - raw-response validation                     |
| - provider evidence + acquisition audit       |
|                                               |
| market-contract package                       |
| - proto + generated DTOs                      |
| - closed errors                               |
| - operation/capability catalog                |
| - version + descriptor hash                   |
+----------------------|------------------------+
                       | mTLS gRPC
                       v
+-----------------------------------------------+
| stock_analysis repository                     |
|                                               |
| MarketEvidencePort interface                  |
| - GrpcMarketAdapter (production)              |
| - InMemoryTestAdapter (cfg(test), TEST_CODE)  |
|                                               |
| data_gateway deep module                      |
| - canonical identity                          |
| - freshness and evidence admission            |
| - Admitted<T> construction                    |
|                                               |
| trading / decision / monitor / push           |
+-----------------------------------------------+
```

The gRPC transport is a remote-but-owned dependency. `MarketEvidencePort` is a
real seam because it has two justified adapters: the production gRPC adapter
and a test-only in-memory adapter. Internal provider adapters are not exposed
through the interface.

## 6. Module responsibilities

### 6.1 `market-contract`

The package owns the smallest complete interface between repositories:

- provider-neutral request/response DTOs;
- canonical operation identifiers and capability declarations;
- contract version and deterministic descriptor/catalog hashes;
- typed evidence metadata;
- closed error/disposition types; and
- source-backed price-limit state (`Bounded`, `NoLimit`, `Unavailable`).

It contains no provider implementation, environment lookup, database access,
business calculation, `magic-*` type or production fallback. Additive contract
changes remain compatible for at least two deployed releases. A removal or
semantic reinterpretation requires a new major contract version and an explicit
cross-repository cutover design.

### 6.2 `provider-host`

The module hides all provider complexity behind the gRPC interface. It owns:

- `magic-*` dependencies and pinned revision policy;
- provider-specific identifiers, clients and error conversion;
- provider routing and source-specific retry decisions;
- raw completeness, value, continuity, duplicate and conflict validation;
- conversion from provider types into contract DTOs; and
- acquisition-side tamper-resistant audit.

Provider failure returns a typed failure. It never reports success with a fake,
default, silently truncated or unqualified empty result.

### 6.3 `MarketEvidencePort`

The interface represents evidence acquisition from the application's point of
view. Callers do not learn about tonic channels, provider clients, feature flags
or provider-specific errors.

The production `GrpcMarketAdapter` owns mTLS, deadlines, contract negotiation
and transport-to-domain failure conversion. `InMemoryTestAdapter` exists only
under test compilation, accepts only `TEST_CODE` identities and cannot be
constructed by a production binary.

### 6.4 `stock_analysis::data_gateway`

This remains the application admission owner. It consumes contract DTOs and
constructs private-field `Admitted<T>` values only after verifying:

- canonical instrument and trading-session identity;
- completeness and provider evidence;
- source and local acquisition times without substitution;
- the applicable 2.4 freshness window;
- price positivity and finite values;
- time continuity and duplicate absence;
- split/dividend consistency where applicable; and
- source-backed order-range evidence where required.

Trading, decision, monitoring and push modules consume only admitted domain
values. They do not consume raw gRPC DTOs.

## 7. Evidence ownership and audit chain

The provider repository records acquisition facts. `stock_analysis` records
admission and decision facts. Both retain the same immutable `batch_id` (and
event identity where applicable), allowing an exact join:

```text
provider request
  -> raw provider response
  -> normalized contract batch
  -> stock_analysis admission
  -> decision/order/push outcome
```

Provider-side audit includes provider identity, provider timestamp when
supplied, local acquisition timestamp, immutable batch identity, completeness,
validation result and retryability. Application-side audit includes the same
batch identity, freshness decision, rejection/admission basis and downstream
decision rule IDs.

Neither audit may expose credentials, account identifiers, real holding lists
or other protected values. Both repositories retain critical audit for at least
five years and fail closed on append/hash/sync failure.

## 8. Failure semantics

The contract distinguishes at least:

- `TransportUnavailable`: gRPC, mTLS, DNS or connection failure;
- `ProviderUnavailable`: upstream source failed or exhausted valid routes;
- `ContractMismatch`: version, descriptor hash or catalog mismatch;
- `EvidenceRejected`: missing, stale, partial, invalid, discontinuous,
  duplicated or conflicting evidence; and
- `OperationUnsupported`: provider-host does not implement the requested
  operation/capability.

All states are explicit and fail closed. In particular:

- no error converts into an empty success;
- no missing timestamp is replaced by database or process time;
- no transport/provider failure falls back to local `magic-*` code;
- a contract mismatch rejects production channel startup and emits an explicit
  disabled/error banner;
- `Unavailable` price-limit evidence rejects an order; and
- `NoLimit` is accepted only with same-instrument, same-session provider
  evidence and never inferred from zero, infinity, code/board/ST name or a
  default percentage.

## 9. Migration sequence

Each phase is a separately reviewable PR/change set. A later phase cannot start
until the previous phase's acceptance evidence is complete.

### Phase 1: extract the contract

Create the `magic-market-provider` repository and its `market-contract`
package. Move the canonical proto, generated DTO policy, operation catalog,
closed errors, version and hashes without changing wire behavior.

Acceptance:

- old and new descriptor/catalog hashes are byte-identical;
- bidirectional serialization tests pass;
- `stock_analysis` behavior is unchanged; and
- both repositories pin the same contract revision.

### Phase 2: move provider implementation

Move with history where practical:

- `src/grpc_server/**`;
- `src/bin/grpc_market_server.rs`;
- provider-specific halves of `src/data_gateway/**`;
- TDX/Tencent/provider probe binaries;
- provider live/integration tests; and
- all `magic-*` dependency declarations.

The original paths remain temporarily available only to support comparison and
rollback; no new business behavior is added to them.

Acceptance:

- provider-host serves every already-hooked operation;
- provider live-data and protocol checks pass;
- provider acquisition audit is durable; and
- the old and new server responses have equal contract hashes for the same
  captured request/evidence window.

### Phase 3: close the final operation gap

Implement and verify the complete `strong_stock_reasons` gRPC path. Move the
operation from `KEEP_LOCAL_OPS` to `HOOKED_OPS`, then require
`KEEP_LOCAL_OPS.is_empty()`.

Any filter, sort, limit or dedup semantics touched in this move must first be
registered or updated in `docs/business_rules.md` with corresponding BR IDs
(2.10). No local fallback is deleted until fidelity and failure behavior are
verified.

### Phase 4: shadow and authoritative cutover

Deploy a backward-compatible provider-host. Run it as a non-authoritative
shadow against the still-authoritative old path for at least one complete
trading day. Compare completeness, provider/source identity, timestamps,
values, ordering and deterministic result hashes.

Shadow output is real data but cannot participate in orders, decisions or push
delivery. Every discrepancy is an explicit blocking record. After a clean
window, deploy a gRPC-only `stock_analysis` release and make provider-host the
single authority. An outage after cutover fails closed; it does not reactivate
the old path.

### Phase 5: delete provider references from `stock_analysis`

Delete:

- `src/magic_compat/**`;
- `src/grpc_server/**` and `grpc_market_server` target;
- provider-specific gateway implementations and provider-only probes/tests;
- all `#[cfg(feature = "magic-gateway")]` branches;
- the `magic-gateway` feature and its default activation;
- all fourteen direct `magic-*` dependencies and remaining transitive lockfile
  packages; and
- `DATA_GATEWAY_GRPC_DISABLED`, library fallback and other switches that could
  select a local provider implementation.

Keep and deepen:

- `src/grpc_client/**` as the production adapter implementation;
- provider-neutral admission logic in `src/data_gateway/**`;
- canonical identity, evidence and freshness validation;
- order/decision/audit modules; and
- process-level gRPC compatibility tests.

### Phase 6: cleanup and performance evidence

Update repository architecture/instruction documents, remove obsolete M5
claims, rerun both repositories' complete gates and record the focused build
measurement on the same host/toolchain as the 357.24s baseline.

## 10. Test strategy

### 10.1 Contract package

- deterministic descriptor/catalog hash tests;
- request, success and every closed failure round trip;
- compatibility tests for the previous supported contract release;
- a source scan proving no `magic_*` type appears in the public interface; and
- capability negotiation mismatch tests.

### 10.2 Provider repository

- adapter-level parsing and validation tests;
- explicit missing/bad/stale/duplicate/gap/conflict failure tests;
- provider timeout, rate limit and unavailable tests;
- process-level gRPC tests for every operation;
- real-data canaries for registered providers; and
- at least 95% coverage of core acquisition, validation and audit links.

Test fixtures and adapters are test-only, use `TEST_CODE` namespaces and cannot
enter a production binary. Production validation uses real provider evidence.

### 10.3 `stock_analysis`

- tests through `MarketEvidencePort`, not provider implementation internals;
- gRPC process tests for successful and rejected evidence;
- transport, mTLS, version, unsupported-operation and provider-error tests;
- freshness and canonical-identity admission tests;
- assertions that downstream decision/order/push call counts stay zero for
  every rejected input; and
- test/live account, symbol, database, log and audit isolation tests.

Tests made obsolete by moving a shallow provider implementation are moved or
replaced at the new interface. They are not duplicated across repositories.

### 10.4 Cross-repository production evidence

- all registered operations reach provider-host through gRPC;
- `KEEP_LOCAL_OPS` is empty before cutover and removed after deletion;
- provider and application audit rows join exactly by `batch_id`;
- realtime quotes remain within 5 seconds, position/cash within 30 seconds, net
  value within the same trading day and daily/history within one trading day;
- shadow discrepancies are zero or block cutover with an explicit record; and
- production logs prove the new provider-to-admission-to-consumer chain.

## 11. Acceptance criteria

After Phase 5, the following checks must pass in `stock_analysis`:

```bash
cargo metadata --format-version 1 | jq -e '
  [.. | objects | .name? // empty | select(startswith("magic-"))]
  | length == 0'

! rg -n \
  'magic_(tdx|market_|eastmoney|sina|tencent|ths|cninfo|cls|jin10|thepaper|exchange|baidu)' \
  src tests build.rs Cargo.toml

! rg -n 'magic-gateway|KEEP_LOCAL_OPS|DATA_GATEWAY_GRPC_DISABLED' \
  src tests build.rs Cargo.toml

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

The focused build benchmark repeats the original command after a real focused
source change invalidates the application library target:

```bash
/usr/bin/time -p cargo test --lib \
  block_on_async_with_timeout_panics_with_flavor_error_in_current_thread
```

The result must be recorded beside the 357.24s baseline. The architecture
objective is at least a 50% reduction on the same host, toolchain, feature set
and dependency-cache state. A slower result blocks the build-performance goal
but never permits a data or fund safety check to be skipped.

Equivalent complete format, strict lint, tests, compliance, coverage and
provider live-data checks must pass in `magic-market-provider` before its
release is accepted.

## 12. Failure modes and rollback

| Failure | Required behavior | Rollback |
| --- | --- | --- |
| Contract hash/version mismatch | Reject startup/channel; explicit banner | Deploy last compatible contract/provider artifact |
| Provider-host unavailable | Explicit retryable transport failure; zero local fallback | Roll back to previous verified application binary only if its provider path remains operational |
| Provider response incomplete/stale | Reject batch; zero decision/order/push consumption | Fix provider adapter or routing; do not fabricate evidence |
| Shadow discrepancy | Block authoritative cutover | Keep old path authoritative and correct Phase 2/3 |
| Audit append/hash/sync failure | Fail closed | Repair audit storage; do not disable audit |
| Phase 5 compile/test regression | Return to Gate B; recheck Gate A seam | `git revert` the scoped Phase 5 commit/PR |
| Production cutover regression | Stop new monitor, restore last verified binary; provider-host remains backward compatible | Artifact rollback plus audited incident record |

Rollback is release/commit based. Runtime environment variables cannot restore
deleted compile-time implementations. Provider-host must support the previous
contract for at least one rollback window, and signed/verified prior binaries
must be retained before cutover.

## 13. Old module disposition

| Existing module/path | Decision | Reason |
| --- | --- | --- |
| `src/grpc_client/**` | adopt and deepen | Production adapter at the gRPC seam |
| `src/grpc_contract/**`, `grpc/market.proto` | move/bridge to `market-contract` | One canonical cross-repository interface |
| `src/grpc_server/**` | move | Provider-host implementation belongs in provider repository |
| `src/bin/grpc_market_server.rs` | move | Provider-host executable must not be a stock-analysis target |
| provider-specific `src/data_gateway/**` code | move | Provider adapters must not leak into application admission |
| provider-neutral `src/data_gateway/**` code | adopt and deepen | Owns canonical evidence admission and `Admitted<T>` |
| `src/magic_compat/**` | delete after cutover | Transitional mirror; keeping it would preserve provider coupling |
| `magic-gateway` feature | delete after cutover | Feature isolation is not repository isolation |
| provider probes/replays | move or explicitly retire | Their dependency owner is provider-host |
| `KEEP_LOCAL_OPS` / local library fallback | delete after final operation cutover | A second production path would violate single authority/fail-closed behavior |

## 14. Data-redline mapping

- **2.1:** provider failure never becomes mock/default/empty data; production
  has only the gRPC adapter.
- **2.2:** missing contract/provider fields remain absent or reject admission.
- **2.3:** provider and application admission validate values, continuity,
  duplicates, corporate actions and source-backed price-limit evidence.
- **2.4:** application admission enforces existing freshness windows; shadow
  and live cutover cannot bypass the freshness script/gate.
- **2.5:** test adapters, symbols, accounts, databases, logs and audit use
  `TEST_CODE` and remain physically isolated.
- **2.6:** orders consume same-session source-backed `Bounded`/`NoLimit` state;
  `Unavailable` rejects.
- **2.7:** provider acquisition and application decision audits join by
  immutable batch/event identity and retain at least five years.
- **2.8:** moved save/sync/reconcile operations remain real; logging-only
  implementations are forbidden.
- **2.10:** any moved/refined dedup, mutex, filter, sort or limit behavior is
  registered before implementation and cited by BR ID.

## 15. PR evidence requirements

Every migration PR includes:

- `Refs:` this design section and the relevant contract/provider spec;
- `Data-Redlines:` the exact applicable 2.x rules;
- `OldModules:` adopt/move/delete decision for each touched module;
- `Threshold-Proof:` `N/A` unless a real threshold/config field changes;
- `Business-Rules:` affected BR IDs, especially for
  `strong_stock_reasons` and routing/filter behavior;
- `Validation:` focused checks plus the gate required for that phase; and
- `Rollback:` exact commit/artifact rollback steps.

No phase is release-ready while a blocking review finding, freshness failure,
contract mismatch, shadow discrepancy, missing audit join, coverage gap or
production evidence gap remains open.
