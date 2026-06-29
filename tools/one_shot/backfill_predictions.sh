#!/usr/bin/env bash
# 一次性回填历史 prediction 的 actual_change 和 hit
# 用法: STOCK_DB=data/stock_analysis.db bash tools/one_shot/backfill_predictions.sh [DAYS=14]
set -euo pipefail
DAYS="${1:-14}"
DB="${STOCK_DB:-data/stock_analysis.db}"
[ ! -f "$DB" ] && { echo "DB $DB 不存在"; exit 1; }

START_DATE=$(date -v -"$DAYS"d +%Y-%m-%d 2>/dev/null || date -d "$DAYS days ago" +%Y-%m-%d)
END_DATE=$(date +%Y-%m-%d)
echo "回填 $START_DATE → $END_DATE (近 $DAYS 天)"

STOCK_DB="$DB" cargo run --quiet --bin backfill_predictions -- "$DAYS" 2>&1 | tail -50

echo "回填完成。验证分布:"
sqlite3 "$DB" "SELECT hit, COUNT(*) FROM prediction_tracker WHERE pred_date >= '$START_DATE' AND hit IS NOT NULL GROUP BY hit ORDER BY hit DESC;"