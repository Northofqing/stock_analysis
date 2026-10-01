# M4 v2 run descriptor prerequisite audit

Read-only implementation brief for Task 2 of
`.planning/2026-10-01-m4-core-v2-first-slice.md`, against the branch after
`d8e28544`. No run descriptor or output writer is admitted by this brief.

## What can already be bound

- `AShareFeePolicyV2` exposes canonical bytes and a domain-separated instance
  ID. `ResearchPortfolioV2` owns the policy, initial cash, and the ordered
  `PreparedResearchFillV2` effects actually applied. Each effect retains the
  assumed fill input, exact integer notional, full fee components, cash delta,
  and poststate. Its effects are modeled research outcomes, not exchange fills.
- `BenchmarkReader::read_exact(manifest_hash, request)` verifies an exact
  persisted BR-251 manifest, matching request, and complete benchmark payload.
  This is the correct admission operation outside a future pure descriptor.

## Why a complete descriptor cannot be constructed yet

1. `AdmittedDailyBars` is a non-forgeable, nonempty Gateway batch with target
   code, records, and `BatchEvidence`. It does not carry its requested `days`,
   exact backtest date vector/window, a persisted content manifest, or a
   disposition for requested symbols that were skipped. Its public contract
   does not expose a digest of all bars. `backtest_runner` currently takes the
   records and drops the evidence while allowing short or failed histories to
   be skipped. A string hash of those records would describe bytes but not prove
   complete admitted run inputs.
2. `AdmittedOutcomeDailyBars` has a strict exact-window proof, but it is bound
   to an outcome sample, phase, due date, and its own provider request. It is
   not a general historical-backtest manifest and must not be relabeled as one.
3. `BenchmarkManifestRef` and `BenchmarkSnapshotRef` expose public fields;
   callers can construct or change them without a `BenchmarkReader` read. A
   pure descriptor receiving either value, or a manifest-hash string alone,
   cannot prove the exact persisted request and coverage. The current legacy
   wrapper supplies no benchmark manifest/request and reports unavailability.
4. There is no explicit canonical v2 strategy/config descriptor or verified
   code revision type. `ResearchPortfolioV2` does not expose its required fee
   coverage separately, which matters for a run with zero effects. Hashing
   caller-supplied names or a Git-looking string would make a declared
   identity, not verified code or configuration provenance.

## Minimum next seams

1. At the already admitted historical-bar acquisition boundary, retain a
   sealed per-symbol input containing the exact request, records and evidence,
   canonical finite-value content digest, requested date coverage, and a
   typed included/skipped disposition. Aggregate the full requested universe
   in deterministic symbol order. Keep the current legacy wrapper unchanged.
2. Have an I/O boundary call `BenchmarkReader::read_exact` once and return a
   private-field verified benchmark capability bound to the exact
   `BenchmarkRequest` and manifest. The later pure descriptor consumes that
   capability; it neither queries the store nor trusts a public ref alone.
3. Define explicit canonical strategy/config and reviewed code revision
   identities, and expose the portfolio's required coverage even when it has
   no effects. Then a pure descriptor can reject missing, duplicate, mismatched,
   or incomplete inputs with typed errors. Its domain-separated canonical bytes
   should sort independent bar manifests by instrument while preserving the
   actual fill-effect execution order, policy bytes, and output schema version.
   Different policy or actual effects must produce different run IDs.

Only after these inputs exist should a separate versioned CSV/JSON/report
writer use the run ID for atomic, retry-safe output. No legacy `Trade`, runner,
PaperLedger, or output path is changed here.

Code inspected: `src/strategy/research_fill_v2.rs`,
`src/strategy/research_portfolio_v2.rs`, `src/performance/fee_policy.rs`,
`src/data_gateway/historical_bars.rs`, `src/data_gateway/outcome_daily_bars.rs`,
`src/data_gateway/benchmark.rs`, `src/database/benchmark_segments.rs`, and
`src/pipeline/backtest_runner.rs`.
