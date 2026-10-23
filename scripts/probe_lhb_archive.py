#!/usr/bin/env python3
"""ReviewLhb 投递归档探针（方案 2.6 前置，只读）。

问题：LHB 数据在 `delivery_decisions.envelope_canonical` 里，不在任何库表。
「谁在买（营业部席位）」这一维能不能拿到，必须先解码看正文，不能靠猜。

用法：
    python3 scripts/probe_lhb_archive.py [--days 30] [--show 3]

输出：
    1. ReviewLhb 投递计数与时间跨度
    2. 正文里是否出现「席位 / 营业部 / 净买 / 买入额 / 卖出额」等关键词
    3. 若干条正文样例（供人读，判断能否结构化）

边界：只读；不推断原文里没有的数字。
"""

import argparse
import json
import os
import sqlite3
import sys

RUNTIME = os.path.expanduser("~/.local/share/stock-analysis-runtime")
DELIVERY_DB = f"{RUNTIME}/data/durable_delivery.sqlite3"

KEYWORDS = ["营业部", "席位", "净买", "买入额", "卖出额", "上榜", "机构专用", "游资"]


def ro(path):
    return sqlite3.connect(f"file:{path}?mode=ro", uri=True)


def decode(content):
    if content is None:
        return ""
    # envelope_canonical 里的 rendered_content 是 **UTF-8 字节数组**（[240,159,...]），
    # 不是字符串。直接 str() 会得到一串数字（2026-10-23 探针第一版就踩了这个坑）。
    if isinstance(content, list) and all(isinstance(item, int) for item in content):
        return bytes(content).decode("utf-8", errors="replace")
    if isinstance(content, (bytes, bytearray)):
        try:
            return bytes(content).decode("utf-8")
        except UnicodeDecodeError:
            return bytes(content).decode("utf-8", errors="replace")
    if isinstance(content, str):
        return content
    return str(content)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--days", type=int, default=30)
    ap.add_argument("--show", type=int, default=3)
    args = ap.parse_args()

    if not os.path.exists(DELIVERY_DB):
        print(f"缺少投递库: {DELIVERY_DB}")
        return 2

    con = ro(DELIVERY_DB)
    rows = con.execute(
        """SELECT business_date, state, envelope_canonical
             FROM delivery_decisions
            WHERE push_kind = 'ReviewLhb'
              AND business_date >= date('now', ?)
            ORDER BY business_date DESC""",
        (f"-{args.days} day",),
    ).fetchall()

    print(f"=== ReviewLhb 投递（最近 {args.days} 天）===")
    print(f"决策行 {len(rows)} 条")
    if not rows:
        con.close()
        return 0

    states = {}
    for business_date, state, _ in rows:
        states[state] = states.get(state, 0) + 1
    print(f"state 分布: {states}")

    texts = []
    parse_fail = 0
    for business_date, state, raw in rows:
        if state != "Delivered":
            continue
        try:
            env = json.loads(decode(raw))
        except Exception:
            parse_fail += 1
            continue
        text = decode(env.get("rendered_content"))
        if text:
            texts.append((business_date, text))

    print(f"可解码正文 {len(texts)} 条；envelope 解析失败 {parse_fail} 条")

    hit_count = {key: 0 for key in KEYWORDS}
    for _, text in texts:
        for key in KEYWORDS:
            if key in text:
                hit_count[key] += 1
    print("\n=== 关键词命中（按正文条数）===")
    for key in KEYWORDS:
        print(f"  {key:<6} {hit_count[key]:>3} / {len(texts)}")

    if texts:
        print(f"\n=== 样例（{min(args.show, len(texts))} 条）===")
        for business_date, text in texts[: args.show]:
            print(f"\n--- {business_date}  ({len(text)} 字) ---")
            print(text[:1200])

    con.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
