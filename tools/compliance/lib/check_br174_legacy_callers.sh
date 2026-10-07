#!/usr/bin/env bash
#
# BR-171 / BR-174 / BR-178 final-cutover compliance gate.
#
# This check intentionally fails while a deprecated caller still exists. It is
# not a compatibility allow-list: the final cutover may pass only after the
# fixed production audit writer and the receipt-bound outcome capability seam
# have replaced every legacy production caller listed below.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
FAILURES=0
SELF_TEST_TMP=""

cleanup_self_test() {
    if [ -n "$SELF_TEST_TMP" ] && [ -d "$SELF_TEST_TMP" ]; then
        rm -rf -- "$SELF_TEST_TMP"
    fi
}

fail() {
    echo "[check_br174_legacy_callers] FAIL: $*" >&2
    FAILURES=$((FAILURES + 1))
}

scan_forbidden() {
    local root="$1"
    local rule_id="$2"
    local description="$3"
    local pattern="$4"
    shift 4

    local output
    local status
    if output=$(rg \
        --no-heading \
        --line-number \
        --with-filename \
        --color never \
        --multiline \
        --pcre2 \
        --glob '*.rs' \
        "$pattern" \
        "$@" 2>&1); then
        fail "$rule_id $description"
        printf '%s\n' "$output" | sed 's/^/  /' >&2
        return
    else
        status=$?
    fi

    if [ "$status" -ne 1 ]; then
        fail "$rule_id scan error under $root (rg exit=$status): $description"
        printf '%s\n' "$output" | sed 's/^/  /' >&2
    fi
}

require_pattern() {
    local rule_id="$1"
    local description="$2"
    local pattern="$3"
    local path="$4"

    if [ ! -e "$path" ]; then
        fail "$rule_id required search path is absent: ${path#$REPO_ROOT/}"
        return
    fi
    if ! rg --quiet --multiline --pcre2 "$pattern" "$path"; then
        fail "$rule_id $description: ${path#$REPO_ROOT/}"
    fi
}

require_absent_file() {
    local rule_id="$1"
    local description="$2"
    local path="$3"

    if [ -e "$path" ]; then
        fail "$rule_id $description: ${path#$REPO_ROOT/}"
    fi
}

check_audit_root_contract() {
    local root="$1"
    local src="$root/src"
    local audit="$src/selection/audit.rs"

    scan_forbidden \
        "$root" \
        "AUDIT-OLD-CONSTRUCTOR" \
        "caller-selected SelectionAuditWriter::for_environment remains" \
        'SelectionAuditWriter\s*::\s*for_environment\b' \
        "$src"
    scan_forbidden \
        "$root" \
        "AUDIT-OLD-ENVIRONMENT" \
        "SelectionAuditEnvironment remains constructible" \
        '\bSelectionAuditEnvironment\b' \
        "$src"
    scan_forbidden \
        "$root" \
        "AUDIT-CWD-RELATIVE" \
        "a CWD-relative data/audit path constructor remains" \
        '(?:PathBuf\s*::\s*from|Path\s*::\s*new|SelectionAuditWriter\s*::\s*(?:new|from_root|with_root))\s*\(\s*"data/audit(?:/|")' \
        "$src"
    scan_forbidden \
        "$root" \
        "AUDIT-CWD-LOOKUP" \
        "current_dir is used to derive a data/audit path" \
        '(?s)(?:current_dir\s*\(\s*\).{0,400}data/audit|data/audit.{0,400}current_dir\s*\(\s*\))' \
        "$src"

    require_pattern \
        "AUDIT-FIXED-ROOT" \
        'production audit writer is not anchored by env!("CARGO_MANIFEST_DIR")' \
        'env!\s*\(\s*"CARGO_MANIFEST_DIR"\s*\)' \
        "$audit"
    require_pattern \
        "AUDIT-FIXED-PATH" \
        "fixed production audit namespace is missing" \
        '"data/audit/production"' \
        "$audit"
}

