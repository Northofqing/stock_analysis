# BR-192 / BR-194 Incomplete-Commit Recovery Design

**Status:** Gate A repair after independent RED review; Gate B prohibited until the exact active
rule-ledger additions, this design and the companion manifest receive fresh C0/I0/M0 review

**Business rules:** BR-051, BR-159, BR-162, BR-190, BR-192, BR-194, BR-203

**Fixed baseline:** `9307b6785420c32b57fe210f9c9b870d83e4a52d`

The immutable object aliases used throughout this document and the companion
manifest are closed:

| Alias | Immutable Git object |
| --- | --- |
| `BASELINE` | `9307b6785420c32b57fe210f9c9b870d83e4a52d` |
| `TRACKED_WIP` | `2a4d1b929507fadadb082c2a803d5fea50cf6dd8` |
| `UNCHANGED_INDEX` | `b2981c4cc84a0d277bf07f51346ac6da84cbcb71` |
| `UNTRACKED_WIP` | `1389098b395a8894578259463923d58ab580a8b6` |
| `PREVIOUS_PARENT` | `b4aeee68d2c0259cc968914b3d39e3a89a18a496` |

An alias is only presentation shorthand for the complete object ID in this
table. A short SHA or ellipsis is never an extraction authority.

## 1. Problem statement

Commit `BASELINE` claims that the BR-194 Gate B/C/D implementation was closed,
but it omitted the production callers, unified R-04/R-09 gateways, counted
delivery schema-v3 implementation, immutable append authority and process
tests that its own checker and runtime require. The committed tree therefore
fails to build at `src/event/dispatcher.rs` because
`COUNTED_DELIVERY_AUDIT_SCHEMA_VERSION` is absent.

The omitted code is recoverable from the exact pre-merge stash objects:

| role | immutable Git object |
| --- | --- |
| tracked working-tree snapshot | `TRACKED_WIP` |
| unchanged index snapshot | `UNCHANGED_INDEX` |
| untracked-files snapshot | `UNTRACKED_WIP` |

Applying the complete stash to its exact parent reconstructs a coherent
workspace: `cargo check --lib`, three exact BR-192 tests, the frozen BR-194
checker, 31 monitor BR-194 tests and three process-isolation BR-194 tests pass.
This evidence proves recoverability; it does **not** authorize importing all
213 changed files as one patch.

The first frozen review packet was rejected by three independent reviews. It
incorrectly treated broad dirty-worktree versions of `docs/business_rules.md`,
`Cargo.toml`, `Cargo.lock` and `tests/magic_market_release_revision.rs` as P1
authority. Those bytes include later BR-198/BR-200/BR-202 semantics, unrelated
dependency/profile changes and five test inputs absent from the fixed
baseline. Their former blob identities are rejection evidence only and must
not be copied, staged or used as Gate-A authority.

### 1.1 Reproducible recovery evidence

The commands below were rerun on 2026-08-03 in the repository-owned safe clone
`target/br194-stash-audit-20260803`, with repository-relative
`CARGO_TARGET_DIR=target`. They
performed no production database, provider or sink operation.

```text
$ env CARGO_TARGET_DIR=target cargo check --lib
Checking stock_analysis v0.1.2 (target/br194-stash-audit-20260803)
Finished `dev` profile [unoptimized + debuginfo] target(s) in 1m 00s

$ env CARGO_TARGET_DIR=target cargo test --lib event::envelope::tests::br192_counted_delivery_v3_binds_attempt_artifact_result_and_receipt -- --exact
test event::envelope::tests::br192_counted_delivery_v3_binds_attempt_artifact_result_and_receipt ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 2316 filtered out

$ env CARGO_TARGET_DIR=target cargo test --lib event::push_record::tests::br192_authoritative_record_requires_the_exact_counted_join -- --exact
test event::push_record::tests::br192_authoritative_record_requires_the_exact_counted_join ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 2316 filtered out

$ env CARGO_TARGET_DIR=target cargo test --lib event::delivery_observation_tests::br192_counted_delivery_is_persisted_before_success_returns -- --exact
test event::delivery_observation_tests::br192_counted_delivery_is_persisted_before_success_returns ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 2316 filtered out

$ bash tools/compliance/lib/check_br194_review_dependency.sh
BR-194 review dependency static contract: PASS

$ env CARGO_TARGET_DIR=target cargo test --bin monitor br194_ -- --nocapture
test result: ok. 31 passed; 0 failed; 0 ignored; 0 measured; 495 filtered out

$ env CARGO_TARGET_DIR=target cargo test --test monitor_help_isolation br194_ -- --nocapture
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 21 filtered out

$ env CARGO_TARGET_DIR=target cargo test --lib database::data_acquisition_audit::tests::br159_ -- --test-threads=1
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 2313 filtered out

$ env CARGO_TARGET_DIR=target cargo test --lib data_gateway::dragon_tiger::tests::br162_ -- --test-threads=1
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 2311 filtered out

$ env CARGO_TARGET_DIR=target cargo test --lib data_gateway::capital::tests::provider_top_n_ -- --test-threads=1
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 2314 filtered out

$ env CARGO_TARGET_DIR=target cargo test --lib event::durable_delivery_append::tests::br192_ -- --test-threads=1
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 2308 filtered out

$ git show b4aeee68d2c0259cc968914b3d39e3a89a18a496:Cargo.toml | rg -n 'rusqlite|magic-market|magic-tdx|5f1ce936|d7dfa314'
49:rusqlite = { version = "0.31", features = ["chrono"] }
54:magic-tdx-rs = { package = "magic-tdx-rs", path = "../magic-market-data-rs/crates/magic-tdx-rs" }

$ git show 9307b6785420c32b57fe210f9c9b870d83e4a52d:Cargo.toml | rg -n 'rusqlite|magic-market|magic-tdx|5f1ce936|d7dfa314'
49:rusqlite = { version = "0.31", features = ["chrono"] }
54:magic-tdx-rs = { package = "magic-tdx-rs", path = "../magic-market-data-rs/crates/magic-tdx-rs" }
```

The non-zero counts are part of the evidence. A future filtered command that
matches zero tests is a failure even if Cargo exits successfully.

## 2. Goals and non-goals

### Goals

1. Restore the previously designed unified R-04 and R-09 source contracts that
   BR-194 adopts as prerequisites.
2. Restore the complete counted-delivery schema-v3 audit authority. A constant
   without envelope, parser, persistence, append and finalization support is
   prohibited.
3. Restore the frozen BR-194 SourceOnly/account-dependent partition, canonical
   merge, production callers and test/live isolation.
4. Preserve exact provider evidence, freshness, failure and durable audit
   semantics. Missing fields remain missing and provider failure remains an
   explicit failure.
5. Produce clean, reviewable commits and a PR rather than modifying the broad
   dirty `feat/event-scoped-selection-shadow` worktree.

### Non-goals

- No schema-v4/v5/v6, source-batch or provider-free retry changes.
- No BR-197, BR-198, BR-199 or BR-200 behavior.
- No R-08 SourceOnly promotion and no A-10/A-01 SourceOnly promotion.
- No weakening or removal of AGENTS.md rule 2.3, including the current
  adjacent-change alert/manual-confirmation requirement.
