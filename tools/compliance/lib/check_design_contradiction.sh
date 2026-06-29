#!/usr/bin/env bash
#
# check_design_contradiction.sh — AGENTS.md §2.9 设计矛盾门禁 (R-2 修复新增)
#
# 目的: 拦截"推送门 > 评分封顶"这类上下游配置矛盾。
# 原理: 从 config/opportunity.toml 读 event_risk_score_threshold,
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
CONFIG="$REPO_ROOT/config/opportunity.toml"
SRC_DIR="$REPO_ROOT/src/opportunity"

FAIL=0

# 1. 提取 toml 里 event_risk_score_threshold
THRESHOLD=$(grep -E "^\s*event_risk_score_threshold\s*=" "$CONFIG" 2>/dev/null | \
  grep -oE "[0-9]+" | head -1 || echo "")
if [ -z "$THRESHOLD" ]; then
  echo "[check_design_contradiction] SKIP: 未找到 event_risk_score_threshold 配置"
  exit 0
fi

# 2. 提取 rust 源码里 event_risk_score 上下文的最大封顶值
#    找所有 min(N.0) 在 winrate_score.is_none() 路径附近
CLAMP_MAX=$(grep -rn -A 3 "winrate_score.is_none()" "$SRC_DIR" 2>/dev/null | \
  grep -oE "min\([0-9.]+\)" | grep -oE "[0-9.]+" | sort -rn | head -1 || echo "")

# 3. 兜底: 找 event_risk_score 的所有 clamp/min
if [ -z "$CLAMP_MAX" ]; then
  CLAMP_MAX=$(grep -rn "event_risk_score" "$REPO_ROOT/src/" 2>/dev/null | \
    grep -oE "(min\([0-9.]+\)|clamp\([0-9.]+)" | \
    grep -oE "[0-9.]+" | sort -rn | head -1 || echo "")
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
