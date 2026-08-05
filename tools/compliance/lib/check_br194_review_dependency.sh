#!/usr/bin/env bash
# BR-194 / BR-199 / BR-200: review dependency and R-08 SourceOnly mutation gate.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"

python3 - "$REPO_ROOT" <<'PY'
from pathlib import Path
import sys

root = Path(sys.argv[1])
main = (root / "src/bin/monitor/main.rs").read_text()
review = (root / "src/bin/monitor/review_batch.rs").read_text()
push = (root / "src/bin/monitor/push_templates.rs").read_text()
notify = (root / "src/bin/monitor/notify.rs").read_text()
v14 = (root / "src/bin/monitor/v14_adapter.rs").read_text()
runtime = (root / "src/bin/monitor/durable_delivery_runtime.rs").read_text()
coordinator = (root / "src/durable_delivery/coordinator.rs").read_text()
schema = (root / "src/durable_delivery/schema.rs").read_text()
calendar = (root / "src/calendar.rs").read_text()
exchange_calendar_authority = (
    root / "src/data_gateway/exchange_calendar_authority.rs"
).read_text()
cargo = (root / "Cargo.toml").read_text()
process_tests = (root / "tests/monitor_help_isolation.rs").read_text()
verifier = (root / "tools/release/verify_br194_review_join.py").read_text()

failed_replay_reasons = (
    "terminal_replay_identity_invalid",
    "terminal_replay_not_delivered",
    "terminal_replay_hydration_not_applied",
    "terminal_replay_would_require_sink",
    "terminal_replay_watermark_changed",
    "terminal_replay_evidence_unavailable",
)

def function_body(source: str, marker: str) -> str:
    start = source.find(marker)
    if start < 0:
        raise AssertionError(f"missing function marker {marker}")
    brace = source.find("{", start)
    if brace < 0:
        raise AssertionError(f"missing function body {marker}")
    depth = 0
    state = "code"
    escaped = False
    index = brace
    while index < len(source):
        char = source[index]
        nxt = source[index + 1] if index + 1 < len(source) else ""
        if state == "line_comment":
            if char == "\n":
                state = "code"
        elif state == "block_comment":
            if char == "*" and nxt == "/":
                state = "code"
                index += 1
        elif state in ("string", "char"):
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif (state == "string" and char == '"') or (state == "char" and char == "'"):
                state = "code"
        elif char == "/" and nxt == "/":
            state = "line_comment"
            index += 1
        elif char == "/" and nxt == "*":
            state = "block_comment"
            index += 1
        elif char == '"':
            state = "string"
        elif char == "'" and (
            (index + 2 < len(source) and source[index + 2] == "'")
            or (nxt == "\\" and index + 3 < len(source) and source[index + 3] == "'")
        ):
            state = "char"
        elif char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return source[start:index + 1]
        index += 1
    raise AssertionError(f"unterminated function body {marker}")

callers = [
    function_body(main, "async fn run_review_only()"),
    function_body(main, "async fn attempt_post_session_review("),
    function_body(main, "async fn run_strict_review_only_inner("),
]
dispatcher = function_body(push, "pub async fn dispatch_post_session_review(")
r04 = function_body(push, "pub async fn dispatch_r04_lhb_outcome(")
r04_loader = function_body(push, "async fn dispatch_r04_lhb_outcome_with_loader")
r04_producer = function_body(push, "fn prepare_review_lhb_delivery(")
r08 = function_body(push, "pub async fn dispatch_r08_event_calendar_outcome(")
r08_loader = function_body(push, "async fn dispatch_r08_event_calendar_outcome_with_loader")
r08_path = r08 + r08_loader
source_entry = function_body(notify, "pub async fn push_counted_source_only_with_binding(")
r08_presented_entry = function_body(
    notify, "pub async fn push_r08_presented_source_only_with_binding("
)
r08_source_entry = function_body(notify, "async fn push_r08_source_only_with_binding(")
source_orchestrator = function_body(
    notify, "async fn push_counted_source_only_after_validation_with"
)
source_gate = function_body(v14, "pub fn v14_gate_counted_source_only_binding(")
r08_source_gate = function_body(v14, "pub fn v14_gate_r08_source_only_binding(")
source_profile = function_body(v14, "fn counted_source_only_profile(")
prepared_gate = function_body(v14, "fn v14_gate_prepared(")
source_context = function_body(v14, "fn current_counted_source_only_governance_ctx()")
replay_runner = function_body(runtime, "pub fn run_production_audited_terminal_replay(")
replay_orchestrator = function_body(runtime, "fn run_audited_terminal_replay_with")
replay_classifier = function_body(runtime, "pub fn replay_terminal_envelope(")
replay_reason_validator = function_body(coordinator, "fn validate_replay_reason_code(")
replay_cli_parser = function_body(main, "fn parse_br194_terminal_replay_command(")
source_validator = function_body(runtime, "pub fn validate_r04_source_only(&self)")
source_canonical_validator = function_body(
    push, "pub(super) fn validate_review_lhb_source_binding_canonical_bytes("
)
source_projection_validator = function_body(runtime, "fn r04_projection_is_exact(")
source_text_validator = function_body(
    runtime, "pub fn validate_r04_source_only_text(&self"
)
r08_source_validator = function_body(
    runtime, "pub fn validate_r08_public_source_only(&self)"
)
r08_source_text_validator = function_body(
    runtime, "pub fn validate_r08_public_source_only_text("
)
r08_canonical_validator = function_body(
    push, "pub(super) fn validate_r08_public_source_binding_canonical_bytes("
)
envelope_builder = function_body(runtime, "fn envelope_from_binding(")
r08_dependency = function_body(review, "pub fn dependency(self)")