- No destructive schema migration, schema downgrade, production provider call,
  Feishu send or production data mutation during recovery implementation. P1
  does install the already specified BR-159 append-only acquisition-audit
  table, indexes and triggers in isolated test databases; that additive,
  idempotent schema installation is an explicit migration and is retained by
  every forward-compatible rollback build.
- No wholesale copy of `main.rs`, `notify.rs`, `push_templates.rs`,
  `review.rs`, the complete `src/data_gateway/` tree or the complete stash.

## 3. Triggered data red lines

- **2.1 / 2.2:** R-04 uses only admitted Eastmoney batches; R-09 uses only
  admitted provider Top-N batches. Empty and unavailable remain distinct.
- **2.3:** Existing R-04 date, identity, amount, unique rank and exact
  buy-five/sell-five checks remain unchanged. No bad row is silently admitted.
- **2.4:** Provider `source_at`, post-response `observed_at` and batch identity
  remain mandatory where registered by BR-162/BR-190.
- **2.5:** `--test --review` rejects real R-04/R-09 provider and sink calls
  before database/provider initialization and uses a `TEST_CODE` namespace.
- **2.7:** Counted delivery must bind decision, attempt, artifact, sink result,
  receipt and immutable audit bytes before success returns.
- **2.8:** A declared verifier/append/push authority must be reachable and must
  perform its real persistence or delivery operation.
- **2.10:** All partition, filter, stable-sort, limit and duplicate semantics
  remain those already registered by BR-162, BR-190, BR-192 and BR-194.

## 4. Recovery slices and ownership

The slice labels preserve rule ownership; implementation order is **P0 → P2 →
P1 → P3 → P4**. P0 is the documentation-only Rule-2.10 registration and must
be independently accepted before any P2 source byte is staged. The fixed first parent cannot compile because its committed dispatcher
already references the missing counted schema-v3 constant, so requiring P1 to
compile before P2 would be physically impossible. P2 first restores only the
unreachable counted authority and returns the fixed parent to a compilable
state. P2 is nevertheless a reachable shared event/push-log migration, not an
"unreachable-only" edit: existing schema-v2 publication and generic push-log
callers must retain byte-compatible behavior while schema-v3 authority is
added. P1 then installs the source prerequisites without a production caller.
P3 atomically installs both production callers. P4 then closes release-only
coverage and exact-count tooling without changing business behavior. Every implemented slice must
compile and pass its focused tests before the next slice starts.

The exact immutable-object admission set is maintained in
`2026-08-03-br192-br194-recovery-hunk-manifest.md`. That manifest is part of
this Gate-A design and must have no pending row before independent review.

### Slice P0: documentation-only recovery registration

Before any recovery source slice is staged, P0 uses two docs-only commits. `P0-M0` materializes the
frozen active-ledger preimage without BR-203 together with this recovery design, the companion hunk
manifest and the preserved BR-204 Gate-A design. Its tree contains no recovery source/config/runtime
change. `P0-A1` is the direct child of `P0-M0` and adds the canonical BR-203 row to the active
`docs/business_rules.md` ledger without changing any pre-existing row or any other path. The row registers the incomplete-
commit recovery sequence, immutable upstream/package closure, rejected
facades, exact-count verifier and golden compatibility obligations. Its Code
cell names only this design and the companion hunk manifest, which are already
materialized Gate-A authorities; future P1/P2/P3 source paths remain in the
manifest until they exist. The historical `BASELINE:docs/business_rules.md` blob
`a5325bdfb381ed187f1acbf70819260f38e18646` (SHA-256
`2c1d3634b38649ecb804a525bc896db0c9989eab9903dd54fc3ba1e7b0a312b9`) is extraction evidence only;
it must not overwrite later accepted rule-ledger additions or amendments. P0 changes no source,
configuration, dependency, lockfile, test or runtime behavior. The two committed trees record the
complete pre/post ledger hashes and prove that the only `P0-A1` semantic addition is BR-203;
independently owned,
already-present rule rows such as BR-204 are preserved byte-for-byte.

| path | ownership in P0 |
| --- | --- |
| `docs/business_rules.md` | `P0-M0`: materialize the frozen active-ledger preimage; `P0-A1`: add only the canonical BR-203 row while preserving every pre-existing row byte-for-byte |
| recovery design, hunk manifest, BR-204 Gate-A design | materialize byte-identically in `P0-M0`; unchanged in `P0-A1` |

### Slice P1: admitted source prerequisites

Recover only the already designed source contracts required by the two
SourceOnly tasks:

- R-04: the Magic Eastmoney whole-market DragonTiger gateway, exact source
  disclosures/seats, `Available` versus `VerifiedEmpty`, provider provenance
  and acquisition audit.
- R-09: the admitted Eastmoney provider Top-N pair, canonical request evidence
  and complete two-batch projection.
- The smallest shared Gateway evidence/error/audit seam required by those two
  contracts.
- The BR-159 `data_acquisition_audit` database schema/repository method used by
  that shared seam; an audit write failure remains a source failure.
- Restore the existing project-wide approved fourteen-direct/fifteen-lock
  Magic release closure, all pinned to the authoritative repository release
  `5f1ce93656a55854c844065390520cd4aecd9a14` at version `=0.2.0`. The older
  `d7dfa3140919525f3280bed87136602a78fa17ad` revision recorded in the recovered stash is evidence of the
  historical implementation only and must not re-enter `Cargo.toml` or
  `Cargo.lock`.

Only `magic-eastmoney-rs`, `magic-market-composition`, `magic-market-core` and
`magic-market-router` are imported by the recovered R-04/R-09 P1 modules. The
other ten direct packages remain root dependencies for existing independently
owned project consumers; their presence is release-closure preservation, not
authorization for P1 to import more capabilities. In particular,
`provider_top_n.rs` must not import `magic-exchange-rs`.

This is an atomic prerequisite amendment, not accepted fixed-HEAD fact.
Reproducible inspection shows both `PREVIOUS_PARENT` and `BASELINE` still have
`rusqlite` with only `chrono` and a sibling path dependency for
`magic-tdx-rs`; neither has the fourteen direct
`5f1ce93656a55854c844065390520cd4aecd9a14` pins. The current
worktree versions of the BR-192 row, `Cargo.toml`, `Cargo.lock` and revision
test are explicitly rejected because their bytes are broader than that
closure. P1 must derive a minimal amendment from `BASELINE`: preserve every
unrelated dependency, Polars `0.46` feature, target-specific dependency,
dev-dependency and profile byte; change only `rusqlite` to retain `chrono` and
add `functions`, replace the one sibling `magic-tdx-rs` path with the exact
fourteen direct Magic dependencies, and regenerate the lockfile from that
minimal manifest. All fourteen direct and fifteen lock packages must resolve
to version `0.2.0` and revision
`5f1ce93656a55854c844065390520cd4aecd9a14`. P1 also creates a narrow
revision test that reads only `Cargo.toml` and `Cargo.lock`, proves that exact
closure, rejects any sibling path and rejects both historical revisions
`d7dfa3140919525f3280bed87136602a78fa17ad` and
`660902ff93a07f18367dc16879cf67732accd25a`. A partial dependency transition or retention
of a rejected revision fails. The older
`2026-07-29-provider-topn-ranking-gateway-design.md` and stash bytes at
`d7dfa3140919525f3280bed87136602a78fa17ad` remain historical recovery evidence only.

