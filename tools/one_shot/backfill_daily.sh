#!/usr/bin/env bash
# 一次性回填 stock_daily 数据 (R-3 修复)
#
# BR-009: monitor 工作流必须有显式 timeout
#  - 默认 30min timeout (env `BACKFILL_DAILY_TIMEOUT_SECS` 可覆盖)
#  - 超时未完成 → log error + flush + exit 2 (不静默)
#  - 0/负数/非法值 fallback 到 1800s
#
# 用法:
#   STOCK_DB=data/stock_analysis.db bash tools/one_shot/backfill_daily.sh
#   STOCK_DB=data/stock_analysis.db STOCK_LIST=000001,600519 bash tools/one_shot/backfill_daily.sh
#   STOCK_DB=data/stock_analysis.db bash tools/one_shot/backfill_daily.sh 000001,600519
#
# 数据源: RustDX 通达信 (主) → GtimgProvider (备) → HttpProvider (备)
# 写表: stock_daily (UPSERT, ON CONFLICT DO UPDATE)
#
# 与 backfill_predictions.sh 风格保持一致 (一次性脚本, 不入 monitor 主循环).
set -euo pipefail
DB="${STOCK_DB:-data/stock_analysis.db}"
[ ! -f "$DB" ] && { echo "DB $DB 不存在"; exit 1; }

# BR-009 timeout 包装 (复用一个 one_shot 公共函数, P0-G 阶段, 2026-07-01)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=./_timeout_lib.sh
source "$SCRIPT_DIR/_timeout_lib.sh"

LIST="${1:-${STOCK_LIST:-}}"
if [ -z "$LIST" ]; then
    # 默认回填自选股 (从 .env 读 STOCK_LIST; 若没有, 用监控常见标)
    if [ -f .env ]; then
        LIST=$(grep -E '^STOCK_LIST=' .env | cut -d= -f2- | tr -d '"' || echo "")
    fi
fi
LIST="${LIST:-000001,600519,000858,002415,300750}"

echo "回填 stock_daily, 标的: $LIST"
echo "DB: $DB"
echo "timeout: ${BACKFILL_DAILY_TIMEOUT_SECS:-1800}s (env BACKFILL_DAILY_TIMEOUT_SECS 可覆盖)"

# BR-009: 用 timeout 包装 cargo run, 超时 exit 2
with_timeout "${BACKFILL_DAILY_TIMEOUT_SECS:-1800}" \
  bash -c "STOCK_DB='$DB' cargo run --quiet --bin backfill_daily -- '$LIST' 2>&1 | tail -30" \
  || { rc=$?; echo "✗ BR-009 timeout 或 cargo 失败 (exit $rc)"; exit $rc; }

echo ""
echo "验证 stock_daily 状态:"
sqlite3 "$DB" "SELECT MAX(date) AS latest, COUNT(*) AS total FROM stock_daily;"

echo ""
echo "如需刷合规门禁, 跑: bash tools/compliance/check.sh"