for body in callers + [dispatcher]:
    for forbidden in ("BannerCtx", "current_banner()", "evaluate_account_mode_hook(true)"):
        assert forbidden not in body, f"review pre-gate restored: {forbidden}"

assert "Self::R04 | Self::R08 | Self::R09 | Self::A10 | Self::A01 =>" in r08_dependency
assert "ReviewTaskDependency::SourceOnly" in r08_dependency

assert dispatcher.find("let preflight = review_preflight") < dispatcher.find(
    "let phases = partition_review_tasks"
)
assert dispatcher.find("let phases = partition_review_tasks") < dispatcher.find("tokio::join!"), (
    dispatcher.find("let phases = partition_review_tasks"),
    dispatcher.find("tokio::join!"),
    dispatcher[-160:],
)
assert dispatcher.find("tokio::join!") < dispatcher.find(
    "let account_required = account_dependency_outcomes"
)
assert dispatcher.find("let account_required = account_dependency_outcomes") < dispatcher.rfind(
    "merge_review_task_outcomes"
)
for source_only_task in (
    "ReviewTask::R04",
    "ReviewTask::R08",
    "ReviewTask::R09",
    "ReviewTask::A10",
    "ReviewTask::A01",
):
    assert source_only_task in dispatcher
for required in (
    "dispatch_catalyst_review_daily_outcome",
    "dispatch_paper_review_daily_outcome",
):
    assert required in dispatcher, f"source-only provider missing: {required}"
for forbidden in ("dispatch_r03_industry_chain_outcome",):
    assert forbidden not in dispatcher, f"conservative provider restored: {forbidden}"

for static_task in ("Self::R02", "Self::R05", "Self::R06"):
    assert static_task in review
assert "test_environment_external_provider_blocked" in review
assert "duplicate review task outcome" in review
assert "tasks.sort_by_key" in review
assert "context.review_date() > current_date" in review
assert "context.review_date() == current_date" in review
assert "provider_top_n_future_date" in review
assert "provider_top_n_current_date_only" not in review
for name in (
    "br198_r09_closed_day_uses_prior_review_trading_date",
    "br198_r09_future_review_date_fails_nonretryable_before_provider",
):
    assert review.count(f"fn {name}(") == 1, f"expected one BR-198 test {name}"

assert "BannerCtx" not in r04 and "BannerCtx" not in r04_loader
assert "push_counted_source_only_with_binding" in r04_loader
assert "push_counted_with_binding" not in r04_loader
for forbidden in (
    "BannerCtx",
    "load_user_confirmed_r08_positions",
    "event_calendar_virtual_holdings",
    "broker_holdings",
    "push_counted_with_binding(",
):
    assert forbidden not in r08_path, f"R-08 public dispatcher restored forbidden dependency: {forbidden}"
for required in (
    "dispatch_r08_event_calendar_outcome_with_loader",
    "inspect_r08_review_occurrence",
):
    assert required in r08, f"R-08 public dispatcher lost BR-200 preflight route: {required}"
for required in (
    "prepare_r08_counted_delivery",
    "CountedDeliveryOrigin::Provider",
    "push_r08_presented_source_only_with_binding",
):
    assert required in r08_loader, f"R-08 public dispatcher lost closed route: {required}"
assert r08_presented_entry.find("token.descriptor().push_kind") < r08_presented_entry.find(
    "push_r08_source_only_with_binding"
)
assert "PushKind::EventCalendar" in r08_presented_entry
assert r08_source_entry.find("validate_r08_public_source_only_text") < r08_source_entry.find(
    "push_counted_source_only_after_validation_with"
)
assert "PushKind::EventCalendar" in r08_source_entry
assert "PushKind::EventCalendar" in r08_source_gate
assert "validate_r08_public_source_only" in r08_source_gate
assert "counted_source_only_profile(kind)" in r08_source_gate
for required in (
    "validate_r08_public_source_binding_canonical_bytes",
    "transition_basis_canonical",
    "ordered_batch_ids",
    "max_observed_at",
):
    assert required in r08_source_validator, f"R-08 durable validator lost: {required}"
assert "sha256_hex(text.as_bytes())" in r08_source_text_validator
for required in ("serde_json::from_slice", "serde_json::to_vec", "expected != canonical"):
    assert required in r08_canonical_validator, f"R-08 byte-exact validator lost: {required}"
assert source_entry.find("is_counted_kind") < source_entry.find(
    "validate_r04_source_only_text"
)
assert source_entry.find("validate_r04_source_only_text") < source_entry.find(
    "push_counted_source_only_after_validation_with"
)
assert source_orchestrator.find("if !launch(kind)") < source_orchestrator.find(
    "match gate(kind, &binding)"
)
assert source_orchestrator.find("match gate(kind, &binding)") < source_orchestrator.find(
    "deliver(binding, kind, text.to_owned())"
)
assert "PushKind::ReviewLhb" in source_gate
assert "validate_r04_source_only" in source_gate
assert "counted_source_only_profile(kind)" in source_gate
assert "DataMode::Down" in source_profile
assert "always_send_on_data_source_down = false" in source_profile
assert "GovernanceContextSource::CountedSourceOnly" in prepared_gate
for forbidden in ("LATEST_BANNER", "current_governance_ctx()", "current_source_fact_governance_ctx()"):
    assert forbidden not in source_context, f"SourceOnly context reads forbidden authority: {forbidden}"
for required in ("current_data_health_input", "evaluate", "is_frozen: false"):
    assert required in source_context, f"SourceOnly context lost governance: {required}"

