#!/usr/bin/env python3
"""公司行动同步（7A 的一半）：本机桥 CorporateActions → corporate_action 表。

为什么用 grpcurl 而不是 SecurityLifecycleGateway：
    网关的 `audit_routed_gateway_result` 要求 `DatabaseManager` 已初始化（BR-159 审计），
    离线批处理里初始化一次要跑全库审计（分钟级到 20 分钟），不适合每晚批量。
    grpcurl 直连本机桥拿同一份 canonical batch，审计由桥自己记。

用法：
    python3 scripts/sync_corporate_actions.py --start 2021-08-01 --end 2026-10-23 \
        --db ~/.local/share/stock-analysis-runtime/data/stock_analysis.db [--limit N] [--workers 16]

幂等：PK(code, effective_on, category) + UPSERT；同日重跑不产生重复行。
边界：只覆盖 stock_daily 在册代码（已退市票不在 TDX vipdoc）。
"""

from __future__ import annotations

import argparse
import base64
import json
import sqlite3
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime

PROTO_DIR = ".worktrees/tdx-local-provider-20260923/client-bundle"
BRIDGE = "127.0.0.1:18082"
METHOD = "magic.market.v1.MarketDataService/CorporateActions"

DDL = """
CREATE TABLE IF NOT EXISTS corporate_action (
  code TEXT NOT NULL,
  effective_on TEXT NOT NULL,
  category TEXT NOT NULL,
  record_on TEXT,
  ex_on TEXT,
  payable_on TEXT,
  terms_json TEXT NOT NULL,
  batch_id TEXT,
  provider TEXT,
  captured_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (code, effective_on, category)
);
CREATE INDEX IF NOT EXISTS idx_corporate_action_date ON corporate_action(effective_on);
"""


def fetch(code: str, start: str, end: str) -> dict:
    params = json.dumps(
        {"code": code, "window_start": start, "window_end": end}, separators=(",", ":")
    ).encode()
    body = {
        "context": {"protocolVersion": 1, "requestId": f"ca-sync-{code}"},
        "payload": {
            "schema": "market.corporate_actions",
            "schemaVersion": 1,
            "contentType": "application/json; charset=utf-8",
            "data": base64.b64encode(params).decode(),
        },
    }
    proc = subprocess.run(
        [
            "grpcurl", "-plaintext",
            "-import-path", PROTO_DIR, "-proto", "market.proto",
            "-d", json.dumps(body), BRIDGE, METHOD,
        ],
        capture_output=True, text=True, timeout=60,
    )
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip()[:220] or "grpcurl failed")
    response = json.loads(proc.stdout)
    if not response.get("complete", False):
        raise RuntimeError("batch incomplete")
    records = []
    for record in response.get("records", []):
        raw = base64.b64decode(record.get("data", "")).decode("utf-8", "replace")
        parsed = json.loads(raw) if raw.strip() else []
        # 桥对单票返回的是数组；若将来改为单对象也兼容
        if isinstance(parsed, dict):
            parsed = [parsed]
        records.extend(parsed)
    return {"records": records, "batch_id": response.get("batchId"), "provider": response.get("selectedProvider")}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", required=True)
    ap.add_argument("--start", default="2021-08-01")
    ap.add_argument("--end", default="2026-10-23")
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--workers", type=int, default=16)
    args = ap.parse_args()

    conn = sqlite3.connect(args.db, timeout=120)
    conn.execute("PRAGMA busy_timeout=120000")
    for attempt in range(10):
        try:
            conn.executescript(DDL)
            conn.commit()
            break
        except sqlite3.OperationalError as error:
            if "locked" not in str(error).lower() or attempt == 9:
                raise
            import time as _time
            _time.sleep(5)

    codes = [row[0] for row in conn.execute("SELECT DISTINCT code FROM stock_daily ORDER BY code")]
    if args.limit:
        codes = codes[: args.limit]
    total = len(codes)
    print(f"[公司行动] codes={total} window={args.start}..{args.end} workers={args.workers}")

    started = datetime.now()
    done = errors = rows_written = 0
    pending: list[tuple] = []

    def flush() -> None:
        nonlocal rows_written
        if not pending:
            return
        conn.executemany(
            """INSERT INTO corporate_action
               (code, effective_on, category, record_on, ex_on, payable_on, terms_json, batch_id, provider)
               VALUES (?,?,?,?,?,?,?,?,?)
               ON CONFLICT(code, effective_on, category) DO UPDATE SET
                 record_on=excluded.record_on, ex_on=excluded.ex_on, payable_on=excluded.payable_on,
                 terms_json=excluded.terms_json, batch_id=excluded.batch_id,
                 provider=excluded.provider, captured_at=CURRENT_TIMESTAMP""",
            pending,
        )
        conn.commit()
        rows_written += len(pending)
        pending.clear()

    with ThreadPoolExecutor(max_workers=args.workers) as pool:
        futures = {pool.submit(fetch, code, args.start, args.end): code for code in codes}
        for future in as_completed(futures):
            code = futures[future]
            done += 1
            try:
                result = future.result()
            except Exception as error:  # noqa: BLE001
                errors += 1
                if errors <= 10:
                    print(f"[公司行动] {code}: {error}", file=sys.stderr)
                continue
            for record in result["records"]:
                terms = record.get("terms")
                pending.append(
                    (
                        record.get("code", code),
                        record.get("effective_on", ""),
                        record.get("category", "Unknown"),
                        record.get("record_on"),
                        record.get("ex_on"),
                        record.get("payable_on"),
                        json.dumps(terms, ensure_ascii=False, separators=(",", ":")),
                        result["batch_id"],
                        result["provider"],
                    )
                )
            if len(pending) >= 500:
                flush()
            if done % 500 == 0:
                elapsed = (datetime.now() - started).total_seconds()
                print(f"[公司行动] {done}/{total} rows={rows_written + len(pending)} errors={errors} elapsed={elapsed:.0f}s")
    flush()

    elapsed = (datetime.now() - started).total_seconds()
    print(f"[公司行动] 完成 codes={done} rows={rows_written} errors={errors} elapsed={elapsed:.1f}s")
    return 0 if errors == 0 else 2


if __name__ == "__main__":
    sys.exit(main())
