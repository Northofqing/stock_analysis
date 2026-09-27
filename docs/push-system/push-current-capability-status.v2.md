# 当前源码推送状态增量 v2

状态：PROVISIONAL。源码快照：`402a7da74870a81fc58975628203923da4dc35f8`。

仅修订四个已核实 kind 的当前源码状态事实；不替代 65-kind v1 身份、不更改运行时 MachineCatalog、readiness 或 activation。source_commit 固定审计源码快照，未证明当前部署或外部接收。

历史 catalog SHA-256：`0aa6a2fd87ee9c235073cad3beef44229437f3fe62987b0db510ad36a93aace3`。

current v1 catalog SHA-256：`b3c04218e3548f80c026db905e3d0ac2eed59d7ce24efeefa8e69a20b417de93`。

沿用 v1 的其余状态后，65-kind 投影为 ACTIVE 37、INACTIVE 22、STARVED 4、OPT-IN 2。此处只描述固定源码快照，不更改运行时目录。

| kind | v1 状态 | 源码状态 | 新 claim 路径 | 结构化事实 | 说明与证据 |
| --- | --- | --- | --- | --- | --- |
| `IndustryChain` | STARVED | ACTIVE | CONDITIONAL | `scheduler_wired=true`、`account_metrics_complete_required=true`、`historical_decision_resume_independent=true` | R-03 在 account phase 内只于 banner.account_metrics_complete 为真时调用真实 counted dispatcher；缺指标时保持 typed AccountMetricsIncomplete。既存 immutable decision 的启动恢复是独立路径。ACTIVE 只表示条件生产调用可达，不表示已部署、源批次成功或 sink 已接收。<br>[r03-account-phase](#r03-account-phase)、[r03-banner](#r03-banner)、[r03-counted](#r03-counted) |
| `PostFixedPriceOrder` | STARVED | STARVED | BLOCKED | `scheduler_wired=true`、`source_registered=false` | T-14 盘中 900 秒调度接线存在，TradeEventSource 未在生产源码注册；读取边界失败，无新 order 事件可投递。保留 v1 的调度/Unit 状态语义。<br>[trade-schedule](#trade-schedule)、[trade-source-fetch](#trade-source-fetch)、[trade-source-register](#trade-source-register) |
| `PostFixedPriceFill` | STARVED | STARVED | BLOCKED | `scheduler_wired=true`、`source_registered=false`、`after_hours_schedule_reachable=false` | T-15 盘中 300 秒调度接线存在，TradeEventSource 未注册；15:00 后盘中分支退出，盘后成交语义窗口另受时段结构阻断。无新 fill 事件可投递。<br>[trade-schedule](#trade-schedule)、[trade-source-fetch](#trade-source-fetch)、[trade-source-register](#trade-source-register) |
| `BlockTradePriceRange` | INACTIVE | INACTIVE | BLOCKED | `side_route_wired=true`、`required_range_present=false` | T-19 北交所生产侧路由固定传 None 价格区间，dispatcher 在渲染和 sink 前返回 false；review.price 作为均价不能代替区间。<br>[block-review-route](#block-review-route)、[block-range-guard](#block-range-guard) |

## 源码证据

### r03-account-phase

`src/bin/monitor/push_templates.rs::dispatch_post_session_review`；symbol SHA-256 `8a369a27a849bcc52d68e129447e4348aa166eb432ca3b09db095bc403be86c3`。

### r03-banner

`src/bin/monitor/main.rs::build_banner`；symbol SHA-256 `147fc6881a61da7ad1d1b1763421eeed1ccee53bc8ea3d1d7a120fe4b083da7a`。

### r03-counted

`src/bin/monitor/push_templates.rs::dispatch_r03_industry_chain_outcome`；symbol SHA-256 `65c8cdda384711bcf3f0f61b78a7b6ed9dd6324ec00be06ea60476c3e30b8da8`。

### trade-schedule

`src/bin/monitor/main.rs::monitor_loop`；symbol SHA-256 `aaec1b5851fcfc6455b1f39864617c7515de2dce8069531c13fa510cc935b148`。

### trade-source-fetch

`src/bin/monitor/push_templates.rs::fetch_pending_trade_events`；symbol SHA-256 `374ceabfbe0ee4bf67e66b605aa094a6fff0251bb5b7f6ab4bb04f9e3b1c556b`。

### trade-source-register

`src/bin/monitor/push_templates.rs::register_trade_event_source`；symbol SHA-256 `b8c90bfef5380351e27bb5948a6d95053c4d9460e016492a8d210b03d24b38e1`。

### block-review-route

`src/bin/monitor/push_templates.rs::dispatch_block_trade_review`；symbol SHA-256 `ca8ef312363fac83b2209403c9ddf0a9857fa8b3f71f7ae9a80b7155e23478ea`。

### block-range-guard

`src/bin/monitor/push_templates.rs::dispatch_block_trade_price_range`；symbol SHA-256 `453e74335e301177207e5e3f74932d88d1a1d4b808b7f568f9ad3ba0d3791375`。

v2 中的 ACTIVE、STARVED、INACTIVE 是源码能力观察，不是 activation、真实批次、typed receipt 或生产送达证明。
