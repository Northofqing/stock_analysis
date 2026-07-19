#!/usr/bin/env bash
# AGENTS.md §3.1: validate required pull-request evidence fields.

set -euo pipefail

BODY="${PR_BODY:-}"
if [ -z "$BODY" ]; then
    echo "[check_pr_evidence] FAIL: PR_BODY 为空" >&2
    exit 1
fi

missing=0
for pattern in \
    'Refs:[[:space:]]*(spec|docs/|config/)' \
    'Data-Redlines:[[:space:]]*\[[^]]+\]' \
    'OldModules:' \
    'Threshold-Proof:' \
    'Business-Rules:' \
    'Rollback:'; do
    if ! grep -Eq "$pattern" <<<"$BODY"; then
        echo "[check_pr_evidence] FAIL: 缺少 PR 字段/$pattern" >&2
        missing=1
    fi
done

if [ "$missing" -ne 0 ]; then
    exit 1
fi
echo "[check_pr_evidence] PASS"
