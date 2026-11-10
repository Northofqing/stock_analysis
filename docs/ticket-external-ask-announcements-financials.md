# 公告 / 财报：Windows 可用，问题在 Mac 桥路由（2026-11-10 修订）

> 前一版把 Mac 本地桥的实测误当成 Windows ExternalV1 的结论，已作废。
> 本版依据 operator 在 Windows 侧的直接实测（2026-11-10），以及 Mac 侧 grpcurl 对照。

## 结论

| 能力 | Windows ExternalV1 | Mac 本地桥 127.0.0.1:18082 | 归属 |
|---|---|---|---|
| 按票公告 `Announcements` | 可用：单 instrument + 日期范围，001216 返回 8 条 | 未按票路由，落到全市场 feed | Mac 桥改路由 |
| 全市场公告 `MarketAnnouncements` | 可用但有上限：`limit<=300`，无 page/offset | 忽略日期、只给 100 条、`complete=true` 错报 | Mac 桥修请求/字段 |
| 结构化财报 `FinancialStatements` | 可用：001216 8 期、77 lines/期、约 410ms | 报 `observation_stale age_ms=-1` | Mac 本地 gateway 修复 |
| `fiscal_period` | 恒为 null | 同 | 字段缺口，不伪造 Q1/H1 |

## Windows 实测（operator 提供）

公告窗口：

```
start=2026-11-06 end=2026-11-06 limit=300
selected_provider=Cninfo
pages=10 total=1104 limit=300
raw=300 unique=300 returned=300
complete=false            # limit 截断，不是全量
url/pdf_url 300/300
```

按票公告（001216，2026-10-01..2026-11-10）：

```
selected_provider=Cninfo  complete=true  records=8
published_at 范围 2026-10-26 .. 2026-11-10
canonical_url / pdf_url 8/8
```

请求形态：

```json
{
  "instrument": {"exchange":"Shenzhen","code":"001216","asset_class":"Equity"},
  "start":"2026-10-01",
  "end":"2026-11-10",
  "limit":100
}
```

6 只持仓（11-03..11-10）：001216 / 002131 / 600396 有记录；
002421 / 600667 / 600703 返回 `CNInfo returned no announcements`（合法空，
应记 0 命中，不能当 gateway failure）。

财报（001216）：`report_period` 2026-09-30/06-30/03-31…；`announced_on` 取
Sina publish_date；`lines` 77 条/期含 null，不补 0；`fiscal_period=null`。
6 只一次请求 48 条、约 1120ms，无显式限频；单次最多 10 个 instrument。

## Mac 桥需要改什么

1. `Announcements` 改为按票请求，路由到 Windows ExternalV1 的
   `Announcements`（instrument + start/end/limit），不要落全市场 feed；
   多票由 Mac 侧循环，单次一个 instrument。
2. 公告空结果单独归类为「0 命中」，不得报 gateway failure。
3. `MarketAnnouncements` 修正：日期窗口生效、不要虚报 `complete=true`；
   全市场分页（page/offset）属于合同变更，Gate A，暂不做。
4. `FinancialStatements` 修本地 gateway 的 observation 时间；按
   `report_period/announced_on/lines` 验收，`fiscal_period` 保持 null。
5. 财报不要在 09:25 逐票调用：盘后预取/缓存，09:25 只读缓存（6 只约 1.1s）。

## 验收

- 001216 近 5 个交易日公告 ≥1 条，且 `canonical_url` 非空；
- 002421 这类无公告票返回 0 条 + 明确「0 命中」，不是错误；
- 6 只持仓财报一次取回 48 条（8 期/只），字段完整；
- ticket 证据包 `selection.filings[]` 与 `fundamentals` 能引用上述批次 id。

Windows 侧这轮不需要改代码；A（按票）与财报接线在 Mac 侧，估 1 天以内。

## 追加（2026-11-10）：Mac 客户端 proto 快照缺 FinancialStatements

- Windows 数据面正常：grpcurl 直调 50052 的
  `MarketDataService/FinancialStatements`，schema
  `magic.market.financial_statements.request`，`kind=Balance/Income/CashFlow`，
  001216 单票 8 期、lines 含 null、Sina provider。
- monitor 的 ExternalV1 客户端返回 `Unimplemented`；原因是
  `src/grpc_client/external_pb.rs` 使用的 proto 快照没有该 RPC，不是对端拒绝。
- 需要：用 Windows 当前 `market.proto` 重新生成 external proto 快照（或书面确认
  该 RPC 已冻结）。完成后 Mac 侧 6.6 预取即可接线，Windows 无需改代码。
