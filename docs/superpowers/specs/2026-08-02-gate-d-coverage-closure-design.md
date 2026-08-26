# Gate D 覆盖率收口设计

**状态：** 2026-08-26 分层门禁修订已写入 §10，等待用户书面复核。§0–§9
保留为历史设计审计；凡与 §10 冲突之处，均由 §10 取代，不得继续作为实施依据。
本次只完成 Gate A 文档与 BR-252 登记，不声称 Gate B/C/D 完成。

**Date:** 2026-08-02

**Scope:** coverage measurement, deterministic behavior tests, isolated protocol
tests, and Gate D evidence. This design does not change trading, selection,
provider, notification, or account semantics.

**Identity compatibility and migration:** no business module, `PushKind`,
database filename/schema identity, or audit event type is renamed. The coverage
artifact API **does change** from a standalone
`target/coverage/coverage.json` upload, and this sixth remediation also replaces
the previously proposed non-portable terminal contract before implementation.
Local durable publication uses an immutable
`gate-d-runs/<source-sha>/<run-id>/` bundle, a
`gate-d-local-publication-terminal/v1` that may bind local path/device/inode,
and a later synced journal confirmation. Portable delivery then creates one
canonical `gate-d-portable-archive/v1` tar, one detached
`gate-d-portable-authority-terminal/v1`, and its detached Ed25519 signature.
The portable terminal binds content identities and canonical relative names,
never a download path/device/inode. CI retains artifact name `coverage-report`
but uploads only those three portable files. The bundle still supplies a
read-only root `coverage.json` compatibility copy inside the archive. Section
2.5 freezes CI/README/consumer migration, deprecation, quarantine, rollback and
supersession. The old proposed `gate-d-authority-terminal/v1` identity is
withdrawn unused; none of these v1 identities has been published as release
authority. Any later semantic/layout change adds a reviewed higher version and
migration reader; it never silently reuses a v1 identity.

**Supersedes for future work:**
`docs/superpowers/plans/2026-07-18-gate-d-coverage-closure.md`. The old file is
retained as historical evidence only. It assigns acquisition work to the old
`src/data_provider/**`/RustDX path and names PR #2, so it is not an executable
plan against the current unified Magic Gateway tree. The compatibility types
that still exist under `src/data_provider/` remain core and are not deleted.

## 0. Pre-flight

### Impacted paths

Current Gate-A documentation paths edited by this repair:

- `docs/business_rules.md`
- `docs/superpowers/specs/2026-08-02-gate-d-coverage-closure-design.md`

The following are **planned Gate-B paths, not current Code-cell claims**. A path
must be added to the BR-202 Code cell and contain an exact `BR-202` citation in
the same staged source slice before it is created or modified. A repository
test rejects a planned path that exists without both conditions:

- `tools/coverage/check_thresholds.py`
- `tests/test_coverage_thresholds.rs`
- `tools/coverage/run_isolated_gate.sh`
- `tools/coverage/gate_d_journal.py`
- `tools/coverage/entrypoint_inventory.v1.json`
- `tools/coverage/behavior_authorities.v1.json`
- `tools/coverage/behavior_residuals.v1.json`
- `tools/coverage/behavior_clusters.v1.json`
- `tools/coverage/decision_sinks.v1.json`
- `tools/coverage/decision_site_owners.v1.json`
- `tools/coverage/decision_denominator.schema.v1.json`
- `tools/coverage/extract_decision_denominator.py`
- `tools/coverage/rustc_decision_capture.py`
- `tools/coverage/fixtures/decision_probe/**`
- `tools/coverage/dependency_snapshot.v1.json`
- `tools/coverage/build_profile.schema.v1.json`
- `tools/coverage/gate_d_bundle.schema.v1.json`
- `tools/coverage/host_execution_inputs.schema.v1.json`
- `tools/coverage/host_execution_policy.v1.json`
- `tools/coverage/capture_host_execution_inputs.py`
- `tools/coverage/isolation_policy.v1.json`
- `tools/coverage/local_publication_terminal.schema.v1.json`
- `tools/coverage/portable_archive.schema.v1.json`
- `tools/coverage/portable_authority_terminal.schema.v1.json`
- `tools/coverage/portable_signers.v1.json`
- `tools/coverage/build_portable_archive.py`
- `tools/coverage/sign_portable_terminal.py`
- `tools/coverage/gate_d_attestation.schema.v1.json`
- `tools/coverage/verify_gate_d_attestation.py`
- `tests/test_gate_d_attestation.rs`
- `tests/test_coverage_entrypoints.rs`
- `.cargo/gate-d-vendor-config.toml` and `vendor/gate-d/**`
- `docs/ENGINEERING_RULES_V2.md` (active raw baseline retained and classified;
  this Gate A remediation does not edit it)
- `.github/workflows/coverage.yml` and `README.md`: Gate B migrates both
  documented/CI entrypoints to the isolation wrapper and rejects every direct
  release-gate invocation; this Gate A remediation does not edit either file
- behavior tests beside existing production modules under the registered core
  directories
- all production source paths classified by the complete ownership inventory in
  Section 2.2; unclassified or multiply classified paths fail closed
- isolated process/protocol tests under `tests/`
- this design, the future post-review implementation plan,
  `docs/business_rules.md`, and PR evidence

### 0.1 Current registration, staging, and PR evidence

The current BR-202 Code cell contains only the two existing documentation paths
edited at Gate A. It intentionally does not cite nonexistent future files or
claim missing implementation citations. This worktree repair does not stage the
shared rules file. Before the next formal review, the dispatcher must stage the
exact design blob and apply/stage only the unique BR-202 row change without
capturing unrelated `docs/business_rules.md` worktree bytes, then record these
checks:

```bash
DESIGN=docs/superpowers/specs/2026-08-02-gate-d-coverage-closure-design.md
RULES=docs/business_rules.md
git ls-files --error-unmatch "$DESIGN" "$RULES"
test "$(git hash-object "$DESIGN")" = "$(git rev-parse ":$DESIGN")"
test "$(git hash-object "$RULES")" = "$(git rev-parse ":$RULES")"
test "$(git show ":$RULES" | rg -c '^\| BR-202 \|')" = 1
cmp \
  <(git show ":$RULES" | awk '/^\| BR-202 \|/{print}') \
  <(git show ":$DESIGN" | awk '/^\| BR-202 \|/{print}')
git diff --cached --check -- "$DESIGN" "$RULES"
```

Expected: both paths resolve in the index; index blob IDs equal reviewed
worktree bytes; exactly one index BR-202 row exists; the two index row copies
are byte-identical; whitespace validation exits 0. “Tracked”, “registered in
the PR”, or “ready for review” may be stated only after these literal results
are pasted. `git diff --cached -- docs/business_rules.md` must show the unique
BR-202 row and no unrelated row. The PR must contain both paths in one docs-only
Gate-A commit and preserve row equality. Gate B then updates the row in each
source slice before or atomically with the first creation/modification of every
planned path; the slice fails if the new path is missing, absent from Code, or
lacks its literal citation.

### Triggered rules

- AGENTS 2.1: coverage tests cannot create a production mock or fake success.
- AGENTS 2.2: missing fixture or source fields remain absent and are asserted.
- AGENTS 2.3: bad data, including an adjacent valid-value change beyond 20%,
  exercises the alert plus manual-confirmation contract; coverage work cannot
  remove or weaken that rule.
- AGENTS 2.4: current freshness behavior is tested without relaxing windows.
- AGENTS 2.5: every security identity is `TEST_CODE_`; databases, logs, audit
  roots, sinks, and accounts are physically test-isolated.
- AGENTS 2.6: order tests retain cash, lot, price-limit, idempotency, and
  secondary-confirmation boundaries.
- AGENTS 2.7: audit-chain success and failure branches remain traceable.
- AGENTS 2.8: no logging-only test seam may satisfy a production operation.
- AGENTS 2.9: immutable coverage floors and any future config/schema identity
  remain bidirectionally proved; threshold greater than its clamp fails.
- AGENTS 2.10 / BR-202: core-scope filtering and missed-line prioritization are
  registered in the two current documentation paths before implementation;
  every planned Gate-B path, including `tests/test_coverage_entrypoints.rs`,
  must join the canonical Code cell and cite BR-202 in its first source slice.
- Gate D: global line coverage is at least 80% and core line coverage is at
  least 95%. Neither threshold may be lowered and the denominator may not be
  narrowed.

### Validation commands

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
bash tools/coverage/run_isolated_gate.sh
cargo build --release --bin monitor
git diff --check
```

### Rollback

Revert the smallest failed test batch and its coverage-tool change. Never roll
back by deleting audit/data evidence, disabling a target, adding an exclusion,
lowering 80%/95%, or moving code outside a core path. A behavior mismatch returns
to Gate B; an ownership/scope mistake returns to Gate A.

## 1. Reproducible current facts (provisional planning evidence)

The report below predates the final BR-196/BR-201 merge and is a planning
baseline, not release evidence.

Command:

```bash
stat -f 'mtime=%Sm size=%z' -t '%Y-%m-%dT%H:%M:%S%z' \
  target/coverage/coverage.json
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
```

Output:

```text
mtime=2026-08-01T23:42:34+0800 size=35049846
global coverage gate failed
core coverage gate failed
global line coverage: 149200/189647 = 78.67% (required 80.00%)
core line coverage: 121025/154838 = 78.16% (required 95.00%, 204 files)
```

The current checker is not an ownership inventory. It covers 15 directory
prefixes, omits clearly production-owned contexts and root/bin entrypoints, and
does not reject an unclassified source file. Two examples are broker/calendar;
the runtime bridge in `src/lib.rs` is a third:

```bash
rg -n '^pub mod (broker|calendar);' src/lib.rs
rg -nA3 'stock_analysis::calendar::|stock_analysis::broker::|crate::calendar::|crate::broker::' \
  src/bin/monitor src --glob '*.rs' | head -20
rg -nA4 'crate::block_on_async\(' src/opportunity/chain_mapper.rs
jq -r '.data[0].files[]
  | select(.filename|endswith("/src/calendar.rs") or
      endswith("/src/broker.rs") or endswith("/src/lib.rs"))
  | [.filename,.summary.lines.covered,.summary.lines.count] | @tsv' \
  target/coverage/coverage.json