The fourteen direct package names are closed and ordered lexicographically:
`magic-baidu-rs`, `magic-cls-rs`, `magic-cninfo-rs`, `magic-eastmoney-rs`,
`magic-exchange-rs`, `magic-jin10-rs`, `magic-market-composition`,
`magic-market-core`, `magic-market-router`, `magic-sina-rs`, `magic-tdx-rs`,
`magic-tencent-rs`, `magic-thepaper-rs`, and `magic-ths-rs`. The lockfile set is
exactly those fourteen plus the transitive `magic-market-transport`. Every one
must have version `0.2.0`, repository
`https://github.com/Northofqing/magic-market-data-rs.git`, and source revision
`5f1ce93656a55854c844065390520cd4aecd9a14`; no other `magic-*` package or
source identity is admitted.

“Resolver-required deltas” is not an open allowance. P1 must generate the lock
once from the fixed-baseline manifest in an isolated worktree. After applying
only P1-A2 and proving `cargo --version` is the recorded Cargo 1.95.0 toolchain,
the sole accepted resolver invocation, run exactly once, is:

```bash
cargo update -p url@2.5.8 -p rustls@0.23.37 -p time@0.3.47
```

`cargo generate-lockfile`, bare `cargo update`, a second resolver invocation or
online metadata is not target authority. P1 must record the exact
target `Cargo.toml` and `Cargo.lock` SHA-256 values and the complete changed
package-record whitelist in the implementation ledger before staging, then
reject any byte or package-record drift. A BR-203 checker must bind the two
fixed-baseline input hashes, the two generated target hashes, the exact 14/15
Magic closure, root dependency preservation, unchanged Polars `0.46`, absence
of sibling paths and rejected revisions, and successful
`cargo metadata --locked --offline`. No unlisted non-Magic record is allowed.
The isolated Cargo 1.95.0 generation is now frozen at target SHA-256
`11c3b3914089c29e0b10f0bdbc9be1e55ae65a2d77f6ae251624860ad052c877`
for `Cargo.toml` and
`cb2460bc9872143891efdf5c2df8e17318c6cae5210d3c1861e68416626c1935`
for `Cargo.lock`; `cargo metadata --locked --offline --format-version 1`
exited zero after cache prewarming. The exact 34 added, eight removed and seven
same-identity changed lock records are frozen in the hunk manifest. Cache
prewarming is environmental setup only and is not accepted release evidence;
the checker itself remains strictly offline.

This slice is owned by the existing BR-159/BR-162/BR-190 source designs. It may
not introduce BR-194 task orchestration.

The Gate-B path closure is fixed at module/hunk granularity:

| path | ownership in P1 |
| --- | --- |
| `Cargo.toml`, `Cargo.lock` | P1 minimal baseline-derived BR-203 implementation of the named fourteen direct/15 lock Magic identities at the accepted revision, removal of the sibling path identity, adjacent path-ownership comment correction, plus `rusqlite` `chrono,functions`; target hashes and the exact changed-record whitelist are frozen; no unrelated dependency, Polars or profile change |
| `tests/magic_market_release_revision.rs` | new narrow executable proof that reads only Cargo manifest/lock and asserts the 14-direct/15-lock identities, no sibling path and neither rejected revision |
| `tools/compliance/lib/check_br203_magic_dependencies.sh`, `tools/compliance/check.sh` | fail-closed BR-203 checker and its one compliance-runner registration; verifies fixed input/target hashes, exact package-record whitelist and `cargo metadata --locked --offline`, without network access or business-data calls |
| `src/data_gateway/admission.rs` | new internal deep module containing shared batch evidence, typed outcome/error, canonical request hash and audited outcome helpers extracted from historical `review.rs`; two reviewed adaptations replace its hard-coded `review` capability/failure source with explicit caller capability/source |
| `src/data_gateway/provider_top_n.rs` | narrow deep module extracted from historical `capital.rs`; its sole external interface is `ProviderTopNDataGateway::pair(date)` and it excludes fund-flow/HKEX code |
| `src/data_gateway/dragon_tiger.rs` | adopt the existing `DragonTigerGateway::market_review` interface and exact BR-162 aggregation |
| `src/data_gateway/mod.rs` | register/re-export only the P1 modules in the clean recovery branch; later unified Gateway expansion remains separately owned |
| `src/database/data_acquisition_audit.rs` | adopt the BR-159 immutable hash-chain implementation |
| `src/database/mod.rs` | only BR-159 module registration and schema initialization; `record_data_acquisition` is owned by `src/database/data_acquisition_audit.rs` |
| `src/lib.rs` | only the `data_gateway` registration required by this slice |

`admission.rs` is an internal seam, not a caller-injectable provider port. Real
providers remain constructed inside the admitted Gateway implementations;
private fixture seams test normalization without allowing production callers to
forge provenance. The external interfaces remain the typed R-04/R-09 Gateway
methods and `GatewayBatch<T>` outcomes. `provider_top_n.rs` hides the zero-
argument concrete Magic composition Router, atomic two-batch validation and
audit; it must not expose provider injection or import unrelated fund-flow,
northbound or `magic-exchange-rs` contracts.

The two `admission.rs` adaptations are mandatory audit-correctness fixes. The
historical implementation attributed every batch and failed append to the
`review` capability even when called by DragonTiger or Provider Top-N. The
target API therefore requires the admitted Gateway to supply its fixed calling
capability/failure source; this value is not caller-selected at the production
orchestration boundary. Tests must prove provider mismatch, repository
unavailability and append failure remain fail closed, and that missing batch
evidence retains the real calling capability rather than fabricating `review`.

The Rule-2.10 amendment is deliberately additive. The BR-192 row in the frozen active-ledger
preimage already owns provider Top-N plus retry/schema safety and remains byte-identical;
this recovery must not delete, narrow or restate it. New BR-203 registers only
the incomplete-commit recovery boundary: the exact upstream revision/package
identity, the new `admission`/`provider_top_n`/`dragon_tiger` realization of
existing BR-159/BR-162/BR-192 semantics, the P0→P2→P1→P3→P4 order, rejected old
facades and exact-count verifier. BR-159 retains all existing semantics and its
entire row, including its Code cell, remains byte-identical. BR-203 must not absorb retry-cycle,
provider-capture, R-08, A-10/A-01, schema-v4+ or later fixed-HEAD semantics.

### Slice P2: counted delivery schema v3 and shared-path compatibility

Recover the complete BR-192 counted authority:

- `src/event/envelope.rs`: begin from the complete immutable
  `TRACKED_WIP` file, retain its schema-v3 constant, counted fields,
  constructor, canonical join hash and non-retryable uncertainty contract,
  then insert only the frozen P2-T4 compatibility test at the named test-module
  anchor. This is a controlled whole-file adaptation, not an exact target.
