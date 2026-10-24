#!/usr/bin/env python3
"""Backfill stock_daily.ma5/ma10/ma20/volume_ratio from the rows already in the DB.

Canonical semantics (must match src/monitor/daily_bars_indicators.rs):
  - maN = arithmetic mean of close over the latest N trading rows of that code,
    INCLUDING the current row; NULL when fewer than N rows exist.
  - volume_ratio = current volume / mean(volume over the latest 5 rows, including
    the current row); NULL when the 5-row mean is missing or <= 0.

Default mode fills NULLs only (COALESCE); --recompute overwrites every row.
Nothing else is changed: data_source and updated_at are left untouched.

RUN THIS ON THE MAC AGAINST THE LOCAL DB. Do not point it at an SMB/UNC path:
SQLite's locking contract does not cover network filesystems, and the production
DB is written by the monitor.

Examples:
  python3 scripts/backfill_daily_ma.py --db ~/.local/share/stock-analysis-runtime/data/stock_analysis.db
  python3 scripts/backfill_daily_ma.py --db data/stock_analysis.db --codes 000001,600519
  python3 scripts/backfill_daily_ma.py --db data/stock_analysis.db --recompute --batch-size 100
"""

from __future__ import annotations

import argparse
import os
import sqlite3
import sys
import time
from pathlib import Path

WINDOW_SQL = """
WITH calc AS (
  SELECT rowid AS rid,
         CASE WHEN COUNT(close) OVER w5 = 5 THEN AVG(close) OVER w5 END AS ma5,
         CASE WHEN COUNT(close) OVER w10 = 10 THEN AVG(close) OVER w10 END AS ma10,
         CASE WHEN COUNT(close) OVER w20 = 20 THEN AVG(close) OVER w20 END AS ma20,
         CASE WHEN COUNT(volume) OVER w5 = 5 AND AVG(volume) OVER w5 > 0
              THEN volume / AVG(volume) OVER w5 END AS volume_ratio
  FROM stock_daily
  WHERE code = ?
  WINDOW w5  AS (ORDER BY date ROWS BETWEEN 4 PRECEDING AND CURRENT ROW),
         w10 AS (ORDER BY date ROWS BETWEEN 9 PRECEDING AND CURRENT ROW),
         w20 AS (ORDER BY date ROWS BETWEEN 19 PRECEDING AND CURRENT ROW)
)
UPDATE stock_daily
SET ma5 = {ma5},
    ma10 = {ma10},
    ma20 = {ma20},
    volume_ratio = {volume_ratio}
FROM calc
WHERE stock_daily.rowid = calc.rid
  AND ({row_filter})
"""

FILL_SQL = WINDOW_SQL.format(
    ma5="COALESCE(stock_daily.ma5, calc.ma5)",
    ma10="COALESCE(stock_daily.ma10, calc.ma10)",
    ma20="COALESCE(stock_daily.ma20, calc.ma20)",
    volume_ratio="COALESCE(stock_daily.volume_ratio, calc.volume_ratio)",
    row_filter=(
        "stock_daily.ma5 IS NULL OR stock_daily.ma10 IS NULL "
        "OR stock_daily.ma20 IS NULL OR stock_daily.volume_ratio IS NULL"
    ),
)

RECOMPUTE_SQL = WINDOW_SQL.format(
    ma5="calc.ma5",
    ma10="calc.ma10",
    ma20="calc.ma20",
    volume_ratio="calc.volume_ratio",
    row_filter="1",
)


def is_network_path(value: str) -> bool:
    text = value.replace("/", "\\")
    return text.startswith("\\\\") or text.startswith("//")


def connect(path: Path) -> sqlite3.Connection:
    conn = sqlite3.connect(str(path), timeout=60.0, isolation_level=None)
    conn.execute("PRAGMA busy_timeout = 60000")
    conn.execute("PRAGMA journal_mode = WAL")
    conn.execute("PRAGMA synchronous = NORMAL")
    conn.execute("PRAGMA temp_store = FILE")
    return conn


