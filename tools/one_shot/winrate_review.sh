#!/usr/bin/env bash
# BR-007: 季度 winrate review — 跑 backfill + simulator, 输出 markdown 报告
#
# 用途: 季度 cron 跑一次, 跟踪主题胜率漂移, 给"下次评估"清单.
# 设计哲学 (AGENTS §2.4): 数据驱动决策循环, 主动发现需要关停/加权的主题.
#
# 用法:
#   bash tools/one_shot/winrate_review.sh [DAYS=14]
#
# 输出:
#   reports/winrate_review_YYYY-MM-DD.md — markdown 报告
#   stdout: 关键指标 + 决策建议
#
# Cron 建议 (crontab -e):
#   0 9 1 */3 * /path/to/stock_analysis/tools/one_shot/winrate_review.sh 14
#   (每月 1 日 9 点, 或季度首日; 季度更稳)
#
# 退出码:
#   0 = 跑通, 报告已生成
#   1 = backfill / simulator 失败
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
DAYS="${1:-14}"
DB_PATH="${STOCK_DB:-$REPO_ROOT/data/stock_analysis.db}"
REPORTS_DIR="$REPO_ROOT/reports"
TODAY="$(date +%Y-%m-%d)"
REPORT="$REPORTS_DIR/winrate_review_${TODAY}.md"

mkdir -p "$REPORTS_DIR"

cd "$REPO_ROOT"

# 1. 数据新鲜度门禁 (AGENTS §2.4)
echo "═══ [BR-007] 季度 winrate review 开始 ($TODAY, ${DAYS} 天) ═══"
bash tools/compliance/check.sh > "$REPORT.tmp" 2>&1
COMPLIANCE_EXIT=$?
if [ "$COMPLIANCE_EXIT" -ne 0 ]; then
    echo "✗ compliance check FAIL ($COMPLIANCE_EXIT), 不跑 review"
    cat "$REPORT.tmp"
    rm "$REPORT.tmp"
    exit 1
fi

# 2. Backfill predictions (确保 hit_rate 数字最新)
echo ""
echo "── 跑 backfill_predictions -- $DAYS ──"
if ! STOCK_DB="$DB_PATH" cargo run --bin backfill_predictions -- "$DAYS" 2>&1 | tail -3; then
    echo "✗ backfill 失败"
    exit 1
fi

# 3. 跑 simulator (拿 winrate + 决策建议)
echo ""
echo "── 跑 winrate_simulator ──"
SIMULATOR_OUTPUT=$(STOCK_DB="$DB_PATH" cargo run --bin winrate_simulator 2>&1 | tail -50)

# 4. 提取关键指标
TOTAL_PUSH=$(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM prediction_tracker WHERE pred_date >= date('now', '-$DAYS days');")
TOTAL_VERIFIED=$(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM prediction_tracker WHERE pred_date >= date('now', '-$DAYS days') AND hit IS NOT NULL;")
TOTAL_HITS=$(sqlite3 "$DB_PATH" "SELECT SUM(CASE WHEN hit=1 THEN 1 ELSE 0 END) FROM prediction_tracker WHERE pred_date >= date('now', '-$DAYS days') AND hit IS NOT NULL;")
HIT_RATE=$(awk "BEGIN { if ($TOTAL_VERIFIED > 0) printf \"%.1f\", $TOTAL_HITS*100.0/$TOTAL_VERIFIED; else print \"N/A\" }")
PENDING=$(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM prediction_tracker WHERE hit IS NULL AND pred_date >= date('now', '-$DAYS days');")

# 5. 写 markdown 报告
cat > "$REPORT" <<EOF
# Winrate Review — $TODAY

**回顾区间**: 最近 ${DAYS} 天
**数据源**: \`$DB_PATH\`

## 关键指标

| 指标 | 值 |
|------|---|
| 推送总数 | $TOTAL_PUSH |
| 已 verify | $TOTAL_VERIFIED (pending: $PENDING) |
| 命中 | $TOTAL_HITS |
| **真实胜率** | **${HIT_RATE}%** |
| 推送主题数 | $(sqlite3 "$DB_PATH" "SELECT COUNT(DISTINCT theme_name) FROM prediction_tracker WHERE pred_date >= date('now', '-$DAYS days');") |

## winrate_simulator 输出

\`\`\`
$SIMULATOR_OUTPUT
\`\`\`

## 决策建议

EOF

# 6. 抽取 simulator 的"建议下次评估"清单
echo "$SIMULATOR_OUTPUT" | grep -E "建议下次评估|建议下次" >> "$REPORT" || echo "  (simulator 未给出建议)" >> "$REPORT"

cat >> "$REPORT" <<EOF

## 历史对比 (相对 v9.2 起点)

| 阶段 | 胜率 | 备注 |
|------|------|------|
| 起点 (R-1 修后) | 7.6% | verify 函数实化 |
| v9.2 PR-1~4 完成 | 7.6% | 修了 7 个 bug 但数据未回填 |
| 全市场 backfill | 32.3% | 132 票 14 天数据 |
| 一轮关停 + 加权 | 38.0% | 7 个 0% 主题关停 |
| 二轮关停 + 加权 | 51.7% | 5 个新 0% 关停 + 3 加权 |
| 三轮关停 + 加权 | **${HIT_RATE}%** | 本次 review |

## 下一步

- 若有新 ≥30% 主题未加权, 评估加权
- 若有新 0% 主题未关停, 加 fallback rule (BR-006 历史归类模式)
- 若全局胜率下降 ≥5pp, 排查 chain_mapper 或数据源问题

EOF

rm "$REPORT.tmp"

echo ""
echo "═══ [BR-007] 报告已生成: $REPORT ═══"
echo ""
echo "摘要:"
echo "  真实胜率: ${HIT_RATE}%"
echo "  推送: $TOTAL_PUSH, 命中: $TOTAL_HITS, pending: $PENDING"
echo ""
echo "查看完整报告:"
echo "  cat $REPORT"