- `src/event/push_record.rs`: begin from the complete immutable
  `TRACKED_WIP` file, retain its exact schema-v3 parsing and join validation,
  add `#[serde(skip_serializing_if = "Option::is_none")]` to exactly the eight
  counted-only optional output fields `decision_identity_hash`,
  `attempt_identity_hash`, `artifact_sha256`, `sink_result_sha256`,
  `receipt_sha256`, `counted_join_hash`, `durable_push_kind` and
  `stable_template_id`, then insert the frozen P2-T5 compatibility test at the
  named test-module anchor. This mandatory compatibility adaptation restores
  the fixed schema-v2 582-byte output instead of serializing eight new `null`
  members; it does not weaken schema-v3 parsing. This is a controlled
  whole-file adaptation, not an exact target. The immutable eight-field
  preimage SHA-256 is
  `4bea98c2ed8934fb2cae65b974ead87f3ea2a94aada7011527a3dde1b79230f1`;
  the exact sixteen-line attribute-plus-field target hunk is
  `9c716e0ac8bbd18cebb314fe7c2225d642cb4530504fba24380b49b29a5365a5`.
- `src/event/mod.rs`: counted audit persistence/publication plus production and
  test capability binding.
- `src/event/durable_delivery_append.rs`: exact-byte immutable append owner.
- the complete bounded notification authority closure: pinned push-log writer,
  eager binding, generic counted entry, authoritative blocking adapter,
  pending/audit/commit ordering and exact terminal verification. It reuses the
  fixed baseline's existing send-type/transport/target/bin/home/receipt-parser
  helpers; the recovered WIP's unrelated token, daemon and transport rewrites
  are explicitly excluded.
- the five exact `ReviewProviderTopN` monitor-enum/exhaustive-match closure hunks
  required to compile the fixed baseline, whose already committed durable/L5
  mappings reference that missing variant; this is type/catalog closure only
  and creates no R-09 producer;
- both legacy generic-path counted-kind rejection guards, both real
  namespace-aware `save_push_log` call sites, their test fixtures, and the
  startup `eager_bind_runtime_artifacts()` call before sink initialization;
- the exact `mod durable_delivery_runtime;` declaration already required by
  that eager-bind call. The existing runtime module file is unchanged from
  `BASELINE` blob `a635b90237413577a51d5bc92ae29c40ae2afac4`.

P2 changes existing production-reachable `event::publish_delivery` and generic
push-log paths. It must therefore retain schema-v2 parsing/publication bytes,
legacy capability registry membership, existing audit-root semantics and every
non-counted caller result. Golden before/after tests must prove identical v2
records, identical non-counted push-log artifacts and zero new sink calls; new
tests separately prove schema-v3 counted joins and namespace isolation. A
compatibility failure returns to Gate B and blocks P1. Forward rollback retains
the v3 decoder/append authority and the namespace-aware writer while disabling
only new producers; it never restores a binary that cannot read already-written
v3 or namespaced artifacts.

The four compatibility tests are not optional implications of the counted
suite. P2 adds
`br192_schema_v2_golden_publication_bytes_are_unchanged`,
`br192_schema_v2_golden_parser_output_bytes_are_unchanged`,
`br192_non_counted_dry_run_golden_push_log_has_exact_bytes_and_zero_sink_calls`
and
`br192_non_counted_live_golden_push_log_has_exact_bytes_and_one_existing_sink_call`.
Their fixed inputs, expected bytes and SHA-256 values are exported from the
fixed-baseline behavior into the hunk manifest before this Gate-A packet is
reviewed. The “live” compatibility test uses only an internal injected spy and
must never resolve or call an external sink.

The compatibility fixture is no longer pending. Its canonical schema-v2 input
is 254 bytes with SHA-256
`95426b1e6fc5a66cdfd2df9340536db5acc630666f6cc872b0306e4fb29b2802`;
the publication output is 689 bytes with SHA-256
`8de344f9fa9b80cbd114474f9299190a7e53a2d57553a4f649d2f4ef9f36bd33`;
the parser output is 582 bytes with SHA-256
`bd198be71b8cdc2e3f66b93ac3bc515f6bff453412639176dcb449c1beab6680`;
and the non-counted artifact is 73 bytes with SHA-256
`41d03c80490b6c553aba19da0219db7ad3b69527f2bb24f80dfe9a52e496fb6d`.
The companion manifest freezes the literal bytes and newline contract.

P2-T7 uses a closed `#[cfg(test)]` sink-spy seam at the existing `push_wechat`
sink boundary. A private shared implementation performs the same namespace
binding and exact one-time artifact write before delivery; the production
wrapper can select only the existing production sink mode. Only test builds
compile the spy mode/helper. The spy increments one atomic call counter and
returns a fixed test result immediately at that boundary. It has no env/global
switch and cannot resolve a token, daemon, transport or external sink. T6
asserts one exact artifact and zero sink calls; T7 asserts the same artifact
and exactly one spy call, so neither test performs or adds a real sink call.

P1 and P2 expose no reachable R-04 or R-09 production producer. In particular,
R-04 must never temporarily enter the generic counted interface because that
path can read CombinedAccount/banner state. P3 atomically installs the R-04 and
R-09 production acquisition/renderer/binding code together with the dedicated
BR-194 SourceOnly gate and its tests. Thus every intermediate commit either has
no production caller or has the frozen SourceOnly caller; no unsafe generic
transition exists.

P2 may change only `src/event/{envelope,mod,push_record}.rs`, add
`src/event/durable_delivery_append.rs`, add the exact P2-owned enum closure,
header, secure-writer, generic rejection, namespace-call-site, counted-entry,
finalization, test-helper and test ranges listed in the hunk manifest to
`src/bin/monitor/notify.rs`, and add the exact eager-bind hunk in
`src/bin/monitor/main.rs`, including its exact durable-runtime module
declaration. The final
`tests/durable_delivery_counted_cutover.rs` test belongs to P3 because it also
requires the producer/catalog cutover. `src/event/dispatcher.rs` is
test/reference-only for this slice because its schema-v3 verifier already
exists in `BASELINE`.

The already committed dispatcher schema-v3 verifier is retained. It must not be
changed to accept schema v2.

### Slice P3: atomic R-04/R-09 producer cutover and BR-194 gate

Recover the source producers and frozen BR-194 orchestration atomically:

- after core database installation and before any startup provider,
  scheduler, producer or sink activation, run
  `durable_delivery_runtime::ensure_startup_reconciled().await` to a durable
  fixed point. Failure exits closed with zero provider and zero sink calls;
  this row belongs to P3 because P3 is the slice that activates producers;

- add the BR-162 R-04 renderer, canonical evidence/binding preparation and
  production Gateway outcome mapping, with its only delivery route being the
  dedicated R-04 SourceOnly entry;
- add the BR-192 R-09 renderer, immutable request/batch/projection envelope,
  production `ProviderTopNDataGateway::pair(date)` outcome mapping and existing
  counted durable entry;

- remove caller-wide AccountMode/banner gates from the three review callers;
- run static preflight before dependency acquisition;
- allow only R-04 and R-09 in the SourceOnly phase;
- construct fixed typed conservative failures for R-03/R-08/A-10/A-01;
- keep R-02/R-05/R-06 disabled;
- pass every outcome through the one canonical stable merge;
- route R-04 through its allowlisted Launch → L5 → counted durable owner;
- keep R-09 on its canonical envelope → durable owner;
- restore the exact BR-194 unit and process-isolation tests.

