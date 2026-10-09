# Task 1 report: trustworthy signal scorecards and registry v0

## Scope and implementation

Branch: `codex/goal-first-delivery-20261009`. Worktree: `/Users/zhangzhen/.codex/worktrees/goal-first-delivery/stock_analysis`.

Added a descriptive registry for NewsCatalyst, MainNetInflow, VolumeSurge, PostCloseFundInflow, StreakLeader and ThemePrediction. Versions ending in `review-v0` name explicit review assumptions; they are not claims about deployed strategy releases. `lot-rates-v1` names the existing paper scenario cost model. All default actions are `observe` and statuses `evidence_pending`. The CLI alone imports this registry; no trading, selection, Gateway, activation, DB schema or delivery owner imports it.

Extended the existing weekly report without removing its legacy fields, requested periods, Shanghai clock, verified trading-day maturity, original rows, open cycles, bounded denominators, or reader failures. Each family has separate price-observation, simulated-fill and net-return sections. Every family is currently explicitly unavailable: the original readers do not supply an authoritative family plus signal-version join. In particular, a prediction code/direction, `virtual_reason`, recorded outcome, candidate or archive row does not establish family attribution or physical delivery. Null reasons identify the missing original typed join; paper performance also needs exit/cost-version lineage.

Pooled evidence is separate from the family cards. It reuses the current predictions/classify reader and existing effective paper capability. Price observations remain observational even after snapshot revalidation because legacy rows do not establish historical availability/PIT. Simulated ledger counts can be reliable only for the existing effective simulated-ledger boundary when there is no legacy-without-terminal lineage; legacy counts are observational. Such a grade never qualifies prices, brokerage execution or a strategy sample. Modeled costs/net PnL remain observational. Executable net return stays null/unavailable. Readable empty sources produce usable zero counts; absent/failed sources produce null. No closed/revalidated sample produces null returns/rates, with a reason.

The economic owner's typed `NetSummary::Unavailable` also blocks aggregate weekly scenario amounts, including an unresolved open/historical dispute that is outside the weekly closed subset. Dependent lifecycle null amounts/reasons remain intact; unrelated per-cycle amounts retain their existing owner-approved values. No original fill is removed, repriced or rewritten. Added a 100,000-paper-row preflight before the existing effective read so a failed raw denominator cannot silently permit an unbounded paper scorecard.

## Exact CLI and output contract

All existing required flags and default Markdown behavior remain. New optional flags:

- `--registry PATH`: read/validate an explicit descriptive TOML registry. Maximum 128 KiB. Default: compile-time embedded `config/signal_registry.toml`; source label is `embedded:config/signal_registry.toml`. Default resolution never depends on current working directory, runtime installation layout, HOME or launchd cwd. Editing the file requires a rebuilt CLI or explicit `--registry` to affect reports.
- `--evidence-manifest PATH`: create a private sidecar, or reuse an existing byte-identical sidecar for the second report format. If omitted with `--output`, use `output.with_extension("evidence.json")` (e.g. `review.json` and `review.md` both select `review.evidence.json`). An incompatible existing manifest fails before writing the report. Report/manifest path collision fails. Existing report files are never overwritten.

Explicit detached-fixture example (not a command executed against production):

```sh
weekly_outcome_review --database /tmp/detached-snapshot.db \
  --from 2026-09-28 --to 2026-10-04 \
  --observed-at 2026-10-08T00:52:00+08:00 \
  --registry /absolute/path/signal_registry.toml \
  --format json --output /tmp/review.json \
  --evidence-manifest /tmp/evidence-manifest.json
```

`weekly-outcome-review.py` accepts the same new flags in single-format mode. Weekly mode accepts `--registry`, computes its existing Monday-Sunday Shanghai scope, and fixes one clock. It normalizes one WAL-correct read-only SQLite backup, optionally freezes one registry file into the same private temporary directory, and supplies both to the JSON/Markdown children. The embedded default is the same binary's embedded registry. Weekly versions contain:

- `review.json`
- `review.md`
- `evidence-manifest.json`
- `run-status.json`

The manifest is produced by the CLI from the detached reader; the wrapper does not reopen production to construct it. The JSON and Markdown each embed the same manifest. Private directories remain 0700, artifacts 0600, input backup/explicit registry snapshot 0400. Successful weekly children must produce the manifest. Completed artifacts remain listed in order (normally JSON, manifest, Markdown); partial failures retain completed files and the child exit status. No output/stdout-only invocation invents a manifest destination; the embedded manifest is still emitted and an explicit sidecar is supported.

