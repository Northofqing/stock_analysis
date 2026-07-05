# v12 monitor 实盘验证报告

> 验证时间: 2026-07-05 08:53 周日 (非交易日, 用 `--test` 跳过日历)
> 命令: `V10_DRY_RUN_PUSH=1 cargo run --bin monitor -- --test`
> 日志: /tmp/m.err (1449 行)

---

## 一、整体结论

**5 个 pre-v12 推送路径全部跑通**：
- ✅ 告警聚合摘要 (T-alert)
- ✅ 交易复盘 (R-daily)
- ✅ 新闻 Ranker (T-news, 但 A=0 B=0 C=0)
- ✅ 排除板块命中 (T-exclusion)
- ✅ 现金预警 (T-cash)
- ✅ 赛道分档 (T-sector)

**14 个 v12 新增 PushKind 路径未触发**：
- T-01 账户模式: 0 次
- T-02 数据状态: 0 次
- T-03 持仓建议: 0 次
- T-04 持仓紧急风险: 0 次
- T-05 做T建议: 0 次
- T-06 做T禁止: 0 次
- T-07 候选触发: 0 次
- T-08 候选失效: 0 次
- T-09 禁止操作: 0 次
- T-10 虚拟盘成交: 0 次
- T-11 竞价异动: 0 次 (周日)
- T-12 尾盘决策: 0 次 (周日)
- R-03~R-08 盘后: 0 次 (周日)

---

## 二、发现的关键 Bug

### Bug A — 数据库迁移未执行 (HIGH)

**日志**：
```
[08:53:22 WARN] [净值快照] 获取持仓失败: no such column: stock_position.chain_name
```

**根因**：
- migration `v12-p0-paper-and-adjust/up.sql` 用 `ALTER TABLE stock_position ADD COLUMN chain_name` 加列
- `src/database/mod.rs::run_migrations()` 是**硬编码 SQL**，**不读取 migrations/ 目录里的 SQL 文件**
- Diesel 端 `src/schema.rs` 加了 `chain_name` 列声明 → Rust 编译期认为该列存在
- SQLite 端 `stock_position` 表**实际没有该列** → 运行时 `SELECT stock_position.chain_name` 报错

**影响**：
- `account_mode_log` 写入虽不依赖 chain_name (建表用 raw SQL) ✅ OK
- `paper_trades` 写入同理 OK
- **但**: 所有读 `stock_position.chain_name` 的路径 (含 `query_chain_held_count`) 在生产路径会失败
- **业务影响**: BR-015 集中度检查失效 → 操作员能超限建仓