P3 may change only the enumerated BR-162/BR-192 producer hunks and BR-194-owned
hunks in `src/bin/monitor/{main,notify,push_templates}.rs`, plus the exact
counted-catalog test increment, shared SourceOnly fixture and three SourceOnly
notify test hunks, the final counted-cutover integration test, the BR-194
process tests in `tests/monitor_help_isolation.rs`, the exact startup
fixed-point barrier in `main.rs`, and the separately
manifested scheduler-test import adaptation. The monitor enum/match closure is
already restored in P2 solely to make the fixed baseline compile. Any required change
outside those paths is a Gate-A scope failure, not permission to import another
stash file.

Existing committed `review_batch.rs`, `v14_adapter.rs`, terminal replay schema
and lower durable runtime remain adopted unless an exact focused test proves a
contradiction.

P3 also changes the R-09 caller from the historical capital facade to the
narrow `ProviderTopNDataGateway::pair(date)` interface without changing request,
provider, ordering, evidence or rendering semantics.

Two recovered tests require controlled extraction rather than byte copying:
the R-04 source-route assertion must use an exact/syntax-aware call check so
`push_counted_with_binding` cannot false-match the longer
`push_counted_source_only_with_binding` identifier, and the first process test
retains BR-194 provider/sink/account preflight assertions while excluding its
unrelated BR-183 database/selection assertions. Both adaptations are listed in
the hunk manifest and may not change production semantics.

### Slice P4: validation authority only

P4 adds `tools/release/check_br194_recovery_focused.sh`,
`tools/release/check_br194_bounded_startup.sh`, its structured
`verify_br194_bounded_startup.py` authority and their process fixtures. It also
adapts `tools/coverage/check_thresholds.py` and adds
`tests/test_coverage_thresholds.rs`. It changes no production behavior,
provider, database schema, sink, threshold value or business decision. The
focused verifier has a closed per-slice manifest, runs argv vectors without
`eval`/`bash -c`, proves enumeration and execution counts independently and
fails on zero/duplicate/ignored/count drift.

#### Frozen coverage authority and current red baseline

The checker keeps the fixed-baseline seven-prefix tuple byte-for-byte and in
this order:

```text
src/risk/
src/trading/
src/database/
src/data_provider/
src/decision/
src/pipeline/
src/event/
```

It must not add the broad `src/data_gateway/`, `src/bin/monitor/` or
`src/review/` prefixes. Those prefixes pull unrelated monoliths into the
denominator and do not prove the recovered links. Instead P4 adds a second,
closed `RECOVERY_CORE_FILES` tuple, also order-sensitive:

```text
src/data_gateway/admission.rs
src/data_gateway/dragon_tiger.rs
src/data_gateway/provider_top_n.rs
src/bin/monitor/durable_delivery_runtime.rs
src/bin/monitor/main.rs
src/bin/monitor/notify.rs
src/bin/monitor/push_templates.rs
src/bin/monitor/review_batch.rs
src/bin/monitor/v14_adapter.rs
```

Gate D requires all three independent conditions: global line coverage >=80%,
the seven-prefix aggregate >=95%, and the exact recovery-file aggregate >=95%.
Every recovery file must occur exactly once with a non-zero line denominator;
a missing/duplicate path is an invalid report (exit 2), not a smaller
denominator. CLI overrides may raise a floor but may not lower 80/95. The
focused hunk/test ledger remains the separate proof that every recovered
monolith hunk is behaviorally exercised; the fixed file tuple prevents adding
unrelated high-coverage files to dilute it.

The read-only diagnostic report that existed before P1-P3 has SHA-256
`56a27082674e3630214be83f31f6a3b14e79de748b40731e22e9cedd75c1978f`.
It is not Gate-D evidence, but its exact arithmetic proves that the old plan
was not viable:

| diagnostic set | files | covered / total | percent | lines needed at frozen floor if denominator stayed fixed |
| --- | ---: | ---: | ---: | ---: |
| global | 373 | 149200 / 189647 | 78.6725% | 2518 for 80% |
| fixed seven prefixes | 100 | 48391 / 57740 | 83.8085% | 6462 for 95% |
| proposed ten broad prefixes | 162 | 93609 / 120189 | 77.8848% | 20571 for 95% |
| seven recovery files present in that old report | 7 | 22710 / 31124 | 72.9662% | 6858 for 95%; `admission.rs` and `provider_top_n.rs` were absent |

The fixed checker input at `BASELINE` has SHA-256
`0b5c0b4a745e8209b7c5b4a8cc258fe7b4a15c68db9ad9f3b26f6e0d214d301a`.
These values are reproduced from `target/coverage/coverage.json` by summing
llvm-cov `summary.lines.{covered,count}` after repository-relative path
normalization; percentages are `covered * 100 / count`, and required extra
lines are `ceil(floor * count) - covered`.

P4 must therefore generate a fresh report from the final P1-P3 source SHA
before claiming a coverage plan. It emits an uncovered-line debt ledger for
the frozen sets and closes that ledger with failure-path/branch tests only.
It may not use `#[coverage(off)]`, source exclusions, generated-report edits,
unreachable-code deletion, denominator padding or weaker thresholds. If the
fresh debt requires test hunks outside the already manifested P1-P3 test
anchors, work stops at Gate A and adds exact test destinations/hashes in an
amendment before editing them. The stale arithmetic above means this recovery
is currently Gate-D red; a green unit suite alone cannot change that status.

#### Bounded startup state machine

The bounded verifier accepts exactly two arguments:
`./target/release/monitor` and `30`. It rejects any other arity, path spelling,
symlink-resolved target or timeout. A monotonic 30-second budget covers child
spawn, both readiness banners, parent SIGTERM, graceful banner and process
reap. Before spawn it validates the authority snapshots below. It launches the
normal binary with no application arguments, rejects inherited test/review
overrides, captures stdout/stderr, and requires exactly one each of
`🚀 Stock Monitor 启动`, `[复盘调度][BR-139] started` and the normal-mode marker
`模式: 正常`. Once both readiness banners exist, the parent records that fact,
sends SIGTERM exactly once and requires exactly one `监控已安全关闭` and exit 0
before the same deadline. Exit before the recorded parent signal is failure
even when the status is zero.

The launch environment must reject, rather than silently unset, non-empty
`STOCK_ENV_MODE=test`, `V10_DRY_RUN_PUSH`, `DURABLE_DELIVERY_TEST_CODE`,
`EVENT_AUDIT_DIR` or `PUSH_LOG_DIR`. Output containing `模式: 复盘`,
`isolated_test=true`, `TEST_CODE` or a BR-196 test-delivery marker is an
unexpected-mode failure.

