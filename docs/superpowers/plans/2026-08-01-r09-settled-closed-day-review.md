# R-09 Settled Closed-Day Review — BR-192 Supporting Implementation Map

**Upstream debt**

- Fixed baseline `b4aeee68d2c0259cc968914b3d39e3a89a18a496` has
  `ReviewTask::R09` and a current-date-only static preflight, but no production
  `dispatch_r09_provider_top_n_outcome`, no `CapitalDataGateway`, no BR-200
  occurrence inspection and no atomic Magic release-identity test.
- The severed production path is therefore
  `dispatch_post_session_review -> R-09 provider -> renderer -> counted durable
  delivery -> sink`: fixed HEAD stops before the R-09 provider/producer exists.
- Dirty-worktree copies of the missing symbols, dependency rows and tests are
  unaccepted candidate bytes. They are not a base or proof of implementation.

**Rename impact**

- No public business name is renamed. BR-192 Task 8 must replace every
  one-argument `provider_top_n_pair(trading_date)` and
  `dispatch_r09_provider_top_n_outcome(business_date)` candidate call with the
  accepted typed Shanghai observation-window seam.
- Before implementation, enumerate all production, test, checker, snapshot and
  documentation references with:

  ```bash
  rg -n "provider_top_n_pair|dispatch_r09_provider_top_n_outcome|ReviewRunContext" \
    src tests tools docs
  ```

- No old one-argument call, host-local clock substitution, or checker snapshot
  may survive the BR-192 implementation manifest.

**Production evidence**

- Successful real delivery must join
  `data/push_log/YYYY-MM-DD/*_audit_pending.json`,
  `data/push_log/YYYY-MM-DD/*_committed.json` and
  `data/event_bus/YYYY-MM-DD.jsonl` with exact
  `event_type="push.delivery.audit"` and `ReviewProviderTopN` identity.
- Enabled startup must contain exactly:

  ```text
  [BR-192][counted-producer] push_kind=ReviewProviderTopN enabled=durable_binding producer=push_templates::dispatch_r09_provider_top_n_outcome
  ```

- Until BR-192 enables the producer, startup must instead retain:

  ```text
  [BR-192][counted-producer] push_kind=ReviewProviderTopN disabled=no_producer reason=capability_unavailable:<reason_code>
  ```

- A Markdown preview, `--test` disabled path, function definition or startup
  banner is not a real receipt.

## 0. Ownership and sequencing

This is not an independent BR-198 implementation plan. It is the closed
supporting contract and test map consumed by BR-192 Task 8/Gate B.

The only valid progression is:

1. accept BR-200 Gate C while R-09 remains disabled;
2. accept the BR-192 Gate-A authority that incorporates the paired BR-198
   supporting contract;
3. in one clean isolated BR-192 worktree, atomically create the R-09 gateway,
   producer, observation-window validation, dependency closure, tests and
   compliance checks plus the checked-in forward rollback patch;
4. obtain BR-192 Gate B/C independently;
5. obtain Gate D only from accepted BR-202 coverage authority plus real provider,
   durable-audit and receipt evidence.

BR-198 has no separate Gate B, Gate C, implementation commit, PR or prerequisite
Gate C. It must not block BR-192 from creating the artifacts through which the
contract is implemented.

No task in this document authorizes edits to `docs/business_rules.md`, BR-192 or
BR-200 documents, production code, dependencies or the index. The BR-192 owner
performs those actions only after its own accepted Gate-A sequence.

## 1. Fixed baseline and isolation

The factual source baseline is:

```text
b4aeee68d2c0259cc968914b3d39e3a89a18a496
```

The BR-192 owner must record a literal `BR192_BASE_SHA` for the accepted
implementation base and use a clean isolated worktree. It must not execute from
the shared dirty worktree, whole-file stage an existing candidate, use
`HEAD~N`, or infer a base from commit count.

The BR-198 subset of the BR-192 Task-8 allowlist is:

- `Cargo.toml`
- `Cargo.lock`
- `src/bin/monitor/main.rs`
- `src/bin/monitor/review_batch.rs`
- `src/bin/monitor/push_templates.rs`
- `src/data_gateway/capital.rs`
- `tests/magic_market_release_revision.rs`
- `tools/compliance/lib/check_br194_review_dependency.sh`
- `tools/release/disable_br192_periodic_retry.patch`
- the accepted BR-192 design/plan and business-rule evidence owned by BR-192.

The BR-192 plan may have a larger reviewed allowlist for its retry state machine,
but every BR-198 change must be attributable to the exact accepted base and
final source manifest. Verification uses:

```bash
git diff --check "${BR192_BASE_SHA}..HEAD"
git diff --name-status "${BR192_BASE_SHA}..HEAD"
git diff --exit-code "${BR192_BASE_SHA}..HEAD" -- \
  docs/superpowers/specs/2026-08-01-r09-settled-closed-day-review-design.md \
  docs/superpowers/plans/2026-08-01-r09-settled-closed-day-review.md
```

The last command proves the accepted supporting-contract blobs did not drift
during implementation. There is deliberately no independent BR-198 `git add`,
commit or merge recipe.

## 2. Data-red-line execution matrix

| Rule | Applicability and required evidence |
| --- | --- |
| 2.1 | Sole real `EastmoneyProviderTopNRankingRouter::new()` route; no mock/cache/local/alternate/fabricated fallback. |
| 2.2 | Missing, empty or partial evidence is a typed error; nothing is filled. |
| 2.3 | Finite values, exact `f297`, order, pair identity and trusted observation window are validated atomically. Price-series-only checks are scoped N/A. |
| 2.4 | Fixed Asia/Shanghai clock, same-day 15:35 gate, exact calendar-selected date and `request_start <= capture <= completion`. |
| 2.5 | `--test` blocks durable/provider/sink work first; fixtures use only `TEST_CODE`. |
| 2.6 | N/A: no order path is created or invoked. |
| 2.7 | Request, raw provider timestamps, completion, batch, decision, receipt and audit identities are retained for at least five years. |
| 2.8 | Acquisition/delivery performs real target work or fails; logging-only success is forbidden. |
| 2.9 | N/A: no `config/*.toml` threshold changes. |
| 2.10 | BR-192 owner reconciles BR-198 in the same accepted implementation authority before Gate B. |

## 3. BR-192 Task-8 target mapping

### 3.1 Trusted Shanghai context

BR-192 replaces host-local naive observation in the R-09 production path with a
trusted observation captured from system UTC and converted to fixed Shanghai
`+08:00`. `ReviewRunContext` remains the sole owner of the calendar-selected
business date and immutable request start.

Production must not use `chrono::Local::now().naive_local()` as BR-198 authority.
A test-only clock is private, compiled only under `cfg(test)`, and requires
`TEST_CODE` fixtures.

### 3.2 Static preflight

Preserve this exact order:

```text
test isolation
  -> future review date = Failed(false, provider_top_n_future_date)
  -> same date before 15:35 Shanghai = ExpectedWait(15:35)
  -> same date at/after 15:35 = runnable
  -> calendar-selected prior date = runnable
  -> BR-200 durable occurrence preflight
```

There is no arbitrary date CLI and no inferred settlement date.

### 3.3 Gateway observation window

The target gateway receives the requested date and typed Shanghai
`request_started_at`. It captures `capture_completed_at` from the same trusted
clock after both real pages return and before admission.

The sole private validator parses each raw provider `observed_at` byte string,
preserves those bytes, and rejects the entire pair unless:

- `request_started_at <= volume_capture <= capture_completed_at`;
- `request_started_at <= inflow_capture <= capture_completed_at`;
- request start, both captures and completion have one Shanghai date;
- completion is not earlier than request start;
- every row's provider `f297` equals the requested trading date;
- both sides are non-empty, finite, correctly typed, source ordered and have
  exact request/provider/source/metric/unit/filter/batch evidence.

