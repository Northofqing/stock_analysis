# M0 B06：缺失来源的通知裁决（2026-09-28）

状态：源码/产品边界裁决；生产状态与物理投递另行对账。

| 项 | 当前源码事实 | 本轮裁决 | 再开启前置与验收 |
| --- | --- | --- | --- |
| T-14 `PostFixedPriceOrder` | `main.rs` 仍每 15 分钟尝试订单 dispatcher；`push_templates.rs::fetch_pending_trade_events` 依赖全局 `TradeEventSource`，全仓没有生产调用 `register_trade_event_source`。当前无可证明的真实券商订单回报。 | 保持 `INACTIVE`，禁止通过测试桩、模拟 fill 或人工确认伪造订单已接受。目录身份保留作历史索引；定时入口应在后续清理任务中按显式 disabled 状态静默跳过并留下可查询原因。 | 若将来产品需要**模拟盘订单**通知，先定义新事件语义、PaperLedger receipt、PushKind/Unit 身份和唯一 completion owner，再做同事实 shadow；不能复用真实券商语义。真实券商接线不在本计划。 |
| T-15 `PostFixedPriceFill` | 与 T-14 共用未注册来源；当前调用嵌在盘中分支，15:05–15:30 盘后窗口不可达。 | 保持 `INACTIVE`，不注册虚构 fill 来源，也不为可达性单独放宽调度。 | 模拟成交提示须以已持久化的 paper fill 为事实，含 partial/no-fill、取消、T+1、单 owner 和真实 sink 回执；采用新版本语义，而非宣称券商成交。 |
| T-19 `BlockTradePriceRange` | `dispatch_block_trade_price_range` 在 `block_price_range=None` 时拒绝；当前大宗交易调用固定传入 `None`，所以即使北交所 review 有价格也不会发送此 kind。 | 保持 `STARVED`；成交价或前收盘价不能充当权威区间。 | 需有可验证的北交所当日价格区间/均价来源合同、时间与批次证据，以及坏值/缺值拒绝样本；随后做定向测试、shadow 与真实回执。 |

此裁决沿用 [完整路线图](../superpowers/plans/2026-09-28-platform-complete-roadmap.md) 的“只做模拟盘与人工决策支持，不接券商”边界。它不将存在 dispatcher、模板、kind 或测试桩解释为 active producer。下一步由 M1 catalog delta 明确这三个 kind 的 source、owner、调度及 operator 可见状态；不能为了达成 52 Unit 数字而自动开启。
