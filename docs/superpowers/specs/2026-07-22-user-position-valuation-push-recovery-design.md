# 用户完整持仓快照、收盘估值与推送恢复设计

**日期：** 2026-07-22
**状态：** 书面设计已于 2026-07-22 获用户批准；Gate A 计划编制中
**范围：** 生产异常止血、用户确认完整持仓、收盘估值、账户/数据状态消息
**不包含：** 券商账户接入、真实现金/总资产、实盘订单、用估值解除 Frozen

**规则基线：** AGENTS.md 2.1、2.2、2.3、2.4、2.7、2.8、2.10；BR-103、BR-108、BR-116、BR-130、BR-134、BR-135、BR-141、BR-142。涉及快照选择、原子切换、估值覆盖率和投递重试的新业务规则，必须在各实现工作流 Gate A 中读取当时最新规则表并分配连续 ID 后，才允许修改生产代码。

## 1. 目标

用三个边界清晰的工作流尽快解决当前用户可见异常：

1. 恢复 delivery audit 可追加性，阻止已物理送达消息因审计失败被重复发送，并修复 debug/test Tokio runtime panic。
2. 不接入真实券商账户，改由用户提交完整持仓快照；系统使用经过质量校验的真实日线收盘价生成“本地持仓收盘估值”。
3. 消息区分实时账户事实、本地持仓事实、估值时点和数据能力状态，不再把“有本地持仓但不能授权风控”渲染成“仓位缺失”。

最终系统必须同时做到：信息有用、缺失诚实、审计可追踪、估值不越权。

## 2. 当前问题与证据

### 2.1 投递异常

- 年度 delivery audit 存在合法哈希链的 legacy 记录，其 `payload.code` 与 `entity_key` 是字节精确空字符串。
- 当前 legacy 语义解析器把空字符串视为非法证券身份；全链校验因此在历史前缀失败。
- `AuditDispatcher` 首次失败后把进程内链状态置为 poisoned；重启只能再次命中同一历史记录。
- sink 已返回 `Pushed` 后才写 delivery audit。审计失败被包装成 `SinkError`，BR-116 保留到期状态并释放去重，可能重复物理投递。
- I-01 async dispatcher 在函数体内直接调用使用 `reqwest::blocking` 的同步数据加载器；debug/test 构建可在 Tokio 上下文析构内部 runtime 时 panic。

### 2.2 账户与估值异常

- 本地有持仓记录，但真实账户快照过期，日盈亏来源不可用。
- 完整账户指标还依赖未接通的 broker trade-sync watermark，因此当前 `ActionPortfolioMetrics` 无法完成。
- 失败后系统把所有指标压成 `None`，已经存在的本地持仓事实也被横幅渲染成“仓位缺失”。
- 用户明确选择暂不接入真实账户，要求后续持仓变更以用户提交的完整数据为准，并按每日收盘价计算具体持仓盈亏。

### 2.3 消息异常

- DataMode 启动时早于首次真实探测，未探测能力被当成 Missing 并立即推送 `未建立 → Unsafe`。
- 未探测、过期、失败和未接入没有区分；OrderBook 当前无真实生产源，却与可恢复能力使用相同文案。
- 数据状态模板只接收模式、缺失列表、限制和静态 ETA，没有最后成功时间、年龄、来源、错误和下次重试信息。

## 3. 安全边界

### 3.1 三种事实必须分开

| 事实 | 权威来源 | 用途 | 禁止用途 |
| --- | --- | --- | --- |
| 用户持仓事实 | 最新已确认完整用户快照 | 本地估值、用户消息 | 真实账户证明、现金/仓位比、下单授权 |
| 市场价格事实 | 通过 2.3/2.4 校验的真实未复权日线 | 收盘估值 | 缺失时补零、手工覆盖市场价格 |
| 行动账户事实 | 30 秒内完整真实账户批次 | AccountMode、风控、订单 | 本期不接入，不得由用户估值替代 |

`DisplayAccountFacts` 可以展示用户持仓版本、估值覆盖率和缺失原因。`ActionPortfolioMetrics` 继续 fail-closed；本期实现不得让前者隐式转换为后者。

