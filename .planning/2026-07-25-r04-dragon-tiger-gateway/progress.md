# Progress — R-04 Dragon-Tiger Gateway

## 2026-07-25

- Re-read repository engineering constraints and the diagnosing/planning
  workflows.
- Reproduced the user's symptom at the exact production R-04 caller.
- Ranked and checked three hypotheses:
  - active caller not migrated: confirmed;
  - committed Cargo dependency lacks the new upstream contract: confirmed;
  - strict `--review` bypasses the dispatcher: rejected; it reaches the same
    R-04 dispatcher.
- Registered BR-162 and wrote the focused Gate A design before code changes.
- Added a source-level regression guard at the exact production dispatcher;
  it requires `DragonTigerGateway` and rejects the legacy loader/direct HTTP.
- The first RED run was blocked before the target test by the known adjacent
  dependency mismatch (`SecurityBar` vs Core `Bar`). Temporarily redirected
  the four Magic path dependencies to the approved integration worktree.
- Re-ran the exact guard against unified dependencies: RED as expected because
  production R-04 did not contain `DragonTigerGateway`.
- Added the typed R-04 Gateway, batch evidence validation, exact seat mapping,
  post-group positive-net filtering/sort/limit, and tests proving distinct
  source disclosures are retained without summing their net amounts.
- Gateway unit tests pass: 2 passed, 0 failed.
- Replaced the active R-04 loader with `DragonTigerGateway`, added a typed
  evidence/TRADE_ID/seat renderer, and updated dispatcher tests for wait,
  verified-empty, and retryable unavailable outcomes.
- Production source guard and dispatcher state tests pass: 2 passed, 0 failed.
- Added a renderer regression test for both distinct source TRADE_ID values,
  exact buy/sell seats, highest-disclosure ranking net, and absence of the old
  malformed missing-data phrase.
- First live Gateway probe with an over-broad 100-disclosure request failed
  closed as designed because a lower-ranked source row had only 4 buy seats.
  Narrowed production to the provider's whole-market-discovered top-five
  complete disclosure batch; no completeness rule was relaxed.
- Real 2026-07-22 Gateway probe PASS:
  - 600396: TRADE_ID 100380472 and 100380465, both 10 seats;
  - 603459: TRADE_ID 100380454, 10 seats;
  - 002396: TRADE_ID 100379754 and 100379769, both 10 seats.
  The resulting stock ranking uses 375176977.24, 351439671.98 and
  303776811.63 yuan respectively and retains all five disclosures.
- Deleted the temporary live-probe source after capturing evidence.
- Compliance passed fake-implementation, freshness, design-contradiction,
  backfill propagation and no-silent-fallback checks. The business-rule gate
  identified one R-04-local missing source citation plus two unrelated
  concurrent-slice errors; added the required `BR-162` module citation.
- Re-ran the business-rule check: the BR-162 error is cleared; only unrelated
  BR-161 event-calendar and BR-158 review citation errors remain.
- `cargo clippy --lib -- -D warnings`: PASS.
- `cargo clippy --bin monitor -- -D warnings`: blocked only by unrelated
  `src/bin/monitor/main.rs:5228` useless `format!`.
- `cargo build --bin monitor`: PASS against the approved upstream integration
  worktree.
- Restored all four committed Magic dependency paths to
  `../magic-market-data-rs`; upstream integration must land there before the
  final combined build can use the new R-04 contract.
