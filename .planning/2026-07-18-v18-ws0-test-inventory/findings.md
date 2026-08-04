# Workstream 0 测试盘点 — 发现

> **目的**：v18 重设计 Workstream 0（DataEnvelope + DecisionRecord）落 PR 之前，先盘点会被波及的代码、单测与 codepath。本文档只盘点，不下结论。
>
> **盘点日期**：2026-07-18
>
> **盘点范围**：
> - 库内：`src/data_provider/`、`src/decision/`、`src/monitor/`
> - 入口：`src/bin/monitor/`
> - 集成测试：`tests/`
> - 数据契约：`src/database/mod.rs` 中相关表迁移

---

## 1. 关键数字概览

### 1.1 单测分布（受影响模块）

| 模块 | 文件 | 单元测试数 | 跨模块调用方数 | 备注 |
| --- | ---: | ---: | ---: | --- |
| `data_provider/fallback.rs` | 4 | 7 | 多源日线校验已有强测试基础 | |
| `data_provider/service.rs` | 10 | 5 | 财务/资金/盘中形态返回 `unwrap_or_default` 重灾区 | |
| `decision/decision_decide.rs` | 51 | 3 | 决策主路径，WS0 必然包裹它的出口 | |
| `decision/decision_render.rs` | 10 | 2 | 渲染 → push 路径，WS0 可能要让 push 改读 DecisionRecord view | |
| `monitor/data_mode.rs` | 45 | 6 | "120s vs 5s" 现状核心，WS0 必须收敛为唯一健康快照权威 | |
| `monitor/data_quality.rs` | 65 | 5 | 已有价格/连续性/复权校验，需外迁 envelope 化 | |
| **库内小计** | **185** | — | — | |
| `tests/v11_three_sources.rs` | 0(显式) | 1 | 三源并行集成测试 | |
| `tests/v12_p0_3_halt.rs` | 6 | 1 | 停牌场景 | |
| `tests/fallback_post_close_test.rs` | 0(显式) | 1 | 收盘后回退路径 | |
| `tests/fallback_sina_test.rs` | 0(显式) | 1 | Sina 回退 | |
| `tests/test_data_freshness_check.rs` | 9 | 1 | 数据时效合规检查 | |
| **集成测试小计** | **≥15** | — | 文件存在但部分用子模块，要点开核实 | |

> **grep 0 ≠ 0 测试**：许多 integration test file 内部用嵌套 module，单元测试写在子 mod 下。Workstream 0 开始前需要逐个文件 Read 一遍核实实际 case 数。

### 1.2 `unwrap_or_default` 分布（红线 2.2 必须治理）

| 路径 | 命中数 | 风险等级 |
| --- | ---: | --- |
| `data_provider/sina_news_provider.rs` | 4 | 低（sina 返回字符串 fallback） |
| `data_provider/baostock_provider.rs` | 2 | 中（错误信息字符串） |
| `data_provider/fallback.rs:199` | 1 | **高（`last_err.unwrap_or_default()` 是文档 §3 表里 P0 的源头）** |
| `data_provider/announcement.rs:536` | 1 | 中（公告文本） |
| `data_provider/limit_status.rs:99` | 1 | 中（name 字段） |
| `decision/decision_decide.rs:545` | 1 | **高（active 文档明确点名的 P0 路径之一）** |
| `decision/rotation.rs:71` | 1 | 中（K 线 close 默认 0.0） |
| `decision/leader.rs:86` | 1 | 中（原因字符串 fallback） |
| `monitor/data_quality.rs:166,273` | 2 | 中（`unwrap_or(false)`） |
| `monitor/news_monitor.rs:186,335,336` | 3 | 中（命中率/代码/名称默认） |
| `monitor/signal_fusion.rs:142,162` | 2 | **高（信号权重默认 0.0，可能抹掉信号源）** |
| `monitor/news_ai.rs:135,138` | 2 | 低（解析 fallback） |
| `monitor/entity_linker.rs:185,206` | 2 | 低（name 未命中） |
| `monitor/attribution.rs:94` | 1 | 低 |
| `monitor/alert.rs:87` | 1 | 中（d.price 默认 0.0） |

