#!/usr/bin/env bash
# AGENTS.md §3.1: validate required pull-request evidence fields.

set -euo pipefail

BODY="${PR_BODY:-}"
if [ -z "$BODY" ]; then
    echo "[check_pr_evidence] FAIL: PR_BODY 为空" >&2
    exit 1
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"

missing=0
for pattern in \
    'Refs:[[:space:]]*(spec|docs/|config/)' \
    'Data-Redlines:[[:space:]]*\[[^]]+\]' \
    'OldModules:' \
    'Threshold-Proof:' \
    'Business-Rules:' \
    'Rollback:' \
    '^Gate-Policy:[[:space:]]*PR=core-patch90\+other-patch85\+ratchet;[[:space:]]*Release=global80\+core95\+freshness\+live[[:space:]]*$' \
    '^Bootstrap-Baseline:[[:space:]]*(true|false)[[:space:]]*$' \
    '^Baseline-Source-SHA:[[:space:]]*[0-9a-f]{40}[[:space:]]*$' \
    '^Baseline-Global:[[:space:]]*[0-9]+/[0-9]+[[:space:]]*$' \
    '^Baseline-Core:[[:space:]]*[0-9]+/[0-9]+[[:space:]]*$' \
    '^Coverage-Tools:[[:space:]]*rustc=[^;]+;[[:space:]]*LLVM=[^;]+;[[:space:]]*cargo-llvm-cov=[^;[:space:]]+[[:space:]]*$' \
    '^Gate-C:[[:space:]]*PASS[[:space:]]*$' \
    '^Gate-D:[[:space:]]*(PASS|Release Blocked)[[:space:]]*$'; do
    if ! grep -Eq "$pattern" <<<"$BODY"; then
        echo "[check_pr_evidence] FAIL: 缺少 PR 字段/$pattern" >&2
        missing=1
    fi
done

if [ "$missing" -ne 0 ]; then
    exit 1
fi

if ! command -v python3 >/dev/null 2>&1; then
    echo "[check_pr_evidence] FAIL: python3 不可用，无法核对 coverage contract" >&2
    exit 1
fi
CONTRACT_VALUES="$(python3 - "$REPO_ROOT/config/design_contracts.toml" <<'PY'
import pathlib
import sys
import tomllib

coverage = tomllib.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))["coverage"]
print("|".join(str(coverage[name]) for name in (
    "source_sha",
    "global_covered",
    "global_count",
    "core_covered",
    "core_count",
    "rustc_release",
    "llvm_version",
    "cargo_llvm_cov_version",
)))
PY
)" || {
    echo "[check_pr_evidence] FAIL: 无法读取 coverage contract" >&2
    exit 1
}
IFS='|' read -r SOURCE_SHA GLOBAL_COVERED GLOBAL_COUNT CORE_COVERED CORE_COUNT RUSTC_RELEASE LLVM_VERSION CARGO_LLVM_COV_VERSION <<<"$CONTRACT_VALUES"

for exact in \
    "Baseline-Source-SHA: $SOURCE_SHA" \
    "Baseline-Global: $GLOBAL_COVERED/$GLOBAL_COUNT" \
    "Baseline-Core: $CORE_COVERED/$CORE_COUNT" \
    "Coverage-Tools: rustc=$RUSTC_RELEASE; LLVM=$LLVM_VERSION; cargo-llvm-cov=$CARGO_LLVM_COV_VERSION"; do
    if ! grep -Fqx "$exact" <<<"$BODY"; then
        echo "[check_pr_evidence] FAIL: PR evidence does not match contract: $exact" >&2
        exit 1
    fi
done

if [ -n "${PR_BASE_SHA:-}" ]; then
    if ! git -C "$REPO_ROOT" cat-file -e "$PR_BASE_SHA^{commit}" 2>/dev/null; then
        echo "[check_pr_evidence] FAIL: PR_BASE_SHA 不可验证" >&2
        exit 1
    fi
    EXPECTED_BOOTSTRAP="true"
    if git -C "$REPO_ROOT" show "$PR_BASE_SHA:config/design_contracts.toml" 2>/dev/null \
        | grep -qx '\[coverage\]'; then
        EXPECTED_BOOTSTRAP="false"
    fi
    if ! grep -Fqx "Bootstrap-Baseline: $EXPECTED_BOOTSTRAP" <<<"$BODY"; then
        echo "[check_pr_evidence] FAIL: Bootstrap-Baseline 与 base contract 状态不一致" >&2
        exit 1
    fi
fi
echo "[check_pr_evidence] PASS"