| observation | verifier result |
| --- | --- |
| release path is not the literal path, resolves through a symlink, or timeout is not literal `30` | fail before child spawn |
| startup banner absent by deadline | fail; terminate/reap child |
| scheduler banner absent by deadline | fail; terminate/reap child |
| either readiness banner occurs more than once | fail; terminate/reap child |
| child exits before parent SIGTERM, including exit 0 | fail as early exit |
| graceful banner absent or duplicated after SIGTERM | fail |
| child exits non-zero or cannot be reaped within the common deadline | fail |
| any unexpected test/review mode marker or override is present | fail before/while running |
| a pre-existing audit chain is invalid, an old authority member changes/disappears, or count/hash disagree | fail before/after launch |
| authority growth is zero and all pre/post watermarks are equal | pass the side-effect check |
| authority growth is non-zero and every new member forms the exact join below | pass the side-effect check |
| any new sink result, counted delivery audit, push artifact or immutable audit record is unjoined/ambiguous | fail |

#### Watermark files, fields and exact join

The Python authority opens only these fixed production locations through
no-follow regular-file/directory checks; environment path overrides are
forbidden:

1. `data/durable_delivery.sqlite3`: in one read transaction, order every
   `sink_results` row by `result_event_identity` and hash a typed canonical
   JSON array containing all 21 columns in schema order:
   `result_event_identity`, `attempt_identity`, `decision_identity`,
   `result_kind`, `observed_at`, `fence_token`, `authoritative_for_state`,
   `late_after_fence`, `authority_audit_identity`,
   `late_receipt_audit_identity`, `result_canonical`, `result_sha256`,
   `channel`, `provider`, `message_id`, `platform_message_id`, `accepted_at`,
   `latency_ms`, `frozen_delivery_audit_canonical`,
   `frozen_delivery_audit_sha256`, `delivery_audit_ref`. BLOBs are tagged
   lowercase hex and NULL remains a typed NULL, so concatenation cannot be
   ambiguous. The watermark is `(row_count, sha256(array_bytes))`.
2. `data/event_audit/<YYYY>.jsonl`: sort filenames and line ordinals, validate
   the complete file with the existing dispatcher's committed legacy/v2
   `previous_hash`/`record_hash` rules (an historical legacy record need not
   invent a `hash_domain`), then hash the canonical array
   `(relative_path, line_ordinal, record_hash)`. The watermark is
   `(record_count, sha256(array_bytes))`.
3. `data/push_log/**`: sort regular files by repository-relative path, reject
   symlinks/non-regular/multi-link replacements, and hash the canonical array
   `(relative_path, byte_length, sha256(file_bytes))`. The watermark is
   `(file_count, sha256(array_bytes))`.
4. `data/durable_delivery_audit/durable_delivery_v1.jsonl`: validate every
   `hash_domain`, `previous_hash`, `record_hash`, `canonical_hex` and
   `canonical_sha256`, then hash the canonical array
   `(line_ordinal, record_kind, identity, canonical_sha256, record_hash)`.
   The watermark is `(record_count, sha256(array_bytes))`.

The post snapshot must contain every pre member unchanged. New members are set
differences by primary identity/path/hash, never by mtime or tail offset. For
each new `sink_results` row the verifier requires exactly one attempt and
decision identity, one authoritative immutable audit identity, exactly one
`AuditPending` and one `Committed`
`stock_analysis.counted_push_log.v1` artifact, and exactly one schema-v3 event
whose `id == counted_join_hash == committed.counted_join_hash`. It recomputes
and equates the pending/committed `decision_identity_hash`,
`attempt_identity_hash`, `pending_artifact_sha256`, `sink_result_sha256` and
`receipt_sha256`; it also requires event payload hashes to equal those values,
`delivery_audit_ref` to identify that event, and
`sha256(frozen_delivery_audit_canonical) == frozen_delivery_audit_sha256`.
Every new immutable record must decode from `canonical_hex`, match its
`canonical_sha256`, and join through its `identity` plus decoded
decision/attempt identity to exactly one new sink row. The reverse relation is
also total: every new counted event/artifact/immutable member must be consumed
once by one new sink row. Missing, duplicate, cross-decision or orphan growth
is failure. Fixture tests create an isolated temporary root containing a fake
file at the literal relative path `./target/release/monitor`; they never invoke
a real provider or sink and exercise both zero-growth and exactly-joined-growth
positive cases.

## 5. Explicit exclusions from the recovered blobs

The recovery extractor must reject any hunk or file content containing these
later contracts unless a separate accepted design owns it:

- `SOURCE_BATCH_DELIVERY_AUDIT_SCHEMA_VERSION`, `new_source_batch`, or BR-160
  source-batch persistence;
- BR-197 DataMode relaxation;
- BR-198 A-10/A-01 SourceOnly promotion;
- BR-199 R-08 SourceOnly entry;
- BR-200 settled-terminal provider preflight;
- BR-196 presentation or transport policy changes.

Marker searches are necessary but insufficient. The tracked
`2026-08-03-br192-br194-recovery-hunk-manifest.md` is a two-phase authority.
At Gate A, every historical input row must contain its immutable source Git
object, source path, inclusive interval and raw-byte SHA-256, plus a separate
splice ledger mapping that row to a destination symbol/anchor, owning rule and
exact non-zero focused tests. Raw-byte hashing is intentional; it is stronger
than a whitespace-normalized identity and the manifest must not call it a
normalized target hash. Before each Gate-B slice is staged, exact-copy rows
must reproduce the same destination bytes and extract/adapt/new rows must add a
candidate target-hunk SHA-256 and explicit destination line interval to the
implementation ledger. Extraction accepts only those rows, verifies source
and target hashes, and rejects any unclassified candidate hunk.

After each slice, a reviewer must inspect the complete staged diff with at
least 80 lines of context and trace every multiline call from the production
entry back through Gateway, renderer, counted/SourceOnly gate and durable
owner. Syntax-aware checks must reject the later APIs/semantics themselves,
including source-batch constructors/schema, R-08/A-10/A-01 SourceOnly entries,
R-04 global DataMode relaxation and settled-terminal preflight, whether or not
the hunk contains a `BR-160`/`BR-197`/`BR-198`/`BR-199`/`BR-200` literal. The
manifest SHA set, staged path set and traced caller set must all be exact; an
unclassified hunk or caller is a blocking scope failure.

Whole-file rejection is supported by direct recovery-object evidence:

```text
$ rg -n 'BR-160' src/bin/monitor/push_templates.rs src/data_gateway
src/bin/monitor/push_templates.rs:5941:/// BR-160: A-10 only consumes the exact visible batch published by the
src/bin/monitor/push_templates.rs:14550:        // 建议型模板必须保留辅助建议尾注；A-10 只呈现 BR-160
src/data_gateway/chain_intelligence.rs:1://! BR-159/BR-160 deterministic A-10 chain-intelligence derivation.
src/data_gateway/chain_intelligence.rs:685:            "BR-160 chain_intelligence config is unavailable",
```

Therefore neither `push_templates.rs` nor the recovered data-gateway tree is a
valid copy unit even if a later-rule marker search otherwise returns empty.

## 6. Failure modes

