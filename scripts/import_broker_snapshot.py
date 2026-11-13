#!/usr/bin/env python3
"""把券商对账单文本导入系统，并按「交易所收盘价」重算市值/资产（2026-10-26 定的口径）。

规则（用户确认）：
- 券商页面只取「现金」与「数量/成本」；市值与资产一律用 stock_daily 的最新收盘价重算。
- 原因：券商「当前价」会滞后，甚至给的是当日成交均价（2026-10-26 600667 给 14.34，收盘 13.93）。

用法：
    python3 scripts/import_broker_snapshot.py --text-file broker.txt --db <db> [--apply]

不带 --apply 时只打印解析结果与差异，不写库。
"""
import argparse
import datetime as dt
import json
import re
import sqlite3
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
IMPORTER_SNAP = ROOT / "target/release/import_user_position_snapshot"
IMPORTER_SUM = ROOT / "target/release/import_user_account_summary"

MONEY = {
    "余额": r"余额[:：]\s*([0-9.]+)",
    "可用": r"可用[:：]\s*([0-9.]+)",
    "可取": r"可取[:：]\s*([0-9.]+)",
    "参考市值": r"参考市值[:：]\s*([0-9.]+)",
    "资产": r"资产[:：]\s*([0-9.]+)",
    "盈亏": r"盈亏[:：]\s*(-?[0-9.]+)",
}
ROW = re.compile(
    r"^(\d{6})\s+(\S+)\s+(-?\d+)\s+(-?\d+)\s+(-?\d+)\s+([0-9.]+)\s+([0-9.]+)\s+([0-9.]+)\s+(-?[0-9.]+)\s+(-?[0-9.]+)"
)
ORDER_ROW = re.compile(
    r"^(\d{8})\s+(\d{2}:\d{2}:\d{2})\s+(\d{6})\s+(\S+)\s+(证券[买卖][入出])\s+"
    r"([0-9.]+)\s+(\d+)\s+(\d+)\s+(\d+)\s+([0-9.]+)\s+(\d+)\s+(\S+)\s+(\S+)\s+(\S+)\s+(\d{8})"
)


def parse_orders(text: str):
    """解析当日委托表；只用于判断是否有未终结委托/冻结资金，不写库。"""
    orders = []
    for line in text.splitlines():
        m = ORDER_ROW.match(line.strip())
        if not m:
            continue
        (
            date,
            time,
            code,
            name,
            side,
            price,
            order_qty,
            order_no,
            filled_qty,
            filled_amount,
            cancel_qty,
            status,
            flag,
            shareholder,
            op_date,
        ) = m.groups()
        orders.append(
            {
                "date": date,
                "time": time,
                "code": code,
                "name": name,
                "side": side,
                "price": float(price),
                "order_qty": int(order_qty),
                "order_no": order_no,
                "filled_qty": int(filled_qty),
                "filled_amount": float(filled_amount),
                "cancel_qty": int(cancel_qty),
                "status": status,
                "flag": flag,
                "shareholder": shareholder,
                "op_date": op_date,
            }
        )
    return orders


def parse(text: str):
    cash = {}
    for name, pattern in MONEY.items():
        m = re.search(pattern, text)
        cash[name] = float(m.group(1)) if m else None
    rows = []
    for line in text.splitlines():
        m = ROW.match(line.strip())
        if not m:
            continue
        code, name, holding, inventory, sellable, cost, price, value, pnl, pct = m.groups()
        rows.append(
            {
                "code": code,
                "name": name,
                "holding": int(holding),
                "quantity": int(inventory),  # 库存数量=总持仓（含当日买入）
                "sellable": int(sellable),
                "cost_price": float(cost),
                "broker_price": float(price),
                "broker_value": float(value),
                "broker_pnl": float(pnl),
            }
        )
    return cash, rows


