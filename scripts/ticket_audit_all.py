#!/usr/bin/env python3
"""6.9 统一审计入口（只读）。

- 6.9 决策证据包：调用 ticket_decision_evidence_audit.py；
- WP6 证据包：若给定 --pack（或存在 data/research/ticket_packages），
  调用 ticket_evidence_audit.py 校验 manifest/必需文件。
不复制两套规则，只做统一入口和汇总退出码。
"""
from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path


REPO = Path(__file__).resolve().parent.parent
DECISION_AUDIT = REPO / "scripts" / "ticket_decision_evidence_audit.py"
WP6_AUDIT = REPO / "scripts" / "ticket_evidence_audit.py"


def run(cmd: list[str]) -> tuple[int, str]:
    result = subprocess.run(cmd, text=True, capture_output=True)
    output = (result.stdout or "") + (result.stderr or "")
    return result.returncode, output.strip()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", required=True, help="运行根")
    parser.add_argument("--date", required=True)
    parser.add_argument("--pack", action="append", default=[], help="WP6 包目录，可重复")
    parser.add_argument("--no-report", action="store_true")
    args = parser.parse_args()

    failed = False

    decision_cmd = [
        sys.executable,
        str(DECISION_AUDIT),
        "--root",
        args.root,
        "--date",
        args.date,
    ]
    if args.no_report:
        decision_cmd.append("--no-report")
    code, output = run(decision_cmd)
    print("[6.9 决策证据包]")
    print(output)
    if code != 0:
        failed = True

    packs = list(args.pack)
    if not packs:
        default_pack = Path(args.root) / "data" / "research" / "ticket_packages" / args.date
        if default_pack.is_dir():
            packs = [str(default_pack)]
    if not packs:
        print("[WP6 证据包] SKIP：未给定 --pack，默认目录也不存在")
    for pack in packs:
        code, output = run([sys.executable, str(WP6_AUDIT), "--pack", pack])
        print(f"[WP6 证据包] {pack}")
        print(output)
        if code != 0:
            failed = True

    print("RESULT:", "FAIL" if failed else "PASS")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
