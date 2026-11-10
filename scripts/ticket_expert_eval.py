#!/usr/bin/env python3
"""6.8 预注册确认性评估（只读，不改规则）。

按 `docs/research/ticket_expert-preregistration-2026-11-10.json` 的冻结条款：
- 样本单位：一次 `entry_triggered` 事件；(code, trade_date) 去重取最早；
- 触发价：事件 summary 里的「现价」；
- 收益：D+1 / D+3 收盘 / 触发价 - 1，扣双边成本（默认 0.2%，另报 0.1%/0.3% 与未扣）；
- 分组：Q1 前 3 日涨幅、Q2 竞价涨幅、Q3 有/无底仓；
- 每组报 n、均值、中位、胜率、95% bootstrap CI（默认 10000 次）；
- 停止规则：确认性窗口至少 20 个交易日且组 n>=30，否则只报样本量，不判确认。

用法：
    python3 scripts/ticket_expert_eval.py --root <运行根>
    python3 scripts/ticket_expert_eval.py --root <运行根> --window exploratory
    python3 scripts/ticket_expert_eval.py --root <运行根> --json-out /tmp/eval.json
"""

from __future__ import annotations

import argparse
import json
import random
import re
import sqlite3
import statistics
import sys
from pathlib import Path

PRICE_RE = re.compile(r"现价\s*([0-9]+(?:\.[0-9]+)?)")
DEFAULT_PREREG = "data/research/ticket_expert/ticket_expert-preregistration-2026-11-10.json"
CAPITAL = 10000


def bootstrap_ci(values: list[float], rng: random.Random, rounds: int) -> tuple[float, float] | None:
    if len(values) < 2:
        return None
    means = []
    n = len(values)
    for _ in range(rounds):
        means.append(sum(values[rng.randrange(n)] for _ in range(n)) / n)
    means.sort()
    return means[int(0.025 * rounds)], means[int(0.975 * rounds)]


def load_bundle(root: Path, date: str, code: str) -> dict:
    for suffix in ("03", "02", "01"):
        path = root / "data" / "ticket_evidence" / date / f"{code}-{suffix}.json"
        if path.exists():
            try:
                return json.loads(path.read_text(encoding="utf-8"))
            except Exception:  # noqa: BLE001
                continue
    return {}


def group_of_pre3(value: float) -> str:
    if value > 10.0:
        return "pre3_gt_10"
    if value >= 3.0:
        return "pre3_3_to_10"
    return "pre3_le_3"


def group_of_auction(value: float | None) -> str | None:
    if value is None:
        return None
    if value < -2.0:
        return "auction_lt_-2"
    if value <= 2.0:
        return "auction_-2_to_2"
    if value <= 6.0:
        return "auction_2_to_6"
    return "auction_gt_6"


