#!/usr/bin/env python3
"""Enforce AGENTS.md global/core line-coverage thresholds from llvm-cov JSON."""

from __future__ import annotations

import argparse
import json
import pathlib
import sys


CORE_PREFIXES = (
    "src/risk/",
    "src/trading/",
    "src/database/",
    "src/data_provider/",
    "src/decision/",
    "src/pipeline/",
    "src/event/",
)


def percentage(covered: int, count: int) -> float:
    return 100.0 if count == 0 else covered * 100.0 / count


def repository_relative_path(filename: str) -> str:
    """Normalize llvm-cov paths from local and repeated GitHub workspaces."""
    normalized = filename.replace("\\", "/")
    marker = "/stock_analysis/"
    if marker in normalized:
        return normalized.rsplit(marker, 1)[-1]
    try:
        return pathlib.Path(normalized).resolve().relative_to(pathlib.Path.cwd()).as_posix()
    except (OSError, ValueError):
        return normalized.lstrip("./")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("report", type=pathlib.Path)
    parser.add_argument("--global-min", type=float, default=80.0)
    parser.add_argument("--core-min", type=float, default=95.0)
    args = parser.parse_args()

    try:
        payload = json.loads(args.report.read_text(encoding="utf-8"))
        run = payload["data"][0]
        totals = run["totals"]["lines"]
        files = run["files"]
    except (OSError, json.JSONDecodeError, KeyError, IndexError, TypeError) as exc:
        print(f"coverage report is missing or invalid: {exc}", file=sys.stderr)
        return 2

    global_count = int(totals["count"])
    global_covered = int(totals["covered"])
    global_pct = percentage(global_covered, global_count)

    core_count = 0
    core_covered = 0
    matched = []
    for item in files:
        relative = repository_relative_path(str(item.get("filename", "")))
        if not relative.startswith(CORE_PREFIXES):
            continue
        lines = item.get("summary", {}).get("lines", {})
        count = int(lines.get("count", 0))
        covered = int(lines.get("covered", 0))
        core_count += count
        core_covered += covered
        matched.append(relative)

    if not matched or core_count == 0:
        print("coverage report contains no registered core-module lines", file=sys.stderr)
        return 2
    core_pct = percentage(core_covered, core_count)

    print(
        f"global line coverage: {global_covered}/{global_count} = {global_pct:.2f}% "
        f"(required {args.global_min:.2f}%)"
    )
    print(
        f"core line coverage: {core_covered}/{core_count} = {core_pct:.2f}% "
        f"(required {args.core_min:.2f}%, {len(matched)} files)"
    )

    failed = False
    if global_pct + 1e-9 < args.global_min:
        print("global coverage gate failed", file=sys.stderr)
        failed = True
    if core_pct + 1e-9 < args.core_min:
        print("core coverage gate failed", file=sys.stderr)
        failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
