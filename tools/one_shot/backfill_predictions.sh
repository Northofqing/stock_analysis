#!/usr/bin/env bash
# 一次性回填所有已到期且 hit IS NULL 的 prediction。
#
# BR-009: monitor 工作流必须有显式 timeout (默认 30min, env BACKFILL_PRED_TIMEOUT_SECS 可覆盖)
#
# 用法: STOCK_DB=data/stock_analysis.db bash tools/one_shot/backfill_predictions.sh [DAYS]
# DAYS 仅保留旧调用兼容，已弃用；不再限制执行或统计范围。
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./_timeout_lib.sh
source "$SCRIPT_DIR/_timeout_lib.sh"

DB="${STOCK_DB:-data/stock_analysis.db}"
[ ! -f "$DB" ] && { echo "DB $DB 不存在"; exit 1; }

if [ "$#" -gt 0 ]; then
  echo "days=$1 已弃用：仅兼容旧参数，不限制回填或统计范围" >&2
fi
AS_OF=$(date +%Y-%m-%d)
echo "回填所有 target_date <= as_of=$AS_OF 且 hit IS NULL 的到期记录（程序以本地运行日为准）"
echo "timeout: ${BACKFILL_PRED_TIMEOUT_SECS:-1800}s (env BACKFILL_PRED_TIMEOUT_SECS 可覆盖)"

# BR-009: timeout 包装 cargo run, 超时 exit 2
export STOCK_DB="$DB"
with_timeout "${BACKFILL_PRED_TIMEOUT_SECS:-1800}" \
  bash -o pipefail -c 'cargo run --quiet --bin backfill_predictions -- "$@" 2>&1 | tail -50' \
  backfill_predictions "$@" \
  || { rc=$?; echo "✗ BR-009 timeout 或 cargo 失败 (exit $rc)"; exit $rc; }

AS_OF=$(date +%Y-%m-%d)
echo "回填完成。所有到期记录快照（统计 as_of=${AS_OF}；已验证为累计值，非本轮新增）:"
sqlite3 "$DB" "SELECT '到期待验证剩余', COUNT(*) FROM prediction_tracker WHERE target_date <= '$AS_OF' AND hit IS NULL;
SELECT '到期已验证累计', hit, COUNT(*) FROM prediction_tracker WHERE target_date <= '$AS_OF' AND hit IS NOT NULL GROUP BY hit ORDER BY hit DESC;"
