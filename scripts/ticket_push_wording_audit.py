#!/usr/bin/env python3
"""6.10 用户可见推送文案审计（只读 data/push_log/<date>）。

按首行识别模板，检查参考语义与禁止词：
- 09:25 竞价：reference 时必须有「竞价参考价」，不得有「买价」，
  且必须保留「T+1 不可卖」；
- 候选触发（含 ticket 盘中 `⚡ 突破确认`）：不得有「低吸参考」，必须有
  「参考区间」与「非自动买入」；
- 价格失效（`⚠ 价格失效` / 候选失效）：不得出现买入语义；
- 持仓建议：必须有「非下单指令」；
- 定向监控：只检查不出现「保证成交/可执行」。
当前日期是切换前/后的基线，--expect reference 才把违规作为失败。
"""
from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path
from typing import Any


def classify(first_line: str) -> str:
    if "09:25" in first_line and "竞价" in first_line:
        return "auction_0925"
    # ticket 盘中触发走 Hit::render()，首行是「⚡ 突破确认」；
    # legacy 候选模板首行是「候选触发」，两者都按候选触发口径审计。
    if "突破确认" in first_line or "候选触发" in first_line:
        return "candidate_triggered"
    if "价格失效" in first_line or "候选失效" in first_line:
        return "price_invalidated"
    if "持仓建议" in first_line:
        return "holding_plan"
    if "定向监控" in first_line:
        return "intraday_watch"
    return "other"


def audit_text(text: str, kind: str, expect: str) -> list[str]:
    errors: list[str] = []
    if kind == "auction_0925":
        if expect == "reference":
            if "竞价参考价" not in text:
                errors.append("缺少「竞价参考价」")
            if "买价" in text:
                errors.append("仍出现「买价」")
        if "T+1 不可卖" not in text:
            errors.append("缺少「T+1 不可卖」")
    elif kind == "candidate_triggered":
        if "低吸参考" in text:
            errors.append("仍出现「低吸参考」")
        if "参考区间" not in text:
            errors.append("缺少「参考区间」")
        if "非自动买入" not in text:
            errors.append("缺少「非自动买入」")
    elif kind == "price_invalidated":
        for word in ("买价", "低吸", "建议买入", "保证成交"):
            if word in text:
                errors.append(f"出现「{word}」")
    elif kind == "holding_plan":
        if "非下单指令" not in text:
            errors.append("缺少「非下单指令」")
    elif kind == "intraday_watch":
        for word in ("保证成交", "可执行"):
            if word in text:
                errors.append(f"出现「{word}」")
    for word in ("保证成交", "保证买入"):
        if word in text:
            errors.append(f"出现「{word}」")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", required=True)
    parser.add_argument("--date", required=True)
    parser.add_argument("--expect", choices=("reference", "legacy", "any"), default="any")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    directory = Path(args.root) / "data" / "push_log" / args.date
    if not directory.is_dir():
        print(f"push_log 目录不存在: {directory}")
        return 2

    counts: Counter[str] = Counter()
    failures: list[dict[str, Any]] = []
    files = 0
    for path in sorted(directory.glob("*.md")):
        files += 1
        text = path.read_text(encoding="utf-8", errors="replace")
        first = text.splitlines()[0] if text.splitlines() else ""
        kind = classify(first)
        counts[kind] += 1
        errors = audit_text(text, kind, args.expect)
        if errors:
            failures.append(
                {"file": path.name, "kind": kind, "errors": errors, "head": first[:120]}
            )

    result = {
        "date": args.date,
        "files": files,
        "kinds": dict(counts),
        "expect": args.expect,
        "failures": failures,
    }
    if args.json:
        print(json.dumps(result, ensure_ascii=False, indent=2))
    else:
        print(f"{args.date} push_log={files} kinds={dict(counts)} expect={args.expect}")
        for item in failures[:20]:
            print(f"{item['file']} [{item['kind']}] " + "；".join(item["errors"]))
        if len(failures) > 20:
            print(f"... 其余 {len(failures)-20} 个")
    if args.expect == "reference" and failures:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
