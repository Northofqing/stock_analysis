# v19.14 推送模板重构设计

## 1. 背景与动机

### 1.1 现状 (v19.13)

- **文档**: `docs/architecture/v12-push-templates.md` 定义 18 个推送模板 (T-01~T-12 实盘 + R-01~R-08 盘后) + 14.0 全局横幅协议 + 14.3 治理规则
- **渲染**: `src/bin/monitor/push_templates.rs` 3485 行, **20 个 render 函数已存在** (含 BannerCtx)
- **调用**: `src/bin/monitor/main.rs` 31 处 `push_governor` 调用, **覆盖 22 种 PushKind** (其中 deprecated 全部保留=true)
- **数据通路**: 部分模板填充函数仍 hardcode 0 占位 (R-05 信号复盘 v19.12 部分修, 但 R-06/R-07/R-04/P3 outcome 等仍有问题)

### 1.2 问题 (用户反馈)

1. **AGENTS.md §2.1 违规**: v19.12 之前做T推送对 watchlist 候选票推 (已 v19.13 修)
2. **模板数据不对**: R-05/R-06/P3 等 hardcode 0 占位, R-04 模板显示错误字段
3. **调用路径不全**: T-01/T-02/T-03/T-04/T-05/T-06/T-07/T-08/T-09/T-10/T-11/T-12 中部分未在 monitor_loop 调用, 只在 --test 跑

### 1.3 重构目标

1. **push_templates.rs 严格对齐 v12-push-templates.md**: 字段名/结构/顺序/banner 位置全部按文档来, 不偏差
2. **main.rs 全模板调用路径接通**: 18 模板全部在 monitor_loop 或 news_monitor_loop 真接, 不再只在 --test 跑
3. **填充函数真接 DB/API**: 不再有 hardcode 0, 0 数据显式标注"样本不足" 等合规标识

---

## 2. 设计原则

- **AGENTS.md §2.1 红线**: 真实数据, 不编造, 数据源失败显式报错
- **Karpathy Simplicity First**: 最小改动达到目标, 不重新设计架构
- **Surgical Changes**: 只动相关代码, 不重构无关模块
- **Goal-Driven**: 18 模板全部推送 = 完成定义

---

## 3. 实施步骤

### 3.1 第一阶段: 渲染层对齐文档

**目标**: push_templates.rs 每个 render 函数输出的文本结构与 v12-push-templates.md 模板 100% 一致 (字段名/顺序/banner/条件段)。

**做法**:
1. 在 push_templates.rs 顶部加 `// v19.14: 模板对齐 docs/architecture/v12-push-templates.md §14.x` 注释
2. 对每个 render 函数:
   - 加 `// 模板来源: §14.x 标题` 注释
   - 用 doc-test 或单测验证输出字符串包含模板关键字段
3. **不改 render 函数体** — 如果现状已经覆盖文档字段, 只加注释; 如果有偏差 (例如 banner 缺失/字段名差异), 修

**影响的 render 函数** (20 个):
- 实盘: T-01 AccountMode / T-02 DataMode / T-03 HoldingPlan / T-04 HoldingEvent / T-05 T0Advice / T-06 T0Forbid / T-07 CandidateTriggered / T-08 CandidateInvalidated / T-09 ForbiddenOps / T-10 PaperTrade / T-11 AuctionVolume / T-12 CloseCall
- 盘后: R-01 DailyReport / R-02 ReviewMarket / R-03 IndustryChain / R-04 ReviewLhb / R-05 ReviewSignal / R-06 ReviewFailure / R-07 TomorrowWatch / R-08 EventCalendar

### 3.2 第二阶段: main.rs 调用路径接通

**目标**: 18 模板全部在 monitor_loop 或 news_monitor_loop 真实调用, 不再只在 --test 跑.

**当前状态**:
- --test 路径: 几乎所有模板都跑 (T-01~T-03 演示 + R-01~R-08 全跑)
- monitor_loop 路径: 持仓健康度 / 信号融合 / 涨停产业链 / 龙虎榜 / NewsRanker 等, 缺 T-01~T-10 大量模板
- news_monitor_loop: 公告推送 + NewsRanker (v19.12 加)

**新增调用**:
| 模板 | 在哪个循环 | 触发条件 | PushKind |
|---|---|---|---|
| T-01 AccountMode | monitor_loop | 模式变化时 | AccountMode |
| T-02 DataMode | monitor_loop | 数据降级时 | DataMode |
| T-03 HoldingPlan | monitor_loop | 每 30 分钟/票 | HoldingPlan |
| T-04 HoldingEvent | monitor_loop | 止损触发 | HoldingEvent |
| T-05/T-06 T0Advice | monitor_loop | 持仓股强信号 (v19.13 已加) | T0Advice |
| T-07 CandidateTriggered | news_monitor_loop | 候选 A 档命中 | CandidateTriggered |
| T-09 ForbiddenOps | monitor_loop | 排雷命中 | ForbiddenOps |
| T-11 AuctionVolume | monitor_loop | 09:25 竞价 | AuctionVolume |
| T-12 CloseCall | monitor_loop | 14:45 尾盘 | CloseCall |
| R-01~R-08 | review_loop (15:00 后) | 盘后 | DailyReport/ReviewMarket/... |

