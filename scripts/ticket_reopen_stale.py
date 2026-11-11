#!/usr/bin/env python3
"""把“提案已 expired/invalidated、但 stage 仍 entry_proposed”的票放回 observing。

只处理 ticket_state_v1 中 stage=entry_proposed 且提案状态为终态的票；
默认 dry-run，`--apply` 才改 stage 并追加 proposal_reopened 事件。

不删除历史、不关闭 ticket；重复执行幂等（stage 已不是 entry_proposed 即跳过）。
"""

from __future__ import annotations

import argparse
import sqlite3
import sys
from datetime import datetime


def load_rows(conn: sqlite3.Connection) -> list[sqlite3.Row]:
    return conn.execute(
        """
        SELECT ticket_id, code, proposal_status, latest_event_kind, stale_days
        FROM ticket_state_v1
        WHERE close_date IS NULL
          AND stage = 'entry_proposed'
          AND proposal_status IN ('expired', 'invalidated')
        ORDER BY stale_days DESC, ticket_id
        """
    ).fetchall()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--database", required=True)
    parser.add_argument("--apply", action="store_true")
    args = parser.parse_args()

    conn = sqlite3.connect(f"file:{args.database}?mode={'rw' if args.apply else 'ro'}", uri=True)
    conn.row_factory = sqlite3.Row
    try:
        rows = load_rows(conn)
        print(f"待回收: {len(rows)}")
        for row in rows:
            print(
                f"  ticket={row['ticket_id']} {row['code']} "
                f"status={row['proposal_status']} stale={row['stale_days']}"
            )
        if not args.apply:
            print("dry-run：未改库；确认后加 --apply")
            return 0

        now = datetime.now().astimezone().replace(microsecond=0).isoformat()
        changed = 0
        with conn:
            for row in rows:
                cur = conn.execute(
                    "UPDATE ticket_lifecycle SET stage='observing', updated_at=CURRENT_TIMESTAMP "
                    "WHERE id=? AND close_date IS NULL AND stage='entry_proposed'",
                    (row["ticket_id"],),
                )
                if cur.rowcount == 0:
                    continue
                conn.execute(
                    "INSERT INTO ticket_event "
                    "(ticket_id, event_at, event_kind, push_kind, occurrence_identity, summary) "
                    "VALUES (?, ?, 'proposal_reopened', NULL, NULL, ?)",
                    (
                        row["ticket_id"],
                        now,
                        f"提案 {row['proposal_status']}，回到观察池等待下一次提案",
                    ),
                )
                changed += 1
        print(f"已回收: {changed}")
        return 0
    finally:
        conn.close()


if __name__ == "__main__":
    sys.exit(main())
