# Monitor test CLI contract implementation plan

## Scope

Implement BR-136 from
`docs/superpowers/specs/2026-07-20-monitor-test-cli-contract-design.md` without changing production
review evidence requirements.

## Task 1: Lock the CLI contract with failing tests

- Extend the isolated process test to execute bare `--test` and require the existing E2E completion
  marker, isolated database, positive L7 decision count, and no production database creation.
- Extend the help test to require `--test`, `--review`, and the safety distinction.
- Run the focused test and preserve the RED evidence.

```bash
cargo test --test monitor_help_isolation bare_test_alias -- --exact --nocapture
cargo test --test monitor_help_isolation help_exits_without_creating_runtime_state -- --exact
```

## Task 2: Route the alias and document help

- Separate the explicit `--e2e` request from the effective E2E route.
- Select effective E2E for explicit `--test --e2e` or exactly bare `--test`.
- Keep `--test --review`, `--test --v13-diag`, replay/history, and production `--review` unchanged.
- Expand terminal help without moving it below runtime initialization.

## Task 3: Focused and regression validation

```bash
cargo fmt --all -- --check
cargo test --test monitor_help_isolation -- --test-threads=1
cargo test --workspace --all-targets --all-features -- --test-threads=1
cargo clippy --workspace --all-targets --all-features -- -D warnings
bash tools/compliance/check.sh
cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

Run bare `./target/release/monitor --test` only in a fresh temporary working directory with an
explicit isolated `DATABASE_PATH`, empty stock list, dry-run, and all notification credentials
removed. Require exit 0 and exactly one final E2E completion marker.

## Task 4: PR evidence and rollback

- Complete Refs, Data-Redlines, OldModules, Threshold-Proof, Business-Rules, Validation, and Rollback.
- Obtain independent zero-blocker review before Ready/merge.
- Revert only the merge commit and rebuild for rollback; do not mutate any data file.
