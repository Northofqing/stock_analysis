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
    """P1.1 观察池：最近 N 个交易日必须真的选出了候选票。

    判定口径（2026-10-22 修正）：看 `board_candidates`，不只看 `opened`。

    原因：`opened=0` 不等于失败 —— 选中的票已在 ticket_lifecycle 里时，
    走 `appended` 补事件路径，不新开票。2026-10-21 22:13 实测
    `boards=30 board_candidates=7 board_added=7 total=7 opened=0 appended=7`，
    观察池明明跑通并选出 7 只，旧口径仍判「未达标」。

    真正表示"观察池空转"的是 `board_candidates=0`。
    """
    r = Result("P1.1", "观察池构建")
    opened = news_cand = 0
    board_cand = 0
    ok_days = 0
    empty_rounds = 0
    source_unavailable = 0
    for line in log_lines:
        # 兼容两种历史格式：带 board_candidates 的新格式 / 不带它的旧格式
        m = re.search(
            r"\[观察池\] news_scanned=(\d+) news_candidates=(\d+)"
            r"(?: boards=(\d+) board_candidates=(\d+) board_added=(\d+))?"
            r".*?total=(\d+) opened=(\d+)",
            line,
        )
        if not m:
            continue
        news_cand += int(m.group(2))
        candidates = int(m.group(4)) if m.group(4) is not None else int(m.group(6))
        opened_here = int(m.group(7))
        board_cand += candidates
        opened += opened_here
        # 2026-10-27：日志新增三个健康字段；旧日志没有时保持 None。
        if re.search(r"news_health=(?:unavailable|stale|conflict)", line):
            source_unavailable += 1
        if re.search(r"board_health=(?:unavailable|stale|conflict)", line):
            source_unavailable += 1
        if re.search(r"mf_health=(?:unavailable|stale|conflict)", line):
            source_unavailable += 1
        if candidates > 0 or opened_here > 0 or int(m.group(2)) > 0:
            ok_days += 1
        elif m.group(4) is not None:
            empty_rounds += 1

    r.detail = (
        f"日志累计 board_candidates={board_cand} opened={opened} "
        f"news_candidates={news_cand} 有产出的轮次={ok_days}"
    )
    if source_unavailable:
        r.detail += f" | 来源不可用记录 {source_unavailable} 条"
    if ok_days > 0:
        r.status = OK
        if opened == 0:
            r.detail += " | opened=0 属正常（选中的票已在库，走 appended 补事件）"
    elif empty_rounds > 0:
        if source_unavailable > 0:
            r.status = PARTIAL
            r.detail += (
                f" | {empty_rounds} 轮无候选，但存在来源不可用；"
                "不能判定为观察池空转"
            )
        else:
            r.status = FAIL
            r.detail += (
                f" | {empty_rounds} 轮 board_candidates=0，观察池真空转"
                "（查 BoardFlows / 东财 BK 成分股 / news_ai_assessment）"
            )
    else:
        r.status = FAIL
        r.detail += " | 日志里没有观察池轮次（调度未触发或日志已被轮转）"
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
    """P2 可买价：不仅要产出 entry_proposed，还要看是否进入触发/执行。"""
    r = Result("P2", "可买价/entry_proposed")
    if not table_exists(main, "ticket_event"):
        r.detail = "ticket_event 缺失"
        r.status = FAIL
        return r
    dates = [
        row[0]
        for row in main.execute(
            "SELECT DISTINCT substr(event_at, 1, 10) FROM ticket_event "
            "WHERE event_at IS NOT NULL AND length(event_at) >= 10 "
            "ORDER BY 1 DESC LIMIT ?",
            (days,),
        ).fetchall()
    ]
    cutoff = min(dates) if dates else "0000-00-00"
    ev = main.execute(
        "SELECT COUNT(*) FROM ticket_event "
        "WHERE event_kind='entry_proposed' AND substr(event_at,1,10) >= ?",
        (cutoff,),
    ).fetchone()[0]
    trig = main.execute(
        "SELECT COUNT(*) FROM ticket_event "
        "WHERE event_kind='entry_triggered' AND substr(event_at,1,10) >= ?",
        (cutoff,),
    ).fetchone()[0]
    invalidated = main.execute(
        "SELECT COUNT(*) FROM ticket_event "
        "WHERE event_kind='entry_invalidated' AND substr(event_at,1,10) >= ?",
        (cutoff,),
    ).fetchone()[0]
    st = main.execute(
        "SELECT COUNT(*) FROM ticket_lifecycle WHERE stage='entry_proposed'"
    ).fetchone()[0]
    obs = main.execute(
        "SELECT COUNT(*) FROM ticket_lifecycle WHERE stage='observing' AND close_date IS NULL"
    ).fetchone()[0]
    decisions = 0
    if table_exists(main, "ticket_decision"):
        decisions = main.execute(
            "SELECT COUNT(*) FROM ticket_decision WHERE as_of >= ?", (cutoff,)
        ).fetchone()[0]
    r.detail = (
        f"窗口 {cutoff}~最新 entry_proposed={ev} 触发={trig} 失效={invalidated} "
        f"| 活跃状态={st} decision={decisions} | 仍停在 observing={obs}"
    )
    if ev == 0 and st == 0:
        r.status = FAIL
        r.detail += " | 从未产出过买入区间"
    elif trig == 0:
        r.status = PARTIAL
        r.detail += " | 有提案但窗口内 0 触发；不能算漏斗打通"
    else:
        r.status = OK
    return r


