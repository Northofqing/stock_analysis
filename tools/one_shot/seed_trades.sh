#!/usr/bin/env bash
# 一次性 seed trades (v67 — R-05/R-06 验证用)
#
# R-05 信号复盘 + R-06 失败归因 都依赖 trades 表
# 沙箱 0 trades → dispatcher skip. 这里 insert 1 笔平仓验证.
#
# 用法:
#   bash tools/one_shot/seed_trades.sh

set -euo pipefail
DB="${STOCK_DB:-data/stock_analysis.db}"
[ ! -f "$DB" ] && { echo "DB $DB 不存在: $DB"; exit 1; }

TODAY=$(date -u +%Y-%m-%d)
echo "[v67] seed trades @ $TODAY"

# 1 笔平仓 (sell) + 1 笔买入 (buy) — R-05 / R-06 都看 sell 交易
sqlite3 "$DB" <<SQL
DELETE FROM trades WHERE traded_at = '$TODAY';
INSERT INTO trades (code, name, direction, price, shares, amount, reason, traded_at) VALUES
  ('002208', '合肥城建', 'buy',  19.27, 200, 3854.0, '实盘建仓',   '$TODAY 09:35:00'),
  ('002208', '合肥城建', 'sell', 17.50, 200, 3500.0, '止损卖出',   '$TODAY 14:35:00');
SQL

echo "[v67] trades: $(sqlite3 "$DB" "SELECT COUNT(*) FROM trades WHERE traded_at='$TODAY';") 行"
