#!/usr/bin/env python3
"""ticket 本轮改动验收自检（只读）。

对应 2026-11-07 部署的 6.1/6.3/6.4 + 批 8 补跑，验收日默认由 `--date` 指定。

    python3 scripts/ticket_acceptance_check.py --date 2026-11-09 --phase open
    python3 scripts/ticket_acceptance_check.py --date 2026-11-09 --phase close

退出码：0=没有 FAIL（含 PENDING）；1=有 FAIL。
PENDING 不当作错误，避免定时任务把「还没到时间」报成告警。
"""

from __future__ import annotations

import argparse
import json
import sqlite3
import subprocess
import sys
from datetime import datetime
from pathlib import Path
from zoneinfo import ZoneInfo

RUNTIME = Path.home() / ".local/share/stock-analysis-runtime"
DB = RUNTIME / "data" / "stock_analysis.db"


def db_ro() -> sqlite3.Connection:
    return sqlite3.connect(f"file:{DB}?mode=ro", uri=True)


def prev_trading_date(conn: sqlite3.Connection, date: str) -> str | None:
    row = conn.execute(
        "SELECT MAX(date) FROM stock_daily WHERE date < ?", (date,)
    ).fetchone()
    return row[0] if row and row[0] else None


def check(name: str, ok: bool | None, detail: str) -> str:
    status = "PASS" if ok is True else ("FAIL" if ok is False else "PENDING")
    print(f"{status:7} {name}: {detail}")
    return status


def bundle_counts(date: str) -> dict[str, int]:
    base = RUNTIME / "data" / "ticket_evidence" / date
    counts = {"01_p01": 0, "02_auction": 0, "03_trigger": 0, "other": 0}
    if not base.is_dir():
        return counts
    for path in base.glob("*.json"):
        stem = path.stem.rsplit("-", 1)[-1]
        if stem == "01":
            counts["01_p01"] += 1
        elif stem == "02":
            counts["02_auction"] += 1
        elif stem == "03":
            counts["03_trigger"] += 1
        else:
            counts["other"] += 1
    return counts


def main() -> int:
    global RUNTIME, DB
    parser = argparse.ArgumentParser()
    parser.add_argument("--date", default=None)
    parser.add_argument("--phase", choices=["open", "close", "all"], default="all")
    parser.add_argument("--root", type=Path, default=RUNTIME)
    args = parser.parse_args()
    if args.date is None:
        args.date = datetime.now(ZoneInfo("Asia/Shanghai")).date().isoformat()
    RUNTIME = args.root
    DB = RUNTIME / "data" / "stock_analysis.db"
    if not DB.exists():
        print(f"FAIL    主库不存在: {DB}")
        return 1

    statuses: list[str] = []
    today = datetime.now(ZoneInfo("Asia/Shanghai")).date().isoformat()
    now = datetime.now(ZoneInfo("Asia/Shanghai"))
    future = args.date > today

    def pending_before(hhmm: str) -> bool:
        return future or (args.date == today and now.strftime("%H%M") < hhmm)

    conn = db_ro()
    try:
        # 1) 进程与启动（每次运行都查）
        pid_file = RUNTIME / "logs" / "monitor-launchd.stderr.log"
        running = subprocess.run(
            ["pgrep", "-f", str(RUNTIME / "target/release/monitor")],
            capture_output=True,
            text=True,
        ).stdout.strip()
        statuses.append(check("monitor 进程", bool(running), f"pid={running or '无'}"))

        # 2) 证据包（按 occurrence 分类）
        counts = bundle_counts(args.date)
        total = sum(counts.values())
        evidence_ok: bool | None = (total > 0) if not future else None
        statuses.append(
            check(
                "6.3 证据包",
                evidence_ok,
                f"P01={counts['01_p01']} 竞价={counts['02_auction']} 触发={counts['03_trigger']} 其他={counts['other']}",
            )
        )

        # 3) 09:25 竞价推送 marker
        marker = RUNTIME / "data" / "auction_price" / args.date
        detail = marker.read_text(encoding="utf-8").strip() if marker.exists() else "无 marker"
        statuses.append(
            check(
                "09:25 竞价推送",
                marker.exists() if not pending_before("0926") else None,
                detail[:160],
            )
        )

        # 4) 15:40 补跑（上一交易日 marker）
        prev = prev_trading_date(conn, args.date)
        if prev is None:
            statuses.append(check("淘汰补跑", None, "找不到上一交易日"))
        else:
            evict = RUNTIME / "data" / "ticket_eviction" / prev
            detail = evict.read_text(encoding="utf-8").strip() if evict.exists() else f"无 {prev} marker"
            statuses.append(
                check(
                    "15:40 淘汰补跑",
                    evict.exists() if not pending_before("1545") else None,
                    detail,
                )
            )

        # 5) 当日淘汰事件数
        evicted = conn.execute(
            "SELECT COUNT(*) FROM ticket_event WHERE event_kind='evicted' AND date(event_at)=date(?)",
            (args.date,),
        ).fetchone()[0]
        statuses.append(
            check(
                "当日淘汰事件",
                evicted > 0 if not pending_before("1545") else None,
                f"evicted={evicted}",
            )
        )

        # 6) 日线覆盖
        daily = conn.execute(
            "SELECT COUNT(*) FROM stock_daily WHERE date=?", (args.date,)
        ).fetchone()[0]
        statuses.append(
            check(
                "当日日线覆盖",
                daily >= 800 if not pending_before("2140") else None,
                f"{daily} 行（阈值 800）",
            )
        )

        # 7) 审计脚本（有包才跑）
        audit = subprocess.run(
            [sys.executable, str(Path(__file__).resolve().parent / "ticket_decision_evidence_audit.py"),
             "--root", str(RUNTIME), "--date", args.date],
            capture_output=True,
            text=True,
        )
        audit_ok = audit.returncode == 0
        tail = (audit.stdout or audit.stderr).strip().splitlines()
        statuses.append(
            check(
                "6.9 证据审计",
                audit_ok if (not future and total > 0) else None,
                tail[-1] if tail else "无输出",
            )
        )
    finally:
        conn.close()

    if "FAIL" in statuses:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