check_outcome_capability_contract() {
    local root="$1"
    local src="$root/src"

    scan_forbidden \
        "$root" \
        "OUTCOME-PUBLIC-TERMINAL" \
        "public CompletedSessionTerminal DTO remains" \
        'pub(?:\s*\([^)]*\))?\s+(?:enum|struct)\s+CompletedSessionTerminal\b' \
        "$src"
    scan_forbidden \
        "$root" \
        "OUTCOME-PUBLIC-SETTLED" \
        "caller-built CompletedSessionTerminal::Settled remains" \
        '\bCompletedSessionTerminal\s*::\s*Settled\b' \
        "$src"

    require_pattern \
        "OUTCOME-ADMITTED-CAPABILITY" \
        "AdmittedOutcomeDailyBars capability is absent" \
        'pub\s+struct\s+AdmittedOutcomeDailyBars\s*\{' \
        "$src"
    require_pattern \
        "OUTCOME-DUE-CAPABILITY" \
        "VerifiedOutcomeDue capability is absent" \
        'pub\s+struct\s+VerifiedOutcomeDue\s*\{' \
        "$src"

    scan_forbidden \
        "$root" \
        "OUTCOME-ADMITTED-FIELDS" \
        "AdmittedOutcomeDailyBars exposes forgeable fields" \
        '(?s)pub\s+struct\s+AdmittedOutcomeDailyBars\s*\{[^}]*\bpub(?:\s*\([^)]*\))?\s+[A-Za-z_][A-Za-z0-9_]*\s*:' \
        "$src"
    scan_forbidden \
        "$root" \
        "OUTCOME-DUE-FIELDS" \
        "VerifiedOutcomeDue exposes forgeable fields" \
        '(?s)pub\s+struct\s+VerifiedOutcomeDue\s*\{[^}]*\bpub(?:\s*\([^)]*\))?\s+[A-Za-z_][A-Za-z0-9_]*\s*:' \
        "$src"
    scan_forbidden \
        "$root" \
        "OUTCOME-ADMITTED-CONSTRUCTOR" \
        "AdmittedOutcomeDailyBars exposes a public constructor" \
        '(?s)impl\s+AdmittedOutcomeDailyBars\s*\{.{0,1600}\bpub(?:\s*\([^)]*\))?\s+fn\s+(?:new|from_[A-Za-z0-9_]*|build|create)\b' \
        "$src"
    scan_forbidden \
        "$root" \
        "OUTCOME-DUE-CONSTRUCTOR" \
        "VerifiedOutcomeDue exposes a public constructor" \
        '(?s)impl\s+VerifiedOutcomeDue\s*\{.{0,1600}\bpub(?:\s*\([^)]*\))?\s+fn\s+(?:new|from_[A-Za-z0-9_]*|build|create)\b' \
        "$src"
}

check_legacy_cutover_contract() {
    local root="$1"
    local src="$root/src"

    scan_forbidden \
        "$root" \
        "LEGACY-AGGREGATOR" \
        "evidence-erasing legacy news batch API remains" \
        '\b(?:SelectionEventBatch|FeedAttemptStatus|tick_news_aggregator_batch|evaluate_news_batch)\b' \
        "$src"
    scan_forbidden \
        "$root" \
        "LEGACY-OUTCOME-MODULE-CALLER" \
        "legacy opportunity news_outcome caller remains" \
        '(?:stock_analysis\s*::\s*)?opportunity\s*::\s*news_outcome\s*::' \
        "$src"
    scan_forbidden \
        "$root" \
        "LEGACY-V1-REPORT" \
        "legacy selection report/backtest caller remains" \
        '(?:stock_analysis\s*::\s*)?selection\s*::\s*report(?:::|\b)|\.visible_samples\s*\(' \
        "$src/bin" "$src/opportunity" "$src/selection"
    scan_forbidden \
        "$root" \
        "LEGACY-V1-WRITER" \
        "public/direct legacy v1 candidate or outcome writer remains" \
        '\.(?:append_candidate|append_outcome)\s*\(' \
        "$src/bin" "$src/opportunity" "$src/selection"
    scan_forbidden \
        "$root" \
        "LEGACY-V1-DUE" \
        "legacy v1 due-outcome caller remains outside its frozen repository" \
        '(?:\.due_outcomes|load_due_outcomes)\s*\(' \
        "$src/bin" "$src/opportunity" "$src/selection"
    scan_forbidden \
        "$root" \
        "LEGACY-SETTLEMENT-OWNER" \
        "monitor still calls the legacy selection::outcome settlement owner" \
        '(?s)(?:use\s+stock_analysis\s*::\s*selection\s*::\s*outcome\s*::|selection_shadow\s*::\s*settle_due_outcomes\s*\()' \
        "$src/bin"
    scan_forbidden \
        "$root" \
        "LEGACY-FUZZY-BOARD" \
        "legacy fuzzy/Top-N board candidate acquisition remains" \
        '\b(?:resolve_stocks|search_board_code_by_keyword|fetch_board_components)\s*\(' \
        "$src/opportunity/chain_mapper.rs"
    scan_forbidden \
        "$root" \
        "LEGACY-OPPORTUNITY-CANDIDATES" \
        "legacy opportunity candidate-generation entry point remains" \
        '\b(?:run_opportunity_scan|run_post_close_candidates)\s*\(' \
        "$src/opportunity/mod.rs"

    require_absent_file \
        "LEGACY-NEWS-OUTCOME-FILE" \
        "replaced opportunity outcome module still exists" \
        "$src/opportunity/news_outcome.rs"
}

