#!/usr/bin/env bash
# 一次性回填 stock_daily 数据 (R-3 修复)
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

STOCK_DB="$DB" cargo run --quiet --bin backfill_daily -- "$LIST" 2>&1 | tail -30

echo ""
echo "验证 stock_daily 状态:"
sqlite3 "$DB" "SELECT MAX(date) AS latest, COUNT(*) AS total FROM stock_daily;"

echo ""
echo "如需刷合规门禁, 跑: bash tools/compliance/check.sh"