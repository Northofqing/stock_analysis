# BR-192/BR-194 incomplete-commit recovery hunk manifest

**Status:** Gate A P0-A3 repair; Gate B prohibited pending fresh C0/I0/M0

This manifest binds recovery work to immutable Git objects. Source ranges are
one-based inclusive ranges in the named object and every listed SHA-256 is over
the exact raw range bytes emitted by `git show`; it is not a normalized target
hunk hash. A destination is never allowed to copy bytes outside a listed
range. `exact` means byte-for-byte admission; `extract` means the source is
evidence for a smaller reviewed deep module and must not be represented as a
byte recovery; `new` means candidate target work with no immutable source
equivalent. Before a Gate-B slice is staged, every partial-file row, including
`exact`, must gain a literal destination interval and target-hunk SHA-256 in
the implementation ledger. Exact whole-file rows use the whole destination
file and must match the listed source hash. A `controlled whole-file
adaptation` starts from the complete immutable source file, permits only its
named splice, and must freeze a new whole-target Git blob and SHA-256 before
staging; its historical whole-file hash remains preimage evidence and is never
claimed as the target identity.

## Immutable authorities

| Purpose | Object |
| --- | --- |
| `BASELINE` — fixed first parent | `9307b6785420c32b57fe210f9c9b870d83e4a52d` |
| `TRACKED_WIP` — tracked WIP stash commit | `2a4d1b929507fadadb082c2a803d5fea50cf6dd8` |
| `UNCHANGED_INDEX` — unchanged index snapshot | `b2981c4cc84a0d277bf07f51346ac6da84cbcb71` |
| `UNTRACKED_WIP` — untracked-file stash commit | `1389098b395a8894578259463923d58ab580a8b6` |
| `PREVIOUS_PARENT` — prior comparison parent | `b4aeee68d2c0259cc968914b3d39e3a89a18a496` |

Aliases are presentation shorthand for these complete immutable object IDs.
No short SHA or ellipsis is an extraction authority.

The tracked WIP object's first parent is the fixed first parent above. The
candidate ranges can be reproduced with:

```bash
git show <object>:<path> | sed -n '<start>,<end>p' | shasum -a 256
```

## Rejected source/dependency packets and candidate P2-F exception

The former source candidate blobs `ec52754ace19f5e09341416abd37c4876963943e`
and `e79ac5a5d159e8cb534fd9778c5043dd65935f50` remain rejected evidence because
they import later rules and unowned paths. The former whole dependency blobs
`17e4ff819323d1126f434875d4098681578243c8` and
`24b2e7d0e4d912404213ad23a1abdf62792b5ad3` are also rejected as a P2/P1
predecessor: an isolated compile against old source produced fourteen removed
legacy dependency imports and one Polars 0.54 API mismatch. They remain the
BR-164 final release goal only. P2-F instead admits the exact minimal Cargo,
lock and two strict selection-caller targets frozen below. No rejected blob
grants source authority.

## P0 documentation-only Rule-2.10 registration

P0 runs before every source slice. `P0-M0` first materializes the immutable active-rule-ledger
preimage plus the recovery design, this manifest and the preserved BR-204 Gate-A design in one
docs-only commit tree. `P0-A1` is its direct child and changes only the rule ledger by adding BR-203.
After independent review found the original slice order was not compile-closed,
`P0-A2` amends only the BR-203 row and these two Gate-A documents to bind the
then-understood atomic P1-A/P2 compile closure, the P2-owned L0/M2/T6/T7 compatibility closure
and the pure R-04 validator seam. It is the direct child of `P0-A1` and still
precedes every dependency/source byte. The first proposed P0-A3 whole-Cargo
target was rejected before commit after its isolated compile failed; it has no
place in the Gate chain. The candidate `P0-A3` must be the direct child of `P0-A2`;
it changes only the same three Gate-A documents, records the clean-lineage experiments, binds the minimal P2-F
dependency and strict selection-audit caller targets, and defers final
0.54.4/no-qmt convergence to BR-164 data-domain cutovers. It also precedes
every new dependency/source stage.
The historical
`BASELINE:docs/business_rules.md` Git blob
`a5325bdfb381ed187f1acbf70819260f38e18646` (SHA-256
`2c1d3634b38649ecb804a525bc896db0c9989eab9903dd54fc3ba1e7b0a312b9`) remains extraction evidence
only and must not overwrite later rule-ledger additions or amendments.

| ID | Class | Fixed preimage | Destination/anchor | Owner | Exact acceptance |
| --- | --- | --- | --- | --- | --- |
| P0-M0 | docs-only authority materialization | current implementation parent plus exact frozen active-ledger preimage | materialize active ledger without BR-203, recovery design, this manifest and preserved BR-204 Gate-A design | BR-203 | no source/config/runtime path changes; the ledger equals its frozen preimage hash and all three document blobs are committed |
| P0-A1 | new additive docs-only rule row | exact committed `P0-M0` tree | insert canonical BR-203 row; its Code cell names only this recovery design and companion manifest | BR-203 | `P0-A1` is a direct child of `P0-M0`; every pre-existing active-ledger row and every non-ledger path remains byte-identical; pre/post hashes prove BR-203 is the only child-commit semantic addition; historical baseline is not restored |
| P0-A2 | docs-only compile-closure correction | exact committed `P0-A1` tree | amend only BR-203 ordering text plus this design/manifest | BR-203 | direct child of `P0-A1`; no source/config/runtime/dependency/lock/test path changes; records the then-proposed `P0 → (P1-A+P2) → P1-source → P3 → P4` closure, P2-L0/M2/R4V1/T6/T7 and invalidated partial P2 test claim; superseded by direct-child P0-A3 and not current Gate-B authority |
| P0-A3 | docs-only executable-lineage correction | exact committed `P0-A2` tree | amend only BR-203 P2-F/BR-164 ownership plus this design/manifest evidence | BR-203 | direct child of `P0-A2`; no source/config/runtime/dependency/lock/test path changes; records the rejected uncommitted whole-Cargo proposal, binds minimal P2-F and keeps 0.54.4/no-qmt as final BR-164 release target |

P0 changes no source, configuration, dependency, lockfile or test. The frozen
active-ledger preimage without BR-203 has SHA-256
`9e149cc950c40976fe10a8a4f0f43d8e70a984b5c83595123e4f52e813a52c96`; the P0-A1 literal BR-203 row has
SHA-256 `d89a3ecbad17a99b60201c71515c46c88235c7cde168db6c9e0eaaa8fc9a2ce5`; its complete target has
Git blob `138a6725eb53420f3dfad3ba3ed086618be9873b` and SHA-256
`bf0955f05dd792fc7088a29a7197910d7b9cecc6d65cbc1e33f1932876355666`. The P0-A2 literal BR-203 row
has SHA-256 `d71e37bce3ca1bbc89e2158273307fd8d563475eb46d323caa19865ffbb6456b`;
its complete ledger target has Git blob
`d2a9208b19c9fb0ac87e29a30c2af0b8f780ae5a` and SHA-256
`a8c09e0111623d0bbf4d5065359960c907047ffbfe3b14a0c373d2eaae7b0431`.
The P0-A3 literal BR-203 row has SHA-256
`ad48af414b240d1b0e7be7717eda66d4e0223308e56fe71c0836af30e6b21634`;
its complete ledger target has Git blob
`9ed01130d06ff700146b8f9e0e4b6f37e6dd5c6e` and SHA-256
`e56fe49fe66a30ede0708c95cb24f543d5e775d79e47113bc889b1d40518358a`.
Any change outside the single BR-203 row from the frozen preimage to P0-A1 is
a hard failure; any P0-A1-to-P0-A2 change outside BR-203 and the two recovery
documents is also a hard failure; any P0-A2-to-P0-A3 change
outside the same three paths or beyond the named executable-lineage/dependency-ownership
correction is a hard failure.

The P2-F dependency/caller candidate is the experimentally validated minimal
transition below:

| ID | Class | Fixed preimage | Destination/anchor | Owner | Exact acceptance |
| --- | --- | --- | --- | --- | --- |
| P2-C1 | controlled minimal manifest target | `BASELINE:Cargo.toml` blob `2118a3e490efe2d3416b2554559ca0347947c533` / SHA-256 `521c3b24795288ddce453e714a74e23fe96afe348dfa49c5d68681f0fdf2adfa` | whole `Cargo.toml` blob `f194a746da45ecec93cc809d30bfa12be6546ad2` / SHA-256 `6f2065fa487b3175bcb09c3baafdd4ef5d990a737fcd77abd8758c672190b45e` | BR-203/P2-F | only `rusqlite=chrono,functions` and direct same-sibling-path `magic-market-core =0.2.0`; every other byte remains baseline-identical |
| P2-C2 | controlled root-lock target | `BASELINE:Cargo.lock` blob `95481362e8061a1724cd1682d23b4e8a14f16377` / SHA-256 `cd86df085943a710c17ec2cb5aceaef0acc0bde949443dce3fe802e99fbe74fd` | whole `Cargo.lock` blob `e51a19684170f3c8677bebd41a4a1351c9176f27` / SHA-256 `f3c0540d3e5d6653918e4b1cd553e4063782addc4b36f7aff270fae3b136263c` | BR-203/P2-F | only root `stock_analysis.dependencies` gains `magic-market-core`; no package record/checksum/source change |
| P2-SA1 | controlled strict audit caller/test target | `96da6747e147788d7ae66c357b5679caf1352f51:src/selection/outcome.rs` SHA-256 `18df8c79c7dc6b50ff51ea6c426a25ab98e9b6e3e227547fd2eda4364dfadd5e` | provisional whole target blob `6bce9901173c606dc33188a040e10e476bc18ae4`, SHA-256 `13f3cbb8e753f4723f91373981d861e37094787da07a4fefc41d8f30a3297c75`; computed only until the P2-F commit exists | BR-203/P2-F | remove old enum/constructor; fixed `production()` opens before due-load; exact code plus Lock/Io-only retryability; includes the exact CF2 outcome test; no fallback path |
| P2-SA2 | controlled strict audit caller/test target | `96da6747e147788d7ae66c357b5679caf1352f51:src/selection/pipeline.rs` SHA-256 `c2662ceea2f7b0b5981d80126f050005b3018d24b2c1575a9aac982d4d579461` | provisional whole target blob `7746ddc127657a31bed79141f754454327d85cef`, SHA-256 `bc345454c9d1d6fd07873ae966d997173dc92e3b1c6a3a55aef6f056184f968e`; computed only until the P2-F commit exists | BR-203/P2-F | fixed `production()` opens before port construction; exact Unavailable code/retryability; includes the exact CF2 pipeline test; zero downstream calls; no fallback path |
| P2-H1 | controlled compile-hygiene hunk | full parent `src/selection/audit.rs` | add `#[cfg(test)]` only to `for_test_code_pinned_root`; freeze whole target | BR-203/P2-F | removes dead production code without an allow; unit tests remain the only caller |
| P2-H2 | controlled compile-hygiene hunk | full parent `src/durable_delivery/schema.rs` | replace the ten-element inline tuple with private `ImmutableAuditOutboxV4Row`; freeze whole target | BR-203/P2-F | behavior-preserving type alias; strict Clippy zero-warning |
| P2-H3 | controlled test-fixture target | `96da6747e147788d7ae66c357b5679caf1352f51:src/durable_delivery/tests.rs`; freeze preimage hash | add only TEST_CODE legacy decision/attempt parents for the two migration rejection fixtures and create `data/test` before the foreign-CWD child; freeze whole target | BR-203/P2-F | keeps foreign keys enabled, exercises the intended migration errors, removes test-order dependence and changes no production path |
| P2-H4 | clean-checkout namespace sentinel | no historical source | whole `data/.gitkeep`; freeze whole target | BR-203/P2-F | makes the non-production test namespace parent available without creating, opening or reading a production SQLite artifact |
| P2-H5 | controlled append-capability compile closure | `96da6747e147788d7ae66c357b5679caf1352f51:src/bin/monitor/review_batch.rs`; freeze preimage hash | add only `append_source_protocol_audit_to`, its production-root wrapper and exact hash-chained append regression; freeze whole target | BR-203/P2-F | satisfies the already-admitted source-protocol audit call with a real immutable append rather than a logging-only implementation; no provider, renderer or sink activation |
| P2-H6 | controlled immutable-run-context compile closure | `96da6747e147788d7ae66c357b5679caf1352f51:src/bin/monitor/main.rs` plus `src/bin/monitor/push_templates.rs`; freeze both preimages | construct one `ReviewRunContext` at the strict-review boundary, pass it atomically to the dispatcher and call the existing three-argument `review_preflight(context,due,is_test)`; freeze both whole targets | BR-203/P2-F | resolves the stale two-argument caller without changing provider/task policy; the review date and eligibility time derive from one observation and TEST_CODE remains physically isolated |
| P2-H7 | production/test boundary compile hygiene | frozen P2-H5/P2-H6 monitor targets plus fixed-baseline `notify.rs`, `v14_adapter.rs` and `durable_delivery_runtime.rs`; freeze every preimage hash | put only the exact P3-owned counted-delivery/replay/hydration/review-orchestration declarations behind `#[cfg(test)]`, keep their existing TEST_CODE tests compiled, and freeze every affected whole target | BR-203/P2-F | no `allow(dead_code)`, fake caller or lint-level relaxation; P2 production retains only eager capability binding and generic counted-kind rejection, while P3 atomically removes each test gate with its real producer/orchestrator caller |
| P2-H8 | eager-bound capability invariant | P2-H7 target `src/bin/monitor/durable_delivery_runtime.rs` | add private `RuntimeState::verify_bound_capabilities()` and call it from `eager_bind_runtime_artifacts()`; freeze whole target and exact regression | BR-203/P2-F | fail closed unless namespace matches, coordinator/append/sink bindings are live, authoritative sink identity is non-empty, producer readiness is false before P3, and both hydration mutexes are healthy/empty; this reads real bound state instead of suppressing field lints and performs no provider/network/sink call |
| P2-H9 | production-delivery observer test isolation | P2 target `src/event/mod.rs` | extract one private `publish_delivery_with_dispatcher` persist-then-publish seam without changing the public production wrapper, then make the BR-130 global-observer regression inject one `TestAuditNamespace` dispatcher; freeze whole target | BR-192/BR-203/P2-F | the regression never relies on process-global `DURABLE_DELIVERY_TEST_CODE`, never opens production audit state and still proves authoritative persistence precedes observation publication |
| P2-H10 | business-date-bound chain appearance prerequisite | fixed parent `src/database/concepts.rs`, `src/pipeline/chain_analysis/mod.rs`, `src/pipeline/extra_context.rs`, `src/app/modes.rs` and `src/pipeline/mod.rs`; freeze every preimage hash | adopt the complete already registered BR-195 contract: inclusive `[as_of-(days-1), as_of]` query, exact two appearance-query callers, typed public chain-analysis business date, both production entry callers, no `max(1)` fill and exact natural-day wording; freeze all five whole targets and exact BR-195 regressions | BR-195/BR-203/P2-F | removes wall-clock-dependent full-suite and replay behavior without changing assertions, filling a missing count or treating future rows as evidence; this is a prerequisite adoption of an independently reviewed rule, not new BR-203 business semantics |
| P2-H11 | typed counted-governance provenance discriminator | P2-H7 target `src/bin/monitor/v14_adapter.rs`; freeze whole preimage and target | add a distinct TEST_CODE-only explicit-counted context, make the explicit counted gate use it, allow counted governance only for that context or the existing SourceOnly context, and keep generic combined-account/source-fact requests fail-closed; freeze exact rejection and explicit-binding regressions | BR-192/BR-203/P2-F | fixes an existing provenance conflation without widening production; P2 production exposes no counted producer and generic counted requests still return `counted_binding_required` |
| P2-H12 | isolated monitor test runtime and stale generic-counted test retirement | P2-H6/H7 targets `src/bin/monitor/main.rs`, `src/bin/monitor/notify.rs`, `src/bin/monitor/push_templates.rs`, `src/bin/monitor/news_aggregator_init.rs`, `src/bin/monitor/v17_sources.rs` and `src/bin/monitor/durable_delivery_runtime.rs`; freeze every preimage and target | remove forbidden audit/push-log overrides during each TEST_CODE guard, eagerly bind the matching push-log capability, restore captured environment, migrate bare dry-run tests to the guard, use non-counted kinds for generic push/cooldown tests, classify template entries lacking a P2-admitted counted binding as disabled before generic dispatch, and put every retained/mutating `data/test` namespace plus delivery-runtime env test in the one existing `cooldown_memo` serial domain; opt-in sink smoke also captures/restores env; freeze exact intrinsic-failure regressions plus serial and default-thread monitor suites | BR-051/BR-136/BR-192/BR-203/P2-F | production audit roots, descriptor/link-count validation and counted rejection are unchanged; no second ineffective serial key, fake binding or real sink call; P2 E2E evidence reports pushed and disabled counts separately and cannot claim future P3 counted templates delivered |
| P2-H13 | production CLI TEST_CODE bootstrap and process-test isolation | P2-H12 target `src/bin/monitor/main.rs` plus fixed parent `tests/monitor_help_isolation.rs`; freeze both preimages and targets | add only `install_cli_test_delivery_code`, call it immediately after explicit CLI Test-mode selection and before BR-144 audit preflight, and make every process regression that crosses runtime preflight include `--test`; freeze the exact helper/call/test hunks and whole targets | BR-051/BR-192/BR-203/P2-F | accepts only path-safe `TEST_CODE*` or creates PID-plus-time nonce authority; performs no provider or sink call; three named CLI regressions use no-follow metadata including size/mode/mtime/ctime to prove the fixed production `data/durable_delivery.sqlite3{,-wal,-shm}` trio is absent or unchanged; other audit/push-log namespace claims remain owned by their focused module tests; plain `--test` is Gate-B isolation evidence, not BR-196 transport or Gate-D production evidence |
| P2-V0A | new fail-closed checker | no historical source | whole `tools/compliance/lib/check_br203_compile_foundation.sh`; freeze whole target | BR-203/P2-F | exact-one runner registration then structured verifier; executable |
| P2-V0B | new structured verifier | no historical source | whole `tools/compliance/lib/verify_br203_compile_foundation.py`; freeze whole target | BR-203/P2-F | hashes Cargo/lock/callers plus every composed H11/H12/H13 target and both raw inclusive H13 `main.rs` hunks; parses TOML and checks exact literal sibling paths plus production ordering |
| P2-V0C | controlled runner registration | `96da6747e147788d7ae66c357b5679caf1352f51:tools/compliance/check.sh` blob `e573230415dc8d5cfc92d64253488944c368cf8f`, SHA-256 `2615beca0978daa539716985d6a80360a262d8cbb35c271fcc839f192d9eff99` | add exactly one `run_check "check_br203_compile_foundation.sh"`; freeze hunk and whole target | BR-203/P2-F | exactly one literal invocation; no parallel/optional bypass |

The final composed P2-H11/H12/H13 candidate targets used by the green test
packet are frozen below. These identities do not make the ignored candidate
reachable or accepted; the eventual P2-F commit must reproduce them exactly:

| Target | Git blob | SHA-256 |
| --- | --- | --- |
| `src/bin/monitor/v14_adapter.rs` | `2a6e91b410507d2ec163ab6f7fdafd7790d7d060` | `192de69dfa4529928825c4cd27a16e9b8526d9ee89dd0980642b507c3a6f4994` |
| `src/bin/monitor/main.rs` | `4df7bf3ced0510dffc44ec11676a2bf3e1fb82bd` | `f9e7d29e3b415f06d37dda18a942c50ce008d6aee6b3aef898aa522302eaf53a` |
| `src/bin/monitor/notify.rs` | `07dc781aa0a477381951df0674749ba0da4f43ab` | `172694bf1f481a09816530ff29a1f583b4029621a0f787247b21f14fcdfd13d9` |
| `src/bin/monitor/push_templates.rs` | `992a651ed675d971aae586efad5d5677569f4bad` | `8f428f24702ef7f27e8b6278dc04fbd2bbbd05f3019cf9c1d4346f684aed2636` |
| `src/bin/monitor/news_aggregator_init.rs` | `35878107684e0c3c0cf73b25f6f7253ca2ad5081` | `ea3143f9fb409ae35ae7e30ed22007a448573a408a86ed71d9151ddaddbb8c44` |
| `src/bin/monitor/v17_sources.rs` | `378cb03ea45ab84b2496e5737f17e63055fc637e` | `5a9326c04e06e4aac0ab65814866c77f741f512417827eb6c91a408729923241` |
| `src/bin/monitor/durable_delivery_runtime.rs` | `f36aed76f5350ffa47228f757cbd62073583cfad` | `1c39eb2beda7831c5348f0fed2dc7997d935523a5e709c3418cdbdf3376d875c` |
| `tests/monitor_help_isolation.rs` | `1df27a0d11780706c03e7c6896d259dc29090ded` | `a52f92c1d3332173b7749e6a45294e30c2f1ace8996aab7923d268d11908baf1` |

P2-H13 additionally freezes the exact `src/bin/monitor/main.rs:69-94`
helper hunk SHA-256
`5bb1e4535ceafacbebed96c9e3bd6915b8e43a75115535243e53e48b71eb7153`
and the exact `src/bin/monitor/main.rs:2970-2980` pre-preflight call-site hunk SHA-256
`607974122926746e1a33bef62e99ba983f2336ee72bc939257ceb0ba0d01e41f`.
Both hashes are over the raw inclusive `sed -n '<start>,<end>p'` bytes without
line-number decoration.

The P2-V0 checker targets are likewise provisional and frozen before the P2-F
commit:

| Target | Mode | Git blob | SHA-256 |
| --- | --- | --- | --- |
| `tools/compliance/lib/check_br203_compile_foundation.sh` | `100755` | `fe3594e9dbd661574799945f13f29450571060fe` | `e58c09352383076e4b49c27018300d4a8306b88e2032d357ad8596fb375ec5e2` |
| `tools/compliance/lib/verify_br203_compile_foundation.py` | `100755` | `829155d5f7d9f4fde4c393de0ce076ea2d95af81` | `bb4eb24dd4d33d69074db5b7b54be51046118eec6c49b0ba2ed904be34b0395e` |
| `tools/compliance/check.sh` | `100755` | `5f53a0419f0c6838849e638408c050e83e484a4b` | `589f2dc6ff09ea83db0ce2d3d46f7ead0fa166651f566750aed7cbf0d3eb009b` |

The only P2-V0C target delta is raw line 49,
`run_check "check_br203_compile_foundation.sh"`, with SHA-256
`c1ed63cc8aa5271986b9fbe943482cb016f2a45f10ebbf17c930f4ea855001e5`.

All P2-F destination identities are provisional and grant no Gate-B authority
until one candidate commit with parent `96da6747e147788d7ae66c357b5679caf1352f51`
materializes every row, `git cat-file -e` succeeds for its commit/tree/blobs,
and the exact packet below is green. Superseded temporary hashes must not be
staged or cited. Strict all-target Clippy must report zero warnings; P2-H1
through P2-H13 own the observed compile/test root causes rather than relying on
local filesystem residue or a lint relaxation. P2-H7's test gates are removed
only by the atomic P3 consumer slice; any `allow(dead_code)`, crate-wide lint
level change, fake reference or production exposure without a real caller is a
hard failure.

The following former P1-A whole dependency claim is rejected for P2/P1 and is
retained only as deferred BR-164 final-release evidence:

| ID | Class | Fixed preimage | Destination/anchor | Owner | Exact acceptance |
| --- | --- | --- | --- | --- | --- |
| REJECTED-P1-A2 | deferred BR-164 dependency target | `BASELINE:Cargo.toml`; target blob `17e4ff819323d1126f434875d4098681578243c8` | whole `Cargo.toml` final target | BR-164 | not P2/P1 staging authority; final goal only after legacy callers are retired |
| REJECTED-P1-A3 | deferred BR-164 closed-lock target | `BASELINE:Cargo.lock`; target blob `24b2e7d0e4d912404213ad23a1abdf62792b5ad3` | whole `Cargo.lock` final target | BR-164 | not P2/P1 staging authority; final goal only after domain cutovers are green |
| REJECTED-P1-A4 | deferred final test | no historical source | `tests/magic_market_release_revision.rs` | BR-164 | release proof only; excluded from P2-F |
| REJECTED-P1-A5 | deferred final checker | no historical source | dependency checker/runner | BR-164 | release proof only; excluded from P2-F |

P2-C1, P2-C2, P2-SA1, P2-SA2 and all counted-authority rows form one
indivisible P2-F compile foundation. Applying only the counted closure leaves
four selection errors; applying only the caller adaptation leaves the schema
constant error. P0-A1 through P0-A3 must be independently accepted before
P2-F staging. The rejected P1-A rows cannot enter that commit.

At final BR-164 release—not in P2-F or P1 staging—the direct set is exactly `magic-baidu-rs`, `magic-cls-rs`,
`magic-cninfo-rs`, `magic-eastmoney-rs`, `magic-exchange-rs`,
`magic-jin10-rs`, `magic-market-composition`, `magic-market-core`,
`magic-market-router`, `magic-sina-rs`, `magic-tdx-rs`, `magic-tencent-rs`,
`magic-thepaper-rs`, and `magic-ths-rs`. The lock set is exactly those fourteen
plus `magic-market-transport`. All fifteen use version `0.2.0`, repository
`https://github.com/Northofqing/magic-market-data-rs.git`, and revision
`5f1ce93656a55854c844065390520cd4aecd9a14`. The rejected baseline identities
are `Cargo.toml` blob/SHA-256
`2118a3e490efe2d3416b2554559ca0347947c533` /
`521c3b24795288ddce453e714a74e23fe96afe348dfa49c5d68681f0fdf2adfa`
and `Cargo.lock` blob/SHA-256
`95481362e8061a1724cd1682d23b4e8a14f16377` /
`cd86df085943a710c17ec2cb5aceaef0acc0bde949443dce3fe802e99fbe74fd`.
They are input evidence only and must not be restored.