def latest_closes(db: str):
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    date = con.execute("SELECT MAX(date) FROM stock_daily").fetchone()[0]
    closes = dict(con.execute("SELECT code, close FROM stock_daily WHERE date=?", (date,)))
    con.close()
    return date, closes


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--text-file", required=True)
    ap.add_argument("--db", required=True)
    ap.add_argument("--apply", action="store_true")
    ap.add_argument(
        "--broker-assets",
        action="store_true",
        help="账户汇总用券商页面的参考市值/资产，而不是按收盘价重算",
    )
    ap.add_argument(
        "--allow-pending",
        action="store_true",
        help="存在未终结委托仍允许写库（不推荐；冻结资金可能让可用失真）",
    )
    args = ap.parse_args()

    text = Path(args.text_file).read_text()
    cash, rows = parse(text)
    orders = parse_orders(text)
    pending = [order for order in orders if order["status"] == "已报"]
    frozen_cash = sum(
        max(0, order["order_qty"] - order["filled_qty"] - order["cancel_qty"]) * order["price"]
        for order in pending
        if order["side"].startswith("证券买")
    )
    if not rows:
        print("未解析到持仓行", file=sys.stderr)
        return 2
    active_rows = [row for row in rows if row["quantity"] > 0]
    if not active_rows:
        print("未解析到库存数量大于 0 的持仓", file=sys.stderr)
        return 2
    date, closes = latest_closes(args.db)
    if cash.get("可用") is None:
        print("未解析到「可用」", file=sys.stderr)
        return 2

    print(f"收盘日 {date}｜可用 {cash['可用']:.2f}｜余额 {cash.get('余额')}｜券商资产 {cash.get('资产')}")
    print(
        f"{'代码':<8}{'名称':<10}{'证券':>6}{'库存':>6}{'可卖':>6}"
        f"{'成本':>9}{'券商价':>9}{'收盘':>9}{'差额':>8}{'市值(收盘)':>12}"
    )
    market = 0.0
    warn = []
    for row in rows:
        close = closes.get(row["code"])
        if close is None:
            warn.append(f"{row['code']} 无 {date} 收盘价")
            close = row["broker_price"]
        diff = row["broker_price"] - close
        market += row["quantity"] * close
        print(
            f"{row['code']:<8}{row['name']:<10}{row['holding']:>6}{row['quantity']:>6}"
            f"{row['sellable']:>6}{row['cost_price']:>9.3f}"
            f"{row['broker_price']:>9.3f}{close:>9.3f}{diff:>+8.3f}{row['quantity']*close:>12.2f}"
        )
        if row["quantity"] == 0:
            warn.append(f"{row['code']} {row['name']} 库存为 0，已排除出持仓快照")
        elif row["holding"] != row["quantity"]:
            warn.append(
                f"{row['code']} {row['name']} 证券数量 {row['holding']} != 库存 {row['quantity']}，按库存入库"
            )
        if abs(diff) > 0.005:
            warn.append(
                f"{row['code']} {row['name']} 券商价 {row['broker_price']:.2f} vs 收盘 {close:.2f}"
            )

    market = round(market, 2)
    close_total = round(market + cash["可用"], 2)
    if args.broker_assets:
        if cash.get("参考市值") is None or cash.get("资产") is None:
            print("--broker-assets 需要解析到「参考市值」与「资产」", file=sys.stderr)
            return 2
        market = round(cash["参考市值"], 2)
        total = round(cash["资产"], 2)
        ratio = round(market / total * 100, 3) if total else 0.0
        print(
            f"\n券商口径：市值 {market:.2f}｜资产 {total:.2f}｜仓位 {ratio:.2f}%"
            f"｜市值+可用 {market + cash['可用']:.2f}｜未解释差额 {total - market - cash['可用']:+.2f}"
        )
        print(f"收盘重算口径：市值 {round(sum(r['quantity'] * closes.get(r['code'], r['broker_price']) for r in rows), 2):.2f}｜资产 {close_total:.2f}")
    else:
        total = close_total
        ratio = round(market / total * 100, 3) if total else 0.0
        print(f"\n收盘口径：市值 {market:.2f}｜资产 {total:.2f}｜仓位 {ratio:.2f}%")
    if cash.get("资产") is not None:
        print(f"券商口径：市值 {cash.get('参考市值')}｜资产 {cash['资产']}｜差额 {total - cash['资产']:+.2f}")

    pnl_sum = round(sum(row["broker_pnl"] for row in rows), 2)
    print(f"持仓浮盈亏合计 {pnl_sum:+.2f}｜券商盈亏 {cash.get('盈亏')}")
    for line in warn:
        print(f"  ⚠ {line}")

    if pending:
        print("\n⚠ 未终结委托（页面可能未到收盘/结算）：")
        for order in pending:
            remaining = max(
                0, order["order_qty"] - order["filled_qty"] - order["cancel_qty"]
            )
            print(
                f"  {order['time']} {order['code']} {order['name']} {order['side']} "
                f"{order['price']:.2f}×{order['order_qty']} 成交{order['filled_qty']} "
                f"状态{order['status']}｜未成交 {remaining}"
            )
        if frozen_cash > 0:
            print(
                f"  买入冻结估算 {frozen_cash:.2f}；可用 {cash['可用']:.2f} 可能不是收盘可用"
            )
        if cash.get("余额") is not None:
            print(f"  余额-可用 = {cash['余额'] - cash['可用']:.2f}")
        print("  建议收盘结算后重导；确认已终结再加 --allow-pending")

    if not args.apply:
        print("\ndry-run：未写库")
        return 0
    if pending and not args.allow_pending:
        print("存在未终结委托，拒绝 --apply；确认后加 --allow-pending", file=sys.stderr)
        return 3

    now = dt.datetime.now().astimezone().replace(microsecond=0).isoformat()
    snap = {
        "schema_version": 1,
        "effective_at": now,
        "confirm_empty": False,
        "items": [
            {"code": r["code"], "name": r["name"], "quantity": r["quantity"], "cost_price": r["cost_price"]}
            for r in active_rows
        ],
    }
    summary = {
        "effective_at": now,
        "total_assets": total,
        "securities_market_value": market,
        "available_cash": cash["可用"],
        "position_ratio_pct": ratio,
        "daily_pnl": 0.0,
    }
    with tempfile.TemporaryDirectory() as tmp:
        snap_path = Path(tmp) / "snapshot.json"
        sum_path = Path(tmp) / "summary.json"
        snap_path.write_text(json.dumps(snap, ensure_ascii=False, indent=2))
        sum_path.write_text(json.dumps(summary, ensure_ascii=False, indent=2))
        for label, binary, flag, path in [
            ("持仓快照", IMPORTER_SNAP, "--snapshot", snap_path),
            ("账户汇总", IMPORTER_SUM, "--summary", sum_path),
        ]:
            result = subprocess.run(
                [str(binary), "--database", args.db, flag, str(path)],
                capture_output=True,
                text=True,
            )
            print(f"\n[{label}] rc={result.returncode}")
            print((result.stdout or result.stderr).strip()[-400:])
            if result.returncode != 0:
                return result.returncode
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