assert main.find("parse_br194_terminal_replay_command") < main.find(
    "bootstrap_selection_process"
)
assert "BR194_REPLAY_AUTHORITY_MANIFEST_V1" in runtime
assert "AuthoritativeSink" not in replay_classifier
assert ".prepare(&input.envelope, 1, now)" in replay_classifier
assert "run_audited_terminal_replay_with" in replay_runner
assert replay_orchestrator.find("append_review_terminal_replay_audit") < replay_orchestrator.find(
    "match classify"
)
assert replay_orchestrator.find("match classify") < replay_orchestrator.find(
    "finish_review_terminal_replay"
)
assert replay_orchestrator.find("finish_review_terminal_replay") < replay_orchestrator.rfind(
    "review_terminal_replay_audit_appended"
)
for required in (
    "BR-194-terminal-replay-attempt-v1",
    "delivery-critical-audit-v1",
    "ReviewTerminalReplayStarted",
    "ReviewTerminalReplayCompleted",
    "SELECT COALESCE(MAX(replay_ordinal),0)+1",
):
    assert required in coordinator or required in runtime, f"missing replay authority {required}"
assert coordinator.count('"ReviewTerminalReplayStarted"') >= 3
assert coordinator.count('"ReviewTerminalReplayCompleted"') >= 3
failed_reason_branch = replay_reason_validator[
    replay_reason_validator.find("ReviewTerminalReplayCompletionState::Failed => matches!("):
    replay_reason_validator.find("),\n    };")
]
for reason in failed_replay_reasons:
    assert reason in failed_reason_branch, f"missing frozen replay reason {reason}"
assert "terminal_replay_classification_failed" not in failed_reason_branch

for required in (
    "FOREIGN KEY(attempt_identity,decision_identity)",
    "REFERENCES immutable_audit_outbox(audit_identity)",
    "validate_review_terminal_replay_attempt_audit_insert",
    "validate_review_terminal_replay_completion_audit_insert",
    "immutable_review_terminal_replay_attempt_update",
    "immutable_review_terminal_replay_attempt_delete",
    "immutable_review_terminal_replay_completion_update",
    "immutable_review_terminal_replay_completion_delete",
):
    assert required in schema, f"missing replay schema authority {required}"
for required in (
    "SCHEMA_VERSION: i64 = 6",
    "migrate_schema_v5_to_v6",
    "register_sha256_function",
    "FunctionFlags::SQLITE_INNOCUOUS",
    "FROM pragma_function_list",
    "sha256_hex(NEW.start_canonical)=NEW.start_sha256",
    "sha256_hex(audit.audit_canonical)=audit.audit_sha256",
    "sha256_hex(NEW.completion_canonical)=NEW.completion_sha256",
    "sha256_hex(audit.audit_canonical)=audit.audit_sha256",
):
    assert required in schema, f"missing v5 replay hash authority {required}"
assert schema.count("FunctionFlags::SQLITE_INNOCUOUS") >= 2
assert '"functions"' in cargo, "rusqlite scalar-function support is missing"
for required in (
    "verified_a_share_trading_day",
    "VERIFIED_TRADING_CALENDAR",
    "coverage_year",
    "OFFICIAL_SSE_AUTHORITY_ROOT",
    "validate_canonical_sse_announcement_url",
):
    assert required in calendar, f"missing immutable trading-calendar authority {required}"
for required in (
    'pub const OFFICIAL_SSE_AUTHORITY_ROOT: &str = "https://www.sse.com.cn/"',
    'OfficialAshareExchange::Sse => ["www.sse.com.cn", "sse.com.cn"]',
):
    assert required in exchange_calendar_authority, (
        f"missing gateway-owned immutable trading-calendar authority {required}"
    )
for required in (
    "--business-date must be specified exactly once",
    "--task must be specified exactly once",
    "verified_a_share_trading_day",
):
    assert required in replay_cli_parser, f"terminal replay CLI weakened: {required}"
assert "validate_review_lhb_source_binding_canonical_bytes" in source_validator
for required in ("serde_json::from_slice", "serde_json::to_vec", "expected != canonical"):
    assert required in source_canonical_validator, (
        f"R-04 byte-exact canonical validator lost: {required}"
    )
assert "terminal_replay_classification_failed" not in replay_orchestrator
assert (
    'ReviewTerminalReplayCompletionState::Failed,\n'
    '                "terminal_replay_evidence_unavailable"'
) in replay_orchestrator
assert "dispatch_all_for_test" not in push
assert "dispatch_r04_lhb_real" not in push

for required in (
    "ProviderId::Eastmoney",
    "disclosure.seats.len() != 10",
    "let mut buy_ranks = [false; 5]",
    "let mut sell_ranks = [false; 5]",
    "source_order_ordinal",
    "rendered_content_sha256: r04_sha256(rendered.as_bytes())",
):
    assert required in r04_producer, f"R-04 producer contract lost: {required}"
for required in (
    '"schema_version"',
    '"business_date"',
    '"template_id"',
    '"review_task_identity"',
    '"delivery_subject_identity"',
    '"evidence"',
    '"ordered_projection"',
    '"rendered_content_sha256"',
    '"task_transition_basis"',
    '"provider"',
    '"Eastmoney"',
    '"source_at"',
    "r04_object_has_exact_keys",
    "r04_projection_is_exact",
):
    assert required in source_validator, f"R-04 runtime validator lost: {required}"
for required in (
    '"source_order_ordinal"',
    '"disclosures"',
    '"seats"',
    "seats.len() == 10",
    "let mut buy_ranks = [false; 5]",
    "let mut sell_ranks = [false; 5]",
):
    assert required in source_projection_validator, (
        f"R-04 projection validator lost: {required}"
    )
