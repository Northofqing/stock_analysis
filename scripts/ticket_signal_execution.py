#!/usr/bin/env python3
"""ticket 信号结果账本（v1，旁路，不触碰 paper_ledger_*）。

从 `ticket_event(kind='entry_triggered')` + 最近一份 `ticket_decision`
+ `stock_daily` 回放“这次触发是否真的能买到”，写独立表
`ticket_signal_execution`，用于胜率分母。

默认 dry-run（只读）；只有显式 --apply 才建表/写库。

成交口径（v1，日线级，不假装盘口）:
- 一字向上（high==low 且 pct_chg>0）→ limit_up_sealed，不成交；
- 竞价高于区间上沿 3% 以上 → unfillable_gap，不成交；
- 当日最低仍高于区间上沿 → no_touch，不成交；
- 开盘在区间内 → 开盘价成交；高于上沿后回落 → 上沿成交；
  低于下沿后回升 → 下沿成交；
- breakout：high < trigger → no_trigger；开盘已过 trigger → 开盘价；
  否则 trigger 价成交；
- 无当日日线 → missing_bar；无决策上下文 → no_decision；
- 成交数量统一 100 股（归一化），费用在报告里按双边 0.2% 扣。
"""

from __future__ import annotations

import argparse
import hashlib
import sqlite3
import statistics
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass
from datetime import date
from typing import Any


COST_ROUND_TRIP = 0.002
DEFAULT_QTY = 100
RULE_VERSION = "ticket-signal-execution-v1"

DDL = """
CREATE TABLE IF NOT EXISTS ticket_signal_execution (
    execution_id TEXT PRIMARY KEY,
    ticket_id INTEGER NOT NULL,
    decision_id TEXT,
    code TEXT NOT NULL,
    business_date DATE NOT NULL,
    side TEXT NOT NULL,
    fill_status TEXT NOT NULL,
    fill_price REAL,
    quantity INTEGER NOT NULL,
    fees REAL NOT NULL DEFAULT 0,
    fill_basis TEXT,
    source TEXT NOT NULL,
    rule_version TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    d1_trade_date DATE,
    d1_close REAL,
    d1_net_pct REAL,
    d3_trade_date DATE,
    d3_close REAL,
    d3_net_pct REAL,
    outcome_updated_at TIMESTAMP,
    UNIQUE(decision_id, business_date, side)
)
"""

OUTCOME_COLUMNS = {
    "d1_trade_date": "DATE",
    "d1_close": "REAL",
    "d1_net_pct": "REAL",
    "d3_trade_date": "DATE",
    "d3_close": "REAL",
    "d3_net_pct": "REAL",
    "outcome_updated_at": "TIMESTAMP",
}


@dataclass
class Attempt:
    ticket_id: int
    code: str
    business_date: str
    decision_id: str | None
    entry_mode: str | None
    price_low: float | None
    price_high: float | None
    trigger_level: float | None
    fill_status: str
    fill_price: float | None
    fill_basis: str | None
    quantity: int = DEFAULT_QTY
    fees: float = 0.0

    @property
    def execution_id(self) -> str:
        raw = f"{self.ticket_id}|{self.business_date}|buy|{RULE_VERSION}"
        return hashlib.sha256(raw.encode()).hexdigest()


