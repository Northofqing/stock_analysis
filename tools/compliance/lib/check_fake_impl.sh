#!/usr/bin/env bash
#
# check_fake_impl.sh — AGENTS.md §2.8 假实现禁令门禁
#
# 目的: 拦截"写日志不操作数据"的 verify/save/notify/sync/update_result 类假实现。
# 反模式 (R-1 修复新增):
#   match db.update_prediction_result(&today, None, 0.0, false) { ... }
#   db.update_prediction_result(..., 0.0, false)?;
#
# 退出码:
#   0 = pass (没有发现假实现)
#   1 = fail (发现假实现模式 / 必须存在的 e2e 测试缺失)
#
# 配套:
#   AGENTS.md §2.8 假实现禁令 — 描述反模式 + 正例
#   tests/e2e_prediction_verify.rs — 真实 verify 行为的 e2e 测试 (必须存在)

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SRC_DIR="$REPO_ROOT/src"
E2E_TEST="$REPO_ROOT/tests/e2e_prediction_verify.rs"

EXIT_CODE=0

# 1. 拦截假实现模式: update_.*result.*0\.0.*false
#    在 src/ 下任何 .rs 文件命中即 fail。
FAKE_PATTERN="update_.*result.*0\.0.*false"
HITS=$(grep -RInE "$FAKE_PATTERN" "$SRC_DIR" --include="*.rs" 2>/dev/null || true)

if [ -n "$HITS" ]; then
    echo "[check_fake_impl] FAIL: 发现假实现模式 '$FAKE_PATTERN':"
    echo "$HITS" | sed 's/^/  /'
    EXIT_CODE=1
else
    echo "[check_fake_impl] OK: src/ 下未发现假实现模式"
fi

# 2. 必须存在 e2e 测试 (R-1 修复新增 — 验证 verify 真实行为)
if [ ! -f "$E2E_TEST" ]; then
    echo "[check_fake_impl] FAIL: 必须存在 e2e 测试 $E2E_TEST (AGENTS §2.8 验证)"
    EXIT_CODE=1
else
    echo "[check_fake_impl] OK: e2e 测试存在 ($E2E_TEST)"
fi

if [ $EXIT_CODE -eq 0 ]; then
    echo "[check_fake_impl] PASS"
else
    echo "[check_fake_impl] FAIL — 见上方"
fi

exit $EXIT_CODE
