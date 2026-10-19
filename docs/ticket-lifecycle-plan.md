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

- **P3.1 新闻/公告按 ticket 的 code 过滤 — 已完成（2026-10-19）**
  - 盘后回溯（`main.rs::post_close_news_review`）：标的从「只持仓」扩到
    「持仓 ∪ `ticket_lifecycle` 活跃票」。以前观察池的票根本没有新闻数据，
    优先级打分里的「新闻催化」维永远 0 分。
  - 盘中扫描（`watch_pool_monitor::scan`）：新增第三条规则——查 `news_items`
    最近 10 分钟、未被 `news_dedup` 标记的新闻，命中就报「新闻：{标题}」。
  - 去重：通知后写 `news_dedup` key=`watch_news:{id}`，30 秒一轮不会重复。
  - 边界：新闻票必须在行情批次里才报（拿不到价格就没有意义）。
- **P3.3 盘中同板块关联票异动 — 已完成（2026-10-19）**
  - `watch_pool_monitor::scan` 第四条规则：观察池的票属于哪些板块（`board_member`），
    这些板块里今天涨幅 >= 5% 的其他票就报，文案「同板块「风电」000099 XX +6.2%」。
  - 每个板块只保留涨幅最高的一只，挂在主票的 `reason` 里，不单独成条。
  - 阈值 5% 而非 3%：板块内每天一堆 3% 的票，报出来是噪音。
  - 单轮最多查 200 只关联票行情（`PEER_QUERY_LIMIT`），保护行情接口。
- **关联公司（直接/间接）仍然做不到，原因是数据不是代码**：
  `chain_relation_candidate` 77 条记录 `object_code` **全部为 NULL**，
  `object_name` 是「钢材」「原料药」「航空煤油」这类产业环节——`chain_ai` 的
  系统提示词明确要求写「具体的大宗商品或原材料」而不是公司（理由：只有具体到
  商品才能盯价格）。所以这一路盯的是**商品价格**，不是关联公司股价。
  要做「直接关联公司」得改提示词让它输出上市公司代码，或接股权/供应链数据源。
- **T+1 当日买入禁卖 — 已完成（2026-10-19）**
  - `position_change_detect::t1_locked_quantities`：当前快照 vs 上一份的
    `quantity` 增量 = 当日买入股数（系统拿不到券商「可用数量」字段，
    `UserPositionItemInput` 只有 quantity / cost_price）。
  - `holding_plan`：整仓都是当日买入时，浮盈 > 5% 不给减仓，降级 `Hold`，
    文案「浮盈 X% 但当日买入 N 股 T+1 锁定，今日不可卖」。
  - 读锁定量失败时 `warn` 并出声，不静默退化成「可以卖」。
  - 边界：两次快照之间买入的都算锁定；隔多日才导入一次会多锁一天（偏保守）。
  - 只挡 `Reduce`，不挡 `StopLossReview`（止损是核查提示，不是卖出指令）。
- 验证：某票当天新闻命中时，事件时间轴出现 `news_catalyst` / `announcement` 行；
  持仓票在买入当日浮盈 >5% 时，HoldingPlan 应输出「T+1 锁定」而不是减仓价。

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

- **P5 轮动信号 — 代码完成（2026-10-19），等数据**
  - `src/monitor/board_rotation_signal.rs` + CLI `board_rotation_build`
  - 表 `board_rotation_signal`（不动旧 `board_rotation_daily`，
    后者要求 `news_title` 非空，硬塞空串等于造假）
  - 三态：`mainline`（连续在 top10）/ `new`（今天新进 top10）/ `fading`（昨天在、今天掉出）
  - 判定用**名次**不用涨跌幅：涨幅可能因复权/口径差异不可比
  - 单日保护：`has_comparable_days` 检查，只有一个交易日时直接报错退出
  - **2026-10-16 实测**：报「之前没有第二个交易日的板块榜，无法判断轮动」✓
  - **数据缺口**：`board_ranking_daily` 目前只有 10-16 一天真实数据。
    2026-10-18 那 175 行是周日手工跑 CLI 写入的 10-16 副本（逐行相同），已删除。
