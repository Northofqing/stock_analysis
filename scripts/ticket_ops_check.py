#!/usr/bin/env python3
"""交易日内自检：只读数据库 + 最近日志 + 当日产物，输出可机读状态。

不修改生产数据；唯一副作用是 `--notify fail` 时通过 magiclaw CLI 推送飞书。

检查分两类：
- 结构项（随时）：日线覆盖、ticket 提案/触发/失效/到期、ticket_decision、行情 coverage；
- 「当日应到」项（按时间窗判定是否到期）：paper 收盘、观察池 marker、关键推送、
  明日计划归档、全市场日线。未到期不算缺失，到期缺失才是 FAIL。

用法：
  python3 scripts/ticket_ops_check.py --db <db> --delivery-db <db> \
    --log <stderr.log> --out <json> [--data-dir <runtime/data>] \
    [--expect-date YYYY-MM-DD] [--notify off|fail|dry-run]
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sqlite3
import subprocess
from collections import deque
from datetime import datetime, time as dtime, timedelta
from pathlib import Path


def scalar(con: sqlite3.Connection, sql: str, params=()):
    row = con.execute(sql, params).fetchone()
    return row[0] if row else None


def load_dotenv(path: Path) -> dict[str, str]:
    """读取 KEY=VALUE 形式的 .env；缺失或格式不合法时返回空字典。"""
    values: dict[str, str] = {}
    if not path.is_file():
        return values
    for raw in path.read_text(encoding="utf-8", errors="ignore").splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        if key.startswith("export "):
            key = key[len("export ") :].strip()
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
            value = value[1:-1]
        if key:
            values[key] = value
    return values


def last_weekday(day):
    while day.weekday() >= 5:
        day -= timedelta(days=1)
    return day


def expected_check_date(now: datetime, override: str | None):
    if override:
        parsed = datetime.strptime(override, "%Y-%m-%d").date()
        return parsed, parsed == now.date()
    if now.weekday() < 5:
        return now.date(), True
    # 周末手工巡检：以最近一个工作日为基准，按“日终”口径检查历史缺口。
    return last_weekday(now.date()), False


def slot_name(now: datetime) -> str:
    minute = now.hour * 60 + now.minute
    if minute < 10 * 60:
        return "preopen"
    if minute < 15 * 60:
        return "intraday"
    if minute < 16 * 60 + 5:
        return "preclose"
    if minute < 21 * 60 + 30:
        return "close"
    return "final"


def due_at(slot: str, name: str, minute_of_day: int) -> bool:
    """按当地时刻判断“当日应到”项是否已经到期；历史巡检一律视为已到期。

    阈值来自实际落库时间，不按方案里的整点假设：
    - paper close 依赖当日合格日线，实测 10-27/28/29 都在 21:23~21:35 才写入；
    - 观察池 marker 15:40 后才写，15:45 的首轮留 7 分钟余量；
    - 明日计划 16:00 后生成。
    """
    if slot == "historical":
        return True
    thresholds = {
        "PreopenNewsHot": 9 * 60 + 10,
        "auction_raw": 9 * 60 + 35,
        "HoldingPlan": 10 * 60,
        "watch_pool": 15 * 60 + 52,
        "TomorrowWatch": 16 * 60 + 10,
        "tomorrow_plan_archive": 16 * 60 + 10,
        "paper_close": 21 * 60 + 45,
        "stock_daily": 21 * 60 + 30,
    }
    return minute_of_day >= thresholds[name]


def paper_close_state(con: sqlite3.Connection, limit: int = 40):
    """从 paper_ledger_event.Marked.marks[].source 提取 qualified_daily_close_v1 的 price_date。"""
    dates: dict[str, int] = {}
    rows = con.execute(
        "SELECT payload FROM paper_ledger_event "
        "WHERE instr(payload,'qualified_daily_close_v1')>0 "
        "ORDER BY seq DESC LIMIT ?",
        (limit,),
    ).fetchall()
    for (payload,) in rows:
        try:
            marked = (json.loads(payload) or {}).get("Marked") or {}
        except (TypeError, ValueError):
            continue
        for mark in marked.get("marks") or []:
            source = mark.get("source")
            if isinstance(source, str):
                try:
                    source = json.loads(source)
                except (TypeError, ValueError):
                    continue
            if not isinstance(source, dict):
                continue
            if source.get("kind") != "qualified_daily_close_v1":
                continue
            price_date = source.get("price_date")
            if price_date:
                dates[price_date] = dates.get(price_date, 0) + 1
    latest = max(dates) if dates else None
    return latest, dates.get(latest, 0) if latest else 0, dates


def last_log_reason(log_path: Path, pattern: str, tail_bytes: int = 96_000_000) -> str | None:
    """在当前日志尾部找最后一次匹配，返回匹配组；找不到返回 None。"""
    if not log_path.is_file():
        return None
    with log_path.open("rb") as handle:
        handle.seek(0, 2)
        size = handle.tell()
        handle.seek(max(0, size - tail_bytes))
        text = handle.read().decode("utf-8", errors="ignore")
    found = None
    for match in re.finditer(pattern, text, re.MULTILINE):
        found = match.group(1)
    return found[:400] if found else None


def push_deliveries(con: sqlite3.Connection, check_date: str) -> dict[str, dict]:
    rows = con.execute(
        "SELECT push_kind, COUNT(*), MAX(updated_at) FROM delivery_decisions "
        "WHERE business_date=? AND state='Delivered' GROUP BY push_kind",
        (check_date,),
    ).fetchall()
    return {
        row[0]: {"count": row[1], "last_at": row[2]} for row in rows
    }


def build_alert(report: dict, missing: list[dict]) -> str:
    slot_label = {
        "preopen": "盘前",
        "intraday": "盘中",
        "preclose": "盘后",
        "close": "收盘后",
        "final": "终检",
        "historical": "历史补齐",
    }.get(report["expectations"]["slot"], report["expectations"]["slot"])
    lines = [
        f"[stock_analysis 巡检 {report['status']}] {report['generated_at'][:16].replace('T', ' ')}",
        f"基准日 {report['expectations']['check_date']}（{slot_label}）",
    ]
    for item in missing:
        lines.append(f"· {item['label']}：{item['detail']}")
    lines.append(f"报告：{report['report_path']}")
    return "\n".join(lines)


def send_feishu(message: str, env: dict[str, str]) -> tuple[bool, str]:
    target = env.get("FEISHU_TO", "").strip()
    if not target:
        return False, "missing_feishu_target"
    binary = env.get("MAGICLAW_BIN", "").strip() or str(
        Path.home() / "Desktop/magiclaw/target/release/magiclaw"
    )
    home = env.get("MAGICLAW_HOME", "").strip() or str(Path.home() / "Desktop/magiclaw")
    if not Path(binary).is_file():
        return False, f"magiclaw_bin_missing:{binary}"
    command = [
        binary,
        "send",
        "--channel",
        "feishu",
        "--message",
        message,
        "--to",
        target,
    ]
    try:
        completed = subprocess.run(
            command,
            cwd=home if Path(home).is_dir() else None,
            capture_output=True,
            text=True,
            timeout=90,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        return False, f"spawn_failed:{error}"
    output = (completed.stdout or completed.stderr or "").strip().replace("\n", " ")[:300]
    return completed.returncode == 0, f"exit={completed.returncode} {output}".strip()


def notify_fail(report: dict, data_dir: Path, env: dict[str, str], mode: str) -> dict:
    missing = [item for item in report["expectations"]["items"] if item["status"] == "missing"]
    missing_keys = {item["key"] for item in missing}
    # expectations 之外的结构性 critical 也要通知，否则「行情全断」这类失败不会出声。
    for entry in report.get("critical", []):
        key = entry.split(":", 1)[0]
        if key in missing_keys:
            continue
        missing.append({"key": key, "label": "巡检", "detail": entry})
    if not missing or mode == "off":
        return {"mode": mode, "status": "skipped", "detail": "no_missing_or_off"}
    signature = hashlib.sha256(
        "|".join(sorted(item["key"] for item in missing)).encode()
    ).hexdigest()[:16]
    alert_dir = data_dir / "ops_check" / "alerted"
    alert_dir.mkdir(parents=True, exist_ok=True)
    marker = alert_dir / f"{report['expectations']['check_date']}.json"
    if marker.is_file():
        try:
            previous = json.loads(marker.read_text(encoding="utf-8"))
        except (TypeError, ValueError):
            previous = {}
        if previous.get("signature") == signature:
            return {"mode": mode, "status": "deduped", "signature": signature}
    message = build_alert(report, missing)
    if mode == "dry-run":
        return {"mode": mode, "status": "dry_run", "signature": signature, "message": message}
    ok, detail = send_feishu(message, env)
    log_path = data_dir / "ops_check" / "notify.log"
    with log_path.open("a", encoding="utf-8") as handle:
        handle.write(
            json.dumps(
                {
                    "at": report["generated_at"],
                    "ok": ok,
                    "signature": signature,
                    "detail": detail,
                },
                ensure_ascii=False,
            )
            + "\n"
        )
    if ok:
        marker.write_text(
            json.dumps(
                {
                    "signature": signature,
                    "sent_at": report["generated_at"],
                    "keys": sorted(item["key"] for item in missing),
                },
                ensure_ascii=False,
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
    return {
        "mode": mode,
        "status": "sent" if ok else "failed",
        "signature": signature,
        "detail": detail,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--db", required=True)
    parser.add_argument("--delivery-db", required=True)
    parser.add_argument("--log", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--data-dir", default=None)
    parser.add_argument("--expect-date", default=None)
    parser.add_argument("--notify", choices=["off", "fail", "dry-run"], default="off")
    args = parser.parse_args()

    db_path = Path(args.db)
    delivery_path = Path(args.delivery_db)
    data_dir = Path(args.data_dir) if args.data_dir else db_path.parent
    now = datetime.now().astimezone()
    check_date, is_today = expected_check_date(now, args.expect_date)
    check_text = check_date.isoformat()
    slot = slot_name(now) if is_today else "historical"

    db = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    delivery = sqlite3.connect(f"file:{delivery_path}?mode=ro", uri=True)
    report: dict = {
        "generated_at": now.isoformat(),
        "expectations": {
            "check_date": check_text,
            "is_today": is_today,
            "slot": slot,
            "items": [],
        },
    }
    pushes: dict[str, dict] = {}
    paper_latest = None
    paper_marks = 0
    try:
        latest = db.execute(
            "SELECT date, COUNT(*) FROM stock_daily GROUP BY date ORDER BY date DESC LIMIT 2"
        ).fetchall()
        report["stock_daily"] = [{"date": row[0], "rows": row[1]} for row in latest]
        today_rows = db.execute(
            "SELECT COUNT(*) FROM stock_daily WHERE date=? AND close > 0", (check_text,)
        ).fetchone()[0]
        report["stock_daily_today"] = {"date": check_text, "rows": today_rows}
        report["ticket_decision"] = {
            "rows": scalar(db, "SELECT COUNT(*) FROM ticket_decision"),
            "latest_as_of": scalar(db, "SELECT MAX(as_of) FROM ticket_decision"),
            "dates": [
                {"as_of": row[0], "rows": row[1]}
                for row in db.execute(
                    "SELECT as_of, COUNT(*) FROM ticket_decision "
                    "GROUP BY as_of ORDER BY as_of DESC LIMIT 10"
                ).fetchall()
            ],
            "statuses": {
                row[0]: row[1]
                for row in db.execute(
                    "SELECT proposal_status, COUNT(*) FROM ticket_decision "
                    "GROUP BY proposal_status ORDER BY proposal_status"
                ).fetchall()
            },
            "modes": {
                row[0]: row[1]
                for row in db.execute(
                    "SELECT entry_mode, COUNT(*) FROM ticket_decision "
                    "GROUP BY entry_mode ORDER BY entry_mode"
                ).fetchall()
            },
        }
        # JSON 解析放在 Python 侧：旧 context 没有 selection 字段，不能把 null
        # 当成 0；统计的是“有真实字段的决策数”，不是行数。
        selection_known = 0
        selection_states: dict[str, int] = {}
        selection_unknown = 0
        latest_selection_date = None
        latest_selection_known = 0
        for row in db.execute(
            "SELECT as_of, context_json FROM ticket_decision "
            "ORDER BY as_of DESC, created_at DESC LIMIT 500"
        ).fetchall():
            try:
                context = json.loads(row[1] or "{}")
            except (TypeError, ValueError):
                selection_unknown += 1
                continue
            selection = context.get("selection") or {}
            extension = context.get("extension") or {}
            has_selection = selection.get("pre20_return") is not None
            state = extension.get("state") or "unknown"
            selection_states[state] = selection_states.get(state, 0) + 1
            if latest_selection_date is None:
                latest_selection_date = row[0]
            if row[0] == latest_selection_date:
                latest_selection_known += int(has_selection)
            if has_selection:
                selection_known += 1
            else:
                selection_unknown += 1
        report["ticket_decision"]["selection"] = {
            "known": selection_known,
            "unknown": selection_unknown,
            "states": selection_states,
            "latest_as_of": latest_selection_date,
            "latest_known": latest_selection_known,
        }
        report["entry_events"] = {
            row[0]: {"count": row[1], "last_date": row[2]}
            for row in db.execute(
                "SELECT event_kind, COUNT(*), MAX(substr(event_at,1,10)) "
                "FROM ticket_event WHERE event_kind IN "
                "('entry_proposed','entry_triggered','entry_invalidated','entry_expired') "
                "GROUP BY event_kind ORDER BY event_kind"
            ).fetchall()
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
        open_positions = scalar(
            db,
            "SELECT COUNT(*) FROM stock_position WHERE quantity > 0",
        ) if db.execute(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='stock_position'"
        ).fetchone() else 0
        report["open_positions"] = open_positions or 0
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
        pushes = push_deliveries(delivery, check_text)
        paper_latest, paper_marks, paper_dates = paper_close_state(db)
        report["paper_close"] = {
            "latest_date": paper_latest,
            "latest_marks": paper_marks,
            "dates": dict(sorted(paper_dates.items())[-5:]),
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
        selection_lines = []
        for line in current.splitlines():
            if "[入场参考][selection]" in line:
                selection_lines.append(line[:500])
        report["selection_log"] = {
            "lines": len(selection_lines),
            "last": selection_lines[-1] if selection_lines else None,
        }
        report["last_log_line"] = text.splitlines()[-1][:300] if text.splitlines() else ""
    report["coverage"] = coverage
    report["log_path"] = str(log_path)
    report["key_pushes"] = {
        kind: pushes.get(kind, {"count": 0, "last_at": None})
        for kind in ("PreopenNewsHot", "HoldingPlan", "TomorrowWatch")
    }

    watch_pool_marker = data_dir / "watch_pool" / check_text
    plan_archive = data_dir / "buy_zone_archive" / f"{check_text}.txt"
    auction_raw_path = data_dir / "auction_raw" / f"{check_text}.json"
    report["auction_raw"] = {
        "path": str(auction_raw_path),
        "exists": auction_raw_path.is_file(),
    }
    paper_reason = None
    if paper_latest != check_text:
        paper_reason = last_log_reason(
            log_path,
            rf"\[PaperLedger-account\] paper close unavailable date={re.escape(check_text)}: (.+)$",
        )
    paper_detail = (
        f"paper 收盘已确认 {check_text}（{paper_marks} 条）"
        if paper_latest == check_text
        else f"最近收盘 {paper_latest or '无'}，缺 {check_text}"
        + (f"；日志：{paper_reason}" if paper_reason else "")
    )

    def item(key: str, label: str, ok: bool, detail: str) -> dict:
        due = due_at(slot, key, now.hour * 60 + now.minute)
        return {
            "key": key,
            "label": label,
            "due": due,
            "status": "ok" if ok else ("missing" if due else "not_due"),
            "detail": detail,
        }

    items = []
    items.append(
        item(
            "PreopenNewsHot",
            "09:00 盘前热点推送",
            report["key_pushes"]["PreopenNewsHot"]["count"] > 0,
            f"Delivered {report['key_pushes']['PreopenNewsHot']['count']} 条",
        )
    )
    items.append(
        item(
            "auction_raw",
            "竞价原始归档",
            auction_raw_path.is_file(),
            f"{auction_raw_path} {'存在' if auction_raw_path.is_file() else '缺失'}",
        )
    )
    holding_due = report["open_positions"] > 0
    items.append(
        item(
            "HoldingPlan",
            "持仓计划推送",
            (not holding_due) or report["key_pushes"]["HoldingPlan"]["count"] > 0,
            "无持仓，跳过"
            if not holding_due
            else f"Delivered {report['key_pushes']['HoldingPlan']['count']} 条",
        )
    )
    items.append(
        item(
            "paper_close",
            "paper 收盘",
            paper_latest == check_text,
            paper_detail,
        )
    )
    items.append(
        item(
            "watch_pool",
            "观察池 marker",
            watch_pool_marker.is_file(),
            f"{watch_pool_marker} {'存在' if watch_pool_marker.is_file() else '缺失'}",
        )
    )
    items.append(
        item(
            "TomorrowWatch",
            "明日计划推送",
            report["key_pushes"]["TomorrowWatch"]["count"] > 0 or plan_archive.is_file(),
            f"Delivered {report['key_pushes']['TomorrowWatch']['count']} 条"
            + ("；归档存在" if plan_archive.is_file() else ""),
        )
    )
    items.append(
        item(
            "tomorrow_plan_archive",
            "明日计划归档",
            plan_archive.is_file(),
            f"{plan_archive} {'存在' if plan_archive.is_file() else '缺失'}",
        )
    )
    items.append(
        item(
            "stock_daily",
            "全市场日线",
            report["stock_daily_today"]["rows"] >= 800,
            f"stock_daily[{check_text}] {report['stock_daily_today']['rows']} 行（要求 ≥800）",
        )
    )
    report["expectations"]["items"] = items

    critical = [f"{entry['key']}:{entry['detail']}" for entry in items if entry["status"] == "missing"]
    warnings = []
    today = now.strftime("%Y-%m-%d")
    if not report["stock_daily"] or report["stock_daily"][0]["rows"] < 1000:
        critical.append("stock_daily_latest_not_full_universe")
    if report["ticket_decision"]["rows"] == 0:
        critical.append("ticket_decision_empty_shadow_has_no_sample")
    if coverage["unavailable"] > 0 and coverage["available"] == 0:
        critical.append("peer_quote_source_unavailable")
    if plan_archive.is_file() and report["key_pushes"]["TomorrowWatch"]["count"] == 0:
        # 已知 B.5：明日计划可能绕过投递库直推；归档在、投递审计缺失 → 提示而非 FAIL。
        warnings.append("tomorrow_plan_delivery_not_audited")
    selection = report["ticket_decision"].get("selection") or {}
    decision_date = report["ticket_decision"].get("latest_as_of")
    # 09:35 时当天决策尚未生成；日线未落地时提案也无法运行。只有两者都满足才告警。
    minute = now.hour * 60 + now.minute
    daily_ready = report["stock_daily_today"]["rows"] >= 800
    if minute >= 15 * 60 + 37 and daily_ready and decision_date != check_text:
        warnings.append("ticket_decision_not_updated_today")
    elif decision_date != today and not daily_ready and minute >= 21 * 60 + 30:
        warnings.append("ticket_decision_pending_daily_bars")
    if (
        decision_date == today
        and selection.get("latest_known", 0) == 0
    ):
        critical.append("selection_context_missing_on_latest_decision")
    report["critical"] = critical
    report["warnings"] = warnings
    report["status"] = "FAIL" if critical else ("PARTIAL" if warnings else "OK")
    report["report_path"] = str(Path(args.out).resolve())

    try:
        env = load_dotenv(data_dir.parent / ".env")
    except OSError:
        env = {}
    report["notify"] = notify_fail(report, data_dir, env, args.notify)

    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n")
    print(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True))
    return 1 if report["status"] == "FAIL" else 0


if __name__ == "__main__":
    raise SystemExit(main())
