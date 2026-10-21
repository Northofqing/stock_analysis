#!/usr/bin/env python3
"""单票生命周期（ticket）阶段验收检查。

用途
----
方案 `docs/ticket-lifecycle-plan.md` 里的每个阶段都写了「验证」和「做不到」。
但「代码提交了」和「产出符合预期」是两件事——2026-10-21 对账发现
P3.3 提交两天后一次都没成功执行、P6 只覆盖 4 天、P1.1 新闻通道 0 产出，
而文档里这些都标着「已完成」。

本脚本不看代码，只看**产出**：每个阶段必须有可观测的数据库行或日志成功记录。
连续 N 个交易日没有产出，就判未达标，无论代码写没写。

用法
----
    python3 phase_acceptance.py                  # 默认看最近 5 个交易日
    python3 phase_acceptance.py --days 10
    python3 phase_acceptance.py --log-tail 50000 # 扫更多日志行

输出
----
每行一个阶段：状态 / 关键指标 / 卡点。退出码 1 表示有未达标项。
"""
import argparse
import json
import os
import re
import sqlite3
import sys
from collections import defaultdict
from datetime import datetime, timedelta

RUNTIME = "/Users/zhangzhen/.local/share/stock-analysis-runtime"
MAIN_DB = f"{RUNTIME}/data/stock_analysis.db"
DELIVERY_DB = f"{RUNTIME}/data/durable_delivery.sqlite3"
STDERR_LOG = f"{RUNTIME}/logs/monitor-launchd.stderr.log"

OK, FAIL, PARTIAL, SKIP = "达标", "未达标", "部分", "无法判断"


def ro(path):
    return sqlite3.connect(f"file:{path}?mode=ro", uri=True)


def trading_days(con, days):
    """从 stock_daily 取最近 N 个有数据的交易日（降序）。"""
    rows = con.execute(
        "SELECT DISTINCT date FROM stock_daily ORDER BY date DESC LIMIT ?", (days,)
    ).fetchall()
    return [r[0] for r in rows]


def table_exists(con, name):
    return con.execute(
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?", (name,)
    ).fetchone() is not None


class Result:
    def __init__(self, phase, name):
        self.phase = phase
        self.name = name
        self.status = SKIP
        self.detail = ""

    def __repr__(self):
        return f"{self.phase:6s} {self.name:16s} [{self.status}] {self.detail}"


def check_p11(main, days, log_lines):
    """P1.1 观察池：最近 N 个交易日必须有过 opened>0 或 news_candidates>0。"""
    r = Result("P1.1", "观察池构建")
    opened = news_cand = 0
    ok_days = 0
    for line in log_lines:
        m = re.search(
            r"\[观察池\] news_scanned=(\d+) news_candidates=(\d+).*?total=(\d+) opened=(\d+)",
            line,
        )
        if not m:
            continue
        news_cand += int(m.group(2))
        opened += int(m.group(4))
        if int(m.group(2)) > 0 or int(m.group(4)) > 0:
            ok_days += 1
    r.detail = f"日志累计 opened={opened} news_candidates={news_cand} 有产出的轮次={ok_days}"
    if ok_days > 0:
        r.status = OK
    elif opened == 0 and news_cand == 0:
        r.status = FAIL
        r.detail += " | 新闻通道 0 产出（依赖 news_ai_assessment）"
    return r


def check_p12(delivery, days):
    """P1.2 推送记账：最近 N 日 ticket_event 要有 push_delivered 增长。"""
    r = Result("P1.2", "推送记账")
    if not table_exists(delivery, "delivery_decisions"):
        r.detail = "delivery_decisions 表缺失"
        r.status = FAIL
        return r
    n = delivery.execute(
        """SELECT COUNT(*) FROM delivery_decisions
           WHERE push_kind IN ('T0Advice','HoldingPlan','CloseCall')
             AND state='Delivered'
             AND business_date >= date('now','-10 day')"""
    ).fetchone()[0]
    r.detail = f"最近 10 天 Delivered(T0Advice/HoldingPlan/CloseCall)={n}"
    r.status = OK if n > 0 else FAIL
    return r


def check_p14(main, days):
    """P1.4 P01 开票：最近 N 个交易日 source=p01_preopen 要有新增。"""
    r = Result("P1.4", "P01 自动开票")
    if not table_exists(main, "ticket_lifecycle"):
        r.detail = "ticket_lifecycle 缺失"
        r.status = FAIL
        return r
    rows = main.execute(
        """SELECT open_date, COUNT(*) FROM ticket_lifecycle
           WHERE source='p01_preopen' GROUP BY open_date ORDER BY open_date DESC LIMIT ?""",
        (days,),
    ).fetchall()
    total = sum(c for _, c in rows)
    r.detail = f"最近 {len(rows)} 个开票日共 {total} 张 " + (
        f"（最新 {rows[0][0]}）" if rows else "")
    if total == 0:
        r.status = FAIL
    elif len(rows) < min(days, 2):
        r.status = PARTIAL
        r.detail += " | 覆盖天数不足，可能是回填而非自动产出"
    else:
        r.status = OK
    return r