Any violation returns exact `provider_top_n_invalid_evidence`; it never returns
`VerifiedEmpty`, partial success, `NoData`, current-date retry or fabricated
evidence.

### 3.4 Dispatcher and delivery order

After static preflight, R-09 performs the accepted BR-200 occurrence inspection.
Only exact `None` may acquire the BR-192 counted-producer permit and call the
gateway. Existing terminal evidence reuses/fails closed provider-free. The
loader passes the exact business date and request start; it must not reread a
wall clock or substitute a date.

`BannerCtx`, AccountMode, broker snapshots, local portfolio, cache and alternate
providers remain forbidden on the R-09 SourceOnly acquisition path.

### 3.5 Atomic release identity

BR-192 Task 8 installs/verifies exactly:

- Git repository `https://github.com/Northofqing/magic-market-data-rs.git`;
- revision `5f1ce93656a55854c844065390520cd4aecd9a14`;
- every Magic package at exact version `=0.2.0`;
- fourteen direct dependencies:
  `magic-baidu-rs`, `magic-cls-rs`, `magic-cninfo-rs`,
  `magic-eastmoney-rs`, `magic-exchange-rs`, `magic-jin10-rs`,
  `magic-market-composition`, `magic-market-core`, `magic-market-router`,
  `magic-sina-rs`, `magic-tdx-rs`, `magic-tencent-rs`,
  `magic-thepaper-rs`, `magic-ths-rs`;
- exactly fifteen Magic lock packages, with only
  `magic-market-transport` transitive.

The release test rejects path dependencies, direct transport, a second
repository, mixed revision/version, missing/extra packages and old revision
`660902ff93a07f18367dc16879cf67732accd25a`.

## 4. Canonical BR-192 Task-8 behavioral test map

BR-198 declares no independent tests. The sole executable declarations and
`--exact` commands live in the BR-192 Task-8 plan and implementation. This
supporting map mirrors that one canonical `br192_br198_*` suite; a second
`br198_*` namespace is forbidden.

| Canonical exact test | Required proof |
| --- | --- |
| `br192_br198_closed_day_r09_uses_review_business_date_and_exact_f297` | Calendar-selected prior date is runnable and every row keeps exact `f297`. |
| `br192_br198_future_r09_fails_before_durable_preflight_permit_provider_renderer_sink` | Future date is non-retryable `provider_top_n_future_date` with zero downstream calls. |
| `br192_br198_same_day_1535_boundary_precedes_terminal_preflight` | 15:34:59 waits, exact 15:35 and later run, all before durable/provider access. |
| `br192_br198_closed_day_rejection_does_not_extend_source_expiry_or_retry` | Prior-date initial rejection never extends retry expiry. |
| `br192_br198_host_tz_cannot_change_shanghai_review_date_or_1535_boundary` | Different host `TZ` child processes produce the same Shanghai decision. |
| `br192_br198_capture_before_trusted_request_start_fails_pair_before_durable_sink` | Same-date capture before trusted start rejects the pair atomically. |
| `br192_br198_capture_after_trusted_request_completion_fails_pair_before_durable_sink` | Same-date capture after trusted completion rejects the pair atomically. |
| `br192_br198_capture_raw_bytes_round_trip_and_mutation_rejects_pair_before_durable_sink` | Exact typed raw fields/hash round-trip; byte mutation, including equivalent instant text, fails before durable/sink. |
| `br192_br198_capture_before_request_date_fails_pair_before_durable_sink` | Capture date before requested date rejects both metrics. |
| `br192_br198_capture_crosses_shanghai_midnight_fails_pair_before_durable_sink` | Start/capture/completion crossing Shanghai midnight rejects both metrics. |
| `br192_br198_invalid_provider_capture_timestamp_fails_pair_before_durable_sink` | Malformed complete timestamp/raw evidence rejects both metrics. |
| `br192_br198_prior_date_initial_admission_ignores_retry_expiry_but_retry_rejects` | Initial prior-date acquisition remains allowed while retry remains expired. |

