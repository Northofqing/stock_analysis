#!/usr/bin/env python3
"""统计 entry_triggered 的触发价是否落在提案区间内。只读数据库。

用途（批 7.1）：在改触发逻辑之前，先量出「区间外触发」的真实占比。
数据源用 `ticket_event.entry_triggered.summary`（触发当时写入的现价与区间），
不再从 shadow 差异日志反推——差异日志只记录新旧判据不一致的样本，不是全量触发。

用法：
  python3 scripts/ticket_trigger_audit.py --db <stock_analysis.db> [--since D] [--until D] [--json]
"""
from __future__ import annotations

import argparse
import json
import re
import sqlite3
from collections import defaultdict
from pathlib import Path

PRICE_RE = re.compile(r"现价\s*([0-9]+(?:\.[0-9]+)?)")
RANGE_RE = re.compile(r"区间\s*([0-9]+(?:\.[0-9]+)?)\s*-\s*([0-9]+(?:\.[0-9]+)?)")


def parse_summary(summary: str):
    price = PRICE_RE.search(summary or "")
    zone = RANGE_RE.search(summary or "")
    if not price or not zone:
        return None
    low, high = float(zone.group(1)), float(zone.group(2))
    price_value = float(price.group(1))
    if price_value <= 0 or low <= 0 or high < low:
        return None
    return price_value, low, high


def classify(price: float, low: float, high: float):
    if price < low:
        gap = (price - low) / low * 100.0
        return "below", gap
    if price > high:
        gap = (price - high) / high * 100.0
        return "above", gap
    return "inside", 0.0


def percentile(values: list[float], pct: float):
    if not values:
        return None
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, int(round((len(ordered) - 1) * pct))))
    return ordered[index]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--db", required=True)
    parser.add_argument("--since", default=None)
    parser.add_argument("--until", default=None)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    where = ["e.event_kind='entry_triggered'"]
    params: list[str] = []
    if args.since:
        where.append("substr(e.event_at,1,10) >= ?")
        params.append(args.since)
    if args.until:
        where.append("substr(e.event_at,1,10) <= ?")
        params.append(args.until)
    sql = (
        "SELECT e.ticket_id, l.code, l.source, substr(e.event_at,1,10) AS trade_date, "
        "e.summary FROM ticket_event e JOIN ticket_lifecycle l ON l.id=e.ticket_id "
        f"WHERE {' AND '.join(where)} ORDER BY trade_date, e.ticket_id, e.id"
    )

    con = sqlite3.connect(f"file:{args.db}?mode=ro", uri=True)
    try:
        rows = con.execute(sql, params).fetchall()
    finally:
        con.close()

    # 同一票同日可能因旧二进制重复记账；按 (ticket_id, date) 只保留首条。
    seen = set()
    samples = []
    for ticket_id, code, source, trade_date, summary in rows:
        key = (ticket_id, trade_date)
        if key in seen:
            continue
        seen.add(key)
        parsed = parse_summary(summary)
        if not parsed:
            samples.append(
                {
                    "ticket_id": ticket_id,
                    "code": code,
                    "source": source,
                    "date": trade_date,
                    "status": "unparsed",
                    "summary": summary,
                }
            )
            continue
        price, low, high = parsed
        status, gap = classify(price, low, high)
        samples.append(
            {
                "ticket_id": ticket_id,
                "code": code,
                "source": source,
                "date": trade_date,
                "status": status,
                "price": price,
                "low": low,
                "high": high,
                "gap_pct": round(gap, 3),
                "summary": summary,
            }
        )

    counts = defaultdict(int)
    by_source = defaultdict(lambda: defaultdict(int))
    above_gaps: list[float] = []
    below_gaps: list[float] = []
    for sample in samples:
        counts[sample["status"]] += 1
        by_source[sample["source"]][sample["status"]] += 1
        if sample["status"] == "above":
            above_gaps.append(sample["gap_pct"])
        elif sample["status"] == "below":
            below_gaps.append(sample["gap_pct"])
    total = len(samples)
    report = {
        "since": args.since,
        "until": args.until,
        "ticket_days": total,
        "counts": dict(counts),
        "above_share_pct": round(counts["above"] / total * 100.0, 1) if total else None,
        "outside_share_pct": round(
            (counts["above"] + counts["below"]) / total * 100.0, 1
        )
        if total
        else None,
        "above_gap_pct": {
            "median": percentile(above_gaps, 0.5),
            "p90": percentile(above_gaps, 0.9),
            "max": max(above_gaps) if above_gaps else None,
        },
        "by_source": {source: dict(values) for source, values in by_source.items()},
        "samples": samples,
    }

    if args.json:
        print(json.dumps(report, ensure_ascii=False, indent=2))
    else:
        print(
            f"触发样本（票日去重） {total}｜区间内 {counts['inside']}｜"
            f"区间上方 {counts['above']}｜区间下方 {counts['below']}｜无法解析 {counts['unparsed']}"
        )
        if total:
            print(
                f"区间上方占比 {report['above_share_pct']}%｜区间外占比 {report['outside_share_pct']}%"
            )
        if above_gaps:
            print(
                f"上方偏离：中位 {percentile(above_gaps, 0.5):.2f}%｜"
                f"P90 {percentile(above_gaps, 0.9):.2f}%｜最大 {max(above_gaps):.2f}%"
            )
        for source in sorted(by_source):
            values = by_source[source]
            print(
                f"  {source}: 内 {values['inside']} / 上 {values['above']} / "
                f"下 {values['below']} / 解析失败 {values['unparsed']}"
            )
        for sample in samples:
            if sample["status"] in ("above", "below"):
                print(
                    f"  {sample['date']} #{sample['ticket_id']} {sample['code']} "
                    f"{sample['status']} {sample['gap_pct']:+.2f}% "
                    f"(现价 {sample['price']} / 区间 {sample['low']}-{sample['high']})"
                )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
