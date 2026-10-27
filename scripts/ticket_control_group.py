#!/usr/bin/env python3
"""PIT 对照组的只读最小实现。

目的不是证明策略有效，而是回答：在同一交易日、同一板块、相近涨跌幅的
股票里，目标票是否有可比的参照样本。

本脚本只使用 as_of 当日及之前可见的日线，不读取未来收益，不补缺失价格。
样本不足时明确输出 INSUFFICIENT，而不是伪造对照组。

用法：
    python3 scripts/ticket_control_group.py --db <db> --code 001216 \
      --as-of 2026-10-26 --out <dir>
"""
from __future__ import annotations

import argparse
import json
import sqlite3
from pathlib import Path


def board_of(code: str) -> str:
    if code.startswith(("60", "68")):
        return "sh"
    if code.startswith(("00", "30")):
        return "sz"
    if code.startswith(("8", "4")):
        return "bj"
    return "unknown"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--db", required=True)
    parser.add_argument("--code", required=True)
    parser.add_argument("--as-of", required=True, dest="as_of")
    parser.add_argument("--out", required=True)
    parser.add_argument("--min-samples", type=int, default=20)
    parser.add_argument("--pct-band", type=float, default=1.0)
    args = parser.parse_args()

    db = Path(args.db)
    out = Path(args.out)
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    con.row_factory = sqlite3.Row
    try:
        target = con.execute(
            "SELECT code, date, close, pct_chg FROM stock_daily "
            "WHERE code=? AND date=?",
            (args.code, args.as_of),
        ).fetchone()
        if target is None or target["pct_chg"] is None:
            result = {
                "status": "BLOCKED",
                "reason": "target daily bar or pct_chg missing",
                "code": args.code,
                "as_of": args.as_of,
            }
        else:
            board = board_of(args.code)
            board_rows = con.execute(
                "SELECT DISTINCT board_code FROM board_member WHERE instrument_code=?",
                (args.code,),
            ).fetchall()
            board_codes = {row["board_code"] for row in board_rows}
            member_set: set[str] = set()
            if board_codes:
                placeholders = ",".join("?" for _ in board_codes)
                member_rows = con.execute(
                    f"SELECT instrument_code FROM board_member WHERE board_code IN ({placeholders})",
                    tuple(board_codes),
                ).fetchall()
                member_set = {row["instrument_code"] for row in member_rows}
            rows = con.execute(
                "SELECT code, date, close, pct_chg FROM stock_daily "
                "WHERE date=? AND code<>? AND close IS NOT NULL AND pct_chg IS NOT NULL",
                (args.as_of, args.code),
            ).fetchall()
            candidates = [
                {
                    "code": row["code"],
                    "close": row["close"],
                    "pct_chg": row["pct_chg"],
                }
                for row in rows
                if board_of(row["code"]) == board
                and (not member_set or row["code"] in member_set)
                and abs(row["pct_chg"] - target["pct_chg"]) <= args.pct_band
            ]
            candidates.sort(key=lambda item: (abs(item["pct_chg"] - target["pct_chg"]), item["code"]))
            if not board_codes:
                status = "WEAK" if len(candidates) >= args.min_samples else "INSUFFICIENT"
            else:
                status = "OK" if len(candidates) >= args.min_samples else "INSUFFICIENT"
            result = {
                "status": status,
                "code": args.code,
                "as_of": args.as_of,
                "target": {
                    "code": target["code"],
                    "close": target["close"],
                    "pct_chg": target["pct_chg"],
                },
                "method": {
                    "board": board,
                    "board_source": "board_member" if board_codes else "exchange_prefix_fallback",
                    "board_codes": sorted(board_codes),
                    "pct_band": args.pct_band,
                    "min_samples": args.min_samples,
                    "source": "stock_daily as_of + board_member",
                    "future_data_used": False,
                },
                "candidate_count": len(candidates),
                "candidates": candidates[:200],
            }
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n")
        print(json.dumps({"status": result["status"], "candidate_count": result.get("candidate_count", 0)}, ensure_ascii=False))
        return 0 if result["status"] in {"OK", "WEAK", "INSUFFICIENT"} else 1
    finally:
        con.close()


if __name__ == "__main__":
    raise SystemExit(main())
