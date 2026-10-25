#!/usr/bin/env python3
"""修复 2026-10-25 恢复后残留的库损坏（audit / chain / news N01 三张表 + 一个索引）。

做法：把三张表能读出的合法行导出到 gzip，DROP 掉四棵坏树，VACUUM 重建文件，
再让 monitor 启动时按代码 DDL 重建空表，最后把导出的行按主键顺序灌回去。
链式哈希按字符串原样保留，中间被跳过的坏行会断链，但尾链校验只看最后一行，
所以只要最后几行完整，追加就能恢复。

用法（必须在 monitor / 桥都停掉时跑）：
    python3 scripts/repair_audit_corruption_20261025.py --db <path> [--apply]

不带 --apply 只导出并打印统计。
"""

from __future__ import annotations

import argparse
import gzip
import json
import os
import sqlite3
import sys
from datetime import datetime

AUDIT_COLS = [
    "id", "schema_version", "capability", "provider", "source", "request_hash",
    "source_at", "observed_at", "batch_id", "outcome", "request_count",
    "accepted_count", "rejected_count", "reason_code", "retryable", "created_at",
]
CHAIN_COLS = ["acquisition_audit_id", "previous_hash", "record_hash", "created_at"]
NEWS_COLS = ["seq", "assessment_id", "evidence_json", "evidence_sha256", "previous_hash", "record_hash"]

DROP_SQL = [
    "DROP INDEX IF EXISTS idx_data_acquisition_audit_provider",
    "DROP TABLE IF EXISTS data_acquisition_audit_chain",
    "DROP TABLE IF EXISTS data_acquisition_audit",
    "DROP TABLE IF EXISTS news_ai_global_critical_v1",
]


def export_table(conn: sqlite3.Connection, out: str, cols: list[str], order: str, valid) -> tuple[int, int]:
    total = 0
    kept = 0
    with gzip.open(out, "wt", encoding="utf-8") as handle:
        cursor = conn.execute(f"SELECT {','.join(cols)} FROM {order} NOT INDEXED ORDER BY rowid")
        for row in cursor:
            total += 1
            if valid(row):
                kept += 1
                handle.write(json.dumps(row, ensure_ascii=False) + "\n")
    return total, kept


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--db", required=True)
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--out-dir", default="/Users/zhangzhen/Desktop/Quant/stock_analysis/data/audit/repair_20261025")
    args = parser.parse_args()

    os.makedirs(args.out_dir, exist_ok=True)
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")

    reader = sqlite3.connect(f"file:{args.db}?mode=ro", uri=True, timeout=30)
    reader.execute("PRAGMA query_only=ON")

    audit_out = f"{args.out_dir}/data_acquisition_audit_{stamp}.jsonl.gz"
    chain_out = f"{args.out_dir}/data_acquisition_audit_chain_{stamp}.jsonl.gz"
    news_out = f"{args.out_dir}/news_ai_global_critical_v1_{stamp}.jsonl.gz"

    def audit_valid(row) -> bool:
        return (
            isinstance(row[0], int)
            and isinstance(row[1], int)
            and all(isinstance(row[i], str) and row[i] for i in (2, 3, 4, 5, 7))
            and isinstance(row[10], int)
            and isinstance(row[11], int)
            and isinstance(row[12], int)
            and isinstance(row[13], str)
            and isinstance(row[14], int)
            and isinstance(row[15], str)
        )

    def chain_valid(row) -> bool:
        return isinstance(row[0], int) and len(row[1]) == 64 and len(row[2]) == 64

    def news_valid(row) -> bool:
        return isinstance(row[0], int) and len(row[1]) == 64 and len(row[3]) == 64 and len(row[5]) == 64

    audit_total, audit_kept = export_table(reader, audit_out, AUDIT_COLS, "data_acquisition_audit", audit_valid)
    chain_total, chain_kept = export_table(reader, chain_out, CHAIN_COLS, "data_acquisition_audit_chain", chain_valid)
    news_total, news_kept = export_table(reader, news_out, NEWS_COLS, "news_ai_global_critical_v1", news_valid)
    reader.close()

    print(f"[导出] audit {audit_kept}/{audit_total}  chain {chain_kept}/{chain_total}  news {news_kept}/{news_total}")
    print(f"[导出] 目录 {args.out_dir}")
    if not args.apply:
        print("[dry-run] 未改库")
        return 0

    writer = sqlite3.connect(args.db, timeout=60)
    writer.execute("PRAGMA busy_timeout=30000")
    for statement in DROP_SQL:
        writer.execute(statement)
    writer.commit()
    print("[修复] 已删除 4 个损坏对象")
    writer.execute("VACUUM")
    writer.commit()
    print("[修复] VACUUM 完成")
    writer.close()

    # 用 scripts/repair_audit_schema.sql 重建空表，然后灌回导出的行。
    schema = os.path.join(os.path.dirname(__file__), "repair_audit_schema.sql")
    writer = sqlite3.connect(args.db, timeout=60)
    writer.execute("PRAGMA busy_timeout=30000")
    writer.executescript(open(schema, encoding="utf-8").read())
    writer.commit()
    print("[修复] 空表已按代码 DDL 重建")

    def import_table(path: str, table: str, cols: list[str]) -> int:
        sql = f"INSERT INTO {table} ({','.join(cols)}) VALUES ({','.join('?' * len(cols))})"
        count = 0
        with gzip.open(path, "rt", encoding="utf-8") as handle:
            batch = []
            for line in handle:
                batch.append(json.loads(line))
                if len(batch) >= 2000:
                    writer.executemany(sql, batch)
                    count += len(batch)
                    batch.clear()
            if batch:
                writer.executemany(sql, batch)
                count += len(batch)
        writer.commit()
        return count

    imported_audit = import_table(audit_out, "data_acquisition_audit", AUDIT_COLS)
    imported_chain = import_table(chain_out, "data_acquisition_audit_chain", CHAIN_COLS)
    imported_news = import_table(news_out, "news_ai_global_critical_v1", NEWS_COLS)
    print(f"[修复] 灌回 audit={imported_audit} chain={imported_chain} news={imported_news}")
    writer.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    writer.close()
    print("[修复] 完成")
    return 0


if __name__ == "__main__":
    sys.exit(main())