def check_p13(main, days):
    """P1.3 价格带落库：查 trading_fact_snapshot（不是 stock_daily）。

    2026-10-21 修正：初版查 stock_daily 的 price_band 列，判成「未做」，
    实际实现落在独立表 trading_fact_snapshot，15:50 调度，
    标的 = 当日 active tickets + 当前持仓。
    """
    r = Result("P1.3", "价格带落库")
    if not table_exists(main, "trading_fact_snapshot"):
        r.status = FAIL
        r.detail = "trading_fact_snapshot 表不存在"
        return r
    rows = main.execute(
        """SELECT trade_date, COUNT(*), COUNT(limit_up), COUNT(price_band_source)
           FROM trading_fact_snapshot
           GROUP BY trade_date ORDER BY trade_date DESC LIMIT ?""",
        (days,),
    ).fetchall()
    if not rows:
        r.status = FAIL
        r.detail = "表存在但无数据"
        return r
    latest = rows[0][0]
    total = sum(x[1] for x in rows)
    band = sum(x[2] for x in rows)
    src = sum(x[3] for x in rows)
    r.detail = (
        f"{len(rows)} 天 / {total} 行（最新 {latest}）"
        f" limit_up 非空 {band}/{total} price_band_source 非空 {src}/{total}"
    )
    try:
        lag = (datetime.now().date() - datetime.strptime(latest, "%Y-%m-%d").date()).days
    except Exception:
        lag = 999
    if lag > 7:
        r.status = PARTIAL
        r.detail += f" | 最新一天距今 {lag} 天，可能已停跑"
    elif band == 0:
        r.status = PARTIAL
        r.detail += " | 全部 NULL（非快照日，按 BR-222 不补值）"
    else:
        r.status = OK
    return r


def check_p2(main, days):
    """P2 可买价：ticket_event 要有 entry_proposed。"""
    r = Result("P2", "可买价/entry_proposed")
    if not table_exists(main, "ticket_event"):
        r.detail = "ticket_event 缺失"
        r.status = FAIL
        return r
    ev = main.execute(
        "SELECT COUNT(*) FROM ticket_event WHERE event_kind='entry_proposed'"
    ).fetchone()[0]
    st = main.execute(
        "SELECT COUNT(*) FROM ticket_lifecycle WHERE stage='entry_proposed'"
    ).fetchone()[0]
    obs = main.execute(
        "SELECT COUNT(*) FROM ticket_lifecycle WHERE stage='observing' AND close_date IS NULL"
    ).fetchone()[0]
    r.detail = f"entry_proposed 事件={ev} 状态={st} | 仍停在 observing={obs}"
    if ev == 0 and st == 0:
        r.status = FAIL
        r.detail += " | 从未产出过买入区间"
    else:
        r.status = OK
    return r


def check_p33(log_lines):
    """P3.3 同板块异动：看失败 vs 成功的比例。"""
    r = Result("P3.3", "同板块关联票")
    fail = sum(1 for l in log_lines if "关联票查询失败" in l)
    r.detail = f"最近日志里「关联票查询失败」{fail} 次"
    if fail == 0:
        # 没有任何失败也可能是因为规则没启用
        hit = sum(1 for l in log_lines if "同板块" in l and "关联票查询失败" not in l)
        r.detail += f" | 「同板块」上报 {hit} 次"
        r.status = OK if hit > 0 else SKIP
    else:
        r.status = FAIL
        r.detail += " | 规则空转，行情源失败"
    return r


def check_p4(delivery, days):
    """P4 次日动作：HoldingPlan 最近交易日有没有 Delivered。"""
    r = Result("P4", "HoldingPlan")
    rows = delivery.execute(
        """SELECT business_date, state, COUNT(*) FROM delivery_decisions
           WHERE push_kind='HoldingPlan' AND business_date >= date('now','-10 day')
           GROUP BY business_date, state ORDER BY business_date DESC"""
    ).fetchall()
    if not rows:
        r.status = FAIL
        r.detail = "最近 10 天无 HoldingPlan 记录"
        return r
    delivered = sum(c for _, st, c in rows if st == "Delivered")
    dates = sorted({bd for bd, st, _ in rows if st == "Delivered"})
    r.detail = f"最近 10 天 Delivered={delivered} 覆盖 {len(dates)} 天（最新 {dates[-1] if dates else '—'}）"
    r.status = OK if delivered > 0 else FAIL
    return r


