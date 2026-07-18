#!/usr/bin/env python3
"""Check that review coverage files account for each manifest path and claim row."""

from __future__ import annotations

import csv
import re
import sys
from pathlib import Path


BASE = Path(__file__).resolve().parents[1]


def tsv_paths(path: Path) -> set[str]:
    with path.open(encoding="utf-8", newline="") as handle:
        return {row["path"] for row in csv.DictReader(handle, delimiter="\t")}


def coverage_paths(path: Path) -> set[str]:
    paths: set[str] = set()
    for line in path.read_text(encoding="utf-8").splitlines():
        for candidate in re.findall(r"`([^`]+)`", line):
            if candidate.startswith(("docs/", "src/", "tests/", "tools/", "config/", "migrations/", ".github/", "benches/", "deploy/", ".claude/", ".planning/", ".superpowers/", "reports/", "Cargo", "AGENTS", "CLAUDE", "README", "CHANGELOG", "IMPROVEMENTS", "diesel", ".env", ".gitignore")):
                paths.add(candidate)
        if line.startswith("|"):
            cells = [cell.strip().strip("`") for cell in line.split("|")[1:-1]]
            if cells and cells[0].startswith(("docs/", "src/", "tests/", "tools/", "config/", "migrations/", ".github/", "benches/", "deploy/", ".claude/", ".planning/", ".superpowers/", "reports/", "Cargo", "AGENTS", "CLAUDE", "README", "CHANGELOG", "IMPROVEMENTS", "diesel", ".env", ".gitignore")):
                paths.add(cells[0])
    return paths


def main() -> int:
    doc_manifest = tsv_paths(BASE / "doc_manifest.tsv")
    code_manifest = tsv_paths(BASE / "code_manifest.tsv")
    early = coverage_paths(BASE / "agents/early_coverage.md")
    late = coverage_paths(BASE / "agents/late_coverage.md")
    standards = coverage_paths(BASE / "agents/standards_coverage.md")
    missing_docs = sorted(doc_manifest - early - late)
    missing_code = sorted(code_manifest - standards)
    print(f"doc_manifest={len(doc_manifest)} early={len(early)} late={len(late)} missing={len(missing_docs)}")
    print(f"code_manifest={len(code_manifest)} standards={len(standards)} missing={len(missing_code)}")
    for path in missing_docs:
        print(f"MISSING_DOC\t{path}")
    for path in missing_code:
        print(f"MISSING_CODE\t{path}")
    return 1 if missing_docs or missing_code else 0


if __name__ == "__main__":
    sys.exit(main())
