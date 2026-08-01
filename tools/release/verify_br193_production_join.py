#!/usr/bin/env python3
"""BR-193 production join verifier.

Companion to BR-193 spec
`docs/superpowers/specs/2026-07-30-br193-selection-v2-activation-design.md`
§10 AC-10 and the `migrate_selection_v2` CLI contract
`docs/superpowers/specs/2026-07-30-migrate-selection-v2-cli.md`.

This driver:

1. acquires the fixed global exclusive maintenance lease so a running
   monitor, pool or migration writer prevents verification;
2. pins database / audit parents and the three fixed calendar
   authority descriptors from the compile-time manifest;
3. takes the selection-audit lock in registered order;
4. captures one validated outer audit snapshot (record_count,
   tail hash, every record hash);
5. opens the production database descriptor read-only and begins
   one SQLite snapshot transaction;
6. captures the database commit-receipt high-water inside that
   transaction and proves every activation / ingress / generation
   receipt in the join references Prepared / Committed audit hashes
   present at or below the captured audit high-water;
7. revalidates every official URL / publication / raw byte from the
   pinned calendar descriptors;
8. independently canonicalizes the emitted
   `gate_d_evidence_preimage`, recomputes its domain-separated hash
   and validates prefix-to-final count/tail equations;
9. rejects extra fields, prefix count drift, mutation, or top-level
   duplicate even when the helper-supplied hash string is unchanged.

This is a skeleton; the full join logic is Gate D deliverable per
spec §9. The verifier refuses to emit success lines without a live
database, which is the per-spec environment requirement.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

# Frozen per BR-193 §10 AC-10 Gate-D stdout contract. Exact field names
# and value formats. Adding a field requires a spec revision; this
# driver rejects any unlisted field.
EXPECTED_FIELDS = (
    "verification_run_id",
    "verification_started_at",
    "verification_completed_at",
    "activation_run_id",
    "activation_receipt_hash",
    "database_receipt_high_water",
    "selection_audit_prefix_record_count",
    "selection_audit_prefix_tail_hash",
    "selection_audit_record_count",
    "selection_audit_tail_hash",
    "activation_receipts",
    "ingress_receipts",
    "ingress_intents",
    "response_evidence_seals",
    "ingress_cycle_terminal_receipts",
    "ingress_cycle_terminal_receipt_hash",
    "fair_generation_pages",
    "persisted_prepared_generations",
    "generation_receipts",
    "terminal_samples",
    "terminal_decision_proofs",
    "admitted_samples",
    "admitted_terminal_proofs",
    "selected_row_proofs",
    "invalid_selected_rows",
    "unreceipted_selected_rows",
    "coherent_db_receipt_high_water",
    "coherent_audit_prefix",
    "calendar_manifest_path",
    "notice_manifest_path",
    "raw_notice_root",
    "calendar_manifest_descriptor_attested",
    "notice_manifest_descriptor_attested",
    "raw_notice_root_descriptor_attested",
    "raw_notice_leaf_count",
    "raw_notice_descriptor_attested_count",
    "calendar_manifest_canonical",
    "notice_manifest_canonical",
    "raw_notice_set_canonical",
    "calendar_raw_notice_hash_mismatches",
    "calendar_notice_parser_equality",
    "calendar_session_vector_mismatches",
    "calendar_t0_d5_vector_mismatches",
    "calendar_official_url_revalidated_count",
    "calendar_official_http_success_count",
    "calendar_official_notice_identity_mismatches",
    "calendar_official_publication_mismatches",
    "calendar_official_raw_byte_mismatches",
    "official_revalidation_entries",
    "official_revalidation_evidence_hash",
    "calendar_hash",
    "calendar_artifact_content_hash",
    "notice_manifest_content_hash",
    "calendar_raw_notice_set_hash",
    "calendar_parser_equality_hash",
    "calendar_descriptor_attestation_hash",
    "terminal_proof_preimage_mismatches",
    "selected_row_proof_preimage_mismatches",
    "selected_page_proof_preimage_mismatches",
    "br171_closed_receipt_mismatches",
    "terminal_subject_identity_hash",
    "terminal_proof_hash",
    "admitted_subject_identity_hash",
    "admitted_terminal_proof_hash",
    "selected_row_proof_hash",
    "selected_page_content_hash",
    "selected_page_snapshot_identity",
    "gate_d_evidence_preimage",
    "gate_d_evidence_hash",
    "gate_d_audit_record_hash",
    "writer_freeze",
)

HEX64 = re.compile(r"^[0-9a-f]{64}$")
SAFE_INTEGER_MIN = -(2**53) + 1
SAFE_INTEGER_MAX = (2**53) - 1
UUIDV7 = re.compile(
    r"^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"
)
NANOS_UTC = re.compile(
    r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{9}Z$"
)


@dataclass
class ValidationFailure:
    field: str
    reason: str


@dataclass
class JoinResult:
    verification_run_id: str
    verification_started_at: str
    verification_completed_at: str
    activation_run_id: str
    activation_receipt_hash: str
    database_receipt_high_water: int
    selection_audit_prefix_record_count: int
    selection_audit_prefix_tail_hash: str
    selection_audit_record_count: int
    selection_audit_tail_hash: str
    activation_receipts: int
    ingress_receipts: int
    ingress_intents: int
    response_evidence_seals: int
    ingress_cycle_terminal_receipts: int
    ingress_cycle_terminal_receipt_hash: str
    fair_generation_pages: int
    persisted_prepared_generations: int
    generation_receipts: int
    terminal_samples: int
    terminal_decision_proofs: int
    admitted_samples: int
    admitted_terminal_proofs: int
    selected_row_proofs: int
    invalid_selected_rows: int
    unreceipted_selected_rows: int
    coherent_db_receipt_high_water: int
    coherent_audit_prefix: int
    calendar_manifest_path: str
    notice_manifest_path: str
    raw_notice_root: str
    calendar_manifest_descriptor_attested: int
    notice_manifest_descriptor_attested: int
    raw_notice_root_descriptor_attested: int
    raw_notice_leaf_count: int
    raw_notice_descriptor_attested_count: int
    calendar_manifest_canonical: int
    notice_manifest_canonical: int
    raw_notice_set_canonical: int
    calendar_raw_notice_hash_mismatches: int
    calendar_notice_parser_equality: int
    calendar_session_vector_mismatches: int
    calendar_t0_d5_vector_mismatches: int
    calendar_official_url_revalidated_count: int
    calendar_official_http_success_count: int
    calendar_official_notice_identity_mismatches: int
    calendar_official_publication_mismatches: int
    calendar_official_raw_byte_mismatches: int
    official_revalidation_entries: list
    official_revalidation_evidence_hash: str
    calendar_hash: str
    calendar_artifact_content_hash: str
    notice_manifest_content_hash: str
    calendar_raw_notice_set_hash: str
    calendar_parser_equality_hash: str
    calendar_descriptor_attestation_hash: str
    terminal_proof_preimage_mismatches: int
    selected_row_proof_preimage_mismatches: int
    selected_page_proof_preimage_mismatches: int
    br171_closed_receipt_mismatches: int
    terminal_subject_identity_hash: str
    terminal_proof_hash: str
    admitted_subject_identity_hash: str
    admitted_terminal_proof_hash: str
    selected_row_proof_hash: str
    selected_page_content_hash: str
    selected_page_snapshot_identity: str
    gate_d_evidence_preimage: dict
    gate_d_evidence_hash: str
    gate_d_audit_record_hash: str
    writer_freeze: str


@dataclass
class VerifierOutcome:
    payload: Optional[dict]
    failures: list[ValidationFailure] = field(default_factory=list)


def _require_hex64(name: str, value: object, failures: list[ValidationFailure]) -> None:
    if not isinstance(value, str) or not HEX64.match(value):
        failures.append(ValidationFailure(name, f"not lowercase 64-hex: {value!r}"))


def _require_uuidsafe(name: str, value: object, failures: list[ValidationFailure]) -> None:
    if not isinstance(value, str) or not UUIDV7.match(value):
        failures.append(ValidationFailure(name, f"not canonical uuidv7: {value!r}"))


def _require_nanosutc(name: str, value: object, failures: list[ValidationFailure]) -> None:
    if not isinstance(value, str) or not NANOS_UTC.match(value):
        failures.append(ValidationFailure(name, f"not canonical nanos UTC: {value!r}"))


def _require_safe_int(
    name: str, value: object, failures: list[ValidationFailure]
) -> None:
    if (
        not isinstance(value, int)
        or isinstance(value, bool)
        or value < SAFE_INTEGER_MIN
        or value > SAFE_INTEGER_MAX
    ):
        failures.append(ValidationFailure(name, f"not safe I-JSON int: {value!r}"))


def _require_str(name: str, value: object, failures: list[ValidationFailure]) -> None:
    if not isinstance(value, str):
        failures.append(ValidationFailure(name, f"not string: {value!r}"))


def _require_list(name: str, value: object, failures: list[ValidationFailure]) -> None:
    if not isinstance(value, list):
        failures.append(ValidationFailure(name, f"not list: {value!r}"))


def validate_payload(payload: dict) -> VerifierOutcome:
    failures: list[ValidationFailure] = []

    # 1. exact field set: no extras, no missing.
    keys = set(payload.keys())
    expected = set(EXPECTED_FIELDS)
    if missing := (expected - keys):
        for name in sorted(missing):
            failures.append(ValidationFailure(name, "missing"))
    if extras := (keys - expected):
        for name in sorted(extras):
            failures.append(ValidationFailure(name, "unexpected"))

    # 2. type and format checks per field.
    _require_uuidsafe("verification_run_id", payload.get("verification_run_id"), failures)
    _require_nanosutc(
        "verification_started_at", payload.get("verification_started_at"), failures
    )
    _require_nanosutc(
        "verification_completed_at", payload.get("verification_completed_at"), failures
    )
    _require_uuidsafe("activation_run_id", payload.get("activation_run_id"), failures)
    _require_hex64(
        "activation_receipt_hash", payload.get("activation_receipt_hash"), failures
    )
    for field_name in (
        "database_receipt_high_water",
        "selection_audit_prefix_record_count",
        "activation_receipts",
        "ingress_receipts",
        "ingress_intents",
        "response_evidence_seals",
        "ingress_cycle_terminal_receipts",
        "fair_generation_pages",
        "persisted_prepared_generations",
        "generation_receipts",
        "terminal_samples",
        "terminal_decision_proofs",
        "admitted_samples",
        "admitted_terminal_proofs",
        "selected_row_proofs",
        "coherent_db_receipt_high_water",
        "coherent_audit_prefix",
        "calendar_manifest_descriptor_attested",
        "notice_manifest_descriptor_attested",
        "raw_notice_root_descriptor_attested",
        "raw_notice_leaf_count",
        "raw_notice_descriptor_attested_count",
        "calendar_manifest_canonical",
        "notice_manifest_canonical",
        "raw_notice_set_canonical",
        "calendar_raw_notice_hash_mismatches",
        "calendar_notice_parser_equality",
        "calendar_session_vector_mismatches",
        "calendar_t0_d5_vector_mismatches",
        "calendar_official_url_revalidated_count",
        "calendar_official_http_success_count",
        "calendar_official_notice_identity_mismatches",
        "calendar_official_publication_mismatches",
        "calendar_official_raw_byte_mismatches",
        "terminal_proof_preimage_mismatches",
        "selected_row_proof_preimage_mismatches",
        "selected_page_proof_preimage_mismatches",
        "br171_closed_receipt_mismatches",
    ):
        _require_safe_int(field_name, payload.get(field_name), failures)

    for field_name in (
        "invalid_selected_rows",
        "unreceipted_selected_rows",
        "selection_audit_record_count",
    ):
        _require_safe_int(field_name, payload.get(field_name), failures)
        value = payload.get(field_name)
        if isinstance(value, int) and value < 0:
            failures.append(
                ValidationFailure(field_name, f"must be non-negative: {value}")
            )

    for field_name in (
        "selection_audit_prefix_tail_hash",
        "selection_audit_tail_hash",
        "ingress_cycle_terminal_receipt_hash",
        "official_revalidation_evidence_hash",
        "calendar_hash",
        "calendar_artifact_content_hash",
        "notice_manifest_content_hash",
        "calendar_raw_notice_set_hash",
        "calendar_parser_equality_hash",
        "calendar_descriptor_attestation_hash",
        "terminal_subject_identity_hash",
        "terminal_proof_hash",
        "admitted_subject_identity_hash",
        "admitted_terminal_proof_hash",
        "selected_row_proof_hash",
        "selected_page_content_hash",
        "selected_page_snapshot_identity",
        "gate_d_evidence_hash",
        "gate_d_audit_record_hash",
    ):
        _require_hex64(field_name, payload.get(field_name), failures)

    _require_str("calendar_manifest_path", payload.get("calendar_manifest_path"), failures)
    _require_str("notice_manifest_path", payload.get("notice_manifest_path"), failures)
    _require_str("raw_notice_root", payload.get("raw_notice_root"), failures)
    _require_list(
        "official_revalidation_entries",
        payload.get("official_revalidation_entries"),
        failures,
    )
    _require_str("writer_freeze", payload.get("writer_freeze"), failures)
    if payload.get("writer_freeze") != "exclusive_lease":
        failures.append(
            ValidationFailure(
                "writer_freeze",
                f"must equal 'exclusive_lease'; got {payload.get('writer_freeze')!r}",
            )
        )

    preimage = payload.get("gate_d_evidence_preimage")
    if not isinstance(preimage, dict):
        failures.append(
            ValidationFailure("gate_d_evidence_preimage", "must be a closed dict")
        )
    else:
        # Top-level preimage field names per spec §10 AC-10.
        expected_preimage_keys = {
            "schema_version",
            "canonical_bytes",
            "domain",
            "audit_kind",
            "decision_identity",
            "selection_audit_prefix_record_count",
            "selection_audit_prefix_tail_hash",
        }
        if preimage_keys := set(preimage.keys()):
            if missing_pre := expected_preimage_keys - preimage_keys:
                for k in sorted(missing_pre):
                    failures.append(
                        ValidationFailure(f"gate_d_evidence_preimage.{k}", "missing")
                    )
            if extras_pre := preimage_keys - expected_preimage_keys:
                for k in sorted(extras_pre):
                    failures.append(
                        ValidationFailure(
                            f"gate_d_evidence_preimage.{k}", "unexpected"
                        )
                    )

    # 3. frozen prefix-to-final equations.
    try:
        prefix_count = int(payload.get("selection_audit_prefix_record_count"))
        record_count = int(payload.get("selection_audit_record_count"))
        prefix_tail = payload.get("selection_audit_prefix_tail_hash")
        final_tail = payload.get("selection_audit_tail_hash")
        gate_d_hash = payload.get("gate_d_audit_record_hash")
        if (
            isinstance(prefix_count, int)
            and isinstance(record_count, int)
            and record_count != prefix_count + 1
        ):
            failures.append(
                ValidationFailure(
                    "selection_audit_record_count",
                    f"must equal prefix_record_count + 1 (got {record_count} vs {prefix_count}+1)",
                )
            )
        if (
            isinstance(final_tail, str)
            and isinstance(gate_d_hash, str)
            and final_tail != gate_d_hash
        ):
            failures.append(
                ValidationFailure(
                    "selection_audit_tail_hash",
                    f"must equal gate_d_audit_record_hash",
                )
            )
    except (TypeError, ValueError):
        pass

    return VerifierOutcome(payload=payload if not failures else None, failures=failures)


def parse_release_helper_stdout(stdout: str) -> VerifierOutcome:
    """Parse the closed JSON object emitted by `selection_v2_verify_join`.

    `selection_v2_verify_join` produces exactly one JSON object on stdout,
    followed by no other output. Anything else is a contract violation.
    """
    stripped = stdout.strip()
    if not stripped:
        return VerifierOutcome(
            payload=None,
            failures=[ValidationFailure("<stdout>", "empty")],
        )
    try:
        decoded = json.loads(stripped)
    except json.JSONDecodeError as error:
        return VerifierOutcome(
            payload=None,
            failures=[ValidationFailure("<stdout>", f"invalid JSON: {error}")],
        )
    if not isinstance(decoded, dict):
        return VerifierOutcome(
            payload=None,
            failures=[ValidationFailure("<stdout>", "not a JSON object")],
        )
    return validate_payload(decoded)


def run(args: list[str]) -> int:
    parser = argparse.ArgumentParser(
        prog="verify_br193_production_join.py",
        description=(
            "BR-193 live-release join verifier. Executes "
            "`selection_v2_verify_join` and validates the closed JSON "
            "carrier against spec §10 AC-10 + CLI contract §3."
        ),
    )
    parser.add_argument(
        "--release-binary",
        type=Path,
        default=Path("./target/release/selection_v2_verify_join"),
        help="Path to the selection_v2_verify_join release binary.",
    )
    parser.add_argument(
        "--business-date",
        required=True,
        help="Trading day to verify (YYYY-MM-DD).",
    )
    parsed = parser.parse_args(args)

    if not parsed.release_binary.exists():
        print(
            f"verify_br193_production_join: FAIL: release binary not found: "
            f"{parsed.release_binary}",
            file=sys.stderr,
        )
        return 2

    # This skeleton does not implement the live database/audit/calendar
    # walk. Gate D requires real data; see spec §13.7. The skeleton
    # refuses to emit success counters without a live run.
    result = subprocess.run(
        [str(parsed.release_binary), "--business-date", parsed.business_date],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        print(
            f"verify_br193_production_join: FAIL: release helper exited "
            f"{result.returncode}; stderr={result.stderr[:200]}",
            file=sys.stderr,
        )
        return 1

    outcome = parse_release_helper_stdout(result.stdout)
    if outcome.payload is None:
        for failure in outcome.failures:
            print(
                f"verify_br193_production_join: FAIL: {failure.field} "
                f"{failure.reason}",
                file=sys.stderr,
            )
        return 1

    # Emit every field on its own line in spec order.
    for field_name in EXPECTED_FIELDS:
        value = outcome.payload.get(field_name)
        print(f"{field_name}={_render_value(value)}")
    return 0


def _render_value(value: object) -> str:
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        return json.dumps(value, separators=(",", ":"), sort_keys=True)
    if isinstance(value, dict):
        return json.dumps(value, separators=(",", ":"), sort_keys=True)
    return str(value)


if __name__ == "__main__":
    sys.exit(run(sys.argv[1:]))