This 14-direct/15-lock set is the final project-wide BR-164 release invariant,
not current Gate-B authority.
Only `magic-eastmoney-rs`, `magic-market-composition`, `magic-market-core` and
`magic-market-router` may be imported by the new P1 R-04/R-09 modules. The
remaining ten dependencies retain existing independently owned consumers and
do not widen P1 source authority; `provider_top_n.rs` specifically must not
import `magic-exchange-rs`.

No P2-F/P1 staging may adopt the following deferred whole-file identities; they
are final BR-164 evidence only:

- `Cargo.toml`: Git blob
  `17e4ff819323d1126f434875d4098681578243c8`, SHA-256
  `093c81e706fa1caea33e5f924b53f2e22c7cdd025a55ccfb726e09af58926d7e`;
- `Cargo.lock`: Git blob
  `24b2e7d0e4d912404213ad23a1abdf62792b5ad3`, SHA-256
  `86fa31db32fcd36dc3cff09b360a61db99f21e1b94bbf4a21023490a145cdcf4`.

`cargo metadata --locked --offline --format-version 1 --no-deps` exits zero
against those deferred bytes, but that does not make them a compile predecessor.
At final BR-164 release the manifest must contain
root `polars = 0.54` with `strings`; the lock must contain the Polars
implementation packages at 0.54.4 and no `polars`, `polars-core` or other
implementation package at 0.46/0.52. `polars-arrow-format` and
`polars-parquet-format` retain their independent format-crate versions and
are not implementation-family exceptions. qmt-parser must be absent from both
files. Exact whole-file hashes close every other package/checksum/dependency
edge and the stock_analysis root record; any drift is a hard failure.

The final BR-164 checker parses both TOML files structurally without invoking dependency
resolution. It fails closed on malformed input, duplicate package identities,
a missing or repeated root record, an unexpected `polars-*` implementation
version or any format exception other than exactly `polars-arrow-format` and
`polars-parquet-format`. The complete lock parser and whole-file hashes prove
the 14/15 dependency closure; `--no-deps` only proves the accepted root
manifest/lock pairing without assuming that non-host packages such as a
Windows target crate have already been cached. The two following locked
`cargo check` commands prove host compilation.

## P1 shared admission seam

The full historical `review.rs` blob is
`674f2bae14f361674a453b908e91dc76634fd82d` with SHA-256
`8a1341397fa0f1a26f522ca1d27d027978be7fba97775ce0c076411050e93226`.
Only the following ranges may inform the new internal module; lines 105-117
(`DailyClose` and `UpperLimitRecord`) and all other review behavior are
excluded.

| ID | Class | Source path and inclusive range | SHA-256 | Destination | Dependency/acceptance boundary |
| --- | --- | --- | --- | --- | --- |
| P1-AD1 | extract + controlled adaptation | `UNTRACKED_WIP:src/data_gateway/review.rs:23-53` | `1d2252e4a0bfd21786ec94a471bca237176d13c45da25715795b7f67e050b8b7` | `src/data_gateway/admission.rs` batch evidence | Preserve provider/batch/source timestamps, but replace the historical hard-coded `"review"` capability with an explicit calling capability. |
| P1-AD2 | extract | `UNTRACKED_WIP:src/data_gateway/review.rs:55-103` | `4a352db80c1aa816ed88ceadc8e6ab86ec40ca4247959102a4e979306ba86202` | same path, `GatewayBatch<T>` | Typed `Available`/`VerifiedEmpty` outcome and display contract only. |
| P1-AD3 | extract | `UNTRACKED_WIP:src/data_gateway/review.rs:119-224` | `3b27687606e500932e26f6578be6c251efb11c660ffb17432d92328b18a43794` | same path, `GatewayError` | Preserve explicit provider/admission/audit failure classes; no fallback fabrication. |
| P1-AD4 | extract + controlled adaptation | `UNTRACKED_WIP:src/data_gateway/review.rs:496-667` | `f8c47fc992a704a8952e6fe910a93cb0578d375ccbdb82ca364566be6cf6b753` | same path, request hash and audit helpers | Preserve canonical hashing and fail-closed append, but replace the historical hard-coded `"review-data-gateway"` failure source with the explicit calling capability/source. |

The two adaptations above are required corrections, not optional refactors.
They must be covered by target tests named
`br159_provider_mismatch_fails_closed`,
`br159_repository_unavailable_fails_closed`,
`br159_audit_append_failure_fails_closed`, and
`br159_missing_batch_evidence_keeps_calling_capability`. Historical test
ranges 759-800, 802-810 and 884-924 are semantic inputs only and cannot be
counted as those target tests. The dependency closure is `chrono::Utc`,
`magic_market_core::{ProviderId, Provenance}`, `sha2`, `hex`, `thiserror`,
`tokio`, `log`, `DatabaseManager`, and `DataAcquisitionAuditRecord`.

| ID | Class | Historical source | Destination | Owner | Exact target test |
| --- | --- | --- | --- | --- | --- |
| P1-ADT1 | new | none | `admission.rs` tests | BR-159 | `br159_provider_mismatch_fails_closed` |
| P1-ADT2 | new | none | `admission.rs` tests | BR-159 | `br159_repository_unavailable_fails_closed` |
| P1-ADT3 | new | none | `admission.rs` tests | BR-159 | `br159_audit_append_failure_fails_closed` |
| P1-ADT4 | new | none | `admission.rs` tests | BR-159 | `br159_missing_batch_evidence_keeps_calling_capability` |

## P1 Provider Top-N deep module

The full historical `capital.rs` blob is `960811e2d567a112b6eacda59afafeaeed857ca1` with SHA-256
`9ed85b79f711a16e9dcaf45a55ea01183c021520d8f7cc9e522815f4d6276b85`.
The destination exposes only `ProviderTopNDataGateway::pair(date)`; fund flow,
northbound, HKEX, `magic-exchange-rs`, observation-age policy, identity and
calendar helpers, the broad `CapitalDataGateway` facade and its other tests are
forbidden.

| ID | Class | Source path and inclusive range | SHA-256 | Destination | Dependency/acceptance boundary |
| --- | --- | --- | --- | --- | --- |
| P1-TN1 | extract | `UNTRACKED_WIP:src/data_gateway/capital.rs:42-43` | `bbdf1bec015e18475106b19fa65a609cd732793eec93a95d185cb98b887a886c` | `src/data_gateway/provider_top_n.rs` | Provider Top-N capability constants only. |
| P1-TN2 | extract | `UNTRACKED_WIP:src/data_gateway/capital.rs:45` | `fbe4f8ec1badb56aac6adc4d4dd9f9764a70de152f40e7ed1dfd2b3a2b8d75a7` | same path | Eastmoney source identifier only. |
| P1-TN3 | extract | `UNTRACKED_WIP:src/data_gateway/capital.rs:47` | `bd6848b1e73477feebbf53a704fa8198dddad9516de2b23119ae549886e8f8ef` | same path | Frozen Top-N limit only. |
| P1-TN4 | extract | `UNTRACKED_WIP:src/data_gateway/capital.rs:69-104` | `3b0e11c63c599ab204a612d248ad37127c64c3eb6c3e9a4346f2bb6a32651b2d` | same path, public DTOs | Three fact/request/pair types only. |
| P1-TN5 | extract + rename | `UNTRACKED_WIP:src/data_gateway/capital.rs:185-241` | `ed5f8891782b867e430d770000ff1be78415c09e6f1bf87e93c9827870a58c48` | `ProviderTopNDataGateway::pair(date)` | Preserve provider composition and pair behavior; the historical broad facade/API name is rejected. |
| P1-TN6 | extract | `UNTRACKED_WIP:src/data_gateway/capital.rs:297-327` | `24a74648da302adc3f03ffefc8fc572c6b4781b4bdb28710d6472a6ad179bb63` | same path | Request construction and request evidence only. |
| P1-TN7 | extract | `UNTRACKED_WIP:src/data_gateway/capital.rs:360-468` | `cd373595e90700b86c9e5b9dffef0a10ee1f37cbce36157f79bdf81b50234768` | same path | Atomic pair/router logic only. |
| P1-TN8 | extract | `UNTRACKED_WIP:src/data_gateway/capital.rs:616-743` | `0413ea5fe3cdc6c3a19b72203f055f9d123d5fa05dd65df469226c1e3e3c94ae` | same path | Batch admission and pair validation only. |
| P1-TN9 | extract | `UNTRACKED_WIP:src/data_gateway/capital.rs:1155-1158` | `b76018e27b1ea9c9fd74340878b21a6f5567e9addf20ce1f3a2a9f6b614f7ee2` | same path | ISO date helper only. |
| P1-TN10 | extract | `UNTRACKED_WIP:src/data_gateway/capital.rs:1206-1368` | `2bc691de2766161095bff3ba9fceb576205fdf2ec33b56c84776328ea86673fb` | same path | Eastmoney/router/join failure mapping only. |
| P1-TNT1 | extract fixture | `UNTRACKED_WIP:src/data_gateway/capital.rs:1553-1603` | `0efe3d956f969f9fb3770f7b7851efc7d75fbeb5a3487c43be8d8f17bc50d5eb` | provider Top-N test module | Narrow fixture only. |
| P1-TNT2 | extract test | `UNTRACKED_WIP:src/data_gateway/capital.rs:1768-1801` | `f77424ecaf48970edd69c46e1e78b977a0d500918ab3fa528c66807d3048b861` | same test module | Typed/order/source-at semantics only. |
| P1-TNT3 | extract tests | `UNTRACKED_WIP:src/data_gateway/capital.rs:2247-2393` | `6b6d5909b43d5740b73b2afc83778b54a9d26b088fb43733f2e0525e23e0b101` | same test module | Rejection and atomic-pair semantics only. |

The three target tests are
`br192_provider_top_n_seam_preserves_typed_rows_order_and_absent_source_at`,
`br192_provider_top_n_admission_rejects_partial_order_date_identity_and_source_at`,
and
`br192_provider_top_n_pair_is_atomic_and_rejects_empty_or_metric_drift`.
Dependencies are the admitted Magic Eastmoney/composition/core/router crates,
`chrono::NaiveDate`, `tokio`, and the internal admission seam.

## P1 DragonTiger gateway

The historical full blob is `014cdb3dde8205436fa754d308fa3a90db7b3adc` with SHA-256
`fb73d7a88e376484abcad91505ceb01f43633836917e45484f4cf6872ce34115`.

| ID | Class | Source path and inclusive range | SHA-256 | Destination | Dependency/acceptance boundary |
| --- | --- | --- | --- | --- | --- |
| P1-DT1 | extract + import adaptation | `UNTRACKED_WIP:src/data_gateway/dragon_tiger.rs:16-503` | `f2a2e056b1b8e440e2b978fa3f57840fbeb6db52d46eaf3ce98365584fb10512` | `src/data_gateway/dragon_tiger.rs` | Preserve all production behavior; change only `super::review` imports to the admitted `super::admission` seam. |
| P1-DTT1 | exact | `UNTRACKED_WIP:src/data_gateway/dragon_tiger.rs:504-760` | `62dc7475d44da3ff43afc31f406d31b05ce51347e4b246487eba5eccdc890005` | same file test module | Six BR-162 aggregation, identity, seat, ordering and error/audit tests. |

The six exact tests are
`br162_groups_by_stock_without_summing_distinct_trade_ids`,
`br162_filters_nonpositive_stocks_then_limits_after_grouping`,
`br162_request_identity_and_trade_id_validation_are_explicit`,
`br162_seat_contract_requires_exact_buy_and_sell_five`,
`br162_disclosure_sorting_and_exchange_order_are_deterministic`, and
`br162_eastmoney_errors_keep_retry_and_audit_semantics`.

## P1 BR-159 database and module glue