- **P5 选票入口 — 已接通（2026-10-19）**
  - 桥接 `BoardConstituents` 新增 `board_code` 口径。此前 `fetch_board_constituents`
    只能调 `gateway.memberships(code)`，方向是「个股→板块」；provider 的
    `BoardConstituentRequest { board_code, limit }` 本来就支持按板块查成分，但
    `BoardDataGateway` 没有公开入口（`board_constituents_raw` 是 `pub(super)`，
    `board.rs::board_constituents` 要 `VerifiedBoardBinding`，而 binding 是空列表）。
    改法：`board_runtime::constituents_by_board()` 复用 `board_constituents_raw`
    + 与 `fetch_memberships` 同一套 evidence 校验（改校验返回的 `board_code` 一致），
    `delegate::fetch_board_constituents` 优先读 `board_code`，保留 `codes` 兼容路径。
    提交在 `.worktrees/tdx-local-provider-20260923` 的 `479e761d2`。
  - 新表 `board_member(board_code, board_name, kind, instrument_code)`，
    CLI `board_member_build --from-ranking --date <日> --top 10`。板块名先试
    `tdx:concept:<名>`，失败再试 `tdx:industry:<名>`（新浪概念榜与 TDX 分类不同）。
  - 观察池 `ticket_watch_pool::collect_board_rotation` 接第二路：当日板块榜 top10 →
    `board_member` 成分股 → 按**上一完整交易日**涨幅取每板块前 3（`BOARD_PER_BOARD=3`）。
    用上一交易日而非当日：`stock_daily` 当日只有活跃票样本（10-19 仅 9 只），
    拿它排序会把 1241 只里的强票挡在外面。
  - monitor 15:40 调度：先 `build_from_ranking(10, 10_000)` 再 `build_watch_pool`。
  - **2026-10-19 实测**：top10 板块里 5 个成功、5 个报「服务端内部错误」
    （海上丝路/海工装备/氢燃料/军工航天/抗癌）；成功的是国产软件 261、风电 400、
    仿制药 73、民营医院 78、草甘膦 17 只，共 829 行。观察池 16 候选
    （2 新闻 + 14 板块），开票 14 只 `source=board_rotation`。
  - **已知限制**：约一半新浪概念名在 TDX 板块里对不上（返回内部错误而非空），
    目前只能记 `failures` 跳过。板块覆盖不全 = 选票面偏窄。
  - 桥接重启后**首次监听要等约 140–260 秒**（`DatabaseManager::init` 跑 migrations），
    这段时间内所有数据调用报 `unavailable`。重启桥接要留足观察窗口，
    不要在这个窗口里判定启动失败。
- 每日产出"下一个板块"的候选，进观察池（source=board_rotation）—— 已接。
- 验证：`board_rotation_build --date <日>` 输出「主线/接力/退潮」各几个板块；
  `watch_pool_build --date <日> --dry-run` 能看到 `[board_rotation]` 行。
- 做不到：板块轮动的"早期信号"本质滞后；只能做提示，不能当买入依据。

### P6 逐票复盘归因

- **P6 二层归因 — 已完成（2026-10-19）**：`src/monitor/attribution_benchmark.rs`
  + CLI `attribution_benchmark_build`，落表 `attribution_benchmark_daily`。
  - `stock_pct`    `stock_daily.pct_chg`
  - `market_pct`   当日全市场等权平均，**样本 >= 50 才采纳**，否则 NULL（BR-222）
  - `excess_pct`   `stock_pct - market_pct`，即选股超额
  - `board_pct`    **留 NULL**：板块日线只有 2026-10-16 一天（新浪概念快照），
    D3 板块成分映射未落地，填一个估出来的值比 NULL 更糟。
  - 实测 2026-10-09：817 只全部带基准，市场 +0.69%，
    300413 超额 +19.29 / 300340 +15.20 / 688561 +10.52。
  - 实测 2026-10-16：18 只，样本不足 → `market_pct` / `excess_pct` 全 NULL。
  - 补样本：`daily_bars_build --codes` 可显式指定标的，
    `stock_concepts` 里有 1265 个代码，补完全市场后基准即可采纳。
