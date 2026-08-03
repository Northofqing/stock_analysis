# BR-192/BR-194 incomplete-commit recovery hunk manifest

**Status:** Gate A repair; first frozen target candidates rejected

This manifest binds recovery work to immutable Git objects. Source ranges are
one-based inclusive ranges in the named object and every listed SHA-256 is over
the exact raw range bytes emitted by `git show`; it is not a normalized target
hunk hash. A destination is never allowed to copy bytes outside a listed
range. `exact` means byte-for-byte admission; `extract` means the source is
evidence for a smaller reviewed deep module and must not be represented as a
byte recovery; `new` means accepted target work with no immutable source
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

## Rejected first P1 candidate packet

The former candidate blobs `ec52754ace19f5e09341416abd37c4876963943e`,
`17e4ff819323d1126f434875d4098681578243c8`,
`24b2e7d0e4d912404213ad23a1abdf62792b5ad3` and
`e79ac5a5d159e8cb534fd9778c5043dd65935f50` are rejected evidence. They imported later rules, unrelated
dependency/Profile/Polars changes, an over-broad lockfile and five test paths
absent from `BASELINE`. They are not admitted target authority.

## P0 documentation-only Rule-2.10 registration

P0 runs before every source slice. `P0-M0` first materializes the immutable active-rule-ledger
preimage plus the recovery design, this manifest and the preserved BR-204 Gate-A design in one
docs-only commit tree. `P0-A1` is its direct child and changes only the rule ledger by adding BR-203.
The historical
`BASELINE:docs/business_rules.md` Git blob
`a5325bdfb381ed187f1acbf70819260f38e18646` (SHA-256
`2c1d3634b38649ecb804a525bc896db0c9989eab9903dd54fc3ba1e7b0a312b9`) remains extraction evidence
only and must not overwrite later rule-ledger additions or amendments.

| ID | Class | Fixed preimage | Destination/anchor | Owner | Exact acceptance |
| --- | --- | --- | --- | --- | --- |
| P0-M0 | docs-only authority materialization | current implementation parent plus exact frozen active-ledger preimage | materialize active ledger without BR-203, recovery design, this manifest and preserved BR-204 Gate-A design | BR-203 | no source/config/runtime path changes; the ledger equals its frozen preimage hash and all three document blobs are committed |
| P0-A1 | new additive docs-only rule row | exact committed `P0-M0` tree | insert canonical BR-203 row; its Code cell names only this recovery design and companion manifest | BR-203 | `P0-A1` is a direct child of `P0-M0`; every pre-existing active-ledger row and every non-ledger path remains byte-identical; pre/post hashes prove BR-203 is the only child-commit semantic addition; historical baseline is not restored |

P0 changes no source, configuration, dependency, lockfile or test. The frozen
active-ledger preimage without BR-203 has SHA-256
`9e149cc950c40976fe10a8a4f0f43d8e70a984b5c83595123e4f52e813a52c96`; the literal BR-203 row has
SHA-256 `d89a3ecbad17a99b60201c71515c46c88235c7cde168db6c9e0eaaa8fc9a2ce5`; the complete target has
Git blob `138a6725eb53420f3dfad3ba3ed086618be9873b` and SHA-256
`bf0955f05dd792fc7088a29a7197910d7b9cecc6d65cbc1e33f1932876355666`. Any change outside the
single BR-203 row between the frozen preimage and target is a hard failure.

The replacement P1 target contract is baseline-derived and has no prewritten
code blob at Gate A:

| ID | Class | Fixed preimage | Destination/anchor | Owner | Exact acceptance |
| --- | --- | --- | --- | --- | --- |
| P1-A2 | new minimal manifest delta | `BASELINE:Cargo.toml` | dependency entries plus the adjacent stale path-ownership comment only | BR-203 | preserve all unrelated bytes and Polars 0.46; exact named 14 direct `=0.2.0`/`5f1ce93656a55854c844065390520cd4aecd9a14`; `rusqlite=chrono,functions`; no path dependency |
| P1-A3 | generated closed lock delta | `BASELINE:Cargo.lock` + accepted P1-A2 | exact target hash and changed package-record whitelist, frozen before staging | BR-203 | exactly named 15 Magic packages at version/revision; no open resolver allowance or unlisted non-Magic record |
| P1-A4 | new narrow test | no historical source | `tests/magic_market_release_revision.rs` | BR-203 | reads only `Cargo.toml`/`Cargo.lock`; proves exact 14/15 closure, root preservation, no sibling path and no rejected revision |
| P1-A5 | new checker + one registration hunk | no historical source | `tools/compliance/lib/check_br203_magic_dependencies.sh`; `tools/compliance/check.sh` | BR-203 | binds fixed input/generated target hashes and exact changed-record whitelist; reruns exact 14/15/root/Polars/no-path/no-old-revision checks and `cargo metadata --locked --offline` |

P1-A2, P1-A3, P1-A4 and P1-A5 form one indivisible implementation transition.
P0-A1 is the preceding documentation-only additive BR-203 registration required by
Rule 2.10 and must be independently accepted before source staging. Target hunk
hashes are added after these minimal bytes are generated in the isolated
branch and before either commit is staged.

The direct set is exactly `magic-baidu-rs`, `magic-cls-rs`,
`magic-cninfo-rs`, `magic-eastmoney-rs`, `magic-exchange-rs`,
`magic-jin10-rs`, `magic-market-composition`, `magic-market-core`,
`magic-market-router`, `magic-sina-rs`, `magic-tdx-rs`, `magic-tencent-rs`,
`magic-thepaper-rs`, and `magic-ths-rs`. The lock set is exactly those fourteen
plus `magic-market-transport`. All fifteen use version `0.2.0`, repository
`https://github.com/Northofqing/magic-market-data-rs.git`, and revision
`5f1ce93656a55854c844065390520cd4aecd9a14`. The fixed preimage identities are
`Cargo.toml` blob `2118a3e490efe2d3416b2554559ca0347947c533` / SHA-256
`521c3b24795288ddce453e714a74e23fe96afe348dfa49c5d68681f0fdf2adfa`
and `Cargo.lock` blob `95481362e8061a1724cd1682d23b4e8a14f16377` / SHA-256
`cd86df085943a710c17ec2cb5aceaef0acc0bde949443dce3fe802e99fbe74fd`.
The generated target hashes and the complete changed-record whitelist must be
equal the frozen values below before P1 staging.

This 14-direct/15-lock set is the approved project-wide root release closure.
Only `magic-eastmoney-rs`, `magic-market-composition`, `magic-market-core` and
`magic-market-router` may be imported by the new P1 R-04/R-09 modules. The
remaining ten dependencies retain existing independently owned consumers and
do not widen P1 source authority; `provider_top_n.rs` specifically must not
import `magic-exchange-rs`.

Lock generation is unique and reproducible. In an isolated
`BASELINE`-derived worktree, apply only P1-A2, prove `cargo --version` is the
recorded Cargo 1.95.0 toolchain, and invoke exactly once:

```bash
cargo update -p url@2.5.8 -p rustls@0.23.37 -p time@0.3.47
```

No `cargo generate-lockfile`, bare `cargo update`, second resolver invocation
or online metadata is target authority. The resulting files must match the
frozen hashes and package-record whitelist below; the compliance proof is
`cargo metadata --locked --offline --format-version 1`.

The isolated Cargo 1.95.0 target SHA-256 values are:

- `Cargo.toml`:
  Git blob `2d0280252f45354cde87aa34140d4050d91feb58`, SHA-256
  `11c3b3914089c29e0b10f0bdbc9be1e55ae65a2d77f6ae251624860ad052c877`;
- `Cargo.lock`:
  Git blob `c327c468d825f46f16c205c803d61caa019fed1d`, SHA-256
  `cb2460bc9872143891efdf5c2df8e17318c6cae5210d3c1861e68416626c1935`.

The target lock has exactly 34 added identities: the fifteen named Magic
records above, each with source
`git+https://github.com/Northofqing/magic-market-data-rs.git?rev=5f1ce93656a55854c844065390520cd4aecd9a14#5f1ce93656a55854c844065390520cd4aecd9a14`,
plus this exact non-Magic identity/checksum whitelist:

