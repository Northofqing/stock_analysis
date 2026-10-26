#!/usr/bin/env python3
"""修复 paper 账本 fail-closed：user_position_snapshot 被加列导致 source 事实命名空间漂移。

背景（2026-10-26 定位）：
- paper 账本每次写入都先经过 paper_snapshot_activation_schema_v1::verify_source_catalog_on，
  它把 paper_trades / order_audit / order_audit_chain / user_account_summary /
  user_position_snapshot / user_position_snapshot_item 的 DDL 与 2026-10-09 激活时的
  frozen fixture（fixtures/global_schema_legacy_ddl_v1.tsv）逐字节比较。
- 生产库的 user_position_snapshot 被外部 ALTER 加了 6 列（cash_balance / cash_available /
  cash_withdrawable / market_value / total_assets / total_pnl），没有任何代码读它们，
  但足以让上述比较失败 → 账本从 10-21 起拒绝所有 mark。
- 修法：把该表按 frozen DDL 重建（保留 9 个原有列与全部行），6 列数据先导出 CSV 留证。

用法：
    python3 scripts/repair_snapshot_ddl_20261026.py --db <path>            # dry-run，只报告
    python3 scripts/repair_snapshot_ddl_20261026.py --db <path> --apply    # 执行重建
"""
import argparse
import csv
import os
import sqlite3
import sys

FROZEN_DDL = (
    "CREATE TABLE user_position_snapshot (id INTEGER PRIMARY KEY AUTOINCREMENT, "
    "snapshot_id TEXT NOT NULL UNIQUE, effective_at TEXT NOT NULL, confirmed_at TEXT NOT NULL, "
    "source TEXT NOT NULL, confirm_empty INTEGER NOT NULL CHECK(confirm_empty IN (0,1)), "
    "evidence_sha256 TEXT NOT NULL UNIQUE, item_count INTEGER NOT NULL CHECK(item_count >= 0), "
    "recorded_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP)"
)
TRIGGERS = [
    "CREATE TRIGGER user_position_snapshot_no_update BEFORE UPDATE ON user_position_snapshot "
    "BEGIN SELECT RAISE(ABORT, 'user_position_snapshot is append-only'); END",
    "CREATE TRIGGER user_position_snapshot_no_delete BEFORE DELETE ON user_position_snapshot "
    "BEGIN SELECT RAISE(ABORT, 'user_position_snapshot is append-only'); END",
]
KEEP = ["id", "snapshot_id", "effective_at", "confirmed_at", "source", "confirm_empty",
        "evidence_sha256", "item_count", "recorded_at"]
EXTRA = ["cash_balance", "cash_available", "cash_withdrawable", "market_value",
         "total_assets", "total_pnl"]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", required=True)
    ap.add_argument("--apply", action="store_true")
    ap.add_argument("--backup", default="")
    args = ap.parse_args()

    con = sqlite3.connect(args.db)
    con.execute("PRAGMA busy_timeout=20000")
    cols = [r[1] for r in con.execute("PRAGMA table_info(user_position_snapshot)")]
    ddl = con.execute(
        "SELECT sql FROM sqlite_master WHERE name='user_position_snapshot'").fetchone()[0]
    print(f"columns={cols}")
    if cols == KEEP:
        print("已是 frozen DDL，无需修复")
        return 0
    missing = [c for c in KEEP if c not in cols]
    if missing:
        print(f"拒绝执行：缺少必需列 {missing}", file=sys.stderr)
        return 2
    present_extra = [c for c in EXTRA if c in cols]
    print(f"多余列（无代码读取）：{present_extra}")
    con.close()

    backup = args.backup or (args.db + ".snapshot_extra_cols.csv")
    con = sqlite3.connect(f"file:{args.db}?mode=ro", uri=True)
    rows = con.execute(
        f"SELECT id,{','.join(present_extra)} FROM user_position_snapshot ORDER BY id").fetchall()
    with open(backup, "w", newline="") as fh:
        writer = csv.writer(fh)
        writer.writerow(["id"] + present_extra)
        writer.writerows(rows)
    n_rows = con.execute("SELECT COUNT(*) FROM user_position_snapshot").fetchone()[0]
    con.close()
    print(f"已导出 {len(rows)} 行到 {backup}；表内 {n_rows} 行")

    if not args.apply:
        print("dry-run：未改库")
        return 0

    con = sqlite3.connect(args.db)
    con.execute("PRAGMA busy_timeout=20000")
    con.execute("PRAGMA foreign_keys=OFF")
    try:
        con.execute("BEGIN IMMEDIATE")
        con.execute("CREATE TEMP TABLE snap_backup AS SELECT * FROM user_position_snapshot")
        con.execute("DROP TABLE user_position_snapshot")
        con.execute(FROZEN_DDL)
        con.execute(
            "INSERT INTO user_position_snapshot "
            f"({','.join(KEEP)}) SELECT {','.join(KEEP)} FROM snap_backup ORDER BY id")
        con.execute("DROP TABLE snap_backup")
        for trig in TRIGGERS:
            con.execute(trig)
        con.execute("COMMIT")
    except Exception:
        con.execute("ROLLBACK")
        raise
    after = con.execute("SELECT COUNT(*), MAX(id) FROM user_position_snapshot").fetchone()
    con.close()
    print(f"完成：rows={after[0]} max_id={after[1]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
