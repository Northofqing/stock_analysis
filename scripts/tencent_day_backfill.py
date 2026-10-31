#!/usr/bin/env python3
"""腾讯 K 线兜底：把某交易日的全市场日线写进 stock_daily（仅当 TDX 盘后文件拿不到时用）。

背景（2026-10-30）：Windows VM 的通达信没有做「盘后数据下载」，.day 文件停在 10-29，
盘后同步按设计拒绝导入 → 当天日线缺失。腾讯 K 线接口从 Mac 可直连且返回真实 OHLCV。

口径与限制：
- 数据源标记 `tencent_kline_<date>`，与 tdx_day 区分；后续 TDX 同步会用 ON CONFLICT 覆盖。
- 腾讯日线只有 volume（手），没有 amount → amount 写 0，不做估算（不补造）。
- pct_chg 用库里前一日收盘算；前一日缺失则留 NULL。
- 并发 8，失败逐只重试一次。

用法：
    python3 scripts/tencent_day_backfill.py --db <db> --date 2026-10-30 [--limit N] [--dry-run]
"""
import argparse
import concurrent.futures
import json
import sqlite3
import sys
import urllib.request

URL = "https://web.ifzq.gtimg.cn/appstock/app/fqkline/get?param={sym},day,,,8,"
HEADERS = {"User-Agent": "Mozilla/5.0"}


def symbol(code: str) -> str:
    if code.startswith(("6", "9")):
        return f"sh{code}"
    if code.startswith(("0", "2", "3")):
        return f"sz{code}"
    return f"bj{code}"


def fetch(code: str, date: str):
    url = URL.format(sym=symbol(code))
    for attempt in range(2):
        try:
            raw = urllib.request.urlopen(
                urllib.request.Request(url, headers=HEADERS), timeout=15
            ).read()
            data = json.loads(raw)["data"][symbol(code)]
            key = next((k for k in data if "day" in k), None)
            if not key:
                return None
            for row in data[key]:
                if row[0] == date and len(row) >= 6:
                    return (
                        code,
                        float(row[1]),
                        float(row[3]),
                        float(row[4]),
                        float(row[2]),
                        float(row[5]),
                    )
            return None
        except Exception:
            if attempt == 1:
                return None
    return None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", required=True)
    ap.add_argument("--date", required=True)
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--workers", type=int, default=8)
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()
    date = args.date[:10]

    con = sqlite3.connect(f"file:{args.db}?mode=ro", uri=True)
    codes = [r[0] for r in con.execute(
        "SELECT DISTINCT code FROM stock_daily WHERE date < ? ORDER BY code", (date,))]
    prev = dict(con.execute(
        "SELECT code, close FROM stock_daily WHERE date=(SELECT MAX(date) FROM stock_daily WHERE date < ?)",
        (date,)))
    con.close()
    if args.limit:
        codes = codes[: args.limit]
    print(f"[tencent] 标的 {len(codes)} 只，目标 {date}")

    rows, missing = [], 0
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as pool:
        futures = {pool.submit(fetch, c, date): c for c in codes}
        for i, fut in enumerate(concurrent.futures.as_completed(futures), 1):
            r = fut.result()
            if not r:
                missing += 1
            else:
                code, o, h, l, c, v = r
                base = prev.get(code)
                pct = round((c / base - 1) * 100, 2) if base else None
                rows.append((code, date, o, h, l, c, v, pct))
            if i % 500 == 0:
                print(f"[tencent] {i}/{len(codes)} ok={len(rows)} miss={missing}")
    print(f"[tencent] 完成 ok={len(rows)} missing={missing}")
    if not rows:
        return 2
    if args.dry_run:
        for r in rows[:3]:
            print("  ", r)
        print("[tencent] dry-run，未写库")
        return 0

    con = sqlite3.connect(args.db)
    con.execute("PRAGMA busy_timeout=20000")
    with con:
        con.executemany(
            "INSERT INTO stock_daily(code,date,open,high,low,close,volume,amount,pct_chg,data_source,updated_at) "
            "VALUES(?,?,?,?,?,?,?,0,?,?,CURRENT_TIMESTAMP) "
            "ON CONFLICT(code,date) DO UPDATE SET open=excluded.open,high=excluded.high,low=excluded.low,"
            "close=excluded.close,volume=excluded.volume,pct_chg=excluded.pct_chg,"
            "data_source=excluded.data_source,updated_at=CURRENT_TIMESTAMP",
            [(c, d, o, h, l, cl, v, p, f"tencent_kline_{date}") for c, d, o, h, l, cl, v, p in rows],
        )
    n = con.execute("SELECT COUNT(*) FROM stock_daily WHERE date=?", (date,)).fetchone()[0]
    con.close()
    print(f"[tencent] 已写 {date} = {n} 行")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