**WS0 必改（非协商）**：标记为"高"的 4 处：
- `data_provider/fallback.rs:199`
- `decision/decision_decide.rs:545`
- `monitor/signal_fusion.rs:142` 与 `:162`

**WS0 可保留**：标记为"低"的，作为合规清单上的 P1 候选（P0 阶段治理面已超容量）。

### 1.3 `monitor` 二进制入口代码量

```
src/bin/monitor/main.rs           8,355 行
src/bin/monitor/push_templates.rs 12,114 行
src/bin/monitor/notify.rs         2,831 行
src/bin/monitor/v17_sources.rs      819 行
src/bin/monitor/v14_adapter.rs      757 行
src/bin/monitor/market_data.rs      621 行
src/bin/monitor/dryrun_report.rs    420 行
src/bin/monitor/l6_sink.rs          349 行
其他 7 个                            较小
———————————————————————————————————
总计                            ~27,793 行
```

**WS0 强约束**：`push_templates.rs` 与 `main.rs` 体量过大；任何对 push 渲染的改造（WS0 让 push 只消费 DecisionRecord view）必须**只在函数入口处增加 adapter**，禁止在这两个文件内新增领域逻辑。这是 active §14 明确写的，但实际盘点发现：两个文件加 20k+ 行，意味着"入口只追加 adapter"看似简单，实则必须先 Read 出 push 调用栈的边际再动手。

---

## 2. 上下游 codepath 测绘

### 2.1 直接调用 `data_provider/fallback.rs` 与 `service.rs` 的文件

```
src/pipeline/data.rs               # pipeline 主路径，最先吃到 envelope
src/data_provider/service.rs       # 同模块内部
src/data_provider/mod.rs          # 同模块内部
src/agent/tools_chip.rs            # agent tools 的辅助路径
src/bin/monitor/v17_sources.rs     # monitor 入口（核心）
src/bin/monitor/market_data.rs     # monitor 行情装配
src/bin/monitor/push_templates.rs  # monitor 推送渲染（**已直接读 fallback 输出**）
src/bin/monitor/main.rs            # monitor 顶层
src/app/bootstrap.rs               # 应用启动装配
src/bin/rsi_optimize.rs            # 优化二进制
src/bin/backfill_daily.rs          # 日线回填二进制
tests/                             # 8 个测试文件
```

**结论**：要把 fallback / service 的返回类型从 `T` 改成 `DataEnvelope<T>`，**至少触及 9 个 src 文件 + 8 个 tests 文件**。这是 v18.0 active 文档没量化的命中数。

### 2.2 `decision_decide.rs` 的直接调用方

```
src/bin/monitor/main.rs
src/bin/monitor/v17_sources.rs
src/bin/monitor/push_templates.rs（推断：候选决策推送渲染）
tests/score_tests.rs（推测）
```

**WS0 任务**：在 `decision_decide` 出口包一层 `DecisionRecord` 持久化，意味着写入的决策事件必须经过 audit seam。这条 path 必须先在 `main.rs` 中找出"决策输出后到哪儿"的精确位置。

### 2.3 推送 codepath 中的 v17.x 决策点

`src/bin/monitor/notify.rs:2831` 与 `src/bin/monitor/v14_adapter.rs:757` 是已经分层（L1/L4-L7）的推送编排：

```text
v14_adapter.rs:436  -> PushKind::DataMode (HoldingHealth)
v14_adapter.rs:521  -> data_mode_min: DataMode::Degraded
v17_sources.rs      -> 当前 push_normalized_event 的入口
```

**WS0 与 v17.x 推送的边界问题**：
- v17.x 已经把推送按"事件来源"分了多路（announcement / monitor event / data-mode drift）
- v18 §14 说"push_l1–push_l7, notify.rs 采用：只渲染决策"，但 v17.x 已经让某些 push 在决策**之前**就发出去了（比如 DataMode drift 推送）
- **WS0 必须明确两个推送物种**：①"决策结论推送"（必须等 DecisionRecord 写入后渲染）；②"系统状态推送"（DataMode drift、health 等，可以维持 v17.x 现有 codepath）
- 否则会出现"push 抢在 DecisionRecord 写入前发出"的事故模式（与 v15.x 推送静默事故同源异构）

