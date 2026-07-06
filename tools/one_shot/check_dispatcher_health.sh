#!/usr/bin/env bash
# v14.6: 监控告警脚本 (dispatcher_log 健康度)
# BR-007 风格 — 每日健康检查 + 告警邮件
# 建议 cron: 0 9-19/2 * * 1-5 (每 2 小时工作日 9-19 点)
#
# 告警条件:
#   1. 1 小时内失败 > 3 次
#   2. snapshot_size=0 持续 > 1 小时 (数据源异常)
#   3. 某模板 24 小时无推送 (调度异常)
#
# 使用:
#   bash tools/one_shot/check_dispatcher_health.sh
#   或
#   ALERT_EMAIL=ops@example.com bash tools/one_shot/check_dispatcher_health.sh

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
LOG_DIR="$REPO_ROOT/data/dispatcher_log"
LOG_FILE_PATTERN="$LOG_DIR/$(date +%Y-%m-%d).jsonl"
ALERT_EMAIL="${ALERT_EMAIL:-}"
HOUR_AGO=$(date -v-1H +%Y-%m-%dT%H:%M:%S 2>/dev/null || date -d '1 hour ago' +%Y-%m-%dT%H:%M:%S)

cd "$REPO_ROOT" || exit 1

if [ ! -d "$LOG_DIR" ]; then
    echo "ERROR: dispatcher_log 目录不存在: $LOG_DIR" >&2
    exit 1
fi

# 聚合最近 1 小时的所有日志
RECENT_LOGS=$(cat "$LOG_DIR"/*.jsonl 2>/dev/null | awk -v cutoff="$HOUR_AGO" '
    $0 ~ /"ts":/ {
        # 提取 ts 字段
        match($0, /"ts":"([^"]+)"/, ts)
        if (ts[1] >= cutoff) print
    }
')

if [ -z "$RECENT_LOGS" ]; then
    echo "WARN: 最近 1 小时无 dispatcher_log 记录"
    exit 0
fi

# === 告警 1: 1 小时内失败 > 3 次 ===
FAIL_COUNT=$(echo "$RECENT_LOGS" | grep -c '"success":false' || true)
if [ "$FAIL_COUNT" -gt 3 ]; then
    MSG="[ALERT] $FAIL_COUNT dispatcher failures in last 1h (threshold > 3)"
    echo "$MSG"
    [ -n "$ALERT_EMAIL" ] && echo "$MSG" | mail -s "monitor push alert" "$ALERT_EMAIL" || true
fi

# === 告警 2: snapshot_size=0 持续 > 1 小时 ===
EMPTY_COUNT=$(echo "$RECENT_LOGS" | grep -c '"snapshot_size":0' || true)
if [ "$EMPTY_COUNT" -gt 0 ]; then
    EMPTY_KINDS=$(echo "$RECENT_LOGS" | grep '"snapshot_size":0' | grep -oE '"kind":"[^"]+"' | sort -u)
    MSG="[ALERT] snapshot_size=0 x$EMPTY_COUNT (data source issue). Kinds: $(echo $EMPTY_KINDS | tr '\n' ' ')"
    echo "$MSG"
    [ -n "$ALERT_EMAIL" ] && echo "$MSG" | mail -s "monitor data source alert" "$ALERT_EMAIL" || true
fi

# === 告警 3: 某模板 24 小时无推送 (调度异常) ===
ALL_KINDS="P-01 I-01 I-02 I-03 D-01 A-01"
YESTERDAY=$(date -v-24H +%Y-%m-%d 2>/dev/null || date -d '24 hours ago' +%Y-%m-%d)
YESTERDAY_LOG="$LOG_DIR/${YESTERDAY}.jsonl"

for KIND in $ALL_KINDS; do
    if [ -f "$LOG_FILE_PATTERN" ] || [ -f "$YESTERDAY_LOG" ]; then
        # 检查最近 24h 是否有该 KIND
        PRESENT=$(cat "$LOG_DIR"/*.jsonl 2>/dev/null | grep -c "\"kind\":\"$KIND\"" || echo 0)
        if [ "$PRESENT" -eq 0 ]; then
            MSG="[ALERT] $KIND 24h 无推送 (调度异常)"
            echo "$MSG"
            [ -n "$ALERT_EMAIL" ] && echo "$MSG" | mail -s "monitor schedule alert" "$ALERT_EMAIL" || true
        fi
    fi
done

# === 统计输出 ===
TOTAL=$(echo "$RECENT_LOGS" | wc -l)
SUCCESS=$(echo "$RECENT_LOGS" | grep -c '"success":true' || true)
echo "=== 最近 1h 统计 ==="
echo "  总推送: $TOTAL"
echo "  成功: $SUCCESS"
echo "  失败: $FAIL_COUNT"
echo "  成功率: $([ "$TOTAL" -gt 0 ] && echo "scale=2; $SUCCESS * 100 / $TOTAL" | bc || echo 0)%"

exit 0
