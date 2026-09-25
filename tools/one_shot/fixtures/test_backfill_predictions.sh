#!/usr/bin/env bash
# Exercise the real wrapper with command-boundary stubs: no Cargo build or DB access.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WRAPPER="$SCRIPT_DIR/../backfill_predictions.sh"

# An existing text fixture satisfies -f. Neither stub opens or changes it.
TASK2_TEST_DB="${BASH_SOURCE[0]}"
export TASK2_TEST_DB
cargo() {
  [[ "${STOCK_DB:-}" == "$TASK2_TEST_DB" ]] || return 91
  printf 'TEST_CARGO_ARGS: %s\n' "$*"
  return "${TASK2_CARGO_RC:-0}"
}
sqlite3() {
  [[ "$1" == "$TASK2_TEST_DB" ]] || return 92
  printf 'TEST_SQLITE_CALLED: %s\n' "$2"
}
date() { printf '2026-09-26\n'; }
export -f cargo sqlite3 date

run_wrapper() {
  if OUTPUT=$(TASK2_CARGO_RC="$1" STOCK_DB="$TASK2_TEST_DB" \
      bash "$WRAPPER" "${@:2}" 2>&1); then
    STATUS=0
  else
    STATUS=$?
  fi
}
fail() {
  printf 'FAIL: %s\n%s\n' "$1" "$OUTPUT" >&2
  exit 1
}

run_wrapper 23 14
[[ "$STATUS" -eq 23 ]] || fail "program exit 23 became wrapper exit $STATUS"
[[ "$OUTPUT" != *"回填完成"* ]] || fail "failure printed completion"
[[ "$OUTPUT" != *"TEST_SQLITE_CALLED"* ]] || fail "failure ran success summary"
printf 'PASS: program failure preserves exit 23; no completion or SQLite summary\n'

run_wrapper 0
[[ "$STATUS" -eq 0 ]] || fail "successful program did not succeed"
[[ "$OUTPUT" == *"所有 target_date <= as_of=2026-09-26 且 hit IS NULL"* ]] || fail "missing all-due pending scope"
[[ "$OUTPUT" != *"近 14 天"* ]] || fail "success still advertises a lookback window"
[[ "$OUTPUT" == *"回填完成"* && "$OUTPUT" == *"TEST_SQLITE_CALLED"* ]] || fail "success did not produce a summary"
[[ "$OUTPUT" == *"target_date <= '2026-09-26' AND hit IS NULL"* ]] || fail "summary omits all due remaining rows"
[[ "$OUTPUT" == *"target_date <= '2026-09-26' AND hit IS NOT NULL"* ]] || fail "summary omits previously due verified rows"
[[ "$OUTPUT" != *"pred_date >="* ]] || fail "summary is still limited by creation-date lookback"
[[ "$OUTPUT" == *"非本轮新增"* ]] || fail "cumulative summary could masquerade as this run's writes"
[[ "$OUTPUT" != *"-- 14"* ]] || fail "no-argument call silently injected legacy days"
printf 'PASS: successful no-argument run reports all-due pending and cumulative verified scope\n'

run_wrapper 0 30
[[ "$STATUS" -eq 0 ]] || fail "legacy positive days rejected"
[[ "$OUTPUT" == *"days=30 已弃用"* ]] || fail "legacy argument lacks deprecation warning"
[[ "$OUTPUT" == *"TEST_CARGO_ARGS: run --quiet --bin backfill_predictions -- 30"* ]] || fail "legacy days was not passed unchanged"
[[ "$OUTPUT" == *"所有 target_date <= as_of=2026-09-26 且 hit IS NULL"* ]] || fail "legacy days changed execution scope"
[[ "$OUTPUT" != *"近 30 天"* && "$OUTPUT" != *"pred_date >="* ]] || fail "legacy days still restricts visible scope"
printf 'PASS: legacy days stays accepted and deprecated without changing all-due scope\n'