| Package identity | Checksum |
| --- | --- |
| `combine 4.6.7` | `ba5a308b75df32fe02788e748662718f03fde005016435c444eea572398219fd` |
| `jni 0.22.4` | `5efd9a482cf3a427f00d6b35f14332adc7902ce91efb778580e180ff90fa3498` |
| `jni-macros 0.22.4` | `a00109accc170f0bdb141fed3e393c565b6f5e072365c3bd58f5b062591560a3` |
| `jni-sys 0.4.1` | `c6377a88cb3910bee9b0fa88d4f42e1d2da8e79915598f65fb0c7ee14c878af2` |
| `jni-sys-macros 0.4.1` | `38c0b942f458fe50cdac086d2f946512305e5631e720728f2a61aabcd47a6264` |
| `num-conv 0.2.2` | `521739c6d2bac4aa25192232afe6841231376b2b26d4d9fae5ecf8ca5772e441` |
| `reqwest 0.13.4` | `219c5811de6525e5416c7d5d53bb656d3afdbc6c5af816e0802bcfa42dbdc1c3` |
| `rustls 0.23.42` | `3c54fcab019b409d04215d3a17cb438fd7fbf192ee61461f20f4fe18704bc138` |
| `rustls-platform-verifier 0.7.0` | `26d1e2536ce4f35f4846aa13bff16bd0ff40157cdb14cc056c7b14ba41233ba0` |
| `rustls-platform-verifier-android 0.1.1` | `f87165f0995f63a9fbeea62b64d10b4d9d8e78ec6d7d51fb2125fda7bb36788f` |
| `simd_cesu8 1.2.0` | `11031e251abf8611c80f460e19dbdeb54a66db918e49c65a7065b46ac7aec520` |
| `time 0.3.54` | `3e1d5e639ff6bab73cb6885cc7e7b1de96c3f32c68ec55f3952614bec1092244` |
| `time-core 0.1.9` | `9e1c906769ad99c88eaa54e728060edef082f8e358ff32030cb7c7d315e81109` |
| `time-macros 0.2.32` | `7e689342a48d2ea927c87ea50cabf8594854bf940e9310208848d680d668ed85` |
| `ureq 2.12.1` | `02d1a66277ed75f640d608235660df48c8e3c19f3b4edb6a263315626cc3c01d` |
| `url 2.5.4` | `32f8b686cadd1473f4bd0117a5d28d36b1ade384ea9b5069a1c40aefed7fda60` |
| `webpki-root-certs 1.0.9` | `b96554aa2acc8ccdb7e1c9a58a7a68dd5d13bccc69cd124cb09406db612a1c9b` |
| `webpki-roots 0.26.11` | `521bc38abb08001b01866da9f51eb7c5d647a19260e00054a8c7fd5f9e57f7a9` |
| `webpki-roots 1.0.9` | `7dcd9d09a39985f5344844e66b0c530a33843579125f23e21e9f0f220850f22a` |

