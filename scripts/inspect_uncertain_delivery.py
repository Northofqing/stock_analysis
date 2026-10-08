"""Read original Schema9 manual-review evidence without resolving or sending."""

import argparse
import datetime as dt
import hashlib
import json
import sqlite3
from pathlib import Path


def inspect_database(path: Path) -> dict:
    source = path.resolve(strict=True)
    with sqlite3.connect(source.as_uri() + "?mode=ro", uri=True) as connection:
        connection.row_factory = sqlite3.Row
        connection.execute("PRAGMA query_only=ON")
        connection.execute("BEGIN")
        schema = connection.execute("PRAGMA user_version").fetchone()[0]
        if schema != 9:
            raise ValueError("expected retained delivery Schema9")
        decisions = connection.execute(
            """SELECT decision_identity, business_date, push_kind, sub_kind,
                      cooldown_scope, scope_key, state, envelope_canonical,
                      envelope_sha256, current_cooldown_reservation_identity,
                      created_at, updated_at
               FROM delivery_decisions WHERE state='UncertainManualReview'
               ORDER BY business_date, push_kind, scope_key, decision_identity"""
        ).fetchall()
        records = []
        for decision in decisions:
            issues = []
            envelope = decision["envelope_canonical"]
            if not isinstance(envelope, bytes) or hashlib.sha256(envelope).hexdigest() != decision["envelope_sha256"]:
                issues.append("original_envelope_hash_mismatch")
            scope = tuple(decision[key] for key in ("push_kind", "sub_kind", "cooldown_scope", "scope_key"))
            policy = connection.execute(
                "SELECT window_mode,cooldown_scope FROM delivery_policy_catalog WHERE push_kind=? AND sub_kind=?", scope[:2]
            ).fetchone()
            if policy is None or policy["cooldown_scope"] != decision["cooldown_scope"]:
                issues.append("original_policy_missing_or_scope_mismatch")
            head = connection.execute(
                """SELECT current_reservation_identity,state,blocked_until
                   FROM cooldown_heads WHERE push_kind=? AND sub_kind=?
                     AND cooldown_scope=? AND scope_key=?""", scope
            ).fetchone()
            current_reservation = decision["current_cooldown_reservation_identity"]
            owns_head = bool(head and current_reservation and head["current_reservation_identity"] == current_reservation)
            if owns_head:
                reservation = connection.execute(
                    "SELECT decision_identity FROM cooldown_reservations WHERE cooldown_reservation_identity=?", (current_reservation,)
                ).fetchone()
                if reservation is None or reservation[0] != decision["decision_identity"]:
                    issues.append("original_cooldown_reservation_owner_mismatch")
            blocking_head = bool(policy and policy["window_mode"] == "Rolling" and owns_head and head["state"] in ("Reserved", "Uncertain"))
            claim = connection.execute(
                """SELECT decision_identity FROM business_date_once_claims
                   WHERE business_date=? AND push_kind=? AND sub_kind=? AND scope_key=?""",
                (decision["business_date"], decision["push_kind"], decision["sub_kind"], decision["scope_key"]),
            ).fetchone()
            receipts = []
            for receipt in connection.execute(
                """SELECT result_event_identity,result_kind,observed_at,
                          authoritative_for_state,late_after_fence,
                          platform_message_id,accepted_at,result_canonical,result_sha256
                   FROM sink_results WHERE decision_identity=?
                   ORDER BY observed_at,result_event_identity""", (decision["decision_identity"],)
            ):
                raw = receipt["result_canonical"]
                if not isinstance(raw, bytes) or hashlib.sha256(raw).hexdigest() != receipt["result_sha256"]:
                    issues.append("original_sink_result_hash_mismatch")
                receipts.append({key: receipt[key] for key in (
                    "result_event_identity", "result_kind", "observed_at",
                    "authoritative_for_state", "late_after_fence", "platform_message_id", "accepted_at", "result_sha256",
                )})
            records.append({
                **{key: decision[key] for key in (
                    "decision_identity", "business_date", "push_kind", "sub_kind",
                    "cooldown_scope", "scope_key", "state", "envelope_sha256", "created_at", "updated_at",
                )},
                "policy_window_mode": policy["window_mode"] if policy else None,
                "owns_current_cooldown_head": owns_head,
                "blocks_new_rolling_scope": blocking_head,
                "current_cooldown_head_state": head["state"] if owns_head else None,
                "holds_original_business_date_claim": bool(claim and claim[0] == decision["decision_identity"]),
                "sink_results": receipts,
                "integrity_issues": issues,
                "required_action": "human_review_of_original_evidence",
            })
        connection.rollback()
    return {
        "report_version": 1,
        "coverage": "read_only_original_schema9_no_reconcile_no_resolution_no_delivery",
        "observed_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "database_schema": schema,
        "manual_review_count": len(records),
        "blocking_rolling_scope_count": sum(record["blocks_new_rolling_scope"] for record in records),
        "integrity_issue_count": sum(len(record["integrity_issues"]) for record in records),
        "records": records,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--db", required=True, type=Path)
    parser.add_argument("--output", type=Path, help="Create a new private JSON report; never overwrite an existing file")
    args = parser.parse_args()
    try:
        report = inspect_database(args.db)
        rendered = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
        if args.output:
            # Exclusive creation prevents accidental replacement of the database,
            # evidence, or an existing report; the material contains original IDs.
            import os
            descriptor = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(descriptor, "w") as destination:
                destination.write(rendered)
        else:
            print(rendered, end="")
        return 2 if report["integrity_issue_count"] else 0
    except (OSError, sqlite3.Error, ValueError) as error:
        parser.exit(1, f"uncertain delivery observation failed: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