### 3.2 Frozen 保持

- 用户持仓快照和收盘估值不能解除 Frozen。
- 没有真实现金和总资产时，不计算“仓位 N 成”。
- 没有真实账户当日成交、费用和资金变动时，不把收盘估值称为“券商日盈亏”。

### 3.3 审计不可改写

- 不删除、修改或重算现有 `data/event_audit/*.jsonl`。
- legacy 兼容只发生在只读验证视图中，哈希继续对原始字节对应的 JSON 值验证。
- v2 authoritative 写入、空白字符串、单边空值、身份不匹配和降级写入保持严格拒绝。

## 4. 工作流 A：投递审计与 Runtime 止血

### 4.1 Legacy 兼容

历史读取器仅接受以下 legacy 兼容形状：

```text
payload.code == "" AND envelope.entity_key == ""
```

它在完成原始记录哈希验证后，把这两个值在内存语义视图中规范化为“身份缺失”。以下情况仍拒绝：

- 只有一边为空；
- 任意空白字符串；
- 非空但不相等；
- v2 记录出现 code/entity_key；
- 当前生产路径尝试新写 legacy 记录。

### 4.2 启动预检与临时推送模式

monitor 在初始化 sink 前执行 delivery audit 可追加性预检：

- `Healthy`：允许进入既有治理。
- `Unavailable`：普通业务推送全部阻断；系统进入 `AuditDegraded`，不初始化可产生普通消息的调度。
- 风险消息只有在本次追加仍可取得完整权威审计时才允许发送；审计已知不可写时不得用“风险”名义绕过 2.7。

用户已经批准故障期间暂停普通业务推送。恢复条件是同一进程内重新完成全链验证和一次隔离 canary 追加，不是简单重启或等待时间。

### 4.3 已送达但审计失败

投递结果拆成至少三个独立事实：

```text
sink_outcome
l7_outcome
delivery_audit_outcome
```

sink 已确认 `Pushed` 后，即使 delivery audit 失败，也不得再次调用 sink。该次业务状态记为 `PhysicallyDeliveredAuditFailed`，触发全局 `AuditDegraded`，并保留 incident；它不是 `Pushed` 完成态，也不是可以重发 sink 的 `SinkError`。恢复任务只处理审计 incident，不伪造原投递的权威审计记录。

### 4.4 Runtime 边界

I-01 的同步市场加载全部进入 `tokio::task::spawn_blocking`，async 上下文只接收拥有所有权的结果。对 monitor async dispatcher 做同类扫描：任何 `reqwest::blocking`、同步 DB 长操作或自建 runtime 必须有明确 blocking 边界。

## 5. 工作流 B：用户完整持仓快照

### 5.1 数据模型

新增版本化事实，不直接把 `stock_position` 改造成快照表：

```text
UserPositionSnapshot
├── snapshot_id
├── effective_at
├── confirmed_at
├── source = user_confirmed
├── item_count
├── evidence_hash
└── status = active | superseded

UserPositionSnapshotItem
├── snapshot_id
├── code
├── name
├── quantity
└── cost_basis
```

`stock_position` 继续承载既有投影/交易历史语义。新估值路径只读取新的用户快照边界，避免把旧表更新时间冒充完整快照来源时间。

### 5.2 完整提交与原子切换

每次提交都是完整集合：

1. 在独立事务中验证 snapshot 和全部 item。
2. 代码必须唯一、环境合法；名称非空；数量为正整数；成本为有限正数；时间不得在未来。
3. 允许持仓数量因送转等账户事实出现非 100 整数手；这是持仓快照，不是订单，不套用订单手数规则。
4. 空快照只有在请求显式声明 `confirm_empty=true` 时才表示全部清仓。
5. 任意一项失败，整批拒绝，上一 active 快照保持不变。
6. 全部验证和持久化成功后，在同一事务中把新快照置为 active、旧快照置为 superseded。
7. 新快照未出现的旧代码，从 `effective_at` 起视为不再持有；历史版本仍保留。

选择当前快照使用稳定顺序 `(effective_at, confirmed_at, snapshot_id)`，取最新 active 版本。相同业务提交的幂等与冲突规则必须在实现前登记 2.10 规则 ID。