def check_p33(log_lines):
    """P3.3 同板块异动：失败 vs 成功必须同时可见，不能只数失败文本。"""
    r = Result("P3.3", "同板块关联票")
    fail = sum(1 for l in log_lines if "关联票查询失败" in l)
    coverage_rounds = 0
    requested = accepted = rejected = 0
    unavailable = 0
    for line in log_lines:
        m = re.search(
            r"\[观察池\]\[coverage\] scope=watch_peer(?:_failed)? "
            r"requested=(\d+) accepted=(\d+) missing=(\d+) rejected=(\d+).*?health=(\w+)",
            line,
        )
        if not m:
            continue
        coverage_rounds += 1
        requested += int(m.group(1))
        accepted += int(m.group(2))
        rejected += int(m.group(4))
        if m.group(5) in ("unavailable", "stale", "conflict"):
            unavailable += 1
    if coverage_rounds:
        r.detail = (
            f"coverage 轮次={coverage_rounds} requested={requested} accepted={accepted} "
            f"rejected={rejected} unavailable={unavailable}"
        )
        if requested == 0:
            r.status = SKIP
            r.detail += " | 没有关联票候选（不是行情失败）"
        elif accepted > 0:
            r.status = OK
            if rejected or unavailable:
                r.detail += " | 有成功也有缺口，按覆盖率为准"
        elif unavailable:
            r.status = PARTIAL
            r.detail += " | 关联票行情源不可用；不是“没有关联票”"
        else:
            r.status = PARTIAL
            r.detail += " | 有请求但无接受记录，需查 coverage"
        return r

    # 兼容旧日志：没有 coverage 结构时仍看失败/命中，但明确标为旧口径。
    r.detail = f"旧口径：最近日志里「关联票查询失败」{fail} 次"
    if fail == 0:
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


def check_batch1(main, log_lines):
    """批 1 分型体系（2026-10-23）—— 只认产出：事件行 + 专属日志行。

    判据与方案 1.x 一一对应：
    - 1.1 分型：活跃票里有分型事件的比例；日志出现 `[分型] scanned=`
    - 1.3 竞价买价：日志出现 `[竞价买价] emotion=`；`quoted>0` 才算有价
    - 1.4 突破确认：`entry_triggered` 事件 + `[触发确认] 预热` 日志
    - 1.5 价格失效：`entry_invalidated` 事件 + `[价格失效] 时效检查` 日志
    - 2.3 资金流入口：观察池日志的 `mf_candidates` 字段 + f62 榜行数
    """
    out = []

    r = Result("批1.1", "分型 regime")
    if not table_exists(main, "ticket_event"):
        r.status = FAIL
        r.detail = "ticket_event 缺失"
        out.append(r)
    else:
        n = main.execute(
            "SELECT COUNT(*) FROM ticket_event WHERE event_kind='regime'"
        ).fetchone()[0]
        active = main.execute(
            "SELECT COUNT(*) FROM ticket_lifecycle WHERE close_date IS NULL"
        ).fetchone()[0]
        covered = main.execute(
            """SELECT COUNT(DISTINCT t.id) FROM ticket_lifecycle t
               JOIN ticket_event e ON e.ticket_id = t.id AND e.event_kind='regime'
               WHERE t.close_date IS NULL"""
        ).fetchone()[0]
        log_hit = sum(1 for line in log_lines if "[分型]" in line)
        r.detail = (
            f"regime 事件 {n} 条；活跃票 {active} 只中已分型 {covered} 只；"
            f"日志 [分型] {log_hit} 行"
        )
        if n == 0 or covered == 0:
            r.status = FAIL
        elif covered < active or log_hit == 0:
            r.status = PARTIAL
        else:
            r.status = OK
        out.append(r)

    r = Result("批1.3", "竞价买价")
    quoted = 0
    emotion = 0
    lines = 0
    for line in log_lines:
        m = re.search(r"\[竞价买价\] emotion=(\d+) quoted=(\d+)", line)
        if not m:
            continue
        lines += 1
        emotion = max(emotion, int(m.group(1)))
        quoted = max(quoted, int(m.group(2)))
    r.detail = f"[竞价买价] 日志 {lines} 行，情绪型最多 {emotion} 只、给价 {quoted} 只"
    if lines == 0:
        r.status = FAIL
        r.detail += " | 没有这条日志（调度未触发或代码未部署）"
    elif emotion == 0:
        r.status = PARTIAL
        r.detail += " | 当日无情绪型票，属数据形态不是失败"
    elif quoted == 0:
        r.status = PARTIAL
        r.detail += " | 情绪型票全部落在不给价档（高开过多/缺价）"
    else:
        r.status = OK
    out.append(r)

    r = Result("批1.4", "突破确认")
    triggered = main.execute(
        "SELECT COUNT(*) FROM ticket_event WHERE event_kind='entry_triggered'"
    ).fetchone()[0]
    warm = sum(1 for line in log_lines if "[触发确认] 预热" in line)
    r.detail = f"entry_triggered 事件 {triggered} 条；[触发确认] 预热 {warm} 行"
    if warm == 0:
        r.status = FAIL
        r.detail += " | 预热未发生（entry_proposed 为空或调度未跑到）"
    elif triggered == 0:
        r.status = PARTIAL
        r.detail += " | 有预热但当日无票站上昨高"
    else:
        r.status = OK
    out.append(r)

    r = Result("批1.5", "价格失效")
    invalidated = main.execute(
        "SELECT COUNT(*) FROM ticket_event WHERE event_kind IN ('entry_invalidated','entry_expired')"
    ).fetchone()[0]
    expiry = sum(1 for line in log_lines if "[价格失效] 时效检查" in line)
    r.detail = (
        f"entry_invalidated/entry_expired 事件 {invalidated} 条；"
        f"[价格失效] 时效检查 {expiry} 行"
    )
    if expiry == 0:
        r.status = FAIL
        r.detail += " | 15:42 时效检查没有日志"
    elif invalidated == 0:
        r.status = PARTIAL
        r.detail += " | 无失效/到期（票都还在价格窗口内）"
    else:
        r.status = OK
    out.append(r)

    r = Result("批2.3", "资金流入口")
    mf_log = 0
    mf_candidates = 0
    for line in log_lines:
        m = re.search(r"\[观察池\] .*?mf_candidates=(\d+)", line)
        if not m:
            continue
        mf_log += 1
        mf_candidates = max(mf_candidates, int(m.group(1)))
    f62_days = 0
    if table_exists(main, "board_ranking_daily"):
        f62_days = main.execute(
            "SELECT COUNT(DISTINCT trade_date) FROM board_ranking_daily WHERE rank_fid='f62'"
        ).fetchone()[0]
    r.detail = f"观察池日志 {mf_log} 行，mf_candidates 最多 {mf_candidates}；f62 榜 {f62_days} 天"
    if mf_log == 0:
        r.status = FAIL
        r.detail += " | 日志没有 mf_candidates 字段（新代码未部署）"
    elif f62_days == 0:
        r.status = PARTIAL
        r.detail += " | f62 榜无数据（东财臂被拦截），入口空转属上游缺口"
    elif mf_candidates == 0:
        r.status = PARTIAL
        r.detail += " | 有 f62 数据但没选出票（bk 映射/成分缺失）"
    else:
        r.status = OK
    out.append(r)

    return out


