#!/usr/bin/env python3
"""交易日内自检：只读数据库 + 最近日志，输出可机读状态。

不推送、不修改生产数据。用于 operator 让 agent 自己巡检：
- 全市场日线是否到最新交易日；
- ticket 提案/触发/失效/到期状态；
- ticket_decision 是否开始积累；
- 观察池 coverage 最近成功/部分/失败；
- monitor 日志最近活动。

用法：
  python3 scripts/ticket_ops_check.py --db <db> --delivery-db <db> \
    --log <stderr.log> --out <json>
"""
from __future__ import annotations

import argparse
import json
import re
import sqlite3
from collections import deque
from datetime import datetime
from pathlib import Path


def scalar(con: sqlite3.Connection, sql: str, params=()):
    row = con.execute(sql, params).fetchone()
    return row[0] if row else None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--db", required=True)
    parser.add_argument("--delivery-db", required=True)
    parser.add_argument("--log", required=True)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()

    db = sqlite3.connect(f"file:{args.db}?mode=ro", uri=True)
    delivery = sqlite3.connect(f"file:{args.delivery_db}?mode=ro", uri=True)
    report: dict = {"generated_at": datetime.now().astimezone().isoformat()}
    try:
        latest = db.execute(
            "SELECT date, COUNT(*) FROM stock_daily GROUP BY date ORDER BY date DESC LIMIT 2"
        ).fetchall()
        report["stock_daily"] = [{"date": row[0], "rows": row[1]} for row in latest]
        report["ticket_decision"] = {
            "rows": scalar(db, "SELECT COUNT(*) FROM ticket_decision"),
            "latest_as_of": scalar(db, "SELECT MAX(as_of) FROM ticket_decision"),
        }
        report["ticket_event_5d"] = {
            row[0]: row[1]
            for row in db.execute(
                "SELECT event_kind, COUNT(*) FROM ticket_event "
                "WHERE substr(event_at,1,10) >= date('now','-5 day') "
                "GROUP BY event_kind ORDER BY event_kind"
            ).fetchall()
        }
        report["stages"] = {
            row[0]: row[1]
            for row in db.execute(
                "SELECT stage, COUNT(*) FROM ticket_lifecycle "
                "WHERE close_date IS NULL GROUP BY stage ORDER BY stage"
            ).fetchall()
        }
        report["delivery"] = {
            "delivered_10d": scalar(
                delivery,
                "SELECT COUNT(*) FROM delivery_decisions "
                "WHERE state='Delivered' AND business_date >= date('now','-10 day')",
            ),
            "holding_plan_10d": scalar(
                delivery,
                "SELECT COUNT(*) FROM delivery_decisions "
                "WHERE push_kind='HoldingPlan' AND state='Delivered' "
                "AND business_date >= date('now','-10 day')",
            ),
        }
    finally:
        db.close()
        delivery.close()

    log_path = Path(args.log)
    coverage = {"total": 0, "available": 0, "partial": 0, "unavailable": 0, "last": []}
    if log_path.is_file():
        # 只读尾部，避免历史日志把内存吃满。
        with log_path.open("rb") as handle:
            handle.seek(0, 2)
            size = handle.tell()
            handle.seek(max(0, size - 8_000_000))
            text = handle.read().decode("utf-8", errors="ignore")
        # 只看当前进程启动后的日志；否则重启前的失败会被当成“现在还在失败”。
        anchor = text.rfind("[monitor] bound root")
        current = text[anchor:] if anchor >= 0 else text
        last = deque(maxlen=5)
        for line in current.splitlines():
            match = re.search(
                r"\[观察池\]\[coverage\].*scope=(\S+).*health=(\w+)", line
            )
            if not match:
                continue
            coverage["total"] += 1
            health = match.group(2).lower()
            if health in ("available", "partial", "unavailable"):
                coverage[health] += 1
            last.append({"scope": match.group(1), "health": health})
        coverage["last"] = list(last)
        report["last_log_line"] = text.splitlines()[-1][:300] if text.splitlines() else ""
    report["coverage"] = coverage
    report["log_path"] = str(log_path)

    critical = []
    if not report["stock_daily"] or report["stock_daily"][0]["rows"] < 1000:
        critical.append("stock_daily_latest_not_full_universe")
    if report["ticket_decision"]["rows"] == 0:
        critical.append("ticket_decision_empty_shadow_has_no_sample")
    if coverage["unavailable"] > 0 and coverage["available"] == 0:
        critical.append("peer_quote_source_unavailable")
    report["critical"] = critical
    report["status"] = "FAIL" if "peer_quote_source_unavailable" in critical else "PARTIAL"
    if not critical:
        report["status"] = "OK"

    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n")
    print(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