BR-192 must discover exactly these twelve names once in
`tests/durable_delivery_counted_cutover.rs`:

```bash
test "$(cargo test --test durable_delivery_counted_cutover -- --list | rg -c 'br192_br198_')" -eq 12
```

It then executes every canonical name with `--exact --test-threads=1`; the
commands are frozen once in BR-192 Task 8 and are not independently duplicated
as another Gate authority here. The BR-192 compliance checker must reject any
missing/duplicate declaration, a `br198_*` shadow namespace, host-local time,
one-argument gateway/dispatcher seams, omitted raw evidence field/hash, fallback
provider, or BR-200-before-BR-198 ordering. Test isolation is additionally
proved by BR-192's existing dual-disable and SourceOnly tests; it is not renamed
into a BR-198-only test.

## 5. BR-192 Gate B/C verification mapping

BR-198 does not run these gates independently. The fresh BR-192 verifier runs:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
cargo build --release --bin monitor
V10_DRY_RUN_PUSH=1 ./target/release/monitor --test
```

If freshness compliance fails, only the mandated real-data repair is allowed:

```bash
bash tools/one_shot/backfill_daily.sh
bash tools/compliance/check.sh
```

No synthetic row may satisfy freshness. The release-revision test must report
exactly one test and one pass:

```bash
cargo test --test magic_market_release_revision \
  br192_magic_market_release_revision_is_one_atomic_identity -- \
  --exact --test-threads=1
```

The independent verifier must not trust the implementer. It reruns commands,
checks the production call graph, verifies exact test counts, inspects the full
`BR192_BASE_SHA..HEAD` allowlist, checks current/legacy PushKind annotations, and
attaches the exact production evidence or records Gate D blocked.

## 6. Live validation and Gate D

After BR-192 Gate C, a real closed-day run may execute:

```bash
cargo run --bin monitor -- --review
```

Success evidence must prove:

- the review calendar selected the latest completed trading date;
- request start and completion are trusted fixed Shanghai observations;
- both raw provider capture byte strings were retained and fall inside the
  trusted one-day window;
- every `f297` equals the requested date and the pair is complete/ordered;
- the real Feishu receipt joins the durable decision and exact
  `push.delivery.audit` event.

External unavailability or a provider mismatch is a failed live validation,
never healthy/empty/fabricated data.

Gate D is evaluated only by the accepted BR-202 isolated wrapper
`tools/coverage/run_isolated_gate.sh` after BR-192 Gate C. Raw coverage commands
cannot mint authority. Until coverage floors, real provider evidence, durable
audit and receipt join all pass, BR-192/BR-198 remain not release-ready.

## 7. PR evidence owned by BR-192

The BR-192 PR must include:

```markdown
### Refs
- BR-192 accepted design/plan Task 8
- supporting contract: `docs/superpowers/specs/2026-08-01-r09-settled-closed-day-review-design.md`
- supporting map: `docs/superpowers/plans/2026-08-01-r09-settled-closed-day-review.md`

### Data-Redlines
- [2.1] Sole real Eastmoney Provider Top-N route; no mock/fallback
- [2.2] Missing/partial evidence fails explicitly
- [2.3] Finite/order/date/pair/window validation before computation
- [2.4] Fixed Shanghai time, 15:35 gate, start <= capture <= completion
- [2.5] TEST_CODE physical isolation and zero external test I/O
- [2.6] N/A: no order path
- [2.7] Raw captures, request, completion, decision, receipt and audit retained
- [2.8] No logging-only acquisition or delivery
- [2.9] N/A: no config threshold change
- [2.10] BR-192, BR-194, BR-198, BR-200, BR-202

### OldModules
| module | adopt/reject | reason |
| --- | --- | --- |
| ReviewRunContext | deepen | typed fixed-Shanghai observation authority |
| BR-200 occurrence preflight | adopt | provider-free terminal reuse/failure |
| current-date-only R-09 | reject narrowly | blocks calendar-selected settled closed day |
| cache/local/alternate fallback | reject | cannot prove settlement |
| independent BR-198 Gate B/C | reject | circular with BR-192 artifact creation |

