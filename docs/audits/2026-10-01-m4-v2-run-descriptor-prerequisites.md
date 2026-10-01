# M4 v2 run descriptor prerequisite audit

Read-only implementation brief for Task 2 of
`.planning/2026-10-01-m4-core-v2-first-slice.md`, initially against the branch
after `d8e28544`. Updated after `063aedaa`, which added the exact persisted
benchmark read capability. No run descriptor or output writer is admitted by
this brief.

## What can already be bound

- `AShareFeePolicyV2` exposes canonical bytes and a domain-separated instance
  ID. `ResearchPortfolioV2` owns the policy, initial cash, and the ordered
  `PreparedResearchFillV2` effects actually applied. Each effect retains the
  assumed fill input, exact integer notional, full fee components, cash delta,
  and poststate. Its effects are modeled research outcomes, not exchange fills.
- `BenchmarkReader::read_verified_exact(manifest_hash, request)` now verifies
  an exact persisted BR-251 manifest, matching request, and complete benchmark
  payload, then returns a private-field `VerifiedBenchmarkSnapshot`. Its public
  snapshot getter does not let callers forge that capability. A future v2
  entrypoint can consume the capability, but none does yet.

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
3. `BenchmarkManifestRef` and `BenchmarkSnapshotRef` still expose public fields;
   a pure descriptor receiving either one, or a manifest-hash string alone,
   cannot prove the exact persisted request and coverage. The new
   `VerifiedBenchmarkSnapshot` closes that construction seam. A v2 entrypoint
   must require it explicitly; the current legacy wrapper supplies no
   benchmark manifest/request and reports unavailability.
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
2. **Implemented as a local prerequisite:** the I/O boundary can call
   `BenchmarkReader::read_verified_exact` to obtain the private-field
   capability bound to the exact `BenchmarkRequest` and manifest. The later
   pure descriptor must consume that capability; it must neither query the
   store nor trust a public ref alone. No production run currently supplies it.
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

## Subsequent local observation-identity slice

`strategy::research_run_descriptor_v2` now has a pure, domain-separated
`m4-observed-run-v2` descriptor for an already executed `ResearchPortfolioV2`.
It requires canonical strategy config, explicit strategy/version and declared
full Git SHA, complete requested instrument membership with one sealed
`ObservedDailyBarsCapture` per instrument, and a
`VerifiedBenchmarkSnapshot` whose exact daily request matches the run window.
The canonical bytes retain the portfolio's fee policy, required coverage,
ordered applied effects, initial/final cash and holdings. Missing or ambiguous
observations and mismatched benchmark requests fail closed.

The descriptor's `input_assurance` explicitly says the historical bars are
observed only. Its ID is not a qualified backtest run ID: the Gateway capture
still lacks an exact persisted window, PIT membership, source-qualified
instrument class, and a verified association between the caller-declared Git
SHA and the executing binary. No report writer or legacy/live wrapper consumes
this descriptor. The historical-bars exact read gate above remains open.

Code inspected: `src/strategy/research_fill_v2.rs`,
`src/strategy/research_portfolio_v2.rs`, `src/performance/fee_policy.rs`,
`src/data_gateway/historical_bars.rs`, `src/data_gateway/outcome_daily_bars.rs`,
`src/data_gateway/benchmark.rs`, `src/database/benchmark_segments.rs`, and
`src/pipeline/backtest_runner.rs`.