**关键约束**:
- 不区分盘前/盘中/盘后 (用户要求: 测试阶段, 全部推送)
- 但**模板本身有时间戳字段**, 实际触发时填当前时间即可

### 3.3 第三阶段: 填充函数真接数据

**目标**: 模板填充函数不再 hardcode 0, 全部从 DB/API 拉真实数据, 0 数据显式标注.

**需要修的填充函数**:
| 模板 | 现状 | 修复 |
|---|---|---|
| R-04 龙虎榜 | 0 数据时"无数据" | 真接 lhb_daily 表 + 东方财富 API fallback (v19.12 部分修) |
| R-05 信号复盘 | hardcode 0 | v19.12 已接 paper_trades/execution_tracking, 0 数据显式标注 |
| R-06 失败归因 | hardcode 0 | 真接 execution_tracking 表 (D+1/T+1 失败样本) |
| R-07 明日观察池 | 0 只 | 真接 candidate_panel + watchlist |
| R-02 盘面 | 0 数据时显式缺失 | v19.12 已显式标注 |
| NewsRanker | shadow only | v19.12 已真接公告派生 + 同链合并 |
| NewsRanker 影子 | 没绑 event_hint | v19.11 已加 __EVENT:XXX hint |

**R-06 是新修**: 从 execution_tracking WHERE actual_change_t1 < 0 或 hit=0 派生失败样本.

### 3.4 第四阶段: 全模板推送测试

**目标**: 验证 --test 路径 18 模板全部触发推送, --review 路径盘后模板全部触发推送, monitor_loop 路径持仓相关模板全部触发推送.

**验收**:
1. `cargo run --bin monitor -- --test` 输出包含 18 条飞书推送 (message_id 各异)
2. `cargo run --bin monitor -- --review` 输出包含 8 条 R 系列推送
3. `cargo run --bin monitor` 持仓股触发时包含 T-04/T-05/T-06
4. cargo test --lib 全部 pass

---

## 4. 风险与缓解

### 4.1 风险: 修改 push_templates.rs 影响现有调用

**缓解**: render 函数签名 (参数) 不变, 只调内部字段顺序/banner 位置. 测试用 `cargo test --lib` 验证.

### 4.2 风险: 18 模板全推会被飞书限流

**缓解**: 测试阶段, 飞书推送速率 ~1msg/分钟, 18 模板单次 --test 不会触发限流. 实时 monitor_loop 已有 SignalStateMachine 冷却 (key=code+PushKind), 不会重复推.

### 4.3 风险: 数据缺失时模板输出异常

**缓解**: 0 数据显式标注 "样本不足 / 表空 / 影子期无样本", 不显示 0 数字 (v19.12 已修).

---

## 5. 不在本设计范围

- ❌ 推送预算控制 (BR daily_important_max): 14.3 治理已记录, 不在 v19.14 实施
- ❌ 候选转正 (MVP-3): 等影子样本 ≥30 笔触发, 不阻塞 18 模板推送
- ❌ paper_trades 写入通路: 这是 PR3-3.5 路径, v19.14 只接通读端
- ❌ 北向资金真实数据源: 当前新浪 API 假成功返 0, 等换东方财富 main API
- ❌ 龙虎榜历史数据: lhb_daily 表空, 等真实 API 抓取积累

---

## 6. 验收清单 (DoD)

- [ ] push_templates.rs 每个 render 函数注释模板来源 (14.x)
- [ ] main.rs 18 个模板调用路径全部接通 (至少 --test 路径)
- [ ] R-06 失败归因填充函数接通 execution_tracking
- [ ] R-07 明日观察池填充函数接通 candidate_panel
- [ ] NewsRanker 模板字段对齐文档
- [ ] cargo test --lib 全部 pass
- [ ] cargo run --bin monitor -- --test 输出 ≥18 条飞书推送
- [ ] cargo run --bin monitor -- --review 输出 ≥8 条 R 系列推送
- [ ] 没有 AGENTS.md §2.1 违规 (无 mock 数据, 无 fake)

---

## 7. 时间估算

| 阶段 | 估计耗时 | commit |
|---|---|---|
| 3.1 渲染层对齐 | 30min | v19.14a |
| 3.2 main.rs 调用路径 | 1h | v19.14b |
| 3.3 填充函数真接数据 | 1h | v19.14c |
| 3.4 测试验收 | 30min | v19.14d |
| **总计** | **3h** | 4 个 commit |