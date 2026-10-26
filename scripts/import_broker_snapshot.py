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
    args = ap.parse_args()

    text = Path(args.text_file).read_text()
    cash, rows = parse(text)
    if not rows:
        print("未解析到持仓行", file=sys.stderr)
        return 2
    date, closes = latest_closes(args.db)
    if cash.get("可用") is None:
        print("未解析到「可用」", file=sys.stderr)
        return 2

    print(f"收盘日 {date}｜可用 {cash['可用']:.2f}｜余额 {cash.get('余额')}｜券商资产 {cash.get('资产')}")
    print(f"{'代码':<8}{'名称':<10}{'数量':>6}{'成本':>9}{'券商价':>9}{'收盘':>9}{'差额':>8}{'市值(收盘)':>12}")
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
            f"{row['code']:<8}{row['name']:<10}{row['quantity']:>6}{row['cost_price']:>9.3f}"
            f"{row['broker_price']:>9.3f}{close:>9.3f}{diff:>+8.3f}{row['quantity']*close:>12.2f}"
        )
        if abs(diff) > 0.005:
            warn.append(
                f"{row['code']} {row['name']} 券商价 {row['broker_price']:.2f} vs 收盘 {close:.2f}"
            )

    market = round(market, 2)
    total = round(market + cash["可用"], 2)
    ratio = round(market / total * 100, 3) if total else 0.0
    print(f"\n收盘口径：市值 {market:.2f}｜资产 {total:.2f}｜仓位 {ratio:.2f}%")
    if cash.get("资产") is not None:
        print(f"券商口径：市值 {cash.get('参考市值')}｜资产 {cash['资产']}｜差额 {total - cash['资产']:+.2f}")
    for line in warn:
        print(f"  ⚠ {line}")

    if not args.apply:
        print("\ndry-run：未写库")
        return 0

    now = dt.datetime.now().astimezone().replace(microsecond=0).isoformat()
    snap = {
        "schema_version": 1,
        "effective_at": now,
        "confirm_empty": False,
        "items": [
            {"code": r["code"], "name": r["name"], "quantity": r["quantity"], "cost_price": r["cost_price"]}
            for r in rows
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
