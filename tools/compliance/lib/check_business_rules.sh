#!/usr/bin/env bash
# §2.10 业务规则文档化 (v10 P0.0 动态版 · 2026-07-01)
#
# 升级内容:
#   - REFS 数组硬编码 → 从 docs/business_rules.md 动态解析 BR + 代码位置
#   - 支持任意 BR-NNN (不再卡 BR-001/002/003 硬编码)
#   - 标"待实现" 的 BR (PENDING) 走 WARN 提醒而非 FAIL
#   - 5 类 (去重/互斥/过滤/排序/限额) 必全有 (硬约束)
#
# 设计不变:
#   - 新增文件必须引用 BR (FAIL)
#   - 已存在文件 BR 引用消失仅 WARN (历史遗留, 不阻断)
#   - BASE_REF fallback 与 I-7 修复保持
#
# 实现细节:
#   - FAIL 通过输出 ✗ 行统计 (避免 while-read 子 shell 变量丢失)
#   - 用 mktemp 临时文件收集, 最后汇总
set -euo pipefail
FAIL_LINES_FILE="$(mktemp -t cbr_fail.XXXXXX)"
WARN_LINES_FILE="$(mktemp -t cbr_warn.XXXXXX)"
trap 'rm -f "$FAIL_LINES_FILE" "$WARN_LINES_FILE"' EXIT
# RULES_FILE env override (fixture 测试用), 默认 docs/business_rules.md
RULES_FILE="${RULES_FILE:-docs/business_rules.md}"

fail() { echo "✗ $*" >> "$FAIL_LINES_FILE"; }
warn() { echo "⚠ $*" >> "$WARN_LINES_FILE"; }

# 规则 1: 文件存在
if [ ! -f "$RULES_FILE" ]; then
  echo "✗ §2.10.1 缺业务规则文件: $RULES_FILE" >&2
  exit 1
fi

# 规则 2: 5 类必须全有
for cat in "去重" "互斥" "过滤" "排序" "限额"; do
  if ! grep -q "$cat" "$RULES_FILE"; then
    fail "§2.10.2 缺类别: $cat (在 $RULES_FILE 未找到)"
  fi
done

# 规则 3: 从 business_rules.md 动态提取 BR + 代码位置
# 表格行格式: | BR-NNN | 类别 | 规则 | `file:line` (可能有多个 + 分隔) | 测试位置 | 末审 |
# 输出 TSV: <br_id>\tPENDING\t<code_loc>   OR   <br_id>\t<file>
EXTRACTED=$(awk -F'|' '
  /^\| BR-[0-9]+ / {
    br_id = $2; gsub(/ /, "", br_id);
    code_loc = $5; gsub(/^ +| +$/, "", code_loc);
    if (code_loc == "") next;
    # 标"待实现" 的行走 PENDING 路径 (代码未到位, BR 已登记, 提醒而非 FAIL)
    if (code_loc ~ /待实现/) {
      print br_id "\tPENDING\t" code_loc;
      next;
    }
    # 提取所有 backtick 内的 file paths
    n = split(code_loc, parts, "`");
    for (i = 2; i < n; i += 2) {
      path = parts[i];
      # 提取 file path: 去掉 ::function / :line / 注释 / 周围括号
      sub(/::.*/, "", path);
      sub(/:[0-9].*/, "", path);
      sub(/ \(.*\)/, "", path);
      sub(/^ +| +$/, "", path);
      # 跳过空 / 纯符号 (BR-012 "北向资金" 这种)
      if (path == "" || path !~ /\//) continue;
      print br_id "\t" path;
    }
  }
' "$RULES_FILE")

if [ -z "$EXTRACTED" ]; then
  fail "§2.10.3 业务规则表无任何 BR 行可解析 (检查 markdown 格式)"
fi

# 决定 base ref (保留 I-7 修复)
BASE_REF="${BASE_REF:-origin/master}"
if ! git rev-parse --verify "$BASE_REF" >/dev/null 2>&1; then
  warn "§2.10 BASE_REF '$BASE_REF' 不存在, 走 root commit fallback, 可能误判新增文件 (修复 I-7)"
  BASE_REF="$(git rev-list --max-parents=0 HEAD | tail -n1)"
fi

# 规则 4: 验证每个 (BR, file) 引用 — 用 process substitution 避免子 shell 变量丢失
while IFS=$'\t' read -r br_id file; do
  [ -z "$br_id" ] && continue
  [ -z "$file" ] && continue
  if [ ! -f "$file" ]; then
    fail "§2.10.3 $file 引用 $br_id 但文件不存在"
    continue
  fi
  if ! grep -q "$br_id" "$file" 2>/dev/null; then
    # 判断 file 是不是新增: 在 BASE_REF 之前不存在
    if ! git cat-file -e "$BASE_REF:$file" 2>/dev/null; then
      NEW_FILE=1
    else
      NEW_FILE=0
    fi
    if [ "$NEW_FILE" -eq 1 ]; then
      fail "§2.10.3 $file 是新增文件但未引用 $br_id (AGENTS §2.10 强制)"
    else
      warn "§2.10.3 $file 未引用 $br_id (已存在文件, 仅 warn)"
    fi
  fi
done < <(echo "$EXTRACTED" | awk -F'\t' '$2 != "PENDING"')

# 规则 5: PENDING BR 提醒
PENDING_BRS=$(echo "$EXTRACTED" | awk -F'\t' '$2 == "PENDING" { print $1 }' | sort -u)
PENDING_COUNT=0
if [ -n "$PENDING_BRS" ]; then
  warn "§2.10.4 已登记但代码未到位的 BR (PENDING, 实施后应去掉'待实现:' 前缀):"
  for br in $PENDING_BRS; do
    warn "  - $br"
    PENDING_COUNT=$((PENDING_COUNT+1))
  done
fi

# 汇总 + 输出
FAIL_COUNT=$(wc -l < "$FAIL_LINES_FILE" | tr -d ' ')
WARN_COUNT=$(wc -l < "$WARN_LINES_FILE" | tr -d ' ')

# 输出所有 fail/warn 行 (实际看到)
cat "$FAIL_LINES_FILE" >&2
cat "$WARN_LINES_FILE" >&2

if [ "$FAIL_COUNT" -eq 0 ]; then
  ACTIVE_COUNT=$(echo "$EXTRACTED" | awk -F'\t' '$2 != "PENDING"' | wc -l | tr -d ' ')
  echo "✓ §2.10 业务规则检查通过 (active: $ACTIVE_COUNT, pending: $PENDING_COUNT, warn: $WARN_COUNT)"
  exit 0
else
  echo "✗ §2.10 业务规则检查 FAIL ($FAIL_COUNT errors, $WARN_COUNT warns)" >&2
  exit 1
fi