| ID | Class | Source path and inclusive range | SHA-256 | Destination | Dependency/acceptance boundary |
| --- | --- | --- | --- | --- | --- |
| P1-DB1 | exact | `UNTRACKED_WIP:src/database/data_acquisition_audit.rs:1-561` | `716f08a7159f0c54db76948daf1a823dcc98a2fb74c7e74206b43e6f37b5c873` | same path | Complete append-only hash chain, repository method, table/indexes, four immutable triggers and four BR-159 tests. This is an additive idempotent schema installation. |
| P1-G1 | exact | `TRACKED_WIP:src/lib.rs:16` | `696ce94f4ddaba0c75554e8ee825d6cfae682d8eedfa3b7ea9b5888fa1993663` | `src/lib.rs` | Register `data_gateway` only. |
| P1-G2 | exact | `TRACKED_WIP:src/database/mod.rs:1046` | `b94ee83023ad20a68a95d17473f9ed29be4f0518f2c0a61ef3c618cec05bfa0a` | same path | Register the BR-159 database module only. |
| P1-G3 | exact | `TRACKED_WIP:src/database/mod.rs:1863-1866` | `eda534a56d21b77067a08de42f1d4a66d3ed5b97cd38a6d0205cc6617f844cad` | same path | Initialize the BR-159 schema only; `record_data_acquisition` remains owned by P1-DB1. |
| P1-G4 | new | no historical range admitted | — | `src/data_gateway/mod.rs` | Internal `admission`, public `provider_top_n` and `dragon_tiger`, plus only the DTO re-exports required by P3. The historical 25-module facade is rejected. |

P1-DB1 contains and must retain
`br159_append_is_atomic_and_provider_transitions_are_explicit`,
`br159_invalid_success_without_batch_id_writes_nothing`,
`br159_hash_chain_detects_tampering`, and
`br159_tampered_tail_blocks_the_next_append`. Its dependencies are Diesel
SQLite, serde, sha2, hex and `DatabaseManager`.

## P2 counted authority

| ID | Class | Source path and inclusive range | SHA-256 | Destination | Dependency/acceptance boundary |
| --- | --- | --- | --- | --- | --- |
| P2-E1 | controlled whole-file adaptation | `TRACKED_WIP:src/event/envelope.rs:1-738` | `117179dd80d67c093fe3908051a96f33b4c0d097c46880e56d07d4f0a78f028e` | complete `src/event/envelope.rs`, then insert only P2-T4 in its existing test module | Historical schema-v3 file is the immutable preimage, not target hash; no byte besides the named T4 insertion may differ, and the whole candidate target blob/SHA-256 is frozen before staging. |
| P2-E2 | exact | `TRACKED_WIP:src/event/mod.rs:1-520` | `51cfcc6ae3066dee3376c9b54120d11d387cea9fd7c88fd1466f7b9c853293b6` | `src/event/mod.rs` | Exports existing dispatcher plus exact-byte append; owns counted audit bind/publish and exact persistence tests. |
| P2-E3 | controlled whole-file compatibility adaptation | `TRACKED_WIP:src/event/push_record.rs:1-968` | `0f05bb39b5cb1057d1c1a7b67b2cfacb767aee7623ab7d386301c05fc9d11274` | complete `src/event/push_record.rs`; add `skip_serializing_if = "Option::is_none"` to exactly the eight counted-only optional output fields at source lines 66-73; insert P2-T5 in the existing test module | Historical schema-v2/v3 parser file is the immutable preimage, not target hash. The eight attributes are mandatory to remove WIP-only `null` drift and restore the frozen 582-byte schema-v2 output without changing schema-v3 parsing. No other production byte and no test byte besides T5 may differ; freeze the whole candidate target blob/SHA-256 before staging. |
| P2-E3A | controlled compatibility hunk | `TRACKED_WIP:src/event/push_record.rs:66-73` | `4bea98c2ed8934fb2cae65b974ead87f3ea2a94aada7011527a3dde1b79230f1` | target `src/event/push_record.rs:66-81`, the same eight fields expanded to sixteen attribute-plus-field lines | The exact target-hunk SHA-256 is `9c716e0ac8bbd18cebb314fe7c2225d642cb4530504fba24380b49b29a5365a5`; every attribute is exactly `#[serde(skip_serializing_if = "Option::is_none")]`. Mini-probe output must remain the frozen 582-byte P2-T5 fixture. |
| P2-E4 | exact | `UNTRACKED_WIP:src/event/durable_delivery_append.rs:1-1841` | `1e051097f01012d97b707c88c282219b85617a3509520e4860c3793895ee419c` | `src/event/durable_delivery_append.rs` | Complete exact-byte append owner; nine direct BR-192 parent tests plus isolated TEST_CODE child probes. |
| P2-L0 | exact compile closure | `TRACKED_WIP:src/lib.rs:20-20` | `eaf911fecadb7c1ac32b2166e874323674e8d778e1f1cf2a92dcba8ba185ebbf` | crate-root module declaration after `pub mod decision;` | Add only `pub mod durable_delivery;`; the owned module files already exist in the implementation parent. No other `src/lib.rs` byte belongs to P2. |
| P2-N1 | exact | `TRACKED_WIP:src/bin/monitor/notify.rs:1-22` | `9a56aa3b5471fd919693d86d6e5d0479cd2ae0ec42e8993f082dd01a9c91f389` | `src/bin/monitor/notify.rs` module header | BR-192 Unix `openat`/`mkdirat` platform contract only. |
| P2-K1 | exact compile closure | `TRACKED_WIP:src/bin/monitor/notify.rs:117-118` | `dbd73d1e458d1ecef68e7a5ae507a5ed0ccd4b87ffd40d1e6d34840d02333399` | `PushKind` after `EventCalendar` | Add the monitor `ReviewProviderTopN` variant required by already committed fixed-baseline mappings; adds no producer. |
| P2-K2 | exact compile closure | `TRACKED_WIP:src/bin/monitor/notify.rs:290-290` | `b49116c23d27ab026a54fc93aec9831b3a81a6d14a9f8ad83a4d8bc2d6cf1ee2` | `PushKind::level()` Important chain | Complete the new variant's exhaustive match. |
| P2-K3 | exact compile closure | `TRACKED_WIP:src/bin/monitor/notify.rs:344-344` | `6a27fc758bb2c27712b70d76bb832af904f6ab5bc4c40848553e4edf51979225` | `PushKind::requires_banner()` | Preserve the fixed-baseline SourceOnly admission expectation; no banner read is introduced. |
| P2-K4 | exact compile closure | `TRACKED_WIP:src/bin/monitor/notify.rs:380-380` | `b49116c23d27ab026a54fc93aec9831b3a81a6d14a9f8ad83a4d8bc2d6cf1ee2` | `PushKind::cooldown_secs()` daily chain | Complete the new variant's exhaustive match with the frozen daily cooldown. |
| P2-K5 | exact compile closure | `TRACKED_WIP:src/bin/monitor/notify.rs:482-482` | `c64e03d3687da5fedd860e748c7022db83ed52e7a9df8903028abb4c8affbf27` | `PushKind::label()` after `EventCalendar` | Complete the new variant's label match. `stable_template_id()` is generic and unchanged. |
| P2-N0 | exact | `TRACKED_WIP:src/bin/monitor/notify.rs:883-883` | `25cf5fc23f2a6d23c3ea51e72f5e896c7555fbaa05cd5b1ea182bf9618a8e81b` | immediately before fixed-baseline `create_push_log_file` | Add `#[cfg(test)]` so the replaced legacy helper does not violate strict production clippy. |
| P2-N2 | exact | `TRACKED_WIP:src/bin/monitor/notify.rs:892-2008` | `648412bfbdc1802c6d051a75f035f3cc9665de32d1a405ac7a22d5caed0503f0` | `src/bin/monitor/notify.rs` secure push-log owner | `PushLogError`, pinned directory/writer, identity checks, eager binding and secure generic save wrapper. No token/daemon/BR-160/BR-197–200 behavior is admitted. |
| P2-N2A | exact | `TRACKED_WIP:src/bin/monitor/notify.rs:2109-2114` | `70be544cf073254362cd9433c9f58a9040199c889d2ae75fb4da12108898b44b` | generic governor after `push_governor_inner_with_source_fact` import | Reject counted kinds before they can enter the legacy governor. |
| P2-N2B | exact | `TRACKED_WIP:src/bin/monitor/notify.rs:2193-2198` | `43dbf873a18d6f46e4232e1aa37f8909cd1765480f2ce07e94ec98ea133296bb` | delivery path after `deliver_and_record` import | Reject counted kinds before BR-144/L6/legacy sink fallback. |
| P2-N3 | exact | `TRACKED_WIP:src/bin/monitor/notify.rs:2340-2385` | `06f000a55aa1240ef75f80ee4dae8ff89904f29adaae82e29cfc80a41fd64b62` | `src/bin/monitor/notify.rs` generic counted entry | Adds only the unreachable BR-192 generic counted binding entry; P2 adds no production R-04/R-09 caller. |
| P2-N3A | controlled adaptation | `TRACKED_WIP:src/bin/monitor/notify.rs:2491-2523` | `ce4baff2fb6ef8254bbdd845e0bf849fb9ce6fb16d7b45143fbc898869cd442f` | replace fixed-baseline `push_wechat` prefix and add only the closed P2-T6/T7 `#[cfg(test)]` mode seam at its existing sink boundary | Preserve namespace binding and both real `save_push_log` calls byte-semantically; production wrapper always selects the existing production mode. Test-only spy mode is unconstructable outside `cfg(test)`; its private dry-run selector proves zero boundary calls, while live-spy mode increments exactly once and returns before token/daemon/transport/external-sink resolution. Freeze the complete adapted target hunk before staging. |
| P2-N4 | exact | `TRACKED_WIP:src/bin/monitor/notify.rs:2713-3519` | `a7b4e069c77620e25da4ac016fb0845520a862addc64bb76cca08befdd88d027` | `src/bin/monitor/notify.rs` authoritative adapter | Complete sink adapter, pending/audit/commit finalization, exact terminal joins and blocking CLI receipt wrapper. It depends on the fixed baseline's existing send-type/transport/target/bin/home/receipt-parser helpers; those unrelated helper rewrites are excluded. |
| P2-M0 | exact compile closure | `TRACKED_WIP:src/bin/monitor/main.rs:155-155` | `7f9ea27a99daa0639cf90e31a6712fb7211baf4ea5a366f7a260c9c7834b418a` | monitor module-declaration block | Add `mod durable_delivery_runtime;`; the existing module file remains byte-identical to `BASELINE` Git blob `a635b90237413577a51d5bc92ae29c40ae2afac4`. |
| P2-R4V1 | exact pure compile closure | `TRACKED_WIP:src/bin/monitor/push_templates.rs:8638-8735` | `145892a0bdb3dce3e2f18d0ad100103274839e780caaa0b305be736b085ed811` | `src/bin/monitor/push_templates.rs`, immediately before P3 R-04 preparation | Typed canonical DTOs, strict canonical-byte validator and test-only canonical fixture helper required by the unchanged baseline durable runtime. No provider, renderer, scheduler, producer or sink call is included. |
| P2-M1 | exact | `TRACKED_WIP:src/bin/monitor/main.rs:3359-3365` | `0769047e84502e3d51fa9892b1f38aa79824606d3183661b40d43070cb2ec898` | after BR-144 audit preflight and before sink initialization | Eagerly bind runtime audit/push-log artifacts before any production delivery path. |
| P2-M2 | exact test-runtime namespace-isolation closure | `TRACKED_WIP:src/bin/monitor/main.rs:81-111` | `9034bfe82508f5f7fd86e6b70cf0d3590612b144cdaac4bb1e36bc2c72a89179` | existing `TestEnvGuard::dry_run_non_quiet` | Capture and set an invocation-unique `DURABLE_DELIVERY_TEST_CODE` alongside the existing test environment/audit namespace. This row changes only `#[cfg(test)]` guard behavior; the separately manifested P2-H13 production-compiled helper is outside P2-M2. |
| P2-T0 | exact | `TRACKED_WIP:src/bin/monitor/notify.rs:4668-4670` | `4831d7e30bf2d01efb752a537c7735d0c22149c6d347ec77a73f0de3e531afbd` | existing notify test module before namespace fixture | Declare `TestBannerGuard` and `TestNotifyDir` used by admitted tests. |
| P2-T1 | exact | `TRACKED_WIP:src/bin/monitor/notify.rs:4672-4764` | `92c24ca5d093f93edfbaf6a7c541ea76b61dd64529d2e3ab473313e6b589be49` | existing `notify.rs` test module | TEST_CODE pinned namespace fixture and JSON artifact enumerator only. |
| P2-T1A | exact | `TRACKED_WIP:src/bin/monitor/notify.rs:4766-4798` | `1fa88883b0fad152f1a249616ed3c19f6ed604284abd5b198517a0dfbf2c89dd` | after `push_log_json_artifacts` | Implement `TestNotifyDir` and `TestBannerGuard::full`. |
| P2-T2 | exact | `TRACKED_WIP:src/bin/monitor/notify.rs:4824-5653` | `902bef612c042b95163d1c38f5b1596e7dfe70786d3d02159e445471e2c0b7f7` | existing `notify.rs` test module | Counted request/audit fixtures; fail-closed finalization and push-log physical-isolation tests. |
| P2-T3 | exact | `TRACKED_WIP:src/bin/monitor/notify.rs:6053-6180` | `e57a8cd225f85b636f26c073a82110b75414031f8bae9b627d016b9e76fed171` | existing `notify.rs` test module | Explicit counted dry-run, production-no-synthetic-receipt, and generic governor rejection tests. |
| P2-T4 | new controlled test splice | frozen fixture in §P2 golden fixtures | `8de344f9fa9b80cbd114474f9299190a7e53a2d57553a4f649d2f4ef9f36bd33` | `event::envelope::tests::br192_schema_v2_golden_publication_bytes_are_unchanged` in P2-E1's existing test module | Fixed 254-byte input; compare exact 689-byte schema-v2 publication bytes. This is the only permitted P2-E1 insertion. |
| P2-T5 | new controlled test splice | frozen fixture in §P2 golden fixtures | `bd198be71b8cdc2e3f66b93ac3bc515f6bff453412639176dcb449c1beab6680` | `event::push_record::tests::br192_schema_v2_golden_parser_output_bytes_are_unchanged` in P2-E3's existing test module | Parse the fixed input through the public authoritative parser and compare exact 582-byte canonical output. This is the only permitted P2-E3 insertion. |
| P2-T6 | new non-counted golden | frozen fixture in §P2 golden fixtures | `41d03c80490b6c553aba19da0219db7ad3b69527f2bb24f80dfe9a52e496fb6d` | `notify::tests::br192_non_counted_dry_run_golden_push_log_has_exact_bytes_and_zero_sink_calls` | Exercise namespace-aware dry-run through the private implementation shared by `push_wechat`, using only the closed test mode; assert one exact 73-byte artifact and zero sink-boundary calls. |
| P2-T7 | new non-counted golden + `cfg(test)` spy seam | same frozen artifact authority as P2-T6 | `41d03c80490b6c553aba19da0219db7ad3b69527f2bb24f80dfe9a52e496fb6d` | `notify::tests::br192_non_counted_live_golden_push_log_has_exact_bytes_and_one_existing_sink_call` plus the P2-N3A closed test seam | Shared path writes the artifact once before the existing sink boundary; test-only spy increments one atomic counter and returns a fixed result. Assert one artifact and counter=1; no env/global hook, token, daemon, transport or external sink. |

