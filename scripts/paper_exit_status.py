"""Read pending simulated exits without sending messages or changing account data."""

import argparse
import datetime as dt
import json
import sqlite3
from pathlib import Path


def pending_exits(connection: sqlite3.Connection, business_date: str) -> list[dict]:
    """Collapse retries by order; a later Filled attempt resolves that order."""
    dt.date.fromisoformat(business_date)
    connection.row_factory = sqlite3.Row
    rows = connection.execute(
        """
        WITH attempts AS (
            SELECT *, ROW_NUMBER() OVER (
                PARTITION BY business_order_id ORDER BY created_at DESC, id DESC
            ) AS latest,
            COUNT(*) OVER (PARTITION BY business_order_id) AS attempts
            FROM order_audit
            WHERE source = 'PaperTrade' AND side = 'sell'
              AND date(created_at, '+8 hours') = ?
        )
        SELECT business_order_id, code, quantity, requested_price,
               outcome, failure_reason, attempts,
               datetime(created_at, '+8 hours') AS last_attempt_shanghai
        FROM attempts
        WHERE latest = 1 AND outcome <> 'Filled'
          AND NOT EXISTS (
              SELECT 1 FROM attempts resolved
              WHERE resolved.business_order_id = attempts.business_order_id
                AND resolved.outcome = 'Filled'
          )
        ORDER BY code, business_order_id
        """,
        (business_date,),
    )
    return [dict(row) for row in rows]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--db", type=Path, default=Path("data/stock_analysis.db"))
    parser.add_argument("--date", default=dt.datetime.now(dt.timezone(dt.timedelta(hours=8))).date().isoformat())
    args = parser.parse_args()
    with sqlite3.connect(args.db.resolve().as_uri() + "?mode=ro", uri=True) as connection:
        rows = pending_exits(connection, args.date)
    print(json.dumps({"business_date": args.date, "scope": "模拟卖出；未执行二次确认或下单", "pending": rows}, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