assert "sha256_hex(text.as_bytes())" in source_text_validator
assert "validate_r04_source_only_text(text)" in envelope_builder
assert "validate_r08_public_source_only_text(text)" in envelope_builder
for required in (
    "EXPECTED_REPLAY_COLUMNS",
    "EXPECTED_REPLAY_TRIGGER_SQL",
    "normalize_schema_sql",
    "PRAGMA table_info",
    "if actual != expected",
    "replay table CHECK/UNIQUE contract mismatch",
    "EXPECTED_SCHEMA_VERSION = 6",
    "PRAGMA user_version",
    "sha256_hex(NEW.start_canonical)=NEW.start_sha256",
    "sha256_hex(NEW.completion_canonical)=NEW.completion_sha256",
    "EXPECTED_PASSED_REPLAY_REASON",
    "EXPECTED_FAILED_REPLAY_REASONS",
    "verify_replay_completion_reason_vocabulary",
    "out-of-contract replay completion reason",
):
    assert required in verifier, f"BR-194 verifier weakened: {required}"
for reason in failed_replay_reasons:
    assert reason in verifier, f"verifier lost frozen replay reason {reason}"
assert "terminal_replay_classification_failed" not in verifier
assert verifier.find("verify_schema(connection)") < verifier.find(
    "verify_replay_completion_reason_vocabulary(connection)"
)
assert verifier.find("verify_replay_completion_reason_vocabulary(connection)") < verifier.find(
    "expected_task_identity = review_task_identity"
)

required_tests = [
    "br194_review_task_dependency_mapping",
    "br194_preflight_precedes_dependency_acquisition",
    "br194_source_only_runs_before_frozen_account_tasks",
    "br194_account_tasks_are_frozen_without_real_batch_watermark",
    "br194_account_failure_serializes_exact_transition_audit",
    "br194_legacy_transition_fixture_remains_byte_identical_and_hash_valid",
    "br194_account_failure_full_record_fixture_is_fixed_and_hash_valid",
    "br194_transition_failure_wire_rejects_null_array_unknown_and_nonfailed_payloads",
    "br194_review_batch_merge_rejects_duplicate_task",
    "br194_time_boundaries_1535_and_2100",
    "br194_r04_source_only_gate_never_reads_banner",
    "br194_r04_source_only_preserves_l5_and_durable_entry",
    "br194_r04_source_only_denied_launch_has_zero_durable_and_sink",
    "br197_source_only_profile_uses_component_quality_without_changing_default_profile",
    "br194_r04_runtime_revalidates_exact_canonical_schema_and_rendered_text",
    "br194_r04_runtime_rejects_semantically_equal_noncanonical_bytes",
    "br194_r04_runtime_rejects_schema_provider_projection_and_seat_mutations",
    "br194_r04_envelope_rejects_text_not_bound_by_canonical_hash",
    "br194_terminal_replay_passes_with_equal_authority_watermarks",
    "br194_terminal_replay_sink_eligibility_fails_before_sink",
    "br194_terminal_replay_started_or_failed_cannot_verify",
    "br194_terminal_replay_classification_error_persists_failed_completion",
    "br194_terminal_replay_rejects_out_of_contract_failed_reason",
    "br194_terminal_replay_trigger_recomputes_canonical_sha256",
    "br194_terminal_replay_identity_and_audit_join_are_exact",
    "br194_terminal_replay_audit_uses_none_delivery_attempt_binding",
    "br194_terminal_replay_tables_reject_update_delete_and_second_completion",
    "br194_terminal_replay_rejects_mismatched_completion_decision_and_audit",
    "br194_terminal_replay_start_audit_ack_failure_blocks_classification",
    "br194_terminal_replay_completion_write_or_ack_failure_never_passes",
    "br194_terminal_replay_ordinals_advance_after_dangling_or_failed_attempts",
    "br194_terminal_replay_cross_connection_contention_allocates_unique_ordinals",
    "br199_r08_is_source_only_and_partitions_before_account_gate",
    "br199_r08_dispatcher_has_no_account_or_virtual_reader",
    "br199_r08_friday_targets_monday_trading_session",
    "br199_r08_public_binding_freezes_ordered_gateway_batches_and_source_facts",
    "br199_r08_cffex_is_mandatory_for_counted_delivery",
    "br199_r08_verified_empty_cffex_is_a_complete_public_component",
    "br199_r08_durable_binding_rejects_public_evidence_mutations",
    "br199_r08_closed_gate_rejects_non_event_calendar_kind",
    "br199_r08_dispatch_is_joined_before_account_dependency_outcomes",
]
monitor_sources = main + review + push + notify + runtime + v14
for name in required_tests:
    assert monitor_sources.count(f"fn {name}(") == 1, f"expected one monitor test {name}"
assert "fn br194_schema_v5_migration_matrix_is_repeatable_and_rejects_newer_versions(" in (
    root / "src/durable_delivery/tests.rs"
).read_text()
assert "fn br194_sha256_function_catalog_is_deterministic_innocuous_and_blob_only(" in (
    root / "src/durable_delivery/tests.rs"
).read_text()
assert "fn br194_verified_calendar_is_immutable_fail_closed_and_coverage_bounded(" in calendar
for name in (
    "br194_terminal_replay_cli_rejects_ordinal_override_before_database_open",
    "br194_terminal_replay_cli_rejects_duplicates_and_nontrading_dates_before_database_open",
    "br194_test_review_blocks_all_source_providers_and_sinks_before_account_gate",
):
    count = main.count(f"fn {name}(") + process_tests.count(f"fn {name}(")
    assert count == 1, f"expected one process test {name}"