def summarize(samples: list[dict], key: str, cost: float, rng: random.Random, rounds: int) -> dict:
    gross = [s[key] for s in samples if s.get(key) is not None]
    net = [value - cost for value in gross]
    ci = bootstrap_ci(net, rng, rounds)
    return {
        "n": len(net),
        "mean_gross_pct": round(statistics.mean(gross), 4) if gross else None,
        "mean_net_pct": round(statistics.mean(net), 4) if net else None,
        "median_net_pct": round(statistics.median(net), 4) if net else None,
        "win_rate_pct": round(100.0 * sum(1 for v in net if v > 0) / len(net), 2) if net else None,
        "ci95_net": [round(ci[0], 4), round(ci[1], 4)] if ci else None,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--db", type=Path, default=None)
    parser.add_argument("--prereg", type=Path, default=None)
    parser.add_argument("--window", choices=["confirmatory", "exploratory"], default="confirmatory")
    parser.add_argument("--start", default=None)
    parser.add_argument("--end", default=None)
    parser.add_argument("--bootstrap", type=int, default=CAPITAL)
    parser.add_argument("--json-out", type=Path, default=None)
    args = parser.parse_args()

    root = args.root
    db = args.db or root / "data" / "stock_analysis.db"
    prereg_path = args.prereg or root / DEFAULT_PREREG
    if not prereg_path.exists():
        print(f"缺预注册文件: {prereg_path}")
        return 2
    prereg = json.loads(prereg_path.read_text(encoding="utf-8"))
    if prereg.get("status") != "signed_frozen":
        print(f"预注册未签字冻结: status={prereg.get('status')!r}")
        return 2

    window = prereg[f"{args.window}_window"]
    start = args.start or window["start"]
    end = args.end or window.get("end") or "9999-12-31"
    costs = prereg["costs"]
    base_cost = float(costs["base_round_trip_pct"])
    sensitivity = [float(value) for value in costs["sensitivity_pct"]]

    conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    try:
        dates = [row[0] for row in conn.execute("SELECT DISTINCT date FROM stock_daily ORDER BY date")]
        index = {date: i for i, date in enumerate(dates)}
        rows = conn.execute(
            "SELECT e.ticket_id, l.code, substr(e.event_at,1,10) AS d, e.event_at, e.summary "
            "FROM ticket_event e JOIN ticket_lifecycle l ON l.id=e.ticket_id "
            "WHERE e.event_kind='entry_triggered' AND substr(e.event_at,1,10) BETWEEN ? AND ? "
            "ORDER BY e.event_at",
            (start, end),
        ).fetchall()

        closes: dict[tuple[str, str], float | None] = {}

        def close_on(code: str, date: str) -> float | None:
            key = (code, date)
            if key not in closes:
                row = conn.execute(
                    "SELECT close FROM stock_daily WHERE code=? AND date=?", (code, date)
                ).fetchone()
                closes[key] = row[0] if row else None
            return closes[key]

        samples: list[dict] = []
        skipped: dict[str, int] = {}
        seen: set[tuple[str, str]] = set()
        bundle_missing = 0
        auction_missing = 0
        position_missing = 0
        for _ticket_id, code, date, _event_at, summary in rows:
            if (code, date) in seen:
                skipped["dedupe"] = skipped.get("dedupe", 0) + 1
                continue
            seen.add((code, date))
            match = PRICE_RE.search(summary or "")
            if not match:
                skipped["no_trigger_price"] = skipped.get("no_trigger_price", 0) + 1
                continue
            price = float(match.group(1))
            if price <= 0:
                skipped["bad_trigger_price"] = skipped.get("bad_trigger_price", 0) + 1
                continue
            if date not in index:
                skipped["no_trade_date"] = skipped.get("no_trade_date", 0) + 1
                continue
            i = index[date]
            if i < 4:
                skipped["no_D-4"] = skipped.get("no_D-4", 0) + 1
                continue
            base = close_on(code, dates[i - 4])
            prev = close_on(code, dates[i - 1])
            next1 = close_on(code, dates[i + 1]) if i + 1 < len(dates) else None
            if not base or not prev:
                skipped["no_D-4_or_D-1"] = skipped.get("no_D-4_or_D-1", 0) + 1
                continue
            if not next1:
                skipped["no_D+1"] = skipped.get("no_D+1", 0) + 1
                continue
            bundle = load_bundle(root, date, code)
            name = bundle.get("name") or ""
            if not bundle:
                bundle_missing += 1
            if "ST" in name.upper() or "退" in name:
                skipped["st_or_delisting"] = skipped.get("st_or_delisting", 0) + 1
                continue
            next3 = close_on(code, dates[i + 3]) if i + 3 < len(dates) else None
            pricing = bundle.get("pricing") or {}
            auction_pct = pricing.get("auction_pct")
            auction_ref = pricing.get("quoted_price") or pricing.get("auction_price")
            auction_d1 = None
            if auction_ref and auction_ref > 0:
                auction_d1 = round((next1 / auction_ref - 1.0) * 100.0, 4)
            if auction_pct is None:
                auction_missing += 1
            portfolio = bundle.get("portfolio") or {}
            quantity = portfolio.get("position_quantity")
            if quantity is None:
                position_missing += 1
                pos_group = None
            else:
                pos_group = "has_position" if quantity > 0 else "no_position"
            samples.append(
                {
                    "code": code,
                    "date": date,
                    "name": name,
                    "price": price,
                    "pre3_pct": round((prev / base - 1.0) * 100.0, 4),
                    "auction_pct": auction_pct,
                    "position_quantity": quantity,
                    "d1_pct": round((next1 / price - 1.0) * 100.0, 4),
                    "auction_d1_pct": auction_d1,
                    "d3_pct": round((next3 / price - 1.0) * 100.0, 4) if next3 else None,
                }
            )
    finally:
        conn.close()

    rng = random.Random(20261110)
    trading_days = sum(1 for date in dates if start <= date <= min(end, dates[-1] if dates else end))
    print(f"预注册: {prereg_path}（{prereg['status']}）")
    print(f"窗口: {args.window} {start} ~ {end}；窗口内交易日 {trading_days}")
    print(f"触发事件: {len(rows)} 条；入样 {len(samples)} 条；排除 " + ", ".join(f"{k}={v}" for k, v in sorted(skipped.items())))
    print(
        f"证据包缺失 {bundle_missing} 条；auction_pct 缺失 {auction_missing} 条；"
        f"position_quantity 缺失 {position_missing} 条"
    )

    report: dict = {
        "prereg": str(prereg_path),
        "window": {"kind": args.window, "start": start, "end": end, "trading_days": trading_days},
        "triggers": len(rows),
        "samples": len(samples),
        "skipped": skipped,
        "evidence": {
            "bundle_missing": bundle_missing,
            "auction_pct_missing": auction_missing,
            "position_quantity_missing": position_missing,
        },
        "questions": {},
    }

    def by_group(field: str, wanted: list[str], metric: str = "d1_pct") -> dict:
        out = {}
        for group in wanted:
            subset = [s for s in samples if s.get(field) == group]
            out[group] = {
                "metric": metric,
                "d1": summarize(subset, metric, base_cost, rng, args.bootstrap),
                "d3": summarize(subset, "d3_pct", base_cost, rng, args.bootstrap),
                "n": len(subset),
                "base_cost": base_cost,
                "sensitivity_mean_net_pct": {
                    str(cost): round(
                        statistics.mean([s[metric] - cost for s in subset if s.get(metric) is not None]),
                        4,
                    )
                    if subset
                    else None
                    for cost in sensitivity
                },
            }
        return out

    def group_of_position(sample: dict) -> str | None:
        quantity = sample.get("position_quantity")
        if quantity is None:
            return None
        return "has_position" if quantity > 0 else "no_position"

    # 预计算分组，避免 field 名称魔法。
    for sample in samples:
        sample["pre3_group"] = group_of_pre3(sample["pre3_pct"])
        sample["auction_group"] = group_of_auction(sample.get("auction_pct"))
        sample["position_group"] = group_of_position(sample)

    comparisons = {
        "Q1_pre3_gain": ("pre3_group", ["pre3_gt_10", "pre3_le_3"], "d1_pct"),
        "Q2_low_open": ("auction_group", ["auction_lt_-2", "auction_2_to_6"], "auction_d1_pct"),
        "Q3_position_bias": ("position_group", ["has_position", "no_position"], "d1_pct"),
    }
    for question, (field, groups, metric) in comparisons.items():
        result = by_group(field, groups, metric)
        report["questions"][question] = result
        print(f"\n{question}")
        for group in groups:
            d1 = result[group]["d1"]
            d3 = result[group]["d3"]
            ci = d1["ci95_net"]
            ci_text = f"[{ci[0]:+.2f},{ci[1]:+.2f}]" if ci else "—"
            print(
                f"  {group:<16} n={d1['n']:>3} D+1均值(净){d1['mean_net_pct'] if d1['mean_net_pct'] is not None else '—':>8} "
                f"中位{d1['median_net_pct'] if d1['median_net_pct'] is not None else '—':>8} "
                f"胜率{d1['win_rate_pct'] if d1['win_rate_pct'] is not None else '—':>6} CI95 {ci_text} "
                f"| D+3 n={d3['n']} 均值(净){d3['mean_net_pct'] if d3['mean_net_pct'] is not None else '—'}"
            )
        enough = all(result[g]["d1"]["n"] >= 30 for g in groups) and trading_days >= 20
        print(f"  结论: {'可评估' if enough else '样本不足（需 20 个交易日且每组 n>=30），不判确认/否定'}")

    if args.json_out:
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
        print(f"\nJSON: {args.json_out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
