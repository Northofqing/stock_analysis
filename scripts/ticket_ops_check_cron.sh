#!/bin/bash
# 交易日自检包装器：只读生产库，输出 JSON；供 Hermes cron 调用。
# 到期项缺失时由 Python 侧经 magiclaw CLI 推送飞书（同日同失败集合只推一次）。
set -uo pipefail
R=/Users/zhangzhen/.local/share/stock-analysis-runtime
REPO=/Users/zhangzhen/Desktop/Quant/stock_analysis
OUT="$R/data/ops_check/$(date +%F-%H%M).json"
mkdir -p "$R/data/ops_check"
# monitor 可能由 launchd 或手动进程启动；按最近修改选当前 stderr，
# 否则 cron 会一直读旧 launchd 日志，coverage/失败原因全部失真。
LOG="${MONITOR_LOG:-}"
if [ -z "$LOG" ]; then
  LOG=$(ls -t "$R"/logs/monitor-*.stderr.log "$R"/logs/monitor-manual-*.log 2>/dev/null | head -n 1)
fi
[ -n "$LOG" ] || LOG="$R/logs/monitor-launchd.stderr.log"
python3 "$REPO/scripts/ticket_ops_check.py" \
  --db "$R/data/stock_analysis.db" \
  --delivery-db "$R/data/durable_delivery.sqlite3" \
  --log "$LOG" \
  --data-dir "$R/data" \
  --out "$OUT" \
  --notify fail
status=$?
cp "$OUT" "$R/data/ops_check/latest.json"
exit "$status"
