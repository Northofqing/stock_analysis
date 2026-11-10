#!/usr/bin/env python3
"""批 9 观察：触发前 3 日涨幅分组的前瞻收益（只读，描述性统计）。

对应方案 11.5 批 9「前 3 日涨幅 >10% 不触发」的 shadow 观察：
- 用 `ticket_event(kind='entry_triggered')` 的现价作为触发价；
- 前 3 日涨幅 = D-1 收盘 / D-4 收盘 - 1（只用交易日）；
- 前瞻收益 = D+1 / D+3 收盘相对触发价；
- 分组：>10%、3~10%、<=3%。

只做描述性统计，不做显著性结论；样本不够时明确打印 n。

用法：
    python3 scripts/ticket_pre3_gain_audit.py --db <stock_analysis.db> [--since 2026-10-28]
"""
from __future__ import annotations

import argparse
import re
import sqlite3
import sys
from collections import defaultdict

PRICE_RE = re.compile(r"现价\s*([0-9]+(?:\.[0-9]+)?)")


def median(values: list[float]) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    mid = len(ordered) // 2
    if len(ordered) % 2:
        return ordered[mid]
    return (ordered[mid - 1] + ordered[mid]) / 2


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--db", required=True)
    parser.add_argument("--since", default=None)
    args = parser.parse_args()

    con = sqlite3.connect(f"file:{args.db}?mode=ro", uri=True)
    try:
        dates = [row[0] for row in con.execute("SELECT DISTINCT date FROM stock_daily ORDER BY date")]
        index = {date: i for i, date in enumerate(dates)}
        where = ["e.event_kind='entry_triggered'"]
        params: list[str] = []
        if args.since:
            where.append("substr(e.event_at,1,10) >= ?")
            params.append(args.since)
        rows = con.execute(
            "SELECT l.code, substr(e.event_at,1,10), e.summary "
            "FROM ticket_event e JOIN ticket_lifecycle l ON l.id=e.ticket_id "
            f"WHERE {' AND '.join(where)} ORDER BY e.event_at",
            params,
        ).fetchall()

        closes: dict[tuple[str, str], float] = {}

        def close_on(code: str, date: str) -> float | None:
            key = (code, date)
            if key not in closes:
                row = con.execute(
                    "SELECT close FROM stock_daily WHERE code=? AND date=?", (code, date)
                ).fetchone()
                closes[key] = row[0] if row else None  # type: ignore[assignment]
            return closes[key]  # type: ignore[return-value]

        groups: dict[str, list[tuple[float | None, float | None]]] = defaultdict(list)
        skipped = 0
        for code, date, summary in rows:
            match = PRICE_RE.search(summary or "")
            if not match or date not in index:
                skipped += 1
                continue
            price = float(match.group(1))
            i = index[date]
            if i < 4 or i + 1 >= len(dates):
                skipped += 1
                continue
            base = close_on(code, dates[i - 4])
            prev = close_on(code, dates[i - 1])
            if not base or not prev:
                skipped += 1
                continue
            pre3 = (prev / base - 1.0) * 100.0
            next1 = close_on(code, dates[i + 1]) if i + 1 < len(dates) else None
            next3 = close_on(code, dates[i + 3]) if i + 3 < len(dates) else None
            fwd1 = (next1 / price - 1.0) * 100.0 if next1 else None
            fwd3 = (next3 / price - 1.0) * 100.0 if next3 else None
            if pre3 > 10.0:
                group = ">10%"
            elif pre3 >= 3.0:
                group = "3~10%"
            else:
                group = "<=3%"
            groups[group].append((fwd1, fwd3))
    finally:
        con.close()

    if not rows:
        print("没有 entry_triggered 事件")
        return 0
    print(f"触发事件 {len(rows)} 条；可用 {len(rows) - skipped} 条；跳过 {skipped} 条（缺前 3 日/次日数据）")
    header = f"{'分组':<8}{'n':>4}{'D+1胜率':>9}{'D+1均值':>9}{'D+1中位':>9}{'D+3 n':>7}{'D+3均值':>9}{'D+3中位':>9}"
    print(header)
    for group in ("<=3%", "3~10%", ">10%"):
        samples = groups.get(group, [])
        fwd1 = [x for x, _ in samples if x is not None]
        fwd3 = [y for _, y in samples if y is not None]
        if not samples:
            print(f"{group:<8}{0:>4}")
            continue
        win = sum(1 for x in fwd1 if x > 0) / len(fwd1) * 100 if fwd1 else 0
        avg1 = sum(fwd1) / len(fwd1) if fwd1 else 0
        avg3 = sum(fwd3) / len(fwd3) if fwd3 else 0
        print(
            f"{group:<8}{len(samples):>4}{win:>8.1f}%{avg1:>+9.2f}%"
            f"{(median(fwd1) or 0):>+8.2f}%{len(fwd3):>7}{avg3:>+9.2f}%"
            f"{(median(fwd3) or 0):>+8.2f}%"
        )
    print("\n说明：描述性统计，未做显著性检验；D+3 样本通常少于 D+1。")
    return 0


if __name__ == "__main__":
    sys.exit(main())
