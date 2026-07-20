# Monitor test CLI contract design

## 1. Problem and intent

`monitor --test --e2e` is the existing isolated end-to-end harness, but the documented operator
command `monitor --test` falls through to production-strict review. On a fresh isolated database it
therefore exits for the expected missing real-account evidence before exercising the E2E harness.
The result is a misleading CLI: the safe test command cannot test, while `--help` omits test and
review modes entirely.

This change makes only the argument routing contract explicit. It does not relax production review,
create account values, change a notification threshold, or add a second test implementation.

## 2. Selected interface and data flow

```text
monitor --test
  -> set STOCK_ENV_MODE=test and V10_DRY_RUN_PUSH=1 before initialization
  -> isolated database path / TEST_CODE_ fixtures
  -> existing e2e_all_templates_run
  -> existing render -> governor -> dry-run sink -> L7/audit path
  -> exit 0 only after the existing final completion marker

monitor --test --e2e -> identical route
monitor --test --review -> existing StrictDispatchers route, fail closed without live evidence
monitor --review -> existing production StrictDispatchers route, real current evidence only
```

The alias applies only when `--test` is the sole user flag. Explicit terminal modes keep their
existing precedence and semantics. `--e2e` without `--test` remains a BR-051 error.

## 3. Failure modes

- E2E seed, renderer, governor, database, or audit failure propagates and exits nonzero.
- A test database resolving to the production default remains rejected before it is opened.
- Test mode always forces notification dry-run; no credential or `--send-real` shortcut is added.
- Production or explicit test review never consumes the E2E ledger seed and never fills missing
  cash, account, net-value, or risk fields.
- Help returns before database, source, event writer, or sink initialization and creates no state.

## 4. Existing modules

| Module | Decision | Reason |
| --- | --- | --- |
| `e2e_all_templates_run` | adopt | Existing isolated E2E orchestration and completion contract. |
| BR-051 startup isolation | adopt | Establishes test environment and dry-run before initialization. |
| `run_review_only` / `StrictDispatchers` | adopt unchanged | Production review must remain fail closed. |
| legacy plain-test review fallthrough | reject | It prevents the documented test command from reaching E2E. |
| second test fixture runner | reject | Would duplicate seed, routing, and audit behavior. |

## 5. Observability and help

Help names normal monitoring, isolated test, production review, isolated strict-review verification,
replay, and history. It states that `--test` never sends real notifications and that production
`--review` can fail when current complete account evidence is unavailable.

## 6. Validation

The process test runs both `--test` and `--test --e2e` in fresh temporary directories with all
notification credentials removed. Both must finish successfully, emit the existing completion
marker, persist isolated L7 decisions, and leave the production database path absent. Existing
`--test --review` tests must continue exiting 2 for missing live evidence.

Release gates are the repository-required formatting, strict Clippy, full tests, compliance,
workspace coverage thresholds, release build, and an isolated plain-`--test` smoke run.

## 7. Rollback

Revert the merge commit and rebuild `target/release/monitor`. No database migration or data rollback
is required. Never delete test, production, audit, holdings, or account files during rollback.
