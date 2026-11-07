#!/usr/bin/env python3
"""6.2 涨停池原始字段探针（只读）。

目的：在不动生产代码的前提下，用库里已有的快照回答两件事：
1. `streak` 到底是「连续涨停天数」还是「N 天 M 板」里的累计板数；
2. 快照的连续性/覆盖是否足以支撑高位门（streak >= 4 且 phase == climax）。

用法：
    python3 scripts/limit_pool_probe.py --date 2026-11-06
    python3 scripts/limit_pool_probe.py --code 001216

只读打开生产库；不写任何表。
"""

from __future__ import annotations

import argparse
import sqlite3
import sys
from pathlib import Path

DEFAULT_DB = Path.home() / ".local/share/stock-analysis-runtime/data/stock_analysis.db"


def connect(db: Path) -> sqlite3.Connection:
    return sqlite3.connect(f"file:{db}?mode=ro", uri=True)


def trading_dates(conn: sqlite3.Connection) -> list[str]:
    return [
        row[0]
        for row in conn.execute(
            "SELECT DISTINCT trade_date FROM limit_up_pool_daily ORDER BY trade_date"
        )
    ]


def window_count(conn: sqlite3.Connection, code: str, dates: list[str], idx: int, window: int) -> int:
    """最近 window 个快照日里该票出现的次数（检验 N 天 M 板口径）。"""
    lo = max(0, idx - window + 1)
    return conn.execute(
        "SELECT COUNT(*) FROM limit_up_pool_daily WHERE code = ? AND trade_date BETWEEN ? AND ?",
        (code, dates[lo], dates[idx]),
    ).fetchone()[0]


def run_start(conn: sqlite3.Connection, code: str, dates: list[str], idx: int) -> tuple[int, int]:
    """该票本次连续在池的起点 (下标, 起点当日 streak)。"""
    start = idx
    for back in range(idx, -1, -1):
        row = conn.execute(
            "SELECT streak FROM limit_up_pool_daily WHERE trade_date = ? AND code = ?",
            (dates[back], code),
        ).fetchone()
        if row is None:
            break
        start = back
    row = conn.execute(
        "SELECT streak FROM limit_up_pool_daily WHERE trade_date = ? AND code = ?",
        (dates[start], code),
    ).fetchone()
    return start, (row[0] or 0) if row else 0


def probe_date(conn: sqlite3.Connection, date: str) -> int:
    dates = trading_dates(conn)
    if date not in dates:
        print(f"日期 {date} 不在快照里；可用范围 {dates[0]} ~ {dates[-1]}")
        return 1
    idx = dates.index(date)
    rows = conn.execute(
        "SELECT code, streak, sealed_amount, break_count, first_seal_at, board_name "
        "FROM limit_up_pool_daily WHERE trade_date = ? ORDER BY streak DESC, code",
        (date,),
    ).fetchall()
    print(f"== {date} 涨停池 {len(rows)} 只（快照日 {len(dates)} 个）==")
    print(f"{'code':8} {'streak':>6} {'期望':>4} {'5日出现':>6} {'封单(亿)':>9} {'炸板':>4} {'首封':>8} board")
    match_expected = 0
    match_window = 0
    checked = 0
    verified = 0
    mismatches = []
    for code, streak, sealed, breaks, first_seal, board in rows:
        if not streak or streak < 2:
            continue
        checked += 1
        start, anchor = run_start(conn, code, dates, idx)
        expected = anchor + (idx - start) if anchor > 0 else None
        five = window_count(conn, code, dates, idx, 5)
        if idx > start:
            verified += 1
        if expected == streak:
            match_expected += 1
        else:
            mismatches.append((code, streak, expected, five))
            if len(mismatches) <= 12:
                print(
                    f"{code:8} {streak:>6} {str(expected):>4} {five:>6} "
                    f"{(sealed or 0) / 1e8:>9.3f} {breaks if breaks is not None else -1:>4} "
                    f"{first_seal or '-':>8} {board or '-'}  <-- 口径不符"
                )
        if five == streak:
            match_window += 1
    print()
    print(f"streak>=2 的样本 {checked} 只（其中 {verified} 只可独立验证，其余为窗口首日锚点）：")
    print(f"  streak == 锚点+连续在池天数：{match_expected}/{checked}")
    print(f"  streak == 最近5个快照出现次数：{match_window}/{checked}")
    if mismatches:
        print("  不符样例（code, streak, 期望, 5日出现）：")
        for code, streak, expected, five in mismatches[:10]:
            print(f"    {code} streak={streak} 期望={expected} 5日={five}")
    return 0


def probe_code(conn: sqlite3.Connection, code: str) -> int:
    rows = conn.execute(
        "SELECT trade_date, streak, sealed_amount, break_count, first_seal_at, board_name "
        "FROM limit_up_pool_daily WHERE code = ? ORDER BY trade_date",
        (code,),
    ).fetchall()
    if not rows:
        print(f"{code} 不在涨停池快照里")
        return 1
    print(f"== {code} 涨停池历史 ==")
    for trade_date, streak, sealed, breaks, first_seal, board in rows:
        print(
            f"{trade_date} streak={streak} 封单={((sealed or 0) / 1e8):.3f}亿 "
            f"炸板={breaks} 首封={first_seal or '-'} board={board or '-'}"
        )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--db", type=Path, default=DEFAULT_DB)
    parser.add_argument("--date", default=None)
    parser.add_argument("--code", default=None)
    args = parser.parse_args()
    if not args.db.exists():
        print(f"库不存在: {args.db}")
        return 1
    conn = connect(args.db)
    try:
        if args.code:
            return probe_code(conn, args.code)
        if args.date:
            return probe_date(conn, args.date)
        latest = trading_dates(conn)[-1]
        return probe_date(conn, latest)
    finally:
        conn.close()


if __name__ == "__main__":
    sys.exit(main())
