#!/usr/bin/env python3
"""只读巡检 ticket_state_v1：三源状态冲突与陈旧提案。

用法:
  python3 scripts/ticket_state_audit.py --database /path/to/stock_analysis.db

不写库；视图不存在时明确报缺失，不猜测状态。
"""

from __future__ import annotations

import argparse
import sqlite3
import sys
from collections import Counter


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--database", required=True)
    parser.add_argument("--stale-days", type=int, default=5)
    args = parser.parse_args()

    conn = sqlite3.connect(f"file:{args.database}?mode=ro", uri=True)
    try:
        view = conn.execute(
            "SELECT 1 FROM sqlite_master WHERE type='view' AND name='ticket_state_v1'"
        ).fetchone()
        if view is None:
            print("ticket_state_v1 不存在：monitor 尚未用新二进制启动，未建视图")
            return 0

        rows = conn.execute(
            """
            SELECT ticket_id, code, stage, proposal_status, decision_status,
                   decision_id, status_conflict, stale_days, latest_event_kind
            FROM ticket_state_v1
            WHERE close_date IS NULL
            ORDER BY status_conflict DESC, stale_days DESC, ticket_id
            """
        ).fetchall()
        print(f"活跃票: {len(rows)}")
        stages = Counter(row[2] for row in rows)
        proposals = Counter(row[3] or "（无提案事件）" for row in rows)
        print("stage: " + ", ".join(f"{key}={value}" for key, value in stages.most_common()))
        print("提案事件: " + ", ".join(f"{key}={value}" for key, value in proposals.most_common()))

        conflicts = [row for row in rows if row[6]]
        print(f"事件/decision 冲突: {len(conflicts)}")
        for row in conflicts[:20]:
            print(
                f"  ticket={row[0]} {row[1]} stage={row[2]} "
                f"event={row[3]} decision={row[4]} decision_id={row[5]}"
            )

        stale = [row for row in rows if (row[7] or 0) >= args.stale_days]
        print(f"陈旧 >= {args.stale_days} 天: {len(stale)}")
        for row in stale[:20]:
            print(
                f"  ticket={row[0]} {row[1]} stage={row[2]} "
                f"proposal={row[3]} stale_days={row[7]}"
            )
        return 0
    finally:
        conn.close()


if __name__ == "__main__":
    sys.exit(main())