### P2 golden fixtures

The canonical schema-v2 input is exactly 254 UTF-8 bytes, has no trailing LF,
and has SHA-256
`95426b1e6fc5a66cdfd2df9340536db5acc630666f6cc872b0306e4fb29b2802`:

```text
{"id":"TEST_CODE_SCHEMA_V2_EVENT","trace_id":"TEST_CODE_SCHEMA_V2_TRACE","ts":"2026-07-30T15:30:00+08:00","kind":"TEST_CODE_SCHEMA_V2_GOLDEN","code":"TEST_CODE_600519","outcome":"SinkError","channel":"TEST_CODE_DRY_RUN","rendered_len":37,"latency_ms":41}
```

P2-T4's fixed publication output is exactly 689 UTF-8 bytes, has no trailing
LF, and has SHA-256
`8de344f9fa9b80cbd114474f9299190a7e53a2d57553a4f649d2f4ef9f36bd33`:

```text
{"id":"TEST_CODE_SCHEMA_V2_EVENT","ts":"2026-07-30T15:30:00+08:00","trace_id":"TEST_CODE_SCHEMA_V2_TRACE","source":"push_l4","event_type":"push.delivery.audit","entity_key":null,"payload":{"audit_schema_version":2,"channel":"TEST_CODE_DRY_RUN","decision_status":"SinkError","identity_hash":"0fc18304dc89e9273f362c7f3e8e9a98daab342d3a86c6363927d12d981cdb45","kind":"TEST_CODE_SCHEMA_V2_GOLDEN","latency_ms":41,"outcome":"SinkError","reason_code":"delivery.sink_error","rendered_len":37,"retryable":true,"rule_ids":["2.7","BR-091","BR-111","BR-130","BR-142"],"source_as_of":null,"subject_hash":"6011c161a3762e3615acd590160a58bf05b6a258df9c42cba40b0c439d0c6db3"},"version":1,"replay_of":null}
```

P2-T5's fixed parser output is exactly 582 UTF-8 bytes, has no trailing LF,
and has SHA-256
`bd198be71b8cdc2e3f66b93ac3bc515f6bff453412639176dcb449c1beab6680`:

```text
{"id":"TEST_CODE_SCHEMA_V2_EVENT","kind":"TEST_CODE_SCHEMA_V2_GOLDEN","code":null,"trace_id":"TEST_CODE_SCHEMA_V2_TRACE","ts":"2026-07-30T15:30:00+08:00","outcome":"Failed","channel":"TEST_CODE_DRY_RUN","rendered_len":37,"latency_ms":41,"subject_hash":"6011c161a3762e3615acd590160a58bf05b6a258df9c42cba40b0c439d0c6db3","identity_hash":"0fc18304dc89e9273f362c7f3e8e9a98daab342d3a86c6363927d12d981cdb45","decision_status":"Failed","retryable":true,"rule_ids":["2.7","BR-091","BR-111","BR-130","BR-142"],"reason_code":"delivery.sink_error","source_as_of":null,"audit_schema_version":2}
```

P2-T6/T7's artifact is exactly 73 UTF-8 bytes, includes the final LF after the
last line, and has SHA-256
`41d03c80490b6c553aba19da0219db7ad3b69527f2bb24f80dfe9a52e496fb6d`:

```text
TEST_CODE_NON_COUNTED_GOLDEN
模板: 非 counted 推送
金额: ¥123.45
```

## P3 SourceOnly notification seam

| ID | Class | Source path and inclusive range | SHA-256 | Destination | Dependency/acceptance boundary |
| --- | --- | --- | --- | --- | --- |
| P3-N1 | exact | `TRACKED_WIP:src/bin/monitor/notify.rs:2387-2453` | `f7d4fd8acbb6d97458e6cd9e5d0353396e71ececddd5595879fa003688dcd6f0` | `src/bin/monitor/notify.rs` SourceOnly entry | Dedicated R-04-only validation/gate helper. It is committed atomically with its P3 producer and tests, never as a reachable generic transition. |

## P3 production orchestration

| ID | Class | Source path and inclusive range | SHA-256 | Destination | Dependency/acceptance boundary |
| --- | --- | --- | --- | --- | --- |
| P3-M0 | exact startup fixed-point barrier | `TRACKED_WIP:src/bin/monitor/main.rs:3617-3648` | `0b689f74b65878723161fb05d4d12358766ea45a99c0c5241f9916f48c6169a6` | after core database installation and before startup provider, scheduler, producer or sink activation | Await `durable_delivery_runtime::ensure_startup_reconciled()`; on any failure log the fixed-point failure and exit 2 with zero provider/sink calls. This is P3-owned because P3 activates producers. |
| P3-M1 | exact | `TRACKED_WIP:src/bin/monitor/main.rs:3115-3208` | `f6343edf99e35ade69a4614398fb973403da3c25756377b88f2d4129d62aff5d` | same path, replay command/parser | Depends only on `ReviewTask::from_label` and verified A-share calendar. |
| P3-M2 | extract | `TRACKED_WIP:src/bin/monitor/main.rs:3227-3260` | `8cd757d25e5986a77060f32504bd40673be71988f806c0dca647de9bd714ac0f` | beginning of existing `main()` | Install parser before ordinary bootstrap; whole enclosing `main` is forbidden. |
| P3-M3 | exact | `TRACKED_WIP:src/bin/monitor/main.rs:4092-4125` | `bef583d4f808d949a07ee635b033022ca6b67cd5ee9ae8b8916c3b2aaab41bd3` | same path, hydration helpers | Depends on pending/ack hydration runtime and `ReviewScheduleState`. |
| P3-M4 | exact replacement | `TRACKED_WIP:src/bin/monitor/main.rs:4129-4192` | `41a409db8510bf2edc5e87a29e15bcb4fea617dd5a2651afd2e7bf77025465e6` | `run_review_only` | Removes caller-wide AccountMode gate; uses immutable run context and hydration/transition audit. |
| P3-M5 | exact replacement | `TRACKED_WIP:src/bin/monitor/main.rs:4194-4202` | `e96a27f59b8661436978d5e49a331e1e35bbb8fb34376f7bd3990e157ef176cc` | strict inner runner | Canonical dispatcher with one immutable context. |
| P3-M6 | exact replacement | `TRACKED_WIP:src/bin/monitor/main.rs:4210-4221` | `cfb1366a91a2408c0b3690169cf9825ce79c99190d192176967d3509638342c4` | `attempt_post_session_review` | Removes scheduler-wide account gate; preserves timeout and strict runner. |
| P3-M7 | extract | `TRACKED_WIP:src/bin/monitor/main.rs:4322-4365` | `4528db225ec18972b28a6ee55e966298b1010072ca5d1ed479830940b24898ee` | existing scheduler around batch commit | Hydration/transition application only. Enclosing lines 4223–4379 contain unrelated BR-178 selection/AI work and are rejected. |
| P3-R4-1 | exact | `TRACKED_WIP:src/bin/monitor/push_templates.rs:1382-1472` | `b24d8661149dca7214fba9e4ca1c7056e5d35dceb1492b4fa5878e4bdd1fc7b6` | R-04 renderer/helpers | Typed DragonTiger DTO/exchange/side rendering only. |
| P3-R9-1 | extract + path refactor | `TRACKED_WIP:src/bin/monitor/push_templates.rs:5970-6533` | `7115ce138cf3fe8dd049f9bfe1362503cc21ca672517d2d1d59067e26dcb097d` | R-09 structs/renderer/binding/envelope/outcome loader | Historical `data_gateway::capital::*` references are replaced by P1 `provider_top_n`; request/order/evidence/rendering semantics remain unchanged. |
| P3-R9-2 | extract + required refactor | `TRACKED_WIP:src/bin/monitor/push_templates.rs:6535-6544` | `ca771b0d2619cd30360b64d7264557187a93cbabad4642a137e71a36ef9a23d8` | R-09 production wrapper | Source's forbidden `CapitalDataGateway::provider_top_n_pair` is replaced by `ProviderTopNDataGateway::pair(date)`; never exact-copy this row. |
| P3-G1 | exact | `TRACKED_WIP:src/bin/monitor/push_templates.rs:6887-6969` | `bbd5dac43af7b41e93d4d9134d50cec8a7a0a808f0c80ce565d320c0c236b1dd` | canonical post-session dispatcher | Frozen BR-194 preflight/partition/account failure/stable merge plus R-04/R-09 producer calls. |
| P3-R4-2 | exact | `TRACKED_WIP:src/bin/monitor/push_templates.rs:8737-9099` | `2b61b73251f292c8f1b984a9ae622fa6cc76ae3b890db4ef0edb9cba59c2d1fa` | R-04 preparation/dispatch after P2-R4V1 | Depends on the P2 pure validator, P1 DragonTiger Gateway and P3 dedicated SourceOnly notify entry; no later-rule marker is present. |

Rejected `main.rs` enclosing ranges are lines 3210–4060
(`ac78bba346f1b6a71c1b7c3cf85e4c8d107bdffc7f8ed2e6d849d680c8cd9eb9`)
and 4223–4379
(`e3a1b7e2eb8e49588b22383d7e30ec4a0c407860df13736244a5351cf7fa7244`).
Only P3-M2 and P3-M7 may be extracted from them.

## P3 tests

