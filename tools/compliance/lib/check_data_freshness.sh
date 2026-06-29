#!/usr/bin/env bash
#
# check_data_freshness.sh — AGENTS.md §2.4 数据时效门禁 (R-3 修复新增)
#
# 目的: 拦截 stock_daily 等时间序列表停更超过 1 个交易日 (周末/节假日除外)。
# 原理: 读 SQLite 中 stock_daily.MAX(date), 与今日对比。
#
# 退出码:
#   0 = pass (DB 不存在 -> skip, 数据新鲜, 或在 1 个交易日内)
#   1 = fail (数据断层超过 1 个交易日)
#
# 配套:
#   AGENTS.md §2.4 数据时效 — 强化条款 (PR-2)
#   tools/one_shot/backfill_daily.sh — 修复手段

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
DB_PATH="${STOCK_DB:-$REPO_ROOT/data/stock_analysis.db}"

# 1. DB 不存在 -> skip (新环境/沙箱, 不阻断)
if [ ! -f "$DB_PATH" ]; then
    echo "[check_data_freshness] SKIP: DB 不存在 ($DB_PATH)"
    exit 0
fi

# 2. sqlite3 不可用 -> skip
if ! command -v sqlite3 >/dev/null 2>&1; then
    echo "[check_data_freshness] SKIP: sqlite3 命令不可用"
    exit 0
fi

LATEST=$(sqlite3 "$DB_PATH" "SELECT MAX(date) FROM stock_daily;" 2>/dev/null || echo "")
if [ -z "$LATEST" ] || [ "$LATEST" = "" ]; then
    echo "[check_data_freshness] FAIL: stock_daily 表为空" >&2
    echo "    修复: bash tools/one_shot/backfill_daily.sh" >&2
    exit 1
fi

TODAY=$(date +%Y-%m-%d)

# 3. 用 Python 算天数差 (无 python3 则回退到 date 命令)
if command -v python3 >/dev/null 2>&1; then
    STALE_DAYS=$(python3 -c "
from datetime import date
try:
    print((date.fromisoformat('$TODAY') - date.fromisoformat('$LATEST')).days)
except Exception:
    print('error')
" 2>/dev/null || echo "error")
else
    # Fallback: 用 %s 算秒数差
    LATEST_TS=$(date -j -f "%Y-%m-%d" "$LATEST" "+%s" 2>/dev/null || date -d "$LATEST" "+%s" 2>/dev/null || echo "")
    TODAY_TS=$(date -j -f "%Y-%m-%d" "$TODAY" "+%s" 2>/dev/null || date -d "$TODAY" "+%s" 2>/dev/null || echo "")
    if [ -z "$LATEST_TS" ] || [ -z "$TODAY_TS" ]; then
        STALE_DAYS="error"
    else
        STALE_DAYS=$(( (TODAY_TS - LATEST_TS) / 86400 ))
    fi
fi

if [ "$STALE_DAYS" = "error" ]; then
    echo "[check_data_freshness] FAIL: 日期解析失败 (latest=$LATEST today=$TODAY)" >&2
    exit 1
fi

# 4. 允许 1 个交易日滞后 (周末/节假日缓冲)
if [ "$STALE_DAYS" -gt 1 ]; then
    echo "[check_data_freshness] FAIL: §2.4 数据断层 — stock_daily 停更 $STALE_DAYS 天" >&2
    echo "    最新日期: $LATEST" >&2
    echo "    今日: $TODAY" >&2
    echo "    修复: bash tools/one_shot/backfill_daily.sh" >&2
    exit 1
fi

echo "[check_data_freshness] PASS: stock_daily 最新 $LATEST (滞后 $STALE_DAYS 天)"
exit 0