#!/usr/bin/env bash
#
# check.sh — 项目数据合规门禁主入口
#
# 当前包含的检查:
#   - check_fake_impl.sh     (AGENTS §2.8 假实现禁令, PR-1)
#   - check_data_freshness.sh (AGENTS §2.4 数据时效门禁, PR-2)
#   - check_design_contradiction.sh (AGENTS §2.9 设计矛盾禁令, PR-3)
#   - check_business_rules.sh (AGENTS §2.10 业务规则文档化, PR-4)
#
# 后续 PR 会扩展:
#   - check_*.sh (PR-5+)
#
# 用法:
#   bash tools/compliance/check.sh             # 跑全部检查, 失败立即返回
#   bash tools/compliance/check.sh || exit 1   # CI 集成
#
# 退出码:
#   0 = 全部通过
#   非 0 = 至少一个检查失败 (子脚本退出码)

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LIB_DIR="$REPO_ROOT/tools/compliance/lib"

OVERALL_EXIT=0

run_check() {
    local script="$1"
    local path="$LIB_DIR/$script"
    if [ ! -x "$path" ]; then
        echo "[compliance] ERROR: 缺少可执行检查脚本: $path"
        OVERALL_EXIT=1
        return
    fi
    echo "===== $script ====="
    if ! "$path"; then
        OVERALL_EXIT=1
    fi
    echo
}

run_check "check_fake_impl.sh"
run_check "check_data_freshness.sh"
run_check "check_design_contradiction.sh"
run_check "check_business_rules.sh"

if [ $OVERALL_EXIT -eq 0 ]; then
    echo "[compliance] ALL CHECKS PASSED"
else
    echo "[compliance] ONE OR MORE CHECKS FAILED"
fi

exit $OVERALL_EXIT