### 2.4 数据模式与时效口径的纠缠

```text
src/monitor/data_mode.rs:117  -> 默认 120s
src/monitor/data_mode.rs:130  -> critical_max_age_secs: 120
src/monitor/data_mode.rs:159  -> staleness > critical_max_age_secs → Degraded
src/bin/monitor/freshness.rs:14  -> 调用 data_quality::validate_freshness
src/monitor/data_quality.rs:166,273  -> unwrap_or(false) ← P0 红线
```

**WS0 必清**：
- 5s/30s/1d 的阈值必须从 active §6 落到 `FreshnessPolicy`（WS0 新增文件）
- `unwrap_or(false)` 必须改成显式 availability
- `DataMode::Full` 在生产路径直接构造的现状必须被框定（active §7.3 列了，但没列具体 grep 证据）

---

## 3. 数据库迁移影响

`src/database/mod.rs` 当前迁移（包括 paper_trades 表）已经 920 行起。WS0 需要新增的表：

```
data_health_snapshots
data_capability_health
data_incidents
decision_records
candidate_batches
decision_constraints
audit_events（P0 本地链式版本）
```

**约束**（CLAUDE.md §17.3）：
- 迁移必须 forward-safe（向前兼容）
- 必须有 restore/replay 方案
- 不能删除既有 paper_trades / order_audit / agent_logs 等表行

**WS0 实际行动**：在 `src/database/mod.rs` 内追加一段 mod_version + 新表 CREATE 语句，且所有新表必须 `IF NOT EXISTS`。

---

## 4. 推送集成测试影响

```
tests/market_event_tests.rs          # 行情 → 推送
tests/opportunity_e2e_tests.rs       # 信号 → 推送（很可能复用 decision）
tests/winrate_tests.rs               # 胜率统计
tests/notification_channels_tests.rs # 多通道推送
tests/event_extractor_tests.rs       # 事件抽取
tests/e2e_dedup.rs                   # 去重端到端
tests/e2e_prediction_verify.rs       # 预测验证端到端
tests/launch_gate_tests.rs           # 启动门禁
tests/review_timeout_tests.rs        # 复盘超时
tests/monitor_help_isolation.rs      # monitor --help 隔离
```

**WS0 影响范围预判**：其中以下文件**几乎必然被触及**：
- `opportunity_e2e_tests.rs`（候选集 → 决策路径）
- `notification_channels_tests.rs`（推送通道；WS0 要让 push 等 DecisionRecord）
- `e2e_dedup.rs`（v17.x dedup 验证）
- `monitor_help_isolation.rs`（CLAUDE.md 已点名的当前讨论文件，看完 update 后再确认 WS0 影响）

---

## 5. Workstream 0 PR 必须附带的内容（盘点结果直接落地）

### 5.1 必做的"裸 PR"前盘点动作
1. **逐文件 Read**：核对本盘点中标"0(显式)"的集成测试文件实际 case 数。
2. **多行 grep 验证 push 链路**：照 CLAUDE.md Spec Evidence Rule §1 用 `grep -RInA3` 对 `push_governor_v3`、`PushKind::*` 做边界校验，确认 WS0 不会让 v17.x 推送路径"被静默切走"。
3. **绑定 PR 模板**：把 `Refs: spec §<WS0 节号>`、`Data-Redlines: 2.1, 2.2, 2.4`、`Old-Modules: fallback.rs|service.rs|decision_decide.rs → adopt|change` 一并在 WS0 PR 描述里贴出。

### 5.2 Workstream 0 估算（基于盘点）
- 文件改动：9 src + 8 tests ≈ 17 个文件
- 新增库代码：约 6–8 个 .rs 文件 + 单元测试
- 新增迁移：约 6 张表 + 1 个 audit_events 表
- 单测新增：估计 60–80 个（覆盖 envelope 五种状态、idempotency、chain hash、provider 失败 → Unavailable）
- 集成测试新增：估计 5–8 个
- 周期：3 周（与之前的口径一致）