def classify(
    entry_mode: str | None,
    price_low: float | None,
    price_high: float | None,
    trigger_level: float | None,
    bar: sqlite3.Row | None,
) -> tuple[str, float | None, str | None]:
    """纯函数：返回 (fill_status, fill_price, fill_basis)。"""
    if bar is None:
        return "missing_bar", None, None
    open_, high, low, close = bar["open"], bar["high"], bar["low"], bar["close"]
    pct = bar["pct_chg"]
    if open_ is None or high is None or low is None or close is None:
        return "missing_bar", None, None

    # 一字向上：没有可成交的日内价格。
    if high == low and pct is not None and pct > 0:
        return "limit_up_sealed", None, "one_word_up"

    mode = (entry_mode or "").lower()
    if mode == "breakout" and trigger_level is not None and trigger_level > 0:
        if high < trigger_level:
            return "no_trigger", None, None
        if open_ >= trigger_level:
            return "filled", open_, "breakout_gap_open"
        return "filled", trigger_level, "breakout_level"

    if price_low is None or price_high is None or price_high <= 0:
        return "unverified", None, None
    if open_ > price_high * 1.03:
        return "unfillable_gap", None, None
    if low > price_high:
        return "no_touch", None, None
    if price_low <= open_ <= price_high:
        return "filled", open_, "open_in_zone"
    if open_ > price_high:
        return "filled", price_high, "retrace_to_high"
    if open_ < price_low:
        return "filled", price_low, "rise_into_low"
    return "unverified", None, None


def load_attempts(conn: sqlite3.Connection, from_date: str | None, to_date: str | None) -> list[Attempt]:
    conditions = ["e.event_kind = 'entry_triggered'"]
    params: list[Any] = []
    if from_date:
        conditions.append("substr(e.event_at, 1, 10) >= ?")
        params.append(from_date)
    if to_date:
        conditions.append("substr(e.event_at, 1, 10) <= ?")
        params.append(to_date)
    rows = conn.execute(
        f"""
        SELECT e.ticket_id, t.code, substr(e.event_at, 1, 10) AS business_date,
               e.event_at, e.summary
        FROM ticket_event e
        JOIN ticket_lifecycle t ON t.id = e.ticket_id
        WHERE {' AND '.join(conditions)}
        ORDER BY e.event_at ASC, e.id ASC
        """,
        params,
    ).fetchall()

    # 同一票同一天只取最早一次触发（与 6.8 评估口径一致）。
    deduped: dict[tuple[int, str], sqlite3.Row] = {}
    for row in rows:
        deduped.setdefault((row["ticket_id"], row["business_date"]), row)

    bar_cache: dict[tuple[str, str], sqlite3.Row | None] = {}
    out: list[Attempt] = []
    for (ticket_id, business_date), event in sorted(deduped.items()):
        code = event["code"]
        decision = conn.execute(
            """
            SELECT decision_id, entry_mode, price_low, price_high, trigger_level
            FROM ticket_decision
            WHERE ticket_id = ? AND as_of <= ?
            ORDER BY as_of DESC, created_at DESC, decision_id DESC
            LIMIT 1
            """,
            (ticket_id, business_date),
        ).fetchone()
        if decision is None:
            out.append(
                Attempt(
                    ticket_id, code, business_date, None, None, None, None, None,
                    "no_decision", None, None,
                )
            )
            continue
        key = (code, business_date)
        if key not in bar_cache:
            bar_cache[key] = conn.execute(
                "SELECT open, high, low, close, pct_chg FROM stock_daily WHERE code=? AND date=?",
                key,
            ).fetchone()
        status, price, basis = classify(
            decision["entry_mode"],
            decision["price_low"],
            decision["price_high"],
            decision["trigger_level"],
            bar_cache[key],
        )
        out.append(
            Attempt(
                ticket_id,
                code,
                business_date,
                decision["decision_id"],
                decision["entry_mode"],
                decision["price_low"],
                decision["price_high"],
                decision["trigger_level"],
                status,
                price,
                basis,
            )
        )
    return out


def next_closes(conn: sqlite3.Connection, code: str, business_date: str, horizon: int) -> float | None:
    rows = conn.execute(
        "SELECT date, close FROM stock_daily WHERE code=? AND date>? AND close IS NOT NULL "
        "ORDER BY date ASC LIMIT ?",
        (code, business_date, horizon),
    ).fetchall()
    if len(rows) < horizon:
        return None
    return float(rows[horizon - 1]["close"])


