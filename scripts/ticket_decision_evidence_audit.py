#!/usr/bin/env python3
"""6.9 独立审计：决策证据包（data/ticket_evidence/<date>/<code>-NN.json）。

与 `ticket_evidence_audit.py`（WP6 事后包，校验 manifest）不同，这里审的是
6.3 落盘的「当次决策看到了什么」：

1. schema / 目录 / 文件名 / 必填字段；
2. 竞价路径按 tier 重算应给价（不复用被测代码的常量表，独立实现）；
3. 未知值不得伪造来源（streak=null 时 streak_source_date 必须为 null）；
4. 文件 sha256 与 `ticket_event(kind='evidence_bundle')` 的 summary 对账；
5. 统计 missing 字段分布，输出 PASS / PARTIAL / FAIL。

用法：
    python3 scripts/ticket_decision_evidence_audit.py --root <运行根> --date 2026-11-09
    python3 scripts/ticket_decision_evidence_audit.py --root <运行根>          # 全部日期
    python3 scripts/ticket_decision_evidence_audit.py --root ... --no-db      # 只审文件
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sqlite3
import sys
from pathlib import Path

SCHEMA = "ticket-evidence-v1"
REQUIRED_TOP = {
    "schema_version",
    "business_date",
    "code",
    "name",
    "ticket_id",
    "captured_at",
    "level",
    "selection",
    "pricing",
    "portfolio",
    "fundamentals",
    "missing",
}
SOURCES = {"p01_preopen", "auction_0925", "intraday_trigger", "holding_plan"}
MAX_AUCTION_PCT = 6.0
ENSURE_FILL_MARKUP = 0.005
PRICE_TOLERANCE = 0.011


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def expected_tier(auction_pct: float) -> str:
    if auction_pct > MAX_AUCTION_PCT:
        return "skip_high"
    if auction_pct >= 2.0:
        return "light"
    if auction_pct >= -2.0:
        return "ensure_fill"
    return "bargain"


def expected_quote(auction_price: float, tier: str) -> float | None:
    if auction_price <= 0:
        return None
    if tier == "skip_high":
        return None
    if tier == "ensure_fill":
        return auction_price * (1.0 + ENSURE_FILL_MARKUP)
    return auction_price


def audit_bundle(path: Path, date_dir: str) -> list[str]:
    errors: list[str] = []
    try:
        bundle = json.loads(path.read_text(encoding="utf-8"))
    except Exception as error:  # noqa: BLE001 - 审计脚本要收集所有错误
        return [f"{path.name}: JSON 解析失败: {error}"]
    if not isinstance(bundle, dict):
        return [f"{path.name}: 顶层不是对象"]

    missing_top = REQUIRED_TOP - bundle.keys()
    if missing_top:
        errors.append(f"{path.name}: 缺字段 {sorted(missing_top)}")
        return errors
    if bundle["schema_version"] != SCHEMA:
        errors.append(f"{path.name}: schema_version={bundle['schema_version']}")
    if bundle["business_date"] != date_dir:
        errors.append(
            f"{path.name}: business_date={bundle['business_date']} != 目录 {date_dir}"
        )
    match = re.match(r"^(\d{6})-(\d{2})\.json$", path.name)
    if not match:
        errors.append(f"{path.name}: 文件名不是 <code>-<NN>.json")
        return errors
    if match.group(1) != bundle["code"]:
        errors.append(f"{path.name}: code={bundle['code']} 与文件名不符")
    if not bundle["captured_at"]:
        errors.append(f"{path.name}: captured_at 为空")
    if bundle["level"] != "L1":
        errors.append(f"{path.name}: level={bundle['level']}（应为 L1）")

    selection = bundle["selection"]
    pricing = bundle["pricing"]
    portfolio = bundle["portfolio"]
    fundamentals = bundle["fundamentals"]
    source = selection.get("source")
    if source not in SOURCES:
        errors.append(f"{path.name}: selection.source={source!r}")
        return errors

    # 未知不得带来源（伪造 provenance）。
    for value_key, source_key in (
        ("streak", "streak_source_date"),
        ("phase", "phase_source_date"),
    ):
        if pricing.get(value_key) is None and pricing.get(source_key) is not None:
            errors.append(f"{path.name}: {value_key}=null 但 {source_key} 有值")

    if source == "holding_plan":
        holding = bundle.get("holding") or {}
        if not holding.get("action") or not holding.get("reason"):
            errors.append(f"{path.name}: holding_plan 缺 action/reason")
        if pricing.get("quoted_price") is None:
            errors.append(f"{path.name}: holding_plan 缺 quoted_price")
    elif source == "auction_0925":
        auction_price = pricing.get("auction_price")
        auction_pct = pricing.get("auction_pct")
        tier = pricing.get("tier")
        if auction_price is None or auction_pct is None or tier is None:
            errors.append(f"{path.name}: 竞价包缺 auction_price/pct/tier")
        else:
            want_tier = expected_tier(auction_pct)
            if tier != want_tier:
                errors.append(
                    f"{path.name}: tier={tier}，按 {auction_pct}% 应为 {want_tier}"
                )
            want_quote = expected_quote(auction_price, tier)
            quoted = pricing.get("quoted_price")
            if want_quote is None:
                if quoted is not None:
                    errors.append(f"{path.name}: tier={tier} 不应给价，quoted={quoted}")
            elif quoted is None:
                errors.append(f"{path.name}: tier={tier} 应给价 {want_quote:.4f}")
            elif abs(quoted - want_quote) > PRICE_TOLERANCE:
                errors.append(
                    f"{path.name}: quoted={quoted} 与重算 {want_quote:.4f} 不符"
                )
    elif source == "intraday_trigger":
        low = pricing.get("entry_low")
        high = pricing.get("entry_high")
        if pricing.get("quoted_price") is None:
            errors.append(f"{path.name}: 盘中触发包缺 quoted_price")
        if low is not None and high is not None and low > high:
            errors.append(f"{path.name}: entry_low={low} > entry_high={high}")
    else:  # p01_preopen
        if pricing.get("auction_price") is not None:
            errors.append(f"{path.name}: P01 包不应有 auction_price")
        if not selection.get("concept"):
            errors.append(f"{path.name}: P01 包缺 concept")

    if portfolio.get("source") not in {"user_confirmed_snapshot", "read_failed"}:
        errors.append(f"{path.name}: portfolio.source={portfolio.get('source')!r}")
    if portfolio.get("source") == "read_failed" and portfolio.get("position_quantity"):
        errors.append(f"{path.name}: 读取失败却带持仓数量")

    filings = bundle.get("filings")
    if filings is None and bundle["business_date"] >= "2026-11-11":
        errors.append(f"{path.name}: 缺 filings（6.5 已接线）")
    has_filings = filings is not None
    filings = filings or {}
    if has_filings:
        if filings.get("status") not in {"ok", "empty", "unavailable", "timeout", "not_fetched"}:
            errors.append(f"{path.name}: filings.status={filings.get('status')!r}")
        if filings.get("status") == "ok" and not filings.get("items"):
            errors.append(f"{path.name}: filings.status=ok 但 items 为空")
        if filings.get("status") != "ok" and "official_filing" not in (bundle.get("missing") or []):
            errors.append(f"{path.name}: 公告未取到但 missing 未列 official_filing")

    status = fundamentals.get("status")
    if status not in {"ok", "empty", "unavailable", "invalid_cache"}:
        errors.append(f"{path.name}: fundamentals.status={status!r}")
    if status == "ok":
        kinds = fundamentals.get("kinds") or []
        if not kinds:
            errors.append(f"{path.name}: fundamentals.status=ok 但 kinds 为空")
        period = fundamentals.get("report_period") or ""
        if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", period):
            errors.append(f"{path.name}: report_period={period!r}")
        announced = fundamentals.get("announced_on") or ""
        if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", announced):
            errors.append(f"{path.name}: announced_on={announced!r}")
        elif announced > bundle["business_date"]:
            errors.append(
                f"{path.name}: announced_on={announced} 晚于业务日 {bundle['business_date']}"
            )
        if not fundamentals.get("batch_id"):
            errors.append(f"{path.name}: fundamentals.status=ok 但 batch_id 为空")
        if "financial_statements" in (bundle.get("missing") or []):
            errors.append(f"{path.name}: 财务已取到但 missing 仍列 financial_statements")
    elif status == "unavailable":
        if "financial_statements" not in (bundle.get("missing") or []):
            errors.append(f"{path.name}: 财务不可用但 missing 未列 financial_statements")
    return errors


def audit_events(root: Path, db: Path, bundles: list[tuple[Path, dict]]) -> tuple[list[str], int]:
    errors: list[str] = []
    checked = 0
    if not db.exists():
        return [f"主库不存在: {db}"], 0
    conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    try:
        for path, bundle in bundles:
            identity = f"evidence:{bundle['business_date']}:{bundle['code']}:{path.stem.split('-')[-1]}"
            rows = conn.execute(
                "SELECT summary FROM ticket_event WHERE event_kind='evidence_bundle' "
                "AND occurrence_identity=?",
                (identity,),
            ).fetchall()
            if not rows:
                errors.append(f"{path.name}: 缺 ticket_event({identity})")
                continue
            if len(rows) > 1:
                errors.append(f"{path.name}: ticket_event 重复 {len(rows)} 条")
            summary = rows[0][0] or ""
            parts = summary.split("|")
            actual = sha256(path)
            if len(parts) < 2 or parts[1] != actual:
                errors.append(f"{path.name}: sha256 与事件摘要不符")
                continue
            if parts[0] != str(path):
                event_path = Path(parts[0])
                if not event_path.is_absolute():
                    event_path = root / event_path
                if event_path.resolve() != path.resolve():
                    errors.append(f"{path.name}: 事件路径 {parts[0]} 与文件不符")
            checked += 1
    finally:
        conn.close()
    return errors, checked


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--date", default=None)
    parser.add_argument("--no-db", action="store_true")
    args = parser.parse_args()

    base = args.root / "data" / "ticket_evidence"
    if not base.exists():
        print(f"没有证据包目录: {base}")
        return 0
    dates = [args.date] if args.date else sorted(
        p.name for p in base.iterdir() if p.is_dir() and re.fullmatch(r"\d{4}-\d{2}-\d{2}", p.name)
    )
    errors: list[str] = []
    bundles: list[tuple[Path, dict]] = []
    missing_counter: dict[str, int] = {}
    risk_flagged = 0
    risk_conflicts = 0
    fillability_counts: dict[str, int] = {}
    fillability_conflicts = 0
    skipped = 0
    for date in dates:
        date_dir = base / date
        if not date_dir.is_dir():
            errors.append(f"缺目录 {date_dir}")
            continue
        for path in sorted(date_dir.glob("*.json")):
            if not re.fullmatch(r"\d{6}-\d{2}\.json", path.name):
                skipped += 1
                continue
            errors.extend(audit_bundle(path, date))
            try:
                bundle = json.loads(path.read_text(encoding="utf-8"))
            except Exception:  # noqa: BLE001 - 已在 audit_bundle 记错
                continue
            bundles.append((path, bundle))
            pricing = bundle.get("pricing") or {}
            if pricing.get("risk_note"):
                risk_flagged += 1
                if pricing.get("quoted_price") is not None:
                    risk_conflicts += 1
            state = pricing.get("fillability")
            if state:
                fillability_counts[state] = fillability_counts.get(state, 0) + 1
                if state == "unfillable_gap" and pricing.get("quoted_price") is not None:
                    fillability_conflicts += 1
                    errors.append(
                        f"{path.name}: fillability=unfillable_gap 却给了参考价"
                    )
            for field in bundle.get("missing") or []:
                missing_counter[field] = missing_counter.get(field, 0) + 1

    event_checked = 0
    if not args.no_db:
        event_errors, event_checked = audit_events(
            args.root, args.root / "data" / "stock_analysis.db", bundles
        )
        errors.extend(event_errors)

    print(f"日期: {', '.join(dates) if dates else '（无）'}")
    print(f"证据包: {len(bundles)} 个；哈希对账: {event_checked} 个；跳过非 6.3 文件: {skipped} 个")
    if risk_flagged:
        print(f"6.10 风险披露: 带 risk_note {risk_flagged} 个；其中仍给参考价的冲突 {risk_conflicts} 个")
    if fillability_counts:
        top = ", ".join(
            f"{key}={value}"
            for key, value in sorted(fillability_counts.items(), key=lambda x: -x[1])
        )
        print(f"6.9 可成交性: {top}；其中不可成交却给价 {fillability_conflicts} 个")
    if missing_counter:
        top = ", ".join(
            f"{key}={value}" for key, value in sorted(missing_counter.items(), key=lambda x: -x[1])
        )
        print(f"missing 分布: {top}")
    if errors:
        print(f"FAIL: {len(errors)} 项")
        for error in errors[:50]:
            print(f"  - {error}")
        if len(errors) > 50:
            print(f"  ... 还有 {len(errors) - 50} 项")
        return 1
    print("PASS" if bundles else "PASS（无证据包，未发现不一致）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
