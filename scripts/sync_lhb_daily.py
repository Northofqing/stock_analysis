#!/usr/bin/env python3
"""龙虎榜日更：直接取东方财富 datacenter LHB 明细，写入 lhb_daily。

背景（2026-10-26）：
- 原来的 lhb_daily 靠 monitor 从 durable_delivery 的 ReviewLhb 渲染文本解析，
  ReviewLhb 自 10-22 起没有新决策，且渲染模板改版导致解析 0 行 → 表停更。
- DragonTiger 走本地桥 RPC 实测挂死（>180s，grpc 13），请求到不了对端。
- 东方财富 datacenter 的 RPT_DAILYBILLBOARD_DETAILSNEW 从 Mac 直连可取（10-26 实测 18 页）。

口径：
- trade_date 统一 YYYY-MM-DD（历史脏值 '2026-07-06 00:00:00' 会被清洗）
- lhb_ratio = DEAL_AMOUNT_RATIO / 100（表内存的是比例而非百分数）
- 同一 (code, trade_date) 只留一行：先按日删除再整批写入，保证幂等且集合精确

用法：
    python3 scripts/sync_lhb_daily.py --db <db> --date 2026-10-26
    python3 scripts/sync_lhb_daily.py --db <db> --date 2026-10-26 --dry-run
"""
import argparse
import json
import sqlite3
import sys
import urllib.parse
import urllib.request

URL = "https://datacenter-web.eastmoney.com/api/data/v1/get"
REPORT = "RPT_DAILYBILLBOARD_DETAILSNEW"


def fetch_all(date: str) -> list[dict]:
    rows: list[dict] = []
    page = 1
    while True:
        query = urllib.parse.urlencode(
            {
                "reportName": REPORT,
                "columns": "ALL",
                "filter": f"(TRADE_DATE='{date}')",
                "pageNumber": page,
                "pageSize": 500,
                "source": "WEB",
                "client": "WEB",
            }
        )
        request = urllib.request.Request(f"{URL}?{query}", headers={"User-Agent": "Mozilla/5.0"})
        with urllib.request.urlopen(request, timeout=30) as response:
            payload = json.loads(response.read().decode("utf-8"))
        result = payload.get("result") or {}
        data = result.get("data") or []
        rows.extend(data)
        pages = result.get("pages") or 1
        if page >= pages or not data:
            break
        page += 1
    return rows


def normalize(raw: dict, date: str) -> tuple:
    code = (raw.get("SECURITY_CODE") or "").strip()
    name = (raw.get("SECURITY_NAME_ABBR") or "").strip()
    reason = (raw.get("EXPLANATION") or "").strip() or "未披露原因"
    close = float(raw.get("CLOSE_PRICE") or 0.0)
    pct = float(raw.get("CHANGE_RATE") or 0.0)
    buy = float(raw.get("BILLBOARD_BUY_AMT") or 0.0)
    sell = float(raw.get("BILLBOARD_SELL_AMT") or 0.0)
    total = float(raw.get("BILLBOARD_DEAL_AMT") or 0.0)
    ratio = float(raw.get("DEAL_AMOUNT_RATIO") or 0.0) / 100.0
    return (code, name, date, reason, pct, close, buy, sell, buy - sell, total, ratio)


def clean_dirty_dates(conn: sqlite3.Connection) -> int:
    dirty = conn.execute(
        "SELECT id, code, substr(trade_date,1,10) FROM lhb_daily WHERE trade_date LIKE '% %'"
    ).fetchall()
    cleaned = 0
    for row_id, code, day in dirty:
        exists = conn.execute(
            "SELECT 1 FROM lhb_daily WHERE code=? AND trade_date=? AND id<>?", (code, day, row_id)
        ).fetchone()
        if exists:
            conn.execute("DELETE FROM lhb_daily WHERE id=?", (row_id,))
        else:
            conn.execute("UPDATE lhb_daily SET trade_date=? WHERE id=?", (day, row_id))
        cleaned += 1
    return cleaned


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", required=True)
    ap.add_argument("--date", required=True)
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()
    date = args.date[:10]

    try:
        raw = fetch_all(date)
    except Exception as error:  # noqa: BLE001
        print(f"[lhb] 取数失败 {date}: {error}", file=sys.stderr)
        return 2
    # 只保留 A 股个股（0/3/6 开头六位），可转债/基金不进 lhb_daily。
    raw = [
        item
        for item in raw
        if len((item.get("SECURITY_CODE") or "").strip()) == 6
        and (item.get("SECURITY_CODE") or "").strip()[0] in "036"
    ]
    # 同一只票当日可能因多个原因重复上榜：按 code 合并，原因去重拼接，取第一条的数值。
    merged: dict[str, list] = {}
    for item in raw:
        row = normalize(item, date)
        if not row[0] or not row[1]:
            continue
        if row[0] in merged:
            reason = row[3]
            if reason not in merged[row[0]][3]:
                merged[row[0]][3] = f"{merged[row[0]][3]}；{reason}"
        else:
            merged[row[0]] = list(row)
    rows = [tuple(row) for row in merged.values()]
    print(f"[lhb] {date} 取到 {len(rows)} 只（原始 {len(raw)} 行）")
    if not rows:
        return 3
    for row in rows[:3]:
        print("  ", row[0], row[1], row[3][:24], f"净额={row[8]:.0f}")
    if args.dry_run:
        print("[lhb] dry-run，未写库")
        return 0

    conn = sqlite3.connect(args.db)
    conn.execute("PRAGMA busy_timeout=20000")
    try:
        with conn:
            cleaned = clean_dirty_dates(conn)
            conn.execute("DELETE FROM lhb_daily WHERE trade_date=?", (date,))
            conn.executemany(
                "INSERT INTO lhb_daily(code,name,trade_date,reason,pct_change,close_price,"
                "buy_amount,sell_amount,net_amount,total_amount,lhb_ratio) "
                "VALUES(?,?,?,?,?,?,?,?,?,?,?)",
                rows,
            )
        total = conn.execute("SELECT COUNT(*) FROM lhb_daily WHERE trade_date=?", (date,)).fetchone()[0]
        print(f"[lhb] 写入 {date} = {total} 行；清洗脏日期 {cleaned} 行")
    finally:
        conn.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