def next_close_row(
    conn: sqlite3.Connection, code: str, business_date: str, horizon: int
) -> tuple[str, float] | None:
    rows = conn.execute(
        "SELECT date, close FROM stock_daily WHERE code=? AND date>? AND close IS NOT NULL "
        "ORDER BY date ASC LIMIT ?",
        (code, business_date, horizon),
    ).fetchall()
    if len(rows) < horizon:
        return None
    row = rows[horizon - 1]
    return str(row["date"]), float(row["close"])


def ensure_outcome_columns(conn: sqlite3.Connection) -> None:
    existing = {
        row[1] for row in conn.execute("PRAGMA table_info(ticket_signal_execution)")
    }
    for name, column_type in OUTCOME_COLUMNS.items():
        if name in existing:
            continue
        conn.execute(
            f"ALTER TABLE ticket_signal_execution ADD COLUMN {name} {column_type}"
        )


def update_outcomes(conn: sqlite3.Connection, attempts: list[Attempt]) -> int:
    ensure_outcome_columns(conn)
    updated = 0
    with conn:
        for attempt in attempts:
            if attempt.fill_status != "filled" or not attempt.fill_price:
                continue
            fill_price = float(attempt.fill_price)
            d1 = next_close_row(conn, attempt.code, attempt.business_date, 1)
            d3 = next_close_row(conn, attempt.code, attempt.business_date, 3)
            if d1 is None and d3 is None:
                continue
            d1_net = d1[1] / fill_price - 1.0 - COST_ROUND_TRIP if d1 else None
            d3_net = d3[1] / fill_price - 1.0 - COST_ROUND_TRIP if d3 else None
            cursor = conn.execute(
                """
                UPDATE ticket_signal_execution
                   SET d1_trade_date=?, d1_close=?, d1_net_pct=?,
                       d3_trade_date=?, d3_close=?, d3_net_pct=?,
                       outcome_updated_at=CURRENT_TIMESTAMP
                 WHERE execution_id=?
                """,
                (
                    d1[0] if d1 else None,
                    d1[1] if d1 else None,
                    d1_net,
                    d3[0] if d3 else None,
                    d3[1] if d3 else None,
                    d3_net,
                    attempt.execution_id,
                ),
            )
            updated += cursor.rowcount
    return updated


def summarize(attempts: list[Attempt], conn: sqlite3.Connection) -> dict[str, Any]:
    status_counts = Counter(a.fill_status for a in attempts)
    filled = [a for a in attempts if a.fill_status == "filled" and a.fill_price and a.fill_price > 0]
    result: dict[str, Any] = {
        "attempts": len(attempts),
        "status": status_counts,
        "filled": len(filled),
    }
    for horizon in (1, 3):
        returns: list[float] = []
        for attempt in filled:
            fill_price = attempt.fill_price
            assert fill_price is not None and fill_price > 0
            exit_price = next_closes(conn, attempt.code, attempt.business_date, horizon)
            if exit_price is None:
                continue
            returns.append(exit_price / fill_price - 1.0 - COST_ROUND_TRIP)
        wins = [value for value in returns if value > 0]
        losses = [value for value in returns if value < 0]
        result[f"d{horizon}_n"] = len(returns)
        result[f"d{horizon}_win_rate"] = (len(wins) / len(returns)) if returns else None
        result[f"d{horizon}_mean"] = statistics.mean(returns) if returns else None
        result[f"d{horizon}_median"] = statistics.median(returns) if returns else None
        result[f"d{horizon}_profit_factor"] = (
            sum(wins) / abs(sum(losses)) if losses and sum(losses) != 0 else None
        )
    return result