| ID | Class | Source path and inclusive range | SHA-256 | Destination | Dependency/acceptance boundary |
| --- | --- | --- | --- | --- | --- |
| P3-T0 | exact import | `TRACKED_WIP:src/bin/monitor/main.rs:8471-8471` | `a595dbca09db3ab1bc5c64137785d7e173876961de58e4d4a97a5218b38c4e42` | scheduler test module after `chrono::{NaiveDate, NaiveDateTime}` | Import `sha2::{Digest, Sha256}` required by the admitted hydration helper tests. |
| P3-T1 | exact | `TRACKED_WIP:src/bin/monitor/main.rs:8480-8533` | `5744ecf507e038640006d68ea9e4d2d26562d0b2cb7d502dea5341c1c45b1d43` | existing scheduler test module | Exact hydration helpers; add the required `sha2::{Digest, Sha256}` module import. |
| P3-T2 | exact | `TRACKED_WIP:src/bin/monitor/main.rs:8542-8559` | `e54546f53da2baa2d1748a06934ea7e13a8fb0e7deaf4f37aa81c558bc033154` | scheduler tests | Immutable `ReviewRunContext` test. |
| P3-T3 | exact | `TRACKED_WIP:src/bin/monitor/main.rs:8580-8635` | `45bde6c9d298493c0f55d0d25a11ea1b8faa98d27d2861e69492ae00c25522f1` | scheduler tests | Two hydration application tests. |
| P3-T4 | exact | `TRACKED_WIP:src/bin/monitor/main.rs:8682-8701` | `830b6d415c0964c52da20d20d2d9659cd41c96bf19b8b305bbc3c247a0f1d32b` | scheduler tests | Production source inspection test. |
| P3-T5 | extract + path adaptation | `TRACKED_WIP:src/bin/monitor/push_templates.rs:6546-6885` | `b4a0be152387f4b0fbaab0b06499f614feffe0171c94233008225296a8f4d10f` | R-09 test module | Preserve test semantics while changing historical `capital` paths/types to the fixed P1 `provider_top_n` exports; never classify this row conditionally or exact. |
| P3-T6 | extract | `TRACKED_WIP:src/bin/monitor/push_templates.rs:9129-9377` | `8122cdb1a131868afbf717eaa8aad976bdf827d73566a8ddda2a02c6f3eb1507` | existing R-dispatcher tests | Preserve four frozen behaviors; the over-broad substring assertion is removed rather than copied. |
| P3-T6A | new syntax-aware regression | no historical range admitted | — | same R-dispatcher test module | Exact name `tests_r_dispatchers::br192_r04_source_only_call_is_the_only_counted_entry`; parses the R-04 dispatch function body and asserts one `push_counted_source_only_with_binding(` call and zero standalone `push_counted_with_binding(` calls by token/call identity, not substring. This is the third `br192_r04_*` test, not part of M31. |
| P3-T7 | exact | `TRACKED_WIP:src/bin/monitor/push_templates.rs:9444-9509` | `b521f46bc1302fe17aee34c6bd0ed81897ea5027773d038e0e5895acc79c5102` | existing dispatcher tests | R-04 loader seam and typed outcomes. |
| P3-T8 | extract fixture | `TRACKED_WIP:src/bin/monitor/notify.rs:4519-4542` | `67e154d60e953e1d07035709b290e28b16e47e0fd722c644c4a9ade56e8c31a1` | existing notify test module immediately after `use super::*;` | Shared `br194_test_binding` and `br194_approved_event` fixtures only. |
| P3-T9 | exact | `TRACKED_WIP:src/bin/monitor/notify.rs:4544-4580` | `073ac26217788c00a151ba71a5502c05221c8e5c7867c99b4bdc4710867cc393` | same notify test module | `notify::tests::br194_r04_source_only_gate_never_reads_banner`. |
| P3-T10 | exact | `TRACKED_WIP:src/bin/monitor/notify.rs:4582-4636` | `978d778d915a549defc763f410ec811df4126770e633f73a7f0fa123f2007f1c` | same notify test module | `notify::tests::br194_r04_source_only_preserves_l5_and_durable_entry`. |
| P3-T11 | exact | `TRACKED_WIP:src/bin/monitor/notify.rs:4638-4666` | `f142dd87d13188c3443098487256be2380bad79258171d6ffafd2191e26b8da5` | same notify test module | `notify::tests::br194_r04_source_only_denied_launch_has_zero_durable_and_sink`. |
| P3-T12 | extract one test increment | `TRACKED_WIP:src/bin/monitor/push_templates.rs:14743-14743` | `e3814a9b7f45659429d9f52aafd552ffb58153895c3d47440293321b02ff2105` | `counted_kinds_bypass_process_local_cooldown` array after `ReviewMarket` | Add `ReviewProviderTopN` to the existing counted catalog regression; exact test name `push_templates::tests::counted_kinds_bypass_process_local_cooldown`. |
| P3-CC1 | exact new file | `UNTRACKED_WIP:tests/durable_delivery_counted_cutover.rs:1-49` | `89f840de819c5ed559fac72eda56676a7131efcff58e4e015d6cfdc27d4a0dd4` | `tests/durable_delivery_counted_cutover.rs` | Final cutover test only; belongs to P3 because it requires the producer/catalog migration and removal of the old push-template budget symbols. |
| P3-P1 | extract, remove unrelated assertions | `TRACKED_WIP:tests/monitor_help_isolation.rs:312-415` | `ba526e9dc0036670cf68354c17a258b7537a405e2a68a28377239522f0db2346` | same process-test file | Retain BR-194 provider/sink/account preflight isolation; delete source lines 403–412, which assert unrelated BR-183 DB/selection behavior. |
| P3-P2 | exact | `TRACKED_WIP:tests/monitor_help_isolation.rs:417-463` | `0f0ab71a78b0536742949877c444fa499fff803a616eb25c62fbcba069e4fe29` | same process-test file | Parser must reject ordinal override before database/runtime bootstrap. |
| P3-P3 | exact | `TRACKED_WIP:tests/monitor_help_isolation.rs:465-538` | `292b54919a5979f444af4cdf7ff4553b019e6bb4af1d2bd5888adbb77b82b615` | same process-test file | Verified-calendar/date parser isolation. |

## P4 validation authority

| ID | Class | Source path/range | Raw source SHA-256 | Destination | Owner/acceptance boundary |
| --- | --- | --- | --- | --- | --- |
| V1 | new | none | — | `tools/release/check_br194_recovery_focused.sh` | BR-203 closed per-slice argv/test-name/count verifier; no `eval` or shell-command strings. |
| V2 | new | none | — | `tools/release/check_br194_bounded_startup.sh` | BR-203 release-binary-only 30-second readiness/SIGTERM/graceful-exit and watermark verifier. |
| V2A | new | none | — | `tools/release/verify_br194_bounded_startup.py` | Structured no-follow watermark/chain/join authority for the four fixed production locations; shell passes paths as argv and never evaluates generated commands. |
| V3 | adapt whole file | `BASELINE:tools/coverage/check_thresholds.py:1-98` | `0b5c0b4a745e8209b7c5b4a8cc258fe7b4a15c68db9ad9f3b26f6e0d214d301a` | same file | Preserve the ordered baseline tuple `src/{risk,trading,database,data_provider,decision,pipeline,event}/` exactly; add no broad prefixes. Add the ordered exact-file tuple `src/data_gateway/{admission,dragon_tiger,provider_top_n}.rs` and `src/bin/monitor/{durable_delivery_runtime,main,notify,push_templates,review_batch,v14_adapter}.rs`. Enforce global 80, baseline aggregate 95 and exact-file aggregate 95 independently; missing/duplicate/zero-line exact files and floor-lowering overrides fail closed. |
| V4 | new | none | — | `tests/test_coverage_thresholds.rs` | Seven exact tests: `br203_coverage_preserves_exact_baseline_prefix_set`, `br203_coverage_preserves_exact_recovery_core_file_set`, `br203_coverage_rejects_missing_recovery_core_file`, `br203_coverage_rejects_uncovered_file_in_each_baseline_core_prefix`, `br203_coverage_rejects_recovery_core_below_95`, `br203_coverage_enforces_80_95_and_rejects_lower_overrides`, `br203_coverage_empty_core_fails_closed`. Prefix/file-set fixtures report the exact member that failed. |
| V5 | new | none | — | `tests/br203_recovery_verifiers.rs` | Twenty exact process tests: four focused failures (`br203_focused_rejects_zero_match`, `br203_focused_rejects_duplicate_name`, `br203_focused_rejects_unauthorized_ignored`, `br203_focused_rejects_count_drift`); twelve bounded failures (`br203_bounded_rejects_nonliteral_binary_path`, `br203_bounded_rejects_timeout_other_than_30`, `br203_bounded_rejects_missing_startup_banner`, `br203_bounded_rejects_missing_scheduler_banner`, `br203_bounded_rejects_duplicate_readiness_banner`, `br203_bounded_rejects_missing_graceful_shutdown`, `br203_bounded_rejects_nonzero_exit`, `br203_bounded_rejects_unexpected_test_mode`, `br203_bounded_rejects_early_exit_before_sigterm`, `br203_bounded_rejects_authority_watermark_mutation`, `br203_bounded_rejects_unjoined_delivery_audit_growth`, `br203_bounded_rejects_unjoined_sink_result_growth`); two additional orphan-growth failures (`br203_bounded_rejects_unjoined_push_log_growth`, `br203_bounded_rejects_unjoined_immutable_audit_growth`); and two positives (`br203_bounded_accepts_zero_growth_isolated_fixture`, `br203_bounded_accepts_exactly_joined_growth_isolated_fixture`). Fixtures create an isolated literal `./target/release/monitor` and never call a real provider or sink. |

## Destination splice, owner and non-zero test ledger

Test-set abbreviations below are closed. Each source slice first runs the exact
argv/count packet in §Slice-local executable validation packets directly;
P4's `tools/release/check_br194_recovery_focused.sh` later codifies and reruns
the same commands for release closure:

- `CF2`: two exact strict selection-audit caller failure tests;
- `CFV1`: one P2-F structural verifier command registered exactly once in the compliance runner;
- `DD108`: exactly 108 durable-delivery core tests, zero failed and zero
  ignored; this set is first exposed by P2-L0 and is part of P2-C1/P2-F
  acceptance rather than deferred release evidence;
- `AD4`: four exact BR-159 admission tests;
- `DB4`: four exact BR-159 database tests;
- `TN3`: three exact Provider Top-N tests;
- `DT6`: six exact DragonTiger tests;
- `EV4/PR3/DO3/DA9`: exact envelope/push-record/delivery-observation/append
  counts respectively; EV4 and PR3 each include one of the two V2C2 tests;
- `NT26`: 26 direct notify BR-192 tests, including the NC2 compatibility
  subset; its one ignored child helper is
  exercised by a passing parent and is not counted;
- `V2C2/NC2`: two exact schema-v2 golden tests and two exact non-counted
  push-log/sink-call compatibility tests;
- `EB1`: one exact eager-runtime-artifact binding test;
- `R4V4`: four exact durable-runtime R-04 canonical validation tests, all
  runnable in the P2-F candidate before any provider/producer exists;
- `CC1`: one counted-cutover integration test;
- `M31/P3`: 31 monitor BR-194 tests and three BR-194 process tests;
- `WK1/SM3/R96/R4B3/R4S3/CAT1`: respectively one weekend-date caller test,
  three scheduler BR-192 tests, six R-09 renderer/caller tests, three R-04
  BR-162 tests, three R-04 BR-192 tests and one counted-catalog test.
- `V4/V5`: seven coverage-checker tests and twenty recovery-verifier process tests
  with the exact names frozen in rows V4/V5.

`R4V4` is the exact qualified filter
`durable_delivery_runtime::tests::br194_r04_` and contains only
`br194_r04_runtime_revalidates_exact_canonical_schema_and_rendered_text`,
`br194_r04_runtime_rejects_semantically_equal_noncanonical_bytes`,
`br194_r04_runtime_rejects_schema_provider_projection_and_seat_mutations`, and
`br194_r04_envelope_rejects_text_not_bound_by_canonical_hash`. Its accepted
result is exactly four passed, zero failed/ignored/measured/filtered-in-extra.

`M31` is a closed membership set, not merely a substring count. Its seventeen
`durable_delivery_runtime::tests` members are
`br194_r04_runtime_revalidates_exact_canonical_schema_and_rendered_text`,
`br194_r04_runtime_rejects_semantically_equal_noncanonical_bytes`,
`br194_r04_runtime_rejects_schema_provider_projection_and_seat_mutations`,
`br194_r04_envelope_rejects_text_not_bound_by_canonical_hash`,
`br194_terminal_replay_passes_with_equal_authority_watermarks`,
`br194_terminal_replay_sink_eligibility_fails_before_sink`,
`br194_terminal_replay_started_or_failed_cannot_verify`,
`br194_terminal_replay_classification_error_persists_failed_completion`,
`br194_terminal_replay_identity_and_audit_join_are_exact`,
`br194_terminal_replay_trigger_recomputes_canonical_sha256`,
`br194_terminal_replay_audit_uses_none_delivery_attempt_binding`,
`br194_terminal_replay_tables_reject_update_delete_and_second_completion`,
`br194_terminal_replay_rejects_mismatched_completion_decision_and_audit`,
`br194_terminal_replay_start_audit_ack_failure_blocks_classification`,
`br194_terminal_replay_completion_write_or_ack_failure_never_passes`,
`br194_terminal_replay_ordinals_advance_after_dangling_or_failed_attempts`, and
`br194_terminal_replay_cross_connection_contention_allocates_unique_ordinals`.
Its ten `review_batch::tests` members are
`br194_review_task_dependency_mapping`,
`br194_account_tasks_are_frozen_without_real_batch_watermark`,
`br194_account_failure_serializes_exact_transition_audit`,
`br194_legacy_transition_fixture_remains_byte_identical_and_hash_valid`,
`br194_account_failure_full_record_fixture_is_fixed_and_hash_valid`,
`br194_transition_failure_wire_rejects_null_array_unknown_and_nonfailed_payloads`,
`br194_preflight_precedes_dependency_acquisition`,
`br194_time_boundaries_1535_and_2100`,
`br194_review_batch_merge_rejects_duplicate_task`, and
`br194_source_only_runs_before_frozen_account_tasks`. The remaining four are
`v14_adapter::tests::br194_source_only_profile_enforces_real_data_mode_without_changing_default_profile`
and the three exact notify tests in P3-T9 through P3-T11.

