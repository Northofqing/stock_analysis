#!/usr/bin/env python3
"""Validate exhaustive claim/finding ledgers and print reconciliation statistics."""

from __future__ import annotations

import collections
import csv
import sys
from pathlib import Path


BASE = Path(__file__).resolve().parents[1]
AGENTS = BASE / "agents"
ALLOWED = {"verified_complete", "partial", "unresolved", "contradicted", "unverifiable", "superseded"}


def rows(path: Path) -> list[dict[str, str]]:
    with path.open(encoding="utf-8", newline="") as handle:
        return list(csv.DictReader(handle, delimiter="\t"))


def main() -> int:
    manifest = {row["path"] for row in rows(BASE / "doc_manifest.tsv")}
    claims: list[dict[str, str]] = []
    errors: list[str] = []
    for name in ("early_claims.tsv", "late_claims.tsv"):
        path = AGENTS / name
        current = rows(path)
        print(f"{name}\t{len(current)}")
        for number, row in enumerate(current, 2):
            if row["source"] not in manifest:
                errors.append(f"{name}:{number}: source outside manifest: {row['source']}")
            if row["status"] not in ALLOWED:
                errors.append(f"{name}:{number}: invalid status: {row['status']}")
            if not row["claim_id"] or not row["normalized_requirement"]:
                errors.append(f"{name}:{number}: missing ID or normalized requirement")
        claims.extend(current)
    ids = collections.Counter(row["claim_id"] for row in claims)
    for claim_id, count in ids.items():
        if count > 1:
            errors.append(f"duplicate claim_id {claim_id}: {count}")
    print(f"claims_total\t{len(claims)}")
    for status, count in sorted(collections.Counter(row["status"] for row in claims).items()):
        print(f"status_{status}\t{count}")
    print(f"claim_bearing_docs\t{len({row['source'] for row in claims})}")
    weak_verified = [
        row for row in claims
        if row["status"] == "verified_complete"
        and (not row["implementation"] or not row["tests"] or not row["failure_path"])
    ]
    print(f"verified_complete_missing_impl_test_or_failure_evidence\t{len(weak_verified)}")
    standards = rows(AGENTS / "standards_findings.tsv")
    print(f"standards_findings\t{len(standards)}")
    for severity, count in sorted(collections.Counter(row["severity"] for row in standards).items()):
        print(f"standards_{severity.lower()}\t{count}")
    for error in errors:
        print(f"ERROR\t{error}")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