def validate_review_boundary(body: str) -> None:
    for forbidden in ("BannerCtx", "current_banner()", "evaluate_account_mode_hook(true)"):
        assert forbidden not in body

def validate_dispatcher(body: str) -> None:
    for required in (
        "let preflight = review_preflight",
        "let phases = partition_review_tasks",
        "tokio::join!",
        "let account_required = account_dependency_outcomes",
        "merge_review_task_outcomes",
        "dispatch_r08_event_calendar_outcome",
        "dispatch_catalyst_review_daily_outcome",
        "dispatch_paper_review_daily_outcome",
    ):
        assert required in body
    assert body.find("let preflight = review_preflight") < body.find(
        "let phases = partition_review_tasks"
    )
    assert body.find("let phases = partition_review_tasks") < body.find("tokio::join!")
    assert body.find("tokio::join!") < body.find(
        "let account_required = account_dependency_outcomes"
    )
    assert body.find("let account_required = account_dependency_outcomes") < body.rfind(
        "merge_review_task_outcomes"
    )
    for forbidden in ("dispatch_r03_industry_chain_outcome",):
        assert forbidden not in body

def validate_r08_dependency(body: str) -> None:
    assert "Self::R04 | Self::R08 | Self::R09 | Self::A10 | Self::A01 =>" in body
    assert "ReviewTaskDependency::SourceOnly" in body

def validate_r08_dispatcher(body: str) -> None:
    for forbidden in (
        "BannerCtx",
        "load_user_confirmed_r08_positions",
        "event_calendar_virtual_holdings",
        "broker_holdings",
        "push_counted_with_binding(",
    ):
        assert forbidden not in body
    for required in (
        "prepare_r08_counted_delivery",
        "CountedDeliveryOrigin::Provider",
        "push_r08_presented_source_only_with_binding",
    ):
        assert required in body

def validate_r08_presented_entry(body: str) -> None:
    for required in (
        "token.descriptor().push_kind",
        "PushKind::EventCalendar",
        "presentation_token_kind_mismatch",
        "push_r08_source_only_with_binding",
    ):
        assert required in body
    assert body.find("token.descriptor().push_kind") < body.find(
        "push_r08_source_only_with_binding"
    )

def validate_r08_source_entry(body: str) -> None:
    for required in (
        "let kind = PushKind::EventCalendar",
        "validate_r08_public_source_only_text",
        "push_counted_source_only_after_validation_with",
        "v14_gate_r08_source_only_binding",
    ):
        assert required in body
    assert body.find("validate_r08_public_source_only_text") < body.find(
        "push_counted_source_only_after_validation_with"
    )

def validate_r08_source_gate(body: str) -> None:
    for required in (
        "PushKind::EventCalendar",
        "validate_r08_public_source_only",
        "counted_source_only_profile(kind)",
        "GovernanceContextSource::CountedSourceOnly",
    ):
        assert required in body
    assert "CountedCombinedAccount" not in body

def validate_r08_source_validator(body: str) -> None:
    for required in (
        "validate_r08_public_source_binding_canonical_bytes",
        "transition_basis_canonical",
        "ordered_batch_ids",
        "max_observed_at",
        "CountedDeliveryOrigin::Provider",
    ):
        assert required in body
    assert body.count("ordered_batch_ids") >= 3

def validate_r08_canonical_validator(body: str) -> None:
    for required in ("serde_json::from_slice", "serde_json::to_vec", "expected != canonical"):
        assert required in body

def validate_r04_loader(body: str) -> None:
    assert "BannerCtx" not in body
    assert "push_counted_source_only_with_binding" in body
    assert "push_counted_with_binding" not in body

def validate_r04_producer(body: str) -> None:
    for required in (
        "ProviderId::Eastmoney",
        "disclosure.seats.len() != 10",
        "let mut buy_ranks = [false; 5]",
        "let mut sell_ranks = [false; 5]",
        "source_order_ordinal",
        "rendered_content_sha256: r04_sha256(rendered.as_bytes())",
    ):
        assert required in body

def validate_source_entry(body: str) -> None:
    for required in (
        "is_counted_kind",
        "validate_r04_source_only_text",
        "push_counted_source_only_after_validation_with",
    ):
        assert required in body
    assert body.find("is_counted_kind") < body.find(
        "validate_r04_source_only_text"
    )
    assert body.find("validate_r04_source_only_text") < body.find(
        "push_counted_source_only_after_validation_with"
    )

def validate_source_orchestrator(body: str) -> None:
    for required in (
        "if !launch(kind)",
        "match gate(kind, &binding)",
        "deliver(binding, kind, text.to_owned())",
    ):
        assert required in body
    assert body.find("if !launch(kind)") < body.find("match gate(kind, &binding)")
    assert body.find("match gate(kind, &binding)") < body.find(
        "deliver(binding, kind, text.to_owned())"
    )

def validate_source_gate(body: str) -> None:
    assert "PushKind::ReviewLhb" in body
    assert "validate_r04_source_only" in body
    assert "counted_source_only_profile(kind)" in body
    assert "LATEST_BANNER" not in body
    assert "current_source_fact_governance_ctx" not in body

def validate_source_profile(body: str) -> None:
    assert "BR-197" in body
    assert "DataMode::Down" in body
    assert "always_send_on_data_source_down = false" in body

