# 单票完整生命周期（ticket）实施方案

状态：进行中（2026-10-17 起草）
依据：operator 2026-10-17 给出的五条诉求 + 当日代码/数据实测
定位：这是**方案**，不是承诺。每阶段都写明"能验证什么"和"做不到什么"。

## 一、目标（operator 原话拆解）

1. 以周为周期（按板块轮动而非固定周），周末/周初用新闻催化 + 上周板块轮动迹象，
   在周一开盘与集合竞价找出热点板块最热的票，给出**可买入价格**；T+1 当天不考虑卖出。
2. 盘中密切关注该票：新闻、盘中数据、公司公告，以及**直接/间接关联公司**。
3. 次日集合竞价 + 盘中，判断**持有 / 加仓 / 做 T**，且每个动作要有**明确价格**。
4. 同时关注板块轮动，准备下一个板块的股票。
5. 无论盈利亏损都要有**复盘归因**：为什么赚、如何继续；为什么亏、如何避免。

## 二、现状（2026-10-17 实测，不含推测）

已完成（地基）：

- `ticket_lifecycle` / `ticket_event` 两张表 + `(code, open_date)` 唯一索引。
- `stage` 为 TEXT 无 CHECK 约束（枚举可演进）。
- 手工工具：`bin ticket_open` / `bin ticket_show`。
- 观察池生成器：`monitor/ticket_watch_pool.rs`（新闻催化路），`bin watch_pool_build --dry-run`。
- op 67 解析层：`data_gateway/provider_observed_trading_facts.rs`（8 单测）+ 请求构造
  `build_external_native_query_request::QualifiedTradingFacts`。

未接（缺口，按实测）：

- **monitor 零 ticket 调度**：`grep ticket src/bin/monitor/main.rs` = 0 处。
- **推送 → ticket_event 零接线**：`append_event` 只有观察池生成器与手工 CLI 调用。
- **生产库 0 行**：ticket_lifecycle 0 / ticket_event 0。
- **无"可买价"**：`PoolCandidate` 只有 code / note，没有价格字段。
- **板块轮动路未接**：`save_board_rotations` 零生产调用，`board_rotation_daily` 停在 2026-07-16。
- **关联公司未做**（直接/间接都没有）。
- **HoldingPlan 停供**（2026-09-23 起 owner 读取拒绝）。
- **逐票归因未做**：`advice_outcome_tracking` 只接 T0Advice 一种 push_kind。

## 三、阶段设计

### P1 让票真的被跟踪（一切的前置）

P1.1 monitor 每日调度观察池

- 位置：`src/bin/monitor/main.rs::market_loop`，按 `static X_LAST: Mutex<Option<NaiveDate>>` 模式。
- 时机：交易日 15:40 之后（盘后，新闻评估已产出），每日一次。
- 动作：`ticket_watch_pool::build_watch_pool(db, date, lookback=5, dry_run=false)`。
- 验证：日志出现 `[观察池] scanned=N candidates=M opened=K appended=J`；
  `SELECT COUNT(*) FROM ticket_lifecycle WHERE source='news_catalyst'` 增长。
- 做不到：输入源断供时（`news_ai_assessment` 无新增）只会开存量票，日志必须如实报 0。

P1.2 推送 → ticket_event 自动记账（best-effort）

- 位置：T0Advice / HoldingPlan / CloseCall 推送成功后（durable delivery 已确认 Delivered）。
- 动作：`append_event(ticket_id, event_at, event_kind, push_kind, occurrence_identity, summary)`。
- 约束：**best-effort**——写失败只 `log::warn`，绝不影响推送；票不存在则跳过（不自动开票）。
- 验证：推一条 T0Advice 后，对应 ticket 的事件时间轴多一行。
- 做不到：只覆盖"有 ticket 的票"；没有 ticket 的推送不回溯补建。

P1.3 op 67 价格带每日落库

- 原因：对端明确"价格带只在观测快照日有值"，错过当天就永久丢失（不能用时再查）。
- 动作：每交易日盘后查一次 `QualifiedTradingFacts`（include=lifecycle+price_band），
  落 `stock_daily` 或独立表（字段：code / date / limit_up / limit_down / price_band_source / evidence）。
- 验证：表中每个交易日的行数 = 当日查询成功的票数；`price_band_source` 非空。
- 做不到：非快照日返回 null（按 BR-222 保持 NULL，不补值）。
- **前置**：op 67 当前只有请求构造，没有业务调用点；本项即第一个调用点。

### P2 可买价（观察池 → 入场参考）

- 复用：`decision/holding_plan.rs` 的支撑/压力位逻辑 + `stock_daily` 的 ma5/ma10/ma20/volume_ratio。
- 输出：**参考区间 + 触发条件**，不是单一价格。例："12.30-12.55 区间，需放量站上 ma5"。
- 写入：`ticket_event(kind=entry_proposed)` + `ticket_lifecycle.stage='entry_proposed'`。
- 验证：dry-run 输出每个候选的区间与依据字段（ma/量比/前高）。
- 做不到：不预测涨停；无 `stock_daily` 覆盖的票不给区间（只标注"数据不足"）。

