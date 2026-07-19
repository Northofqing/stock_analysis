#!/usr/bin/env bash
#
# check_no_silent_fallback_global.sh — AGENTS.md §2.1 / §2.2 全局 WARN 监测
#
# 退出码: 始终 0 (WARN only, 不阻断 CI)
# 配套: tools/compliance/check.sh:49 `run_check ... || true` 兜底,
#       即使本脚本非零退出, check.sh 也不会失败。
#
# 目的: 扫描整个 src/ 找出 unwrap_or(0.0) / unwrap_or("N/A") / unwrap_or("—") 等
#       silent fill 反模式。**仅 WARN，不阻断 CI** —— 因为:
#       1. data_provider/*.rs 的 parse::<f64>().unwrap_or(0.0) 是 API 容错（合法）
#       2. strategy/ 的 0.0 是数学默认值（合法）
#       3. 推送层真违规由 check_no_silent_fallback_push.sh 拦截
#       4. 全局数据供未来重构决策（哪些 0.0 可改为 Option<f64> + log warn）
#
# 退出码:
#   0 = 始终返回 0（仅 WARN）
#
# 配套:
#   check_no_silent_fallback_push.sh —— 推送层精准 FAIL
#   AGENTS.md §2.1 / §2.2
#   docs/business_rules.md BR-004

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SRC_DIR="$REPO_ROOT/src"

if [ ! -d "$SRC_DIR" ]; then
    echo "[check_no_silent_fallback_global] SKIP: $SRC_DIR 不存在"
    exit 0
fi

echo "[check_no_silent_fallback_global] 扫描整个 src/ (仅 WARN, 不阻断 CI)"

# ----------------------------------------------------------------------------
# 分类统计
# ----------------------------------------------------------------------------

count_for() {
    local label="$1"
    local pattern="$2"
    local count
    count=$(grep -RInE "$pattern" "$SRC_DIR" \
        --include="*.rs" \
        --exclude-dir=target \
        --exclude-dir=docs \
        2>/dev/null | wc -l | tr -d ' ')
    echo "  $label: $count 处"
}

count_for "unwrap_or(0.0)" '\.unwrap_or\(0\.0\)'
count_for 'unwrap_or("N/A")' '\.unwrap_or\("N/A"\)'
count_for 'unwrap_or("—") em-dash' '\.unwrap_or\("—"\)'
count_for "unwrap_or(&0.0)" '\.unwrap_or\(\&0\.0\)'

# ----------------------------------------------------------------------------
# 按目录分桶（看哪些层违规最多）
# ----------------------------------------------------------------------------

echo ""
echo "[check_no_silent_fallback_global] 按目录分桶 (前 5, 3 层深度):"
grep -RInE '\.unwrap_or\(0\.0\)|\.unwrap_or\("N/A"\)|\.unwrap_or\("—"\)|\.unwrap_or\(\&0\.0\)' \
    "$SRC_DIR" --include="*.rs" --exclude-dir=target --exclude-dir=docs 2>/dev/null \
    | sed "s|$REPO_ROOT/||" \
    | awk -F'/' 'NF>=3 {print $1"/"$2"/"$3}' \
    | sort | uniq -c | sort -rn | head -5 \
    | sed 's/^/  /'

# ----------------------------------------------------------------------------
# 列出"应被 check_no_silent_fallback_push.sh 抓住但实际是全局漏的" — 用于回归
# F5 修订: 精确匹配 push 脚本的扫描面 (仅 main.rs/push_templates.rs/notify.rs)
# ----------------------------------------------------------------------------

PUSH_LEAKS=$(grep -RInE '\.unwrap_or\(0\.0\)|\.unwrap_or\("—"\)' \
    "$SRC_DIR/bin/monitor/main.rs" \
    "$SRC_DIR/bin/monitor/push_templates.rs" \
    "$SRC_DIR/bin/monitor/notify.rs" \
    2>/dev/null | wc -l | tr -d ' ')

# 注: 此处数字应 == push 脚本输出 (W1 后现状，因 push 脚本 v14.2 实施前跳过 unwrap_or 检查,
#     此处用于"如果 push 脚本启用，预期会 FAIL 多少处"的预演)

echo ""
echo "[check_no_silent_fallback_global] bin/monitor 推送层 0.0/— 总数: $PUSH_LEAKS"
echo "  (推送层违规数 = push 脚本输出; 此处 WARN = 推送脚本未抓到的)"
echo ""
echo "[check_no_silent_fallback_global] WARN 完成 (exit=0, 不阻断 CI)"

exit 0