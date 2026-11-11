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


def load_rows(conn: sqlite3.Connection) -> tuple[list[tuple], bool]:
    view = conn.execute(
        "SELECT 1 FROM sqlite_master WHERE type='view' AND name='ticket_state_v1'"
    ).fetchone()
    if view is not None:
        return (
            conn.execute(
                """
                SELECT ticket_id, code, stage, proposal_status, decision_status,
                       decision_id, status_conflict, stale_days, latest_event_kind
                FROM ticket_state_v1
                WHERE close_date IS NULL
                ORDER BY status_conflict DESC, stale_days DESC, ticket_id
                """
            ).fetchall(),
            True,
        )
    # 视图尚未部署时用同一口径的 CTE 兜底；不写库。
    return (
        conn.execute(
            """
            WITH events AS (
              SELECT ticket_id,
                     CASE event_kind
                       WHEN 'entry_proposed' THEN 'open'
                       WHEN 'entry_triggered' THEN 'triggered'
                       WHEN 'entry_invalidated' THEN 'invalidated'
                       WHEN 'entry_expired' THEN 'expired'
                       WHEN 'entry_superseded' THEN 'superseded'
                     END AS status,
                     event_kind, event_at,
                     ROW_NUMBER() OVER (PARTITION BY ticket_id
                                        ORDER BY event_at DESC, id DESC) AS rn
              FROM ticket_event
              WHERE event_kind IN ('entry_proposed','entry_triggered',
                                   'entry_invalidated','entry_expired','entry_superseded')
            ), decisions AS (
              SELECT ticket_id, proposal_status, decision_id,
                     ROW_NUMBER() OVER (PARTITION BY ticket_id
                                        ORDER BY as_of DESC, created_at DESC, decision_id DESC) AS rn
              FROM ticket_decision
            )
            SELECT t.id, t.code, t.stage,
                   e.status, d.proposal_status, d.decision_id,
                   CASE WHEN e.status IS NOT NULL AND d.proposal_status IS NOT NULL
                              AND e.status <> d.proposal_status THEN 1 ELSE 0 END,
                   CAST(julianday('now') - julianday(COALESCE(e.event_at, t.open_date)) AS INTEGER),
                   e.event_kind
            FROM ticket_lifecycle t
            LEFT JOIN events e ON e.ticket_id=t.id AND e.rn=1
            LEFT JOIN decisions d ON d.ticket_id=t.id AND d.rn=1
            WHERE t.close_date IS NULL
            ORDER BY 7 DESC, 8 DESC, t.id
            """
        ).fetchall(),
        False,
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--database", required=True)
    parser.add_argument("--stale-days", type=int, default=5)
    args = parser.parse_args()

    conn = sqlite3.connect(f"file:{args.database}?mode=ro", uri=True)
    try:
        rows, from_view = load_rows(conn)
        print(f"数据源: {'ticket_state_v1' if from_view else 'CTE 兜底（视图未部署）'}")
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