| failure | required behavior |
| --- | --- |
| Git object or parent mismatch | stop before extraction; no source mutation |
| prerequisite closure incomplete | compile/test fails; return to its owning slice (P2 first when baseline compile closure is involved) |
| R-04/R-09 evidence incomplete | typed task failure; no cross-source fill |
| BR-159 additive schema install/read-back fails | transaction rollback and explicit startup/source failure; no partially initialized repository |
| counted v3 join incomplete | fail before sink/success; no v2 downgrade |
| immutable audit append unavailable | explicit failure or uncertainty terminal; no automatic duplicate send |
| `--test --review` reaches provider/sink | blocking test failure and Gate B rollback |
| unmanifested hunk/caller or later behavior enters recovery | reject before compile and return to Gate A scope review; marker absence is not proof |
| full check conflicts with focused green | fix root cause; focused green is not release evidence |

## 7. Validation gates

### Focused Gate B/C

```bash
bash tools/release/check_br194_recovery_focused.sh
cargo check --lib
cargo test --test magic_market_release_revision -- --test-threads=1
cargo test --lib database::data_acquisition_audit::tests::br159_ -- --test-threads=1
cargo test --lib data_gateway::admission::tests::br159_ -- --test-threads=1
cargo test --lib data_gateway::dragon_tiger::tests::br162_ -- --test-threads=1
cargo test --lib data_gateway::provider_top_n::tests::br192_ -- --test-threads=1
cargo test --lib event::envelope::tests::br192_ -- --test-threads=1
cargo test --lib event::push_record::tests::br192_ -- --test-threads=1
cargo test --lib event::delivery_observation_tests::br192_ -- --test-threads=1
cargo test --lib event::durable_delivery_append::tests::br192_ -- --test-threads=1
cargo test --bin monitor notify::tests::br192_ -- --test-threads=1
cargo test --bin monitor durable_delivery_runtime::tests::br192_main_eagerly_binds_runtime_artifacts_exactly_once_before_sink_init -- --exact --test-threads=1
cargo test --test durable_delivery_counted_cutover -- --test-threads=1
cargo test --bin monitor br194_ -- --test-threads=1 --nocapture
cargo test --bin monitor tests_post_session_review_scheduler::br140_weekend_manual_review_uses_the_latest_completed_trading_day -- --exact --test-threads=1
cargo test --bin monitor tests_post_session_review_scheduler::br192_ -- --test-threads=1
cargo test --bin monitor br192_provider_top_n_tests:: -- --test-threads=1
cargo test --bin monitor tests_r_dispatchers::br162_r04_ -- --test-threads=1
cargo test --bin monitor tests_r_dispatchers::br192_r04_ -- --test-threads=1
cargo test --bin monitor push_templates::tests::counted_kinds_bypass_process_local_cooldown -- --exact --test-threads=1
cargo test --test monitor_help_isolation br194_ -- --test-threads=1 --nocapture
cargo test --test test_coverage_thresholds br203_ -- --test-threads=1
cargo test --test br203_recovery_verifiers br203_ -- --test-threads=1
bash tools/compliance/lib/check_br203_magic_dependencies.sh
bash tools/compliance/lib/check_br194_review_dependency.sh
```

`check_br194_recovery_focused.sh` is a new fail-closed Gate-B verifier. It runs
the closed argv commands above without `eval`/`bash -c`, first enumerates the
qualified tests, requires the exact declared count for every filter and then
requires the same passed count with zero failed. Its frozen counts are:
revision 1, BR-159 database 4, BR-159 admission 4, DragonTiger 6, Provider
Top-N Gateway 3, envelope BR-192 4 (including one v2 golden), push-record
BR-192 3 (including one v2 golden), delivery observation 3, immutable append 9,
notify BR-192 26 (including two non-counted golden tests), eager-bind 1,
counted cutover 1, monitor BR-194 31,
weekend-date caller 1, scheduler BR-192 3, R-09 renderer/caller 6, R-04 BR-162
3, R-04 BR-192 3, counted catalog 1, process BR-194 3, coverage-verifier 7 and
recovery-verifier 20. The sole permitted ignored test is the exact named child
`notify::tests::br192_push_log_process_writer_helper`; the passing parent must
spawn it with `--ignored`. Any other ignored test, zero match, duplicate name,
count drift or parser ambiguity exits non-zero.

The P1 filters must be backed by an exact manifest containing at minimum all
four BR-159 acquisition-audit tests, four admission fail-closed/attribution
tests (provider mismatch, repository unavailable, audit append failure, and
missing batch evidence retaining the calling capability), all six
BR-162 DragonTiger tests, and three R-09 tests for typed/order preservation,
partial/date/evidence rejection and atomic empty/metric-drift rejection. The
append filter must report exactly nine passing parent tests and execute the
complete BR-192 physical namespace/link/mode matrix, including its isolated
child. The BR-194 aggregate command is accepted only if its output is exactly
31 passing monitor tests: 17 durable-runtime tests, ten review-batch tests, one
v14-adapter test and the three dedicated notify SourceOnly tests. It does not
stand in for the independently counted scheduler, R-09, R-04 or catalog groups
listed above. The process command must report exactly three passing tests; zero
matches or a changed manifest is failure.