def check_p5(main, days):
    """P5 轮动：board_rotation_signal 最近交易日行数 + board_ranking_daily 覆盖。"""
    r = Result("P5", "板块轮动")
    parts = []
    if not table_exists(main, "board_rotation_signal"):
        parts.append("board_rotation_signal 缺失")
        sig_days = 0
    else:
        cols = [c[1] for c in main.execute("PRAGMA table_info(board_rotation_signal)")]
        date_col = next((c for c in ("signal_date", "trade_date", "as_of_date", "date") if c in cols), None)
        if not date_col:
            parts.append("无日期列")
            sig_days = 0
        else:
            rows = main.execute(
                f"SELECT {date_col}, COUNT(*) FROM board_rotation_signal GROUP BY {date_col} ORDER BY {date_col} DESC LIMIT ?",
                (days,),
            ).fetchall()
            sig_days = len(rows)
            parts.append(f"rotation_signal {sig_days} 天" + (f"（最新 {rows[0][0]}）" if rows else ""))
    rank_days = 0
    if table_exists(main, "board_ranking_daily"):
        rank_days = main.execute(
            "SELECT COUNT(DISTINCT trade_date) FROM board_ranking_daily"
        ).fetchone()[0] if any(
            c[1] == "trade_date" for c in main.execute("PRAGMA table_info(board_ranking_daily)")
        ) else 0
        parts.append(f"board_ranking_daily {rank_days} 天")
    r.detail = " | ".join(parts)
    if sig_days >= 2 and rank_days >= 2:
        r.status = OK
    elif sig_days >= 1:
        r.status = PARTIAL
    else:
        r.status = FAIL
    return r


def check_p6(main, days):
    """P6 二层归因：attribution_benchmark_daily 最近交易日覆盖 + board_pct 非空率。"""
    r = Result("P6", "二层归因")
    if not table_exists(main, "attribution_benchmark_daily"):
        r.status = FAIL
        r.detail = "attribution_benchmark_daily 缺失"
        return r
    rows = main.execute(
        """SELECT trade_date, COUNT(*), COUNT(market_pct), COUNT(board_pct)
           FROM attribution_benchmark_daily
           GROUP BY trade_date ORDER BY trade_date DESC LIMIT ?""",
        (days,),
    ).fetchall()
    if not rows:
        r.status = FAIL
        r.detail = "无数据"
        return r
    latest = rows[0][0]
    total = sum(x[1] for x in rows)
    mkt = sum(x[2] for x in rows)
    brd = sum(x[3] for x in rows)
    r.detail = (
        f"{len(rows)} 天 / {total} 行（最新 {latest}）"
        f" market_pct 非空 {mkt}/{total} board_pct 非空 {brd}/{total}"
    )
    today = datetime.now().date()
    try:
        lag = (today - datetime.strptime(latest, "%Y-%m-%d").date()).days
    except Exception:
        lag = 999
    if lag > 7:
        r.status = PARTIAL
        r.detail += f" | 最新一天距今 {lag} 天，可能已停跑"
    elif brd == 0:
        r.status = PARTIAL
        r.detail += " | board_pct 全空（板块日线缺）"
    else:
        r.status = OK
    return r


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--days", type=int, default=5)
    ap.add_argument("--log-tail", type=int, default=20000)
    args = ap.parse_args()

    for p in (MAIN_DB, DELIVERY_DB):
        if not os.path.exists(p):
            print(f"缺少数据库: {p}")
            return 2

    main_db = ro(MAIN_DB)
    delivery_db = ro(DELIVERY_DB)

    log_lines = []
    if os.path.exists(STDERR_LOG):
        with open(STDERR_LOG, "r", errors="replace") as f:
            # 取尾部 N 行
            from collections import deque
            log_lines = list(deque(f, maxlen=args.log_tail))

    checks = [
        check_p11(main_db, args.days, log_lines),
        check_p12(delivery_db, args.days),
        check_p13(main_db, args.days),
        check_p14(main_db, args.days),
        check_p2(main_db, args.days),
        check_p33(log_lines),
        check_p4(delivery_db, args.days),
        check_p5(main_db, args.days),
        check_p6(main_db, args.days),
    ]

    print(f"=== ticket 阶段验收（{datetime.now():%Y-%m-%d %H:%M}，日志 {len(log_lines)} 行）===\n")
    for c in checks:
        print(c)

    bad = [c for c in checks if c.status == FAIL]
    warn = [c for c in checks if c.status == PARTIAL]
    print(f"\n达标 {sum(1 for c in checks if c.status == OK)} / 部分 {len(warn)} / 未达标 {len(bad)}")
    if bad:
        print("\n未达标项：")
        for c in bad:
            print(f"  {c.phase} {c.name} — {c.detail}")

    main_db.close()
    delivery_db.close()
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
