#!/usr/bin/env python3
"""多 Agent 深度研判报告的 EV / 胜率校准（离线只读）。

读 reports/details/{date}_{code}.md 与主库 stock_daily，回答三个问题：
1. 报告给的档位（买入/观望/减持…）之后 5/10 个交易日的真实收益如何；
2. 报告写的“乐观目标/悲观目标”谁先被触及，实际命中率与声明的胜率差多少；
3. 报告算出的 EV 与真实 10 日收益是否同向（EV 为负的票是否真的更差）。

用法：
  python3 scripts/ev_calibration.py \
      --reports ~/.local/share/stock-analysis-runtime/reports/details \
      --db ~/.local/share/stock-analysis-runtime/data/stock_analysis.db \
      --out /tmp/ev_calibration.csv

只读：DB 以 mode=ro 打开，不写报告、不调网络。
"""

import argparse
import glob
import os
import re
import sqlite3
import statistics as st
from collections import Counter, defaultdict

BANDS = [
    (70, "强烈买入"),
    (55, "买入"),
    (45, "观望"),
    (30, "减持"),
    (0, "强烈卖出"),
]
BAND_ALIASES = {
    "强烈买入": ["强烈买入", "强烈看好", "重仓买入"],
    "买入": ["买入", "增持", "偏多", "看多"],
    "观望": ["观望", "中性", "持有", "观察"],
    "减持": ["减持", "卖出", "规避", "回避", "偏空", "减仓"],
    "强烈卖出": ["强烈卖出", "清仓", "坚决卖出"],
}


def band_of(score):
    for floor, name in BANDS:
        if score >= floor:
            return name
    return "强烈卖出"


def parse_report(path):
    text = open(path, encoding="utf-8").read()
    base = os.path.basename(path)[:-3]
    date, code = base.split("_")
    row = {"date": date, "code": code, "band": None, "win": None, "odds": None,
           "ev": None, "opt_target": None, "pes_target": None, "stop": None}
    m = re.search(r"加权综合[：:]\s*(\d+)/100", text)
    if m:
        row["band"] = band_of(int(m.group(1)))
    else:
        dirs = re.findall(r"方向：\s*\**\s*([^\n（(*]+)", text)
        got = dirs[0] if dirs else ""
        for name, words in BAND_ALIASES.items():
            if any(w in got for w in words):
                row["band"] = name
                break
    m = re.search(r"胜率\s*=[^\n]*?=\s*\**\s*([\d.]+)%", text)
    if m:
        row["win"] = float(m.group(1))
    m = re.search(r"赔率\s*=\s*[\d.]+/[\d.]+\s*=\s*\**([\d.]+)", text)
    if m:
        row["odds"] = float(m.group(1))
    ev_line = [l for l in text.split("\n") if re.search(r"EV\s*=", l)]
    if ev_line:
        nums = re.findall(r"([+\-−]?[\d.]+)%", ev_line[-1])
        if nums:
            row["ev"] = float(nums[-1].replace("−", "-"))
    seg = text.split("## 情景树")[-1].split("## EV 算式")[0] if "## 情景树" in text else ""
    for key, label in (("opt_target", "乐观"), ("pes_target", "悲观")):
        m = re.search(label + r"（P=\d+%）[^\n]*?目标价\s*¥([\d.]+)", seg)
        if m:
            row[key] = float(m.group(1))
    m = re.search(r"止损价：\s*¥([\d.]+)", text)
    if m:
        row["stop"] = float(m.group(1))
    # 报告的“数据时点”才是前向收益的起点（报告是盘后基于该日收盘写的）；
    # 老报告没有尾注时退回文件名日期。
    m = re.search(r"数据时点：\s*(\d{4}-\d{2}-\d{2})", text)
    row["data_date"] = m.group(1) if m else f"{date[:4]}-{date[4:6]}-{date[6:]}"
    return row