def validate_source_validator(body: str) -> None:
    for required in (
        '"schema_version"',
        '"provider"',
        '"Eastmoney"',
        '"source_at"',
        '"ordered_projection"',
        '"rendered_content_sha256"',
        "r04_object_has_exact_keys",
        "r04_projection_is_exact",
    ):
        assert required in body
    for repeated in (
        '"schema_version"',
        '"provider"',
        '"source_at"',
        '"ordered_projection"',
        '"rendered_content_sha256"',
    ):
        assert body.count(repeated) >= 2
    assert "validate_review_lhb_source_binding_canonical_bytes" in body

def validate_source_canonical_validator(body: str) -> None:
    for required in ("serde_json::from_slice", "serde_json::to_vec", "expected != canonical"):
        assert required in body

def validate_source_projection_validator(body: str) -> None:
    for required in (
        '"source_order_ordinal"',
        '"disclosures"',
        '"seats"',
        "seats.len() == 10",
        "let mut buy_ranks = [false; 5]",
        "let mut sell_ranks = [false; 5]",
    ):
        assert required in body
    for repeated in ('"source_order_ordinal"', '"disclosures"', '"seats"'):
        assert body.count(repeated) >= 2

def validate_source_text_validator(body: str) -> None:
    assert "validate_r04_source_only()" in body
    assert "sha256_hex(text.as_bytes())" in body
    assert '"rendered_content_sha256"' in body

def validate_envelope_builder(body: str) -> None:
    assert "PushKind::ReviewLhb" in body
    assert "validate_r04_source_only_text(text)" in body
    assert "PushKind::EventCalendar" in body
    assert "validate_r08_public_source_only_text(text)" in body

def validate_source_context(body: str) -> None:
    for forbidden in (
        "LATEST_BANNER",
        "current_governance_ctx()",
        "current_source_fact_governance_ctx()",
    ):
        assert forbidden not in body
    for required in ("current_data_health_input", "evaluate", "is_frozen: false"):
        assert required in body

def validate_replay_runner(body: str) -> None:
    assert "run_audited_terminal_replay_with" in body
    assert "replay_terminal_envelope" in body

def validate_replay_orchestrator(body: str) -> None:
    assert body.count("append_review_terminal_replay_audit") >= 2
    assert body.count("finish_review_terminal_replay") >= 2
    assert body.count("review_terminal_replay_audit_appended") >= 2
    assert body.find("append_review_terminal_replay_audit") < body.find(
        "match classify"
    )
    assert body.find("match classify") < body.find(
        "finish_review_terminal_replay"
    )
    assert body.find("finish_review_terminal_replay") < body.rfind(
        "review_terminal_replay_audit_appended"
    )
    assert "finish_review_terminal_replay" in body
    assert "ReviewTerminalReplayCompletionState::Passed" in body
    assert "terminal_replay_classification_failed" not in body
    assert (
        'ReviewTerminalReplayCompletionState::Failed,\n'
        '                "terminal_replay_evidence_unavailable"'
    ) in body

def validate_replay_classifier(body: str) -> None:
    assert "AuthoritativeSink" not in body
    assert ".prepare(&input.envelope, 1, now)" in body
    assert "DecisionState::Delivered" in body
    assert "ScheduleHydrationState::Applied" in body

def validate_schema(body: str) -> None:
    for required in (
        "FOREIGN KEY(attempt_identity,decision_identity)",
        "REFERENCES immutable_audit_outbox(audit_identity)",
        "validate_review_terminal_replay_attempt_audit_insert",
        "validate_review_terminal_replay_completion_audit_insert",
        "immutable_review_terminal_replay_attempt_update",
        "immutable_review_terminal_replay_attempt_delete",
        "immutable_review_terminal_replay_completion_update",
        "immutable_review_terminal_replay_completion_delete",
    ):
        assert required in body
    assert body.count("validate_review_terminal_replay_attempt_audit_insert") >= 2
    assert body.count("validate_review_terminal_replay_completion_audit_insert") >= 2
    for required in (
        "SCHEMA_VERSION: i64 = 6",
        "migrate_schema_v5_to_v6",
        "register_sha256_function",
        "FunctionFlags::SQLITE_INNOCUOUS",
        "FROM pragma_function_list",
        "sha256_hex(NEW.start_canonical)=NEW.start_sha256",
        "sha256_hex(audit.audit_canonical)=audit.audit_sha256",
        "sha256_hex(NEW.completion_canonical)=NEW.completion_sha256",
        "sha256_hex(audit.audit_canonical)=audit.audit_sha256",
    ):
        assert required in body
    assert body.count("FunctionFlags::SQLITE_INNOCUOUS") >= 2

def validate_replay_cli_parser(body: str) -> None:
    for required in (
        "--business-date must be specified exactly once",
        "--task must be specified exactly once",
        "verified_a_share_trading_day",
    ):
        assert required in body

def validate_replay_reason_validator(body: str) -> None:
    failed_branch = body[
        body.find("ReviewTerminalReplayCompletionState::Failed => matches!("):
        body.find("),\n    };")
    ]
    for reason in failed_replay_reasons:
        assert reason in failed_branch
    assert "terminal_replay_classification_failed" not in failed_branch

def validate_calendar(body: str) -> None:
    for required in (
        "verified_a_share_trading_day",
        "VERIFIED_TRADING_CALENDAR",
        "static VERIFIED_TRADING_CALENDAR: Lazy<Result<VerifiedTradingCalendar, String>>",
        "let calendar = VERIFIED_TRADING_CALENDAR",
        "VERIFIED_TRADING_CALENDAR_AUTHORITY_ORIGIN",
        "coverage_year",
        "OFFICIAL_SSE_AUTHORITY_ROOT",
        "validate_canonical_sse_announcement_url",
    ):
        assert required in body
    assert body.count("VERIFIED_TRADING_CALENDAR") >= 2

