#!/usr/bin/env python3
"""6.7 只读差异分类：原始竞价文件缺失的活跃票，是“没有报价”还是“未入选”。

输入一天的 auction_raw 文件，按 code 对比 ticket_lifecycle 活跃票；
对缺失票读取最近事件和最近 decision，输出可复核的原因分类。
不补造报价，不写生产库。
"""
from __future__ import annotations

import argparse
import json
import sqlite3
from pathlib import Path


def latest_events(conn: sqlite3.Connection, ticket_id: int) -> list[str]:
    rows = conn.execute(
        "SELECT event_kind, substr(event_at,1,10) FROM ticket_event "
        "WHERE ticket_id=? ORDER BY event_at DESC, id DESC LIMIT 3",
        (ticket_id,),
    ).fetchall()
    return [f"{kind}@{day}" for kind, day in rows]


def latest_decision(conn: sqlite3.Connection, ticket_id: int, as_of: str) -> tuple[str, str]:
    row = conn.execute(
        "SELECT proposal_status, regime FROM ticket_decision "
        "WHERE ticket_id=? AND as_of<=? ORDER BY as_of DESC, created_at DESC LIMIT 1",
        (ticket_id, as_of),
    ).fetchone()
    return (row[0], row[1]) if row else ("none", "none")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--db", required=True)
    parser.add_argument("--raw-file", required=True)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    raw_path = Path(args.raw_file)
    payload = json.loads(raw_path.read_text(encoding="utf-8"))
    records = payload.get("records")
    if not isinstance(records, list):
        raise SystemExit("raw payload has no records[]")
    raw_codes = {
        row.get("code") for row in records if isinstance(row, dict) and row.get("code")
    }
    business_date = str(payload.get("date") or raw_path.stem)

    conn = sqlite3.connect(f"file:{args.db}?mode=ro", uri=True)
    conn.row_factory = sqlite3.Row
    try:
        active = conn.execute(
            "SELECT id, code, stage FROM ticket_lifecycle "
            "WHERE close_date IS NULL ORDER BY code"
        ).fetchall()
        missing = []
        for row in active:
            if row["code"] in raw_codes:
                continue
            proposal_status, regime = latest_decision(
                conn, row["id"], business_date
            )
            missing.append(
                {
                    "code": row["code"],
                    "ticket_id": row["id"],
                    "stage": row["stage"],
                    "latest_events": latest_events(conn, row["id"]),
                    "proposal_status": proposal_status,
                    "regime": regime,
                }
            )
    finally:
        conn.close()

    result = {
        "date": business_date,
        "raw_rows": len(records),
        "raw_unique_codes": len(raw_codes),
        "active_tickets": len(active),
        "missing_count": len(missing),
        "missing": missing,
    }
    if args.json:
        print(json.dumps(result, ensure_ascii=False, indent=2))
    else:
        print(
            f"{business_date} raw={result['raw_rows']} "
            f"active={result['active_tickets']} missing={result['missing_count']}"
        )
        for row in missing:
            print(
                f"{row['code']} ticket={row['ticket_id']} stage={row['stage']} "
                f"regime={row['regime']} decision={row['proposal_status']} "
                f"events={','.join(row['latest_events'])}"
            )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
