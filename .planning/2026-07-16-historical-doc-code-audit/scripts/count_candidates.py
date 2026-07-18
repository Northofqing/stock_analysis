#!/usr/bin/env python3
"""Summarize the high-recall claim candidates per manifest document."""

from __future__ import annotations

import collections
import csv
import subprocess
from pathlib import Path


BASE = Path(__file__).resolve().parents[1]
extractor = Path(__file__).with_name("extract_claim_candidates.py")
raw = subprocess.run(["python3", str(extractor)], check=True, stdout=subprocess.PIPE).stdout
rows = list(csv.DictReader(raw.decode("utf-8", "replace").splitlines(), delimiter="\t"))
by_path: dict[str, list[dict[str, str]]] = collections.defaultdict(list)
for row in rows:
    by_path[row["path"]].append(row)

columns = ["checkbox", "action", "status", "action-heading", "table-row"]
print("path\tcandidate_count\tcheckbox\taction\tstatus\taction_heading\ttable_row")
with (BASE / "doc_manifest.tsv").open(encoding="utf-8", newline="") as handle:
    for doc in csv.DictReader(handle, delimiter="\t"):
        matches = by_path[doc["path"]]
        counts = [sum(tag in row["tags"].split(",") for row in matches) for tag in columns]
        print("\t".join(map(str, [doc["path"], len(matches), *counts])))
