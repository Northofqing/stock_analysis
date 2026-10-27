#!/usr/bin/env python3
"""WP8 新入口策略 gates：只读判断是否允许把 shadow 切到 new。

规则（2026-10-27 文档 11.11.8）：
- 至少 5 个不同交易日有 ticket_decision；
- 至少 1 次 entry_triggered；
- 至少 1 次 entry_expired 或 entry_invalidated；
- 没有致命的 context/source 缺口时，才输出 READY。

这里只给 gate 结论，不修改 .env、不重启、不切换策略。
"""
from __future__ import annotations

import argparse
import json
import sqlite3
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--db", required=True)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    con = sqlite3.connect(f"file:{args.db}?mode=ro", uri=True)
    reasons: list[str] = []
    try:
        tables = {row[0] for row in con.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        if "ticket_decision" not in tables:
            result = {"status": "BLOCKED", "reasons": ["ticket_decision table missing"]}
        else:
            dates = [
                row[0]
                for row in con.execute(
                    "SELECT DISTINCT as_of FROM ticket_decision WHERE as_of IS NOT NULL ORDER BY as_of"
                ).fetchall()
            ]
            decisions = con.execute("SELECT COUNT(*) FROM ticket_decision").fetchone()[0]
            context_rows = con.execute(
                "SELECT COUNT(*) FROM ticket_decision WHERE context_json IS NOT NULL AND context_json <> ''"
            ).fetchone()[0]
            triggered = con.execute(
                "SELECT COUNT(*) FROM ticket_event WHERE event_kind='entry_triggered'"
            ).fetchone()[0]
            expired = con.execute(
                "SELECT COUNT(*) FROM ticket_event WHERE event_kind IN ('entry_expired','entry_invalidated')"
            ).fetchone()[0]
            if len(dates) < 5:
                reasons.append(f"only {len(dates)} decision dates, need 5")
            if decisions == 0:
                reasons.append("no ticket_decision rows")
            if context_rows < decisions:
                reasons.append(f"{decisions - context_rows} decisions lack context_json")
            if triggered == 0:
                reasons.append("no entry_triggered event")
            if expired == 0:
                reasons.append("no entry_expired/entry_invalidated event")
            result = {
                "status": "READY" if not reasons else "BLOCKED",
                "decision_dates": dates,
                "decision_rows": decisions,
                "context_rows": context_rows,
                "triggered_events": triggered,
                "terminal_events": expired,
                "reasons": reasons,
            }
    finally:
        con.close()
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n")
    print(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True))
    return 0 if result["status"] == "READY" else 1


if __name__ == "__main__":
    raise SystemExit(main())
