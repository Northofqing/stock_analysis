# 数据源缺口清单 — Windows 侧回复

日期：2026-10-18
对应：`2026-10-18-data-source-gaps.md`
结论：**7 项里 2 项已实现（D1/D2 走错 op），1 项接口已有但源不一致（D3），4 项没有。**

## 两条必须先纠正的事实

1. `src/bin/grpc_market_server/grpc_server/delegate.rs` **不在本仓**。本仓只有 `crates/`，
   全仓搜不到 `fetch_board_ranking` 和 `no_verified_batch`（已实测）。这个错误来自 Mac 侧
   LocalBridge 的 delegate，不是 Windows 服务端。本仓 `MarketRankings` 是另一个 op。
2. `MarketRankings` 在结构上做不了板块榜。它声明了 `Industry/Concept/Region`
   （`crates/magic-market-core/src/signals.rs:8-15`），但能力闸门只对
   `VolumeRatio` 和 `MainNetInflow` 放行，其余一律 `false`
   （`signals.rs:494-508`）。所以拿 fid=f62 去请求板块榜，即使到了服务端也会被拒。

## 逐项判定

| # | 判定 | 应该用的 op | 谁改 |
|---|---|---|---|
| D1 | 已实现 | `BoardFlows`（本仓 op 36） | Mac bridge 改映射 |
| D2 | 已实现 | `BoardFlows` | Mac bridge 改映射 |
| D3 | 已实现（Windows 侧） | `BoardMemberships` + `preferred_provider=Eastmoney` | Mac 切换 |
| D4 | 公开源没有 | — | 无 |
| D5 | 部分（年报 16 条），无 op | — | 待拍板 |
| D6 | 能力有、未暴露 | 无 op；TDX block bars 88xxxx | 需 Gate A |
| D7 | 无逐笔来源 | `CurrentAuctionObservations` 仅快照 | 无 |

## D1 / D2：BoardFlows 已经就是这两项

实现位置：`crates/magic-eastmoney-rs/src/board_flow.rs`

- 分类：Industry=`m:90+t:2`、Concept=`m:90+t:3`、Region=`m:90+t:1`（`:24-33`）
- Day1：`fid=f62`，fields=`f12,f14,f3,f62,f184,f204,f205,f206,f66,f72,f78,f84,f124`（`:34-38`）
- 记录：`board_code`、`board_name`、`category`、`interval`、`rank`、`return_ratio`(f3)、
  `main_net`(f62)、超大/大/中/小单、领涨股、evidence（`crates/magic-market-core/src/capital.rs:44-62`）
- 注册：`admitted(Operation::BoardFlows, "Eastmoney", "industry, concept or region board flows with an explicit interval and limit")`
  （`crates/magic-market-composition/src/grpc_production.rs:3152-3171`）

请求（schema `magic.market.board_flows.request`）：

```json
{"category":"Industry","interval":"Day1","limit":100}
```

- `category`：`Industry` / `Concept` / `Region`
- `interval`：`Day1` / `Day5` / `Day10`（其他值 typed Unsupported）
- `limit`：1..=200

一次调用就能同时拿到 f3 和 f62：`return_ratio` 是涨幅，`main_net` 是主力净流入。
不需要两次请求，也不需要 fid 参数。

验收：`BoardFlows(category=Industry, interval=Day1, limit=100)` 返回 100 条，
`main_net` 非空、`board_code` 为东财 BK 码。若 Mac bridge 的 40 op 表里没有
`BoardFlows`，按 RPC 名补映射即可——服务端已在 67 op 里注册。

## D3：Windows 侧已实现 Eastmoney 板块归属

`BoardMemberships`（op 29）新增第二个 provider `Eastmoney`：

- 请求加 `preferred_provider=Eastmoney`（空值仍走 Tdx，行为不变）。
- 返回 `eastmoney:industry:<名称>` / `eastmoney:concept:<名称>` / `eastmoney:region:<名称>`，
  `board_name` 是东财个股页的原始名称。
- 与 `BoardFlows` 的 `board_name` 是同一来源，精确 join，不再用新浪 175 概念或本地标签。
- 边界：1..=50 只沪深 A 股；北交所显式 Unsupported；无 `source_at`。
- 证据：`magic-market-data-rs/docs/integrations/eastmoney-board-memberships.md`（2 live + 3 serial）。
- `GetCapabilities` 现在会返回**两条** `BoardMemberships` 能力（provider=`Tdx` / `Eastmoney`），
  按 `provider` 字段区分，不能默认取第一条。

Mac 侧只需要：一次调用传 9 只票 + `preferred_provider=Eastmoney`，按 `board_name` 关联
`BoardFlows`。不需要再拉 175 个板块成分，也不需要名称模糊匹配。

部署（2026-10-19 更新）：已切换到 0.2.0 /
`56fba282dbb93f588ea73c0d201e56d090630d15` /
`dde8897c4559b7a057bd33a26f8fd1d85aef5c3dcfb86195be10115e6af27688`（不变）/
`1388a8612064bff1855eb53b2d250a67ee2e4d90e992f512a7a362a604b86e42`。
（2026-10-18 的 `e2319f5`/`bdcada24` 已被本次竞价修复取代。）
新 bundle `2026-10-18.1` 已交付，Mac 需替换 `external_v1_current` 并重钉；
线上已用 `preferred_provider=Eastmoney` 验证 600519/000338/600396 返回 `eastmoney:*` 记录。

## D4：产业链

公开源没有带名字的公司级上下游（已实测年报、同花顺、东财、TDX F10）。
详见证件 `magic-market-data-rs/docs/evidence/2026-10-18-related-company-yield/`。
这一项不是接口问题。

## D5：股权

年报口径已产出 16 条 A 股↔A 股边（2 持股、14 同实控人），覆盖 6/9 只票；
无 op，当前是 curated 修订。是否开 op 需单独拍板（契约变更 + Mac 重钉）。

## D6：板块指数日线

通达信 block 客户端已有 `get_block_bars(category, code 88xxxx, start, count)`
和 `get_block_quotes`（`crates/magic-tdx-rs/src/block/client.rs:198-247`），
但对外没有 op，且 `InstrumentId` 没有 Board 资产类别。要暴露需契约设计。
不是"做不到"，是"没做"。

## D7：交接为 L1 快照

Windows 侧按 L1 收口：使用 `CurrentAuctionObservations`（stage=live/final）提供竞价快照；
9:15–9:20 的逐步变化与 L2 逐笔不由 Windows 提供。Mac 如需更细粒度，需自接 L1/L2 行情源。

边界与字段语义见 `2026-10-18-auction-l1-handoff.md`（同目录）。
## 建议顺序

1. Mac bridge 把 D1/D2 切到 `BoardFlows`（服务端已有，零 Windows 改动）。
2. D3 用 `BoardMemberships` + `preferred_provider=Eastmoney`（已部署），不要再拉 175 个板块成分。
3. D4 停止在公开源里找；要密度只能走商业终端授权。
4. D5 保持 curated；D6 不立项；D7 按 L1 快照交接（见 `2026-10-18-auction-l1-handoff.md`）。
