#!/usr/bin/env python3
"""Extract high-recall actionable claim candidates from every text document."""

from __future__ import annotations

import csv
import re
import subprocess
import sys
from pathlib import Path


BASE = Path(__file__).resolve().parents[1]
ROOT = BASE.parents[1]
MANIFEST = BASE / "doc_manifest.tsv"
ACTION = re.compile(
    r"(?:\bMUST\b|\bSHALL\b|\bSHOULD\b|\bTODO\b|\bFIXME\b|\bBUG\b|"
    r"\bP[0-3]\b|\bAC[- ]?\d+\b|验收|完成标准|完成条件|必须|不得|禁止|应当|需要|"
    r"待办|待修|未修|未完成|未实现|缺失|阻塞|风险|修复|解决|实现|接入|删除|迁移|"
    r"上线|发布|回滚|兼容|去重|限流|互斥|过滤|排序|阈值|失败|错误|异常|bug)",
    re.IGNORECASE,
)
STATUS = re.compile(
    r"(?:✅|❌|⚠️|⏳|🚧|\bDONE\b|\bPASS\b|\bFAIL\b|\bBLOCKED\b|"
    r"\bIN[ -]?PROGRESS\b|\bDEFERRED\b|已完成|已修复|已解决|部分完成|进行中|延期|"
    r"搁置|跳过|未验证|不可验证)",
    re.IGNORECASE,
)
CHECKBOX = re.compile(r"^\s*(?:[-*+]\s+)?\[[ xX]\]")
NUMBERED = re.compile(r"^\s*(?:[-*+]\s+|\d+[.)、]\s+)")


def content(row: dict[str, str]) -> str:
    path = row["path"]
    if row["present_workspace"] == "true":
        return (ROOT / path).read_text(encoding="utf-8", errors="replace")
    commit = row["content_commit"]
    proc = subprocess.run(
        ["git", "show", f"{commit}:{path}"],
        cwd=ROOT,
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return proc.stdout.decode("utf-8", "replace")


def tags(line: str) -> list[str]:
    found: list[str] = []
    if CHECKBOX.search(line):
        found.append("checkbox")
    if ACTION.search(line):
        found.append("action")
    if STATUS.search(line):
        found.append("status")
    if line.lstrip().startswith("#") and ACTION.search(line):
        found.append("action-heading")
    if "|" in line and (ACTION.search(line) or STATUS.search(line)):
        found.append("table-row")
    return found


def main() -> int:
    writer = csv.writer(sys.stdout, delimiter="\t", lineterminator="\n")
    writer.writerow(["candidate_id", "path", "content_commit", "line", "tags", "text"])
    with MANIFEST.open(encoding="utf-8", newline="") as handle:
        for row in csv.DictReader(handle, delimiter="\t"):
            if row["content_kind"] != "text":
                continue
            for line_no, raw_line in enumerate(content(row).splitlines(), 1):
                line = " ".join(raw_line.strip().split())
                if not line:
                    continue
                matched = tags(line)
                if not matched:
                    continue
                # Avoid treating long prose mentions as atomic claims unless it has a status/requirement marker.
                if len(line) > 800 and not (CHECKBOX.search(line) or STATUS.search(line)):
                    continue
                candidate_id = f"CAND-{row['path']}:{line_no}"
                writer.writerow([candidate_id, row["path"], row["content_commit"], line_no, ",".join(matched), line])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
