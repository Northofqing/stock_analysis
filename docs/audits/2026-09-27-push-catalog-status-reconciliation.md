# 推送目录状态差异审计（2026-09-27）

范围：逐项对照旧蓝图 [§24](../Project_Architecture_Blueprint.md#24-推送系统专项架构与演进路线) 的 65 个 `PushKind`、冻结的 [历史 catalog v1](../push-system/push-capability-catalog.v1.json)、[current catalog v1](../push-system/push-current-capability-catalog.v1.json)，并复核有差异的生产调用链。源码观察基于 `402a7da7`；这不是部署版本或实际送达审计。

## 精确差异

旧蓝图 §24 的 65 行与历史 catalog v1 的 65 个 kind 身份完全相同，只有下列 **4 个状态**不同。历史 catalog v1 与 current catalog v1 的 65 个状态完全相同。

| kind | 旧 §24 | 历史/current v1 | 当前源码可达性证据 |
| --- | --- | --- | --- |
| `IndustryChain`（R-03） | `INACTIVE` | `STARVED` | 旧 §24 所述账户门曾使新 claim 停在 `AccountMetricsIncomplete`，而冻结 catalog 另登记了既存 immutable decision 的启动恢复。当前 [post-session dispatcher](../../src/bin/monitor/push_templates.rs#L12292) 已在 `banner.account_metrics_complete` 为真时调用 [真实 R-03 dispatcher](../../src/bin/monitor/push_templates.rs#L14825)，其成功路径可进入 counted delivery。指标不完整时仍返回 typed failure；现有源码不能证明某次生产运行已实际投递。 |
| `PostFixedPriceOrder`（T-14） | `INACTIVE` | `STARVED` | [盘中分支](../../src/bin/monitor/main.rs#L10524) 内有 900 秒调度 [T-14](../../src/bin/monitor/main.rs#L11546)，但 `TRADE_EVENT_SOURCE` 的 [读取](../../src/bin/monitor/push_templates.rs#L5711)在未注册时直接失败。全 `src/` 的 `register_trade_event_source(` 精确调用仅有定义，没有生产注册；当前无新事件进入 sink。 |
| `PostFixedPriceFill`（T-15） | `INACTIVE` | `STARVED` | 同一 [盘中分支](../../src/bin/monitor/main.rs#L10524)有 300 秒 [T-15 调度](../../src/bin/monitor/main.rs#L11562)，但共用未注册 source；[15:00 后分支退出](../../src/bin/monitor/main.rs#L11709)，盘后语义窗口还受时间结构阻断。旧 §24 的 `INACTIVE` 是严格的新发送可达性判断；v1 的 `STARVED` 保留了已接线的调度/Unit 身份。 |
| `BlockTradePriceRange`（T-19） | `ACTIVE` | `INACTIVE` | 唯一生产侧路由对北交所记录 [固定传 `None` 区间](../../src/bin/monitor/push_templates.rs#L10285)；[dispatcher 前置校验](../../src/bin/monitor/push_templates.rs#L7389)在渲染与 sink 前返回 `false`。`review.price` 被用作均价，不能补出缺失的价格区间。 |

旧 §24 汇总为 `ACTIVE 37 / INACTIVE 24 / STARVED 2 / OPT-IN 2`；两个 v1 catalog 及 [MachineCatalog 固定计数](../../src/monitor/push_job/catalog.rs#L502) 为 `36 / 22 / 5 / 2`。这四行恰好解释全部计数差额。旧 §24 的行号证据和状态语义保留为历史观察，不应直接覆盖冻结目录。

## 冻结目录之后的源码漂移

`IndustryChain` 另有一项 **catalog 与当前源码** 的差异。current catalog 的 [R-03 行](../push-system/push-current-capability-catalog.v1.json)仍写“新 claim 由账户门阻断 provider/renderer/sink”，但生产代码现已允许指标完整时进入真实 dispatcher。`account_metrics_complete` 来自 [banner 构造](../../src/bin/monitor/main.rs#L1711)的 `PortfolioMetrics::is_complete()`：当日有效账户摘要、当日盈亏、纸面账本连续止损计数和仓位均可形成完整指标；[缺任一项时保守失败](../../src/bin/monitor/main.rs#L2536)。[定向源码测试](../../src/bin/monitor/push_templates.rs#L14572)确认 R-03 只在 account phase 调用。因此按旧 §24 的“composition root 有 producer”定义，当前源码属于**条件可达的 `ACTIVE`**，但仍需生产事实证明当天输入、上游批次和 sink 可用。不能把 catalog `STARVED` 当作自动启用迁移的许可，也不能把测试调用当作生产投递。

T-14/T-15 的 `STARVED` 与旧 §24 `INACTIVE` 主要是**状态词口径冲突**：两者都观察到生产无新发送，前者保留已接线调度/Unit，后者强调缺 source 后无法进入业务发送链。若仍用单一状态字段，必须先明确它表达“调度已接线”还是“当前可产生新发送”，否则改名会掩盖相同的阻断事实。

## 版本化调和及下一步门禁

1. 保留历史 `push-capability-catalog.v1.json`、对应 RFC 摘要、旧蓝图 §24 和当前运行时 v1 的原字节。运行时 [MachineCatalog](../../src/monitor/push_job/catalog.rs#L15)按 SHA-256、65/102/52 数量及状态计数加载 v1；[current audit checker](../../scripts/architecture-docs/current_audit.rb#L125)还强制 current v1 与历史 v1 的 `status`、`producer_ids` 等字段相同。原地修改任何 v1 状态会破坏这两道固定门。
2. 最小可执行目录变更：新建版本化的 **current-source v2**（或以历史 SHA 和 current v1 SHA 绑定的版本化 delta），只更新 R-03 的当前源码状态及说明；记录 T-14/T-15 的 `scheduler_wired=true`、`source_registered=false`，T-15 的 `after_hours_schedule_reachable=false`，以及 T-19 的 `required_range_present=false`。不要把这些事实折成一个“已发送”状态。若沿用 v1 的 STARVED 口径，v2 的源码状态计数应为 `ACTIVE 37 / INACTIVE 22 / STARVED 4 / OPT-IN 2`；这是**待验证提案**，不是当前机器目录的事实。保留既存 decision 恢复与新 claim 可达性的独立字段。
3. 为 v2 调整 current audit 校验：继续 exact-match 65 kind、102 producer、52 Unit 的稳定身份和 owner；允许有版本记录、源码 pin 和证据的当前状态差异。更新生成的 current Markdown 与蓝图 current 投影，明确旧 §24 是历史基线。不要从状态更新自动改变 activation、readiness 或源注册。
4. 验证门禁：静态差分必须只含获证实的状态/说明变化；运行 `ruby scripts/architecture-docs/check-catalog.rb --root . --check`、`ruby scripts/architecture-docs/render-catalog.rb --root . --current --check`、`ruby scripts/architecture-docs/check.rb --check --root .` 和相关 Ruby fixture；若更改 `MachineCatalog` 消费版本或 readiness，另运行 `cargo test --locked --offline --lib w06_` 及受影响的 readiness/activation 定向测试。R-03 晋级仍需有效完整指标、真实源批次、durable 决策与 sink 回执的运行证据；T-14/T-15 需真实 source 注册及盘后时窗验收；T-19 需有来源证明的价格区间，方可声称有新发送能力。

本审计只做文档证据整理；没有修改冻结 catalog、运行时代码、激活状态或生产部署。
