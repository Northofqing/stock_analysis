#!/usr/bin/env python3
"""Read-only integrity audit for stock_daily.

This is the gate before/after a TDX import or MA backfill. It never writes the
database. Run it on the Mac against the local runtime DB; a read-only run from
Windows over SMB is acceptable for baseline inspection only.

Checks:
  - total rows, distinct codes, date range
  - duplicate (code, date)
  - future dates, null/non-positive OHLC, high/low violations
  - negative volume/amount
  - last N dates coverage and MA coverage
  - MA NULL counts vs the expected warm-up (first <20 rows per code)
  - target-date coverage and missing codes
  - data_source distribution

Exit code 1 means a structural violation; a missing target date is reported but
does not fail unless --require-date is passed.
"""

from __future__ import annotations

import argparse
import json
import sqlite3
import sys
from datetime import date
from pathlib import Path


def connect(path: str) -> sqlite3.Connection:
    if path.startswith("//") or path.startswith("\\\\"):
        connection = sqlite3.connect(path, timeout=30.0)
        connection.execute("PRAGMA query_only = ON")
    else:
        connection = sqlite3.connect(f"file:{path}?mode=ro", uri=True, timeout=30.0)
    connection.execute("PRAGMA busy_timeout = 30000")
    return connection


def scalar(conn: sqlite3.Connection, sql: str, params: tuple = ()):
    return conn.execute(sql, params).fetchone()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--db", required=True)
    parser.add_argument("--date", default="2026-10-23", help="target trading date")
    parser.add_argument("--recent-days", type=int, default=15)
    parser.add_argument("--out", help="write JSON report to this path")
    parser.add_argument("--require-date", action="store_true", help="fail when target date has no rows")
    parser.add_argument("--integrity", action="store_true", help="also run PRAGMA integrity_check")
    args = parser.parse_args()

    conn = connect(args.db)
    today = date.today().isoformat()
    violations: list[str] = []

    total, codes, min_date, max_date, null_close, null_volume, high_low, open_high, close_high, open_low, close_low, nonpos_close, neg_volume, neg_amount, future = conn.execute(
        """
        SELECT COUNT(*), COUNT(DISTINCT code), MIN(date), MAX(date),
               SUM(close IS NULL), SUM(volume IS NULL),
               SUM(high IS NOT NULL AND low IS NOT NULL AND high < low),
               SUM(open IS NOT NULL AND high IS NOT NULL AND open > high),
               SUM(close IS NOT NULL AND high IS NOT NULL AND close > high),
               SUM(open IS NOT NULL AND low IS NOT NULL AND open < low),
               SUM(close IS NOT NULL AND low IS NOT NULL AND close < low),
               SUM(close IS NOT NULL AND close <= 0),
               SUM(volume < 0), SUM(amount < 0), SUM(date > ?)
        FROM stock_daily
        """,
        (today,),
    ).fetchone()

    duplicate_groups, duplicate_rows = conn.execute(
        "SELECT COUNT(*), COALESCE(SUM(n), 0) FROM "
        "(SELECT code, date, COUNT(*) AS n FROM stock_daily GROUP BY code, date HAVING COUNT(*) > 1)"
    ).fetchone()

    if duplicate_groups:
        violations.append(f"duplicate (code,date) groups={duplicate_groups} rows={duplicate_rows}")
    if future:
        violations.append(f"future dates rows={future} (today={today})")
    if null_close:
        violations.append(f"null close rows={null_close}")
    if null_volume:
        violations.append(f"null volume rows={null_volume}")
    for label, value in [("high<low", high_low), ("open>high", open_high), ("close>high", close_high), ("open<low", open_low), ("close<low", close_low), ("close<=0", nonpos_close), ("volume<0", neg_volume), ("amount<0", neg_amount)]:
        if value:
            violations.append(f"{label} rows={value}")

    recent = conn.execute(
        "SELECT date, COUNT(*), COUNT(DISTINCT code), SUM(ma5 IS NOT NULL) "
        "FROM stock_daily GROUP BY date ORDER BY date DESC LIMIT ?",
        (args.recent_days,),
    ).fetchall()

    ma = conn.execute(
        "SELECT SUM(ma5 IS NULL), SUM(ma10 IS NULL), SUM(ma20 IS NULL), SUM(volume_ratio IS NULL) FROM stock_daily"
    ).fetchone()
    any_null = conn.execute(
        "SELECT COUNT(*) FROM stock_daily WHERE ma5 IS NULL OR ma10 IS NULL OR ma20 IS NULL OR volume_ratio IS NULL"
    ).fetchone()[0]
    expected_warmup = conn.execute(
        "SELECT COALESCE(SUM(CASE WHEN n < 20 THEN n ELSE 19 END), 0) "
        "FROM (SELECT code, COUNT(*) AS n FROM stock_daily GROUP BY code)"
    ).fetchone()[0]

    sources = conn.execute(
        "SELECT COALESCE(data_source, '<null>'), COUNT(*) FROM stock_daily GROUP BY data_source ORDER BY 2 DESC"
    ).fetchall()

    target_total, target_codes = conn.execute(
        "SELECT COUNT(*), COUNT(DISTINCT code) FROM stock_daily WHERE date = ?", (args.date,)
    ).fetchone()
    all_codes = {row[0] for row in conn.execute("SELECT DISTINCT code FROM stock_daily")}
    target_set = {row[0] for row in conn.execute("SELECT code FROM stock_daily WHERE date = ?", (args.date,))}
    missing_target = sorted(all_codes - target_set)

    if target_total == 0 and args.require_date:
        violations.append(f"target date {args.date} has no rows")

    integrity = None
    if args.integrity:
        integrity = conn.execute("PRAGMA integrity_check").fetchone()[0]
        if integrity != "ok":
            violations.append(f"integrity_check={integrity}")

    report = {
        "db": args.db,
        "generated_at": __import__("datetime").datetime.now().astimezone().isoformat(),
        "today": today,
        "total_rows": total,
        "distinct_codes": codes,
        "min_date": min_date,
        "max_date": max_date,
        "null_close": null_close,
        "null_volume": null_volume,
        "duplicate_groups": duplicate_groups,
        "duplicate_rows": duplicate_rows,
        "future_rows": future,
        "ohlc_violations": {
            "high_lt_low": high_low,
            "open_gt_high": open_high,
            "close_gt_high": close_high,
            "open_lt_low": open_low,
            "close_lt_low": close_low,
            "close_le_zero": nonpos_close,
        },
        "negative": {"volume": neg_volume, "amount": neg_amount},
        "recent_dates": [
            {"date": r[0], "rows": r[1], "codes": r[2], "ma5_non_null": r[3]} for r in recent
        ],
        "ma": {
            "ma5_null": ma[0],
            "ma10_null": ma[1],
            "ma20_null": ma[2],
            "volume_ratio_null": ma[3],
            "any_null": any_null,
            "expected_warmup_null": expected_warmup,
            "unbackfilled_rows": max(0, any_null - expected_warmup),
        },
        "data_sources": [{"source": r[0], "rows": r[1]} for r in sources],
        "target_date": args.date,
        "target_rows": target_total,
        "target_codes": target_codes,
        "target_missing_count": len(missing_target),
        "target_missing_sample": missing_target[:30],
        "integrity_check": integrity,
        "violations": violations,
        "ok": not violations,
    }

    text = json.dumps(report, ensure_ascii=False, indent=2)
    if args.out:
        out = Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(text, encoding="utf-8")
        print(f"report={out}")
    print(text)
    conn.close()
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