def validate_coordinator(body: str) -> None:
    for required in (
        "BR-194-terminal-replay-attempt-v1",
        "SELECT COALESCE(MAX(replay_ordinal),0)+1",
        "with_immediate_transaction",
    ):
        assert required in body
    assert body.count('"ReviewTerminalReplayStarted"') >= 3
    assert body.count('"ReviewTerminalReplayCompleted"') >= 3

def validate_verifier(body: str) -> None:
    for required in (
        "EXPECTED_REPLAY_COLUMNS",
        "EXPECTED_REPLAY_TRIGGER_SQL",
        "normalize_schema_sql",
        "PRAGMA table_info",
        "if actual != expected",
        "replay table CHECK/UNIQUE contract mismatch",
        "EXPECTED_SCHEMA_VERSION = 6",
        "PRAGMA user_version",
        "sha256_hex(NEW.start_canonical)=NEW.start_sha256",
        "sha256_hex(NEW.completion_canonical)=NEW.completion_sha256",
        "EXPECTED_PASSED_REPLAY_REASON",
        "EXPECTED_FAILED_REPLAY_REASONS",
        "verify_replay_completion_reason_vocabulary",
        "out-of-contract replay completion reason",
    ):
        assert required in body
    for reason in failed_replay_reasons:
        assert reason in body
    assert "terminal_replay_classification_failed" not in body
    assert body.find("verify_schema(connection)") < body.find(
        "verify_replay_completion_reason_vocabulary(connection)"
    )
    assert body.find("verify_replay_completion_reason_vocabulary(connection)") < body.find(
        "expected_task_identity = review_task_identity"
    )
    assert body.count("EXPECTED_REPLAY_COLUMNS") >= 2
    assert body.count("EXPECTED_REPLAY_TRIGGER_SQL") >= 3
    assert body.count("if actual != expected") >= 2

def expect_mutation_detected(
    original: str, needle: str, replacement: str, validator
) -> None:
    assert needle in original, f"mutation target missing: {needle}"
    mutated = original.replace(needle, replacement, 1)
    try:
        validator(mutated)
    except AssertionError:
        return
    raise AssertionError(f"mutation escaped BR-194 checker: {needle} -> {replacement}")