### P3 定向监控（观察池的票）

- P3.1 新闻/公告按 ticket 的 code 过滤（现有全局采集 → 过滤 → 追加事件）。
- P3.2 关联公司（弱关联版）：`board_memberships` 同板块 + 新闻共同提及；
  明确标注"弱关联"，不做股权穿透（无数据源）。
- 验证：某票当天新闻命中时，事件时间轴出现 `news_catalyst` / `announcement` 行。
- 做不到：间接关联只能做到"同板块 + 同新闻"，无法做供应链/股权穿透。

### P4 次日动作 + 价格

- 路径 A（推荐）：修 HoldingPlan（真因已有观测，待盘中 tick）→ 复用其动作与价格。
- 路径 B（备选）：`ticket` 专属次日决策模块，复用 T0Advice 的价位计算，但**不新造一套风控**。
- 验证：次日 9:25-9:35 对观察池票输出 `hold/add/t0` + 价位；写入 ticket_event。
- 做不到：HoldingPlan 未修好前，"减仓/止损"类结论只有 T0Advice 一条腿。

#### 排查记录（2026-10-18）

**现象**：2026-09-23 09:46 之后，每日 5 只持仓票全部
`[I-04][T-03] code=xxx 原 owner 读取拒绝: holding_plan_owner_read_failed`，
HoldingPlan 决策不再写入。

**证据**：

```
delivery_decisions  HoldingPlan 按日：
  2026-09-21  290
  2026-09-22    5
  2026-09-23    5     ← 全部 Delivered，时间 09:46-09:47（北京时间）
  之后          0

state 分布：RejectedDurable 2859 / Delivered 161 / ManualResolvedRejected 6
```

错误消息只有前缀，`{e}` 渲染为空 —— 底层错误没有 Display 文本。

**已排除**（逐条实测）：

| 假设 | 证据 | 结论 |
|---|---|---|
| 表缺失 | `immutable_audit_outbox` / `delivery_disposition_payloads` / `task_transition_payloads` 都在 `durable_delivery.sqlite3`（19 张表） | ✗ |
| 多进程争用 shm | `lsof` 显示只有 monitor 打开该库（2 main + 3 shm fd） | ✗ |
| schema 版本不符 | `PRAGMA user_version=9`，代码用 `open_existing_monitor_schema9`；启动时 catalog 校验通过 | ✗ |
| `OccurrenceContext` 校验 | 该函数各分支错误都有具体消息（`ticket_not_equity` / `ticket_code_invalid`） | ✗ |
| 当日无决策导致失败 | `load_strict_owner` 无记录时返回 `Ok(None)` → `Missing`，走 fresh 分支，不报错 | ✗ |
| database_binding 缺失 | 只在 teardown 时为 `None`；进程正常运行是 `Some` | ✗ |

**待定**：真实错误在 `with_connection` 或 `capture_sql_binding` 内部，
但 Display 为空，无法从日志判断。

**已做**：把 5 处 `format!("...: {e}")` 改成 `{e:#}`（anyhow alternate 输出完整因果链），
位于 `src/bin/monitor/durable_delivery_runtime/holding_plan.rs:40,65,102,204,235`。

**验证时点**：周一 10:30 I-04 首次触发时，日志会带完整错误链。
拿到真实错误后按错误类型修（结构问题改 SQL/校验；状态问题改流程）。

### P5 板块轮动接力

- 接 `save_board_rotations` 写入方：板块排名（已有 gateway）+ 板块成分 + 新闻标题，三源组装。
- 每日产出"下一个板块"的候选，进观察池（source=board_rotation）。
- 验证：`board_rotation_daily` 开始每日新增；观察池出现 source=board_rotation 的票。
- 做不到：板块轮动的"早期信号"本质滞后；只能做提示，不能当买入依据。

### P6 逐票复盘归因

- `advice_outcome_tracking` 的逐票结果 append 到 `ticket_event`。
- 归因维度：个股自身（事件/新闻）vs 板块（同板块涨跌）vs 大盘（指数）。
- 验证：一只票平仓后，时间轴尾部出现 `outcome` 行，带 T+1/T+3/T+5 与归因标签。
- 做不到：归因是统计相关，不是因果；报告必须写明"假设执行、非真实收益"。

## 四、顺序原则

1. **P1 必须最先**——没有自动化，后面每个阶段都只有 dry-run 能跑。
2. **P1.3 与 P1.1 并行**——价格带错过当天永久丢，时间敏感。
3. **P2 依赖 P1.1 的票**，P4 依赖 P1.2 的时间轴，P6 依赖 P4 的动作记录。
4. **P5 可独立**——不依赖 ticket，是另一个数据源接线。
5. **P3 的弱关联版可以晚做**——它有替代品（人看新闻），P2/P4 没有替代品。

## 五、明确不做

- 不自动改规则/不自动调参（AGENTS §2.10、`performance_feedback.rs:4`）。
- 不补 0、不推导涨跌停价（BR-222 / BR-205）。
- 不给"保证盈利"的单一买价；只给区间 + 触发条件。
- 不做股权穿透式的关联公司分析（无数据源）。
