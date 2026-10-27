#!/usr/bin/env python3
"""校验 WP6 证据包的完整性；只读，不生成交易结论。

检查：
- manifest.sha256 是否覆盖且匹配；
- 必需文件是否存在且 JSON/JSONL 可解析；
- gaps.json 是否是可解析数组；
- eval.json 是否存在（可选，缺失记为 PARTIAL）。

退出码：0=PASS/PARTIAL，1=FAIL。
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


REQUIRED = [
    "memo.md",
    "gaps.json",
    "timeline.jsonl",
    "raw/ticket_lifecycle.json",
    "raw/ticket_event.jsonl",
    "raw/stock_daily.jsonl",
    "raw/delivery_decisions.json",
]


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def check_json(path: Path) -> str | None:
    try:
        json.loads(path.read_text())
    except Exception as exc:
        return str(exc)
    return None


def check_jsonl(path: Path) -> str | None:
    try:
        for line in path.read_text().splitlines():
            if line.strip():
                json.loads(line)
    except Exception as exc:
        return str(exc)
    return None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pack", required=True)
    args = parser.parse_args()
    pack = Path(args.pack)
    failures: list[str] = []
    warnings: list[str] = []

    for relative in REQUIRED:
        if not (pack / relative).is_file():
            failures.append(f"missing {relative}")
    if not (pack / "eval.json").is_file():
        warnings.append("missing eval.json (evaluation not run)")

    manifest_path = pack / "manifest.sha256"
    listed: set[str] = set()
    if not manifest_path.is_file():
        failures.append("missing manifest.sha256")
    else:
        for line in manifest_path.read_text().splitlines():
            if not line.strip():
                continue
            parts = line.split("  ", 1)
            if len(parts) != 2:
                failures.append(f"bad manifest line: {line[:80]}")
                continue
            expected, relative = parts
            listed.add(relative)
            target = pack / relative
            if not target.is_file():
                failures.append(f"manifest target missing: {relative}")
                continue
            actual = sha256(target)
            if actual != expected:
                failures.append(f"hash mismatch: {relative}")
        actual_files = {
            str(path.relative_to(pack))
            for path in pack.rglob("*")
            if path.is_file() and path.name != "manifest.sha256"
            and path.name not in {"eval.json", "audit.json"}
        }
        for relative in sorted(actual_files - listed):
            failures.append(f"not in manifest: {relative}")

    for relative in REQUIRED:
        path = pack / relative
        if not path.is_file():
            continue
        if path.suffix == ".json":
            error = check_json(path)
        elif path.suffix == ".jsonl":
            error = check_jsonl(path)
        else:
            error = None
        if error:
            failures.append(f"parse error {relative}: {error}")

    status = "FAIL" if failures else ("PARTIAL" if warnings else "PASS")
    result = {
        "status": status,
        "failures": failures,
        "warnings": warnings,
        "manifest_entries": len(listed),
    }
    (pack / "audit.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0 if status != "FAIL" else 1


if __name__ == "__main__":
    raise SystemExit(main())
