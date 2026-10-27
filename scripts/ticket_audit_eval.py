#!/usr/bin/env python3
"""对 WP6 证据包做只读评估，不产生交易结论。

评估只使用证据包里的 raw 数据：
- 有 DecisionContext 时，记录其 entry range，不把区间中点伪装成成交价；
- T+1/T+3/T+5 只按 as_of 之后实际存在的日线计算；
- MFE/MAE 需要 high/low；high/low 缺失就输出 null；
- 没有成交证据时 verdict 固定为 INCONCLUSIVE / UNKNOWN。

用法：
    python3 scripts/ticket_audit_eval.py --pack <evidence-pack>
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


def read_json(path: Path, default: Any) -> Any:
    try:
        return json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        return default


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    rows = []
    try:
        for line in path.read_text().splitlines():
            if line.strip():
                rows.append(json.loads(line))
    except (OSError, json.JSONDecodeError):
        return []
    return rows


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pack", required=True)
    args = parser.parse_args()
    pack = Path(args.pack)
    gaps = read_json(pack / "gaps.json", [])
    tickets = read_json(pack / "raw" / "ticket_lifecycle.json", [])
    decisions = read_jsonl(pack / "raw" / "ticket_decision.jsonl")
    bars = read_jsonl(pack / "raw" / "stock_daily.jsonl")

    if not tickets:
        result = {
            "status": "FAIL",
            "reason": "no ticket_lifecycle rows",
            "verdict": "INCONCLUSIVE",
        }
        (pack / "eval.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
        return 1

    as_of = pack.name if len(pack.name) == 10 else tickets[0].get("open_date")

    closes = [
        row
        for row in bars
        if row.get("date") and row.get("close") is not None
    ]
    closes.sort(key=lambda row: row["date"])
    before = [row for row in closes if row["date"] <= as_of]
    after = [row for row in closes if row["date"] > as_of]
    base = before[-1] if before else None

    def pct(value: float | None, baseline: float | None) -> float | None:
        if value is None or baseline in (None, 0):
            return None
        return (value - baseline) / baseline * 100.0

    def nth_return(n: int) -> dict[str, Any]:
        if not base or len(after) < n:
            return {"date": None, "close": None, "pct": None, "status": "MISSING"}
        row = after[n - 1]
        return {
            "date": row.get("date"),
            "close": row.get("close"),
            "pct": pct(row.get("close"), base.get("close")),
            "status": "OBSERVED",
        }

    highs = [float(row["high"]) for row in after if row.get("high") is not None]
    lows = [float(row["low"]) for row in after if row.get("low") is not None]
    mfe = pct(max(highs), base.get("close")) if highs and base else None
    mae = pct(min(lows), base.get("close")) if lows and base else None

    entry_reference: dict[str, Any] = {
        "status": "UNKNOWN",
        "low": None,
        "high": None,
        "reason": "no DecisionContext",
    }
    if decisions:
        decision = decisions[0]
        low = decision.get("price_low")
        high = decision.get("price_high")
        if low is not None or high is not None:
            entry_reference = {
                "status": "REFERENCE_RANGE_ONLY",
                "low": low,
                "high": high,
                "reason": "DecisionContext range; no fill evidence",
            }
        else:
            entry_reference = {
                "status": "UNKNOWN",
                "low": None,
                "high": None,
                "reason": "DecisionContext has no price range",
            }

    observed = sum(1 for item in (nth_return(1), nth_return(3), nth_return(5)) if item["status"] == "OBSERVED")
    result = {
        "status": "PARTIAL" if observed else "FAIL",
        "verdict": "INCONCLUSIVE",
        "code": tickets[0].get("code"),
        "as_of": as_of,
        "base_close": base,
        "entry_reference": entry_reference,
        "returns": {
            "t1": nth_return(1),
            "t3": nth_return(3),
            "t5": nth_return(5),
        },
        "mfe_pct": mfe,
        "mae_pct": mae,
        "gaps": gaps,
        "limits": [
            "收益按收盘价计算，不代表可成交价",
            "没有原始成交/排队证据，不能计算成交概率",
            "样本不足时 verdict 保持 INCONCLUSIVE",
        ],
    }
    (pack / "eval.json").write_text(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n")
    print(json.dumps({"status": result["status"], "t1": result["returns"]["t1"], "entry": entry_reference}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
