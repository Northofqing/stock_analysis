#!/usr/bin/env python3
"""把 income / cash_flow 财报落到 financial_statement 表（grpcurl 直连本机桥）。

为什么用 grpcurl：monitor 侧 `holding_earnings::sync` 走 CompanyDataGateway，
需要 DatabaseManager 初始化（BR-159 审计）；离线补数用同一份 canonical batch 直连更快。

用法：
    python3 scripts/sync_financial_statements.py --db <path> [--codes 002131,600396] [--kinds income,cash_flow]

覆盖范围默认 = 最新持仓快照 ∪ 虚拟仓位（paper_ledger_head 的 lots）。
幂等：PK(code, kind, report_period) + UPSERT。
"""

from __future__ import annotations

import argparse
import base64
import json
import sqlite3
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

PROTO_DIR = ".worktrees/tdx-local-provider-20260923/client-bundle"
BRIDGE = "127.0.0.1:18082"
METHOD = "magic.market.v1.MarketDataService/FinancialStatements"
KIND_MAP = {"income": "Income", "cash_flow": "CashFlow", "balance": "Balance"}
WIRE_KIND = {"Income": "income", "CashFlow": "cash_flow", "Balance": "balance"}

DDL = """
CREATE TABLE IF NOT EXISTS financial_statement (
  code TEXT NOT NULL,
  kind TEXT NOT NULL,
  report_period TEXT NOT NULL,
  announced_on TEXT,
  lines_json TEXT NOT NULL,
  provider TEXT,
  batch_id TEXT,
  captured_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (code, kind, report_period)
);
"""


def fetch(code: str, kind: str) -> tuple[list[dict], str | None]:
    params = json.dumps({"codes": [code], "kind": WIRE_KIND.get(kind, kind.lower())}, separators=(",", ":")).encode()
    body = {
        "context": {"protocolVersion": 1, "requestId": f"fs-{code}-{kind}"},
        "payload": {
            "schema": "market.financial_statements",
            "schemaVersion": 1,
            "contentType": "application/json; charset=utf-8",
            "data": base64.b64encode(params).decode(),
        },
    }
    proc = subprocess.run(
        ["grpcurl", "-plaintext", "-import-path", PROTO_DIR, "-proto", "market.proto",
         "-d", json.dumps(body), BRIDGE, METHOD],
        capture_output=True, text=True, timeout=90,
    )
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip().splitlines()[-1][:200])
    response = json.loads(proc.stdout)
    records: list[dict] = []
    for chunk in response.get("records", []):
        raw = base64.b64decode(chunk.get("data", "")).decode("utf-8", "replace")
        parsed = json.loads(raw) if raw.strip() else []
        if isinstance(parsed, dict):
            parsed = [parsed]
        records.extend(parsed)
    return records, response.get("batchId")


def covered_codes(conn: sqlite3.Connection) -> list[str]:
    codes = {row[0] for row in conn.execute(
        """SELECT item.code FROM user_position_snapshot_item item
           JOIN user_position_snapshot snap ON snap.snapshot_id = item.snapshot_id
           WHERE snap.snapshot_id = (SELECT snapshot_id FROM user_position_snapshot ORDER BY effective_at DESC LIMIT 1)"""
    )}
    try:
        row = conn.execute("SELECT projection_bytes FROM paper_ledger_head ORDER BY rowid DESC LIMIT 1").fetchone()
        if row:
            for lot in json.loads(row[0]).get("lots", []):
                if lot.get("code"):
                    codes.add(lot["code"])
    except Exception:  # noqa: BLE001
        pass
    return sorted(codes)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", required=True)
    ap.add_argument("--codes")
    ap.add_argument("--kinds", default="income,cash_flow")
    ap.add_argument("--workers", type=int, default=4)
    args = ap.parse_args()

    conn = sqlite3.connect(args.db, timeout=120)
    conn.execute("PRAGMA busy_timeout=120000")
    conn.executescript(DDL)
    conn.commit()

    codes = [c.strip() for c in args.codes.split(",")] if args.codes else covered_codes(conn)
    kinds = [KIND_MAP[k.strip()] for k in args.kinds.split(",") if k.strip() in KIND_MAP]
    print(f"[财报] codes={len(codes)} kinds={kinds}")

    written = errors = 0
    rows: list[tuple] = []
    with ThreadPoolExecutor(max_workers=args.workers) as pool:
        futures = {(code, kind): pool.submit(fetch, code, kind) for code in codes for kind in kinds}
        for (code, kind), future in futures.items():
            try:
                records, batch_id = future.result()
            except Exception as error:  # noqa: BLE001
                errors += 1
                print(f"[财报] {code}/{kind}: {error}", file=sys.stderr)
                continue
            for record in records:
                lines = {line["key"]: line.get("value") for line in record.get("lines", []) if line.get("value") is not None}
                rows.append((
                    code, kind, record["report_period"], record.get("announced_on"),
                    json.dumps(lines, ensure_ascii=False, separators=(",", ":")),
                    (record.get("evidence") or {}).get("provider"), batch_id,
                ))
    if rows:
        conn.executemany(
            """INSERT INTO financial_statement
               (code, kind, report_period, announced_on, lines_json, provider, batch_id)
               VALUES (?,?,?,?,?,?,?)
               ON CONFLICT(code, kind, report_period) DO UPDATE SET
                 announced_on=excluded.announced_on, lines_json=excluded.lines_json,
                 provider=excluded.provider, batch_id=excluded.batch_id,
                 captured_at=CURRENT_TIMESTAMP""",
            rows,
        )
        conn.commit()
        written = len(rows)
    print(f"[财报] 完成 rows={written} errors={errors}")
    return 0 if errors == 0 else 2


if __name__ == "__main__":
    sys.exit(main())