### Threshold-Proof
- N/A: no threshold or config change.

### Business-Rules
- BR-192, BR-194, BR-198, BR-200, BR-202

### Validation
- exact named/cardinality tests: PASS
- full Gate B/C commands: PASS
- fixed 14-direct/15-lock identity: PASS
- production evidence: attached or Gate D explicitly blocked

### Rollback
- Apply the Gate-B-created `tools/release/disable_br192_periodic_retry.patch` to the literal accepted BR-192 release SHA; never revert the atomic Task-8 commit.
- Require `src/bin/monitor/main.rs` to be the patch's sole diff target and disable only periodic retry-runner startup.
- Preserve all R-09/BR-200 behavior, v6 schema, exact 15-row catalog, durable/audit semantics and Magic 14-direct/15-lock identity.
```

## 8. Rollback verification

BR-192 Gate B creates and checks in
`tools/release/disable_br192_periodic_retry.patch`. Its SHA-256 is recorded in
the BR-192 PR and release evidence. The patch applies only to the literal
accepted BR-192 release commit; `HEAD~N`, reverting the atomic Task-8 commit and
an independent BR-198 commit are forbidden.

The rollback operator runs this exact Bash sequence from the repository that
contains the accepted release commit:

```bash
: "${BR192_RELEASE_SHA:?set BR192_RELEASE_SHA to the literal accepted 40-hex BR-192 release commit}"
: "${BR192_ROLLBACK_PATCH_SHA256:?set BR192_ROLLBACK_PATCH_SHA256 to the Gate-B-recorded 64-hex patch digest}"
printf '%s\n' "${BR192_RELEASE_SHA}" | rg -q '^[0-9a-f]{40}$'
printf '%s\n' "${BR192_ROLLBACK_PATCH_SHA256}" | rg -q '^[0-9a-f]{64}$'
test "$(git rev-parse "${BR192_RELEASE_SHA}^{commit}")" = "${BR192_RELEASE_SHA}"

BR192_ROLLBACK_PATCH=tools/release/disable_br192_periodic_retry.patch
BR192_ROLLBACK_ROOT="$(mktemp -d /tmp/stock-analysis-br192-rollback.XXXXXX)"
BR192_ROLLBACK_BRANCH="rollback/br192-disable-periodic-${BR192_RELEASE_SHA}"
git worktree add -b "${BR192_ROLLBACK_BRANCH}" \
  "${BR192_ROLLBACK_ROOT}" "${BR192_RELEASE_SHA}"

test -f "${BR192_ROLLBACK_ROOT}/${BR192_ROLLBACK_PATCH}"
test "$(shasum -a 256 "${BR192_ROLLBACK_ROOT}/${BR192_ROLLBACK_PATCH}" | cut -d ' ' -f1)" = \
  "${BR192_ROLLBACK_PATCH_SHA256}"
test "$(git -C "${BR192_ROLLBACK_ROOT}" apply --numstat -- \
  "${BR192_ROLLBACK_PATCH}" | wc -l | tr -d ' ')" -eq 1
test "$(git -C "${BR192_ROLLBACK_ROOT}" apply --numstat -- \
  "${BR192_ROLLBACK_PATCH}" | cut -f3)" = "src/bin/monitor/main.rs"
git -C "${BR192_ROLLBACK_ROOT}" apply --check --index -- \
  "${BR192_ROLLBACK_PATCH}"
git -C "${BR192_ROLLBACK_ROOT}" apply --index -- \
  "${BR192_ROLLBACK_PATCH}"
test "$(git -C "${BR192_ROLLBACK_ROOT}" diff --cached --name-only --)" = \
  "src/bin/monitor/main.rs"
test -z "$(git -C "${BR192_ROLLBACK_ROOT}" diff --name-only --)"

