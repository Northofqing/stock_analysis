#!/usr/bin/env python3
"""Apply a small TDX-derived JSONL patch to stock_daily (Mac-local only).

Rows are produced by magic-market-data-rs tools/ops/tdx_day_manifest.py.
The patch only touches OHLCV/amount/pct_chg/data_source; MA columns are left
alone and are filled afterwards by scripts/backfill_daily_ma.py.

Run on the Mac against the local runtime DB, never over SMB.
"""

from __future__ import annotations

import argparse
import json
import sqlite3
import sys
from pathlib import Path


def is_network_path(value: str) -> bool:
    text = value.replace("/", "\\")
    return text.startswith("\\\\") or text.startswith("//")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--db", required=True)
    parser.add_argument("--rows", required=True)
    parser.add_argument("--source", default="tdx_day")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    if is_network_path(args.db):
        parser.error("refusing a network/UNC DB path; run this on the Mac against the local DB")
    db = Path(args.db).expanduser()
    rows_path = Path(args.rows).expanduser()
    if not db.exists():
        parser.error(f"database not found: {db}")
    if not rows_path.exists():
        parser.error(f"rows file not found: {rows_path}")

    rows = [json.loads(line) for line in rows_path.read_text(encoding="utf-8").splitlines() if line.strip()]
    if not rows:
        print("no rows")
        return 0
    required = {"code", "date", "open", "high", "low", "close", "amount", "volume_lots", "pct_chg"}
    missing = required - set(rows[0])
    if missing:
        parser.error(f"rows are missing fields: {sorted(missing)}")

    conn = sqlite3.connect(str(db), timeout=60.0, isolation_level=None)
    conn.execute("PRAGMA busy_timeout = 60000")
    conn.execute("PRAGMA journal_mode = WAL")
    sql = """
    INSERT INTO stock_daily
      (code, date, open, high, low, close, volume, amount, pct_chg, data_source)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    ON CONFLICT(code, date) DO UPDATE SET
      open=excluded.open, high=excluded.high, low=excluded.low, close=excluded.close,
      volume=excluded.volume, amount=excluded.amount, pct_chg=excluded.pct_chg,
      data_source=excluded.data_source,
      ma5=NULL, ma10=NULL, ma20=NULL, volume_ratio=NULL,
      updated_at=CURRENT_TIMESTAMP
    """
    print(f"db={db} rows={len(rows)} source={args.source} dry_run={args.dry_run}")
    if args.dry_run:
        print(json.dumps(rows[:3], ensure_ascii=False))
        conn.close()
        return 0
    conn.execute("BEGIN IMMEDIATE")
    try:
        conn.executemany(
            sql,
            [
                (
                    row["code"], row["date"], row["open"], row["high"], row["low"], row["close"],
                    row["volume_lots"], row["amount"], row.get("pct_chg"), args.source,
                )
                for row in rows
            ],
        )
        conn.execute("COMMIT")
    except BaseException:
        conn.execute("ROLLBACK")
        raise
    print(f"applied={len(rows)}")
    conn.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
