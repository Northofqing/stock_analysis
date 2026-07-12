#!/usr/bin/env bash
#
# check_design_contradiction.sh — AGENTS.md §2.9 设计矛盾门禁 (R-2 修复新增)
#
# 目的: 拦截"推送门 > 评分封顶"这类上下游配置矛盾。
# 原理: 从 config/strategy.toml 读 event_risk_score_threshold (v12 重构后从 opportunity.toml 合并),
#       从 src/opportunity/ 源码里 grep 出最大的 min(N.0) 封顶值,
#       若 threshold > clamp_max 即 fail。
#
# 退出码:
#   0 = pass (已对齐, 或缺配置 skip)
#   1 = fail (设计矛盾)
#
# 配套:
#   AGENTS.md §2.9 设计矛盾禁令
#   tests/test_design_contradiction.rs

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
CONFIG="$REPO_ROOT/config/strategy.toml"  # v12 重构后从 opportunity.toml 合并
SRC_DIR="$REPO_ROOT/src/opportunity"

FAIL=0

# 1. 提取 toml 里 event_risk_score_threshold
THRESHOLD=$(grep -E "^\s*event_risk_score_threshold\s*=" "$CONFIG" 2>/dev/null | \
  grep -oE "[0-9]+" | head -1 || echo "")
if [ -z "$THRESHOLD" ]; then
  echo "[check_design_contradiction] SKIP: 未找到 event_risk_score_threshold 配置"
  exit 0
fi

# 2. 提取 rust 源码里 event_risk_score 的最大封顶值
#    适配 3 种模式 (R-2 修复后):
#    (a) 直接: min(70.0)
#    (b) 变量: clamp_max = 70.0 (let clamp_max = if gray_open { 70.0 } else { ... })
#    (c) 注释: // 封顶 70
CLAMP_MAX=""

# (a) 抓 min(数字) 字面量
CLAMP_A=$(grep -rn "event_risk_score" "$REPO_ROOT/src/" 2>/dev/null | \
  grep -oE "min\([0-9.]+\)" | grep -oE "[0-9.]+" | sort -rn | head -1 || echo "")

# (b) 抓 clamp_max = ... 的完整表达式块, 支持多行 if/else:
#     let clamp_max = if gray_open {
#         70.0
#     } else {
#         (THRESHOLD_FALLBACK - 1.0).max(0.0)
#     };
CLAMP_B=$(awk '
  /clamp_max[[:space:]]*=/ { in_block=1 }
  in_block {
    line=$0
    while (match(line, /[0-9]+(\.[0-9]+)?/)) {
      print substr(line, RSTART, RLENGTH)
      line=substr(line, RSTART + RLENGTH)
    }
    if (line ~ /;/) { in_block=0 }
  }
' "$SRC_DIR"/*.rs 2>/dev/null | sort -rn | head -1 || echo "")

# (c) 兜底: 抓注释 // 封顶 数字
#     限制: 数字后必须接非数字 (避免抓到行号 "line 647")
CLAMP_C=$(grep -rn "event_risk_score" "$REPO_ROOT/src/" 2>/dev/null | \
  grep -E "封顶\s*[0-9]+\.[0-9]+|[0-9]+\s*分" 2>/dev/null | \
  grep -oE "[0-9]+\.[0-9]+" | sort -rn | head -1 || echo "")

# 取三路中最大值 (灰度期的 70 是设计意图, 应被检测)
CLAMP_MAX=$(printf "%s\n%s\n%s\n" "$CLAMP_A" "$CLAMP_B" "$CLAMP_C" | sort -rn | head -1)
if [ -z "$CLAMP_MAX" ]; then
  CLAMP_MAX="0.0"
fi

# 4. 矛盾检测
if [ -n "$CLAMP_MAX" ]; then
  if python3 -c "import sys; sys.exit(0 if $THRESHOLD > $CLAMP_MAX else 1)" 2>/dev/null; then
    echo "✗ §2.9 设计矛盾: 推送门 ($THRESHOLD) > 评分封顶 ($CLAMP_MAX)" >&2
    echo "    来源: config/opportunity.toml vs src/opportunity/score.rs" >&2
    echo "    修复: 调低 threshold 或调高 clamp, 或补 winrate 数据让 NS3 解除" >&2
    FAIL=1
  else
    echo "✓ §2.9 阈值对齐: threshold ($THRESHOLD) ≤ clamp ($CLAMP_MAX)"
  fi
else
  echo "[check_design_contradiction] SKIP: 未找到 event_risk 封顶值"
fi

exit $FAIL