```

Relevant output:

```text
src/lib.rs:7:pub mod broker;
src/lib.rs:10:pub mod calendar;
src/bin/monitor/main.rs:34:use stock_analysis::calendar::{self, current_session, is_market_active, MarketSession};
src/bin/monitor/main.rs:1557:tokio::task::spawn_blocking(move || stock_analysis::broker::execution_quote(&code))
src/bin/monitor/main.rs:3204:match stock_analysis::calendar::verified_a_share_trading_day(business_date) {
src/bin/monitor/main.rs:3705:let broker_src = match stock_analysis::broker::detect_and_register() {
src/opportunity/chain_mapper.rs:225:        crate::block_on_async(async move {
/Users/zhangzhen/Desktop/Quant/stock_analysis/src/broker.rs 82 127
/Users/zhangzhen/Desktop/Quant/stock_analysis/src/calendar.rs 297 343
/Users/zhangzhen/Desktop/Quant/stock_analysis/src/lib.rs 45 74
```

The complete proposed ownership inventory in Section 2.2 produces this stale,
planning-only baseline:

```text
global=149200/189647=78.67%
proposed_core=147898/187949=78.69% instrumented_files=363
required_core_covered=178552 provisional_deficit=30654
```

The global lower bound is `ceil(0.80 * 189647) - 149200 = 2518`
additional covered lines. The proposed core lower bound is 30,654. Core work
also counts toward global coverage, so implementation remains core-first and
measures both totals after every batch. These numbers MUST NOT enter PR
Threshold-Proof; active BR-196/BR-201 changed core paths after this report.

The exact source-cardinality audit commands are:

```bash
# All 36 top-level library directories must classify as core.
find src -mindepth 1 -maxdepth 1 -type d ! -name bin \
  -exec basename {} \; | sort

# Every root Rust file and every auto-discovered bin must appear exactly once in
# the core/global-only exact-file registries.
find src -maxdepth 1 -type f -name '*.rs' -print | sort
find src/bin -maxdepth 1 -type f -name '*.rs' -print | sort
find src -mindepth 1 -maxdepth 1 -type d ! -name bin | wc -l
find src -maxdepth 1 -type f -name '*.rs' | wc -l
find src/bin -maxdepth 1 -type f -name '*.rs' | wc -l
```

Current count output (path lists are frozen explicitly in Section 2.2):

```text
36
29
16
```

The final inventory generation is not duplicated here; Section 2.4 is its one
exact authority and creates it before hashing. Until `--print-inventory` exists,
classifies every `src/**/*.rs` exactly once with same-run zero proof, and
receives independent review, even the proposed figures above remain provisional
rather than auditable release evidence.

The full provisional top-20 core ranking is generated from the dated report by
this exact diagnostic command. It repeats the ten frozen GlobalOnly identities
only to reproduce the pre-implementation planning result; release ranking uses
`CoverageScopeRegistry`, never this jq filter:

```bash
TOP20_FILE=$(mktemp "${TMPDIR%/}/br202-top20.XXXXXXXX")
trap 'rm -f "$TOP20_FILE"' EXIT
jq -r '
  .data[0].files[]
  | (.filename | sub("^.*/stock_analysis/"; "")) as $path
  | select($path | startswith("src/"))
  | select((["src/bin/agent_test.rs",
      "src/bin/boll_macd_backtest.rs",
      "src/bin/rsi_optimize.rs",
      "src/bin/v14_e2e.rs",
      "src/bin/winrate_simulator.rs",
      "src/gate_d_chain_analysis_regression.rs",
      "src/gate_d_event_cli_regression.rs",
      "src/gate_d_event_envelope_regression.rs",
      "src/gate_d_score_breakdown_regression.rs",
      "src/gate_d_veto_chain_regression.rs"] | index($path)) == null)
  | [(.summary.lines.count - .summary.lines.covered),
      .summary.lines.covered, .summary.lines.count, $path]
  | @tsv' target/coverage/coverage.json \
  | LC_ALL=C sort -k1,1nr -k4,4 \
  | head -20 > "$TOP20_FILE"
cat "$TOP20_FILE"
awk '{sum += $1} END {print "top20_missed_sum=" sum}' "$TOP20_FILE"
```

Output:

```text
3449	9810	13259	src/bin/monitor/push_templates.rs
2759	3651	6410	src/bin/monitor/main.rs
2581	5305	7886	src/database/selection_v2_repository.rs
1915	1596	3511	src/data_gateway/outcome_daily_bars.rs
1825	5356	7181	src/selection/schema_v2.rs
1623	390	2013	src/database/selection_v2_read_model.rs
1326	3423	4749	src/bin/monitor/notify.rs
1230	5230	6460	src/durable_delivery/coordinator.rs
1102	2173	3275	src/database/global_schema_v1.rs
868	1298	2166	src/selection/outcome_v2.rs
649	764	1413	src/data_gateway/historical_bars.rs
633	1796	2429	src/data_gateway/market_capabilities.rs
505	2017	2522	src/database/global_schema_catalog_v1.rs
458	1097	1555	src/data_gateway/chain_intelligence.rs
450	478	928	src/data_gateway/board.rs
408	1958	2366	src/data_gateway/capital.rs
407	614	1021	src/data_gateway/magic_tdx_t0.rs
380	1346	1726	src/monitor/news_ai.rs
375	1583	1958	src/selection/audit.rs
342	1278	1620	src/selection/ingress_v2.rs
top20_missed_sum=23285
```

The top 20 files contain 23,285 missed lines, less than the 30,654-line proposed
core deficit. Therefore a top-20-only effort is mathematically insufficient even
if every line became covered. Ranking controls work order only; it never changes
the metric or truncates the measured file set.

The old plan is stale by direct evidence. Its line-5 historical warning repeats
the search terms, so the reproducible count deliberately excludes Markdown
blockquote warnings and counts only actionable plan references:

```bash
rg -n 'rustdx|data_provider|PR #2' \
  docs/superpowers/plans/2026-07-18-gate-d-coverage-closure.md \
  | rg -v '^[0-9]+:>' \
  | wc -l
```

```text
12
```

No task may recreate an old provider merely to execute an old coverage item.

The stale report also cannot establish any zero-instrumented exception. The
current tree/report cardinality diagnostic is:

```bash
find src -type f -name '*.rs' -print | wc -l
jq -r '.data[0].files[].filename
  | sub("^.*/stock_analysis/"; "")' target/coverage/coverage.json \
  | sort -u | wc -l
comm -23 \
  <(find src -type f -name '*.rs' -print | sort) \
  <(jq -r '.data[0].files[].filename' target/coverage/coverage.json \
    | while IFS= read -r report_path; do realpath "$report_path"; done \
    | sed "s#^$PWD/##" | sort -u) \
  | wc -l
```

```text
408
373
35
```

The exact normalized set difference is 35; lexical `..` report identities are
resolved back to their repository paths before `comm`. This is only stale
cardinality evidence: it does not prove that any missing path has zero
executable lines. In
particular, active files added after the report and declaration-only-looking
`mod.rs` files are treated identically as unproved report omissions. Every one
fails closed until the same-source-SHA proof in Section 2.2 succeeds.

## 2. Coverage authority and scope

### 2.1 Fixed policy floors

`GLOBAL_MIN = 80.0` and `CORE_MIN = 95.0` are policy floors. Command-line
arguments may raise a floor for diagnostics but may not lower it. A requested
`--global-min 79.99` or `--core-min 94.99` exits 2 before reading success as a
release result. Tests that currently pass lower values to inspect file inclusion
must be rewritten to use the policy floors.

### 2.2 Core ownership

The checker owns one deep classification seam, `CoverageScopeRegistry`. Its
interface is: normalize one repository path (including lexical `..` from
`#[path]` coverage rows), classify it exactly once as `Core` or `GlobalOnly`,
declare whether it must have instrumented lines, and return the complete sorted
inventory. Callers do not duplicate prefix logic. The registry has closed
directory, exact-file, and exact zero-instrumented-file sets.

All 36 top-level library directories are core directory prefixes:

```text
src/agent/
src/analyzer/
src/app/
src/auth/
src/breakout/
src/bus/
src/data_gateway/
src/data_provider/
src/database/
src/decision/
src/durable_delivery/
src/event/
src/indicators/
src/llm/
src/market_analyzer/
src/monitor/
src/news/
src/notification/
src/opportunity/
src/performance/
src/pipeline/
src/portfolio/
src/push_l1/
src/push_l2/
src/push_l4/
src/push_l5/
src/push_l6/
src/push_l7/
src/registry/
src/review/
src/risk/
src/search_service/
src/selection/
src/signal/
src/strategy/
src/trading/
```

`src/bin/monitor/` is also a core prefix. It is kept separate because other
auto-discovered binaries require exact disposition.

Exact root core files are all production library/default-binary roots:

```text
src/announcement.rs
src/broker.rs
src/calendar.rs
src/capital_flow.rs
src/chart_generator.rs
src/cli.rs
src/company_financials.rs
src/company_metrics.rs
src/config.rs
src/deep_analyzer.rs
src/enums.rs
src/errors.rs
src/http_client.rs
src/lhb_analyzer.rs
src/lib.rs
src/main.rs
src/market_data.rs
src/models.rs
src/schema.rs
src/sharpe_calculator.rs
src/traits.rs
src/trend_analyzer.rs
src/types.rs
src/util.rs
```

Exact operational binaries are core:

```text
src/bin/backfill_daily.rs
src/bin/backfill_predictions.rs
src/bin/confirm_daily_change.rs
src/bin/import_real_account_snapshot.rs
src/bin/import_user_account_summary.rs
src/bin/import_user_position_snapshot.rs
src/bin/lhb_query.rs
src/bin/migrate_selection_v2.rs
src/bin/produce_winrate_samples.rs
src/bin/run_closing_valuation.rs
src/bin/selection_live_probe.rs
```

This is intentionally broader than the old 15-prefix checker. `CLAUDE.md`
identifies Portfolio, Market, Signal, Opportunity, Review, Decision, Risk, and
Breakout as live contexts; the producer-to-delivery chain additionally crosses
agent/analyzer, app, bus, news, notification, registry, search, strategy, all
push tiers, database, event, and durable delivery. Root modules own data,
calendar, configuration, CLI, analysis, and the default binary. `src/lib.rs` is
not declaration-only: it implements the production async runtime bridge, and
`src/opportunity/chain_mapper.rs:225` calls it. Exact operational bins write or
read production market/account/confirmation/valuation/review evidence, so they
are core even when normally invoked manually.

The only `GlobalOnly` exact source files are test/research tools rather than live
trading/data authority:

```text
src/bin/agent_test.rs
src/bin/boll_macd_backtest.rs
src/bin/rsi_optimize.rs
src/bin/v14_e2e.rs
src/bin/winrate_simulator.rs
src/gate_d_chain_analysis_regression.rs
src/gate_d_event_cli_regression.rs
src/gate_d_event_envelope_regression.rs
src/gate_d_score_breakdown_regression.rs
src/gate_d_veto_chain_regression.rs
```

Evidence-based disposition:

| Exact path/group | Disposition | Evidence/reason |
| --- | --- | --- |
| `src/bin/agent_test.rs` | global-only | credential-driven AgentRunner diagnostic; not a monitor producer or release authority |
| `boll_macd_backtest.rs`, `rsi_optimize.rs`, `winrate_simulator.rs` | global-only | file headers declare backtest/optimizer/simulator; they remain in global 80% and cannot be excluded from the workspace report |
| `src/bin/v14_e2e.rs` | global-only | file header declares an in-memory/SQLite architecture test binary, not the production monitor |
| five `src/gate_d_*_regression.rs` files | global-only | each is included only through a `#[cfg(test)]` parent `#[path]`; it tests core behavior but is not production code |
| `src/bin/selection_live_probe.rs` | core | read-only production Magic TDX capability probe; it validates the live selection data contract |
| remaining one-shot bins | core | account/position import, data freshness backfill, manual change confirmation, migration, valuation, LHB and verified outcome operations affect production evidence |

No directory or file may disappear silently. At default invocation the checker
walks the fixed reviewed worktree's `src/**/*.rs`, normalizes each path, and
requires exactly one classification. A new/unclassified path, a path matching
both a prefix and exact disposition, or a report source absent from the
inventory exits 2. Exact matches prevent `src/broker.rs.bak` or a similarly
named path from being counted accidentally. Tests execute the checker from a
minimal temporary repository containing a complete fixture inventory; the
release interface has no repository-root override that could point at a reduced
tree.

`CoverageScopeRegistry::ZERO_INSTRUMENTED` is a closed, source-controlled exact
path set and starts empty. A classified path absent from the JSON report exits
2 unless every condition below is true in artifacts generated from the same
fixed source SHA and the same merged coverage profile:

1. the path is explicitly registered in `ZERO_INSTRUMENTED`; omission never
   auto-populates the set;
2. a normalized compiler dep-info manifest proves the path participated in at
   least one instrumented workspace/all-features target;
3. the JSON report has no normalized row for the path and the complete
   `llvm-cov show -format=text` artifact has no executable coverage region for
   it;
4. the proof records the path's source-byte SHA-256, the fixed source commit,
   JSON report SHA-256, LLVM-show manifest SHA-256, dep-info manifest SHA-256,
   and zero-set registry SHA-256;
5. the checker reconciles the exact sets: every report-missing source is one
   proved registered zero path, every registered zero path is report-missing
   and proved, and there is no extra proof row.

The wrapper invokes `cargo llvm-cov report --all-features --text`
immediately after the JSON export, without cleaning or rerunning tests, so both
views consume the same merged profile. It creates a normalized, sorted
`llvm-cov-show.manifest`, a normalized, sorted
`instrumented-dependencies.manifest`, and deterministic
`zero-instrumented.json`; none contains a wall-clock time, nonce path, or
absolute worktree prefix. The show-manifest emitter reconciles the show output
against the complete source inventory and emits exactly one row per source,
including `artifact_present=false` rather than silently dropping an absent show
file. Each row freezes schema version, source SHA, normalized source path,
nullable annotated-artifact byte hash and exact executable region count; each
dependency-manifest row freezes schema version, source SHA,
normalized source path and the sorted instrumented target identities that named
it. Their hashes are release evidence. The checker emits the zero proof only
after all five conditions pass. A report omission, show
omission, declaration-only-looking `mod.rs`, platform-gated source, or the stale
35/408 cardinality gap is never by itself proof of zero executable lines.

Bootstrap is fail-closed. With an empty zero set and any report omission,
`--emit-zero-proof` exits 2 and does not create a final proof. A separate
`--diagnose-report-missing` mode may emit a sorted, non-authoritative candidate
artifact containing source hashes plus compiler/show observations, but it
always exits 2 and the default checker refuses that artifact type. An
independent BR-202 review must approve each exact path before a later source
commit adds it to `ZERO_INSTRUMENTED`; the complete wrapper is then rerun from
that new fixed SHA. The candidate diagnostic can discover a possible entry but
can never satisfy Gate D.

Adding an exact zero entry is a reviewed BR-202 filter change, not an automatic
coverage-tool repair. The mutation contract is two-sided: stale zero proof
after any source-byte change exits 2 on the source hash, and a fresh same-SHA
run where that path gains one executable line exits 2 because the path now has
a report/show region while still being zero-listed. The named regression is
`zero_listed_source_gaining_executable_line_is_invalid_report`.

The complete sorted inventory is emitted with relative path, Core/GlobalOnly,
report-required/zero-instrumented, covered, count, missed, classification
reason, and zero-proof identity. Its SHA-256 is release evidence.
Zero-instrumented rows contribute no synthetic denominator. The checker rejects
a missing inventory/proof/show/dependency artifact, hash mismatch, missing
required core row, non-exact set reconciliation, or empty core set.

### 2.3 Strict report and CLI validation

The parser accepts exactly one llvm-cov run (`data.len() == 1`). It rejects
before threshold comparison:

- malformed JSON; absent/extra run; missing files/totals/line fields;
- threshold values that are strings, booleans, NaN, infinity, zero, negative,
  or below the immutable floors (finite fractional percentages at or above the
  floors remain valid, for example `95.5`);
- line counters that are strings, booleans, NaN, infinity, fractional, negative,
  or outside exact integer range; `count == 0` is invalid, while
  `covered == 0` is valid and normally fails the percentage gate;
- `covered > count` for any file or total;
- totals whose count/covered do not exactly equal the sum of every file row;
- duplicate identities after slash, dot-component, repository-marker, and
  canonical relative-path normalization;
- an absolute/report path that cannot be proven to belong to the fixed source
  inventory.

The checker never trusts llvm-cov's reported percentage; it computes using exact
integer counters and compares without presentation rounding.

Counter representation is frozen across Python, Rust, JSON Schema, and any
future verifier. `covered`, `count`, and every derived aggregate are JSON number
tokens matching `0|[1-9][0-9]*`: no sign, decimal point, exponent, leading zero,
quoted number, `null`, or boolean is accepted. The semantic range is the
cross-language exact-integer subset of unsigned 64-bit values,
`0..=9_007_199_254_740_991` (`2^53-1`). Parsing first validates the raw token,
then converts with an arbitrary-precision integer parser, then applies the upper
bound; it never passes through IEEE-754. `count` is additionally `1..=2^53-1`,
and `covered <= count`. File sums use arbitrary-precision checked accumulation
and are rejected if either aggregate exceeds `2^53-1`; JSON totals must equal
those recomputed aggregates exactly.

Threshold comparison also avoids float and overflow. Policy floors are canonical
basis points `8000` and `9500`; a permitted higher decimal CLI value is parsed
from its ASCII decimal spelling into an integer numerator/10,000 with at most
four fractional digits. The gate compares
`covered * 100_0000 >= count * threshold_numerator` using arbitrary-precision
integers. Display rounding occurs only after the boolean result and cannot alter
it. Required boundary fixtures include 0 covered, count 1, covered=count,
`2^53-1`, `2^53`, `u64::MAX`, aggregate sum exactly `2^53-1`, aggregate sum
`2^53`, `-0`, `-1`, `01`, `1.0`, `1e0`, strings, booleans, and null. Exactly
`2^53-1` is accepted when the other invariants hold; every token or sum above it
exits 2 before threshold output.

### 2.4 Denominator integrity

The Gate D command is exactly the workspace/all-features command in the
engineering rules. This design prohibits:

- `#[coverage(off)]`, filename-ignore regexes, `cargo llvm-cov --exclude`, or
  feature/target omission;
- deleting behavior or tests solely to change the ratio;
- moving core code to an unregistered directory;
- reporting library-only, package-only, focused, or stale coverage as Gate D;
- rounding 79.995% or 94.995% upward;
- using a custom lower `--global-min` or `--core-min` as release evidence.

Gate D runs against one fixed reviewed commit in the isolated wrapper. The
wrapper records the commit before and after report generation and requires
equality; operators invoke only the first command below, while the remaining
commands are the wrapper's fixed internal sequence. `set -euo pipefail`, the
invocation-unique detached worktree, and required-absent fixed artifact paths
ensure a failed generation cannot leave a stale final inventory or proof for a
later hash step while preserving the engineering-rule path
`target/coverage/coverage.json`:

```bash
bash tools/coverage/run_isolated_gate.sh

# internal to the detached worktree wrapper
set -euo pipefail
COVERAGE_GATE_SHA_BEFORE=$(git rev-parse HEAD)
install -d target/coverage
GATE_RUN_DIR=target/coverage
REPORT=$GATE_RUN_DIR/coverage.json
SHOW_DIR=$GATE_RUN_DIR/llvm-cov-show
SHOW_MANIFEST=$GATE_RUN_DIR/llvm-cov-show.manifest
DEPENDENCY_MANIFEST=$GATE_RUN_DIR/instrumented-dependencies.manifest
ZERO_PROOF=$GATE_RUN_DIR/zero-instrumented.json
INVENTORY=$GATE_RUN_DIR/core-inventory.txt
test ! -e "$REPORT"
test ! -e "$SHOW_DIR"
test ! -e "$SHOW_MANIFEST"
test ! -e "$DEPENDENCY_MANIFEST"
test ! -e "$ZERO_PROOF"
test ! -e "$INVENTORY"

cargo llvm-cov --workspace --all-features --json \
  --output-path "$REPORT" -- --test-threads=1
cargo llvm-cov report --all-features --text \
  --output-dir "$SHOW_DIR"
python3 tools/coverage/check_thresholds.py \
  --source-sha "$COVERAGE_GATE_SHA_BEFORE" \
  --emit-show-manifest "$SHOW_DIR" \
  > "$SHOW_MANIFEST.tmp"
mv "$SHOW_MANIFEST.tmp" "$SHOW_MANIFEST"
python3 tools/coverage/check_thresholds.py \
  --source-sha "$COVERAGE_GATE_SHA_BEFORE" \
  --emit-instrumented-dependency-manifest "$CARGO_TARGET_DIR" \
  > "$DEPENDENCY_MANIFEST.tmp"
mv "$DEPENDENCY_MANIFEST.tmp" "$DEPENDENCY_MANIFEST"
python3 tools/coverage/check_thresholds.py "$REPORT" \
  --source-sha "$COVERAGE_GATE_SHA_BEFORE" \
  --show-manifest "$SHOW_MANIFEST" \
  --dependency-manifest "$DEPENDENCY_MANIFEST" \
  --emit-zero-proof > "$ZERO_PROOF.tmp"
mv "$ZERO_PROOF.tmp" "$ZERO_PROOF"
python3 tools/coverage/check_thresholds.py "$REPORT" \
  --source-sha "$COVERAGE_GATE_SHA_BEFORE" \
  --zero-proof "$ZERO_PROOF" --print-inventory > "$INVENTORY.tmp"
test -s "$INVENTORY.tmp"
mv "$INVENTORY.tmp" "$INVENTORY"
python3 tools/coverage/check_thresholds.py "$REPORT"
COVERAGE_GATE_SHA_AFTER=$(git rev-parse HEAD)
test "$COVERAGE_GATE_SHA_BEFORE" = "$COVERAGE_GATE_SHA_AFTER"
shasum -a 256 "$REPORT"
shasum -a 256 "$SHOW_MANIFEST"
shasum -a 256 "$DEPENDENCY_MANIFEST"
shasum -a 256 "$ZERO_PROOF"
shasum -a 256 "$INVENTORY"
```

The manifest emitters normalize to repository-relative paths, sort bytewise,
hash each artifact's bytes, and reject symlinks, duplicate normalized paths,
absolute output paths, or paths outside the detached tree. The zero-proof
emitter binds the JSON, show and dependency manifests to the same source SHA and
exits 2 before rename if exact reconciliation fails. Thus `INVENTORY` is always
created before its hash is computed and a failed temporary file is never release
evidence.

The required one-positional-argument checker command resolves only the exact
sibling names shown above and verifies their embedded source SHA and hashes; a
missing/extra/mismatched companion exits 2. Release mode has no flags to select
an alternate companion directory. Construction flags are available only to the
preceding emitter steps and isolated checker fixtures, so the engineering-rule
command remains exact and fail-closed.

The report, show/dependency manifests, zero proof, inventory, build/profile,
isolation, behavior and fixed-SHA hashes go in PR evidence. Detached-tree
intermediates remain local; only the verified Section 2.5.2 bundle is the CI
release artifact.

#### 2.4.1 Source/build/profile cryptographic chain

`coverage.json` bytes and a companion self-reported source SHA are insufficient
release evidence. The wrapper exports `build-profile.v1.json` plus the following
regular, single-link artifacts from the same invocation; absolute worktree paths
are represented only by normalized repository-relative or bundle-relative IDs:

- `source/tree.manifest.z` is the bytewise-sorted NUL-delimited `git ls-tree`
  stream for the closed fixed-source input set at `S`. For the repository root
  and every Cargo workspace-member/path-dependency root, that set includes all
  Cargo manifests plus `Cargo.lock`, `.cargo/**`,
  `rust-toolchain`/`rust-toolchain.toml` when present, every `build.rs`,
  `src/**`, **all `tests/**` bytes**, `benches/**`, `examples/**`, `config/**`,
  `tools/**`, `.github/workflows/coverage.yml`, and `README.md`.
  `source/files.manifest` records every such regular input path, Git mode/blob
  ID, byte length and SHA-256, not merely paths that appear in coverage rows;
- `build/generated-inputs.manifest` records every build-script/proc-macro output,
  generated source, response file and compile-time environment/file input named
  by Cargo JSON, rustc dep-info or the process/file trace. Each row binds the
  producing invocation, normalized generated identity, size and SHA-256. A
  compiler/test read under the repository, worktree, Cargo home or target root
  that is absent from either the fixed-source set or generated-input manifest
  fails; absolute or ambient inputs outside the closed toolchain/vendor/run-root
  policy fail;
- `build/cargo-metadata.json`, exact `Cargo.toml`, `Cargo.lock`, `.cargo/config.toml`,
  Gate-D vendor config, dependency snapshot, and their byte/tree hashes bind the
  resolved package graph;
- `build/toolchain.json` records exact `rustc -vV`, `cargo -Vv`,
  `cargo llvm-cov --version`, `llvm-cov --version`, and
  `llvm-profdata --version` outputs and executable byte hashes;
- `build/compile-invocations.jsonl` is generated from the OS process trace and
  Cargo JSON messages. It records every rustc/link invocation, target triple,
  package/target/kind, enabled feature set, coverage-related encoded rustflags,
  remap flags, output identity, and relevant sanitized environment. The wrapper
  requires the canonical workspace/all-features/test-thread arguments and
  rejects a target, package, feature, doctest, or test-binary omission;
- `build/objects.manifest` records every executable/object carrying an LLVM
  coverage map, its full-file SHA-256, byte length, exact read-only mode,
  platform build ID (`ELF NT_GNU_BUILD_ID`, Mach-O `LC_UUID`, or PE CodeView
  GUID+age), coverage-map section hash, contributing Cargo target, compile
  invocation and the exact `build/objects/<sha256>` payload path. The
  content-addressed store identity is `gate-d-object-store/v1`: it contains one
  regular, single-link, read-only byte copy for every unique object hash; the
  lowercase 64-hex filename must equal the streamed byte hash and duplicate
  logical objects reference the same blob. Missing/extra/writable/link payload,
  size/mode/hash/build-ID/map mismatch or a manifest row without a blob fails.
  Every blob is copied, file-fsynced, directory-fsynced and re-opened through a
  pinned no-follow descriptor before build cleanup. A platform/object without
  an extractable stable build ID fails; a path or JSON assertion is not a build ID;
- `profiles/raw.manifest` and `profiles/raw/**` retain every nonempty profraw
  file with byte hash, size, process/test identity, and referenced binary build
  IDs. `profiles/merged.profdata` and `profiles/merged.manifest` bind the exact
  sorted raw set, merge command, merged bytes, and `llvm-profdata show --binary-ids`
  output. Unknown, duplicate, zero-byte, foreign-build-ID, or unconsumed profiles
  fail;
- `build/coverage-mapping.manifest` is independently emitted from the exported
  objects and maps every normalized coverage filename/region to object build ID,
  object hash, source blob/hash, and merged-profile hash. It reconciles exactly
  with the report/show/inventory sets.

Before dependency bootstrap, before Cargo, immediately after coverage/profile
generation, and immediately before export, the wrapper runs a pathspec-limited
`git status --porcelain=v2 -z --untracked-files=all` over the complete fixed
input set and requires empty output. Intentional report/target/run artifacts are
outside those input roots. It also regenerates `source/tree.manifest.z` and
`source/files.manifest` from the detached tree before and after the run and
requires byte equality to `git ls-tree S`; a modified or untracked test,
build-script, example, benchmark, config, tool or other input fails even though
`git rev-parse HEAD` is unchanged. The file trace must reconcile every actual
compiler/test read to a fixed or generated input. These checks detect dirty
detached-worktree test bytes rather than trusting the commit name alone.

The verifier does not trust the declarations. With an installed tool executable
whose bytes equal the recorded executable hash/version, it (1) hashes Git object
`S`, regenerates the complete fixed-source input manifests including tests, and
reconciles generated inputs and the compiler/test read trace; (2) opens every
retained `build/objects/<sha256>` blob, streams its hash, extracts its build ID
and coverage-map section, and reconciles Cargo target output to compile trace; (3)
hashes and merges the bytewise-sorted raw profiles into a fresh temporary
profdata, requiring its SHA-256 and binary-ID set to equal the export; (4) runs
`llvm-cov export --instr-profile <remerged-profdata>` over the exact ordered
retained object blob set, normalizes only the fixed detached-worktree prefix, and requires the
resulting file rows/counters to equal `coverage.json`; and (5) resolves every
coverage mapping filename to the Git blob at `S` and requires the source hash.
Only this independently reproduced object+profile→report relation proves that
the report came from binaries compiled from `S` under the recorded dependency,
feature, target, and toolchain inputs.

Required forgery negatives replace one source or test blob while keeping a
claimed SHA, add an untracked test/build input without changing `HEAD`, change a
generated input or omit an observed read,
swap a same-name binary from another build, edit/remove/add a compile invocation
or feature/target, remove/change/add a retained object blob, replace an object
build ID or mapping section, omit/add/swap a
profraw, substitute profdata, change Cargo metadata/lock/config/tool versions,
and rewrite all companion JSON hashes to match the forged bytes. Each exits 2:
the independently recomputed Git/object/profile/mapping relation still differs.

#### 2.4.2 Offline dependency bootstrap

A fresh empty `CARGO_HOME` plus `CARGO_NET_OFFLINE=true` is not assumed to contain
the Git-pinned Magic crates. Gate B creates a source-controlled, release-hashed
`vendor/gate-d/` using a separately authorized maintenance command equivalent to
`cargo vendor --locked --versioned-dirs`; it must include crates.io dependencies
and the complete Magic repository packages pinned at
`5f1ce93656a55854c844065390520cd4aecd9a14`. Network and credentials are allowed
only during that reviewed maintenance action, never during a Gate D run.

`.cargo/gate-d-vendor-config.toml` is the sole Gate-D Cargo source-replacement
template. It replaces both crates.io and the exact Cargo.lock Git source URL,
and its only substitution token is `${GATE_D_VENDOR_DIR}`. The wrapper verifies
the template hash, substitutes the canonical no-symlink `vendor/gate-d` under
the currently verified source root, and writes a 0600 expanded config into the
invocation's empty Cargo home. The detached worktree gets a second expansion to
its byte-identical vendor tree; normalizing that one allowed absolute prefix
back to the literal token must make the two configs byte-identical. No operator
path override exists. The template/expanded configs contain no registry token,
credential helper, network mirror or path outside the verified source tree and
are hashed by `tools/coverage/dependency_snapshot.v1.json`. The snapshot records
schema/version, exact Cargo.lock SHA-256, every locked package
name/version/source/checksum, each vendored file path/mode/size/SHA-256, every
`.cargo-checksum.json`, the aggregate bytewise tree hash, and the pinned Magic
commit. Symlinks, submodules, `.git`, credential/config files, writable files,
unexpected packages/files, a lock package without exactly one vendor row, or a
vendor row absent from Cargo.lock fail.

After the persistent journal in Section 3.4.1 is pinned but before `RUN_ROOT` or
a worktree exists, the wrapper verifies the clean caller tree is `S`, verifies
the source-tree vendor/template/snapshot hashes, creates an exclusive
`<journal-root>/bootstrap-cargo-home/`, expands only the credential-free config,
makes the vendor tree read-only (and read-only bind mounts it where supported),
then runs `cargo metadata --locked --offline --format-version 1` against `S`.
After the detached tree exists, the coverage child uses a fresh RUN_ROOT Cargo
home expanded to that tree and repeats exact graph equality before compilation.
Both normalized expanded configs and metadata outputs are build-profile
evidence. The resolved graph must exactly equal Cargo.lock and the snapshot.
Missing/corrupt dependencies, attempted
network/Git access, lock drift, a writable vendor source, or a credential lookup
records `dependency_bootstrap_failed` in the persistent journal, exits 2, and
creates no `RUN_ROOT`. The later coverage build uses that same pinned config and
read-only vendor; it never expects an ambient Cargo cache.

#### 2.4.3 Closed host execution inputs

Rust/Cargo/LLVM hashes alone do not close a native build. V1 therefore adds
`gate-d-host-execution-inputs/v1`, generated under
`tools/coverage/host_execution_policy.v1.json` and validated by
`tools/coverage/host_execution_inputs.schema.v1.json`. It covers every process
executed by the wrapper, compiler, linker, tests, verifier, archive writer and
signer, including the shell, Git, Python interpreter and imported modules,
linker/clang, system archive reader, `install`, `mv`, `mktemp`, `shasum`, `cmp`,
`awk`, `sed`, `sort`, `head`, `find`, `realpath`, `jq`, `rg`, platform sandbox
and trace tools, dynamic loader and every transitive executable or shared
library actually opened. A basename or `PATH` directory is not an identity.

The manifest records for every executable/library its canonical absolute path,
device/inode observed locally, file type, mode, byte length, streamed SHA-256,
version output or explicit `version_unavailable`, loader identity and complete
transitive dynamic-dependency edges. Device/inode is local replay evidence only;
portable verification uses path class plus bytes/hash/version and never requires
the downloader's inode. macOS additionally records the complete canonical
CommandLineTools/Xcode SDK tree selected by `xcrun --show-sdk-path`, clang/linker
bytes, `otool -L`/loader closure and SDKSettings identity. Linux records the
selected compiler/linker/sysroot, interpreter/loader, `ldd` closure and every
resolved system library. Each SDK/sysroot row includes relative path, type,
mode, size and SHA-256; symlinks record target text and must resolve inside the
same closed root. The bytewise tree hash is checked before Cargo and after
portable archive/signature publication.

`host/environment.v1.json` freezes OS family/release, kernel release/build,
architecture, target triple, CPU feature string, filesystem case-sensitivity,
timezone database identity, `LANG`/`LC_ALL`/`TZ`, page size and the exact
sanitized child environment. Archive generation itself fixes
`LC_ALL=C`, `TZ=UTC`, numeric owner/group zero and timestamp zero. These host
facts are compatibility inputs, never substitutes for source identity. A host
change requires a new run; it cannot be normalized away.

The persistent journal is created first. Its first synced record includes the
absolute bytes/hash/version and dynamic-loader closure of the phase-zero shell,
Git and Python used to create it. The independent verifier reopens and rehashes
those phase-zero inputs; they are not trusted because the journal named them.
The wrapper then runs the exact feature probe before dependency bootstrap or
`RUN_ROOT` creation:

```bash
python3 tools/coverage/capture_host_execution_inputs.py probe \
  --schema tools/coverage/host_execution_inputs.schema.v1.json \
  --policy tools/coverage/host_execution_policy.v1.json \
  --platform auto \
  --output-fd "$HOST_PROBE_FD"
# HOST_EXECUTION_PROBE status=PASS platform=<linux|macos> \
# enforcement=<backend-id> trace=<backend-id> dynamic_deps=closed \
# sdk_or_sysroot=closed phase_zero=closed unknown_tools=0 unknown_reads=0
```

Linux v1 admits only an exact `strace` backend executing
`strace -ff -qq -yy -s 4096 -e trace=%file,%process,%network` inside the private
network namespace. macOS v1 admits only the source-controlled deny profile via
`/usr/bin/sandbox-exec -f <pinned-profile>` plus the policy-selected
`/usr/sbin/dtrace -q -s <pinned-script>` file/process/network trace. The macOS
probe performs a real child open/exec/loopback/denied-connect calibration and
requires every event in the trace; lacking DTrace privilege/entitlement or a
readable event stream exits 2 before dependency bootstrap. No unsupported
fallback to `fs_usage` summaries or hash-only inference is allowed. Thus the
contract is executable on an authorized macOS host and fails honestly on an
unauthorized one; CI's Ubuntu host uses the Linux backend.

After the probe, the complete run is executed once under that backend. The
capture command is fixed:

```bash
python3 tools/coverage/capture_host_execution_inputs.py run \
  --schema tools/coverage/host_execution_inputs.schema.v1.json \
  --policy tools/coverage/host_execution_policy.v1.json \
  --source-sha "$COVERAGE_GATE_SHA_BEFORE" \
  --journal-fd "$LIFECYCLE_FD" \
  --trace-dir-fd "$HOST_TRACE_DIR_FD" \
  -- tools/coverage/run_isolated_gate.sh --internal-covered-child
# HOST_EXECUTION_INPUTS status=PASS tools=<n> libraries=<n> \
# sdk_files=<n> exec_events=<n> open_read_events=<n> \
# unknown_tools=0 unknown_reads=0 pre_post_equal=true
```

The trace and Cargo/rustc dep-info are reconciled in both directions: every
observed exec/open/read belongs to fixed source, generated input, read-only
vendor, Rust toolchain, the fully manifested host/SDK roots, or the
invocation-unique run root; every declared executed tool was observed. Unknown,
missing, unreadable, mutable, path-swapped, extra or hash-drifted inputs exit 2.
The exported bundle retains `host/execution-inputs.v1.json`,
`host/environment.v1.json`, `host/tools.manifest`,
`host/dynamic-libraries.manifest`, `host/sdk-or-sysroot.manifest` and
`host/trace.jsonl`. Forgery tests replace a linker, SDK header/library, Python
module, shell utility, tar reader, dynamic dependency, locale/TZ input, OS fact
or trace event and require exit 2 after all stored hashes are rewritten.

### 2.5 Single release entrypoint and CI/README migration

`tools/coverage/run_isolated_gate.sh` is the only public release-coverage
entrypoint and the only process allowed to create the candidate
`release-attestation.v1.json`, publish the local
`gate-d-local-publication-terminal/v1`, append its later durable journal
confirmation, construct the canonical portable archive, obtain the detached
portable-terminal signature, or emit the exact release PASS marker. External
`cargo llvm-cov` is an independent binary: the wrapper cannot and does not claim
to change its exit status. Raw llvm-cov and direct
`tools/coverage/check_thresholds.py` commands may run and may return their normal
diagnostic success/failure codes, but their outputs have `release_authority=false`
by construction and can never create a release attestation, context, or PASS.
Relabeling a raw result in CI/prose does not grant authority.

This keeps the raw baseline commands required by
`docs/ENGINEERING_RULES_V2.md §5`: the wrapper executes that exact
workspace/all-features llvm-cov generation followed by the one-positional
checker, then adds build/profile, isolation, export, cleanup, and attestation
verification. The engineering-rules snippet remains an active normative
diagnostic/generation sequence, not a second release minter. The checker may
consume a wrapper-created invocation-unique internal context FD while emitting
component manifests; that FD binds wrapper PID/parent, detached-worktree
device/inode, `S`, run directory, phase, and a random 256-bit nonce. It is never
accepted by pathname/env token, never independently becomes release authority,
and is closed before export. Only the wrapper, after final verification and
cleanup, may perform the ordered local-bundle and local-terminal
publication protocol in Section 3.4.1. Local durable publication does not become
portable authority: portable authority exists only when canonical archive bytes,
the detached portable terminal and its approved signature all verify.

#### 2.5.1 Complete invocation inventory

Gate B adds `tools/coverage/entrypoint_inventory.v1.json`. Each occurrence has
an exact tracked path, parsed command/block identity, semantic digest, and one
closed disposition: `release_wrapper`, `active_internal_diagnostic`,
`test_fixture`, or `historical_only`. The initial complete disposition is:

| Caller class | Exact active paths | Required disposition/migration |
| --- | --- | --- |
| CI release caller/upload | `.github/workflows/coverage.yml` | replace both raw run steps with one wrapper call; upload the verified bundle under Section 2.5.2, never standalone JSON |
| human release instructions | `README.md` | replace the raw release pair with the wrapper; raw/focused examples, if retained, are labelled diagnostic and cannot mint authority |
| normative raw baseline | `docs/ENGINEERING_RULES_V2.md` | retain the required raw command sequence; inventory marks it `active_internal_diagnostic` and verifies the wrapper contains the same argv contract |
| canonical business rule | `docs/business_rules.md` unique BR-202 row | active normative rule text; not an executable caller, but its entrypoint/artifact tokens and Code paths are digest-bound |
| release minter | `tools/coverage/run_isolated_gate.sh` | sole `release_wrapper`; no alias, Make target, composite action, or second script may mint a candidate attestation, local terminal/journal confirmation, portable archive/terminal/signature, or PASS |
| parser and process tests | `tools/coverage/check_thresholds.py`, `tests/test_coverage_thresholds.rs`, `tests/test_coverage_entrypoints.rs`, `tests/test_gate_d_attestation.rs` | internal diagnostic or test fixture only; test success is not release authority |
| current design and future implementation plan | this file and `docs/superpowers/plans/2026-08-02-gate-d-coverage-closure.md` | active normative design/plan; executable release steps name the wrapper, while explicit internal/raw steps are tagged diagnostic |
| prior coverage plan | `docs/superpowers/plans/2026-07-18-gate-d-coverage-closure.md` | `historical_only`; already superseded by this design and never executable authority |
| every other dated spec/plan coverage invocation | every matching file under `docs/superpowers/specs/**` and `docs/superpowers/plans/**` except the two current paths above | invocation-level `historical_only`; the owning feature design is not erased, but its embedded old raw coverage command cannot authorize release |
| tracked reports/transcripts | `.planning/2026-07-16-event-replay-safety-remediation/progress.md`, `docs/v16.x/v16.x-completion-audit-2026-07-19.md`, and `progress.md` | `historical_only`; quoted commands/results remain evidence of their date only |

The machine scan is repository-wide, not limited to CI and README. It enumerates
every regular path from `git ls-files -z` (including dotfiles) and parses GitHub
workflow `run` scalars, composite actions, shell/Make scripts, root task-runner
files, README and all Markdown fenced shell blocks, Python/Rust subprocess
literals, and prose occurrences of `cargo llvm-cov`, `check_thresholds.py`,
`coverage.json`, `gate-d-runs`, wrapper, release-attestation, or PASS tokens.
Known generated/vendor/binary files are classified by exact path before content
scanning; no broad `docs/**` or dotfile exclusion exists. Every **tracked** hit
must match
exactly one inventory row and semantic digest. A new file, alias, variable-built
command, workflow/composite indirection, changed block, missing row, duplicate
row, or an inventory row with no live hit fails
`tests/test_coverage_entrypoints.rs`. The test also requires all current
spec/plan matches returned from Git object `S` by this command to be explicitly
classified. It never asks `rg` to open a tracked-but-deleted worktree path:

```bash
S=$(git rev-parse HEAD)
git grep -Il -E \
  'cargo llvm-cov|check_thresholds\.py|coverage\.json|gate-d-runs|run_isolated_gate' \
  "$S" -- \
  | sed "s#^$S:##" \
  | LC_ALL=C sort
# expected: every output path passes git ls-files --error-unmatch at S and has
# exactly one parsed inventory row; unclassified=0 duplicate=0 dead_rows=0
```

Two current worktree diagnostics deliberately remain outside the required
inventory because they are not tracked authority:

```bash
for path in \
  docs/handoffs/HANDOFF_2026-07-18_REPOSITORY_SAFETY_CLOSURE.md \
  findings.md
do
  if git ls-files --error-unmatch "$path" >/dev/null 2>&1; then
    printf 'tracked %s\n' "$path"
  else
    printf 'untracked_non_authority %s\n' "$path"
  fi
done
```

```text
untracked_non_authority docs/handoffs/HANDOFF_2026-07-18_REPOSITORY_SAFETY_CLOSURE.md
untracked_non_authority findings.md
```

They supply no release evidence and create no required inventory row. If either
is later tracked with a matching token, the fresh Git-object scan fails until a
reviewed row classifies it; silently ignoring a newly tracked path is forbidden.

This Gate A edit changes only the design and BR-202. CI, README, scripts, tests,
the future plan, and the inventory itself are Gate B work.

#### 2.5.2 Local bundle, portable archive, and standalone-JSON compatibility

The migration is explicitly a standalone-file-to-portable-archive change. The
successful local caller-worktree directory is
`target/coverage/gate-d-runs/<40-lower-hex-S>/<run-id>/`; `run-id` is the
wrapper-generated lowercase 32-byte random hex identity. The immutable local v1
bundle layout is:

```text
manifest.v1.json
release-attestation.v1.json
coverage/coverage.json
coverage/core-inventory.txt
coverage/llvm-cov-show.manifest
coverage/instrumented-dependencies.manifest
coverage/zero-instrumented.json
coverage.json                         # read-only byte-identical compatibility copy
source/tree.manifest.z
source/files.manifest
build/build-profile.v1.json
build/cargo-metadata.json
build/toolchain.json
build/compile-invocations.jsonl
build/generated-inputs.manifest
build/objects.manifest
build/objects/<sha256>                 # gate-d-object-store/v1 retained bytes
build/coverage-mapping.manifest
profiles/raw.manifest
profiles/raw/**
profiles/merged.profdata
profiles/merged.manifest
host/execution-inputs.v1.json
host/environment.v1.json
host/tools.manifest
host/dynamic-libraries.manifest
host/sdk-or-sysroot.manifest
host/trace.jsonl
behavior/authorities.json
behavior/residuals.json
behavior/clusters.json
behavior/decision-sinks.json
behavior/decision-denominator.json
behavior/decision-site-owners.json
behavior/evidence-index.json
behavior/evidence/<cluster-id>.json
isolation/policy.json
isolation/trace.jsonl
isolation/result.json
diagnostics/lifecycle.jsonl
logs/wrapper.log
```

The local publication terminal is deliberately outside the bundle it
describes:

```text
target/coverage/gate-d-local-publication/<S>/<run-id>.terminal.v1.json
```

Its schema identity is `gate-d-local-publication-terminal/v1`. It binds the
exact source SHA, run ID, local canonical final bundle path, local bundle
device/inode, payload-manifest hash, release-attestation hash,
persistent-journal cleanup-terminal hash and the literal successful
bundle-parent-fsync phase. It is a regular single-link read-only local file. A
bundle, including one already renamed to its final run path, is only a local
release candidate until this terminal is atomically published, its parent is
fsynced, and the later `local_publication_terminal_parent_fsynced` journal
record is fdatasynced. Local path/device/inode facts never enter portable
authority and are never compared on a downloader.

`manifest.v1.json` is a closed, sorted list of every allowed payload relative
regular single-link file, size and SHA-256. To avoid a recursive hash, the
schema reserves exactly two control paths outside that payload list:
`manifest.v1.json` itself and `release-attestation.v1.json`; the attestation
hashes the finalized payload manifest, while the manifest never hashes the
attestation. The verifier separately requires and hashes both control files.
Unexpected/missing payload/control paths, links, writable **internal bundle**
files, or a manifest self-entry fail. The compatibility `coverage.json` is a
separate read-only
regular copy whose bytes and hash must exactly equal
`coverage/coverage.json`; it is diagnostic-only and never sufficient without
the manifest and release attestation. Bundle consumers use
`coverage/coverage.json` plus the verifier. A v1 bundle always retains the root
compatibility copy. A later v2 may remove it only after two successful mainline
v1 releases, repository-wide consumer inventory shows zero alias readers, every
external consumer owner acknowledges migration, and a reviewed deprecation
record names the rollback.

Local publication is converted to `gate-d-portable-archive/v1` only after the
later local journal confirmation is durable. The source-controlled
`build_portable_archive.py` writer emits exactly these three outer files:

```text
target/coverage/portable/<S>/<run-id>/coverage-report.v1.tar
target/coverage/portable/<S>/<run-id>/portable-terminal.v1.json
target/coverage/portable/<S>/<run-id>/portable-terminal.v1.sig
```

The uncompressed tar has exactly one member for every local bundle regular file
under canonical prefix `gate-d-runs/<S>/<run-id>/`, plus immutable copies
`proof/local-publication-terminal.v1.json` and
`proof/local-publication-journal-confirmation.v1.json`. It has no directory,
symlink, hardlink, sparse, device or FIFO member. Member paths are UTF-8,
repository-style `/`, bytewise sorted, unique, relative, and contain no empty,
`.` or `..` component. The canonical POSIX-pax writer fixes uid/gid to zero,
uname/gname empty, mtime zero, type regular, size from streamed bytes, member
mode to the locally verified bundle/proof mode, PAX key order bytewise, and all
header/data padding to zero. It emits no global PAX header, atime, ctime, host
path, device or inode. The final two 512-byte zero blocks are mandatory and no
trailing bytes are allowed. Archive construction is performed twice into two
exclusive temporary files and byte equality is required before the sole final
rename. The exact archive size and streamed SHA-256 are journaled.

`portable-terminal.v1.json` has schema identity
`gate-d-portable-authority-terminal/v1` and canonical RFC-8785 bytes with one
trailing newline. It binds `S`, run ID, the canonical three outer basenames,
archive schema/size/SHA-256, bundle-manifest and release-attestation hashes,
local-publication-terminal hash, exact copied journal-confirmation record/hash/
chain tail, host-input hash, behavior/evidence-index hash, and signer key ID. It
contains no absolute path, device, inode, extraction root, signature bytes or
terminal self-hash. This avoids archive/terminal/signature recursion.

The signature is Ed25519 over the exact domain bytes
`stock_analysis.gate_d.portable_terminal.v1\0` followed by the exact terminal
bytes. `portable-terminal.v1.sig` is one lowercase 128-hex signature plus one
newline. `tools/coverage/portable_signers.v1.json` is a closed source-controlled
registry of approved key IDs and 32-byte public keys. The private signing key is
available only to the outer post-cleanup signer through a pinned descriptor; it
never enters the coverage child, bundle, archive, environment or trace output.
Absence of an approved signer—normal for an untrusted pull request—creates only
`coverage-diagnostics`, leaves Gate D blocked, and cannot fall back to an
unsigned terminal. The wrapper remains the sole minter because only it may
request the signature after all local proofs pass; signer failure is a terminal
portable-export failure with no PASS.

Download verification is ordered and does not trust ZIP-extracted modes. It
first validates terminal schema/canonical bytes/key ID and detached signature,
then streams the archive size/SHA-256. Only then does it parse every tar header,
reject forbidden/duplicate/unlisted members, and extract descriptor-relatively
into a new 0700 no-follow temporary directory using exclusive regular-file
creation. It applies and rechecks the **internal** recorded modes, hashes and
manifest after extraction, validates the copied local terminal/journal proof as
historical content without comparing local path/device/inode, and reruns the
complete semantic verifier. The outer archive/terminal/signature modes and
download path/device/inode are deliberately irrelevant; changing outer modes
cannot change signed bytes. The root `coverage.json` compatibility copy remains
inside the archive and regains its required read-only internal mode before
bundle verification.

The successful GitHub Actions artifact identity remains exactly
`coverage-report`; its payload is exactly the archive, detached terminal and
detached signature above—never the live directory or standalone JSON. Upload
uses one explicit three-line path list, `if-no-files-found: error`, no wildcard,
and runs only after wrapper exit 0 plus a fresh portable verification. A failed
wrapper may upload persistent journal/staging evidence under non-authoritative
fixed name `coverage-diagnostics`; it must not upload or overwrite
`coverage-report`. GitHub's outer ZIP permission normalization is accepted only
because all authoritative modes are tar-member facts covered by the signed
archive hash. Consumers that only need counters may extract the root compatibility
copy after signature/hash verification but must label it diagnostic. Release
consumers must verify all three files and the unpacked bundle; no runner-local
path/inode is portable authority.

The downloader uses this exact interface. `probe` validates the approved
Ed25519 implementation and safe descriptor-relative extraction primitives
without opening the archive; `verify-portable` performs the ordered checks
above. Interface absence or an unsupported primitive is closed `BLOCKED` with
exit 2; malformed, forged, or semantically inconsistent evidence is `FAIL`
with exit 1:

```bash
python3 tools/coverage/verify_gate_d_attestation.py probe \
  --capability portable-v1-safe-extraction
# GATE_D_PORTABLE_PROBE status=PASS schema=gate-d-portable-archive/v1

python3 tools/coverage/verify_gate_d_attestation.py verify-portable \
  --archive coverage-report.v1.tar \
  --terminal portable-terminal.v1.json \
  --signature portable-terminal.v1.sig \
  --expected-source "$S" --expected-run-id "$RUN_ID"
# GATE_D_PORTABLE_VERIFY status=PASS source=<S> run_id=<run-id> signature=true archive=true safe_extract=true semantic=true
```

The verifier returns 0 only after the exact PASS marker, 2 only for an
explicitly enumerated unavailable capability before archive extraction, and 1
for all input, signature, archive, extraction, mode, hash, manifest,
local-proof, or semantic mismatches. It never emits PASS after a warning or
partial check.

Rollback reverts the CI/README/consumer migration and wrapper/archive/terminal/
signer/schema source commit together to the last independently verified portable
version, retains all local bundles/journals/terminals and portable archives/
terminals/signatures, and appends a signed supersession record. It never
re-signs or promotes an old local candidate, never compares a downloaded inode,
and never restores standalone JSON upload as release authority. A revoked key
or defective archive version requires a reviewed higher-version reader plus
supersession; existing bytes remain retained. If a consumer cannot migrate in
the same release, the v1 root copy inside the verified archive is the transition
shim; if that shim is defective, Gate D remains blocked.

Migration acceptance is:

```bash
cargo test --test test_coverage_entrypoints -- --test-threads=1
# expected: all tracked invocation hits classified; one release wrapper identity
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
# expected: ordinary diagnostic parser result; never release-attestation/PASS
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
# expected: ordinary raw diagnostic result; never release-attestation/PASS
bash tools/coverage/run_isolated_gate.sh
# expected only after local durable publication plus canonical portable export:
# archive, detached portable terminal/signature, fresh portable verification and PASS
```

## 3. Test architecture and data red lines

### 3.1 Behavior, not line execution

Each test names a business result or explicit failure and asserts externally
observable state. Tests do not call private branches only to increment a counter.
Private resolved-input seams are allowed only immediately behind an unchanged
real adapter when behavior genuinely varies. They stay private and cannot make a
production fake source constructible.

### 3.2 Protocol fixtures

Protocol tests use one of these test-only forms:

1. an exact local response body matching a documented current provider schema;
2. a `cfg(test)` loopback HTTP/TCP endpoint exercised by the production client;
3. an already validated typed batch passed to a private resolved-input seam.

Every security identity is prefixed `TEST_CODE_`. Missing optional fields stay
missing; malformed, stale, partial, conflicting, duplicate, gapped, non-finite,
non-positive, or unconfirmed greater-than-20% adjacent changes assert a typed
failure or pending-manual-confirmation result. A fixture never becomes production
evidence, never writes a production cache, and never supplies an account,
position, cash, net-value, order, notification receipt, or audit success.

Tests use invocation-unique temporary database, audit, push-log, report, and
socket roots. Production code rejects `TEST_CODE_` orders; test mode rejects real
symbols. Network credentials and real notification recipients are removed from
coverage processes.

### 3.3 Live evidence is separate

Coverage proves deterministic behavior; it does not prove provider liveness.
Gate D separately runs the repository's reviewed live-data canaries or isolated
protocol equivalents. A source transport failure remains an explicit failure and
cannot be changed to a verified empty batch to make a test pass. Production push
and order canaries require their existing controlled authority and audit trail;
coverage collection itself performs neither.

### 3.4 Executable test/live isolation wrapper

Gate D coverage is run only through `tools/coverage/run_isolated_gate.sh`; a raw
`cargo llvm-cov` result is diagnostic, not release evidence. The wrapper:

1. requires a clean fixed reviewed source SHA and no running production monitor;
2. loads the closed, versioned `GATE_D_ISOLATION_POLICY_V1` from
   `tools/coverage/isolation_policy.v1.json` and records a SHA-256 manifest of
   every exact production resource in that registry;
3. creates a detached temporary Git worktree at that exact SHA, rejects symlink
   or hardlink aliases back to the production data roots, and runs from that
   worktree so compile-time `CARGO_MANIFEST_DIR` and CWD-relative paths are both
   isolated;
4. refuses to start if the environment contains a real symbol (`STOCK_LIST`
   member without `TEST_CODE_`), a key matching the forbidden credential/sink
   patterns below, a production resource/DSN value, or a `.env` file in the
   detached worktree;
5. binds invocation-unique `TEST_CODE_COVERAGE_<nonce>` database, audit,
   push-log, report, socket, temp, durable-delivery, and `CARGO_TARGET_DIR`
   namespaces and configures the test process to reject external recipients and
   real-symbol orders;
6. runs the full workspace/all-features tests and llvm-cov command, then the
   default checker;
7. rejects any production-shaped DB/audit/push-log/order/notification artifact
   in the detached worktree, verifies that only declared `TEST_CODE_` namespaces
   were written, compares the original production manifest byte-for-byte, and
   verifies the OS file/socket trace contains zero production-resource reads,
   zero production-resource writes, and zero non-loopback network/sink attempts;
8. emits one machine-readable marker only after every assertion succeeds.

Executable acceptance command and exact marker:

```bash
bash tools/coverage/run_isolated_gate.sh
# COVERAGE_ISOLATION status=PASS source_sha=<40-hex> report_sha256=<64-hex> \
# inventory_sha256=<64-hex> show_manifest_sha256=<64-hex> \
# dependency_manifest_sha256=<64-hex> zero_proof_sha256=<64-hex> \
# build_profile_sha256=<64-hex> object_manifest_sha256=<64-hex> \
# raw_profile_manifest_sha256=<64-hex> profdata_sha256=<64-hex> \
# generated_inputs_sha256=<64-hex> retained_object_cas_sha256=<64-hex> \
# host_execution_inputs_sha256=<64-hex> host_trace_sha256=<64-hex> \
# decision_denominator_sha256=<64-hex> decision_site_owners_sha256=<64-hex> \
# behavior_authority_sha256=<64-hex> behavior_residual_sha256=<64-hex> \
# bundle_manifest_sha256=<64-hex> release_attestation_sha256=<64-hex> \
# local_publication_terminal_sha256=<64-hex> local_confirmation_sha256=<64-hex> \
# portable_archive_sha256=<64-hex> portable_terminal_sha256=<64-hex> \
# portable_signature_sha256=<64-hex> lifecycle_journal_sha256=<64-hex> \
# cleanup_succeeded=true local_terminal_parent_fsynced=true \
# portable_export_parent_fsynced=true portable_signature_verified=true \
# production_manifest_unchanged=true real_symbols=0 credentials=0 \
# production_db_writes=0 production_audit_writes=0 production_push_log_writes=0 \
# production_order_writes=0 production_notification_writes=0 \
# production_reads=0 external_connect_attempts=0 sink_exec_attempts=0
```

Negative process tests seed one forbidden item at a time (real stock code,
recipient, credential, production DB path, production audit/push path, symlink,
hardlink) and require exit 2 before Cargo starts. A child-process test also
attempts a real-symbol order in test mode and a `TEST_CODE_` order in production
mode; both must be rejected with zero order/audit/sink writes. Merely unsetting
known variables is insufficient because `dotenv()` could reload credentials;
the detached worktree must contain no `.env`, and the wrapper records its
sanitized environment allowlist.

The isolation policy is a closed schema, not “any configured store.” Its
`production_resources` registry contains these exact repository-relative roots
and their descendants: `data/stock_analysis.db` plus `-wal`/`-shm`,
`data/durable_delivery.sqlite3` plus `-wal`/`-shm`, `data/audit/production/`,
`data/event_audit/`, `data/event_bus/`, `data/push_log/`,
`data/dispatcher_log/`, and `data/review_audit/`. It also contains exact
canonical paths resolved from the production configuration fields
`DATABASE_PATH`, `STOCK_DB`, and `MAGICLAW_DB_PATH` before sanitization, and
rejects every `file:`, `sqlite:`, `postgres:`, `postgresql:`, `mysql:`,
`redis:`, `http:`, `https:`, or unknown URI/DSN resource in the coverage child.
Adding or changing a production path/DSN field requires a reviewed policy
version bump in the same source commit; an observed resource not classified as
an allowed read-only toolchain input, an invocation-unique TEST_CODE resource,
or one of the forbidden production resources is `unknown` and exits 2.

The coverage child starts with `env -i`. Its complete environment-key allowlist
is `PATH`, `HOME`, `TMPDIR`, `LANG`, `LC_ALL`, `TZ`, `CARGO_HOME`,
`RUSTUP_HOME`, `CARGO_TARGET_DIR`, `CARGO_NET_OFFLINE`, `RUST_BACKTRACE`,
`RUST_LOG`, `STOCK_ENV_MODE`, `STAGE`, `STOCK_LIST`, `DATABASE_PATH`,
`DISPATCHER_LOG_DIR`, `REVIEW_AUDIT_DIR`, `V10_DRY_RUN_PUSH`, `TEST_VERBOSE`,
and `NO_PROXY`. `HOME`, `TMPDIR`, `CARGO_HOME`, `CARGO_TARGET_DIR`, database and
log values are wrapper-created children of the run root; `STOCK_ENV_MODE=test`,
`STAGE=test`, `STOCK_LIST=TEST_CODE_COVERAGE_<nonce>`,
`CARGO_NET_OFFLINE=true`, `V10_DRY_RUN_PUSH=1`, and
`NO_PROXY=127.0.0.1,localhost,::1` are fixed. `PATH` and `RUSTUP_HOME` are
canonicalized and admitted only as read-only toolchain roots. Any extra key in
the child environment is unknown and exits 2 before Cargo.

Before `env -i`, the wrapper rejects any inherited key or value matching the
case-insensitive closed forbidden families `BROKER|ACCOUNT|RECIPIENT|RECEIVER|`
`SINK|WEBHOOK|FEISHU|WECHAT|DINGTALK|DISCORD|TELEGRAM|PUSHOVER|SLACK|SMTP|`
`EMAIL|TOKEN|SECRET|PASSWORD|PASSWD|API_KEY|AUTH|BEARER|COOKIE|SESSION|DSN|`
`DATABASE_URL|PROXY|MAGICLAW|LIVE|PROD`, except the exact wrapper constants
listed above after their values have been replaced. Tests enumerate every
current repository environment lookup and require either exact child admission
or forbidden/cleared classification; a newly discovered lookup is unknown and
fails the policy test.

File and socket isolation is enforced, not inferred only from absent
credentials. Linux uses a private network namespace and macOS uses an equivalent
deny-by-default sandbox profile; both permit AF_UNIX only under the run root and
TCP only to an ephemeral loopback listener created by the current test process.
DNS, multicast, non-loopback connect, and process execution of MagicLaw or any
notification helper are denied and audited. A platform without the required
deny policy plus readable file/socket event log exits 2. The verifier reconciles
the log against the closed policy and requires `production_reads=0`,
`production_writes=0`, `external_connect_attempts=0`, and `sink_exec_attempts=0`;
pre/post hashes are an additional write check, not proof of zero reads.

#### 3.4.1 Wrapper lifecycle and evidence preservation

The failure journal must exist before anything temporary can fail. After
resolving clean fixed `S` and random `run-id`, but before dependency bootstrap,
`mktemp`, worktree creation, or export, the wrapper uses
`tools/coverage/gate_d_journal.py` to create this independent caller-worktree
root with exclusive/no-follow operations:

```text
target/coverage/gate-d-journals/<S>/<run-id>/
  lifecycle.jsonl
  first-cause.json
  partial-artifacts.manifest
```

The helper opens and returns a pinned directory FD and an `O_CREAT|O_EXCL|
O_APPEND|O_NOFOLLOW` lifecycle FD, both tied to the recorded device/inode. It
uses mode 0700/0600, writes canonical `journal_ready`, calls `fdatasync` on the
file and `fsync` bottom-up through every newly created directory including
`gate-d-journals`, then proves the pathname still resolves to the pinned
device/inode. If creation, the first append, or any initial file/directory fsync
fails, preflight exits 2 and **must not create `RUN_ROOT`**. If the root cannot
be made durable there is necessarily no durable file evidence; stderr prints
only `GATE_D_PREFLIGHT_FAIL reason=journal_unavailable run_root_created=false`
and no success/attestation token.

Only after `journal_ready` is durable does the wrapper install idempotent
ERR/EXIT traps, complete Section 2.4.2 dependency bootstrap, create
`RUN_ROOT=$(mktemp -d "${TMPDIR%/}/stock-analysis-gate-d.XXXXXXXX")`, and add
the detached worktree. Every phase transition is appended through the already
pinned lifecycle FD and synced. The first failing command/phase/status becomes
immutable `first_cause`; later failures are ordered `secondary_causes` and
cannot replace it. The ERR trap records sanitized argv/environment, trace tail,
available artifact hashes, and whether Cargo/worktree/export began.

The independent journal is not inside `RUN_ROOT`, the detached worktree, the
export staging tree, local-terminal directory, or portable-export directory.
Therefore a host probe/trace, generation, object-copy, export file-fsync,
export-directory-fsync, verifier, bundle rename/fsync, terminal/archive/
signature write/rename/fsync, or cleanup failure can still append
and sync `first_cause`, `cleanup_started`, and exactly one terminal
`cleanup_succeeded`/`cleanup_failed` record through the pinned journal FD. The
FD remains open through local and portable publication; “cleanup terminal” is
not the lifecycle close. An
export fsync fault never asks the failing export filesystem object to preserve
its own sole diagnosis. If a later journal append/sync itself fails, the run
fails with no PASS and preserves all previously synced journal records; stderr
identifies `journal_terminal_unconfirmed`. Such a run cannot be attested.

The caller-worktree export parent is
`target/coverage/gate-d-runs/<S>/`. Export uses exclusive regular-file creation
under `.exporting-<run-id>`, rejects links/path swaps, copies every available
artifact, constructs the Section 2.5.2 manifest, fsyncs every file then every
directory through `gate-d-runs`, and runs descriptor-relative pre-verification.
The mandatory success order is **persistent journal -> closed host-input probe ->
dependency preflight -> traced generation -> export including retained object
bytes and host manifests -> every file/directory fsync -> candidate semantic
verification -> cleanup -> cleanup-terminal journal fsync ->
release-attestation candidate -> finalized bundle manifest/fsync -> candidate
semantic verification -> atomic bundle rename -> bundle-parent fsync -> local
terminal file write+fsync -> local-terminal rename -> local-terminal-parent
fsync -> append+fdatasync `local_publication_terminal_parent_fsynced` journal
record -> canonical archive build twice/byte-equality -> archive file+parent
fsync -> portable terminal file+parent fsync -> detached signature file+parent
fsync -> fresh signature/archive/safe-extraction semantic verification ->
append+fdatasync `portable_export_confirmed` -> PASS**.
`release-attestation.v1.json` is not created before cleanup succeeds,
but its mere presence is never authority. Before bundle rename, finalization
copies the cleanup-terminal lifecycle prefix into
`diagnostics/lifecycle.jsonl`, updates/fsyncs the manifest, and runs the complete
candidate verifier.

Only after the final bundle path is renamed and its parent fsync succeeds may
the wrapper create an exclusive `.terminal-writing-<run-id>` file under the
pinned `gate-d-local-publication/<S>/` directory. It writes the closed local
binding, fdatasyncs the file, atomically renames it to
`<run-id>.terminal.v1.json`, and fsyncs that directory and every new ancestor.
It then appends and fdatasyncs exactly one
`local_publication_terminal_parent_fsynced` lifecycle record containing local
path/device/inode, terminal hash, bundle manifest/attestation hashes and the
successful parent-fsync phase. That relation proves local durability only.

The wrapper next copies the exact local terminal and confirmation record into
the canonical archive proof members, writes/fsyncs/renames the archive, writes
the content-only portable terminal, and asks the approved pinned-FD signer for
the detached signature. After file and parent fsync, a new no-follow temp root
performs the complete download-side verification from the three final outer
files. Only then is `portable_export_confirmed` appended and fdatasynced. This
record binds the three outer hashes and successful verification but is not
needed to break a signature recursion: the signed portable terminal already
binds the prior local journal confirmation copied into the archive. No
failure-prone publication step remains between this final synced record and
PASS.

`on_exit` is the sole cleanup owner. It removes only exact validated temporary
worktree/Cargo/temp roots; it never removes the persistent journal, a published
bundle candidate, local-terminal candidate, portable archive/terminal/signature,
a prior run, or the only extant copy of a partial export/diagnostic. If
generation/export/fsync/pre-verification fails, cleanup still runs and the
journal persists even when no bundle can be published. If cleanup fails, the
journal records residual canonical paths/device/inodes, staging and `RUN_ROOT`
remain for read-only inspection, exit is 2, and no release attestation/PASS is
created. An earlier cause remains primary with `cleanup_failed` secondary.
If bundle-parent fsync or any local-terminal phase fails, the final-path bundle
is quarantined/incomplete: no confirmed local relation, portable export or PASS
exists. If canonical archive, portable terminal, signing, parent-fsync or fresh
portable verification fails, the already confirmed local bundle remains local
evidence but is explicitly `portable_quarantined` and cannot be uploaded as
`coverage-report`. Recovery preserves every bundle, terminal, archive staging
file and journal; it never appends a missing confirmation, signs a partially
constructed terminal or promotes the old run. A new run ID must repeat host
capture, generation, export, cleanup and both local and portable publication.

Negative tests inject initial journal create/append/file-fsync/directory-fsync,
host probe/trace/reconciliation, dependency bootstrap, generation, object copy, file-fsync,
directory-fsync at each ancestor, pre-verification, worktree-remove,
temp-root-remove, cleanup-terminal journal sync, candidate verification, bundle
rename, bundle-parent fsync, local-terminal write/file-fsync/rename/parent-fsync,
local-confirmation journal sync, archive first/second build mismatch, tar
canonicalization, archive rename/fsync, portable-terminal write/fsync, signer/key/
signature, portable-parent fsync, safe-extraction and final-verifier failures.
They assert no PASS or portable signed authority, candidate attestations remain
non-authoritative, every final-path failure tuple is rejected/quarantined,
immutable first cause,
exactly one cleanup terminal when the journal remains writable, no earlier-run
overwrite, no deletion of the sole failure evidence, and `RUN_ROOT` absence for
initial journal/host/dependency-preflight failures. Recovery tests prove no old run
can be promoted by merely completing a missing rename/fsync/journal record.

### 3.5 Independent production-decision denominator and behavior clusters

`A union R` cannot prove its own completeness: an unregistered decision omitted
from both sets would remain invisible. BR-202 therefore makes a compiler/source-
derived denominator independent of authority/residual annotations. The v1 claim
is deliberately narrow and truthful: completeness covers every non-test
production decision site or entrypoint that can reach a registered externally
observable business-decision/side-effect sink, plus every AGENTS 2.10
deduplication, mutex/locking, filtering, sorting or limiting site. It does not
claim to enumerate semantic behavior outside that mechanically closed scope.
Any unresolved call, dynamic target, macro expansion, generated source, sink
class or source span keeps Gate D blocked instead of being silently excluded.

Gate B adds these six distinct v1 inputs:

1. `tools/coverage/decision_sinks.v1.json` is the closed sink/operator registry;
2. `tools/coverage/decision_denominator.schema.v1.json` freezes denominator row
   identity and extraction invariants;
3. `tools/coverage/decision_site_owners.v1.json` maps each independently emitted
   denominator identity to exactly one canonical behavior;
4. `tools/coverage/behavior_authorities.v1.json` declares the closed list of
   executable authority adapters and their exact same-SHA inventory commands;
5. `tools/coverage/behavior_residuals.v1.json` is the explicit audited registry
   for denominator-backed behaviors that genuinely have no enum/descriptor/
   dispatcher/task/strategy/provider/trading/risk registry owner; and
6. `tools/coverage/behavior_clusters.v1.json` maps each canonical behavior
   identity to tests, source participation and its evidence contract.

The extractor runs from `S` using the exact pinned compiler/toolchain recorded in
the build profile. Compiler MIR supplies resolved production DefPath identities,
basic blocks, terminators, call edges, monomorphized/dynamic targets and source
spans; rustdoc/private-item metadata supplies stable item/signature ownership;
a Rust source-AST/expanded-source pass independently enumerates branch/match/
guard/short-circuit sites, macro expansions and Rule-2.10 operators, including
registered SQL `ORDER BY`/`LIMIT` literals. The three views must reconcile by
DefPath/source-span hashes. Test-only cfg items, `tests/**`, examples and benches
remain fixed build inputs but are excluded from the **production** denominator.

The closed sink registry names exact compiler-resolved symbols/signature hashes
for order/reservation/outbox/delivery, audit append, production database mutation,
notification/sink, provider admission/outcome, CLI operation result and public
business-decision result constructors. It also names low-level file/database/
network/process primitives; every production call to one must resolve through a
registered higher business sink or fail as an uncovered sink boundary. The
Rule-2.10 operator registry covers standard/custom filter, sort, dedup, mutex/
lock and limit/take/truncate operations. Unknown FFI, trait-object, function-
pointer, proc-macro or generated call targets fail extraction unless a closed
compiler-resolved target set is registered and reproduced.

Each denominator row is
`decision:<defpath-hash>:<mir-basic-block>:<terminator-or-entry-index>` and binds
schema/source SHA, production target, source path/blob/span hash, decision kind,
resolved reachable sink/operator IDs and a compiler call-path proof. Straight-
line sink entrypoints receive an entry row, so a behavior need not contain a
branch to enter the denominator. Removing/changing a source annotation cannot
remove a row; only compiler/source/sink evidence constructs `D(S)`. The sink
registry itself is checked against every resolved low-level effect call and
every authority result constructor, so deleting a known sink or adding a new
effect wrapper creates an unmatched call and fails.

Authority adapters use production registry APIs or compiler-checked exhaustive
enumeration, never a loose source regex. The initial required authority families
are:

| Authority ID | Same-SHA identity source |
| --- | --- |
| `push_kind` | every `src/bin/monitor/notify.rs::PushKind` value, joined exactly to its stable template ID/durable kind disposition |
| `presentation_descriptor` | every `src/bin/monitor/presentation_registry.rs::descriptors()` row, including active/disabled status, producer and renderer |
| `dispatcher_task` | every production dispatcher route plus `src/bin/monitor/review_batch.rs::ReviewTask::ALL` and operational CLI command descriptors |
| `strategy` | every `registry::StrategyRegistry` identity and every configured production strategy selected through it |
| `provider_capability` | every unified Magic Router/provider/capability descriptor admitted by `src/data_gateway/**` or `src/search_service/**`; scattered provider match arms must first be represented by a compiler-checked descriptor rather than scraped |
| `trading_order` | every trading/order command/result kind and business-order transition registry |
| `risk_rule` | every hard-limit/order-safety/veto rule descriptor, including cash, lot, price range, idempotency and secondary confirmation |

Each adapter emits canonical rows
`<authority-id>:<stable-id>` with authority source symbol, status, owner BR/spec,
and schema version. The safe inventory command initializes no provider, DB,
account, sink or monitor loop. Its output is built at and binds `S`; tests compare
exhaustive enum/descriptor/registry counts and fail when a new variant/row/match
arm is not emitted. If a current area lacks such a production registry, its
behaviors remain in the audited residual registry until a separately reviewed
registry exists; this coverage design does not invent runtime infrastructure.

Every residual row has a permanent `residual:<stable-id>`, one or more exact
denominator identities, exact owning symbol, source path/hash, owner BR/spec,
reviewer, reason no authority registry exists, and one explicit test/evidence
contract. The owning symbol carries a
compiler-visible `gate_d_residual_behavior_id` declaration. A same-SHA compiler
inventory must equal the residual JSON rows exactly; a declaration without a
row, a row without a declaration, changed owning bytes, duplicate stable ID, or
an authority-derived behavior also present as residual fails. Adding/deleting a
residual is a reviewed BR-202 change, never automatic deduplication.

Let `D(S)` be the independently extracted decision-site denominator, `A_i(S)`
the exact sorted behavior set emitted by authority adapter `i`,
`A(S)=union_i A_i(S)`, `R(S)` the residual behavior set, `B(S)` the canonical
behavior identities in clusters, and `M(S)` the site-owner rows. Generation
requires all following bytewise relations and publishes every operand, row,
hash and cardinality:

```text
duplicates_within_or_between_A_i = empty
A(S) intersection R(S) = empty
B(S) = A(S) union R(S)
domain(M) = D(S)
each d in D has exactly one M row and exactly one owner in B
range(M) = B(S)
missing_sites = D - domain(M) = empty
extra_sites = domain(M) - D = empty
missing_behaviors = (A union R) - B = empty
extra_behaviors = B - (A union R) = empty
```

There is no silent last-writer-wins dedup. A legitimate cross-registry alias
requires an explicit reviewed equivalence row in the authority schema naming
all origins and one canonical ID; the verifier independently reconstructs that
equivalence and otherwise rejects the collision. Independently, `D` makes a new
unannotated decision/sink/Rule-2.10 site fail until it has one owner. A source
change that omits both authority and residual declarations therefore cannot
pass merely because `A`, `R`, and `B` shrank together. If the extractor cannot
prove completeness for a current language/sink class, it emits an unresolved
row, exits 2 and the release claim stays blocked; reviewers may not waive it by
calling the residual registry “audited.”

#### 3.5.1 Executable D/M extraction interface

The extractor has exactly three subcommands: `probe`, `generate`, and `verify`.
Every subcommand accepts only the fixed flags below, emits canonical RFC-8785
JSON artifacts plus one final marker on stdout, writes diagnostics to stderr,
returns 0 only on the named PASS marker, and returns 2 for unavailable compiler
features, malformed input, unresolved classes or semantic mismatch. Exit 1 is
reserved for an unexpected extractor crash and is also a release failure. No
subcommand accepts a repository root, target subset, feature subset, cfg override,
sink override or allow-unresolved flag.

The first command runs before Cargo coverage and uses the exact rustc/rustdoc
executables already pinned by the host/toolchain manifests. Its fixture is the
source-controlled `tools/coverage/fixtures/decision_probe/` crate and covers a
branch, short-circuit, match guard, declarative macro, proc macro, trait object,
function pointer, `extern "C"` declaration and every Rule-2.10 operator class:

```bash
python3 tools/coverage/extract_decision_denominator.py probe \
  --source-sha "$COVERAGE_GATE_SHA_BEFORE" \
  --toolchain-manifest target/coverage/build/toolchain.json \
  --host-inputs target/coverage/host/execution-inputs.v1.json \
  --fixture tools/coverage/fixtures/decision_probe/Cargo.toml \
  --output target/coverage/behavior/probe.v1.json
# DECISION_EXTRACTOR_PROBE status=PASS emit_mir=true rustdoc_json=true \
# expanded_source=true private_items=true source_spans=true \
# proc_macro_spans=true target_enumeration=true unresolved_probe_classes=0
```

The probe internally executes and records these exact compiler-interface shapes
for the fixture, substituting only recorded executable/input/output paths:

```text
<rustc> <captured crate argv> --emit=mir -o <exclusive-output>
<rustdoc> <captured crate argv> --document-private-items --output-format json -o <exclusive-output-dir>
<rustc> <captured crate argv> -Zunpretty=expanded -o <exclusive-output>
```

`RUSTC_BOOTSTRAP`, injected sysroot/cfg/feature flags and a second compiler are
forbidden. If the pinned toolchain does not support any literal interface above,
does not retain source spans/expansion origins, or cannot reproduce the probe's
closed target set, `probe` exits 2 with
`DECISION_EXTRACTOR_PROBE status=BLOCKED reason=<closed_reason>` and Gate D stays
blocked. The allowed reasons are exactly `emit_mir_unavailable`,
`rustdoc_json_unavailable`, `expanded_source_unavailable`,
`private_items_unavailable`, `source_spans_unavailable`,
`proc_macro_spans_unavailable`, and `target_enumeration_unavailable`. The design
does not claim that the current stable toolchain already passes this probe.

After probe PASS, the wrapper obtains Cargo's closed target/feature/cfg set with
this exact command and then runs the production capture build:

```bash
cargo metadata --locked --offline --format-version 1 \
  > target/coverage/behavior/cargo-metadata.json.tmp
mv target/coverage/behavior/cargo-metadata.json.tmp \
  target/coverage/behavior/cargo-metadata.json

env -i \
  PATH="$PATH" HOME="$HOME" TMPDIR="$TMPDIR" LC_ALL=C TZ=UTC \
  CARGO_HOME="$CARGO_HOME" RUSTUP_HOME="$RUSTUP_HOME" \
  CARGO_TARGET_DIR="$CARGO_TARGET_DIR" CARGO_NET_OFFLINE=true \
  GATE_D_DECISION_CAPTURE_DIR="$GATE_RUN_DIR/behavior/capture" \
  GATE_D_DECISION_SOURCE_SHA="$COVERAGE_GATE_SHA_BEFORE" \
  RUSTC_WRAPPER="$PWD/tools/coverage/rustc_decision_capture.py" \
  cargo build --workspace --all-targets --all-features --locked --offline \
    --message-format=json-render-diagnostics
```

`rustc_decision_capture.py` records every Cargo-supplied rustc argv before
execution. For every metadata target whose Cargo kind is `lib`, `rlib`,
`cdylib`, `staticlib`, `proc-macro` or `bin`, it replays the exact argv three
times using the literal MIR/rustdoc/expanded shapes above; it rejects examples,
benches/tests masquerading as production and records them separately as
non-production build inputs. It preserves the exact package, target, enabled
features, target triple and cfg set. Every proc-macro dylib and expansion-origin
edge is hash-bound. A Cargo target absent from capture, a captured target absent
from metadata, duplicate target identity, target/cfg/feature drift or analysis
replay that changes normal build outputs exits 2.

The generation and total-owner verification commands are exactly:

```bash
python3 tools/coverage/extract_decision_denominator.py generate \
  --source-sha "$COVERAGE_GATE_SHA_BEFORE" \
  --cargo-metadata target/coverage/behavior/cargo-metadata.json \
  --capture-dir target/coverage/behavior/capture \
  --sink-registry tools/coverage/decision_sinks.v1.json \
  --denominator-schema tools/coverage/decision_denominator.schema.v1.json \
  --output-dir target/coverage/behavior/generated
# DECISION_DENOMINATOR_GENERATED status=PASS targets=<n> mir_items=<n> \
# ast_sites=<n> expanded_sites=<n> denominator_sites=<n> unresolved=0

python3 tools/coverage/extract_decision_denominator.py verify \
  --source-sha "$COVERAGE_GATE_SHA_BEFORE" \
  --generated-dir target/coverage/behavior/generated \
  --site-owners tools/coverage/decision_site_owners.v1.json \
  --authorities tools/coverage/behavior_authorities.v1.json \
  --residuals tools/coverage/behavior_residuals.v1.json \
  --clusters tools/coverage/behavior_clusters.v1.json \
  --test-list target/coverage/behavior/cargo-test-list.txt \
  --output-dir target/coverage/behavior/verified
# DECISION_DENOMINATOR status=PASS D=<n> M=<n> B=<n> A=<n> R=<n> \
# unresolved=0 missing_sites=0 extra_sites=0 multi_owner_sites=0 \
# missing_behaviors=0 extra_behaviors=0 reused_evidence=0
```

Before `verify`, the wrapper creates `cargo-test-list.txt` using exactly
`cargo test --workspace --all-targets --all-features -- --list --format terse`
under the same sanitized environment. `generate` emits closed-schema,
bytewise-sorted `compiler-targets.json`, `mir-items.json`, `rustdoc-items.json`,
`expanded-sites.json`, `source-ast-sites.json`, `call-edges.json`,
`rule-2-10-sites.json`, `unresolved.json`, and `decision-denominator.json`.
Every row binds schema, `S`, target/cfg/features, DefPath hash, normalized Git
path/blob/span, expansion origin, resolved calls/sinks/operators and input
artifact hashes. Only the detached-worktree prefix is remapped; basic-block,
span, macro and generated identities are not renumbered or deduplicated.

Rule-2.10 extraction is the union of expanded/source-AST syntax candidates,
compiler-resolved standard/custom operator symbols, parsed SQL
`ORDER BY`/`LIMIT`, synchronization primitives, collection truncation/take,
dedup/unique/set admission and every branch/guard reaching a registered sink.
A dynamic SQL fragment, manual loop/branch or macro that cannot be classified as
operator or non-operator emits an unresolved row rather than disappearing.
Unknown low-level file/DB/network/process calls, FFI, trait-object/function-
pointer target sets, generated source or proc-macro origin likewise block.

Bootstrap/feasibility tests run `probe` against the pinned toolchain, remove
each required compiler feature, perturb one target/feature/cfg, hide one macro
expansion and add one unresolved dynamic/FFI/manual Rule-2.10 site. Each negative
must return exact exit 2/BLOCKED before any denominator PASS. Golden fixtures
assert stable normalized bytes and row hashes. This makes AC9 an executable
fail-closed interface; it does not promise a PASS on an incapable compiler.

The cluster schema identity remains exactly `gate-d-behavior-clusters/v1`.
Cluster IDs are permanent and match `BC-[A-Z0-9][A-Z0-9-]{2,63}`. Each cluster
contains **exactly one** canonical `behavior_id`, one or more exact normalized
`source_paths`, one or more canonical
`<cargo-target>::<fully-qualified-test>` identities, one production binary, and
one evidence contract. It also lists all site-owner rows for that behavior.
Several decision sites and files may implement one behavior, and one file/test
may participate in several behaviors/clusters. Source/test paths may occur in multiple clusters and are
validated against the same-SHA coverage source inventory and canonical `cargo
test --workspace --all-targets --all-features -- --list --format terse` output.
They do not substitute for `D` or `M`. An empty source/test/site set, unknown
path/test/site, unregistered owner, denominator site in zero/multiple behaviors,
or behavior in zero/multiple clusters fails.

The evidence contract is exactly `production_chain` (producer, adapter,
consumer/decision or push, terminal audit event type, locator, freshness,
redaction) or `disabled` (reason matching `[a-z0-9_]{1,64}` and exact same-binary
`[GateD][<cluster_id>] disabled=<reason_code> producer=none`). Each cluster has
one distinct `behavior/evidence/<cluster-id>.json` bound to its single behavior
ID. A record must select a behavior-specific terminal identity/offset and join;
the same evidence record identity/path+offset cannot satisfy two clusters. Thus
one mega-cluster or one generic evidence record cannot cover every behavior.
Underlying redacted production files may be shared only when each behavior has
its own independently verified record and terminal join.

A `production_chain` record binds a registered-window real
producer→adapter→consumer/push→audit chain, source SHA and artifact byte hashes.
A `disabled` record binds literal startup line, `S`, covered binary full hash and
build ID, and banner call-site source hash. Missing, duplicate, stale,
fixture/dry-run, unknown-cluster, wrong-behavior/mode/event/banner, reused record
identity, or extra evidence exits 2. An unavailable producer is never converted
to production-chain evidence; it needs the exact registered disabled banner.

On 2026-08-02 there is no usable `data/push_log/2026-08-02/`,
`data/event_bus/2026-08-02.jsonl`, or real Gate-D disabled-banner capture.
Therefore no behavior-evidence record may currently say PASS, and Gate D remains
blocked even if deterministic coverage later reaches both percentages. Tests
and dry-runs are explicitly rejected by evidence provenance and cannot close
this blocker.

Named Gate B tests include
`behavior_authority_and_residual_exact_union_matches_clusters`,
`new_or_omitted_authority_behavior_fails_closed`,
`decision_denominator_includes_every_registered_sink_and_rule_2_10_site`,
`unannotated_new_decision_site_fails_closed`,
`decision_omitted_from_authority_and_residual_still_fails`,
`unknown_dynamic_ffi_macro_or_effect_target_blocks_release`,
`removing_sink_registry_row_or_adding_effect_wrapper_fails`,
`every_denominator_site_has_exactly_one_behavior_owner`,
`one_behavior_may_span_multiple_sites_and_files`,
`source_and_test_may_participate_in_multiple_clusters`,
`one_cluster_contains_exactly_one_behavior`,
`mega_cluster_or_reused_evidence_identity_is_rejected`,
`behavior_registry_rejects_unknown_duplicate_and_empty_membership`,
`every_cluster_has_exactly_one_fresh_evidence_record`,
`production_evidence_rejects_fixture_dry_run_and_stale_artifacts`, and
`disabled_evidence_requires_same_binary_exact_banner`.

### 3.6 Per-capability production evidence contract

Coverage is not allowed to repair or redefine production behavior. When a new
test exposes a behavior defect, the slice records the failing command, opens or
returns to the owning feature's Gate A, and stops. The behavior fix, regression
test, producer integration, rollback, and production evidence belong to that
feature PR; the coverage PR may later adopt the passing test without claiming
the fix.

Every machine-enumerated behavior-test cluster records one independently
reviewed result in the release evidence manifest. The following table explains
the domain-specific payload expected by the registry; it does not enumerate or
limit the cluster set:

| Capability | Required non-coverage evidence |
| --- | --- |
| market/news/account producer or adapter | real provider/source/batch identity and freshness from producer → typed gateway adapter → consuming decision; unavailable/unsupported remains an explicit typed failure |
| selection/opportunity/review | real admitted batch → candidate/decision → push disposition → durable audit identity; discarded candidates remain explicit |
| notification/durable delivery | real producer → governance → sink receipt → `push.delivery.audit` exact join, with no raw recipient/credential in evidence |
| broker/risk/trading/order | real fresh account/quote evidence → registered safety checks → durable order audit; any live order canary requires its own controlled authority and is never triggered by coverage |
| CLI/operational binary | exact production input authority → operation result → durable DB/audit readback, or fail-closed result with no partial write |

If a real producer is absent or intentionally unsupported, the same release
binary must print one exact startup line
`[GateD][<cluster_id>] disabled=<reason_code> producer=none` and the evidence
manifest records that literal line and source SHA. Silence, a test fixture, a
verified-empty batch from an unavailable source, or coverage percentage cannot
substitute. Pure computation/rendering tests cite the owning live caller's
producer-to-audit evidence rather than inventing a new canary.

## 4. Implementation slices

This section is a sequencing contract, not an executable implementation plan.
After BR-202 registration and fresh Gate A C0/I0 acceptance, create
`docs/superpowers/plans/2026-08-02-gate-d-coverage-closure.md` with tiny
test-first steps and exact current paths. Until that plan exists, no slice below
may enter Gate B.

Every behavior slice begins with a characterization or required-contract test,
adds only a testability seam that preserves existing behavior, runs focused
tests, and regenerates the complete report. It never fixes a discovered product
defect inside the coverage slice; Section 3.5 returns that defect to its owning
Gate A. Percentages are observed outputs, not promised yield. Independent slices
may run in parallel only when their source paths do not overlap; all Cargo
invocations remain serialized because they share the artifact directory.

### Slice 0: make the metric honest

Files:

- `tools/coverage/check_thresholds.py`
- `tests/test_coverage_thresholds.rs`
- `tools/coverage/behavior_authorities.v1.json`
- `tools/coverage/behavior_residuals.v1.json`
- `tools/coverage/behavior_clusters.v1.json`
- `tools/coverage/decision_sinks.v1.json`
- `tools/coverage/decision_site_owners.v1.json`
- `tools/coverage/decision_denominator.schema.v1.json`
- `tools/coverage/extract_decision_denominator.py`
- `tools/coverage/rustc_decision_capture.py`
- `tools/coverage/fixtures/decision_probe/**`
- `tools/coverage/dependency_snapshot.v1.json`
- `tools/coverage/build_profile.schema.v1.json`
- `tools/coverage/gate_d_bundle.schema.v1.json`
- `tools/coverage/gate_d_attestation.schema.v1.json`
- `tools/coverage/verify_gate_d_attestation.py`
- `tests/test_gate_d_attestation.rs`
- `docs/business_rules.md` (BR-202)

Tests first:

- `complete_repository_inventory_classifies_every_source_exactly_once`
- `all_production_contexts_root_bridges_and_operational_bins_are_core`
- `global_only_files_are_exact_and_cannot_mask_a_new_path`
- `missing_or_extra_inventory_path_is_invalid_report`
- `missing_or_extra_sibling_proof_artifact_is_invalid_report`
- `normalized_duplicate_file_identity_is_invalid_report`
- `policy_floors_cannot_be_lowered`
- `policy_floors_may_be_raised`
- table-driven invalid threshold tests for string/bool/NaN/infinity/zero/negative
  and below-floor values
- table-driven invalid counter tests for string/bool/NaN/infinity/fractional/
  negative/zero-count/covered-greater-than-count values
- `llvm_cov_report_requires_exactly_one_run`
- `totals_must_equal_exact_sum_of_all_file_rows`
- `inventory_output_is_sorted_complete_and_hashable`
- `inventory_generation_failure_never_hashes_or_promotes_temp_output`
- `report_missing_path_requires_registered_same_sha_zero_proof`
- `zero_proof_requires_compiled_dependency_and_zero_show_regions`
- `zero_proof_sets_and_artifact_hashes_must_reconcile_exactly`
- `diagnostic_missing_candidates_never_satisfy_gate`
- `zero_listed_source_gaining_executable_line_is_invalid_report`
- strict integer boundary tests from Section 2.3
- independent decision denominator/sink closure/site-owner totality, behavior
  authority/residual exact-union, one-behavior-per-cluster, many-to-many
  source/test participation, and evidence tests from Section 3.5
- source/object-build-ID/profraw/profdata/mapping/toolchain/Cargo graph
  reproducibility and forgery tests from Section 2.4.1
- vendor/config/snapshot/lock exact-match and missing-dependency preflight tests
  from Section 2.4.2
- every stale/forged attestation negative from Section 4.2

Focused command:

```bash
cargo test --test test_coverage_thresholds -- --test-threads=1
```

Expected: all named tests pass; every invalid fixture exits 2 before printing a
passing threshold; lower-floor requests exit 2. A stale source hash and a fresh
same-SHA fixture where a zero-listed file gains one executable region each exit
2. Tests run the checker with complete minimal source, report, LLVM-show,
dependency and zero-proof inventories in an invocation-unique temporary CWD,
never with a repository-root override.

### Slice 0b: isolate every release entrypoint

Files:

- `tools/coverage/run_isolated_gate.sh`
- `tools/coverage/gate_d_journal.py`
- `tools/coverage/entrypoint_inventory.v1.json`
- `tools/coverage/isolation_policy.v1.json`
- `tools/coverage/host_execution_inputs.schema.v1.json`
- `tools/coverage/host_execution_policy.v1.json`
- `tools/coverage/capture_host_execution_inputs.py`
- `tools/coverage/local_publication_terminal.schema.v1.json`
- `tools/coverage/portable_archive.schema.v1.json`
- `tools/coverage/portable_authority_terminal.schema.v1.json`
- `tools/coverage/portable_signers.v1.json`
- `tools/coverage/build_portable_archive.py`
- `tools/coverage/sign_portable_terminal.py`
- `.cargo/gate-d-vendor-config.toml`
- `vendor/gate-d/**`
- `.github/workflows/coverage.yml`
- `README.md`
- `tests/test_coverage_entrypoints.rs`

Tests first cover direct checker/raw llvm-cov release rejection, parsed CI and
README entrypoint uniqueness, complete tracked invocation classification,
ordinary raw diagnostic exit semantics with no release authority, every
forbidden/unknown environment and resource class, production read/write and
network/sink trace reconciliation, vendor/offline bootstrap, bundle compatibility
and every wrapper lifecycle injection in Section 3.4.1. They also cover the
closed host linker/SDK/shell/Python/utility/dynamic-library trace, canonical tar
double-build equality, local-only path/inode facts, detached signature, outer
mode loss, download to a different path/device/inode, safe extraction and full
portable re-verification. The currently existing
`tests/test_coverage_thresholds.rs` has no BR-202 citation, while
`tests/test_coverage_entrypoints.rs` and the wrapper do not yet exist; adding the
citation/test/implementation is explicit Gate B debt and is not evidence from
this Gate A edit. Every new path first joins the BR-202 Code cell and cites the
rule in that same slice. No behavior-test slice begins until the wrapper exports
and fsyncs retained object bytes, candidate-verifies, cleans up, publishes and
fsyncs the final local bundle, durably publishes and confirms the local terminal,
constructs the canonical archive, signs its content-only portable terminal,
verifies all three portable files from a new no-follow extraction root, records
portable export confirmation, and only then prints PASS.

### Slice 1: Selection-v2 persistence and schema

Files:

- `src/database/selection_v2_repository.rs`
- `src/database/selection_v2_read_model.rs`
- `src/database/global_schema_v1.rs`
- `src/selection/schema_v2.rs`
- `src/selection/outcome_v2.rs`

Behavior matrix:

- migration/open/readback success and explicit SQL/transaction/fsync failure;
- TEST_CODE production/test namespace rejection;
- canonical identity conflict, duplicate and stable pagination boundaries;
- missing/stale/cross-batch evidence and manual-confirmation pending state;
- crash-recovery states, generation/fence CAS, audit-link/hash validation;
- empty verified cohort versus unavailable/partial cohort;
- stable full-result ordering before any registered presentation limit.

Tests stay in the existing colocated modules and real isolated SQLite files.

```bash
cargo test --lib database::selection_v2_repository::tests -- --test-threads=1
cargo test --lib database::selection_v2_read_model::tests -- --test-threads=1
cargo test --lib database::global_schema_v1::tests -- --test-threads=1
cargo test --lib selection::schema_v2::tests -- --test-threads=1
cargo test --lib selection::outcome_v2::tests -- --test-threads=1
```

### Slice 2: Unified data gateways

Priority files:

- `src/data_gateway/outcome_daily_bars.rs`
- `src/data_gateway/historical_bars.rs`
- `src/data_gateway/market_capabilities.rs`
- `src/data_gateway/chain_intelligence.rs`
- `src/data_gateway/board.rs`
- `src/data_gateway/capital.rs`
- `src/data_gateway/magic_tdx_t0.rs`

Behavior matrix:

- exact request/result identity and batch cardinality;
- provider/source/source-time/observed-time separation;
- complete, verified-empty, unavailable, partial, conflict, unsupported, and
  stale outcomes;
- OHLC/amount/date continuity/split consistency and source percentage checks;
- greater-than-20% adjacent change requires its registered manual-confirmation
  receipt rather than a fixed blind rejection or silent acceptance;
- stable complete-batch provider routing with no cross-source field merge;
- capture-audit write/readback failure closes admission.

Fixtures use current Magic protocols and `TEST_CODE_`; retired RustDX/local HTTP
provider code is not recreated.

```bash
cargo test --lib data_gateway::outcome_daily_bars::tests -- --test-threads=1
cargo test --lib data_gateway::historical_bars -- --test-threads=1
cargo test --lib data_gateway::market_capabilities -- --test-threads=1
cargo test --lib data_gateway:: -- --test-threads=1
```

### Slice 3: Durable delivery, event, risk, trading, broker, and calendar

Files:

- `src/durable_delivery/{coordinator,tests}.rs`
- uncovered modules under `src/event/`, `src/risk/`, and `src/trading/`
- `src/broker.rs`
- `src/calendar.rs`

Behavior matrix:

- reservation/retry/uncertain/terminal/fence/receipt transactions and crash
  recovery with no duplicate sink authority;
- append-only audit lock, partial-tail, hash-link, write, flush, and sync failure;
- cash, lot, daily-price-range, 60-second idempotency, and >=500,000 RMB
  confirmation;
- missing/stale broker evidence rejected before order construction;
- checked-in trading-calendar success, non-session, invalid artifact/hash, and
  timezone boundaries;
- BR-201 paper-exit closed-session zero-provider path only after BR-201 reaches
  Gate B and its interfaces stop changing.

```bash
cargo test --lib durable_delivery::tests -- --test-threads=1
cargo test --lib event:: -- --test-threads=1
cargo test --lib risk:: -- --test-threads=1
cargo test --lib trading:: -- --test-threads=1
cargo test --lib broker::tests -- --test-threads=1
cargo test --lib calendar::tests -- --test-threads=1
```

### Slice 4: monitor presentation and orchestration

Files:

- `src/bin/monitor/push_templates.rs`
- `src/bin/monitor/main.rs`
- `src/bin/monitor/notify.rs`
- the final BR-196 registry/transport modules

This slice starts only after BR-196 and BR-201 have stable Gate B interfaces.
It covers pure renderer contracts, typed dispatch results, CLI exit mappings,
session skips, governance denials, durable binding, sink failure, and test/live
isolation. It never sends to Feishu during coverage. BR-196 dry-run exercises the
complete template manifest with zero transport; the separately authorized
non-production acceptance target supplies real receipt evidence outside llvm-cov.

```bash
cargo test --bin monitor -- --test-threads=1
cargo test --test monitor_help_isolation -- --test-threads=1
```

### Slice 5: measured core tail

Regenerate the full report. Sort the complete core vector by
`(-missed_lines, repository_relative_path)` and take the next non-overlapping
behavior cluster. The diagnostic display may show 25 rows, but closure always
aggregates all core rows and repeats until the default checker exits 0.

Candidates after the first clusters include `monitor/news_ai.rs`,
`selection/audit.rs`, `selection/ingress_v2.rs`, remaining database catalogs,
decision paths, and pipeline orchestration. Do not add a test merely because a
file appears high in the ranking; first state the business outcome and failure
boundary it proves.

### Slice 6: global confirmation

Core closure should also close the current 2,518-line global lower bound. If the
fresh report still fails global, cover the exact GlobalOnly research/test
binaries and regression modules from Section 2.2 through their existing
interfaces. Do not reclassify them as core, execute real credentials/providers,
or exclude them from the workspace report.

### 4.1 Ordered commits, attestations, and rollback

Slices are independently reviewable source commits in this fixed order; a later
slice never rewrites an earlier attestation:

1. `coverage-scope`: BR-202 + typed inventory/checker + checker tests;
2. `coverage-isolation`: detached-worktree wrapper + negative isolation tests;
3. `coverage-selection-data`: Selection-v2 and unified gateway behavior tests;
4. `coverage-delivery-risk`: durable delivery/event/risk/trading behavior tests;
5. `coverage-monitor`: stable BR-196/BR-201 monitor behavior tests;
6. one or more `coverage-tail-N` commits selected from a fresh complete ranking;
7. `coverage-gate-d-evidence`: no behavior changes; it freezes the final source
   tree for one fixed-SHA report.

Self-referential same-commit attestations are forbidden. Each source commit `S`
is followed by one docs-only attestation commit `A` whose parent is exactly `S`.
`A` adds
`docs/releases/gate-d-coverage/<ordinal>-<S>.json`; the JSON records `S`,
`S^`, the source-tree hash, normalized paths changed by `S`, focused command and
result, report/show/dependency/zero-proof/inventory SHA-256 values, the closed
host-input/trace hashes, local bundle/terminal/journal-confirmation hashes, the
portable archive/terminal/signature hashes and approved signer key ID,
global/core exact counters, isolation marker, owning capability evidence or
disabled banner, and reviewer result. Portable locations and local
path/device/inode values are deliberately absent from the portable identity. It
deliberately does not contain `A`'s own commit SHA.
After `A` exists, PR evidence records `A` from Git and verifies
`git rev-parse A^ == S`; the artifact byte hash binds that external Git identity
without an impossible recursive self-SHA. `git diff --name-only S A` must contain
only that ordinal's `docs/releases/gate-d-coverage/**` artifact. An attestation
may say `planning_only`; it may not say PASS before every recorded command ran
at `S`.

The final merged head may therefore be the docs-only `A`, while the covered
source commit is `S`. Release evidence must prove that the complete Section
2.4.1 fixed-source input set—including `tests/**`, every `build.rs`, examples,
benches and compile/runtime inputs—has identical tree/blob hashes at `S` and
`A`; any non-attestation input change makes the report stale. This ordered `S -> A` model is physically
constructible and preserves fixed-source evidence without speculating about a
future commit identity.

For each literal pair, the attestation verifier runs this exact shape and pastes
the outputs; placeholders are replaced before execution:

```bash
S='paste-literal-40-hex-source-sha'
A='paste-literal-40-hex-attestation-sha'
ORDINAL='paste-literal-ordinal'
ATTESTATION="docs/releases/gate-d-coverage/${ORDINAL}-${S}.json"
test "$(git rev-parse "$A^")" = "$S"
test "$(git diff --name-only "$S" "$A")" = "$ATTESTATION"
git diff --quiet "$S" "$A" -- . \
  ':(exclude)docs/releases/gate-d-coverage/**'
tools/coverage/verify_gate_d_attestation.py \
  --source "$S" --attestation-commit "$A" --attestation "$ATTESTATION" \
  --verify-source-pair-only
git show "$A:$ATTESTATION" | shasum -a 256
```

Any nonzero `test`/`git diff --quiet`/semantic-verifier result, extra changed
path, unequal fixed-input manifest/hash, missing artifact, or artifact hash
mismatch rejects `A`.

Rollback is reverse-order `git revert` of the smallest causal **source** commit.
Committed attestation files are never reverted or deleted; a following docs-only
supersession record names the reverted source/attestation pair and marks its
result invalid. A checker/inventory defect returns to Gate A and reverts the
`coverage-scope` source commit plus dependent source commits whose attestations
relied on it. An isolation defect returns to Gate B and invalidates all later
coverage reports. A behavior defect reverts only the owning testability
seam/test source commit and returns to that feature Gate A; it does not relax the
test. Audit, market/account evidence, receipts, committed attestations, and
generated production data are never deleted. Any source revert invalidates its
and all descendant coverage attestations; a new fixed-SHA report and appended
supersession/attestation pair are mandatory. The signed portable terminal and
archive are retained; a key/archive defect additionally requires a signed
supersession record naming the old archive hash, terminal hash, signer key ID,
reason, replacement version, and replacement source/run identity. Rollback
never re-signs old bytes or restores standalone JSON/local inode authority.

### 4.2 Attestation schema and independent semantic verification

Every attestation validates against the closed schema
`tools/coverage/gate_d_attestation.schema.v1.json` with identity
`gate-d-coverage-attestation/v1`; unknown fields or schema versions fail. The
only verifier is `tools/coverage/verify_gate_d_attestation.py`. It does not trust
precomputed JSON values or the wrapper's PASS line. Given literal source commit
`S`, attestation commit `A`, attestation path, and the three portable artifact
files (or the pinned local run only in pre-portable candidate mode), it
independently performs these field-level algorithms:

| JSON field | Independent recomputation and equality rule |
| --- | --- |
| `source.commit_sha` | require 40 lowercase hex; read the commit object named by `S`, hash its exact canonical `commit <len>\0<body>` bytes with Git SHA-1, and require result = argument = JSON; a symbolic ref is resolved before comparison |
| `source.parent_sha` | require exactly one parent and compare JSON to `git rev-parse S^`; root/merge source commits are rejected |
| `source.tree_manifest_sha256` | regenerate Section 2.4.1's closed fixed-source set from Cargo metadata and Git at `S`: workspace/path-dependency manifests, Cargo lock/config/toolchain files, every `build.rs`, `src/**`, `tests/**`, `benches/**`, `examples/**`, config/vendor/tools/workflow/README and every compiler/test fixed input; hash exact sorted NUL-delimited mode/type/object/path plus per-file SHA-256 rows, require pre/post detached-worktree equality, and reject modified/untracked input bytes even when `HEAD == S` |
| `source.slice_id` and `source.changed_paths` | recompute normalized `git diff --name-only -z S^ S`, reject symlink/submodule/absolute/`..` identities, require exact sorted equality to `changed_paths`, load `slice_id`'s closed allowed-path set from the v1 schema at `S`, and require every changed path to match it; a JSON-provided allowlist cannot expand the schema |
| `attestation.path` plus external `A` | recompute canonical Git hash of the externally supplied `A`, require `A^ == S`, require `git diff --name-only -z S A` to contain exactly the JSON path `docs/releases/gate-d-coverage/<ordinal>-<S>.json`, and require the path's `<S>` token to equal `source.commit_sha`; JSON deliberately contains no self-referential `A` SHA |
| `portable_artifact`, `local_publication` and `coverage.artifacts` | before extraction, validate the closed portable-terminal schema, approved signer ID, detached Ed25519 signature and archive size/SHA-256; parse canonical tar headers, reject unsafe/duplicate/extra members, safely extract into a new no-follow temp root and recheck internal modes/hashes. Validate the local bundle manifest and read-only canonical/root JSON equality, then validate copied local terminal and durable journal-confirmation hashes as historical content without comparing the downloader path/device/inode. Reject local directory upload, outer-mode assumptions, extra/missing files, partial/quarantined staging, candidate-only attestation, absent/mismatched signature, diagnostic artifact name or standalone JSON. |
| `coverage.global`/`coverage.core` | parse `coverage.json` with Section 2.3's raw-token integer rules; independently sum every file, reclassify the complete source inventory at `S`, and require exact covered/count/missed counters plus 80%/95% result; recorded percentages are display-only |
| `dependencies` | hash Cargo.toml/lock/config plus every source-controlled vendor byte at `S`, reconstruct `dependency_snapshot.v1`, require the pinned Magic revision and exact package/checksum/source set, run Cargo metadata locked+offline against a credential-free empty CARGO_HOME/read-only vendor, and compare the resolved graph; ambient cache or self-reported metadata is not accepted |
| `build_profile` | perform Section 2.4.1's source/build/profile algorithm: recompute tool hashes/versions and compile argv/features/targets/generated inputs, validate the exact closed `gate-d-object-store/v1` blob set, independently extract every retained object's hash/size/mode/build ID/coverage-map hash, remerge the exact profraw set, verify profdata binary IDs, rerun llvm-cov export on the ordered retained blobs, and require exact normalized report/mapping/all-source-and-test-input equality |
| `host_execution` | execute Section 2.4.3's feature probe; independently hash every absolute executed tool/interpreter/module/linker/SDK/sysroot/dynamic-library input, compare OS/kernel/arch/locale/TZ/environment, reconcile every exec/open/read event in both directions, and require pre/post host-tree equality. Unknown or unavailable host inputs and a missing macOS/Linux enforcement/trace backend fail. Local device/inode assists pinned reads but is not compared after portable download. |
| `inventory` | enumerate `git ls-tree -r --name-only S -- src` and retain exact `.rs` suffixes, independently normalize/classify all paths, and compare the complete ordered rows, current exact file/directory/root/bin counts, classification counts, row hashes and aggregate hash; no JSON count is accepted without the rows |
| `zero_instrumented` | read the exact source-controlled zero set at `S`, recompute source-byte hashes, compiler participation, show-region count and exact report-missing set from export artifacts, then require ordered set equality and registry/proof hashes; extra or absent rows fail |
| `isolation` | parse the exported structured isolation record, recompute the policy hash and allowed/forbidden resource sets at `S`, reconcile the full file/socket trace, and require fixed SHA/worktree identity plus all production read/write, external-connect and sink-exec counters = 0; a stdout marker alone is insufficient |
| `entrypoints` | scan every tracked path and parsed command/block class in Section 2.5.1, regenerate the exact invocation inventory, require exactly one release minter, require raw ENGINEERING commands to remain internal diagnostic generation, and reject new/changed/unclassified aliases or indirect callers |
| `decision_denominator` and `behavior_clusters` | run the exact Section 3.5.1 `probe`, capture build, `generate` and `verify` commands at `S`; require their literal PASS markers, zero unresolved targets/classes, `domain(M)=D`, exactly one owner per site, `range(M)=B`, `A∩R=empty` and `B=A∪R` by exact rows/cardinalities/hashes. Require one behavior per cluster with many-to-many sites/source/tests and one distinct behavior-specific production/disabled evidence record; reject an unavailable compiler interface, target/cfg/feature drift, unannotated/unregistered or omitted decision/sink/Rule-2.10 site, missing/extra/duplicate authority identity, mega cluster, reused evidence, stale/test/dry-run evidence or wrong binary build ID. |

All set and list equality is cardinality plus byte-for-byte ordered-row equality;
all counters use Section 2.3's exact-integer algorithm. For portable input, the
verifier pins the three outer files, verifies signature/hash, creates one new
safe extraction directory, then pins that directory and uses descriptor-relative
no-follow reads so a path swap cannot change bytes between hashing and parsing.
It emits a semantic PASS record only after every row above succeeds. The wrapper
pre-attestation verification runs the same artifact/inventory/isolation/cluster
algorithms without `A`; after `A` is created, the complete mode additionally
binds the Git parent/child and path restrictions.

Required stale/forgery negative fixtures independently mutate: source SHA,
source parent, attestation parent/path, extra attestation path, slice path,
source/vendor tree; Cargo metadata/lock/config/package/checksum; host OS/kernel/
locale/TZ, shell/Python/module/linker/SDK/system library/tool bytes or trace;
compile argv/feature/target; object bytes/build ID/mapping; profraw set,
profdata or binary IDs; coverage bytes/hash/size/counter/totals; inventory and
zero proof; bundle layout/compatibility copy/local terminal/journal; archive
header/order/padding/member/mode/hash, portable terminal/key/signature and a
changed download path/device/inode;
entrypoint inventory/alias; isolation policy/trace; sink registry, decision
denominator/site owners, authority/residual/equivalence/cluster sets; reused/
missing/duplicate/stale/dry-run evidence; and disabled-banner
reason/binary ID. Each exits 2 with no semantic PASS. Rewriting all companion
JSON hashes to match forged bytes, copying a valid attestation to another
commit, a standalone coverage JSON, or the old stdout marker also fails because
the verifier independently regenerates Git, dependency, object/profile/mapping,
set, trace, lifecycle, and topology relations.

## 5. Failure modes

| Failure | Required result | Gate return |
| --- | --- | --- |
| Report missing/malformed/stale relative to fixed SHA | reject evidence; regenerate | Gate D |
| Source path unclassified/multiply classified or report/inventory differ without the exact zero-proof exception | checker exits 2 | Gate A/B |
| Report-missing source lacks exact registered same-SHA compiler/show zero proof, or any proof/hash/set differs | checker exits 2; do not auto-register it | Gate A/B |
| Zero-listed source gains an executable report/show region or its source bytes change | checker exits 2; remove/re-review the zero registration | Gate A/B |
| Duplicate normalized filename, invalid shape/counter, totals mismatch | checker exits 2 | Gate B |
| Requested minimum below 80/95 | checker exits 2 | Gate B |
| CI/README/script/spec/plan/alias treats raw llvm-cov or checker as release authority | raw command keeps its ordinary diagnostic exit; it cannot create candidate attestation, local terminal/confirmation, portable archive/terminal/signature or PASS, and the caller must migrate to the wrapper | Gate B |
| Tracked coverage invocation is new, changed, indirect, missing or multiply classified | repository-wide entrypoint inventory test exits 2; classify/migrate it before release | Gate A/B |
| Git-pinned/vendor dependency is missing, writable, stale, credential-bearing, not exact to Cargo.lock, or needs network/cache | persistent journal records dependency bootstrap failure; no RUN_ROOT/Cargo coverage starts | Gate A/B |
| Host feature probe, absolute tool/dynamic-library/SDK manifest, environment or exec/open/read trace is missing, unavailable, mutable, unknown or differs pre/post | journal records the exact host blocker; exit 2 before dependency bootstrap or reject the generated run | Gate A/B/D |
| Fixed source/test/build input is dirty/untracked, generated input/read is unbound, or retained object/blob/build ID/profraw/profdata/mapping/toolchain/feature/target/Cargo graph/recomputed report differs | verifier exits 2; report bytes/counters cannot authorize release | Gate B/D |
| Isolation wrapper sees real symbol/credential/path/alias | exit 2 before Cargo | Gate B + red-line review |
| Unknown child environment/resource/DSN, production read, external connect, or sink exec | deny, record sanitized reason, exit 2 | Gate B + red-line review |
| Original production manifest changes or detached tree has a production-shaped write | invalidate report; determine external writer versus test escape | Gate B + red-line review |
| Persistent journal root/initial fsync fails | preflight exits 2 with `run_root_created=false`; no RUN_ROOT/attestation/PASS | Gate B |
| Generation/export/fsync/verification/cleanup fails before candidate attestation | no candidate attestation, local/portable terminal or PASS; append immutable first cause/cleanup terminal when writable | Gate B |
| Candidate attestation/bundle rename/bundle-parent fsync/local-terminal write, rename, parent-fsync or confirmation-journal fsync fails | candidate/final-path bundle remains locally quarantined; no portable files/PASS; preserve bundle/terminal/journal and repeat the full protocol under a new run ID | Gate B |
| Canonical archive differs on rebuild, has an unsafe/noncanonical member, or archive/portable-terminal/signature/fsync/safe-extraction verification fails | local proof remains non-portable; quarantine every outer file, upload no `coverage-report`, preserve evidence and rerun under a new run ID | Gate B/D |
| Download path/device/inode or outer file mode differs while signed bytes are equal | ignore those outer/locality facts; verify signature/archive, safely extract, restore/check internal modes and continue | no failure |
| Fixture accepts missing/stale/partial data as complete | explicit failure; return to owning design | Gate A/B |
| Coverage exposes a real product defect | record failure and stop; open/return to owning feature Gate A, never fix in coverage slice | owning Gate A |
| New code expands denominator | add behavior tests in the same PR; never exclude it | Gate D |
| Live canary unavailable | report exact blocker; isolated protocol evidence does not impersonate live success | Gate D blocked |
| Capability lacks real chain evidence and lacks exact disabled banner | do not attest capability; production silence is a defect | owning Gate A/D |
| Decision denominator/sink extraction is unresolved, `domain(M) != D`, a site has zero/multiple owners, authority/residual/cluster exact union differs, a behavior has zero/multiple clusters, one mega cluster contains multiple behaviors, or evidence is reused/stale/wrong-mode | registry/verifier exits 2; annotations or source/test file counts cannot substitute | Gate A/B/D |
| CI upload/consumer expects standalone JSON or bundle layout/artifact identity differs | use the v1 root compatibility copy diagnostically, migrate release consumers to canonical bundle+verifier, and keep Gate D blocked | Gate B/D |
| Attestation commit is not docs-only, does not directly follow its recorded source SHA, or source trees differ | reject attestation and regenerate from a new fixed source commit | Gate D |
| Any semantic attestation field differs from independently recomputed Git/source/export/trace sets | verifier exits 2 with no semantic PASS | Gate D |
| Any Critical/Important independent finding | fix and rerun from failed gate | corresponding gate |

## 6. Old-module disposition

| Module/evidence | Adopt or reject | Reason |
| --- | --- | --- |
| `tools/coverage/check_thresholds.py` | adopt and deepen | existing diagnostic threshold parser; replace scattered 15-prefix knowledge with one complete fail-closed ownership/validation seam, while only the wrapper mints release authority |
| `tests/test_coverage_thresholds.rs` | adopt and extend in Gate B | existing process-level checker contract currently lacks its required BR-202 citation; this design does not edit the test |
| `.github/workflows/coverage.yml`, `README.md` raw coverage commands | migrate in Gate B | both currently present raw diagnostics as the gate; after migration the wrapper alone mints authority while raw commands retain ordinary diagnostic exits |
| `docs/ENGINEERING_RULES_V2.md` raw baseline | adopt unchanged | active required internal generation/check sequence executed by the wrapper; never a second attestation/PASS minter |
| invocation matches in prior specs/plans/reports | retain text as invocation-level historical-only | complete tracked-tree inventory prevents stale snippets becoming implicit release callers |
| `tools/coverage/entrypoint_inventory.v1.json` and `tests/test_coverage_entrypoints.rs` | add; names/schema frozen | classify every active/historical invocation and reject aliases/indirection; both are planned Gate-B paths and must join BR-202's Code cell with literal citations in the same staged slice before creation |
| decision capture/probe, sink/denominator/site-owner plus behavior authority/residual/cluster registries | add, names/schema/CLI frozen | exact toolchain feature probe and capture/generate/verify commands build an independent denominator; every site has one behavior owner, while authority+residual defines behavior identities and source/test membership remains many-to-many support |
| source-controlled `vendor/gate-d/**`, Gate-D Cargo config and dependency snapshot | add, names/schema frozen | physically runnable offline build for Git-pinned Magic crates without ambient cache or credentials |
| host-execution schema/policy/capture | add, names/schema frozen | bind every linker/SDK/system-library/shell/Python/utility/dynamic dependency and observed exec/open/read; unknown host input or unavailable trace backend fails |
| v1 local bundle/object-store/build-profile, local terminal/journal and portable archive/terminal/signature schemas | add, names/schema frozen | local path/device/inode proves only local durability; signed archive content provides portable authority after safe extraction and complete verification |
| `tools/coverage/isolation_policy.v1.json` | add, name/schema frozen | closed environment/resource/DSN/network policy; unknowns fail rather than expanding an open store set |
| attestation schema/verifier paths in Section 4.2 | add, names/schema frozen | independently recompute semantics instead of trusting JSON hashes or wrapper output |
| `src/lib.rs`, all production contexts/root files/operational bins | adopt as core inventory | live runtime/data/trading/delivery ownership currently omitted or only partially counted |
| test/research exact files in Section 2.2 | retain global-only | still count toward global 80%; exact disposition prevents broad exclusions |
| existing colocated tests and `cfg(test)` loopback seams | adopt after review | test-only, deterministic, no production source fallback |
| standalone `target/coverage/coverage.json` dated 2026-08-01 | reject as release proof | planning baseline predates active work; v1 uploads a signed portable archive and retains only a diagnostic compatibility copy inside it |
| 2026-07-18 coverage plan | retain as historical, reject as execution authority | stale provider paths, RustDX, and PR identity |
| old provider-specific acquisition/RustDX work items | reject | unified Magic Gateway is current production acquisition owner; surviving compatibility types remain core |
| BR-196/BR-201 work-in-progress tests | adopt only after Gate B interface freeze | avoid testing transient or conflicting implementations |
| ignored live integrations as line-coverage substitutes | reject | provider liveness is separate Gate D evidence |
| product behavior fixes discovered during coverage | reject from this slice | return to owning feature Gate A and production evidence contract |

## 7. BR-202 registration text

Because `docs/business_rules.md` is concurrently modified by BR-196/BR-201, the
worktree row is frozen here and synchronized byte-for-byte with the BR-202 row
in `docs/business_rules.md`; neither copy is claimed as index/HEAD authority
until Section 0.1 passes. Future edits must update both without overwriting
concurrent changes:

```markdown
| BR-202 | 🟡 Gate A sixth-remediation worktree candidate after RED C1/I4/M1; not accepted index/HEAD authority; exact two-blob staging and fresh independent C0/I0 required | Gate D release authority is minted only by `tools/coverage/run_isolated_gate.sh` from one clean fixed-SHA workspace/all-features llvm-cov line run with immutable floors global>=80% and core>=95%; raw llvm-cov/checker commands retain ordinary diagnostic exits and cannot mint authority. The fixed-source closure binds every workspace/path-dependency manifest, Cargo lock/config/toolchain input, build.rs, src, tests, benches, examples, config, tools, workflow, README, generated input and observed compiler/test read; a closed host contract additionally binds every shell/Git/Python/module/linker/clang/SDK/sysroot/system-library/tar/utility/dynamic dependency, OS/kernel/arch/locale/TZ and exec/open/read trace, with unknown or unavailable inputs failing. Git-pinned Magic dependencies resolve only from exact Cargo.lock-bound credential-free read-only vendor bytes. Coverage scope classifies every normalized src Rust file exactly once with canonical integers, exact sums, same-run zero proof and immutable denominators. Retained read-only `build/objects/<sha256>` bytes bind object size/mode/build ID/mapping, every profraw and remerged profdata; the verifier re-extracts objects and reruns llvm-cov export. Local durability uses a final bundle, `gate-d-local-publication-terminal/v1` and later synced journal confirmation; local path/device/inode never becomes portable authority. CI `coverage-report` uploads only a deterministic `gate-d-portable-archive/v1`, detached content-only `gate-d-portable-authority-terminal/v1` and approved Ed25519 signature. The terminal binds archive/bundle/source/run/attestation/local-confirmation hashes but no download path/device/inode; download verifies signature/hash before safe no-follow extraction and then checks internal modes/links/manifest, including byte-identical read-only root `coverage.json`. The former proposed `gate-d-authority-terminal/v1` is withdrawn unused; migration, quarantine, key/archive supersession and rollback never restore standalone JSON authority. The invocation inventory consumes only `git ls-files` paths; untracked diagnostics are non-authority and any later tracked hit fails until classified. Behavior denominator D is generated only by the exact feature-probed compiler MIR/rustdoc/source-AST/expanded-macro `probe`/capture/`generate` interface; unavailable interfaces and unresolved call/FFI/dynamic/macro/generated/sink or Rule-2.10 classes exit 2. Total map M has domain exactly D and one behavior owner per site; A∩R is empty, B=A∪R, and an unannotated decision omitted from authority and residual still fails. Every behavior requires a distinct fresh real producer-to-audit record or exact same-covered-binary disabled banner; 2026-08-02 has neither and Gate D remains blocked. No exclusion, denominator reduction, stale/focused/library-only report, coverage-only product fix, unsigned artifact or evidence deletion may satisfy release. Current Code contains only the two existing Gate-A docs; every planned Gate-B path must join this row and cite BR-202 in the same staged source slice before creation/modification. | `docs/business_rules.md`, `docs/superpowers/specs/2026-08-02-gate-d-coverage-closure-design.md` |
```

The two worktree and, after staging, index row copies must be unique and
byte-identical before review. That mechanical synchronization does not close
Gate A; fresh independent C0/I0 review remains the blocker.

## 8. Machine-checkable acceptance criteria

1. `cargo test --test test_coverage_thresholds -- --test-threads=1` passes every
   Slice-0 parser/inventory contract. Both that file and
   `tests/test_coverage_entrypoints.rs` contain the exact BR-202 citation; the
   latter path exists and passes its repository-wide caller inventory tests.
2. Same-SHA inventory generation classifies all 408 `src/**/*.rs` files exactly
   once as Core 398 or GlobalOnly 10. It independently reproduces 36 top-level
   non-bin library directories, 29 root Rust files and 16 top-level bins; the
   three directory-list cardinalities are not added as if they were source-file
   counts. Every report omission has exact registered same-SHA compiler/show/
   hash/set zero proof.
3. Invalid report shape/counters/totals/duplicates/inventory/zero proof fixtures
   exit 2. Canonical raw integer tests accept at most `2^53-1`, reject `2^53`,
   `u64::MAX`, overflowed sums, signs, leading zeroes, fractions, exponents,
   strings, bool and null, and compare immutable 80/95 floors by exact integer
   arithmetic. A zero-listed source gaining one executable region fails.
4. Raw `cargo llvm-cov` and direct checker commands retain their ordinary
   diagnostic exits but, whether zero or nonzero, create no release context,
   `release-attestation.v1.json`, local terminal/confirmation, portable archive/
   terminal/signature, or release PASS.
   Only one parsed/tracked release minter exists: the wrapper. The wrapper embeds
   the exact ENGINEERING_RULES_V2 workspace/all-features raw argv sequence.
5. `tools/coverage/entrypoint_inventory.v1.json` equals a fresh scan of every
   tracked workflow/composite/shell/Make/task file, README/Markdown block,
   Python/Rust subprocess literal and token occurrence. CI, README, checker,
   tests, ENGINEERING rules, current design/plan, every prior matching spec/plan,
   and tracked reports/transcripts are each classified exactly once; every
   required row passes `git ls-files` at `S`. Untracked diagnostics supply no row
   or authority; if later tracked, they fail until classified. A new/changed
   alias or indirection fails.
6. Before any `RUN_ROOT`, worktree or export, the persistent caller-worktree
   journal root and pinned FDs are created and file/directory-fsynced. Initial
   journal, host-probe or dependency-preflight failure exits 2 with no RUN_ROOT.
   Every later generation/object-copy/export/file-fsync/directory-fsync/verify/
   cleanup/local-bundle/terminal/archive/signature injection leaves no portable authority or
   PASS, preserves immutable first cause plus cleanup terminal whenever the FD
   remains writable, quarantines any candidate/final-path bundle, and never
   deletes the only evidence, overwrites a run or later promotes an incomplete run.
7. An empty credential-free CARGO_HOME plus exact Gate-D Cargo config resolves
   `cargo metadata --locked --offline` from read-only `vendor/gate-d/**` with no
   network/cache. Cargo.lock, dependency snapshot, every vendor byte/checksum and
   Magic revision match exactly. Missing/writable/extra/stale/credential-bearing
   content, graph/lock drift or network lookup fails before RUN_ROOT.
8. The Section 2.4.3 probe and capture commands emit their exact PASS markers
   and bind every actual shell/Git/Python/module/linker/clang/SDK/sysroot/system-
   library/archive/utility/dynamic dependency plus OS/kernel/arch/locale/TZ and
   exec/open/read event. macOS lacking its calibrated deny+trace backend and any
   unknown/mutable/missing host input exit 2. Pre/post host tree hashes match.
   The v1 build profile also binds the complete fixed input set including all test/
   build-script/example/bench bytes, every generated input and observed read,
   Cargo graph/lock/config/vendor, tool hashes, compile features/targets, every
   exact retained `build/objects/<sha256>` blob with size/mode/build ID/mapping,
   every profraw and remerged profdata. Dirty/untracked detached inputs fail
   pre/post checks. The verifier re-extracts retained blobs, remerges profiles and
   reruns llvm-cov export; every Section 2.4.1 forgery exits 2 after rewritten hashes.
9. Run the literal Section 3.5.1 `probe`, capture build, `generate`, test-list and
   `verify` commands. They must emit the three exact PASS markers with
   `unresolved=0`; an unavailable compiler interface emits exact BLOCKED/exit 2
   and keeps Gate D blocked. The produced `D(S)` includes all in-scope production
   sink/entry and Rule-2.10 sites. Exact equations prove `domain(M)=D`, one
   owner per site, `range(M)=B`, `A intersection R` empty and `B=A union R`.
   Removing a sink, adding an effect wrapper, or adding an unannotated decision
   omitted from both authority and residual fails. Each cluster contains one
   behavior; sites/files/tests are many-to-many. Mega clusters and reused evidence fail.
10. Every behavior has one distinct fresh, real, behavior-specific
    producer→adapter→consumer/push→audit evidence record or exact disabled banner
    from the same covered binary/build ID. Missing/duplicate/stale/test/dry-run/
    wrong-mode evidence fails. The absence of 2026-08-02 push/event and real
    disabled-banner evidence remains an explicit Gate D blocker; the stale
    `56a270...1978f` report, if referenced, remains diagnostic only.
11. A successful wrapper publishes one local Section 2.5.2 bundle, one local
    publication terminal and its durable journal confirmation, then creates the
    canonical tar twice with byte equality and publishes exactly one portable
    archive, detached terminal and approved Ed25519 signature. The portable
    terminal binds archive/bundle/attestation/local-confirmation hashes but no
    download path/device/inode. Verification succeeds after moving the three
    files to a different path/device/inode and normalizing their outer modes,
    safely extracting and restoring/checking internal modes. CI uploads only the
    three portable files as `coverage-report`; failures use
    `coverage-diagnostics`. A local directory, unsigned tuple, candidate
    attestation, incomplete final path or standalone JSON cannot satisfy the verifier.
12. Consumer migration tests enumerate every tracked standalone-JSON reader.
    Release consumers use the signed archive plus safe-extraction semantic
    verifier; transitional alias readers are
    diagnostic and have owner/deprecation state. V1 retains the alias; v2
    removal requires two successful mainline v1 releases, zero readers and
    owner acknowledgements. Rollback retains evidence and never restores
    standalone upload as authority.
13. Every source slice `S` has one following docs-only attestation `A` with
    `A^ == S`; changed-path/tree equality covers the complete Section 2.4.1
    fixed-source input set, especially tests/build scripts/examples/benches.
    Rollback appends signed supersession without deleting local bundle/journal/
    terminal or portable archive/terminal/signature evidence.
14. `tools/coverage/verify_gate_d_attestation.py` independently recomputes every
    Section 4.2 Git/input, host execution, local bundle/journal/terminal,
    portable archive/terminal/signature, dependency, retained-
    object/profile/mapping/report, inventory/zero, isolation, entrypoint,
    decision-denominator/site-owner and behavior field. Only the exact valid
    relation emits semantic PASS; stored JSON hashes/stdout do not.
15. Gate A remains RED until a fresh independent reviewer returns C0/I0. Only
    after that review and a separate plan may Gate B begin; later Gate C/D still
    require fmt, strict Clippy, all workspace tests, compliance, release build,
    fresh coverage, current production/isolation evidence and independent review.
16. Before that review, Section 0.1 proves both current docs exist in the index,
    their index blobs equal reviewed worktree bytes, BR-202 is unique and
    byte-identical, and cached whitespace checks pass. Every Gate-B slice first
    adds its planned paths to Code with literal citations; nonexistent future
    paths are not claimed by the current row.

## 9. PR evidence template

```markdown
### Refs
- spec: `docs/superpowers/specs/2026-08-02-gate-d-coverage-closure-design.md`

### Data-Redlines
- [2.1] no production mock or fake success; only TEST_CODE isolated fixtures
- [2.3] bad/stale/partial/jump/manual-confirmation failure paths covered
- [2.4] freshness semantics unchanged and exercised
- [2.5] test/live DB, audit, sink, account and symbol isolation proven
- [2.6] cash/lot/limit/idempotency/secondary-confirmation tests pass
- [2.7] audit failure and durable readback paths covered
- [2.10] BR-202 registered before implementation

### OldModules
| module | adopt/reject | reason |
| --- | --- | --- |
| current coverage checker/tests | adopt | harden diagnostic parser/inventory; wrapper alone executes local durable publication plus signed portable archive authority protocol |
| 2026-07-18 plan | reject as execution authority | stale paths and PR identity |
| RustDX/local provider tests | reject | retired; Magic Gateway owns acquisition |

### Threshold-Proof
- fixed SHA: paste the literal output of `git rev-parse HEAD`
- local bundle manifest/release-attestation/local-terminal/journal-confirmation
  SHA-256 and exact local candidate paths; label path/device/inode local-only
- portable archive size/SHA-256, canonicalization version, detached terminal
  SHA-256, signer key ID and detached signature verification
- canonical/compatibility coverage SHA-256 and byte-equality proof:
  `coverage/coverage.json` and root `coverage.json`
- LLVM-show manifest SHA-256: paste the literal output of
  `shasum -a 256 <run-dir>/coverage/llvm-cov-show.manifest`
- instrumented-dependency manifest SHA-256: paste the literal output of
  `shasum -a 256 <run-dir>/coverage/instrumented-dependencies.manifest`
- zero-instrumented proof SHA-256 and exact registered/proved count: paste the
  literal checker output; omitted-report paths must reconcile exactly
- dependency snapshot/Cargo.lock/vendor tree hashes and exact locked offline
  metadata graph; Magic revision and credential/network counts
- complete fixed-input manifest hashes, including all tests, build scripts,
  examples and benches; paste the pre-dependency, pre-Cargo, post-generation
  and pre-export clean/untracked status plus pre/post tree/blob equality proof
- generated-input/read-trace reconciliation hash and unresolved-read count=0
- host execution manifest/environment/SDK-or-sysroot/dynamic-library/trace hashes;
  exact probe marker, pre/post equality, unknown tools/reads=0
- toolchain and compile invocation/features/targets; retained
  `build/objects/<sha256>` CAS count/bytes/mode/hash/build-ID/mapping proof,
  raw-profile set and remerged-profdata hashes
- independently re-extracted retained-object IDs/maps and regenerated
  object+profile llvm-cov report equality after build-directory cleanup
- global: paste the checker's complete global output line, required 80.00%
- core: paste the checker's complete core output line, required 95.00%
- complete inventory SHA-256 and exact counts: 408 total, Core 398,
  GlobalOnly 10; independently reproduce 36 directories, 29 roots, 16 bins
- no exclusion/denominator reduction: verified

### Isolation
- detached worktree source SHA: paste literal output
- paste the complete emitted `COVERAGE_ISOLATION status=PASS` marker
- isolation-policy v1 hash and exact environment/resource registry version
- negative real-symbol/credential/recipient/path/alias/unknown cases: exit 2
- OS trace: production reads/writes=0, external connects=0, sink execs=0
- lifecycle failure-injection suite: no PASS; diagnostics preserved
- persistent journal path/device/inode, initial fsync proof, immutable first
  cause and exactly one cleanup terminal; journal/dependency failure created no RUN_ROOT
- bundle rename and bundle-parent-fsync proof; local terminal path/device/inode/
  hash and parent-fsync proof; later local journal confirmation binds them
- canonical archive double-build equality; portable terminal/signature and
  parent-fsync proof; new-path/device/inode safe-extraction verification PASS
- each publication/fsync/archive/signature injection leaves no portable authority/PASS, preserves
  a quarantined candidate and requires a full new-run-ID rerun
- original production manifest before/after SHA-256: identical

### Entrypoints-And-Migration
- fixed-SHA tracked invocation inventory hash/cardinality, every required path
  passes `git ls-files`, and zero unclassified/duplicate/dead rows; untracked
  diagnostics are explicitly non-authority
- sole release minter: `tools/coverage/run_isolated_gate.sh`
- raw ENGINEERING/llvm-cov/checker commands: diagnostic only; ordinary exits,
  zero release attestations/PASS
- CI artifact: `coverage-report`, exactly canonical archive + detached portable
  terminal + detached signature after full portable verification; never upload
  the live directory or standalone JSON
- standalone JSON consumers: list owner, signed-archive migration or
  diagnostic compatibility-alias state, deprecation and rollback evidence

### Capability-Evidence
- decision-sink, Rule-2.10 operator, compiler/MIR/rustdoc/source-AST
  denominator, site-owner, authority/residual/alias/cluster schema versions,
  row hashes and cardinalities
- paste unresolved call/FFI/dynamic/macro/generated/sink classes=0 and the
  independent denominator `D` extraction/reconciliation hash
- paste the exact Section 3.5.1 probe/generate/verify markers and compiler
  feature/target/cfg/feature capture hashes; unavailable interface must show
  BLOCKED/exit 2 rather than a substituted extractor
- paste exact proofs: `domain(M)=D`, one owner per decision site,
  `range(M)=B`, duplicates=0, `A` intersection `R` empty, `B=A` union `R`,
  missing/extra decision sites=0, missing/extra behaviors=0 and one behavior
  per cluster
- source/test participation is many-to-many; paste unknown-path/test count=0;
  omission of an unannotated production decision or registered sink must fail
- paste the generated evidence index with one distinct behavior-specific
  production-chain artifact or same-covered-binary disabled banner per cluster;
  reused evidence identities=0
- 2026-08-02 production evidence: absent; Gate D BLOCKED until current evidence

### Business-Rules
- BR-202

### Validation
- `cargo fmt --all -- --check`: PASS
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: PASS
- `cargo test --workspace --all-targets --all-features -- --test-threads=1`: PASS
- `bash tools/compliance/check.sh`: PASS
- `cargo test --test test_coverage_entrypoints -- --test-threads=1`: PASS
- wrapper + semantic verifier + default coverage checker: PASS
- `cargo build --release --bin monitor`: PASS
- independent Gate B/C/D review: C0/I0

### Attestations
- for every source slice `S`, paste the following docs-only attestation commit
  `A`, `git rev-parse A^`, the attestation artifact SHA-256, and the exact
  `git diff --name-only S A` output
- prove every Section 2.4.1 fixed input—including `src/`, all `tests/`, every
  `build.rs`, `examples/`, `benches/`, Cargo manifests/lock/config/toolchain,
  workspace/path dependencies, `vendor/gate-d/`, `config/`, `tools/`, workflow
  and README—has identical tree/blob hashes between final covered source `S`
  and docs-only head `A`
- paste `tools/coverage/verify_gate_d_attestation.py` semantic verifier output;
  include host/dependency/generated-input/retained-object/profile/mapping/local-
  bundle/terminal/journal/portable-archive/terminal/signature/entrypoint/
  decision-denominator/site-owner/behavior
  recomputation; wrapper marker or stored JSON fields alone are not verification

### Rollback
List ordered source/attestation pairs. Revert the smallest causal source commit
in reverse dependency order with `git revert <literal-source-sha>`; do not
revert or delete an attestation. Append a docs-only supersession record naming
the invalid source/attestation pair, invalidate descendants, and regenerate
from a new fixed SHA and new run ID. Preserve all audit/data/attestation,
local candidate-bundle/terminal, persistent-journal and portable archive/
terminal/signature evidence. Append signed archive/key supersession where
applicable. Never promote an incomplete/quarantined candidate, re-sign old bytes
or reuse its terminal; repeat the full protocol. Preserve the 80%/95% floors.
```

## 10. 2026-08-26 分层门禁与覆盖率棘轮修订（实施权威）

### 10.1 目标与事实基线

本修订解决的问题不是降低交易系统的安全标准，而是把检查放到能够提供相应证据的阶段：

- 普通 PR 证明“本次改动安全、相关测试充分、没有扩大历史覆盖债”；
- 发布候选证明“整个仓库达到发布目标，并且部署环境中的真实数据足够新鲜”；
- 真实数据、测试/实盘隔离、订单安全、审计和禁止假实现继续是硬红线，任何阶段都不得豁免。

2026-08-26 在 `f05f506` 工作树生成的新鲜报告：

```bash
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
```

结果：第一条命令 exit 0，所有测试通过；第二条命令 exit 1：

```text
global line coverage: 201279/258810 = 77.77% (required 80.00%)
core line coverage: 157652/202935 = 77.69% (required 95.00%, 218 files)
```

若继续把固定阈值用于每个 PR，当前分支需替全仓补约 5,769 条 global 和 35,137 条
core 已覆盖行。这个缺口是历史仓库状态，不是本轮测试失败，不能通过改小分母、排除目录、
降低 80%/95% 或伪造报告消除。

现有 CI 把回填和聚合合规绑定在同一普通 workflow；仓库 workflow 没有自动部署入口。可复验
事实如下（第二条 exit 1 且零输出表示没有命中）：

```bash
rg -n 'backfill_daily|tools/compliance/check\.sh' .github/workflows/compliance.yml
rg -n 'deploy|self-hosted|environment:' .github/workflows
```

```text
22:            cargo run --quiet --bin backfill_daily -- 000001
24:        run: bash tools/compliance/check.sh
```

### 10.2 方案比较与选择

| 方案 | 结果 | 决策 |
| --- | --- | --- |
| 删除覆盖率、freshness 和 compliance 门禁 | 合并快，但真实数据、订单与审计回归失去阻断 | 拒绝 |
| 所有 PR 继续执行全仓 80%/核心 95% 和真实库 freshness | 安全目标清楚，但每个聚焦改动都承担历史覆盖债，并依赖普通 CI 不拥有的生产数据库 | 拒绝 |
| PR 增量覆盖 + 全仓棘轮；发布保留固定阈值 + 真实 freshness/live evidence | 本次改动仍被严格验证，历史债不能恶化，生产发布不降级 | 采用 |

采用方案的核心是深化既有两个模块，而不是新增平行平台：

1. `tools/coverage/check_thresholds.py` 继续是唯一覆盖率 policy seam，共享报告解析、路径归一化、
   core 范围和错误语义，只增加 `pr` 与 `release` 两种策略。
2. `tools/compliance/check.sh` 继续是唯一合规入口，只增加 `--policy pr|release`；默认保持
   `release`，避免旧调用者无意绕开 freshness。

旧 §2.4–§4 尚未实现的隔离构建、签名归档、行为分母平台不进入本轮实现。它们若未来确有
独立消费者，必须重新 Gate A；不得把本次小型门禁修订扩张成基础设施项目。

### 10.3 Gate A–D 新职责

| Gate | 可证明内容 | 阻断结果 |
| --- | --- | --- |
| A 设计 | 数据流、失败方式、旧模块关系、阈值证据和回滚 | 未审阅不得实施 |
| B 实现 | 代码、显式错误、相关单测/进程测试、测试实盘隔离 | 未通过不得进入 PR 合规 |
| C PR 合并 | fmt、strict Clippy、全工作区测试、离线 compliance、改动行覆盖率、全仓覆盖率棘轮、PR 证据 | 未通过不得合并 |
| D 发布 | Gate C 全部证据、global ≥80%、core ≥95%、完整 compliance（含真实 freshness）、live-data/审计证据、独立复核 | 未通过不得发布或部署 |

“合并完成”和“发布就绪”必须分别表述。Gate C 通过只能声明 merge-ready；没有 Gate D
证据时必须明确 `Release Blocked`，不能说“全部完成”或“生产可用”。仓库当前没有自动部署
workflow，因此 Gate C 合并不会自动绕过 Gate D。

### 10.4 PR 覆盖率策略

`check_thresholds.py --policy pr` 的接口必须一次完成两项判定：

1. **核心改动可执行行覆盖率 ≥90%，其他生产改动可执行行覆盖率 ≥85%。** 范围仅为相对
   base ref 新增或修改的 `src/**/*.rs` 可执行行；核心范围复用 BR-250 的 `CORE_PREFIXES`，其余
   `src/**/*.rs` 进入非核心分母。删除行、注释、空行和 llvm-cov 不计入分母的行不计。某一桶
   没有可执行生产行时，输出该桶 `N/A (0 executable changed lines)` 并通过此子项，不得伪报
   100%。两个桶分别判定，禁止用高覆盖的非核心改动稀释核心改动。
2. **全仓覆盖率棘轮。** 当前报告的 global/core 比例都不得低于已审计 baseline；比较使用
   整数交叉相乘，不用浮点舍入决定成败。candidate baseline 不得低于 base 分支 baseline。

初始 baseline 固定为已跟踪的 `config/design_contracts.toml` 中 `[coverage]` 表，至少包含：
schema、global/core 的 covered/count、core 文件数、source SHA、rustc commit/LLVM version、
cargo-llvm-cov version，以及 PR 核心/非核心改动阈值。不得为此强制跟踪被 `.gitignore` 排除的
`tools/`、`tests/` 或其他新文件。本次是该表首次引入，必须在 PR 中明确
`Bootstrap-Baseline: true`，并由同一 source SHA 的
新鲜报告证明 baseline 不高于实际结果。后续 PR 修改 baseline 时，检查器必须同时读取 base
分支版本并拒绝任何比例下降；提高 baseline 是允许的，降低必须 exit 1，不能走普通配置修改。

2026-08-27 对当前分支相对 merge-base `c6024e5` 的 LCOV 改动行复算结果为：核心
`13707/14923 = 91.85%`，其他生产代码 `8368/9824 = 85.18%`。因此统一 95% 会继续把既有
历史改动债转嫁给本轮门禁修订；采用 90%/85% 是以当前真实分母为依据的首个可执行阈值，且
均显著高于当前全仓约 77.7% 的覆盖水平。后续由全仓 global/core baseline 棘轮禁止倒退；阈值
本身只能通过新的 Gate A、BR-252 和 `[coverage]` 配置双向更新。

差分行由 `git diff --find-renames --unified=0 <base>...HEAD -- src` 取得；可执行行与命中次数
来自同一次 workspace/all-features 采集后生成的 LCOV `DA:<line>,<count>`，全仓与核心棘轮继续
读取 JSON totals/file summary。PR policy 因而同时要求 `--report` 与 `--lcov`，两份文件缺失或
源码文件集合不一致必须失败。base ref 缺失、不是当前仓库对象、diff/LCOV 解析失败、coverage
文件缺失、路径逃逸、工具身份不一致、baseline schema 未知或 core 分母为空均 exit 2。任何
失败都不能降级为 N/A。

为避免浮动工具制造假回退，coverage workflow 固定 Rust 1.95.0（含
`llvm-tools-preview`）和 cargo-llvm-cov 0.8.7；baseline 同时记录 `rustc -Vv` 与
`cargo llvm-cov --version` 的身份。以后升级工具链必须在独立 PR 中，对同一 source SHA 用旧、
新工具各生成一次报告，解释分母变化并建立不低于新报告的 successor baseline。

CLI 退出语义固定为：0=策略通过，1=覆盖政策未达标，2=输入/工具/报告不可验证。现有不带
`--policy` 的调用保持 `release` 语义，以免旧脚本从固定 80%/95% 静默降级到 PR 策略。

### 10.5 Release 覆盖率策略

`check_thresholds.py --policy release` 保留现有完整报告判定：

- global line coverage ≥80%；
- core trading/data paths line coverage ≥95%；
- core 范围、worktree 路径归一化和“零 core 行失败”继续由 BR-250 约束；
- 不允许用 patch report、focused test、`--ignore-run-fail`、旧报告或 candidate baseline
  代替新鲜 workspace/all-features 报告。

80%/95% 因此没有被删除或降低，只是不再把尚未达到的发布目标冒充为每个 PR 的改动质量。

### 10.6 Compliance 分层

`tools/compliance/check.sh --policy pr` 运行所有无需生产环境的检查：fake implementation、
design contradiction、business rules、backfill failure propagation、silent fallback、legacy caller、
BR-194 dependency 等。它不运行 `check_data_freshness.sh`，但全工作区测试仍必须运行
`tests/test_data_freshness_check.rs`，证明 fresh/stale/missing 数据的失败语义没有被削弱。

`tools/compliance/check.sh --policy release` 是默认值，运行上述全部检查并追加真实
`check_data_freshness.sh`。Release 调用必须显式绑定部署环境真实 `STOCK_DB`；缺库、过期、
连接失败均阻断。检查器不得自动回填后立刻给自己签发 PASS：若 freshness 失败，由获授权操作者
单独执行 `bash tools/one_shot/backfill_daily.sh`，保留来源/时间/结果审计，再从头重跑 release。

现有 `.github/workflows/compliance.yml` 中“先对 000001 回填一个 CI 临时库，再把它当作
freshness 证据”的步骤删除。GitHub-hosted PR runner 没有生产数据库，它只能给出 Gate C 离线
证据，不能签发 Gate D freshness PASS。

### 10.7 CI 数据流

```text
PR / main push
  ├─ fmt + strict clippy + workspace tests
  ├─ compliance --policy pr
  └─ llvm-cov(head)
       ├─ core patch coverage >=90%
       ├─ other production patch coverage >=85%
       └─ global/core >= tracked ratchet baseline
             └─ Gate C merge-ready

受控发布主机（显式人工触发）
  ├─ 重放 Gate C 证据
  ├─ coverage --policy release (80% / 95%)
  ├─ compliance --policy release (含真实 STOCK_DB freshness)
  ├─ live-data / audit / release binary 证据
  └─ 独立复核通过 → Gate D release-ready
```

### 10.8 失败方式

- PR 核心 patch <90% 或其他生产 patch <85%：只补本次改动的行为/失败路径测试，不修改
  baseline 或排除行。
- global/core 低于 baseline：定位回退文件；补测试或恢复造成回退的改动。禁止降低 baseline。
- 工具身份漂移：exit 2；按 §10.4 的双报告升级流程处理。
- PR freshness 未运行：这是预期的 Gate C 状态，不得展示为 freshness PASS。
- Release freshness 失败：Gate D 阻断；没有回填授权时只报告 blocker，不碰生产库。
- Release 固定覆盖率失败：Gate D 阻断，但不反向宣称已通过 Gate C 的代码存在测试失败。
- baseline 或 diff 被篡改/缺失：exit 2，不能按零改动通过。

### 10.9 旧模块处置

| 模块 | 采用/拒绝 | 原因 |
| --- | --- | --- |
| `tools/coverage/check_thresholds.py` | 采用并深化 | 保留唯一报告解析、核心范围与路径规范化 seam |
| `tests/test_coverage_thresholds.rs` | 采用并扩展 | 继续通过真实 CLI 进程接口验证成功与失败，不测试内部实现 |
| `tools/compliance/check.sh` | 采用并深化 | 单一入口、显式 policy；默认 release 保持 fail-closed |
| `check_data_freshness.sh` | 原样采用 | freshness 语义和阈值不变，只调整执行阶段 |
| `.github/workflows/coverage.yml` | 修改调用 | PR 使用 patch + ratchet；不再要求固定发布目标 |
| `.github/workflows/compliance.yml` | 修改调用 | 删除自动回填，改跑离线 PR policy |
| §0–§9 未实现的覆盖率权威平台 | 拒绝进入本轮 | 接口和基础设施远超当前痛点，没有已存在消费者 |

### 10.10 测试与验收

Gate B 至少新增以下进程级测试：

- 核心 patch 90% 与其他生产 patch 85% 边界通过，各自低一条可执行行失败；
- 注释/空行/删除-only 输出 N/A，不伪报 100%；
- rename 与 worktree 路径正确归一化；
- 当前报告等于 baseline 通过，任一 global/core 比例下降失败；
- candidate baseline 低于 base baseline 失败；首次 bootstrap 缺显式标记失败；
- 缺 base ref、坏 diff、未知 schema、工具身份漂移均 exit 2；
- `--policy release` 仍以 80%/95% 判定，默认无 policy 仍等价于 release；
- compliance PR policy 不执行真实 freshness，但继续执行其余全部脚本；
- compliance release/default 必须执行 freshness，freshness 失败向上传播；
- workflow 静态测试证明 PR 不回填真实代码、不读取生产库、不宣称 Gate D PASS。

实现完成后的验证命令：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh --policy pr
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
cargo llvm-cov report --lcov --output-path target/coverage/lcov.info
python3 tools/coverage/check_thresholds.py --policy pr \
  --report target/coverage/coverage.json --lcov target/coverage/lcov.info \
  --base-ref <merge-base> --bootstrap-baseline
git diff --check
```

完整 `bash tools/compliance/check.sh --policy release`、固定 80%/95%、真实 provider/数据库、
推送和订单验证仍需单独生产授权；未授权或未通过时状态必须写为 `Release Blocked`。

### 10.11 回滚与 PR 证据

实现按“规则文档 → coverage policy → compliance policy → workflow”拆成可独立 revert 的提交。
代码故障回滚最小实现提交；职责划分错误返回 Gate A。回滚不得删除 coverage 报告、审计、持仓、
成交或行情证据，不得恢复 CI 自动回填并冒充生产 freshness。

PR 除 AGENTS §3.1 字段外必须增加：

- `Gate-Policy: PR=core-patch90+other-patch85+ratchet; Release=global80+core95+freshness+live`
- `Bootstrap-Baseline: true|false`
- baseline 的 source SHA、global/core covered/count、工具身份；
- PR Gate C 与 Release Gate D 分开列结果，禁止合并成一个“全部 PASS”。

本修订登记为 BR-252；BR-250 的路径归一化合同继续有效。BR-202 保留为历史候选记录，但其
未实现的固定 PR 阈值与重型权威平台不再是本轮实施依据。

### 10.12 独立审查后的证据闭合（2026-08-27）

独立审查发现原实现仍可能把不可验证输入签成 PASS，因此本节补充并覆盖 §10.4–§10.11 中
较宽松的表述：

1. **源码完整性。** JSON 与 LCOV 的生产源码集合必须完全一致。每个相对 base 新增、修改或
   rename 后仍存在的 `src/**/*.rs` 必须出现在该集合；唯一例外是
   `config/design_contracts.toml [coverage.reviewed_no_region]` 中已审计且当前文件 SHA-256
   精确匹配的路径。首次 bootstrap 可登记现存集合；后续 candidate 不得新增豁免路径。
   缺文件、路径逃逸、rename 丢失、坏 diff 或 hash 漂移均 exit 2，不能输出 N/A。
2. **报告来源。** `[coverage].source_sha` 必须是 HEAD 祖先，且该提交至 HEAD 的 `src/`、
   `Cargo.toml`、`Cargo.lock`、`build.rs` 不得变化；报告必须带匹配的 llvm coverage schema、
   cargo-llvm-cov 版本及当前仓库 `Cargo.toml` 路径。工具身份、JSON/LCOV 集合和 core 文件数
   必须与合同一致。Release 只有完整 provenance 可验证时才可能 PASS；未达固定阈值可直接
   exit 1，但不得用缺 provenance 的高覆盖报告签发 PASS。
3. **Bootstrap。** 初始合同除 CLI `--bootstrap-baseline` 外，必须同时登记
   `bootstrap_approved = true` 与 `bootstrap_rule = "BR-252"`；90%/85% 和 80%/95% 是硬下限，
   任一更低均 exit 2。baseline covered/count 必须不高于同源码新鲜报告。CI 可自动识别“base
   尚无合同”，但不能替代仓库内的显式批准记录。
4. **Release 数据库。** `check.sh --policy release` 必须在运行任何检查前要求显式绝对
   `STOCK_DB`，解析后的真实路径必须等于当前部署 checkout 固定的
   `data/stock_analysis.db`；缺失、别名、测试库或其他路径均 exit 2。Release 同时拒绝
   `FRESHNESS_TODAY` 与 `TRADING_CALENDAR` 覆盖，防止 fixture 或回拨时钟签发生产 freshness。
   直接执行 `check_data_freshness.sh` 的覆盖变量只用于隔离测试，不能穿过 release seam。
5. **PR 证据。** `check_pr_evidence.sh` 除 AGENTS §3.1 字段外，必须校验 §10.11 的精确
   Gate-Policy、Bootstrap-Baseline、baseline source/count/tool identity、`Gate-C: PASS` 与独立
   `Gate-D: PASS|Release Blocked`。Hosted CI checkout 必须 `fetch-depth: 0`，否则提交范围证据
   不可验证。

以上任一输入不可验证都回到 Gate B；Release DB 身份或生产 freshness 不具备时保持
`Release Blocked`，不得以关闭门禁解决。
