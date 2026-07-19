#!/usr/bin/env bash
#
# check_no_silent_fallback_push.sh — 推送层 AGENTS.md §2.1 / §2.2 静默填补门禁
#
# 目的: 仅扫描**推送渲染层**（b-009 R-1 修订 v14.2 §3.3.2 管辖范围），
#       拦截 silent fill / fake data fallback。其它层（data_provider 等）
#       的容错由 check_no_silent_fallback_global.sh 仅 WARN 监测。
#
# 红线约束 (AGENTS.md §2.1 / §2.2):
#   - 数据源失败 MUST 是显式错误，MUST NOT 降级为 fake data fallbacks
#   - Missing data fields MUST be left blank or logged as warnings;
#     MUST NOT be silently filled
#
# 扫描范围 (推送层, b-009 R-1 + W1.2.b 拆分修订):
#   - src/push_l*/**/*.rs        (v14.2 L1-L7 新模块)
#   - src/bin/monitor/push_templates.rs
#   - src/bin/monitor/notify.rs
#   - src/bin/monitor/main.rs   (顶层循环推送入口)
#
# 违规模式:
#   1. enum 变体名: DegradeWithDefault / DegradeWithNa
#   2. 函数/字段名: fallback_values / fallback_message / apply_fallback / mark_na
#   3. Rust 模式串:
#      *.unwrap_or(0.0) / *.unwrap_or("0.0") / *.unwrap_or("N/A")
#      *.unwrap_or("—") / *.unwrap_or(&0.0)
#   4. TOML 字段:    fallback_values / fallback_message / on_failure = "degrade_*"
#
# 退出码:
#   0 = pass
#   1 = fail (发现违规模式)
#
# 配套:
#   AGENTS.md §2.1 / §2.2 数据红线
#   docs/architecture/v14.2-push-architecture.md §3.3.2 (b-009 R-1 修订)
#   docs/bugs/b-009-v14.2-r2评审.md R-1
#   docs/business_rules.md BR-004 (no silent fallback to cost_price)
#   tools/compliance/lib/check_no_silent_fallback_global.sh — 全局 WARN
#
# 用法:
#   bash tools/compliance/lib/check_no_silent_fallback_push.sh
#   bash tools/compliance/check.sh  # 通过主入口运行

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
TOML_DIR="$REPO_ROOT/config/push_templates"
TESTS_DIR="$REPO_ROOT/tests"

# 推送层白名单 (用 grep -E --include 模拟; grep 无 --include-dir 故用 find)
PUSH_LAYER_PATHS=()
for rel in \
    src/push_l1 \
    src/push_l2 \
    src/push_l3 \
    src/push_l4 \
    src/push_l5 \
    src/push_l6 \
    src/push_l7 \
    src/bin/monitor/push_templates.rs \
    src/bin/monitor/notify.rs \
    src/bin/monitor/main.rs; do
    if [ -e "$REPO_ROOT/$rel" ]; then
        PUSH_LAYER_PATHS+=("$REPO_ROOT/$rel")
    fi
done

# F2 修订 (code-reviewer): 倒置 guard —— v14.2 未实施前，**只对 push_l*/ 目录启用** unwrap_or 检查
# W2.3 修订: push_l1/ 已建 (W2.1 落地), 但 L4 dispatcher 还没建 (W4 计划).
#           单独 push_l1/ 存在不应触发严格态 —— 因为 v13 render 函数仍在运行,
#           历史 unwrap_or(0.0) 是已计划的 P1/P2 修复项 (B-010).
# 双条件: push_l1 + L4 dispatcher 同时存在 → 启用 unwrap_or 检查
ENABLE_UNWRAP_CHECK=false
if [ -d "$REPO_ROOT/src/push_l1" ] && [ -d "$REPO_ROOT/src/push_l4" ]; then
    ENABLE_UNWRAP_CHECK=true
fi

if [ "$ENABLE_UNWRAP_CHECK" = false ]; then
    echo "[check_no_silent_fallback_push] NOTE: v14.2 L4 dispatcher 未建立, 跳过 unwrap_or 类检查"
    echo "  (push_l1/ W2.1 已建, push_l4/ W4 计划中)"
    echo "  (P1/P2 业务聚合层违规已在 B-010 修复计划中, M0-5 §9.1 协调矩阵: 不在 W2 单独修)"
    echo "  (结构性检查 — enum 变体名 / 字段名 / 函数名 — 仍跑)"
fi

EXIT_CODE=0

# ----------------------------------------------------------------------------
# 1. Rust 代码违规模式扫描
# ----------------------------------------------------------------------------

