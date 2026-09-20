# Push Template Catalog

> **历史快照提示（2026-09-02）：** 本文固定在 `master@97f28b9` 的 57-kind 审计，不再是当前清单。当前 65-kind、时段分类、producer/authority/completion 状态及问题证据以 [项目架构蓝图 §24](../Project_Architecture_Blueprint.md#24-推送系统专项架构与演进路线) 为准；本文保留作方法与历史追溯。

> **状态：** 历史代码索引 + 生产接线审计；当前事实已由蓝图 §24 上位替代
>
> **核验基线：** `master@97f28b9`（2026-07-22 更新前 HEAD）
>
> **主要来源：** `notify.rs`、`push_templates.rs`、`main.rs`、`v17_sources.rs`、`v14_adapter.rs`、`push_l7/`、`event/`

## 0. 文档定位与可信边界

本文列出当前 57 个 `PushKind`，说明元数据、治理、投递和审计链路，并记录已确认的生产接线缺口。

本文不把“枚举存在”“有 renderer”“测试通过”当作生产可用。生产可用至少需要真实 producer、生产 caller、来源合同、治理入口和投递证据。

### 0.1 术语

| 术语 | 含义 |
| --- | --- |
| 枚举存在 | `PushKind` 中有该 variant |
| Render ready | 有真实字段驱动的 renderer |
| Dispatch ready | 有进入 governor 的 wrapper/dispatcher |
| Producer ready | 有生产事件源和非测试 caller |
| Live evidence | 有真实 attempted/denied/deduped/pushed 审计 |

`is_legacy_v17_5`、`is_low_priority_v17_6`、`is_active_spec_target_v17_7_v17_8` 都是代码标记，不是生产存活证明。

### 0.2 当前总览

- `PushKind` 总数：57。
- `push_templates.rs` 在核验基线为 14,256 行；行数是观测值，不是接口合同。
- 核心元数据由 6 个 match 方法提供，另有 `daily_report_sub_kind()` 和 `dispatch_row()`。
- `DISPATCH_TABLE` 只有 15 个审计快照项，不覆盖全部 57 个 variant。
- 生产调度分散在 `main.rs`、`push_templates.rs`、`v17_sources.rs` 和 `news_aggregator_init.rs`，不存在单一总路由文件。

## 1. 完整 PushKind 清单（57 个）

本节只证明枚举和元数据身份，不证明生产 producer 已接通。生产状态见 §3。

### 1.1 持仓类（5 个）

| PushKind | 来源版本 | 备注 |
| --- | --- | --- |
| `HoldingEvent` | 保留 | 涨跌停突变、炸板、风控、现金预警 |
| `HoldingPlan` | v12 §14.3 T-03/T-04 | 持仓操作建议，30min/票 |
| `T0Advice` | v12 §14.3 T-05/T-06 | 做 T 建议，30min/票 |
| `PaperTrade` | v12 §14.3 T-10 | 虚拟盘成交回报，5min/票 |
| `PaperReview` | v13 §14.3 A-01 | 虚拟仓复盘 |

### 1.2 账户/系统状态类（4 个）

| PushKind | 来源版本 | 备注 |
| --- | --- | --- |
| `AccountMode` | v12 §14.3 T-01 | 账户模式变更，无冷却 |
| `DataMode` | v12 §14.3 T-02 | 数据模式变更，无冷却 |
| `MarketActionAlert` | v15.3 D5.1 | 实盘异常/账户切换，60s |
| `ForbiddenOps` | v12 §14.3 T-09 | 禁止操作提示，60min/票 |

### 1.3 盘前/盘后类（8 个）

| PushKind | 来源版本 | 备注 |
| --- | --- | --- |
| `DailyReport` | 保留 | 盘前/盘后告警、复盘、概览 |
| `PreopenNewsHot` | v13 §14.1 P-01 | 盘前新闻热点，15min |
| `IntradayMarket` | v13 §14.2 I-01 | 盘中轮动总览，15min |
| `ReviewMarket` | v12 §14.2 R-02 | 盘面走向 |
| `ReviewLhb` | v12 §14.2 R-04 | 龙虎榜，盘后 21:00 |
| `ReviewSignal` | v12 §14.2 R-05 | 系统信号复盘 |
| `ReviewFailure` | v12 §14.2 R-06 | 失败样本归因 |
| `CloseCall` | v12 §14.3 T-12 | 尾盘决策，1次/日 |

### 1.4 候选/选股类（7 个）

| PushKind | 来源版本 | 备注 |
| --- | --- | --- |
| `StockPick` | v11-P0-5+ A10 | 选股推荐，移交候选台 |
| `CandidateBoard` | v11-P0-5++ | 候选筛选台卡片 |
| `CandidateTriggered` | v12 §14.3 T-07 | 候选触发，1次/票/日 |
| `CandidateInvalidated` | v14.3 F-12 T-08 | 候选失效 |
| `VirtualWatch` | legacy | 虚拟观察仓位 |
| `NewsRanked` | P2-News | 新闻 Ranker 候选卡片 |
| `NewsToIdea` | v13 §14.4 D-01 | 新闻驱动个股，20min/票 |

### 1.5 产业链类（2 个）

| PushKind | 来源版本 | 备注 |
| --- | --- | --- |
| `IndustryChain` | C4 | 产业链扫描，移交候选台 |
| `IndustryChainIntraday` | v13 §14.2 I-03 | 盘中涨停扩散，30min |

### 1.6 新闻/热点类（4 个）

| PushKind | 来源版本 | 备注 |
| --- | --- | --- |
| `NewsCatalyst` | v13 §14.2 I-02 | 新闻催化映射，10min |
| `Announcement` | 保留 | 公告告警 |
| `NewsFlashCritical` | v17.4 §5.1 / BR-137 | 高分新闻即时推，5min/事件 |
| `NewsFlashAggregated` | v17.4 §5.1 | 四时段聚合 Top3 |

### 1.7 因子/资金验证类（5 个）

| PushKind | 来源版本 | 备注 |
| --- | --- | --- |
| `FactorIC` | v17.6 | `DailyReport` 子段 |
| `SectorTier` | v4/v17.6 | `DailyReport` 子段 |
| `CapitalVerify` | v4/v17.6 | `DailyReport` 子段 |
| `WeeklySOP` | 低优先级 | 周度 SOP |
| `SectorAnomaly` | v13 §14.2 I-09 | 量价反向发现，10min |

### 1.8 板块/异动类（8 个）

| PushKind | 来源版本 | 备注 |
| --- | --- | --- |
| `AuctionVolume` | 低优先级 | 竞价量能 Top10 |
| `AuctionRepush` | legacy | 竞价重推；生产调用已删除 |
| `LimitBoards` | 低优先级 | 首板/二板/三板 Top10 |
| `SectorTop` | 低优先级 | 领涨板块 Top5 |
| `FundInflow` | 低优先级 | 主力净流入 Top10 |
| `TurnoverTop` | v14.5 G-05 T-13 | 盘中换手率 Top10，10min |
| `TomorrowWatch` | v12 §14.2 R-07 | 明日观察池 |
| `EventCalendar` | v12 §14.2 R-08 | 明日事件日历 |

### 1.9 业绩/政策类（5 个）

| PushKind | 来源版本 | 备注 |
| --- | --- | --- |
| `PolicyHit` | v15.3 D5.1 / BR-137 | 政策催化，86400s |
| `EarningsBeat` | v15.3 D5.1 / BR-137 | 业绩超预期，43200s |
| `EarningsMiss` | v15.3 D5.1 / BR-137 | 业绩低于预期，43200s |
| `AnalystUpgrade` | v15.3 D5.1 / BR-137 | 卖方评级上调，86400s |
| `CatalystReview` | v13 §14.3 A-10 | 盘后题材催化复盘 |

### 1.10 IPO 类（3 个）

| PushKind | 来源版本 | 当前事实 |
| --- | --- | --- |
| `IpoListingApproval` | v15.1 C1.2 | 只有枚举/元数据/SignalSource 映射 |
| `IpoProspectus` | v15.1 C1.2 | 只有枚举/元数据/SignalSource 映射 |
| `IpoCatalyst` | v15.1 C1.2 | 只有枚举/元数据/SignalSource 映射 |

### 1.11 大宗交易/盘后固定价格类（6 个）

| PushKind | 来源版本 | 当前事实 |
| --- | --- | --- |
| `PostFixedPriceOrder` | v13.1 §5.2 T-14 | 有周期 caller；真实事件源未注册 |
| `PostFixedPriceFill` | v13.1 §5.3 T-15 | 有周期 caller；真实事件源未注册 |
| `StPriceLimitChanged` | v13.1 §5.4 T-16 | 有生产批次组装和周期入口 |
| `EtfClosingCallAuction` | v13.1 §5.5 T-17 | 有 renderer/dispatcher；生产存活需单独核验 |
| `BlockTradeIntradayConfirm` | v13.1 §5.6 / BR-033 | 只有 dispatcher，无生产 caller |
| `BlockTradePriceRange` | v13.1 §5.7 / BR-034 | 只有 dispatcher，无生产 caller |

## 2. 元数据与 DispatchTable 实况

### 2.1 核心元数据方法

| 方法 | 返回类型 | 用途 |
| --- | --- | --- |
| `level(self)` | `PushLevel` | Info/Important/Emergency |
| `requires_banner(self)` | `bool` | 模板语义标记；当前不控制生产 governor |
| `cooldown_secs(self)` | `Option<u32>` | 冷却窗口 |
| `cooldown_scope(self)` | `CooldownScope` | L4 去重键语义 |
| `label(self)` | `&'static str` | 日志和 UI 中文标签 |
| `stable_template_id(self)` | `String` | 稳定模板 ID |

`daily_report_sub_kind()` 负责 `FactorIC/SectorTier/CapitalVerify` 子段；`dispatch_row()` 查询审计快照。

### 2.2 DISPATCH_TABLE

实际类型是：

```rust
pub const DISPATCH_TABLE: &[(PushKind, DispatchRow)] = &[...];
```

它包含 15 行：3 个 v17.6 low-priority、6 个 v17.7 active target、6 个 v17.8 active target。

它不是 57 项数组，也不是运行时唯一事实源。`level/cooldown_secs/cooldown_scope/label/stable_template_id` 的 match 方法仍提供实际元数据；表内值修改时必须同步。

### 2.3 CooldownScope

当前只有三种作用域：

| Scope | 语义 |
| --- | --- |
| `Global` | 按 kind 全局冷却 |
| `PerTicket` | 按 `(kind, code)` 冷却；缺 code 时 L4 不冷却 |
| `External` | 由外部状态机或专门层管理 |

涉及 cooldown、filter、limit 的修改必须先登记 `docs/business_rules.md`，并在 PR 中引用对应 BR，遵守 2.10。

## 3. 生产接线状态与缺口

### 3.1 已确认的生产来源路由

| 来源 | PushKind | 入口 |
| --- | --- | --- |
| 东方财富公告 | `Announcement` | `main.rs` → `route_announcement_batch` → BR-137 source fact |
| 政策 provider | `PolicyHit` | `main.rs` → `poll_policy_provider` → BR-137 source fact |
| 财务/一致预期 | `EarningsBeat/Miss` | `main.rs` → `poll_earnings_and_analyst` |
| 研报评级 | `AnalystUpgrade` | `main.rs` → `poll_earnings_and_analyst` |
| critical flash | `NewsFlashCritical` | `news_aggregator_init.rs` → BR-137 source fact |
| monitor event | `MarketActionAlert` | EventBus consumer → `handle_monitor_event` |
| 持仓周期监控 | `HoldingEvent/HoldingPlan/T0Advice` | `main.rs` 周期任务 |
| 复盘调度 | `ReviewMarket/Lhb/Signal` | `main.rs` + `review_batch.rs` + template dispatchers |
| 虚拟仓午盘快照 | `PaperReview` | `main.rs` → `dispatch_paper_review_noon` |

`v17_sources.rs` 不是所有 57 个 kind 的总路由。持仓、paper、review、交易新规和部分新闻入口分散在其他模块。

### 3.2 已确认的缺口

| PushKind/模块 | 状态 | 证据结论 |
| --- | --- | --- |
| 3 个 IPO kind | metadata-only | 无 renderer、producer、生产 caller |
| 2 个 BlockTrade kind | dispatcher-only | repo 内无生产 caller |
| T-14/T-15 | source-unavailable | 周期 caller 存在，但 `TradeEventSource` 无生产注册者 |
| `AuctionRepush` | orphan/legacy | 唯一生产调用已注释为删除 |
| `CandidateInvalidated` | wrapper/test-only | 有 wrapper 和 E2E 测试，未确认生产 caller |

`TradeEventSource` 未注册时必须显式失败，禁止用空事件列表伪装成功，遵守 BR-087、2.1 和 2.8。

### 3.3 不可作为生产证据的信号

- enum variant 存在。
- renderer 或 dispatcher 函数存在。
- `DISPATCH_TABLE` 有一行。
- `is_active_spec_target_*()` 返回 true。
- 单元测试或 `--test` E2E 命中。
- 注释写了“production”“active”或“monitor”。

生产状态需要多行调用链检索，并确认最终 caller 位于非测试运行路径。

## 4. 实际治理、去重与投递链路

### 4.1 普通消息路径

```text
producer
  → 可选：current_banner_for（需要 BannerCtx 渲染的 caller）
  → push_governor_v3
  → LaunchGate
  → current_governance_ctx（读取 LATEST_BANNER）
  → L5 governance
  → L4 reserve/dedup
  → L6 SinkRouter 或 push_wechat
  → SQLite L7 + delivery hash-chain
  → 全部成功后 commit dedup；否则 rollback
```

`requires_banner()` 当前没有生产调用，只是元数据和测试断言。它不能解释实际 banner gating。

### 4.2 BR-137 source-fact 路径

```text
真实 provider 事实
  → SourceFactEvidence 强类型校验
  → push_source_fact_v3
  → LaunchGate
  → current_source_fact_governance_ctx
  → L5 governance
  → L4 provider identity 去重
  → sink
  → SQLite L7 + delivery hash-chain
  → commit/rollback
```

白名单固定为 `Announcement/PolicyHit/EarningsBeat/EarningsMiss/AnalystUpgrade/NewsFlashCritical`。

这条路径直接读取真实 capability health，不依赖账户 Banner 是否就绪。它仍执行静默期、去重、真实 sink、L7 和不可篡改审计。

### 4.3 Governance 语义

- 非 Emergency 消息遵守 02:00–06:00 静默期。
- Frozen 不触发 L5 Deny；需要 BannerCtx 的模板可渲染警告，资金安全由 broker 下单层保证。
- 普通业务 profile 的最低数据模式通常是 Degraded；Unsafe/Down 会被 `data_quality` 拒绝。
- `DataMode` 对数据源 Down 事件有已登记豁免。
- BR-137 source fact 的最低数据模式为 Down，但不得扩大到普通业务、行情、持仓或交易消息。

L5 先判治理，L4 后做 reserve。去重窗口只有在 sink、SQLite L7 和 hash-chain 都成功后才提交；失败必须回滚以允许真实重试。

## 5. 审计边界

当前至少有四种不同的落盘对象，不能统称为 `event/jsonl_writer`。

### 5.1 Markdown 推送副本

`notify.rs::save_push_log(text)` 写入：

```text
data/push_log/YYYY-MM-DD/HHMMSS_<unique>.md
```

测试环境写 `data/test/push_log/`。文件采用不可覆盖创建并 `sync_data`；写入失败会阻断 sink，遵守 BR-113。

该函数只接收 `text`。Markdown 不保证结构化包含 PushKind、治理结果和投递通道。

### 5.2 SQLite L7 Analytics

`v14_record_delivery()` 把 template、治理、是否投递、实际 sink 和事件信息写入 `push_analytics` SQLite。

治理拒绝和 dedup 也会进入 L7，用于区分 `denied/deduped/pushed/sink error`。

### 5.3 Delivery hash-chain

sink 返回后，`event::publish_delivery()` 写入投递审计事件。BR-091 要求按年分片、哈希链接、刷新落盘和至少 5 年保留。

hash-chain 或 L7 失败时，即使外部 sink 已接受，也不得向 caller 返回 `Pushed`，遵守 BR-091、BR-113 和 2.7。

### 5.4 通用 EventEnvelope JSONL

`event/jsonl_writer.rs` 写 `{base_dir}/YYYY-MM-DD.jsonl`，内容是通用 `EventEnvelope`。

它不是 Markdown push log，也不是 SQLite L7。其 retention 配置不能替代 BR-091/2.7 的五年投递审计要求。

## 6. 与 v19.x 主设计的关系

### 6.1 PR-2 BannerSnapshot

PR-2 用 `Arc<RwLock<BannerSnapshot>>` 或 `ArcSwap` 替换 `Mutex<Option<BannerCtx>>`，并集中 mode、account、data、capability、breaker 和 freshness 状态。

它影响 `LATEST_BANNER/current_banner/current_governance_ctx` 及需要 `BannerCtx` 渲染的 producer，但不等于由 `requires_banner()` 自动分流。

### 6.2 PR-3 Structured Error

PR-3 用 `ErrorCode` 替换聚合困难的字符串错误，例如 `BannerUnavailable{retry_in_ms}` 和 `CircuitOpen{source,retry_in_ms}`。

### 6.3 PR-9 多层健康通知

PR-9 的三层通知是：本地 heartbeat 文件、可选本地 HTTP `/health`、配置后启用的 webhook。

它不是按 PushKind 分类的 Banner 闸门。当前 57 个 `PushKind` 中也没有 `HealthFail` variant。

### 6.4 PR-10 测试隔离

PR-10 要求 `--test` 强制 dry-run、Banner 显示测试模式，并把 push log 写入 `data/test/push_log/`。

这与 2.5 一致，但仍需验证真实账户、真实证券代码和测试环境物理隔离。

### 6.5 push_templates.rs 拆分

全面拆分 `push_templates.rs` 不属于 v19.x 当前范围。v19.x 只允许 PR-2 所需的局部 Banner 边界调整。

完整 L3 render 模块化应进入 v20+ backlog，并单独完成设计、旧模块关系、失败模式和回滚审查。

## 7. 统计

### 7.1 分类统计

| 分类 | 个数 |
| --- | ---: |
| 持仓 | 5 |
| 账户/系统状态 | 4 |
| 盘前/盘后 | 8 |
| 候选/选股 | 7 |
| 产业链 | 2 |
| 新闻/热点 | 4 |
| 因子/资金验证 | 5 |
| 板块/异动 | 8 |
| 业绩/政策 | 5 |
| IPO | 3 |
| 大宗/盘后固定价格 | 6 |
| **总计** | **57** |

### 7.2 代码状态标记

| 状态标记 | 个数 |
| --- | ---: |
| `is_deprecated` | 0 |
| `is_legacy_v17_5` | 4 |
| `is_low_priority_v17_6` | 3 |
| `is_active_spec_target_v17_7_v17_8` | 12 |

代码中的“v19.12 全保留”是历史实现标签，不等于 `docs/v19.x` 已批准了 v19.12 版本设计。

## 8. 证据与刷新方法

### 8.1 必查命令

```bash
wc -l src/bin/monitor/notify.rs \
  src/bin/monitor/push_templates.rs \
  src/bin/monitor/v14_adapter.rs \
  src/bin/monitor/v17_sources.rs

rg -n 'pub fn (level|requires_banner|cooldown_secs|cooldown_scope|label|stable_template_id)' \
  src/bin/monitor/notify.rs

rg -n -A 8 -B 8 'PushKind::<TARGET>|push_<target>|dispatch_<target>' \
  src/bin/monitor --glob '*.rs'

rg -n -A 8 -B 8 'register_trade_event_source|push_source_fact_v3|current_governance_ctx' \
  src --glob '*.rs'
```

多行调用必须使用 `-A/-B` 上下文核验。单行命中不能区分 enum、match、测试、注释和生产 caller。

### 8.2 刷新检查表

- 枚举集合与 §1 完全一致。
- 分类总数仍为 57。
- 元数据方法和 `CooldownScope` 与实现一致。
- `DISPATCH_TABLE.len()` 与文档一致。
- producer 状态同时核验来源注册和非测试 caller。
- source-fact 白名单与 BR-137 一致。
- 治理顺序和 dedup commit/rollback 与实现一致。
- Markdown、SQLite L7、hash-chain、EventEnvelope JSONL 不混写。
- v19 PR 编号和范围以主设计为准。

## 9. 来源映射

| 关注点 | 权威来源 |
| --- | --- |
| PushKind enum/元数据 | `src/bin/monitor/notify.rs` |
| renderer/dispatcher | `src/bin/monitor/push_templates.rs` |
| 常驻调度 | `src/bin/monitor/main.rs` |
| normalized source facts | `src/bin/monitor/v17_sources.rs` |
| critical flash | `src/bin/monitor/news_aggregator_init.rs` |
| L4/L5/L7 适配 | `src/bin/monitor/v14_adapter.rs` |
| L5 governance | `src/push_l5/governance.rs` |
| SQLite L7 | `src/push_l7/` |
| 通用 JSONL | `src/event/jsonl_writer.rs` |
| 业务规则 | `docs/business_rules.md` |
| v19 范围 | `docs/v19.x/v19.0-operational-clarity-design.md` |

本文是可验证索引，不替代代码、业务规则或 v19 主设计。任何“已生产接通”结论都必须附 producer、caller、治理和审计证据。