**验证代码定位**：
- `src/database/mod.rs::run_migrations()` — 硬编码 SQL，不读 migrations/*.sql
- `src/schema.rs:75` — Diesel 声明 `chain_name: Nullable<Text>` 但 SQLite 实际无此列

**修复方案**：
1. **方案 A (推荐)**: `run_migrations()` 加 `ALTER TABLE stock_position ADD COLUMN chain_name TEXT DEFAULT '其他'` (idempotent 包 try/catch SQLite 1.06 的 "duplicate column" 错误). 同步处理 paper_trades/execution_tracking/position_adjustments/account_mode_log 四张表的 CREATE IF NOT EXISTS.
2. **方案 B (彻底)**: 引入 `diesel_migrations` crate, 自动跑 migrations/ 目录. 长期方案.

---

### Bug B — 主循环未挂载 orchestrator (HIGH)

**现象**：v12 MVP-1/2/3/4/5 的所有 orchestrator (`push_account_mode_change`, `push_data_mode_change`, `push_holding_plan_recommendation` 等) 在 monitor `--test` 跑中没有触发任何一次。

**根因**：
- `src/bin/monitor/main.rs:1428` 的 `evaluate_account_mode_hook()` 存在但**仅在 `monitor_loop()` 路径里调用**
- `monitor --test` 走 `run_test_scan()` (line 468)，不进入 monitor_loop
- `run_test_scan()` 没有调任何 v12 orchestrator

**影响**：
- 14 个 v12 新 PushKind 的模板 + 治理已实现 (push_templates 单测 45 个全绿)
- 但生产路径 (包括 `monitor --test` 冒烟) **没有任何代码触发它们**
- MVP-1~5 的"实际推送"价值 = 0
- v12-dev-plan §15 "每 PR 固定流程" 第 1 步 "涉及新规则 → 先登记 BR" 完成, 但**第 7 步 "测试验证" + 主循环挂载未做**

**修复方案**：
1. **MVP-0 整合 PR** (下一步): 在 `run_test_scan()` 和 `monitor_loop()` 中按时间节奏挂载 orchestrator:
   - 09:25: `push_data_mode_change()` (T-02)
   - 09:30/13:00/14:30: `push_holding_plan_recommendation()` × N (T-03)
   - 09:35/11:30/14:00: `push_t0_advice/forbid()` (T-05/T-06)
   - 盘中紧急: `push_holding_emergency()` (T-04)
   - 19:00/21:00: 盘后 R-01~R-08 推送

---

### Bug C — API Key 几乎全部失败 (MEDIUM)

**日志**：
```
[Bocha] 搜索失败: 余额不足 (HTTP 403)
[SerpAPI] HTTP 429 Too Many Requests
[Tavily] HTTP 432 This request exceeds your plan's set usage limit
```

**频率**: 全部 4 个 topic query (今日 A股/科技/科创板/新能源/产业链) 都触发这 3 个 API 的失败

**影响**：
- 主题新闻流降级为"东方财富搜索 + 新浪快讯 + 华尔街见闻" 三源
- 但东方财富搜索每次只返 3 条, 触发频率 6 query × 3 条 = 18 条/扫描, **信息密度低**
- 候选 state_state.shadow_rank_hits → `A=0 B=0 C=0 Drop=0` (无任何 A 档)

**修复方案**：
1. **临时**: 关闭 Bocha/Tavily (余额不足) + SerpAPI (429), 只用东方财富 + 巨潮 (虽返 0)
2. **长期**: 申请新 API Key 或换 SerpAPI tier
3. **优化**: topic query 合并 (从 6 个 → 2-3 个), 减少 API 调用频次

---

### Bug D — Scanner 加载 0 只自选股 (MEDIUM)

**日志**：
```
[Scanner] 加载 0 只自选股
[测试] Scanner: DQ: total=0 passed=0 (100%) stale=0 halt=0 jump=0 ex_rights=0 price=0 个目标
```

**根因**：
- `.env` 中 `STOCK_LIST` 应配自选股代码 (逗号分隔), 当前**未配或配错**
- 没有自选股 → 没有 ticker-level 数据 → Scanner/DQ 都返回 0
- 没有 ticker → T-03 持仓建议没有标的 → T-03 永远不会触发

**修复方案**：
- 验证 `.env::STOCK_LIST` 配置 (测试时用 `STOCK_LIST=000001,600000,688001`)
- 或在 `config/` 加 `watchlist.toml` 配置 (已有 monitor.toml 可参考)

---

### Bug E — Adapter 推送被默认降级 (INFO)

**日志**：
```
[08:54:52 WARN] [PUSH_GOVERNOR] 降级日志 (kind=产业链, PUSH_VERBOSE 默认精简):
当前产业链信号可信度不足（已降级观察）
[08:54:57 WARN] [PUSH_GOVERNOR] 降级日志 (kind=赛道分档, PUSH_VERBOSE 默认精简):
```

**说明**：
- 这是**设计预期行为**: 11 个低置信度 PushKind 默认降级到 log (PUSH_VERBOSE=false 时)
- 产业链/赛道分档 当前数据不足 → 降级 → 不推 → **正确**
- 不需要修复

---

### Bug F — 多个 push_governor env-var 竞争 (INFO)

**根因**: 已知 (Q-09 in uncertainty-notes), `V10_DRY_RUN_PUSH` / `PUSH_VERBOSE` 在 cargo test 并行时与其他测试共享. `--test` 单进程模式不触发, 但 CI 跑 `cargo test --bin monitor push_templates::tests::e2e_*` 必须 `--test-threads=1`.

---

## 三、修复优先级

| 优先级 | Bug | 修复 PR |
|---|---|---|
| 🔴 P0 | **A** 数据库迁移未执行 | "MVP-0 整合: DB 迁移补全" |
| 🔴 P0 | **B** 主循环未挂载 orchestrator | "MVP-0 整合: 主循环挂 v12 orchestrator" |
| 🟡 P1 | **D** STOCK_LIST 配置 | 文档 + 验证脚本 |
| 🟡 P1 | **C** API Key 失败 | 申请新 Key / 降级策略 |
| 🟢 P2 | E / F | 已知, 文档化即可 |

---

## 四、推荐的 MVP-0 整合 PR 范围

按 v12-dev-plan-2026-07-05.md "每 PR 固定流程" 第 7 步 "测试验证" 要求:

1. **DB 迁移修复**: `run_migrations()` 加 v12 三张表 CREATE IF NOT EXISTS + stock_position ADD COLUMN chain_name idempotent
2. **主循环挂载**: `monitor_loop()` 在行情刷新/交易记录变更后调:
   - `evaluate_account_mode_hook()` (已有, 未调用)
   - `evaluate_data_mode_hook()` (新增)
   - `evaluate_holding_plan_hook()` (新增, 调 T-03)
   - `evaluate_emergency_hook()` (新增, 调 T-04)
   - `evaluate_t0_hook()` (新增, 调 T-05/T-06)
3. **run_test_scan() 同步**: `--test` 路径走同样的 hook 集合
4. **watchlist**: 验证 .env::STOCK_LIST 或 fallback 默认 9 只
5. **E2E 验收**: 跑 `cargo run --bin monitor -- --test` 确认 T-01/T-02/T-03 至少各触发 1 次

预计工时: 8-12h (单 PR)

---

## 五、附录: 完整事件时间线 (08:53:21 ~ 08:54:57)

```
08:53:21 INFO  实盘监控启动 | 非交易日 | 当前: 休市 | 模式: 测试
08:53:21 INFO  [测试] 跳过交易日历，立即执行连通性检查...
08:53:21 INFO  [测试] Scanner: DQ: total=0 ... (无自选股)
08:53:21 INFO  [测试] Detector: 3 条信号
08:53:21 INFO  [测试] 状态机: 过滤后 3 条告警, 已归档到 reports/alerts/
08:53:21 INFO  [测试] 风控: 市场=Structural 止损=9.40 仓位上限=13333
08:53:21 INFO  [测试] 信号融合: 共振=73 建议=强买入（多信号共振）
08:53:21 INFO  [测试] 盘前 Checklist 生成完成 (0 只持仓)
08:53:21 INFO  [测试] 近7天预测命中率: 23%
08:53:21 INFO  [测试] 自适应权重: ... | Shadow: test_vol_burst: 1/1 (100%)
08:53:21 INFO  [V10_DRY_RUN_PUSH] 跳过飞书推送, 内容预览: 📊 告警聚合摘要
08:53:22 INFO  [测试] 复盘报告: 📊 交易复盘 2026-07-05 / 🎯 胜率: 100% (1/1)
08:53:22 WARN  [净值快照] 获取持仓失败: no such column: stock_position.chain_name   ←── Bug A
08:53:23 WARN  [sina_flash] lid 拉取失败: 新浪快讯 lid=2516/2509 解析失败
08:53:35-08:54:42 WARN  [topic] 科创板/巨潮/沪深交易所/SerpAPI/Bocha/Tavily 全部失败 ←── Bug C
08:54:52 INFO  [测试] 新闻Ranker: 📰 新闻Ranker · A=0 B=0 C=0 Drop=0
08:54:52 WARN  [PUSH_GOVERNOR] 降级日志 (kind=产业链)
08:54:57 INFO  [测试] 排除检查: 1 项命中 / 风控检查: 0 项超标
08:54:57 INFO  [V10_DRY_RUN_PUSH] 内容预览: 🛑 排除板块命中 + 💰 现金预警
08:54:57 WARN  [PUSH_GOVERNOR] 降级日志 (kind=赛道分档)
```

---

## 六、建议的后续步骤

1. **立即**: 修复 Bug A (DB 迁移) — `cargo run --bin monitor` 应立即看到 chain_name 相关错误消失
2. **接着**: 修复 Bug B (主循环挂载) — `monitor_loop()` 调度 5 个 v12 orchestrator
3. **次**: 修复 Bug C/D (API Key + STOCK_LIST)
4. **再次**: 跑 `cargo run --bin monitor -- --test` 验证 T-01~T-12 各触发 ≥1 次
5. **最后**: 跑实盘 (周一 09:25) 验证 `push_governor` 真发飞书, 确认 §14 模板逐字符对齐

EOF