def report(attempts: list[Attempt], conn: sqlite3.Connection) -> None:
    summary = summarize(attempts, conn)
    print(f"触发样本: {summary['attempts']}；可成交: {summary['filled']}")
    print(
        "状态: "
        + ", ".join(f"{key}={value}" for key, value in summary["status"].most_common())
    )
    for horizon in (1, 3):
        n = summary[f"d{horizon}_n"]
        if not n:
            print(f"D+{horizon}: 无可计算样本")
            continue
        print(
            f"D+{horizon}: n={n} 净胜率={summary[f'd{horizon}_win_rate']:.1%} "
            f"均值={summary[f'd{horizon}_mean']:+.2%} "
            f"中位={summary[f'd{horizon}_median']:+.2%} "
            f"盈亏比={summary[f'd{horizon}_profit_factor'] or float('nan'):.2f}"
        )
    by_mode: dict[str, list[Attempt]] = defaultdict(list)
    for attempt in attempts:
        by_mode[attempt.entry_mode or "unknown"].append(attempt)
    for mode, group in sorted(by_mode.items()):
        group_summary = summarize(group, conn)
        n = group_summary["d1_n"]
        if n:
            print(
                f"  {mode}: 触发={group_summary['attempts']} 可成交={group_summary['filled']} "
                f"D+1 n={n} 胜率={group_summary['d1_win_rate']:.1%}"
            )


def apply(conn: sqlite3.Connection, attempts: list[Attempt]) -> int:
    conn.execute(DDL)
    inserted = 0
    with conn:
        for attempt in attempts:
            cursor = conn.execute(
                """
                INSERT OR IGNORE INTO ticket_signal_execution
                (execution_id, ticket_id, decision_id, code, business_date, side,
                 fill_status, fill_price, quantity, fees, fill_basis, source, rule_version)
                VALUES (?, ?, ?, ?, ?, 'buy', ?, ?, ?, ?, ?, 'ticket_signal_backfill', ?)
                """,
                (
                    attempt.execution_id,
                    attempt.ticket_id,
                    attempt.decision_id,
                    attempt.code,
                    attempt.business_date,
                    attempt.fill_status,
                    attempt.fill_price,
                    attempt.quantity,
                    attempt.fees,
                    attempt.fill_basis,
                    RULE_VERSION,
                ),
            )
            inserted += cursor.rowcount
        # 日线可能晚于首次 attempt 落库（例如 11-12 在 21:30 前先记
        # missing_bar）。重跑时必须把已有行从 missing_bar 修正为 filled，
        # 否则 INSERT OR IGNORE 会永久保留错误状态。
        for attempt in attempts:
            conn.execute(
                """
                UPDATE ticket_signal_execution
                   SET decision_id=COALESCE(?, decision_id),
                       fill_status=?, fill_price=?, quantity=?, fees=?,
                       fill_basis=?, source='ticket_signal_backfill',
                       rule_version=?
                 WHERE execution_id=?
                """,
                (
                    attempt.decision_id,
                    attempt.fill_status,
                    attempt.fill_price,
                    attempt.quantity,
                    attempt.fees,
                    attempt.fill_basis,
                    RULE_VERSION,
                    attempt.execution_id,
                ),
            )
    return inserted


def apply_and_update(conn: sqlite3.Connection, attempts: list[Attempt]) -> tuple[int, int]:
    inserted = apply(conn, attempts)
    updated = update_outcomes(conn, attempts)
    return inserted, updated


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--database", required=True)
    parser.add_argument("--from-date")
    parser.add_argument("--to-date")
    parser.add_argument("--apply", action="store_true", help="建表并写入；默认 dry-run")
    args = parser.parse_args()

    mode = "rw" if args.apply else "ro"
    conn = sqlite3.connect(f"file:{args.database}?mode={mode}", uri=True)
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA busy_timeout=30000")
    try:
        attempts = load_attempts(conn, args.from_date, args.to_date)
        report(attempts, conn)
        if args.apply:
            inserted, updated = apply_and_update(conn, attempts)
            print(f"已写入 ticket_signal_execution: {inserted} 条（重复忽略）")
            print(f"已回填 D+1/D+3 结果: {updated} 条")
        else:
            print("dry-run：未写库；确认口径后加 --apply")
        return 0
    finally:
        conn.close()


if __name__ == "__main__":
    sys.exit(main())
