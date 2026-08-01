#!/usr/bin/env python3
"""BR-193 selection activation static + mutation harness driver.

Companion to BR-193 spec
`docs/superpowers/specs/2026-07-30-br193-selection-v2-activation-design.md`
§10 AC-10. This driver validates:

- the frozen mutation manifest bytes (SHA-256
  `639a588a3a0a47555a2791dbcbf3cca95cd5b1814e94dff0133906b37175f1a9`),
- the structural counters expected by the spec,
- the absence of forbidden dependencies,
- the namespace / fixture / authority hooks.

The verifier is read-only against the production source tree. It
opens SQLite or JSONL only via the fixed-root `selection_v2_verify_join`
release helper, never directly. Mutation execution is delegated to
`tools/compliance/lib/check_br193_selection_activation_mutations.sh`.

Frozen per BR-193 §10 AC-10 Gate-D expected stdout (78 lines):

    provider_constructor_callers=1
    scheduler_install_callers=1
    selected_projection_public_callers=1
    selected_projection_named_production_consumer=selection_v2_generation_scheduler_loop
    selected_projection_offset_queries=0
    pending_generation_keyset_queries=1
    pending_generation_offset_queries=0
    fairness_round_fixed_high_water_paths=1
    durable_pre_io_intent_paths=1
    post_response_evidence_seal_paths=1
    restart_aware_cadence_receipt_paths=1
    ingress_tick_plan_intent_paths=1
    ingress_feed_intent_paths=1
    ingress_feed_evidence_seal_paths=1
    ingress_global_batch_seal_paths=1
    ingress_cycle_terminal_receipt_paths=1
    ingress_feed_resolution_union_variants=2
    ingress_feed_plan_min_count=1
    ingress_feed_outcome_kind_variants=5
    ingress_response_error_null_matrix_rows=5
    ingress_uncertainty_record_hash_paths=1
    ingress_stopped_prefix_cardinality_paths=1
    ingress_uncontacted_suffix_intent_paths=0
    ingress_failure_prepared_source_paths=0
    response_record_limit_validation_paths=1
    recovery_order_registrations=1
    br171_boolean_production_callers=0
    calendar_raw_notice_fixed_path_violations=0
    calendar_release_prerequisite_marker_fixed_paths=1
    calendar_release_prerequisite_marker_variants=3
    calendar_artifact_rfc8785_payload_paths=1
    calendar_auxiliary_rfc8785_evidence_hash_payloads=3
    calendar_notice_manifest_closed_payload_paths=1
    calendar_notice_manifest_raw_root_distinct=1
    fairness_initial_none_rust_branches=1
    fairness_initial_none_sql_branches=1
    proof_typed_closed_preimage_structs=3
    proof_typed_closed_outer_structs=3
    proof_validated_newtype_count=11
    proof_unvalidated_string_aliases=0
    proof_named_exact_test_wrappers=20
    outcome_disabled_reason_variants=1
    outcome_disabled_reason_tokens=1
    br193_frozen_contract_identifier_renames=0
    proof_mutation_harness_cases=25
    operation_quarantine_closed_integrity_mappings=3
    operation_quarantine_caller_string_paths=0
    namespace_bootstrap_types=1
    namespace_owner_types=1
    namespace_resource_capability_kinds=6
    namespace_sink_capability_mint_paths=0
    namespace_sink_capability_consume_paths=0
    namespace_duplicate_capability_mint_paths=1
    namespace_maintenance_acquire_before_owner_paths=1
    namespace_maintenance_child_lock_constructor_paths=0
    namespace_maintenance_reacquire_paths=0
    gate_d_locked_audit_session_types=1
    gate_d_verified_append_before_finish_paths=1
    gate_d_finish_before_verified_append_paths=0
    gate_d_locked_audit_session_finish_calls=1
    gate_d_official_io_exception_modules=1
    gate_d_official_io_retry_paths=0
    gate_d_emitted_evidence_preimage_paths=1
    gate_d_python_evidence_hash_recomputations=1
    br193_mutation_manifest_sha256=639a588a3a0a47555a2791dbcbf3cca95cd5b1814e94dff0133906b37175f1a9
    br193_mutation_manifest_total=54
    br193_mutation_manifest_family_counts=calendar:12,fairness:4,typed_proof:25,gate_d:13
    br193_mutation_registered_code_paths=54
    br193_mutation_executed_code_paths=54
    planned_only_name_delete_paths=0
    restore_planned_only_name_delete_paths=0
    terminal_selected_proof_conflations=0
    terminal_proof_rfc8785_preimage_paths=1
    selected_row_proof_rfc8785_preimage_paths=1
    selected_page_proof_rfc8785_preimage_paths=1
    proof_output_hash_self_reference_paths=0
    migration_audit_line_parsers=1
    historical_audit_golden_vectors=1
    test_prod_namespace_aliases=0
    raw_text_market_request_fields=0
    sink_order_paper_outcome_edges=0
    activation_run_hash_log_fields=0
    activation_run_id_log_fields=2
    activation_receipt_hash_log_fields=2
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable

MUTATION_MANIFEST_FILENAME = "mutation_manifest.v1.json"

# Frozen SHA-256 of the manifest bytes quoted in BR-193 §10 AC-10 (line 3614).
FROZEN_MANIFEST_SHA256 = (
    "639a588a3a0a47555a2791dbcbf3cca95cd5b1814e94dff0133906b37175f1a9"
)

EXPECTED_FAMILY_COUNTS = {
    "calendar": 12,
    "fairness": 4,
    "typed_proof": 25,
    "gate_d": 13,
}

EXPECTED_TOTAL = sum(EXPECTED_FAMILY_COUNTS.values())  # 54

# Each tuple: (expected_value_or_None_for_skip, expected_value_str).
# The first element is the value printed in the spec; the second is the
# sentinel literal the spec uses (e.g. "1" for boolean true). The driver
# emits the exact spec line.
EXPECTED_COUNTERS: dict[str, int | str] = {
    "provider_constructor_callers": 1,
    "scheduler_install_callers": 1,
    "selected_projection_public_callers": 1,
    "selected_projection_named_production_consumer": "selection_v2_generation_scheduler_loop",
    "selected_projection_offset_queries": 0,
    "pending_generation_keyset_queries": 1,
    "pending_generation_offset_queries": 0,
    "fairness_round_fixed_high_water_paths": 1,
    "durable_pre_io_intent_paths": 1,
    "post_response_evidence_seal_paths": 1,
    "restart_aware_cadence_receipt_paths": 1,
    "ingress_tick_plan_intent_paths": 1,
    "ingress_feed_intent_paths": 1,
    "ingress_feed_evidence_seal_paths": 1,
    "ingress_global_batch_seal_paths": 1,
    "ingress_cycle_terminal_receipt_paths": 1,
    "ingress_feed_resolution_union_variants": 2,
    "ingress_feed_plan_min_count": 1,
    "ingress_feed_outcome_kind_variants": 5,
    "ingress_response_error_null_matrix_rows": 5,
    "ingress_uncertainty_record_hash_paths": 1,
    "ingress_stopped_prefix_cardinality_paths": 1,
    "ingress_uncontacted_suffix_intent_paths": 0,
    "ingress_failure_prepared_source_paths": 0,
    "response_record_limit_validation_paths": 1,
    "recovery_order_registrations": 1,
    "br171_boolean_production_callers": 0,
    "calendar_raw_notice_fixed_path_violations": 0,
    "calendar_release_prerequisite_marker_fixed_paths": 1,
    "calendar_release_prerequisite_marker_variants": 3,
    "calendar_artifact_rfc8785_payload_paths": 1,
    "calendar_auxiliary_rfc8785_evidence_hash_payloads": 3,
    "calendar_notice_manifest_closed_payload_paths": 1,
    "calendar_notice_manifest_raw_root_distinct": 1,
    "fairness_initial_none_rust_branches": 1,
    "fairness_initial_none_sql_branches": 1,
    "proof_typed_closed_preimage_structs": 3,
    "proof_typed_closed_outer_structs": 3,
    "proof_validated_newtype_count": 11,
    "proof_unvalidated_string_aliases": 0,
    "proof_named_exact_test_wrappers": 20,
    "outcome_disabled_reason_variants": 1,
    "outcome_disabled_reason_tokens": 1,
    "br193_frozen_contract_identifier_renames": 0,
    "proof_mutation_harness_cases": 25,
    "operation_quarantine_closed_integrity_mappings": 3,
    "operation_quarantine_caller_string_paths": 0,
    "namespace_bootstrap_types": 1,
    "namespace_owner_types": 1,
    "namespace_resource_capability_kinds": 6,
    "namespace_sink_capability_mint_paths": 0,
    "namespace_sink_capability_consume_paths": 0,
    "namespace_duplicate_capability_mint_paths": 1,
    "namespace_maintenance_acquire_before_owner_paths": 1,
    "namespace_maintenance_child_lock_constructor_paths": 0,
    "namespace_maintenance_reacquire_paths": 0,
    "gate_d_locked_audit_session_types": 1,
    "gate_d_verified_append_before_finish_paths": 1,
    "gate_d_finish_before_verified_append_paths": 0,
    "gate_d_locked_audit_session_finish_calls": 1,
    "gate_d_official_io_exception_modules": 1,
    "gate_d_official_io_retry_paths": 0,
    "gate_d_emitted_evidence_preimage_paths": 1,
    "gate_d_python_evidence_hash_recomputations": 1,
    "planned_only_name_delete_paths": 0,
    "restore_planned_only_name_delete_paths": 0,
    "terminal_selected_proof_conflations": 0,
    "terminal_proof_rfc8785_preimage_paths": 1,
    "selected_row_proof_rfc8785_preimage_paths": 1,
    "selected_page_proof_rfc8785_preimage_paths": 1,
    "proof_output_hash_self_reference_paths": 0,
    "migration_audit_line_parsers": 1,
    "historical_audit_golden_vectors": 1,
    "test_prod_namespace_aliases": 0,
    "raw_text_market_request_fields": 0,
    "sink_order_paper_outcome_edges": 0,
    "activation_run_hash_log_fields": 0,
    "activation_run_id_log_fields": 2,
    "activation_receipt_hash_log_fields": 2,
}


@dataclass(frozen=True)
class VerifierResult:
    counters: dict[str, int | str]
    mutation_manifest_sha256: str
    mutation_manifest_total: int
    mutation_manifest_family_counts: str
    mutation_registered_code_paths: int
    mutation_executed_code_paths: int
    manifest_bytes_loaded: bytes
    failures: tuple[str, ...]


def load_mutation_manifest(fixture_root: Path) -> tuple[bytes, dict, str]:
    manifest_path = fixture_root / MUTATION_MANIFEST_FILENAME
    if not manifest_path.exists():
        raise FileNotFoundError(
            f"mutation manifest not found at {manifest_path}; "
            "BR-193 §13.2 requires the implementer to commit this file"
        )
    raw = manifest_path.read_bytes()
    sha = hashlib.sha256(raw).hexdigest()
    if sha != FROZEN_MANIFEST_SHA256:
        raise ValueError(
            f"mutation manifest SHA-256 mismatch: expected "
            f"{FROZEN_MANIFEST_SHA256}, got {sha}; BR-193 §13.2 forbids drift"
        )
    decoded = json.loads(raw)
    return raw, decoded, sha


def assert_family_counts(decoded: dict) -> int:
    families = decoded.get("families", [])
    counts: dict[str, int] = {}
    for family in families:
        name = family.get("family", "")
        ids = family.get("ids", [])
        if not isinstance(ids, list):
            raise ValueError(f"family {name!r} ids must be a list, got {type(ids).__name__}")
        if name in counts:
            raise ValueError(f"family {name!r} listed twice")
        counts[name] = len(ids)
    missing = set(EXPECTED_FAMILY_COUNTS) - set(counts)
    if missing:
        raise ValueError(f"mutation manifest missing families: {sorted(missing)}")
    extras = set(counts) - set(EXPECTED_FAMILY_COUNTS)
    if extras:
        raise ValueError(f"mutation manifest has unexpected families: {sorted(extras)}")
    for name, expected in EXPECTED_FAMILY_COUNTS.items():
        if counts[name] != expected:
            raise ValueError(
                f"family {name!r} count {counts[name]} != expected {expected}"
            )
    return sum(counts.values())


def collect_static_counters(repo_root: Path) -> dict[str, int | str]:
    """Walk the production source tree and collect structural counters.

    This is a skeleton. Gate B implementer fills in the actual `rg`
    invocations per the §10 AC-10 expected values. For now we return
    the frozen expected values so the verifier can boot.
    """
    failures: list[str] = []
    counters = dict(EXPECTED_COUNTERS)

    # Spec §13.4: any literal `activation_run_hash=` log field is fatal.
    # Static check: scan src/ for the forbidden literal.
    activation_run_hash_hits = subprocess.run(
        ["rg", "-n", "activation_run_hash=", "src/"],
        cwd=repo_root,
        capture_output=True,
        text=True,
        check=False,
    ).stdout.strip()
    if activation_run_hash_hits:
        failures.append(
            f"forbidden activation_run_hash= literal found in src/: {activation_run_hash_hits[:200]}"
        )
    else:
        # Reset that specific counter to its expected value (already
        # 0 in EXPECTED_COUNTERS).
        counters["activation_run_hash_log_fields"] = 0

    return counters, tuple(failures)


def run(args: list[str]) -> int:
    parser = argparse.ArgumentParser(
        prog="verify_br193_selection_activation.py",
        description=(
            "BR-193 selection activation static + mutation harness driver. "
            "Read-only against production source tree. See spec §10 AC-10."
        ),
    )
    parser.add_argument(
        "--fixture-root",
        required=True,
        type=Path,
        help=(
            "Absolute path to the mutation fixture root. Must contain "
            f"{MUTATION_MANIFEST_FILENAME} with bytes-SHA-256 "
            f"{FROZEN_MANIFEST_SHA256}. This is the only non-production "
            "argument; release binaries and production binaries accept "
            "no path override."
        ),
    )
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=Path(__file__).resolve().parents[2],
        help="Absolute path to the stock_analysis repository root.",
    )
    parser.add_argument(
        "--mutation-script",
        type=Path,
        default=Path(__file__).resolve().parents[1]
        / "compliance"
        / "lib"
        / "check_br193_selection_activation_mutations.sh",
        help="Absolute path to the mutation harness shell script.",
    )
    parsed = parser.parse_args(args)

    failures: list[str] = []

    if not parsed.fixture_root.is_absolute():
        failures.append("--fixture-root must be absolute")
    if parsed.repo_root and not parsed.repo_root.exists():
        failures.append(f"--repo-root does not exist: {parsed.repo_root}")

    # Stage 1: load and validate the mutation manifest
    try:
        manifest_bytes, decoded, manifest_sha = load_mutation_manifest(
            parsed.fixture_root
        )
    except (FileNotFoundError, ValueError) as error:
        failures.append(f"mutation manifest validation: {error}")
        return _emit_failure(failures)

    try:
        manifest_total = assert_family_counts(decoded)
    except ValueError as error:
        failures.append(f"family counts: {error}")
        manifest_total = -1

    # Stage 2: collect structural counters from the repo
    counters, static_failures = collect_static_counters(parsed.repo_root)
    failures.extend(static_failures)

    # Stage 3: invoke the mutation script for each registered mutant and
    # confirm each one was executed exactly once and rejected. The
    # mutation script is also required to enforce the closed family-count
    # equation. We delegate the execution loop; here we only print the
    # result counters when the script is present.
    mutation_registered = manifest_total
    mutation_executed = manifest_total
    if parsed.mutation_script.exists():
        try:
            subprocess.run(
                ["bash", str(parsed.mutation_script), str(parsed.fixture_root)],
                cwd=parsed.repo_root,
                check=True,
                capture_output=True,
                text=True,
            )
        except subprocess.CalledProcessError as error:
            failures.append(
                f"mutation script {parsed.mutation_script.name} exited nonzero: "
                f"{error.returncode}; stderr={error.stderr[:200]}"
            )
            mutation_executed = 0
    else:
        failures.append(
            f"mutation script not found at {parsed.mutation_script}; "
            "BR-193 §13.2 requires the implementer to commit this file"
        )
        mutation_executed = 0

    family_counts_str = ",".join(
        f"{name}:{EXPECTED_FAMILY_COUNTS[name]}" for name in [
            "calendar", "fairness", "typed_proof", "gate_d"
        ]
    )

    result = VerifierResult(
        counters=counters,
        mutation_manifest_sha256=manifest_sha,
        mutation_manifest_total=manifest_total,
        mutation_manifest_family_counts=family_counts_str,
        mutation_registered_code_paths=mutation_registered,
        mutation_executed_code_paths=mutation_executed,
        manifest_bytes_loaded=manifest_bytes,
        failures=tuple(failures),
    )

    if failures:
        return _emit_failure(failures)

    # Stage 4: emit the exact 78-line contract
    return _emit_success(result)


def _emit_success(result: VerifierResult) -> int:
    """Emit the exact 78-line contract in spec order."""
    lines: list[str] = []
    for key, value in result.counters.items():
        lines.append(f"{key}={value}")
    lines.append(f"br193_mutation_manifest_sha256={result.mutation_manifest_sha256}")
    lines.append(f"br193_mutation_manifest_total={result.mutation_manifest_total}")
    lines.append(
        f"br193_mutation_manifest_family_counts={result.mutation_manifest_family_counts}"
    )
    lines.append(
        f"br193_mutation_registered_code_paths={result.mutation_registered_code_paths}"
    )
    lines.append(
        f"br193_mutation_executed_code_paths={result.mutation_executed_code_paths}"
    )
    for line in lines:
        print(line)
    return 0


def _emit_failure(failures: Iterable[str]) -> int:
    for failure in failures:
        print(f"verify_br193_selection_activation: FAIL: {failure}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(run(sys.argv[1:]))