test "$(cargo test --manifest-path "${BR192_ROLLBACK_ROOT}/Cargo.toml" \
  --test durable_delivery_counted_cutover -- --list | rg -c 'br192_br198_')" -eq 12
cargo test --manifest-path "${BR192_ROLLBACK_ROOT}/Cargo.toml" \
  --test durable_delivery_counted_cutover br192_br198_ -- --test-threads=1

cargo test --manifest-path "${BR192_ROLLBACK_ROOT}/Cargo.toml" \
  --test durable_delivery_counted_cutover br192_br200_r09_ -- --test-threads=1
cargo test --manifest-path "${BR192_ROLLBACK_ROOT}/Cargo.toml" \
  --lib durable_delivery::tests::br192_schema_v6_fresh_and_v1_v2_v3_v4_v5_upgrade_paths_validate -- \
  --exact --test-threads=1
cargo test --manifest-path "${BR192_ROLLBACK_ROOT}/Cargo.toml" \
  --lib durable_delivery::tests::br192_v5_to_v6_preserves_br194_replay_manifest_audit_kinds_and_rows -- \
  --exact --test-threads=1
cargo test --manifest-path "${BR192_ROLLBACK_ROOT}/Cargo.toml" \
  --test br192_counted_producer_catalog -- --test-threads=1
cargo test --manifest-path "${BR192_ROLLBACK_ROOT}/Cargo.toml" \
  --test magic_market_release_revision \
  br192_magic_market_release_revision_is_one_atomic_identity -- \
  --exact --test-threads=1
cargo test --manifest-path "${BR192_ROLLBACK_ROOT}/Cargo.toml" \
  --lib durable_delivery::tests::br192_rollback_preserves_four_stage_retry_origin_reserved_recovery -- \
  --exact --test-threads=1
cargo test --manifest-path "${BR192_ROLLBACK_ROOT}/Cargo.toml" \
  --test durable_delivery_counted_cutover \
  br192_rollback_never_routes_retry_origin_reserved_to_resume_deliverable -- \
  --exact --test-threads=1

cargo fmt --manifest-path "${BR192_ROLLBACK_ROOT}/Cargo.toml" --all -- --check
cargo clippy --manifest-path "${BR192_ROLLBACK_ROOT}/Cargo.toml" \
  --workspace --all-targets --all-features -- -D warnings
cargo test --manifest-path "${BR192_ROLLBACK_ROOT}/Cargo.toml" \
  --workspace --all-targets --all-features -- --test-threads=1
(cd "${BR192_ROLLBACK_ROOT}" && \
  bash tools/compliance/lib/check_br192_provider_free_retry.sh)
(cd "${BR192_ROLLBACK_ROOT}" && \
  bash tools/compliance/lib/check_br194_review_dependency.sh)
(cd "${BR192_ROLLBACK_ROOT}" && bash tools/compliance/check.sh)
cargo build --manifest-path "${BR192_ROLLBACK_ROOT}/Cargo.toml" \
  --release --bin monitor

git -C "${BR192_ROLLBACK_ROOT}" commit -m \
  "revert: disable BR-192 periodic retry discovery"
BR192_ROLLBACK_SHA="$(git -C "${BR192_ROLLBACK_ROOT}" rev-parse HEAD)"
printf 'BR192_ROLLBACK_SHA=%s\n' "${BR192_ROLLBACK_SHA}"
```

Every command must exit zero. The patch must leave initial and repeated-review
R-09, BR-200 preflight, schema v6 recognition/validation, the exact 15-row
catalog and its enabled R-09 row, retained durable/audit records, uncertainty
reconciliation, and the exact Magic 14-direct/15-lock identity operational.
Only periodic provider-free retry discovery is disabled.

Rollback does not delete this historical supporting contract and never restores
revision `660902ff93a07f18367dc16879cf67732accd25a`, a path dependency, fallback,
partial pair, provider timestamp replacement or host-local time authority. The
reviewed rollback PR records the patch SHA-256, accepted release SHA, rollback
commit SHA, command outputs and reconciled BR-198 business-rule status.
