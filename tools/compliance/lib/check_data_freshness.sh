#!/usr/bin/env bash
# AGENTS §2.4 — stock_daily must be no more than one A-share trading day stale.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
DB_PATH="${STOCK_DB:-$REPO_ROOT/data/stock_analysis.db}"
CALENDAR_PATH="${TRADING_CALENDAR:-$REPO_ROOT/config/a_share_market_holidays.csv}"
TODAY="${FRESHNESS_TODAY:-$(date +%Y-%m-%d)}"

fail() {
    echo "[check_data_freshness] FAIL: $*" >&2
    echo "    修复数据: bash tools/one_shot/backfill_daily.sh" >&2
    exit 1
}

[ -f "$DB_PATH" ] || fail "生产数据库不存在 ($DB_PATH)"
command -v sqlite3 >/dev/null 2>&1 || fail "sqlite3 命令不可用"
command -v python3 >/dev/null 2>&1 || fail "python3 命令不可用，无法计算交易日"
[ -f "$CALENDAR_PATH" ] || fail "交易日历不存在 ($CALENDAR_PATH)"

LATEST_QUERY=$(sqlite3 "$DB_PATH" "SELECT MAX(date) FROM stock_daily;" 2>&1)
QUERY_STATUS=$?
[ "$QUERY_STATUS" -eq 0 ] || fail "无法读取 stock_daily: $LATEST_QUERY"
[ -n "$LATEST_QUERY" ] || fail "stock_daily 表为空"
LATEST="$LATEST_QUERY"

RESULT=$(python3 - "$LATEST" "$TODAY" "$CALENDAR_PATH" <<'PY'
from datetime import date, timedelta
from pathlib import Path
import sys

latest_raw, today_raw, calendar_raw = sys.argv[1:]
try:
    latest = date.fromisoformat(latest_raw)
    today = date.fromisoformat(today_raw)
except ValueError as error:
    print(f"ERROR:日期解析失败: {error}")
    raise SystemExit(0)

if latest > today:
    print(f"ERROR:stock_daily 最新日期 {latest} 晚于检查日期 {today}")
    raise SystemExit(0)

years = set()
holidays = set()
for raw in Path(calendar_raw).read_text(encoding="utf-8").splitlines():
    line = raw.strip()
    if line.startswith("# year="):
        try:
            years.add(int(line.split("=", 1)[1]))
        except ValueError:
            print(f"ERROR:非法 calendar year 标记: {line}")
            raise SystemExit(0)
    elif line and not line.startswith("#"):
        try:
            holidays.add(date.fromisoformat(line))
        except ValueError:
            print(f"ERROR:非法休市日期: {line}")
            raise SystemExit(0)

for year in range(latest.year, today.year + 1):
    if year not in years:
        print(f"ERROR:交易日历缺少 year={year} 覆盖")
        raise SystemExit(0)

trading_days = 0
cursor = latest + timedelta(days=1)
while cursor <= today:
    if cursor.weekday() < 5 and cursor not in holidays:
        trading_days += 1
    cursor += timedelta(days=1)
print(f"OK:{trading_days}")
PY
)

case "$RESULT" in
    ERROR:*) fail "${RESULT#ERROR:}" ;;
    OK:*) STALE_TRADING_DAYS="${RESULT#OK:}" ;;
    *) fail "交易日计算返回未知结果: $RESULT" ;;
esac

if [ "$STALE_TRADING_DAYS" -gt 1 ]; then
    fail "§2.4 stock_daily 已滞后 $STALE_TRADING_DAYS 个交易日 (latest=$LATEST today=$TODAY)"
fi

echo "[check_data_freshness] PASS: stock_daily 最新 $LATEST (滞后 $STALE_TRADING_DAYS 个交易日)"
