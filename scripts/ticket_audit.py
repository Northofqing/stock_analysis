#!/usr/bin/env python3
"""单票证据包（WP6 最小可执行版，2026-10-27）。

只读生产库和 push_log，生成一个可复核的目录：

    <out>/raw/*.json
    <out>/timeline.jsonl
    <out>/gaps.json
    <out>/manifest.sha256
    <out>/memo.md

边界：
- 不对主库/投递库执行写操作（SQLite mode=ro）；
- 不复制整库，只复制与 ticket 相关的 push_log 文件；
- 缺失的原始竞价/成交证据写进 gaps，不补价格、不猜成交概率。

用法：
    python3 scripts/ticket_audit.py --db <db> --delivery-db <db> \
      --code 001216 --as-of 2026-10-26 --out <dir>
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import sqlite3
import sys
from pathlib import Path
from typing import Any, Iterable

ROOT = Path(__file__).resolve().parent.parent


def connect_ro(path: Path) -> sqlite3.Connection:
    con = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    con.row_factory = sqlite3.Row
    return con


def query(con: sqlite3.Connection, sql: str, params: Iterable[Any] = ()) -> list[dict[str, Any]]:
    cur = con.execute(sql, tuple(params))
    return [dict(row) for row in cur.fetchall()]


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n")


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def jsonable_delivery(con: sqlite3.Connection, business_date: str, code: str) -> list[dict[str, Any]]:
    rows = query(
        con,
        "SELECT decision_identity, business_date, push_kind, sub_kind, state, "
        "envelope_canonical, envelope_sha256, created_at, updated_at "
        "FROM delivery_decisions WHERE business_date = ? ORDER BY created_at, decision_identity",
        (business_date,),
    )
    out: list[dict[str, Any]] = []
    for row in rows:
        blob = row.pop("envelope_canonical") or b""
        text = ""
        if isinstance(blob, bytes):
            try:
                text = blob.decode("utf-8")
            except UnicodeDecodeError:
                text = ""
        else:
            text = str(blob)
        row["envelope_bytes_b64"] = base64.b64encode(blob).decode("ascii") if isinstance(blob, bytes) else ""
        row["envelope_text_sha256"] = sha256_bytes(text.encode("utf-8")) if text else None
        row["contains_code"] = code in text
        out.append(row)
    return out


def collect_push_logs(root: Path, code: str, limit: int = 200) -> list[Path]:
    if not root.exists():
        return []
    found: list[Path] = []
    for path in sorted(root.rglob("*")):
        if not path.is_file() or path.suffix not in (".md", ".json", ".txt"):
            continue
        try:
            text = path.read_text(errors="ignore")
        except OSError:
            continue
        if code in text:
            found.append(path)
            if len(found) >= limit:
                break
    return found


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--db", required=True)
    parser.add_argument("--delivery-db", required=True)
    parser.add_argument("--code", required=True)
    parser.add_argument("--as-of", required=True, dest="as_of")
    parser.add_argument("--out", required=True)
    parser.add_argument("--push-log-root", default=None)
    args = parser.parse_args()

    main_db = Path(args.db)
    delivery_db = Path(args.delivery_db)
    push_root = Path(args.push_log_root) if args.push_log_root else main_db.parent / "push_log"
    data_root = main_db.parent
    out = Path(args.out)
    if out.exists() and any(out.iterdir()):
        print(f"输出目录非空，拒绝覆盖: {out}", file=sys.stderr)
        return 2
    out.mkdir(parents=True, exist_ok=True)
    raw = out / "raw"
    raw.mkdir(parents=True, exist_ok=True)

    main = connect_ro(main_db)
    delivery = connect_ro(delivery_db) if delivery_db.exists() else None
    try:
        tickets = query(
            main,
            "SELECT id, code, open_date, close_date, stage, source, note, created_at, updated_at "
            "FROM ticket_lifecycle WHERE code = ? ORDER BY open_date, id",
            (args.code,),
        )
        ticket_ids = [row["id"] for row in tickets]
        events: list[dict[str, Any]] = []
        decisions: list[dict[str, Any]] = []
        if ticket_ids:
            placeholders = ",".join("?" for _ in ticket_ids)
            events = query(
                main,
                f"SELECT id, ticket_id, event_at, event_kind, push_kind, "
                f"occurrence_identity, summary, created_at FROM ticket_event "
                f"WHERE ticket_id IN ({placeholders}) ORDER BY event_at, id",
                ticket_ids,
            )
            decisions = query(
                main,
                f"SELECT decision_id, ticket_id, code, as_of, source, regime, entry_mode, "
                f"proposal_status, price_low, price_high, trigger_level, abandon, "
                f"valid_from, valid_until, evidence_grade, source_health, context_json, "
                f"rule_version, supersedes, created_at, updated_at FROM ticket_decision "
                f"WHERE ticket_id IN ({placeholders}) ORDER BY as_of, decision_id",
                ticket_ids,
            )

        daily = query(
            main,
            "SELECT code, date, open, high, low, close, volume, amount, pct_chg, "
            "ma5, ma10, ma20, data_source FROM stock_daily "
            "WHERE code = ? AND date BETWEEN date(?, '-10 day') AND date(?, '+10 day') "
            "ORDER BY date",
            (args.code, args.as_of, args.as_of),
        )
        regime = query(
            main,
            "SELECT trade_date, limit_up_count, max_streak, broken_count, limit_down_count, "
            "prev_pool_count, prev_premium, phase, captured_at FROM market_regime_daily "
            "WHERE trade_date BETWEEN date(?, '-10 day') AND date(?, '+10 day') "
            "ORDER BY trade_date",
            (args.as_of, args.as_of),
        )
        benchmark = query(
            main,
            "SELECT trade_date, code, stock_pct, market_pct, board_pct, excess_pct, "
            "sample_size, benchmark FROM attribution_benchmark_daily "
            "WHERE code = ? AND trade_date BETWEEN date(?, '-10 day') AND date(?, '+10 day') "
            "ORDER BY trade_date",
            (args.code, args.as_of, args.as_of),
        )
        advice = query(
            main,
            "SELECT occurrence_identity, business_date, push_kind, code, pushed_at, "
            "executed, execution_source, outcome_pct, expected_price, change_t1, change_t3, "
            "change_t5, mfe, mae, note FROM advice_outcome_tracking "
            "WHERE code = ? ORDER BY business_date, id",
            (args.code,),
        )
        deliveries = jsonable_delivery(delivery, args.as_of, args.code) if delivery else []

        write_json(raw / "ticket_lifecycle.json", tickets)
        (raw / "ticket_event.jsonl").write_text(
            "".join(json.dumps(row, ensure_ascii=False, sort_keys=True) + "\n" for row in events)
        )
        (raw / "ticket_decision.jsonl").write_text(
            "".join(json.dumps(row, ensure_ascii=False, sort_keys=True) + "\n" for row in decisions)
        )
        (raw / "stock_daily.jsonl").write_text(
            "".join(json.dumps(row, ensure_ascii=False, sort_keys=True) + "\n" for row in daily)
        )
        write_json(raw / "market_regime_daily.json", regime)
        write_json(raw / "attribution_benchmark_daily.json", benchmark)
        write_json(raw / "advice_outcome_tracking.json", advice)
        write_json(raw / "delivery_decisions.json", deliveries)

        push_files = collect_push_logs(push_root, args.code)
        push_dir = raw / "push_log"
        push_manifest: list[dict[str, Any]] = []
        for source in push_files:
            relative = source.relative_to(push_root)
            target = push_dir / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(source.read_bytes())
            push_manifest.append(
                {
                    "source": str(source),
                    "relative": str(Path("raw") / "push_log" / relative),
                    "sha256": sha256_file(source),
                }
            )
        write_json(raw / "push_log_index.json", push_manifest)

        gaps: list[dict[str, str]] = []
        if not decisions:
            gaps.append(
                {
                    "field": "ticket_decision",
                    "source": "ticket_decision",
                    "blocked_conclusion": "没有 DecisionContext/EntryPolicy 记录；不能声称已按新内核决策",
                }
            )
        if not any(row["stage"] == "entry_proposed" for row in tickets):
            gaps.append(
                {
                    "field": "ticket_stage",
                    "source": "ticket_lifecycle",
                    "blocked_conclusion": "该票在 as_of 不是 entry_proposed；不能按入场触发语义评价",
                }
            )
        if not any(event["event_kind"] == "entry_proposed" for event in events):
            gaps.append(
                {
                    "field": "entry_proposed_event",
                    "source": "ticket_event",
                    "blocked_conclusion": "没有 entry_proposed 事件；缺少提案原文和时间",
                }
            )
        if not any(row["date"] == args.as_of for row in daily):
            gaps.append(
                {
                    "field": "stock_daily_exact_date",
                    "source": "stock_daily",
                    "blocked_conclusion": f"{args.as_of} 无该票收盘价；不能用相邻日期替代",
                }
            )
        if not (data_root / "auction_raw").exists():
            gaps.append(
                {
                    "field": "auction_raw",
                    "source": "data/auction_raw",
                    "blocked_conclusion": "没有原始竞价/逐笔/成交证据；不能证明成交概率或排队",
                }
            )
        if not any(row["push_kind"] == "HoldingPlan" and row["state"] == "Delivered" for row in deliveries):
            gaps.append(
                {
                    "field": "HoldingPlan_delivery",
                    "source": "delivery_decisions",
                    "blocked_conclusion": "没有发现该日 HoldingPlan 的 Delivered 记录",
                }
            )
        write_json(out / "gaps.json", gaps)

        timeline: list[dict[str, Any]] = []
        for row in events:
            timeline.append({"at": row.get("event_at"), "kind": "ticket_event", "data": row})
        for row in daily:
            timeline.append({"at": row.get("date"), "kind": "stock_daily", "data": row})
        for row in regime:
            timeline.append({"at": row.get("trade_date"), "kind": "market_regime", "data": row})
        for row in deliveries:
            timeline.append({"at": row.get("created_at"), "kind": "delivery_decision", "data": row})
        timeline.sort(key=lambda item: str(item.get("at") or ""))
        (out / "timeline.jsonl").write_text(
            "".join(json.dumps(row, ensure_ascii=False, sort_keys=True) + "\n" for row in timeline)
        )

        memo = [
            f"# 单票证据包：{args.code} / {args.as_of}",
            "",
            "## 事实",
            f"- ticket 行数：{len(tickets)}",
            f"- ticket_event：{len(events)} 条",
            f"- ticket_decision：{len(decisions)} 条",
            f"- stock_daily：{len(daily)} 条（含 as-of 前后）",
            f"- delivery_decisions：{len(deliveries)} 条（该业务日）",
            f"- push_log 命中文件：{len(push_manifest)} 个",
            "",
            "## 证据缺口",
        ]
        if gaps:
            memo.extend(f"- {gap['field']}：{gap['blocked_conclusion']}" for gap in gaps)
        else:
            memo.append("- 无结构性缺口；仍不能据此证明成交概率")
        memo.extend(
            [
                "",
                "## 边界",
                "- 本包只读生产库；派生表不能替代原始竞价/成交证据。",
                "- 缺失字段不补 0、不补价格、不推断成交概率。",
            ]
        )
        (out / "memo.md").write_text("\n".join(memo) + "\n")

        sources = {
            "code": args.code,
            "as_of": args.as_of,
            "main_db": str(main_db),
            "delivery_db": str(delivery_db),
            "push_log_root": str(push_root),
            "files": sorted(str(path.relative_to(out)) for path in out.rglob("*") if path.is_file()),
        }
        write_json(out / "sources.json", sources)

        manifest_lines = []
        for path in sorted(out.rglob("*")):
            if not path.is_file() or path.name == "manifest.sha256":
                continue
            manifest_lines.append(f"{sha256_file(path)}  {path.relative_to(out)}")
        (out / "manifest.sha256").write_text("\n".join(manifest_lines) + "\n")
        print(f"evidence pack written: {out}")
        print(f"tickets={len(tickets)} events={len(events)} decisions={len(decisions)} gaps={len(gaps)}")
        return 0
    finally:
        main.close()
        if delivery:
            delivery.close()


if __name__ == "__main__":
    raise SystemExit(main())