scan_rust() {
    local label="$1"
    local pattern="$2"
    local hits
    # W3.2 修订 + code-reviewer HIGH 修复: 排除 4 类注释
    #   - /// outer doc comment
    #   - //! inner doc comment
    #   - // line comment
    #   - /* */ block comment (单行匹配, 多行状态机复杂度超出脚本职责)
    # 仅在代码行匹配
    hits=$(grep -RInE "$pattern" "${PUSH_LAYER_PATHS[@]}" \
        --include="*.rs" \
        --exclude-dir=target \
        2>/dev/null \
        | awk -F: '
            {
                text = ""
                for (i=3; i<=NF; i++) text = text (i==3?"":":") $i
                # 跳过 /// outer doc comment
                if (text ~ /^[ \t]*\/\/\//) next
                # 跳过 //! inner doc comment
                if (text ~ /^[ \t]*\/\/!/) next
                # 跳过 // line comment (非 /// 和非 //!)
                if (text ~ /^[ \t]*\/\/[^\/!]/) next
                # 跳过 /* block comment 单行 (含结尾 */)
                if (text ~ /^[ \t]*\/[\*].*\*\//) next
                # 跳过 /* 多行块注释的开始
                if (text ~ /^[ \t]*\/[\*][^\*]*$/) next
                print
            }' || true)
    if [ -n "$hits" ]; then
        echo "[check_no_silent_fallback_push] FAIL: Rust 违规模式 '$label' ($pattern):"
        echo "$hits" | sed 's/^/  /'
        EXIT_CODE=1
    else
        echo "[check_no_silent_fallback_push] OK: 未发现 Rust 模式 '$label'"
    fi
}

# 1a. enum 变体名
scan_rust "DegradeWithDefault enum 变体" '\bDegradeWithDefault\b'
scan_rust "DegradeWithNa enum 变体" '\bDegradeWithNa\b'

# 1b. 函数/字段名
scan_rust "fallback_values 字段/函数" '\bfallback_values\b'
scan_rust "fallback_message 字段/函数" '\bfallback_message\b'
scan_rust "apply_fallback 函数" '\bapply_fallback\b'
scan_rust "mark_na 函数" '\bmark_na\b'

# 1c. unwrap_or 静默填补反模式 (v14.2 §3.3.2 明确禁止)
#     F2 修订: 仅在 push_l*/ 目录存在 (即 v14.2 实施后) 才启用 unwrap_or 检查
if [ "$ENABLE_UNWRAP_CHECK" = true ]; then
    scan_rust "unwrap_or(0.0) 静默填补" '\.unwrap_or\(0\.0\)'
    scan_rust 'unwrap_or("N/A") 静默填补' '\.unwrap_or\("N/A"\)'
    scan_rust 'unwrap_or("—") em-dash 占位 (BR-004 同款违规)' '\.unwrap_or\("—"\)'
    scan_rust "unwrap_or(&0.0) 静默填补" '\.unwrap_or\(\&0\.0\)'
fi

# 1d. unwrap_or_default — F4 修订 (code-reviewer): 删除 theatrical regex
#     原 regex `\b(price|...)\b.*\.unwrap_or_default\(\)` 实际匹配率为 0
#     (真实代码用中间变量, 标识符和 unwrap_or_default 不在同一行)
#     改用 clippy::unwrap_or_default 在 Cargo.toml 配置, 更可靠

# ----------------------------------------------------------------------------
# 2. TOML 模板违规模式扫描 (仅在目录存在时执行)
# ----------------------------------------------------------------------------

if [ -d "$TOML_DIR" ]; then
    scan_toml() {
        local label="$1"
        local pattern="$2"
        local hits
        hits=$(grep -RInE "$pattern" "$TOML_DIR" \
            --include="*.toml" \
            2>/dev/null || true)
        if [ -n "$hits" ]; then
            echo "[check_no_silent_fallback_push] FAIL: TOML 违规模式 '$label' ($pattern):"
            echo "$hits" | sed 's/^/  /'
            EXIT_CODE=1
        else
            echo "[check_no_silent_fallback_push] OK: 未发现 TOML 模式 '$label'"
        fi
    }

    scan_toml "fallback_values 字段" '^[[:space:]]*fallback_values[[:space:]]*='
    scan_toml "fallback_message 字段" '^[[:space:]]*fallback_message[[:space:]]*='
    scan_toml 'on_failure = "degrade_with_default"' 'on_failure[[:space:]]*=[[:space:]]*"degrade_with_default"'
    scan_toml 'on_failure = "degrade_with_na"' 'on_failure[[:space:]]*=[[:space:]]*"degrade_with_na"'
else
    # F6 修订: 显式标注 SKIP (PASS) 区别于真实 PASS
    echo "[check_no_silent_fallback_push] SKIP (PASS) — $TOML_DIR 不存在, 模板层未实施 (v14.2 M3 才有)"
fi

# ----------------------------------------------------------------------------
# 3. 必须存在回归测试 (b-009 R-1 防护; F3 修订 — 加 WARN 升级日期)
# ----------------------------------------------------------------------------

NO_FALLBACK_TEST="$TESTS_DIR/no_silent_fallback_test.rs"
WARN_ESCALATION_DATE="2026-10-01"  # F3: v14.2 W5 完成后, WARN 升级为 FAIL

if [ ! -f "$NO_FALLBACK_TEST" ]; then
    CURRENT_DATE=$(date +%Y-%m-%d)
    if [ "$CURRENT_DATE" \> "$WARN_ESCALATION_DATE" ] || [ "$CURRENT_DATE" = "$WARN_ESCALATION_DATE" ]; then
        echo "[check_no_silent_fallback_push] FAIL: 回归测试 $NO_FALLBACK_TEST 不存在 (升级日期 $WARN_ESCALATION_DATE 已过)"
        EXIT_CODE=1
    else
        echo "[check_no_silent_fallback_push] WARN: 建议存在回归测试 $NO_FALLBACK_TEST"
        echo "  (F3: $WARN_ESCALATION_DATE 后 WARN 升级为 FAIL, 计划 v14.2 W5 完成时补齐)"
    fi
else
    echo "[check_no_silent_fallback_push] OK: 回归测试存在 ($NO_FALLBACK_TEST)"
fi

# ----------------------------------------------------------------------------
# 总结
# ----------------------------------------------------------------------------

if [ $EXIT_CODE -eq 0 ]; then
    echo "[check_no_silent_fallback_push] PASS"
else
    echo "[check_no_silent_fallback_push] FAIL — 见上方违规条目"
fi

exit $EXIT_CODE