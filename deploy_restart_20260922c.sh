#!/bin/bash
# 2026-09-22 stale 判别日志部署: 重启 monitor
# ⚠️ 2026-09-23 复盘: 下方 nohup 直跑已被证伪 — nohup 只挡 SIGHUP, 挡不住
#    会话退出时的进程组 SIGKILL, 该 monitor 于 09-23 08:14 随会话静默死亡。
#    正确做法是 launchctl load -w (plist 带 KeepAlive)。本脚本仅存历史参考。
set -u
cd /Users/zhangzhen/Desktop/Quant/stock_analysis
OLD_PID=71603
STDERR=logs/monitor-launchd-20260916.stderr.log
OFFSET=$(wc -c < "$STDERR")
echo "== stderr offset=$OFFSET =="
if kill -0 "$OLD_PID" 2>/dev/null; then
  kill "$OLD_PID"
  for i in $(seq 1 30); do kill -0 "$OLD_PID" 2>/dev/null || break; sleep 2; done
  if kill -0 "$OLD_PID" 2>/dev/null; then echo "== SIGKILL =="; kill -9 "$OLD_PID"; sleep 3; fi
else
  echo "== 旧进程已不在 =="
fi
nohup ./target/release/monitor >> logs/monitor-launchd-20260916.stdout.log 2>> "$STDERR" < /dev/null &
NEW_PID=$!
echo "NEW_PID=$NEW_PID"
sleep 30
tail -c +$((OFFSET+1)) "$STDERR" | grep -E "AccountMode|BR-103|activation|selection-v2|capability=disabled|启动评估|NewsFlashStale" | head -10
echo "== disabled 计数 =="
tail -c +$((OFFSET+1)) "$STDERR" | grep -cE "capability=disabled" || true
ps -p "$NEW_PID" -o pid,lstart,etime | tail -1