The separately counted `R96` members are
`br192_binding_preserves_provider_order_and_source_limited_disclaimer`,
`br192_one_verified_empty_metric_rejects_the_atomic_report`,
`br192_provider_ordinal_mismatch_is_not_resorted_or_silently_accepted`,
`br192_r09_envelope_freezes_both_provider_batches_and_task_binding`,
`br192_r09_reports_delivery_only_after_task_hydration_is_durable`, and
`br192_r09_uncertain_delivery_never_becomes_an_automatic_retry`. `R4B3` is
`br162_r04_production_dispatcher_uses_unified_gateway_only`,
`br162_r04_renderer_keeps_trade_ids_and_exact_seats_without_fake_sum`, and
`br162_r04_preserves_wait_empty_and_unavailable_outcomes`. `R4S3` is
`br192_r04_counted_binding_fails_closed_without_exact_provider_evidence`,
`br192_r04_dispatch_uses_only_explicit_counted_binding`, and the new exact
P3-T6A name. `SM3` is the two `br192_main_caller_*` tests plus
`br192_startup_reconciles_before_active_r09_runner_and_passes_one_context`.

## Slice-local executable validation packets

These packets are normative before P4 exists. Every command is executed as
the literal argv shown (no `eval`, `bash -c` or generated shell text), must exit
zero, and must select exactly the stated passed/ignored count. Zero matches,
one extra selected test, one failed test or any unexpected ignored test is a
slice failure. P4 V1 must encode these same packets without changing them.

### P2-F compile-foundation packet

```text
cargo metadata --locked --offline --format-version 1 --no-deps               => exit 0
cargo check --locked --lib                                                   => exit 0
cargo check --locked --bin monitor                                           => exit 0
cargo fmt --all -- --check                                                   => exit 0
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings => exit 0, zero warnings
cargo test --locked --lib durable_delivery:: -- --test-threads=1             => 108 passed, 0 ignored
cargo test --locked --lib event::envelope::tests::br192_ -- --test-threads=1 => 4 passed, 0 ignored
cargo test --locked --lib event::push_record::tests::br192_ -- --test-threads=1 => 3 passed, 0 ignored
cargo test --locked --lib event::delivery_observation_tests::br192_ -- --test-threads=1 => 3 passed, 0 ignored
cargo test --locked --lib event::durable_delivery_append::tests::br192_ -- --test-threads=1 => 9 passed, 0 ignored
cargo test --locked --bin monitor notify::tests::br192_ -- --test-threads=1 => 26 passed, 1 ignored child helper
cargo test --locked --bin monitor durable_delivery_runtime::tests::br194_r04_ -- --test-threads=1 => 4 passed, 0 ignored
cargo test --locked --bin monitor durable_delivery_runtime::tests::br192_main_eagerly_binds_runtime_artifacts_exactly_once_before_sink_init -- --exact --test-threads=1 => 1 passed, 0 ignored
cargo test --locked --lib selection::outcome::br203_production_audit_open_failure_preserves_code_and_stops_before_due_load -- --exact --test-threads=1 => 1 passed, 0 ignored
cargo test --locked --lib selection::pipeline::br203_production_audit_open_failure_returns_unavailable_before_dependencies -- --exact --test-threads=1 => 1 passed, 0 ignored
cargo test --locked --test monitor_help_isolation -- --test-threads=1         => 19 passed, 0 ignored; the unknown-flag, invalid/corrupt-outcome-backfill and registered-backfill regressions leave the exact fixed production SQLite trio absent or unchanged
bash tools/compliance/lib/check_br203_compile_foundation.sh                    => exit 0 and exactly one runner registration
cargo test --locked --workspace --all-targets --all-features -- --test-threads=1 => exit 0, zero failed and exactly the authorized 16-test ignored whitelist below
```

The exact P2-F ignored whitelist is closed and contains sixteen names. Six are
manifest-owned child helpers exercised by passing parents:

1. `event::durable_delivery_append::tests::TEST_CODE_br192_immutable_append_namespace_child`
2. `event::dispatcher::tests::br141_event_audit_process_writer_helper`
3. `durable_delivery_runtime::tests::TEST_CODE_br192_real_full_chain_child`
4. `durable_delivery_runtime::tests::TEST_CODE_br192_runtime_foreign_cwd_child`
5. `review_batch::tests::br140_review_audit_process_writer_helper`
6. `notify::tests::br192_push_log_process_writer_helper`

Ten are explicit live/external integrations, never silent unit-test skips:

1. `fallback_sina_test::fallback_returns_data_with_sina_in_chain`
2. `fallback_sina_test::sina_provider_direct_fetch_works`
3. `v11_three_sources::fallback_returns_consistent_source_and_adjust`
4. `v11_three_sources::fallback_returns_sane_prices`
5. `v11_three_sources::fallback_skips_data_with_extreme_gap`
6. `fallback_post_close_test::post_close_prefers_baostock`
7. `fallback_post_close_test::baostock_provider_direct_fetch_works`
8. `llm::ticker_extractor::tests::test_extract_tickers_real_api`
9. `push_l6::external_sinks::tests::wechat_sink_real_http_fails_without_server`
10. `push_l6::external_sinks::tests::feishu_sink_skeleton_returns_err`

P4 must compare the harness inventory to these exact names. A seventeenth
ignored test, a missing parent exercise for a child helper, or converting a
failed test into `#[ignore]` is a release-blocking count drift.

### Remaining P1 source packet

This packet is ineligible until a separately reviewed BR-164
dependency-identity prerequisite has been committed and proved compile-green.
That prerequisite is outside this manifest and must not reuse
REJECTED-P1-A2 through REJECTED-P1-A5. P1 staging stops if the prerequisite's
exact Cargo/lock authority is absent, fails its rollback contract, or produces
a second `magic-market-core` package identity.

```text
cargo check --locked --lib => exit 0
cargo check --locked --bin monitor => exit 0
cargo fmt --all -- --check => exit 0
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings => exit 0, zero warnings
cargo test --locked --lib data_gateway::admission::tests::br159_ -- --test-threads=1 => 4 passed, 0 ignored
cargo test --locked --lib database::data_acquisition_audit::tests::br159_ -- --test-threads=1 => 4 passed, 0 ignored
cargo test --locked --lib data_gateway::provider_top_n::tests::br192_ -- --test-threads=1 => 3 passed, 0 ignored
cargo test --locked --lib data_gateway::dragon_tiger::tests::br162_ -- --test-threads=1 => 6 passed, 0 ignored
cargo test --locked --workspace --all-targets --all-features -- --test-threads=1 => exit 0, zero failed and zero unexpected ignored
```

### P3 producer packet

```text
cargo check --locked --lib => exit 0
cargo check --locked --bin monitor => exit 0
cargo fmt --all -- --check => exit 0
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings => exit 0, zero warnings
cargo test --locked --bin monitor durable_delivery_runtime::tests::br194_ -- --test-threads=1 => 17 passed, 0 ignored
cargo test --locked --bin monitor review_batch::tests::br194_ -- --test-threads=1 => 10 passed, 0 ignored
cargo test --locked --bin monitor v14_adapter::tests::br194_source_only_profile_enforces_real_data_mode_without_changing_default_profile -- --exact --test-threads=1 => 1 passed, 0 ignored
cargo test --locked --bin monitor notify::tests::br194_r04_source_only_ -- --test-threads=1 => 3 passed, 0 ignored
cargo test --locked --test monitor_help_isolation br194_ -- --test-threads=1 => 3 passed, 0 ignored
cargo test --locked --bin monitor push_templates::br192_provider_top_n_tests::br192_ -- --test-threads=1 => 6 passed, 0 ignored
cargo test --locked --bin monitor push_templates::tests_r_dispatchers::br162_r04_ -- --test-threads=1 => 3 passed, 0 ignored
cargo test --locked --bin monitor push_templates::tests_r_dispatchers::br192_r04_ -- --test-threads=1 => 3 passed, 0 ignored
cargo test --locked --bin monitor tests_post_session_review_scheduler::br192_main_caller_ -- --test-threads=1 => 2 passed, 0 ignored
cargo test --locked --bin monitor tests_post_session_review_scheduler::br192_startup_reconciles_before_active_r09_runner_and_passes_one_context -- --exact --test-threads=1 => 1 passed, 0 ignored
cargo test --locked --bin monitor tests_post_session_review_scheduler::br140_weekend_manual_review_uses_the_latest_completed_trading_day -- --exact --test-threads=1 => 1 passed, 0 ignored
cargo test --locked --bin monitor push_templates::tests::counted_kinds_bypass_process_local_cooldown -- --exact --test-threads=1 => 1 passed, 0 ignored
cargo test --locked --test durable_delivery_counted_cutover -- --test-threads=1 => 1 passed, 0 ignored
cargo test --locked --workspace --all-targets --all-features -- --test-threads=1 => exit 0, zero failed and zero unexpected ignored
```