def forward_bars(conn, code, date, limit=10):
    return conn.execute(
        "SELECT date, high, low, close FROM stock_daily "
        "WHERE code=? AND date>? ORDER BY date LIMIT ?",
        (code, date, limit),
    ).fetchall()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--reports", default="reports/details")
    ap.add_argument("--db", required=True)
    ap.add_argument("--out", default="/tmp/ev_calibration.csv")
    args = ap.parse_args()

    conn = sqlite3.connect(f"file:{args.db}?mode=ro", uri=True)
    rows = []
    for path in sorted(glob.glob(os.path.join(args.reports, "*.md"))):
        if not re.match(r"\d{8}_\d{6}\.md$", os.path.basename(path)):
            continue
        try:
            row = parse_report(path)
        except Exception:
            continue
        bars = forward_bars(conn, row["code"], row["data_date"])
        if not bars:
            continue
        entry = None
        base = conn.execute(
            "SELECT close FROM stock_daily WHERE code=? AND date<=? ORDER BY date DESC LIMIT 1",
            (row["code"], row["data_date"]),
        ).fetchone()
        if base:
            entry = base[0]
        if not entry or entry <= 0:
            continue
        row["ret5"] = bars[min(4, len(bars) - 1)][3] / entry - 1 if bars else None
        row["ret10"] = bars[min(9, len(bars) - 1)][3] / entry - 1 if bars else None
        wins = [b[3] for b in bars]
        row["max_gain"] = max(wins) / entry - 1
        # 谁先触及：乐观目标 vs 悲观目标（同日按先高后低保守计）
        first = None
        if row["opt_target"] and row["pes_target"]:
            for _, high, low, _close in bars:
                if high >= row["opt_target"]:
                    first = "opt"
                    break
                if low <= row["pes_target"]:
                    first = "pes"
                    break
        row["first"] = first
        rows.append(row)

    if not rows:
        print("没有可校准的报告（检查 --reports / --db）")
        return

    print(f"报告数: {len(rows)}  有前向行情: {sum(1 for r in rows if r['ret5'] is not None)}")
    by_band = defaultdict(list)
    for r in rows:
        if r["band"]:
            by_band[r["band"]].append(r)
    print("\n按档位（报告日收盘买入，持有 5/10 个交易日）：")
    print(f"{'档位':>6} {'样本':>4} {'5日均收益':>9} {'5日正收益':>9} {'10日均收益':>10} {'10日正收益':>10}")
    for band in ["买入", "观望", "减持", "强烈卖出", "强烈买入"]:
        items = by_band.get(band, [])
        if not items:
            continue
        r5 = [r["ret5"] for r in items if r["ret5"] is not None]
        r10 = [r["ret10"] for r in items if r["ret10"] is not None]
        print(f"{band:>6} {len(items):>4} {st.mean(r5)*100:>8.2f}% "
              f"{sum(1 for x in r5 if x > 0)/len(r5):>8.0%} "
              f"{st.mean(r10)*100:>9.2f}% {sum(1 for x in r10 if x > 0)/len(r10):>9.0%}")

    both = [r for r in rows if r["first"]]
    if both:
        opt = sum(1 for r in both if r["first"] == "opt")
        declared = [r["win"] for r in both if r["win"] is not None]
        print(f"\n情景校准（乐观/悲观目标都可解析）: {len(both)} 份")
        print(f"  先到乐观目标: {opt}/{len(both)} = {opt/len(both):.0%}")
        if declared:
            print(f"  报告声明胜率均值: {st.mean(declared):.1f}%  实际校准值: {opt/len(both)*100:.1f}%")

    ev_rows = [r for r in rows if r["ev"] is not None and r["ret10"] is not None]
    if ev_rows:
        pos = [r for r in ev_rows if r["ev"] > 0]
        neg = [r for r in ev_rows if r["ev"] <= 0]
        print(f"\nEV 校准: {len(ev_rows)} 份")
        if pos:
            print(f"  EV>0: {len(pos)} 份，真实 10 日均收益 {st.mean(r['ret10'] for r in pos)*100:+.2f}%")
        if neg:
            print(f"  EV≤0: {len(neg)} 份，真实 10 日均收益 {st.mean(r['ret10'] for r in neg)*100:+.2f}%")
        xs = [r["ev"] for r in ev_rows]
        ys = [r["ret10"] * 100 for r in ev_rows]
        mx, my = st.mean(xs), st.mean(ys)
        cov = sum((x - mx) * (y - my) for x, y in zip(xs, ys))
        den = (sum((x - mx) ** 2 for x in xs) * sum((y - my) ** 2 for y in ys)) ** 0.5
        print(f"  EV 与 10 日收益相关系数 r = {cov/den:+.2f}" if den else "  r 无法计算")

    with open(args.out, "w", encoding="utf-8") as fh:
        cols = ["date", "code", "band", "win", "odds", "ev", "opt_target", "pes_target",
                "ret5", "ret10", "max_gain", "first"]
        fh.write(",".join(cols) + "\n")
        for r in rows:
            fh.write(",".join("" if r.get(c) is None else str(r.get(c)) for c in cols) + "\n")
    print(f"\n明细已写: {args.out}")


if __name__ == "__main__":
    main()
