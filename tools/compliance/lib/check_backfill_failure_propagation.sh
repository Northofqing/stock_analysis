#!/usr/bin/env bash
# BR-009 / AGENTS §2.8 — backfill must propagate Cargo failure and stop
# before reporting database verification.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
BACKFILL="$REPO_ROOT/tools/one_shot/backfill_daily.sh"
TEST_TMP="$(mktemp -d "${TMPDIR:-/tmp}/stock-analysis-backfill-check.XXXXXX")"
FAKE_BIN="$TEST_TMP/bin"
TEST_DB="$TEST_TMP/TEST_CODE_stock_analysis.db"

cleanup() {
    rm -rf "$TEST_TMP"
}
trap cleanup EXIT

mkdir -p "$FAKE_BIN"
printf '%s\n' '#!/usr/bin/env bash' 'exit 42' > "$FAKE_BIN/cargo"
chmod +x "$FAKE_BIN/cargo"
: > "$TEST_DB"

set +e
OUTPUT=$(
    PATH="$FAKE_BIN:$PATH" \
        STOCK_DB="$TEST_DB" \
        STOCK_LIST="TEST_CODE_BACKFILL" \
        BACKFILL_DAILY_TIMEOUT_SECS=5 \
        bash "$BACKFILL" 2>&1
)
STATUS=$?
set -e

if [ "$STATUS" -ne 42 ]; then
    echo "[check_backfill_failure_propagation] FAIL: expected exit=42 actual=$STATUS" >&2
    echo "$OUTPUT" >&2
    exit 1
fi

if grep -q '验证 stock_daily 状态' <<<"$OUTPUT"; then
    echo "[check_backfill_failure_propagation] FAIL: verification ran after Cargo failure" >&2
    exit 1
fi

if ! grep -q 'BR-009 timeout 或 cargo 失败 (exit 42)' <<<"$OUTPUT"; then
    echo "[check_backfill_failure_propagation] FAIL: explicit failure evidence missing" >&2
    echo "$OUTPUT" >&2
    exit 1
fi

echo "[check_backfill_failure_propagation] PASS: Cargo failure remains exit=42"