| ID | Destination splice anchor after implementation | Owner | Required test set |
| --- | --- | --- | --- |
| P0-A1 | one new BR-203 row; every frozen active-ledger row byte-identical | BR-203 | Rule-2.10 + whole-file/hash proof |
| P0-A2 | amend only BR-203 ordering plus the two Gate-A recovery docs | BR-203 | docs-only tree/path proof + two independent Gate-A reviews |
| P0-A3 | amend only BR-203 P2-F/BR-164 ownership plus the two Gate-A recovery docs | BR-203 | docs-only tree/path proof + two independent Gate-A reviews |
| P2-C1 | exact minimal `Cargo.toml` target | BR-203 | compile + `CFV1+DD108` |
| P2-C2 | exact minimal `Cargo.lock` target | BR-203 | metadata + `CFV1` |
| P2-SA1 | strict production audit binding in `selection/outcome.rs` | BR-203 | `CF2+CFV1` |
| P2-SA2 | strict production audit binding in `selection/pipeline.rs` | BR-203 | `CF2+CFV1` |
| P2-H1 | `#[cfg(test)]` compile hygiene in `selection/audit.rs` | BR-203 | strict Clippy + full workspace tests |
| P2-H2 | durable schema tuple-alias compile hygiene | BR-203 | strict Clippy + `DD108` + full workspace tests |
| P2-H3 | self-contained durable migration/foreign-CWD TEST_CODE fixtures | BR-203 | `DD108` + exact foreign-CWD test + full workspace tests |
| P2-H4 | tracked clean-checkout `data/` namespace sentinel | BR-203 | `DD108` + no production SQLite artifact growth |
| P2-H5 | real source-protocol immutable append compile closure | BR-203 | exact hash-chain append test + strict Clippy |
| P2-H6 | immutable strict-review run-context caller closure | BR-203 | exact context/preflight tests + strict Clippy |
| P2-H7 | TEST_CODE-only gates for future P3 surfaces | BR-203 | strict all-target Clippy + admitted monitor test packets |
| P2-H8 | fail-closed eager-bound capability invariant | BR-203 | exact invariant test + `EB1` + strict Clippy |
| P2-H9 | production-delivery observer TEST_CODE isolation | BR-192/BR-203 | exact BR-130 global-observer regression + full workspace tests |
| P2-H10 | BR-195 business-date-bound chain appearance adoption | BR-195/BR-203 | exact database lower/upper-bound tests + both caller tests + full workspace tests |
| P2-H11 | typed counted-governance provenance discriminator | BR-192/BR-203 | exact generic rejection + explicit TEST_CODE binding regressions + full workspace tests |
| P2-H12 | one serial domain for every monitor TEST_CODE namespace/environment mutator | BR-051/BR-136/BR-192/BR-203 | default-thread and serial monitor suites + strict Clippy |
| P2-H13 | production CLI TEST_CODE bootstrap before BR-144 preflight plus process isolation | BR-051/BR-192/BR-203 | 19-case `monitor_help_isolation` suite + fixed-production-artifact fingerprint proof |
| P2-V0A | fail-closed P2-F checker | BR-203 | `CFV1` + compliance |
| P2-V0B | structured P2-F verifier | BR-203 | `CFV1` + compliance |
| P2-V0C | exact-one compliance-runner registration | BR-203 | `CFV1` + compliance |
| P1-AD1 | `BatchEvidence` definition/constructor | BR-159 | `AD4` |
| P1-AD2 | `GatewayBatch<T>` definition/impl | BR-159 | `AD4` |
| P1-AD3 | `GatewayError` definition/impl | BR-159 | `AD4` |
| P1-AD4 | canonical request hash and audit helpers | BR-159 | `AD4` |
| P1-ADT1 | provider mismatch fail-closed test | BR-159 | `AD4` |
| P1-ADT2 | repository unavailable fail-closed test | BR-159 | `AD4` |
| P1-ADT3 | audit append failure fail-closed test | BR-159 | `AD4` |
| P1-ADT4 | calling-capability attribution test | BR-159 | `AD4` |
| P1-TN1 | Provider Top-N capability constants | BR-192 | `TN3` |
| P1-TN2 | Eastmoney source constant | BR-192 | `TN3` |
| P1-TN3 | frozen provider limit constant | BR-192 | `TN3` |
| P1-TN4 | Provider Top-N DTO declarations | BR-192 | `TN3` |
| P1-TN5 | `ProviderTopNDataGateway::pair` | BR-192 | `TN3` |
| P1-TN6 | request constructor/evidence helper | BR-192 | `TN3` |
| P1-TN7 | private atomic pair/router helper | BR-192 | `TN3` |
| P1-TN8 | pair admission validator | BR-159/BR-192 | `AD4+TN3` |
| P1-TN9 | private ISO date helper | BR-192 | `TN3` |
| P1-TN10 | provider/router/join error mapping | BR-159/BR-192 | `AD4+TN3` |
| P1-TNT1 | Provider Top-N fixture module | BR-192 | `TN3` |
| P1-TNT2 | typed/order preservation test | BR-192 | `TN3` |
| P1-TNT3 | rejection/atomic pair tests | BR-192 | `TN3` |
| P1-DT1 | `DragonTigerGateway` production module | BR-162 | `AD4+DT6` |
| P1-DTT1 | DragonTiger test module | BR-162 | `DT6` |
| P1-DB1 | whole `data_acquisition_audit.rs` | BR-159 | `DB4` |
| P1-G1 | `src/lib.rs` `data_gateway` module declaration | BR-159 | `AD4+TN3+DT6` |
| P1-G2 | database module declaration | BR-159 | `DB4` |
| P1-G3 | BR-159 schema installation call only | BR-159 | `DB4` |
| P1-G4 | new three-module Gateway root/re-exports | BR-159/BR-162/BR-192 | `AD4+TN3+DT6` |
| P2-E1 | whole `event/envelope.rs` controlled target: immutable preimage plus only T4 | BR-192 | `EV4+V2C2` |
| P2-E2 | whole `event/mod.rs` | BR-192 | `DO3` |
| P2-E3 | whole `event/push_record.rs` controlled target: E3A plus only T5 | BR-192 | `PR3+V2C2` |
| P2-E3A | exact eight-field schema-v2 omission compatibility hunk | BR-192 | `PR3+V2C2` |
| P2-E4 | whole `event/durable_delivery_append.rs` | BR-192 | `DA9` |
| P2-L0 | crate-root durable-delivery module declaration only | BR-192 | compile + `DD108+EV4+PR3+DA9` |
| P2-N1 | `notify.rs` Unix import/header anchor | BR-192 | `NT26` |
| P2-K1 | `PushKind::ReviewProviderTopN` compile-closure variant | BR-192 | compile + `CAT1` deferred to P3 |
| P2-K2 | `PushKind::level` exhaustive arm | BR-192 | compile |
| P2-K3 | `PushKind::requires_banner` exhaustive arm | BR-192/BR-194 | compile + `M31` deferred to P3 |
| P2-K4 | `PushKind::cooldown_secs` exhaustive arm | BR-192 | compile + `CAT1` deferred to P3 |
| P2-K5 | `PushKind::label` exhaustive arm | BR-192 | compile + `R96` deferred to P3 |
| P2-N0 | test-only annotation for replaced legacy writer | BR-192 | clippy + `NT26` |
| P2-N2 | `PushLogError` through secure generic writer | BR-192 | `NT26` |
| P2-N2A | generic governor counted-kind rejection | BR-192 | `NT26` |
| P2-N2B | legacy delivery counted-kind rejection | BR-192 | `NT26` |
| P2-N3 | generic counted binding entry | BR-192 | `NT26` |
| P2-N3A | namespace-aware real `save_push_log` call sites plus closed `cfg(test)` sink-boundary spy mode | BR-192 | `NT26+NC2` |
| P2-N4 | authoritative adapter/finalizer/receipt wrapper | BR-192 | `NT26` |
| P2-M0 | monitor durable-runtime module declaration | BR-192 | compile + `EB1` |
| P2-R4V1 | pure typed R-04 canonical validation/test fixture seam | BR-192 | compile + `R4V4`; no producer |
| P2-M1 | startup eager bind before sink initialization | BR-192 | `EB1` |
| P2-M2 | test-runtime invocation namespace isolation | BR-192 | `NT26+NC2` |
| P2-T0 | notify test fixture declarations | BR-192 | `NT26` |
| P2-T1 | existing notify test fixtures anchor | BR-192 | `NT26` |
| P2-T1A | notify test fixture implementations | BR-192 | `NT26` |
| P2-T2 | existing counted authority tests anchor | BR-192 | `NT26` |
| P2-T3 | existing dry-run/rejection tests anchor | BR-192 | `NT26` |
| P2-T4 | new schema-v2 publication-byte golden test | BR-192 | `V2C2` |
| P2-T5 | new schema-v2 parser-output golden test | BR-192 | `V2C2` |
| P2-T6 | new non-counted dry-run artifact/zero-sink golden test | BR-192 | `NC2` |
| P2-T7 | new non-counted live artifact/one-spy-sink golden test | BR-192 | `NC2` |
| P3-N1 | dedicated SourceOnly notify entry | BR-194 | `M31+P3` |
| P3-M0 | startup durable reconciliation fixed-point barrier before activation | BR-192/BR-194 | `SM3` |
| P3-M1 | replay parser/command before bootstrap | BR-194 | `M31+P3` |
| P3-M2 | first statement block of existing `main` | BR-194 | `M31+P3` |
| P3-M3 | hydration helper block | BR-194 | `M31` |
| P3-M4 | complete `run_review_only` replacement | BR-194 | `M31+P3` |
| P3-M5 | strict inner runner replacement | BR-194 | `M31` |
| P3-M6 | `attempt_post_session_review` replacement | BR-194 | `M31` |
| P3-M7 | scheduler post-batch hydration block | BR-194 | `M31` |
| P3-R4-1 | R-04 DTO renderer helper block | BR-162/BR-194 | `DT6+M31+R4B3+R4S3` |
| P3-R9-1 | R-09 DTO/renderer/binding/outcome block | BR-192/BR-194 | `TN3+M31+R96` |
| P3-R9-2 | R-09 production wrapper body | BR-192/BR-194 | `TN3+M31+R96` |
| P3-G1 | canonical post-session dispatcher body | BR-194 | `M31+P3` |
| P3-R4-2 | R-04 preparation/dispatch block after P2 validator | BR-162/BR-194 | `DT6+M31+R4B3+R4S3` |
| P3-T0 | scheduler test `sha2` import | BR-194 | `SM3` |
| P3-T1 | scheduler hydration test helper block | BR-194 | `SM3` |
| P3-T2 | immutable run-context/weekend-date test | BR-194 | `WK1` |
| P3-T3 | hydration application tests | BR-194 | `SM3` |
| P3-T4 | production source/startup inspection test | BR-194 | `SM3` |
| P3-T5 | adapted R-09 test module | BR-192/BR-194 | `TN3+R96` |
| P3-T6 | four preserved dispatcher tests | BR-194 | `R4B3+R4S3` |
| P3-T6A | new syntax-aware SourceOnly-call regression | BR-194 | `R4S3` |
| P3-T7 | R-04 loader/typed-outcome tests | BR-162/BR-194 | `DT6+R4B3+R4S3` |
| P3-T8 | shared notify SourceOnly fixtures | BR-194 | `M31` |
| P3-T9 | SourceOnly gate-never-reads-banner test | BR-194 | `M31` |
| P3-T10 | SourceOnly L5/durable preservation test | BR-194 | `M31` |
| P3-T11 | denied-launch zero-durable/zero-sink test | BR-194 | `M31` |
| P3-T12 | counted catalog increment | BR-192/BR-194 | `CAT1` |
| P3-CC1 | final counted-cutover integration file | BR-192/BR-194 | `CC1` |
| P3-P1 | first BR-194 process test body | BR-194 | `P3` |
| P3-P2 | ordinal parser process test | BR-194 | `P3` |
| P3-P3 | calendar/date parser process test | BR-194 | `P3` |
| V1 | new focused verifier script | BR-159/BR-162/BR-192/BR-194/BR-203 | all sets above with exact counts + V5 |
| V2 | new bounded release-startup verifier | BR-203 | V5 |
| V2A | new structured watermark/chain/join verifier | BR-192/BR-203 | V5 |
| V3 | whole coverage checker adaptation | BR-203 | V4 |
| V4 | new coverage checker regression file | BR-203 | V4 |
| V5 | new verifier process-fixture file | BR-203 | V5 |

For `exact` rows the destination target hash must equal the listed raw source
hash. Every exact partial-file row and every extract/adapt/new row must replace
the conceptual anchor above with a literal destination line range and
target-hunk SHA-256 before staging. That ledger is reviewed with at least 80
lines of context and is a blocking input to each slice commit.

## Explicitly excluded `notify.rs` bytes

- No whole-file admission of blob `6a2240def9e5e8ef4a0bd48ceb691ac948eb321a`.
- All WIP changes to token issuance/cache, daemon startup/authentication, generic
  HTTP/CLI transport, target discovery, unrelated PushKinds and BR-160 or
  BR-197–200 behavior are excluded.
- The fixed baseline helper definitions for `MessageSendType`,
  `MessageSendTransport`, `CliDeliveryReceipt`, receipt parsing and target/bin/
  home resolution remain authoritative unless an exact focused compile test
  proves a dependency contradiction and returns the design to Gate A.

## Rejected partial P2 focused evidence

The commands below were reported from an earlier reconstructed candidate
before P2-M0 exposed the full unchanged durable runtime and before P2-R4V1 and
the complete P2-F dependency/caller closure were enumerated. They are preserved only
as historical partial evidence and **must not** be cited as current Gate-B or
release proof. The complete composite candidate must rerun every filter plus
`cargo check --lib` and the monitor compile from a clean target before staging.
The earlier partial results were:

```text
cargo test --lib event::envelope::tests::br192_ -- --test-threads=1
test result: ok. 3 passed; 0 failed; 0 ignored; 2314 filtered out

cargo test --lib event::push_record::tests::br192_ -- --test-threads=1
test result: ok. 2 passed; 0 failed; 0 ignored; 2315 filtered out

cargo test --lib event::delivery_observation_tests::br192_ -- --test-threads=1
test result: ok. 3 passed; 0 failed; 0 ignored; 2314 filtered out

cargo test --lib event::durable_delivery_append::tests::br192_ -- --test-threads=1
test result: ok. 9 passed; 0 failed; 0 ignored; 2308 filtered out

cargo test --bin monitor notify::tests::br192_ -- --test-threads=1
test result: ok. 24 passed; 0 failed; 1 ignored; 501 filtered out
```

The ignored notify helper is invoked as an isolated child by the passing
cross-process test; it is not counted as a twenty-fifth direct pass. None of
these counts closes the missing Cargo feature/direct dependency/R-04 validator
seam identified by independent review.

## Remaining manifest gates

- P0/P1/P2/P3 historical source ranges and replacement target contracts are
  enumerated; no unlisted historical byte is admitted and the rejected first
  candidate blobs have no authority.
- P0-A1 and its docs-only P0-A2/P0-A3 corrections must be committed and independently
  accepted before the P2-F transition; the P0-A1 target keeps every row from the frozen active-ledger
  preimage byte-identical, including the active BR-159/BR-192/BR-194 amendments, and adds only
  BR-203, while P0-A2 changes only that row's compile-closure ordering plus the
  two recovery docs, and P0-A3 changes only those same paths to record the
  rejected uncommitted whole-Cargo proposal and bind the minimal P2-F target
  plus the deferred BR-164 dependency ownership. The historical baseline rows remain extraction evidence only and are not a target.
- `P0-M0`, direct-child `P0-A1`, direct-child `P0-A2` and direct-child `P0-A3` must be materialized
  as real Git commit objects. Independent review must prove both exact
  parent/child tree relations and report `C0/I0/M0` before any Gate-B source
  edit.
- Before each accepted slice is staged, exact rows must be reverified, every
  partial-file row must gain a literal destination range and target hunk hash,
  and extract/adapt/new rows must gain the same in the implementation ledger;
  the slice-local executable packet above must prove every declared test
  filter is non-zero with the exact frozen count. P4 V1 later codifies and
  reruns those same packets; it is not a prerequisite file for earlier slices.
