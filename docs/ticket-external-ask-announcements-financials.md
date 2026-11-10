# 外部依赖确认：公告按票查询 / 财报网关（2026-11-10）

> 目的：6.5（公告回链）与 6.6（结构化财务事实）都需要对端能力。以下是本机实测，
> 请桥接/数据侧确认可行性与时间。

## 1. 公告：MarketAnnouncements 目前不可按票、不可回看

实测命令（grpcurl 直连 `127.0.0.1:18082`，proto `client-bundle/market.proto`）：

```
MarketDataService/MarketAnnouncements
payload: {"start":"2026-11-06","end":"2026-11-06","limit":300}
```

返回：

```
batchId = cninfo:...:market-announcements:2026-11-10:2026-11-10:pages=4:total=1039
complete = true
records  = 1 个 chunk，解包后 100 条
```

观察：

1. 请求的 `start/end=2026-11-06` 被忽略，返回的是**当天 2026-11-10** 的公告；
2. `batchId` 说 `pages=4:total=1039`，但实际只给了 **100 条**（约 9.6%），
   `complete=true` 与实际覆盖不一致；
3. `Announcements` 方法同样落到这个 feed，传 `code`/`codes` 都不生效；
4. 记录字段齐全：`announcement_id / category / code / published_at / title / url`，
   `url` 全部存在（100/100）。

需要的能力（任一即可）：

- A. **按票查询**：入参 `code`（或 `codes`）+ 日期范围，返回该票公告；
- B. **分页**：按 `page/page_size` 或 `offset/limit` 翻页，能取完 `total`；
- C. 若短期做不到，明确告知“只能当天前 100 条”，我们就不把公告作为证据来源，
  继续在证据包里写 `missing: official_filing`。

验收口径：目标票（持仓 6 只 + 当日 P01 入选）近 5 个交易日公告命中率；
返回条目必须带 `url` 与 `published_at`。

## 2. 财报：FinancialStatements 网关当前不可用

实测：

```
MarketDataService/FinancialStatements
payload: {"codes":["001216"],"kind":"income"}
```

返回：

```
Code: Internal
Message: 取数失败: 财务报告 Gateway 不可用: CompanyFinancialStatements data gateway
         failed reason_code=observation_stale provider=Some(Sina) retryable=true:
         provider observation failed freshness gate age_ms=-1 max_ms=30000
```

`age_ms=-1` 表示没有拿到有效观测时间，不是“数据旧”，更像上游未返回。

需要确认：

1. 该网关是否还在维护/可用；
2. 可用时 `report_period / announced_on / lines` 的字段口径；
3. 单票单次调用的大致耗时与限频，避免 09:25 路径被拖慢。

在上述两项确认前，ticket 证据包保持：
`fundamentals.status=unavailable, reason=financial_statements_not_fetched`；
公告保持 `missing: official_filing`，不伪造来源。