The exact eight removed identities are path/no-source `magic-market-core
0.2.0`, path/no-source `magic-tdx-rs 0.2.0`, `num-conv 0.2.1`, `rustls
0.23.37`, `time 0.3.47`, `time-core 0.1.8`, `time-macros 0.2.27` and `url
2.5.8`. The exact seven same-identity record changes are `deranged 0.5.8`
(drop its lock-record `powerfmt` dependency edge), `hyper-rustls 0.27.7`,
`quinn 0.11.11`, `quinn-proto 0.11.16`, `reqwest 0.12.28` and `tokio-rustls
0.26.4` (each of those five changes only the dependency edge from `rustls
0.23.37` to `rustls 0.23.42`), plus `stock_analysis 0.1.2` (retain every
baseline direct dependency, replace the Magic TDX path identity and add the
other thirteen direct Magic identities). Any different count, identity,
checksum, source, dependency edge or root dependency is a hard failure.

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
| P2-N3A | controlled adaptation | `TRACKED_WIP:src/bin/monitor/notify.rs:2491-2523` | `ce4baff2fb6ef8254bbdd845e0bf849fb9ce6fb16d7b45143fbc898869cd442f` | replace fixed-baseline `push_wechat` prefix and add only the closed P2-T7 `#[cfg(test)]` mode seam at its existing sink boundary | Preserve namespace binding and both real `save_push_log` calls byte-semantically; production wrapper always selects the existing production mode. Test-only spy mode is unconstructable outside `cfg(test)` and performs no token/daemon/transport/sink resolution. Freeze the complete adapted target hunk before staging. |
| P2-N4 | exact | `TRACKED_WIP:src/bin/monitor/notify.rs:2713-3519` | `a7b4e069c77620e25da4ac016fb0845520a862addc64bb76cca08befdd88d027` | `src/bin/monitor/notify.rs` authoritative adapter | Complete sink adapter, pending/audit/commit finalization, exact terminal joins and blocking CLI receipt wrapper. It depends on the fixed baseline's existing send-type/transport/target/bin/home/receipt-parser helpers; those unrelated helper rewrites are excluded. |
| P2-M0 | exact compile closure | `TRACKED_WIP:src/bin/monitor/main.rs:155-155` | `7f9ea27a99daa0639cf90e31a6712fb7211baf4ea5a366f7a260c9c7834b418a` | monitor module-declaration block | Add `mod durable_delivery_runtime;`; the existing module file remains byte-identical to `BASELINE` Git blob `a635b90237413577a51d5bc92ae29c40ae2afac4`. |
| P2-M1 | exact | `TRACKED_WIP:src/bin/monitor/main.rs:3359-3365` | `0769047e84502e3d51fa9892b1f38aa79824606d3183661b40d43070cb2ec898` | after BR-144 audit preflight and before sink initialization | Eagerly bind runtime audit/push-log artifacts before any production delivery path. |
| P2-T0 | exact | `TRACKED_WIP:src/bin/monitor/notify.rs:4668-4670` | `4831d7e30bf2d01efb752a537c7735d0c22149c6d347ec77a73f0de3e531afbd` | existing notify test module before namespace fixture | Declare `TestBannerGuard` and `TestNotifyDir` used by admitted tests. |
| P2-T1 | exact | `TRACKED_WIP:src/bin/monitor/notify.rs:4672-4764` | `92c24ca5d093f93edfbaf6a7c541ea76b61dd64529d2e3ab473313e6b589be49` | existing `notify.rs` test module | TEST_CODE pinned namespace fixture and JSON artifact enumerator only. |
| P2-T1A | exact | `TRACKED_WIP:src/bin/monitor/notify.rs:4766-4798` | `1fa88883b0fad152f1a249616ed3c19f6ed604284abd5b198517a0dfbf2c89dd` | after `push_log_json_artifacts` | Implement `TestNotifyDir` and `TestBannerGuard::full`. |
| P2-T2 | exact | `TRACKED_WIP:src/bin/monitor/notify.rs:4824-5653` | `902bef612c042b95163d1c38f5b1596e7dfe70786d3d02159e445471e2c0b7f7` | existing `notify.rs` test module | Counted request/audit fixtures; fail-closed finalization and push-log physical-isolation tests. |
| P2-T3 | exact | `TRACKED_WIP:src/bin/monitor/notify.rs:6053-6180` | `e57a8cd225f85b636f26c073a82110b75414031f8bae9b627d016b9e76fed171` | existing `notify.rs` test module | Explicit counted dry-run, production-no-synthetic-receipt, and generic governor rejection tests. |
| P2-T4 | new controlled test splice | frozen fixture in §P2 golden fixtures | `8de344f9fa9b80cbd114474f9299190a7e53a2d57553a4f649d2f4ef9f36bd33` | `event::envelope::tests::br192_schema_v2_golden_publication_bytes_are_unchanged` in P2-E1's existing test module | Fixed 254-byte input; compare exact 689-byte schema-v2 publication bytes. This is the only permitted P2-E1 insertion. |
| P2-T5 | new controlled test splice | frozen fixture in §P2 golden fixtures | `bd198be71b8cdc2e3f66b93ac3bc515f6bff453412639176dcb449c1beab6680` | `event::push_record::tests::br192_schema_v2_golden_parser_output_bytes_are_unchanged` in P2-E3's existing test module | Parse the fixed input through the public authoritative parser and compare exact 582-byte canonical output. This is the only permitted P2-E3 insertion. |
| P2-T6 | new non-counted golden | frozen fixture in §P2 golden fixtures | `41d03c80490b6c553aba19da0219db7ad3b69527f2bb24f80dfe9a52e496fb6d` | `notify::tests::br192_non_counted_dry_run_golden_push_log_has_exact_bytes_and_zero_sink_calls` | Exercise namespace-aware dry-run through `push_wechat`; assert one exact 73-byte artifact and zero sink calls. |
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
| P3-R4-2 | exact | `TRACKED_WIP:src/bin/monitor/push_templates.rs:8638-9099` | `2045e252c6284f3f8f56fac48ce817d047274108ae64d67d96c6c41362f1e4f5` | R-04 binding/validation/preparation/dispatch | Depends on P1 DragonTiger Gateway and P3 dedicated SourceOnly notify entry; no later-rule marker is present. |

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