- 已有的 `paper_attribution_daily` 是**按 signal_family 统计 PnL/胜率**，
  停在 2026-08-28、`attribution_run_audit` 0 行，与本轮的二层拆分不冲突。
- 验证：`attribution_benchmark_build --date <日> --show` 能看到个股/市场/超额三列。
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

## 六、P4 停供真因（2026-10-18 定位并修复）

### 现象

`HoldingPlan` 自 2026-09-23 09:47 之后 0 决策，累计 103 次：

```
[I-04][T-03] code=xxx 原 owner 读取拒绝: holding_plan_owner_read_failed
```

`{e}` 渲染为空，看不到真实原因。

### 真因

`src/durable_delivery/coordinator_holding_plan.rs` 的 `authority_sha()` 遍历
`AUTHORITY_TABLES`（17 张表）时**无条件** `SELECT rowid,* FROM {table}`：

```rust
for table in AUTHORITY_TABLES {
    let sql = format!("SELECT rowid,* FROM {table} WHERE ...");
    let mut query = c.prepare(&sql)?;   // ← 表不存在就到这里炸
```

而第 8 张 `delivery_correlation_observations` 是 **schema v10 才加的**表
（`schema.rs:357`，注释写着「Correlation is an audit edge, not a delivery authority」）。

monitor 用 `open_existing_monitor_schema9` 打开生产库，生产库 `user_version=9`，
**没有这张表** → `prepare` 报 `no such table` → 被上层包装成
`holding_plan_owner_read_failed`。

### 为什么之前排查没找到

`open_existing_monitor_schema9` 的 `monitor_schema9::verify()` **不检查这张表**
（它只校验 v9 的表），所以 coordinator 能正常打开、日志显示一切正常，
只有在真正读 occurrence 时才炸。而且错误被 `{e}` 空渲染吞掉了。

### 怎么提前定位的（不用等交易时段）

写了一个探针 `src/bin/holding_plan_probe.rs`：

- `inspect_holding_plan_occurrence` 是 lib pub；
- `CoordinatorConfig::production()` 接受任意 owner identity
  （不匹配的 owner 走 `Ok(None)` → `Missing`，不是错误）；
- 所以任意时刻都能复现同一条读取路径，不必等 10:30 的 I-04。

第一次跑就拿到：

```
sqlite durable-delivery failure: no such table: delivery_correlation_observations
```

### 修复

`authority_sha()` 里对缺失表跳过，并 frame 一个 `"absent"` 标记：

```rust
if !table_exists(c, table)? {
    frame(&mut h, b"absent");
    continue;
}
```

理由：schema.rs 自己写明这张表只是审计边、不是投递权威，缺失合法；
frame `"absent"` 保证哈希仍随 schema 状态变化（表被创建后哈希会变，
`binding.validate` 会正确报 `exact_owner_sql_witness_changed`）。

其他 16 张表缺失仍然报错（fail-closed 不变）。

### 验证

```
$ holding_plan_probe 601811 2026-10-16
[probe] coordinator 已打开
[probe] OK: Missing        ← 修复前是 no such table
```

### 未验证的部分

- 生产库（v9）上的真实读取：要等运行根部署后跑 probe 确认。
- `Owned` 分支从未被测到过（生产库里 9-23 之前的 HoldingPlan 记录也不会有
  correlation 观测，所以历史决策读出来都是 `Missing`）。
- 修复后能不能真的产生新决策：要等下一个交易日的 I-04 触发。