mutations = [
    (callers[0], "async fn run_review_only()", "async fn run_review_only() /* current_banner() */", validate_review_boundary),
    (callers[1], "async fn attempt_post_session_review(", "async fn attempt_post_session_review(/* evaluate_account_mode_hook(true) */", validate_review_boundary),
    (dispatcher, "let preflight = review_preflight", "let preflight = preflight_removed", validate_dispatcher),
    (dispatcher, "merge_review_task_outcomes(preflight.outcomes", "merge_removed(preflight.outcomes", validate_dispatcher),
    (dispatcher, "let account_required = account_dependency_outcomes", "dispatch_r03_industry_chain_outcome(); let account_required = account_dependency_outcomes", validate_dispatcher),
    (dispatcher, "dispatch_r08_event_calendar_outcome", "r08_dispatch_removed", validate_dispatcher),
    (dispatcher, "dispatch_catalyst_review_daily_outcome", "a10_dispatch_removed", validate_dispatcher),
    (dispatcher, "dispatch_paper_review_daily_outcome", "a01_dispatch_removed", validate_dispatcher),
    (r08_dependency, "Self::R04 | Self::R08 | Self::R09 | Self::A10 | Self::A01 =>", "Self::R04 | Self::R08 | Self::R09 =>", validate_r08_dependency),
    (r08_loader, "push_r08_presented_source_only_with_binding", "push_counted_with_binding", validate_r08_dispatcher),
    (r08_presented_entry, "token.descriptor().push_kind", "PushKind::EventCalendar", validate_r08_presented_entry),
    (r08_presented_entry, "push_r08_source_only_with_binding", "push_counted_with_binding", validate_r08_presented_entry),
    (r08_source_entry, "let kind = PushKind::EventCalendar", "let kind = PushKind::ReviewLhb", validate_r08_source_entry),
    (r08_source_entry, "validate_r08_public_source_only_text", "r08_text_validation_removed", validate_r08_source_entry),
    (r08_source_gate, "validate_r08_public_source_only", "r08_validation_removed", validate_r08_source_gate),
    (r08_source_gate, "GovernanceContextSource::CountedSourceOnly", "GovernanceContextSource::CountedCombinedAccount", validate_r08_source_gate),
    (r08_source_validator, "CountedDeliveryOrigin::Provider", "CountedDeliveryOrigin::InternalDurable", validate_r08_source_validator),
    (r08_source_validator, "ordered_batch_ids", "batch_ids_removed", validate_r08_source_validator),
    (r08_canonical_validator, "expected != canonical", "false", validate_r08_canonical_validator),
    (r04_loader, "push_counted_source_only_with_binding", "push_counted_with_binding", validate_r04_loader),
    (r04_producer, "ProviderId::Eastmoney", "ProviderId::Unknown", validate_r04_producer),
    (r04_producer, "disclosure.seats.len() != 10", "disclosure.seats.is_empty()", validate_r04_producer),
    (r04_producer, "rendered_content_sha256: r04_sha256(rendered.as_bytes())", "rendered_content_sha256: String::new()", validate_r04_producer),
    (source_entry, "validate_r04_source_only_text", "r04_text_validation_removed", validate_source_entry),
    (source_orchestrator, "if !launch(kind)", "if false", validate_source_orchestrator),
    (source_orchestrator, "match gate(kind, &binding)", "match V14Gate::Approved(br194_approved_event())", validate_source_orchestrator),
    (source_orchestrator, "deliver(binding, kind, text.to_owned())", "PushOutcome::Pushed", validate_source_orchestrator),
    (source_gate, "validate_r04_source_only", "r04_validation_removed", validate_source_gate),
    (source_validator, "validate_review_lhb_source_binding_canonical_bytes", "canonical_bytes_validation_removed", validate_source_validator),
    (source_canonical_validator, "expected != canonical", "false", validate_source_canonical_validator),
    (source_profile, "DataMode::Down", "DataMode::Degraded", validate_source_profile),
    (source_profile, "always_send_on_data_source_down = false", "always_send_on_data_source_down = true", validate_source_profile),
    (source_validator, '"schema_version"', '"schema_REMOVED"', validate_source_validator),
    (source_validator, '"Eastmoney"', '"any-provider"', validate_source_validator),
    (source_validator, '"rendered_content_sha256"', '"rendered_hash_REMOVED"', validate_source_validator),
    (source_projection_validator, '"source_order_ordinal"', '"ordinal_REMOVED"', validate_source_projection_validator),
    (source_projection_validator, '"disclosures"', '"disclosures_REMOVED"', validate_source_projection_validator),
    (source_projection_validator, '"seats"', '"seats_REMOVED"', validate_source_projection_validator),
    (source_projection_validator, "seats.len() == 10", "seats.is_empty()", validate_source_projection_validator),
    (source_text_validator, "sha256_hex(text.as_bytes())", "expected_hash.to_owned()", validate_source_text_validator),
    (envelope_builder, "validate_r04_source_only_text(text)", "validate_r04_source_only()", validate_envelope_builder),
    (envelope_builder, "validate_r08_public_source_only_text(text)", "validate_r08_public_source_only()", validate_envelope_builder),
    (source_context, "current_data_health_input", "current_governance_ctx()", validate_source_context),
    (replay_runner, "run_audited_terminal_replay_with", "run_replay_without_audit", validate_replay_runner),
    (replay_orchestrator, "append_review_terminal_replay_audit", "append_replay_audit_REMOVED", validate_replay_orchestrator),
    (replay_orchestrator, "match classify", "match Ok(TerminalReplayClassification::ExistingTerminalHydrated)", validate_replay_orchestrator),
    (replay_orchestrator, 'ReviewTerminalReplayCompletionState::Failed,\n                "terminal_replay_evidence_unavailable"', 'ReviewTerminalReplayCompletionState::Failed,\n                "classification_error_dangling"', validate_replay_orchestrator),
    (replay_reason_validator, "terminal_replay_evidence_unavailable", "classification_reason_removed", validate_replay_reason_validator),
    (replay_orchestrator, "finish_review_terminal_replay", "finish_replay_without_audit", validate_replay_orchestrator),
    (replay_orchestrator, "review_terminal_replay_audit_appended", "assume_replay_audit_appended", validate_replay_orchestrator),
    (replay_classifier, ".prepare(&input.envelope, 1, now)", ".prepare_REMOVED()", validate_replay_classifier),
    (schema, "FOREIGN KEY(attempt_identity,decision_identity)", "FOREIGN KEY(attempt_identity)", validate_schema),
    (schema, "immutable_review_terminal_replay_attempt_update", "immutable_replay_update_REMOVED", validate_schema),
    (schema, "validate_review_terminal_replay_completion_audit_insert", "validate_replay_completion_REMOVED", validate_schema),
    (schema, "SCHEMA_VERSION: i64 = 6", "SCHEMA_VERSION: i64 = 5", validate_schema),
    (schema, "FunctionFlags::SQLITE_INNOCUOUS", "FunctionFlags::SQLITE_DIRECTONLY", validate_schema),
    (schema, "FROM pragma_function_list", "FROM missing_function_catalog", validate_schema),
    (schema, "sha256_hex(NEW.start_canonical)=NEW.start_sha256", "NEW.start_sha256=NEW.start_sha256", validate_schema),
    (schema, "sha256_hex(NEW.completion_canonical)=NEW.completion_sha256", "NEW.completion_sha256=NEW.completion_sha256", validate_schema),
    (replay_cli_parser, "--business-date must be specified exactly once", "duplicate business date accepted", validate_replay_cli_parser),
    (replay_cli_parser, "verified_a_share_trading_day", "is_trading_day", validate_replay_cli_parser),
    (
        calendar,
        "static VERIFIED_TRADING_CALENDAR: Lazy<Result<VerifiedTradingCalendar, String>>",
        "static HOLIDAYS: Lazy<Result<VerifiedTradingCalendar, String>>",
        validate_calendar,
    ),
    (coordinator, "SELECT COALESCE(MAX(replay_ordinal),0)+1", "SELECT 1", validate_coordinator),
    (verifier, "EXPECTED_REPLAY_COLUMNS", "REPLAY_COLUMNS_REMOVED", validate_verifier),
    (verifier, "EXPECTED_REPLAY_TRIGGER_SQL", "REPLAY_TRIGGER_SQL_REMOVED", validate_verifier),
    (verifier, "if actual != expected", "if False", validate_verifier),
    (verifier, "PRAGMA table_info", "PRAGMA table_xinfo_REMOVED", validate_verifier),
    (verifier, "verify_replay_completion_reason_vocabulary(connection)", "replay_reason_scan_REMOVED(connection)", validate_verifier),
    (verifier, "out-of-contract replay completion reason", "reason accepted", validate_verifier),
]
for original, needle, replacement, validator in mutations:
    expect_mutation_detected(original, needle, replacement, validator)

print("BR-194 review dependency static contract: PASS")
PY