Test-set abbreviations below are closed and are enforced by the new
`tools/release/check_br194_recovery_focused.sh` verifier:

- `A1`: one exact Magic manifest/lock revision test;
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
- `CC1`: one counted-cutover integration test;
- `M31/P3`: 31 monitor BR-194 tests and three BR-194 process tests;
- `WK1/SM3/R96/R4B3/R4S3/CAT1`: respectively one weekend-date caller test,
  three scheduler BR-192 tests, six R-09 renderer/caller tests, three R-04
  BR-162 tests, three R-04 BR-192 tests and one counted-catalog test.
- `V4/V5`: seven coverage-checker tests and twenty recovery-verifier process tests
  with the exact names frozen in rows V4/V5.

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

| ID | Destination splice anchor after implementation | Owner | Required test set |
| --- | --- | --- | --- |
| P0-A1 | one new BR-203 row; every frozen active-ledger row byte-identical | BR-203 | Rule-2.10 + whole-file/hash proof |
| P1-A2 | `[dependencies]` exact entries only | BR-203 | `A1` |
| P1-A3 | generated `[[package]]` Magic closure | BR-203 | `A1` |
| P1-A4 | new `magic_market_release_revision` test file | BR-203 | `A1` |
| P1-A5 | new BR-203 compliance checker and runner registration | BR-203 | `A1` + compliance |
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
| P2-M1 | startup eager bind before sink initialization | BR-192 | `EB1` |
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
| P3-R4-2 | R-04 binding/preparation/dispatch block | BR-162/BR-194 | `DT6+M31+R4B3+R4S3` |
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

## P2 reproducible focused baseline

All commands below ran in the repository-owned reconstructed clone with
`CARGO_TARGET_DIR` bound to the repository target. Exact results were:

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
cross-process test; it is not counted as a twenty-fifth direct pass.

## Remaining manifest gates

- P0/P1/P2/P3 historical source ranges and replacement target contracts are
  enumerated; no unlisted historical byte is admitted and the rejected first
  candidate blobs have no authority.
- P0-A1 must be committed before P2; its target keeps every row from the frozen active-ledger
  preimage byte-identical, including the active BR-159/BR-192/BR-194 amendments, and adds only
  BR-203. The historical baseline rows remain extraction evidence only and are not a target.
- `P0-M0` and direct-child `P0-A1` must be materialized as real Git commit objects. Independent
  review must prove the exact parent/child tree relation and report `C0/I0/M0` before any Gate-B
  source edit.
- Before each accepted slice is staged, exact rows must be reverified, every
  partial-file row must gain a literal destination range and target hunk hash,
  and extract/adapt/new rows must gain the same in the implementation ledger;
  the focused verifier must prove every
  declared test filter is non-zero with the exact frozen count.