def read_log_window(latest_day, max_lines, max_bytes):
    """读日志窗口。

    旧实现固定取尾部 2 万行，2026-10-26 3.0GB 日志下只能覆盖 17 点后的
    BuildIdentity 噪音，把当天 09:25/15:45 的产出误判成“调度没跑”。
    这里优先按最近交易日日期锚定起点；找不到锚点才退回尾部行数模式。
    """
    if not os.path.exists(STDERR_LOG):
        return [], "日志不存在"
    size = os.path.getsize(STDERR_LOG)
    start = max(0, size - max_bytes)
    note = f"尾部 {max_bytes // (1024 * 1024)}MB"
    anchored = False
    if latest_day:
        marker = latest_day.encode()
        try:
            import mmap

            with open(STDERR_LOG, "rb") as f:
                with mmap.mmap(f.fileno(), 0, access=mmap.ACCESS_READ) as mm:
                    idx = mm.find(marker)
                    if idx >= 0:
                        start = mm.rfind(b"\n", 0, idx) + 1
                        anchored = True
                        note = f"从 {latest_day} 日期锚点 byte={start}"
                        if size - start > max_bytes:
                            note += f"（截断 {max_bytes // (1024 * 1024)}MB）"
        except (OSError, ValueError) as error:
            note = f"日期锚点失败({error})，尾部 {max_bytes // (1024 * 1024)}MB"
    with open(STDERR_LOG, "rb") as f:
        f.seek(start)
        raw = f.read(max_bytes)
    lines = raw.decode("utf-8", "replace").splitlines()
    if not anchored:
        lines = lines[-max_lines:]
    return lines, note


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--days", type=int, default=5)
    ap.add_argument("--log-tail", type=int, default=20000)
    ap.add_argument("--log-window-mb", type=int, default=512)
    args = ap.parse_args()

    for p in (MAIN_DB, DELIVERY_DB):
        if not os.path.exists(p):
            print(f"缺少数据库: {p}")
            return 2

    main_db = ro(MAIN_DB)
    delivery_db = ro(DELIVERY_DB)

    days = trading_days(main_db, args.days)
    log_lines, log_note = read_log_window(
        days[0] if days else None,
        args.log_tail,
        max(1, args.log_window_mb) * 1024 * 1024,
    )

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
    checks.extend(check_batch1(main_db, log_lines))

    print(f"=== ticket 阶段验收（{datetime.now():%Y-%m-%d %H:%M}，日志 {len(log_lines)} 行，窗口 {log_note}）===\n")
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