def count_null_rows(conn: sqlite3.Connection) -> int:
    return conn.execute(
        "SELECT COUNT(*) FROM stock_daily WHERE ma5 IS NULL OR ma10 IS NULL "
        "OR ma20 IS NULL OR volume_ratio IS NULL"
    ).fetchone()[0]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--db", default=os.environ.get("STOCK_DB"), help="local SQLite path (or STOCK_DB)")
    parser.add_argument("--codes", help="comma-separated codes; default all codes in stock_daily")
    parser.add_argument("--batch-size", type=int, default=100, help="codes per committed transaction")
    parser.add_argument("--recompute", action="store_true", help="overwrite non-NULL values too")
    parser.add_argument("--limit", type=int, help="debug: only the first N codes")
    parser.add_argument("--dry-run", action="store_true", help="only print the current NULL count")
    args = parser.parse_args()

    if not args.db:
        parser.error("--db is required (or set STOCK_DB)")
    if is_network_path(args.db):
        parser.error("refusing a network/UNC DB path; run this script on the Mac against the local path")
    path = Path(args.db).expanduser()
    if not path.exists():
        parser.error(f"database not found: {path}")

    conn = connect(path)
    sqlite_version = conn.execute("SELECT sqlite_version()").fetchone()[0]
    total, codes_total, min_date, max_date = conn.execute(
        "SELECT COUNT(*), COUNT(DISTINCT code), MIN(date), MAX(date) FROM stock_daily"
    ).fetchone()
    print(f"db={path}")
    print(f"sqlite={sqlite_version} rows={total} codes={codes_total} range={min_date}..{max_date}")
    before = count_null_rows(conn)
    print(f"rows_with_any_null_before={before} recompute={args.recompute}")
    if args.dry_run:
        conn.close()
        return 0

    if args.codes:
        codes = [item.strip() for item in args.codes.split(",") if item.strip()]
    else:
        codes = [row[0] for row in conn.execute("SELECT DISTINCT code FROM stock_daily ORDER BY code")]
    if args.limit:
        codes = codes[: args.limit]
    if not codes:
        print("no codes selected", file=sys.stderr)
        return 2

    sql = RECOMPUTE_SQL if args.recompute else FILL_SQL
    batch_size = max(1, args.batch_size)
    updated = 0
    started = time.monotonic()
    try:
        for start in range(0, len(codes), batch_size):
            batch = codes[start : start + batch_size]
            conn.execute("BEGIN IMMEDIATE")
            for code in batch:
                before_changes = conn.total_changes
                conn.execute(sql, (code,))
                updated += conn.total_changes - before_changes
            conn.execute("COMMIT")
            done = start + len(batch)
            elapsed = time.monotonic() - started
            rate = done / elapsed if elapsed > 0 else 0.0
            print(
                f"progress codes={done}/{len(codes)} updated_rows={updated} "
                f"elapsed={elapsed:.1f}s rate={rate:.1f} codes/s",
                flush=True,
            )
            if (start // batch_size) % 10 == 0:
                try:
                    conn.execute("PRAGMA wal_checkpoint(PASSIVE)")
                except sqlite3.Error:
                    pass
    except BaseException:
        try:
            conn.execute("ROLLBACK")
        except sqlite3.Error:
            pass
        raise

    after = count_null_rows(conn)
    print(f"rows_with_any_null_after={after} updated_rows={updated}")
    full_run = not args.codes and not args.limit
    if full_run:
        expected = conn.execute(
            "SELECT COALESCE(SUM(CASE WHEN n < 20 THEN n ELSE 19 END), 0) "
            "FROM (SELECT code, COUNT(*) AS n FROM stock_daily GROUP BY code)"
        ).fetchone()[0]
        print(f"expected_rows_with_any_null={expected} (first <20 rows per code)")
    print(
        "sample=",
        conn.execute(
            "SELECT code,date,close,ma5,ma10,ma20,volume_ratio FROM stock_daily "
            "WHERE code='000001' ORDER BY date DESC LIMIT 3"
        ).fetchall(),
    )
    conn.close()
    if full_run and after > expected:
        print(f"verification failed: {after} NULL rows > expected {expected}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