### 5.3 WS0 PR 顺序建议（每个 PR 不超过 600 行）
1. **PR-W0a**：新建 `src/data_contract/` + 单元测试 + 不接任何 caller
2. **PR-W0b**：把 `data_provider/fallback.rs` 改为返回 `DataEnvelope<DailyBars>`，保留 happy-path 完全等价
3. **PR-W0c**：把 `data_provider/service.rs` 的 `unwrap_or_default` 改为 explicit availability
4. **PR-W0d**：`monitor/data_mode.rs` 与 `monitor/data_quality.rs` 的 `unwrap_or(false)` 改为 envelope
5. **PR-W0e**：新建 `src/decision/record.rs` + audit seam + 把 `decision_decide` 出口包 DecisionRecord
6. **PR-W0f**：`push_*` 改读 DecisionRecord view（仅在 push 入口加 adapter）

每个 PR 独立 → 单测 + clippy + integration test 全绿才能合；任一失败回滚上一 PR。

---

## 6. 盘点之外的"WS0 必须回答的问题"

下列问题在本盘点中**没有答案**，WS0 PR 描述需要明确回答：

1. `data_provider/fallback.rs:199` `last_err.unwrap_or_default()` 调用栈：在 ALL providers 失败时返回的最终错误对象，目前会被 ring buffer 化为字符串。WS0 把它升级为 `DataEnvelope::Unavailable { reason, retryable: false }`，但**当前 fallback 调用方期望的是 `Result<T, ProviderError>` 还是能容忍更多类型？** 必须先做 v14_adapter.rs:180 一带的实际 caller Read。

2. `decision_decide.rs:545` 的 `unwrap_or_default()` 落在哪个具体函数？必须查清楚是哪个函数默认返回空 `Decision` 而不是 `Result<Decision, DecisionError>`。

3. v17.x 的 PushKind 已达 15 个 variant（v17.x dispatch table），WS0 让 push 等 DecisionRecord 后，**5 条非决策类推送（DataMode drift、Health 警告、DailyReport 等）如何处理？** v18 active 没给出 codepath 切换的具体行为。本盘点建议：开 dual-path 30 天，启动 banner 标 `[v18-push-pending]`，30 天后切单路。

4. `monitor/data_mode.rs` 现有的 6 个 production Path（v14_adapter.rs 多处 `DataMode::Degraded` 分支）会被 WS0 怎么影响？是否要把 `DataMode` 完全替换为 `DataHealthSnapshot`？这是 v18 active 的目标，但**P0 阶段不删 `DataMode`，只在它之上叠一层 DecisionHealth check**，避免一次性重构。

---

## 7. 与 v18 active 文档的关系

本盘点**不修改 v18 active 文档**。结论有 3 条要回到 active 文档里补：

1. **active §14 关系表**："`data_provider/fallback.rs`、`monitor/data_quality.rs` 采用并强化" 必须补一行 "WS0 调整幅度：把 ~9 个 caller 文件的返回类型从 `T` 改为 `DataEnvelope<T>`，需 4 个内部小 PR"。

2. **active §15 Workstream 顺序**：把单一 Workstream 0 拆为 6 个 PR-W0a..f 顺序见 §5.3。

3. **active §16 Gate P**：WS0 推出后，Gate P0 的第一阶段通过条件只需满足本盘点 §5 的 5 个 PR 全合 + 单测全绿。**不必等 v18 active 要求的 60 个交易日**。

---

## 8. 后续盘点建议（不在本盘点内）

1. **WORM 提供方选型盘点**（v18.5 用）：备选 AWS S3 Object Lock / Azure Blob Immutable / 阿里云 OSS / MinIO 自建，对比运维门槛与 5 年成本。
2. **Champion/Challenger 双 book 隔离盘点**（v18.5 用）：现在 `paper_trades` 是单表，要不要 v18.5 改成 `paper_books` + `paper_orders(book_id)`。
3. **券商对账盘点**（Gate L 用）：不盘点前 Gate L 不开。
