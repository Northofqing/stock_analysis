#!/usr/bin/env bash
# §2.10 业务规则文档化
# 验证 docs/business_rules.md 存在、含 5 类 (去重/互斥/过滤/排序/限额)、
# 且关键代码文件引用了对应规则编号。
set -euo pipefail
FAIL=0
WARN=0
RULES_FILE="docs/business_rules.md"

# 规则 1: 文件存在
if [ ! -f "$RULES_FILE" ]; then
  echo "✗ §2.10.1 缺业务规则文件: $RULES_FILE" >&2
  exit 1
fi

# 规则 2: 5 类必须全有
for cat in "去重" "互斥" "过滤" "排序" "限额"; do
  if ! grep -q "$cat" "$RULES_FILE"; then
    echo "✗ §2.10.2 缺类别: $cat (在 $RULES_FILE 未找到)" >&2
    FAIL=1
  fi
done

# 规则 3: 关键函数必须引用规则编号
# 新增文件必须引用 (FAIL); 已存在文件 BR 引用消失仅 WARN (历史遗留)
REFS=(
  "src/opportunity/discover.rs BR-001"
  "src/opportunity/chain_mapper.rs BR-002"
  "src/search_service/service.rs BR-003"
)

# 决定 base ref: env 优先, 默认 origin/master, 失败回退到第一个 commit
BASE_REF="${BASE_REF:-origin/master}"
if ! git rev-parse --verify "$BASE_REF" >/dev/null 2>&1; then
  BASE_REF="$(git rev-list --max-parents=0 HEAD | tail -n1)"
fi

for entry in "${REFS[@]}"; do
  file="${entry% *}"
  rule_id="${entry##* }"
  if ! grep -q "$rule_id" "$file" 2>/dev/null; then
    # 判断 file 是不是新增: 在 BASE_REF 之前不存在
    if ! git cat-file -e "$BASE_REF:$file" 2>/dev/null; then
      NEW_FILE=1
    else
      NEW_FILE=0
    fi
    if [ "$NEW_FILE" -eq 1 ]; then
      echo "✗ §2.10.3 $file 是新增文件但未引用 $rule_id (AGENTS §2.10 强制)" >&2
      FAIL=1
    else
      echo "⚠ §2.10.3 $file 未引用 $rule_id (已存在文件, 仅 warn)" >&2
      WARN=$((WARN+1))
    fi
  fi
done

if [ $FAIL -eq 0 ]; then
  echo "✓ §2.10 业务规则检查通过 (warn: $WARN)"
fi
exit $FAIL
