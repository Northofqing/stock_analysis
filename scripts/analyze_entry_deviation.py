#!/usr/bin/env python3
"""成交偏差离线分析（方案 5.2，只读）。

问题：`entry_proposed` 给的区间，和实际买到的价格差多少？
口径未定之前先做离线分布，不上报表。

方法：
  1. 取 `ticket_event` 里 kind='entry_proposed' 的事件，解析摘要 `低-高｜…｜放弃 X`；
  2. 找该票在**提案日之后**的第一份持仓快照（`user_position_snapshot.effective_at`），
     取 `cost_price`；
  3. 分三类：区间内 / 低于下沿 / 高于上沿；算相对区间中点的偏差。

排除：
  - 提案日之前就已经持有该票的（cost_price 是摊薄成本，不是本次成交价）；
  - 提案后没有快照的 = 未成交（或未导入），单独计数，不当作偏差。

边界：快照 cost_price 是**摊薄成本**，即使排除了老仓，也可能含手续费/多笔成交；
      结论只能用于方向性判断（给价偏高还是偏低），不能当成成交价。

用法：
    python3 scripts/analyze_entry_deviation.py [--since 2026-10-01]
"""

import argparse
import os
import re
import sqlite3
import sys

RUNTIME = os.path.expanduser("~/.local/share/stock-analysis-runtime")
MAIN_DB = f"{RUNTIME}/data/stock_analysis.db"

RANGE_RE = re.compile(r"(\d+(?:\.\d+)?)-(\d+(?:\.\d+)?)")


def ro(path):
    return sqlite3.connect(f"file:{path}?mode=ro", uri=True)


def parse_range(summary):
    if not summary:
        return None
    match = RANGE_RE.search(summary)
    if not match:
        return None
    low, high = float(match.group(1)), float(match.group(2))
    if low <= 0 or high < low:
        return None
    return low, high


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--since", default="2026-10-01")
    args = ap.parse_args()

    if not os.path.exists(MAIN_DB):
        print(f"缺少主库: {MAIN_DB}")
        return 2

    con = ro(MAIN_DB)
    events = con.execute(
        """SELECT e.ticket_id, t.code, substr(e.event_at,1,10) AS d, e.summary
             FROM ticket_event e JOIN ticket_lifecycle t ON t.id = e.ticket_id
            WHERE e.event_kind = 'entry_proposed' AND substr(e.event_at,1,10) >= ?
            ORDER BY e.event_at""",
        (args.since,),
    ).fetchall()

    print(f"=== entry_proposed 事件（{args.since} 起）===")
    print(f"共 {len(events)} 条")
    if not events:
        con.close()
        return 0

    def holdings_on(day):
        """该日（含）之前最后一份快照的持仓集合。"""
        row = con.execute(
            "SELECT snapshot_id FROM user_position_snapshot WHERE date(effective_at) <= ? ORDER BY effective_at DESC LIMIT 1",
            (day,),
        ).fetchone()
        if not row:
            return set()
        return {
            code
            for (code,) in con.execute(
                "SELECT code FROM user_position_snapshot_item WHERE snapshot_id = ?", (row[0],)
            )
        }

    def first_cost_after(code, day):
        row = con.execute(
            """SELECT s.effective_at, i.cost_price
                 FROM user_position_snapshot_item i
                 JOIN user_position_snapshot s ON s.snapshot_id = i.snapshot_id
                WHERE i.code = ? AND date(s.effective_at) > ?
                ORDER BY s.effective_at ASC LIMIT 1""",
            (code, day),
        ).fetchone()
        return row

    last_snapshot = con.execute(
        "SELECT MAX(effective_at) FROM user_position_snapshot"
    ).fetchone()[0]
    print(f"最新持仓快照: {last_snapshot}（之后的成交看不到）")

    stats = {"in_range": 0, "below": 0, "above": 0, "no_snapshot": 0, "preheld": 0, "no_price": 0}
    deviations = []
    details = []

    for ticket_id, code, day, summary in events:
        bounds = parse_range(summary)
        if not bounds:
            stats["no_price"] += 1
            continue
        low, high = bounds
        if code in holdings_on(day):
            stats["preheld"] += 1
            continue
        hit = first_cost_after(code, day)
        if not hit:
            stats["no_snapshot"] += 1
            continue
        effective_at, cost = hit
        mid = (low + high) / 2
        dev = (cost - mid) / mid * 100
        if cost < low:
            stats["below"] += 1
            verdict = "低于下沿"
        elif cost > high:
            stats["above"] += 1
            verdict = "高于上沿"
        else:
            stats["in_range"] += 1
            verdict = "区间内"
        deviations.append(dev)
        details.append((code, day, low, high, cost, verdict, dev, effective_at[:10]))

    total_measured = stats["in_range"] + stats["below"] + stats["above"]
    print(f"\n可测样本 {total_measured} 条（有成交且非老仓）")
    print(f"  区间内 {stats['in_range']}")
    print(f"  低于下沿 {stats['below']}")
    print(f"  高于上沿 {stats['above']}")
    print(
        f"未纳入：老仓 {stats['preheld']} / 提案后快照里没有该票 {stats['no_snapshot']} / 摘要解析失败 {stats['no_price']}"
    )
    if stats["no_snapshot"]:
        print(
            "  注：『快照里没有该票』= 未成交或未按提案买入，本身就是一个结论（不是数据缺失）。"
        )

    if deviations:
        deviations.sort()
        mid = deviations[len(deviations) // 2]
        mean = sum(deviations) / len(deviations)
        print(f"\n相对区间中点偏差：中位 {mid:+.2f}%，均值 {mean:+.2f}%（n={len(deviations)}）")
        print("\n明细（代码 提案日 区间 成本 判定 偏差 快照日）")
        for row in details:
            print(f"  {row[0]} {row[1]} {row[2]:.2f}-{row[3]:.2f} {row[4]:.3f} {row[5]} {row[6]:+.2f}% {row[7]}")

    print(
        "\n边界：cost_price 是摊薄成本，老仓已排除；样本量小，结论只作方向性参考，不上报表。"
    )
    con.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