### Full Gate C

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
```

### Gate D/release and runtime evidence

Only after the final fixed source commit and clean Gate C:

```bash
git status --porcelain=v1
git rev-parse HEAD
cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
shasum -a 256 target/coverage/coverage.json target/release/monitor
bash tools/release/check_br194_bounded_startup.sh ./target/release/monitor 30
```

The checker must print and pass all three frozen results: global >=80%, the
fixed seven-prefix core >=95%, and the exact nine-file recovery core >=95%.
The first command must print nothing. The recorded final source Git SHA,
coverage-report SHA-256, release-binary SHA-256 and the two frozen set literals
are one Gate-D evidence packet, generated in the shown order without source or
test mutation between commands. A stale/unbound report, a missing recovery
file, a threshold-lowering argument or only two of the three results is
failure. These floors are the repository authorities in
`docs/ENGINEERING_RULES_V2.md`; P4 does not weaken or reinterpret them.
The later BR-202 `tools/coverage/run_isolated_gate.sh` proposal is absent from
the fixed baseline and explicitly outside this recovery, so it cannot be
invented as a prerequisite. The three BR-194 process tests provide the
`--test --review` zero-provider/zero-sink proof. Plain `--test` has a separate
BR-196 non-production delivery contract and is neither an isolation proof nor
part of this recovery's Gate D. A bounded release-binary normal startup must
separately prove initialization and graceful shutdown without treating process
exit as delivery evidence.

The positive canary is a controlled release-validation phase, not Gate-B
implementation. It runs on one authentic A-share trading date under the normal
audited production runtime: R-09 must be ExpectedWait with zero provider/sink
before 15:35 and complete after 15:35; R-04 must be ExpectedWait with zero
provider/sink before 21:00 and complete after 21:00. For each completed task,
capture the fixed release SHA/binary hash and pre-run watermarks, derive the
business date from admitted review authority, and prove one provider-backed
decision/attempt/artifact/sink-result/receipt/push-log/immutable-audit/hydration
chain. No synthetic receipt or mock production record is permitted.

After same-cycle hydration, substitute that exact admitted business date and
run one audited terminal replay and one independent verifier per task:

```bash
DATE=<admitted-review-business-date>
./target/release/monitor --br194-audited-terminal-replay --business-date "$DATE" --task R-09
./target/release/monitor --br194-audited-terminal-replay --business-date "$DATE" --task R-04
python3 tools/release/verify_br194_review_join.py --business-date "$DATE" --task R-09 --require-passed-replay 1
python3 tools/release/verify_br194_review_join.py --business-date "$DATE" --task R-04 --require-passed-replay 1
```

Each replay must report exactly one Passed attempt, zero provider/resume/sink/
delivery-audit append calls and equal pre/post sink-result and delivery-audit
watermarks. The verifier must join the original provider binding, task,
durable occurrence, sink result, push log, delivery audit and hydration to the
replay start/completion immutable audits. A plain `cargo run -- --review`, a
successful process exit or a final receipt without this exact join is a
false-green and cannot satisfy Gate D.

## 8. Old modules

| module | decision | reason |
| --- | --- | --- |
| committed BR-194 `review_batch.rs` and `v14_adapter.rs` | adopt | focused tests prove the frozen lower-layer contract |
| committed schema-v3 dispatcher verifier | adopt | correct counted-delivery join requirement |
| pre-merge R-04/R-09 Gateway code | adopt narrowly | real admitted source prerequisite, independently tested |
| historical `src/data_gateway/review.rs` | reject as installed module | only the manifest-listed evidence/error/audit ranges inform new internal `admission.rs`; P1 updates the BR-159 code pointer and installs no broad review facade |
| `market_analyzer::lhb_review` production acquisition | reject | superseded by BR-162 unified Gateway |
| generic counted gate for R-04 | reject | can read combined banner and violates SourceOnly scope |
| local projections as broker evidence | reject | no verified same-batch broker/trade-sync watermark |
| BR-197/198/199/200 hunks | reject | outside frozen recovery scope |

## 9. Rollback

Before any production execution, the disposable clone or unmerged recovery
branch may be discarded. Once a recovery binary has opened a production
database or reserved a counted delivery, rollback is a forward state
transition, never a blind P3 → P2 → P1 `git revert`:

1. stop the monitor, record the literal release SHA and immutable audit/database
   watermarks, and build a reviewed forward rollback from that exact SHA;
2. disable the BR-194 R-04/R-09 producer installation before any provider,
   renderer, new decision, reservation or sink call while retaining the same
   schema-v3-aware coordinator and readers;
3. run the existing all-date reconciliation authority for every pending audit,
   disposition and task-transition acknowledgement; do not reacquire provider
   data and do not use a legacy resume/send path;
4. preserve `UncertainManualReview` as non-automatic: an authorized operator
   must resolve it from the exact receipt/audit evidence, and uncertainty must
   never trigger an automatic resend;
5. verify from the durable authority that there are zero active reservations,
   zero unresolved pending acknowledgements/transitions and zero ambiguous
   counted joins before removing any runtime owner;
6. deploy only a forward-compatible rollback binary. It retains BR-159 audit
   tables/triggers/records, schema-v3 recognition, parsers, append/verifier
   authority, historical durable rows and immutable files. It must not lower
   `user_version`, remove `sha256_hex`, launch a schema-v2 binary or delete any
   database, WAL/SHM, reservation, attempt, receipt, disposition, push log or
   audit record; and
7. any later removal of now-unused P1/P2 source code requires a new Gate-A
   design after all retained authority has terminalized. The database and
   immutable audit decoder, delivery audit and push log remain readable and
   tamper-evident for at least five years.

Rollback verification must exercise the forward-disable patch against the
fixed release tree, prove that only producer activation changed, and rerun the
schema read-back, all-date reconciliation and counted-join verifier. A failure
returns to Gate A/B/C by root cause; it never authorizes destructive cleanup.

## 10. PR evidence and draft lifecycle

The recovery is developed on an isolated branch and opened as a Draft PR after
Gate A. It remains Draft through Gate B/C and may become Ready only after Gate D
and two independent reviews. No production account identifiers, tokens,
message bodies, receipt values or absolute user paths may be pasted; evidence
uses hashes, counts, redacted TEST_CODE paths and bounded banner excerpts.

The PR description must contain all AGENTS §3.1 fields:

- `Refs:` this design plus the fixed BR-159/162/192/194 specs and exact
  sections;
- `Data-Redlines:` at least `[2.1,2.2,2.3,2.4,2.5,2.7,2.8,2.10]` with concrete
  evidence;
- `OldModules:` the complete table in §8 with adopt/reject reasons;
- `Threshold-Proof:` no business threshold changed; coverage path membership
  changed in P4 while the immutable 80%/95% floors remain unchanged, with
  passing/failing fixtures attached;
- `Business-Rules:` `BR-159,BR-162,BR-192,BR-194,BR-203`;
- `Rollback:` the forward-disable procedure in §9 and the exact release SHA.

The PR also attaches exact focused counts, complete Gate C output, coverage
JSON/checker output, release binary SHA-256, bounded startup result and the
redacted authentic provider→decision→sink→receipt→immutable-audit join. Missing
or stale evidence keeps the PR Draft and status In Progress.

## 11. Gate A acceptance criteria

- independent review reports C0/I0/M0;
- source prerequisite ownership and exact path closure are enumerated;
- every historical input has a fixed source object/path/interval/raw-byte hash
  and splice-ledger destination, owner and exact non-zero test set; before a
  Gate-B slice is staged, its extract/adapt/new target hunks receive exact
  destination intervals and target SHA-256 identities; unclassified hunks and
  callers are zero;
- P0's additive BR-203 row is independently accepted before any P2 source byte
  is staged; the active ledger's pre-existing rows remain byte-identical and
  the historical baseline ledger remains extraction evidence only;
  Cargo/lock/revision-test then transition atomically from the fixed baseline,
  and neither fixed HEAD nor a dirty worktree is misrepresented as prior
  authority;
- the path closure includes BR-159 acquisition-audit persistence and proves
  that no unrelated A-01/R-03/THS/historical-bars module is required merely to
  host shared R-04/R-09 evidence types;
- no later BR behavior is included;
- P2 controlled whole-file targets freeze new whole-file identities after the
  only admitted T4/T5 and eight-field omission adaptations; P2 shared-path
  compatibility and P4 exact-count/coverage/startup authorities
  are complete and tested, not inferred from a successful build;
- P4 preserves the seven baseline prefixes, freezes the nine recovery files,
  proves all independent 80/95 gates, and has no unclassified coverage debt;
- bounded startup exercises every row in its failure matrix and proves equal
  watermarks or the exact total counted join for all four authorities;
- rollback and failure paths are executable;
- additive BR-159 schema installation is named as a migration and every
  post-release rollback preserves its objects, records and reader;
- counted rollback freezes new work, converges all-date pending authority,
  manually disposes uncertainty and proves zero active/pending state before a
  runtime owner is removed;
- `docs/business_rules.md` rows remain the canonical semantics and require no
  contradictory threshold/config change.
- `P0-M0` and its direct child `P0-A1` are materialized in real Git trees and commit objects;
  `P0-M0` binds the frozen preimage plus all three Gate-A documents, while `P0-A1` changes only the
  BR-203 row. Untracked working-tree hashes or non-written `hash-object` output do not identify the
  review packet. Both commit trees receive fresh independent exact-byte review before Gate B.

Until all criteria pass, status remains **Gate A draft / implementation
prohibited**.