### 5.3 用户更新优先级

本地估值的持仓来源优先级固定为：

```text
最新已激活用户完整快照
> 上一版用户完整快照
> 无可用用户快照时明确不可用
```

不得回退到 watchlist、候选池、推送票池或从交易记录猜测当前持仓。用户只能更新持仓、数量、成本和生效时间，不能覆盖市场收盘价。

## 6. 工作流 B：每日收盘估值

### 6.1 价格口径

- 使用真实、未复权、可交易价格口径的日线收盘价。
- 盘前和盘中使用上一有效交易日收盘价；当日收盘数据只有在来源批次完成并通过质量校验后才切换。
- 价格必须大于零；日期连续、无重复；相邻有效值变化超过 ±20% 时按 2.3 告警并要求人工确认；除权除息一致性必须显式校验。
- 成本价来自用户当前快照；系统不根据历史 K 线反推或调整成本。

### 6.2 计算公式

对每只具备完整价格证据的持仓：

```text
market_value = close × quantity
unrealized_pnl = (close - cost_basis) × quantity
unrealized_return_pct = (close / cost_basis - 1) × 100
daily_price_pnl = (close - previous_close) × quantity
```

`daily_price_pnl` 的名称必须保持“当日价格变动损益”，不能缩写为券商“日盈亏”。它不含：盘中成交、已实现损益、手续费、印花税、分红税费、现金利息和未同步账户事件。

### 6.3 部分覆盖

每只持仓是独立证据单元。单只价格失败可隔离，但总计必须携带覆盖率：

```text
valued_items / snapshot_items
excluded_codes_with_reason
price_trade_date
position_snapshot_id
```

- 7/7 才能显示无修饰总计。
- 6/7 等情况显示“部分估值（6/7）”，不得把缺失项按零计算。
- 0/N 时整份估值不可用，只输出原因，不输出金额总计。

估值结果以 `ClosingValuationRun` 和逐项结果持久化，绑定用户快照 ID、价格交易日、数据来源、计算版本、覆盖率与证据哈希。推送只消费已成功持久化的估值视图。

## 7. 工作流 C：账户与数据状态消息

### 7.1 Banner

没有真实账户时，横幅不再显示“仓位缺失｜日盈亏缺失”，改成事实分层：

```text
[🔴 Frozen | 实时账户未接入 | 本地收盘估值可用 | 数据Degraded]
```

如果估值不可用：

```text
[🔴 Frozen | 实时账户未接入 | 本地估值不可用 | 数据Unsafe]
```

Banner 不显示仓位成数和券商日盈亏。Frozen 保持理由单独说明，避免用户把估值可用误解为交易授权恢复。

### 7.2 估值消息

消息必须包含：

- 用户持仓版本与更新时间；
- 价格交易日与提供方；
- 覆盖率；
- 每只持仓的数量、成本、收盘价、市值、浮动盈亏、收益率和当日价格变动损益；
- 部分失败的脱敏错误原因；
- “本地收盘估值，非券商实时盈亏、非下单指令”。

证券明细和金额只进入授权消息正文及受控业务存储；delivery audit、PR 证据、运行聚合日志和健康输出只保存哈希、计数、覆盖率及状态，不复制持仓列表和金额。

### 7.3 DataMode 诊断

能力状态至少区分：

```text
Warming | Healthy | Stale | Failed | Unsupported
```

- 启动首次探测截止前保持 Warming，不发送 `未建立 → Unsafe`。
- Quote 全局能力用独立真实行情探针判断，不依赖用户持仓快照是否在 30 秒内。
- OrderBook 在真实深度源接通前显示 Unsupported，不给出“等待恢复”ETA。
- 状态消息增加 provider、last_success、age、last_error、next_retry；移除静态“Quote 恢复后”。

## 8. 错误处理

