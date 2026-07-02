#!/usr/bin/env bash
# 一次性回填历史 prediction 的 actual_change 和 hit
#
# BR-009: monitor 工作流必须有显式 timeout (默认 30min, env BACKFILL_PRED_TIMEOUT_SECS 可覆盖)
#
# 用法: STOCK_DB=data/stock_analysis.db bash tools/one_shot/backfill_predictions.sh [DAYS=14]
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./_timeout_lib.sh
source "$SCRIPT_DIR/_timeout_lib.sh"

DAYS="${1:-14}"
DB="${STOCK_DB:-data/stock_analysis.db}"
[ ! -f "$DB" ] && { echo "DB $DB 不存在"; exit 1; }

START_DATE=$(date -v -"$DAYS"d +%Y-%m-%d 2>/dev/null || date -d "$DAYS days ago" +%Y-%m-%d)
END_DATE=$(date +%Y-%m-%d)
echo "回填 $START_DATE → $END_DATE (近 $DAYS 天)"
echo "timeout: ${BACKFILL_PRED_TIMEOUT_SECS:-1800}s (env BACKFILL_PRED_TIMEOUT_SECS 可覆盖)"

# BR-009: timeout 包装 cargo run, 超时 exit 2
with_timeout "${BACKFILL_PRED_TIMEOUT_SECS:-1800}" \
  bash -c "STOCK_DB='$DB' cargo run --quiet --bin backfill_predictions -- '$DAYS' 2>&1 | tail -50" \
  || { rc=$?; echo "✗ BR-009 timeout 或 cargo 失败 (exit $rc)"; exit $rc; }

echo "回填完成。验证分布:"
sqlite3 "$DB" "SELECT hit, COUNT(*) FROM prediction_tracker WHERE pred_date >= '$START_DATE' AND hit IS NOT NULL GROUP BY hit ORDER BY hit DESC;"