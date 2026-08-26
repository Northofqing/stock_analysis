#!/usr/bin/env bash
#
# check.sh — 项目数据合规门禁主入口
#
# 当前包含的检查:
#   - check_fake_impl.sh     (AGENTS §2.8 假实现禁令, PR-1)
#   - check_data_freshness.sh (AGENTS §2.4 数据时效门禁, PR-2)
#   - check_design_contradiction.sh (AGENTS §2.9 设计矛盾禁令, PR-3)
#   - check_business_rules.sh (AGENTS §2.10 业务规则文档化, PR-4)
#   - check_backfill_failure_propagation.sh (BR-009 / AGENTS §2.8)
#   - check_br194_review_dependency.sh (BR-194 复盘依赖、日历与终态重放)
#
# 后续 PR 会扩展:
#   - check_*.sh (PR-5+)
#
# BR-252 分层策略:
#   pr      = Gate C 离线检查；不签发生产 freshness 结论
#   release = Gate D 完整检查；包含生产 freshness（默认，保持兼容）
#
# 用法:
#   bash tools/compliance/check.sh
#   bash tools/compliance/check.sh --policy pr
#   bash tools/compliance/check.sh --policy release
#
# 退出码:
#   0 = 全部通过
#   非 0 = 至少一个检查失败 (子脚本退出码)

set -uo pipefail

POLICY="release"
if [ "$#" -eq 0 ]; then
    :
elif [ "$#" -eq 2 ] && [ "$1" = "--policy" ]; then
    case "$2" in
        pr|release) POLICY="$2" ;;
        *)
            echo "Usage: bash tools/compliance/check.sh [--policy pr|release]" >&2
            exit 2
            ;;
    esac
else
    echo "Usage: bash tools/compliance/check.sh [--policy pr|release]" >&2
    exit 2
fi

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

echo "[compliance] policy=$POLICY"

run_check "check_fake_impl.sh"
if [ "$POLICY" = "release" ]; then
    run_check "check_data_freshness.sh"
else
    echo "[compliance] freshness: NOT RUN (Gate C offline policy)"
    echo
fi
run_check "check_design_contradiction.sh"
run_check "check_business_rules.sh"
run_check "check_backfill_failure_propagation.sh"
run_check "check_no_silent_fallback_push.sh"
run_check "check_no_silent_fallback_global.sh"
run_check "check_br174_legacy_callers.sh"
run_check "check_br194_review_dependency.sh"

if [ $OVERALL_EXIT -eq 0 ]; then
    echo "[compliance] ALL CHECKS PASSED"
else
    echo "[compliance] ONE OR MORE CHECKS FAILED"
fi

exit $OVERALL_EXIT