The CLI now explicitly rejects nonempty input `-wal` files. A live WAL source must enter through the existing backup wrapper. This ensures the SHA covers the actual normalized input, not only a main file that omits committed WAL rows. Before/after main hashes still guard source mutation; the detached read-only API checks its own snapshot/source identity. No global `DatabaseManager::init` is called.

## Registry TOML schema

`schema_version`: integer, exactly 1. `registry_version`: nonempty string (`signal-registry-v0` default). `signals`: six records, exactly the six supported family names. All fields are required; unknown fields fail.

Each signal record:

| Field | Contract |
| --- | --- |
| `id` | nonempty unique lowercase ASCII/digit/underscore string |
| `name` | unique enum: NewsCatalyst, MainNetInflow, VolumeSurge, PostCloseFundInflow, StreakLeader, ThemePrediction |
| `signal_version`, `exit_version`, `cost_version` | nonempty strings; descriptive only |
| `action` | observe, maintain, pause, review |
| `status` | evidence_pending, observational, qualified, paused |
| `entry_assumptions`, `eligibility` | nonempty descriptions |
| `windows` | nonempty unique integer list drawn from existing supported trading windows 1, 3, 5 |

Duplicate IDs/names, missing families/versions, unknown enums/schema/fields, invalid/duplicate/empty windows and invalid IDs fail explicitly. Manual action/status never changes reader metrics or their grades, even if status says `qualified`.

## Read-only assistant artifact schema

Legacy top-level `report_version` remains `H16-descriptive-weekly-v1`. New top-level fields `scorecard` and `evidence_manifest` are always populated by a successful CLI run. They are optional internally only so existing reader-only tests can call `report::read` without pretending they know the input hash.

`scorecard.schema_version` is the string **`weekly-signal-scorecard-v1`**. Fields:

- `registry`: `{source: string, sha256: lowercase hex SHA-256 string, content: Registry, authority: string}`. Full parsed registry content is retained, even when an explicit temporary registry path is deleted after the wrapper finishes.
- `pooled_descriptive_evidence`: `Sections`, separate from attributable performance.
- `families`: six `{signal_id: string, sections: Sections}` records. Join `signal_id` to `registry.content.signals[].id`; family names, versions, assumptions, windows, action/status come from that exact content.

`Sections` has three arrays: `price_observation`, `simulated_fill`, `net_return`. Each `Metric` is:

```json
{
  "id": "t1_revalidated_samples",
  "grade": "observational",
  "value": 0,
  "reason": null,
  "evidence": {
    "reader_id": "weekly_outcome_review::report::predictions/classify@H16-scorecard-v1",
    "input_snapshot_sha256": "64 lowercase hex characters",
    "sample_scope": "explicit reader/sample scope",
    "exclusions": "explicit exclusions",
    "meaning": "explicit interpretation"
  }
}
```

Grades are lowercase `reliable`, `observational`, `unavailable`. Value is a JSON number or null. A null value always has grade `unavailable` and a specific nonempty reason. A usable count can equal zero; that does not establish a return/rate denominator. `reliable` qualifies only the reader boundary expressed in evidence, never a whole card/strategy. No family uses a reliable metric in v1. Consumers should inspect IDs and sections rather than positions; a failed reader uses an unavailable reader-level metric rather than fabricating an array of zero-valued horizon amounts.

Pooled IDs on successful readers:

- For each T+1/3/5: `tN_revalidated_samples`, `tN_hit_rate`, `tN_mean_change_pct`. Scope is horizons maturing in this completed requested week. Rates are fractions, changes percentages.
- `period_fill_rows`, `closed_cycles_in_week`, `open_cycles_at_period_end`. Open cycles are right censored, not losses or closed-return samples.
- `closed_cycle_scenario_cost_cny`, `closed_cycle_scenario_net_pnl_cny`, `executable_net_return`. First two are modeled CNY amounts. Last is always unavailable, not an executable return percentage.
- Family section placeholder IDs are `price_observation`, `simulated_fill`, `net_return`; null missing-join evidence excludes all pooled rows from attribution.

`evidence_manifest.schema_version` is **`weekly-outcome-evidence-manifest-v1`**; `artifact_schema` is `weekly-signal-scorecard-v1`. Fields:

- `input_snapshot_sha256`: exact SHA-256 of normalized input main-file bytes, identical to `input_source.source_main_sha256` and every inline/scoped evidence SHA. It is snapshot identity, not market-data qualification or a live WAL digest.
- `period`: exact serialized report Period, including requested_from/to, observed_at, latest_completed_session, period_completed_through and completed_sessions. Clock is fixed Shanghai +08:00. Consumers must preserve completed-session and maturity boundaries rather than treating requested_to as a trading session.
- `registry`: exact same object as scorecard.registry. `sha256` is exact original loaded/frozen registry UTF-8 bytes, including comments/whitespace; `content` is the parsed representation.
- `paper_binding`: original optional paper-ledger binding environment string read by the existing effective reader. Missing binding preserves existing LegacyRaw/AsKnown behavior; supplied binding preserves existing Epoch/RestatedLatest behavior. Malformed bindings fail the paper section, not the whole diagnostic report.
- `reader_source_sha256`: SHA-256 of the UTF-8 contents of `reader_source_inputs` in listed order, joined by one newline. This source/lock fingerprint identifies the named review readers and central effective/dispute owner sources; it is not advertised as a complete executable/build/whole-repository fingerprint. Reproduction also requires the repository dependencies and existing calendar/schema/runtime binding contracts.
- `reader_source_inputs`: exact ordered relative source paths used by that fingerprint, compiled into the CLI.
- `metric_scopes`: map from JSON-pointer scope to `Evidence`. Longest matching scope wins; `*` represents a single array index. A scope applies to all descendant legacy metrics; inline scorecard metric evidence overrides it. Base scopes cover daily_bars, independent_daily_status, predictions, raw_paper, original_order_attempts, verified_paper, physical_delivery and scorecard. More specific monetary scopes distinguish model amounts from simulated-fill counts, including `exits/*`.
- `boundary`: explicit scope inheritance and authority description.

Legacy whole-snapshot diagnostics intentionally retain future source extents and raw Filled/latest/malformed/future-row evidence. Their scope explicitly says they are not as-of metrics. Weekly/pool metrics exclude future origins/evidence/fills and use completed sessions. Do not convert the diagnostic count into performance or physical delivery. Legacy nullable paper amounts now also carry `scenario_amount_unavailable_reason` on the aggregate and per-exit objects.

## Validation and self-review

`cargo test --bin weekly_outcome_review`: **21 passed, 0 failed**, elapsed initial compilation 5m44s and fixture tests 0.26s. Existing library compilation emitted 887 warnings in unchanged library sources; there were no errors or new weekly-review warnings. `git diff --check`: passed before staging. Python command `python3 -m unittest scripts.tests.test_weekly_outcome_snapshot`: 10 passed. It covers WAL backup committed-row preservation, source inode/catalog/main/WAL bytes, private modes, fixed scope/clock/backup, sidecar retention, fixed explicit registry despite original changes, missing source/registry and child failures.

Rust scope was exactly `cargo test --bin weekly_outcome_review`, preserving existing fixture regressions and adding invalid-registry, duplicate/inferred/missing qualification, family join, usable-zero-vs-null, future evidence/origins, typed disputed-summary suppression, exact JSON/Markdown/manifest snapshot-clock-registry identity, manifest mismatch, read-only source preservation and nonempty-WAL rejection checks. Dispute suppression is tested at the existing owner's typed unavailable-summary boundary; it does not fabricate a production disputed row or weaken owner schema/authority. Existing original-price dispute owner code is unchanged.

Self-review: no actual strategy/selection paths, Gateway qualification, immutable paper facts, T+1 enforcement, dispute owner, counted delivery or Uncertain semantics changed. No production binary, provider, sink, cancellation, seed, activation, monitor restart, release build, push or external notification was run. Tests execute local Rust functions against temporary fixtures and local wrapper children only. Baseline untracked Python cache remains outside commits. Registry/report are under ignored top-level config/.superpowers directories and will be force-added specifically; no unrelated ignored data is included.

Remaining limitations: family attribution is deliberately unavailable pending authoritative original typed family/version lineage; historical PIT and actual execution/settlement/delivery evidence remain missing. This is an E1 evidence baseline and E4 descriptive prerequisite, not E4 runtime wiring or strategy promotion. Registry changes are manual descriptive metadata. A manifest/JSON/hash cannot certify the accuracy of the underlying provider facts. No live/production validation is claimed.
