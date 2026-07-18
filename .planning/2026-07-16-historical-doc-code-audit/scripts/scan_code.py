#!/usr/bin/env python3
"""Read every manifest implementation file and emit per-file semantic-risk metrics."""

from __future__ import annotations

import csv
import hashlib
import re
import subprocess
import sys
from pathlib import Path


BASE = Path(__file__).resolve().parents[1]
ROOT = BASE.parents[1]
MANIFEST = BASE / "code_manifest.tsv"
SNAPSHOT = sys.argv[1] if len(sys.argv) > 1 else "HEAD"

PATTERNS = {
    "declarations": re.compile(r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:fn|struct|enum|trait|type|const|static|mod)\s+[A-Za-z_][A-Za-z0-9_]*"),
    "tests": re.compile(r"#\[(?:tokio::)?test(?:\([^]]*\))?\]"),
    "todo_fixme": re.compile(r"\b(?:TODO|FIXME|XXX)\b|待实现|未实现", re.IGNORECASE),
    "todo_macros": re.compile(r"\b(?:todo|unimplemented)!\s*\("),
    "panic_macros": re.compile(r"\bpanic!\s*\("),
    "unwraps": re.compile(r"\.unwrap\s*\("),
    "expects": re.compile(r"\.expect\s*\("),
    "default_fallbacks": re.compile(r"unwrap_or_default\s*\(|unwrap_or\s*\(\s*(?:0(?:\.0)?|false|None|String::new\(\)|vec!\[\])"),
    "mock_terms": re.compile(r"\b(?:mock|fake|noop|stub|dummy|demo|模拟|伪造)\b", re.IGNORECASE),
    "target_verbs": re.compile(r"\b(?:verify|save|notify|push|sync|update_result|reconcile)[A-Za-z0-9_]*\b"),
    "log_calls": re.compile(r"\b(?:trace|debug|info|warn|error)!\s*\("),
    "unsafe": re.compile(r"\bunsafe\b"),
}


def snapshot_blob(path: str) -> bytes:
    return subprocess.run(
        ["git", "show", f"{SNAPSHOT}:{path}"], cwd=ROOT, check=True,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    ).stdout


def main() -> int:
    writer = csv.writer(sys.stdout, delimiter="\t", lineterminator="\n")
    writer.writerow(["path", "source", "sha_verified", *PATTERNS.keys()])
    with MANIFEST.open(encoding="utf-8", newline="") as handle:
        for row in csv.DictReader(handle, delimiter="\t"):
            if row["content_kind"] != "text":
                writer.writerow([row["path"], "binary-skip", "true", *("" for _ in PATTERNS)])
                continue
            if row["present_snapshot"] == "true":
                data = snapshot_blob(row["path"])
                source = SNAPSHOT
            else:
                data = (ROOT / row["path"]).read_bytes()
                source = "workspace-only-excluded"
            verified = hashlib.sha256(data).hexdigest() == row["sha256"]
            text = data.decode("utf-8", "replace")
            writer.writerow([
                row["path"], source, str(verified).lower(),
                *(len(pattern.findall(text)) for pattern in PATTERNS.values()),
            ])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
