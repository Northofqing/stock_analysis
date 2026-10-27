#!/bin/bash
# 交易日自检包装器：只读生产库，输出 JSON；供 Hermes cron 调用。
set -euo pipefail
R=/Users/zhangzhen/.local/share/stock-analysis-runtime
REPO=/Users/zhangzhen/Desktop/Quant/stock_analysis
OUT="$R/data/ops_check/$(date +%F-%H%M).json"
mkdir -p "$R/data/ops_check"
exec python3 "$REPO/scripts/ticket_ops_check.py" \
  --db "$R/data/stock_analysis.db" \
  --delivery-db "$R/data/durable_delivery.sqlite3" \
  --log "$R/logs/monitor-launchd.stderr.log" \
  --out "$OUT"