| 失败 | 行为 |
| --- | --- |
| delivery audit legacy 校验失败 | 启动进入 AuditDegraded，普通 sink 不启动 |
| sink 已送达、审计失败 | 不重发 sink；记录 incident；暂停后续普通推送 |
| 完整持仓快照任一行非法 | 整批拒绝，保留上一 active 快照 |
| 快照为空但未确认全清 | 拒绝，不改变当前持仓 |
| 单只收盘价缺失/非法 | 排除该只，报告部分覆盖 |
| 所有价格不可用 | 不产生估值金额，只报告不可用原因 |
| 当日收盘批次尚未完成 | 使用上一有效交易日并明确价格日期 |
| 用户快照晚于价格日期 | 允许估值，但同时展示两个时点，不称实时盈亏 |
| 估值持久化失败 | 不推送临时内存结果 |
| DataMode 首次探测未完成 | Warming，按启动诊断记录，不发 Unsafe 变更 |

## 9. 并行开发边界

在用户书面确认设计并完成 implementation plan 后，使用三个并行工作流：

### A. Audit core

主要路径：`src/event/dispatcher.rs`、`src/event/push_record.rs` 及其 focused tests。只负责 legacy 兼容、预检 seam 和严格 v2 回归，不修改账户与消息模板。

### B. Snapshot and valuation core

主要路径：新增用户快照/估值库模块、数据库 repository/migration 和 focused tests。只提供强类型存取与估值结果，不修改 monitor 推送编排。

### C. Monitor runtime and message projection

主要路径：`src/bin/monitor/push_templates.rs`、`src/bin/monitor/main.rs`、`src/monitor/data_mode.rs`。负责 blocking 边界、Banner/估值渲染和能力状态，不修改 audit parser 或快照事务实现。

主集成者在三条工作流完成后串行完成 `notify.rs` 的投递状态组合、main wiring、业务规则登记核对和跨模块测试。共享文件不允许两个并行执行者同时修改。

## 10. 验收条件

### 10.1 Audit/Runtime

- 真实历史形状的测试夹具可以完成 legacy 全链验证并追加 v2，原始 legacy 字节不变。
- 空白、单边空、身份不匹配、新 legacy 和 v2 降级测试全部拒绝。
- audit 预检失败时普通 sink 调用数为零。
- sink 已送达后注入 audit 失败，同一 delivery identity 的 sink 调用数始终为一。
- debug `monitor --test` 的 I-01 路径不再出现 runtime-drop panic。

### 10.2 Snapshot/Valuation

- 两版完整快照提交后只读取新版，旧版仍可审计查询。
- 任一非法 item、未来时间、重复代码或未确认空快照都不会改变 active 版本。
- 收盘价、前收盘和成本的公式测试覆盖盈利、亏损、零变化及部分缺失。
- 缺失价格不补零，部分总计带准确覆盖率。
- 快照更新时间变化后，下一估值绑定新的 snapshot ID。

### 10.3 Message/DataMode

- 没有真实账户时不出现“仓位缺失”“日盈亏缺失”或“仓位 N 成”。
- 消息同时显示持仓版本时间和价格交易日。
- 估值消息明确标注非券商实时盈亏，且不能改变 AccountMode/Frozen。
- Warming、Stale、Failed、Unsupported 具有不同渲染；OrderBook Unsupported 没有恢复 ETA。
- PR/日志/审计验证不输出持仓列表、金额、账户身份或消息正文。

### 10.4 Gate

每条小 PR 必须完成各自 focused RED/GREEN、格式、严格 Clippy、相关集成测试和 `git diff --check`。集成分支再统一执行全 workspace tests、compliance、覆盖率阈值、release build、隔离 E2E、审计 canary 和生产消息 canary。任何 delivery audit、数据新鲜度或隐私检查失败都阻断合并。

## 11. 发布与回滚

发布顺序固定为：

1. Audit legacy 兼容与预检；
2. Runtime blocking 修复；
3. 用户快照与收盘估值，以 shadow 方式计算但不推送；
4. 核对估值覆盖率和用户最新版命中；
5. 启用新 Banner 与估值消息；
6. 最后启用丰富 DataMode 文案。

每一步独立 PR、独立 canary、独立回滚。回滚使用 `git revert` 和向前兼容 feature flag；快照、估值、incident 和审计记录不删除。回滚消息渲染不改变用户 active 快照，回滚估值计算不重新解释历史结果。
