# R-04 Dragon-Tiger Gateway Migration

## Goal

Make the active production `monitor --review` R-04 path use the typed
Magic/Eastmoney whole-market dragon-tiger Gateway, aggregate by stock without
losing distinct source disclosures, and prevent the legacy loader from
returning.

## Scope constraints

- Do not edit `src/data_gateway/review.rs` or database audit files.
- Do not modify the upstream R-04 worktree unless a necessary contract bug is
  proven.
- Preserve all unrelated dirty worktree changes.
- Do not commit or push.

## Phases

1. **Gate A and deterministic repro** — complete
   - Register BR-162 before implementation.
   - Write focused design.
   - Capture the production old-loader source guard as the red signal.
2. **Gateway TDD** — complete
   - Add typed aggregation/evidence tests.
   - Implement `src/data_gateway/dragon_tiger.rs` and export it.
3. **R-04 production migration** — complete
   - Replace the active loader with `DragonTigerGateway`.
   - Render stocks with all source disclosures and exact seats.
   - Preserve wait/verified-empty/unavailable task states.
4. **Validation** — complete
   - Focused tests, fmt, clippy, build, compliance.
   - Compile/probe against the upstream integration worktree.
   - Restore committed Cargo dependency paths and record upstream merge
     dependency explicitly.

## Errors encountered

| Error | Attempt | Resolution |
| --- | --- | --- |
| Production R-04 still imports `fetch_recent_lhb` | 1 | Root cause; migration in progress |
| Adjacent `../magic-market-data-rs` main lacks `MarketDragonTigerRequest` | 1 | Compile temporarily against the approved upstream integration worktree; upstream integration remains an external dependency |
| BR-161 allocated concurrently to R-08 | 1 | Registered this slice as BR-162 |
| Full compliance remains red after the R-04 citation fix | 1 | R-04 error cleared; remaining BR-161/BR-158 errors belong to concurrent slices and are reported to the parent |
