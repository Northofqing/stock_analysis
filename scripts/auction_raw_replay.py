#!/usr/bin/env python3
"""从 data/auction_raw/<date>.json 重算竞价分档与参考价（只读）。

与 Rust `src/monitor/ticket_auction_price.rs` 的口径保持一致：
- > +6%: SkipHigh，不给价
- +2% ~ +6%: Light，给竞价价
- -2% ~ +2%: EnsureFill，给竞价价 * 1.005
- < -2%: Bargain，给竞价价

用法：
  python3 scripts/auction_raw_replay.py --file data/auction_raw/2026-10-26.json
  python3 scripts/auction_raw_replay.py --file ... --code 001216 \
      --expect-tier Bargain --expect-price 26.60
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ENSURE_FILL_MARKUP = 0.005
MAX_AUCTION_PCT = 6.0


def tier_for(auction_pct: float) -> str:
    if auction_pct > MAX_AUCTION_PCT:
        return "SkipHigh"
    if auction_pct >= 2.0:
        return "Light"
    if auction_pct >= -2.0:
        return "EnsureFill"
    return "Bargain"


def quote_price(auction_price: float, tier: str) -> float | None:
    if auction_price <= 0.0:
        return None
    if tier == "SkipHigh":
        return None
    if tier == "EnsureFill":
        return auction_price * (1.0 + ENSURE_FILL_MARKUP)
    return auction_price


def replay(payload: dict) -> dict:
    records = payload.get("records")
    if not isinstance(records, list):
        raise ValueError("raw payload has no records[]")
    rows = []
    for raw in records:
        code = raw.get("code")
        price = raw.get("auction_price")
        pct = raw.get("auction_pct")
        if not isinstance(code, str) or not isinstance(price, (int, float)) or not isinstance(
            pct, (int, float)
        ):
            rows.append(
                {
                    "code": code,
                    "excluded": "auction_price/auction_pct missing or non-numeric",
                }
            )
            continue
        tier = tier_for(float(pct))
        quote = quote_price(float(price), tier)
        rows.append(
            {
                "code": code,
                "name": raw.get("name"),
                "auction_price": float(price),
                "auction_pct": float(pct),
                "tier": tier,
                "quote_price": None if quote is None else round(quote, 2),
                "source_at": raw.get("source_at"),
                "observed_at": raw.get("observed_at"),
                "batch_id": raw.get("batch_id"),
                "provider": raw.get("provider"),
            }
        )
    return {
        "date": payload.get("date"),
        "count": len(rows),
        "rows": rows,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--file", required=True)
    parser.add_argument("--code", help="只校验该代码")
    parser.add_argument("--expect-tier", help="期望分档，例如 Bargain")
    parser.add_argument("--expect-price", type=float, help="期望参考价")
    parser.add_argument("--json", action="store_true", help="输出完整 JSON")
    args = parser.parse_args()

    path = Path(args.file)
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
        result = replay(payload)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"replay failed: {error}", file=sys.stderr)
        return 2

    if args.json:
        print(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True))
    else:
        print(f"{result['date']} records={result['count']}")
        for row in result["rows"]:
            if "excluded" in row:
                print(f"{row.get('code')}: EXCLUDED {row['excluded']}")
            else:
                quote = "—" if row["quote_price"] is None else f"{row['quote_price']:.2f}"
                print(
                    f"{row['code']} {row.get('name') or ''} "
                    f"竞价 {row['auction_price']:.2f} ({row['auction_pct']:+.2f}%) "
                    f"→ {row['tier']} {quote}"
                )

    if args.code:
        matches = [row for row in result["rows"] if row.get("code") == args.code]
        if len(matches) != 1:
            print(f"expect-code {args.code}: found {len(matches)} rows", file=sys.stderr)
            return 1
        row = matches[0]
        if "excluded" in row:
            print(f"{args.code}: excluded, cannot assert", file=sys.stderr)
            return 1
        if args.expect_tier and row["tier"] != args.expect_tier:
            print(
                f"{args.code}: tier={row['tier']} expected={args.expect_tier}",
                file=sys.stderr,
            )
            return 1
        if args.expect_price is not None and row["quote_price"] != round(args.expect_price, 2):
            print(
                f"{args.code}: quote={row['quote_price']} expected={args.expect_price:.2f}",
                file=sys.stderr,
            )
            return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