run_all_checks() {
    local root="$1"

    if [ ! -d "$root/src" ]; then
        fail "repository source directory is absent: $root/src"
        return
    fi
    check_audit_root_contract "$root"
    check_outcome_capability_contract "$root"
    check_legacy_cutover_contract "$root"
}

write_good_self_test_fixture() {
    local root="$1"

    mkdir -p \
        "$root/src/selection" \
        "$root/src/data_gateway" \
        "$root/src/database" \
        "$root/src/bin" \
        "$root/src/opportunity"
    printf '%s\n' \
        'pub struct SelectionAuditWriter;' \
        'impl SelectionAuditWriter {' \
        '    pub fn production() -> Self {' \
        '        let _root = env!("CARGO_MANIFEST_DIR");' \
        '        let _namespace = "data/audit/production";' \
        '        Self' \
        '    }' \
        '}' > "$root/src/selection/audit.rs"
    printf '%s\n' \
        'pub struct AdmittedOutcomeDailyBars { records: Vec<u8> }' \
        'impl AdmittedOutcomeDailyBars {' \
        '    pub fn records(&self) -> &[u8] { &self.records }' \
        '}' \
        'pub struct VerifiedOutcomeDue { sample_key: String }' \
        'impl VerifiedOutcomeDue {' \
        '    pub fn sample_key(&self) -> &str { &self.sample_key }' \
        '}' > "$root/src/data_gateway/outcome_capabilities.rs"
    printf '%s\n' 'pub fn retained_module() {}' > "$root/src/opportunity/mod.rs"
    printf '%s\n' 'pub fn exact_binding_only() {}' > "$root/src/opportunity/chain_mapper.rs"
}

run_self_test() {
    local tmp
    tmp="$(mktemp -d "${TMPDIR:-/tmp}/br174-compliance-self-test.XXXXXX")"
    SELF_TEST_TMP="$tmp"
    trap cleanup_self_test EXIT

    local good="$tmp/good"
    write_good_self_test_fixture "$good"
    FAILURES=0
    run_all_checks "$good"
    if [ "$FAILURES" -ne 0 ]; then
        echo "[check_br174_legacy_callers] SELF-TEST FAIL: good fixture rejected" >&2
        exit 1
    fi

    printf '%s\n' \
        'pub enum SelectionAuditEnvironment { Production }' \
        'fn bad(root: &str) {' \
        '    let _ = SelectionAuditWriter::for_environment(root, SelectionAuditEnvironment::Production);' \
        '    let _ = std::path::PathBuf::from("data/audit/production");' \
        '}' >> "$good/src/selection/audit.rs"
    FAILURES=0
    check_audit_root_contract "$good" >/dev/null 2>&1
    if [ "$FAILURES" -eq 0 ]; then
        echo "[check_br174_legacy_callers] SELF-TEST FAIL: audit violations escaped" >&2
        exit 1
    fi

    write_good_self_test_fixture "$good"
    printf '%s\n' \
        'pub enum CompletedSessionTerminal { Settled { outcome: u8, evidence: u8 } }' \
        'fn bad(v: CompletedSessionTerminal) {' \
        '    let _ = CompletedSessionTerminal::Settled { outcome: 1, evidence: 2 };' \
        '    let _ = v;' \
        '}' >> "$good/src/data_gateway/outcome_capabilities.rs"
    FAILURES=0
    check_outcome_capability_contract "$good" >/dev/null 2>&1
    if [ "$FAILURES" -eq 0 ]; then
        echo "[check_br174_legacy_callers] SELF-TEST FAIL: outcome violations escaped" >&2
        exit 1
    fi

    write_good_self_test_fixture "$good"
    printf '%s\n' \
        'pub async fn tick_news_aggregator_batch() {}' \
        'pub async fn evaluate_news_batch() {}' >> "$good/src/bin/legacy.rs"
    printf '%s\n' \
        'pub fn backfill_recommendations_outcome() {}' > "$good/src/opportunity/news_outcome.rs"
    FAILURES=0
    check_legacy_cutover_contract "$good" >/dev/null 2>&1
    if [ "$FAILURES" -eq 0 ]; then
        echo "[check_br174_legacy_callers] SELF-TEST FAIL: legacy violations escaped" >&2
        exit 1
    fi

    echo "[check_br174_legacy_callers] SELF-TEST PASS"
}

if ! command -v rg >/dev/null 2>&1; then
    echo "[check_br174_legacy_callers] FAIL: rg is required" >&2
    exit 1
fi

case "${1:-}" in
    "")
        run_all_checks "$REPO_ROOT"
        ;;
    "--self-test")
        run_self_test
        exit 0
        ;;
    *)
        echo "usage: $0 [--self-test]" >&2
        exit 2
        ;;
esac

if [ "$FAILURES" -ne 0 ]; then
    echo "[check_br174_legacy_callers] $FAILURES blocking violation(s)" >&2
    exit 1
fi

echo "[check_br174_legacy_callers] PASS"
