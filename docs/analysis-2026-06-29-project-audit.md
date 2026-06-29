# A 股量化系统 — 多维度问题审计

> **审计日期**:2026-06-29
> **审计范围**:`/Users/zhangzhen/Desktop/Quant/stock_analysis` 全项目
> **审计方法**:源码静态扫描 + 既有架构评审/量化机构评审/性能优化报告交叉验证
> **本文档定位**:独立审计,不修改任何代码;以 `docs/PROJECT_DESIGN-2026-06-28.md` 的"修复后 8/10"为参照基线,揭示与该评分的偏差及系统真实健康度

---

## 0. 项目快照

| 指标 | 数值 |
|---|---|
| 代码规模 | 61,024 行 Rust(`find -name "*.rs" -exec wc -l`) |
| 源文件数 | ~165 个,32+ 模块 |
| 三大入口 | `stock_analysis`(主 CLI)+ `lhb_query` + `rsi_optimize` + `bin/monitor` |
| 三大可执行文件总行数 | `bin/monitor/main.rs:1934` + `notify.rs:1056` + `market_data.rs:331` ≈ 3321 行 |
| 核心编排器 | `src/pipeline/mod.rs`(1765 行)+ `src/pipeline/chain_analysis.rs`(1839 行) |
| 测试 | 454 个,串行 100% 通过 |
| `target/` 体积 | **30 GB**(Cargo 全量构建产物) |
| `data/` + `reports/` | 85M + 263M = 348 MB |
| 自我评分 | 修复后 8/10(`docs/PROJECT_DESIGN-2026-06-28.md` §0) |
| 外部架构评审 | **2.3/5**(`docs/reviews/ARCHITECTURE_REVIEW-2026-06-25.md`) |
| 外部量化机构评审 | P0 阻塞 4 项,核心是"风控执行断层" + "AI 评分系统性反向" |

---

## 1. 架构问题(最严重)

### 1.1 `bin/monitor/main.rs` 是"编排器上帝"

**证据**:`bin/monitor/` 三文件共 3321 行,`main.rs` 单文件 1934 行。文件内同时承担:
- `monitor_loop` 行情扫描主循环
- `news_monitor_loop` 独立新闻循环
- `run_test_scan` / `run_review_only` 端到端测试
- `run_factor_ic_analysis` / `run_stock_screener` 选股算法
- `push_sector_leaders` / `push_market_fund_top10` 等 10+ 推送分支
- 虚拟观察仓读写、Token 缓存、守护进程拉起
- 6 个跨窗口的 `static`(`MAGICLAW_DAEMON_BOOT_LOCK` 等)
- 嵌入 `run_test_scan` 内长 ~550 行的端到端连通性串接(main.rs:401-600)

**根因**:binary 兼具 **入口** + **业务编排** + **状态管理** 三重职责,违反单一职责。

**问题**:
- 单文件 1934 行,认知成本极高
- `main.rs` 通过 `crate::notify::*` 调入 `MAGICLAW_*` 等子模块又反向依赖
- `static LAST_DATE: Mutex<Option<String>>` 这种"主循环内置"的可变状态,无法在不动 main.rs 的情况下单独测试
- 6 个 `static` 全局状态(`Lazy`/`Atomic`/`Mutex`)使得 `bin/monitor` 进程内**无法并行运行第二个实例**(例如同时跑 `--review` 和 `--test`)

### 1.2 多 Runtime 嵌套(并发架构 P0)

`docs/reviews/ARCHITECTURE_REVIEW-2026-06-25.md` §5.1 明确指出:

- `main.rs` 是 `#[tokio::main]`,但内部 `modes.rs` / `bootstrap.rs` / `chain_analysis` 等多处又 `Runtime::new()`
- `Runtime::new` / `block_on` 调用 **26 处**,包括:
  - `bin/rsi_optimize.rs:290`、`bin/boll_macd_backtest.rs:129` 用 `Runtime::new()?`
  - `data_provider/` 7 个文件用 `Handle::try_current().block_on(...)`,模式高度重复(`money_flow.rs:499/506/527/534`、`intraday_kline.rs:134/141`、`valuation_history.rs:164/168`、`industry.rs:259/263`、`consensus.rs:201/205`、`eastmoney_provider.rs:354/373`)
  - `opportunity/chain_mapper.rs:164` 在 `spawn_blocking` 内 `block_on`(`market_analyzer/mod.rs:110` 注释明确警告"不要在 async 上下文中用 `block_in_place + block_on`")
- 8 核机器 5 套 Runtime = **40 个 worker 线程**

**根因**:数据层"同步阻塞函数 + 异步同名函数"双轨。`DataFetcherManager` 既暴露 blocking API 又暴露 async API,导致每个调用方都得手写 `Handle::try_current().block_on(...)` 桥接。

**影响**:
- 严重资源浪费(40 个空闲 worker)
- `block_in_place` 容易把 tokio worker 线程**永久占住**
- 复用性极差:同一份代码在 async 调用链内要靠 `try_current` 自适应,在同步链内要自建 Runtime

### 1.3 0 个 channel(无事件总线)

**证据**:
- `mpsc` / `broadcast` / `watch` 在全代码库使用 = **0**(项目自有评估,见架构评审 F6)
- 整个系统用 **全局单例** + **函数直调** 通信
- `monitor_loop` 与 `news_monitor_loop` 两个**核心循环共享状态靠全局**,不通过消息
- 机会扫描用 `tokio::spawn` 启动后 `JoinHandle` 被丢弃(评审 F5.3 指出 fire-and-forget)

**后果**:
- 任何新消费者(审计、行情持久化)都要改生产者代码
- 无法做单元隔离,只能靠 e2e
- 背压完全靠 `buffer_unordered(N)` 兜底,推送无任何速率限制(评审 F5.4)

### 1.4 `AnalysisResult` 130 字段上帝结构体

`src/pipeline/mod.rs:48-178` 单一 struct 同时承担:
- 原始数据(价格/市值)
- 技术信号(MA 排列/买卖点)
- AI 文本(`analysis_summary` 等 Option 字段)
- 评分(`sentiment_score`、`ranking_score`)
- 否决结果(`vetoed` / `veto_reasons`)
- 回测元数据(`backtest_summary`)
- 产业链(`chain_markdown`)

**问题**:CQRS 完全缺失。改一个字段影响所有下游(通知、报告、持久化、回测、推送)。`v3.4` 文档承认"完整拆需 50+ 访问点"。

### 1.5 上下文边界普遍泄漏

| 模块 | 深度导入 | 评审判定 |
|---|---|---|
| `opportunity/mod.rs` | 8+ 模块深引(`assess_quality`、`fetch_financials` 等) | F2 上帝消费者 |
| `deep_analyzer.rs` | 10 项深度导入 `agent::*` 内部 | F3 缺乏 `AgentFacade` |
| `pipeline/mod.rs:526` | 内部 `use crate::strategy::BollMacdAction` | F3 函数体内藏 use |
| `signal/` | **完全死代码** | F1 整个上下文无消费方 |

`signal/` 上下文是评审 F1 标记的"死代码"——文档承诺 8 上下文,实际 7 + 1 死模块。

### 1.6 文件大小 + 复杂度集中度极高

```
src/bin/monitor/main.rs            1934
src/pipeline/chain_analysis.rs     1839
src/strategy/core.rs               1765
src/pipeline/mod.rs                1765
src/search_service/service.rs      1565
src/opportunity/mod.rs             1362
src/strategy/rsi/standard.rs       1194
src/pipeline/backtest_runner.rs    1088
src/bin/monitor/notify.rs          1056
```

`pipeline/mod.rs` 单文件 1765 行 + `chain_analysis.rs` 1839 行 共同承担 6 阶段流水线的全流程,远超"模块"范畴。

---

## 2. 性能问题(实际可达 + 潜在)

### 2.1 HTTP 客户端每请求重建(OPTIMIZATION_REPORT P0-1)

**证据**:
- `reqwest::Client::builder()` 在非测试代码中出现 **28 次**
- `bin/monitor/market_data.rs:46-48` `fetch_eastmoney_quotes` 每次调用都 `reqwest::blocking::Client::builder().timeout(...).build()`
- `OPTIMIZATION_REPORT-2026-06-22.md` P0-1 指出 3 处 `direct_client()` 仍绕过共享 client

**影响**:
- 每次新建 client = TCP 1 RTT + TLS 2 RTT + DNS + 内部 Arc 分配
- 200 只股票批量查询 = 额外 1000 次握手

### 2.2 `std::sync::Mutex` 阻塞 async worker(OPTIMIZATION_REPORT P0-2)

- `analyzer/mod.rs:456` `static ANALYZER: OnceCell<std::sync::Mutex<GeminiAnalyzer>>` 用 `std::sync::Mutex` 保护 **跨 LLM API 调用**(可阻塞 worker 数秒)
- 同类问题在 `search_service/service.rs:340`、`monitor/adaptive.rs:117`、`monitor/rate_budget.rs:15`、`data_provider/industry.rs:12`

**影响**:持有 LLM 锁期间,所有依赖该 Analyzer 的分析请求串行排队。`buffer_unordered` 退化为串行。

### 2.3 数据源 4 路串行(OPTIMIZATION_REPORT P0-4)

`DataFetcherManager::get_daily_data` 中 financials / valuation / consensus / industry **严格串行**:
```rust
let fin = financials::fetch_with_fallback_blocking(...);
let vh = valuation_history::fetch_blocking(...);
let cs = consensus::fetch_blocking(...);
let ib = industry::fetch_blocking(...);
```

**问题**:四路相互独立,应 `tokio::join!`。200 只股票浪费 ~10 分钟。

### 2.4 飞书通知每次重新编译 12 个正则(OPTIMIZATION_REPORT P0-3)

`notification/feishu.rs:109-183` `markdown_to_html` 每次发通知重建 12 个 `Regex::new()`。搜索代码 `Regex` / `Regex::new` 出现 21 处,大部分应该 `Lazy` 静态化。

### 2.5 KDJ / SMA 朴素 O(n²) / O(n·K)

- `indicators/kdj.rs:35-46`、`indicators/skdj.rs:42-52` 滑动窗口朴素 O(n²)
- `trend_analyzer.rs:340-377` MA5/10/20/60 每根 K 线重算 → 250 根 × 95 次 = 23750 次浮点
- `strategy/rsi/standard.rs:1021`、`bollinger_zscore.rs:355` 等 4 处为排序而全量 clone K 线

### 2.6 全局正则 / 字符串重复分配

- `Lazy::new` 出现 28 处(正向),但仍有 `Regex::new` 21 处无 Lazy 化
- 28 处 `reqwest::Client::builder` 多数是 hot path 重复创建

### 2.7 构建体积失控

- `target/` **30 GB**(典型 Rust debug 模式 + plotters + polars + 14 个 dylib 重复编译)
- `Cargo.toml` 没有任何 `[profile.dev]` / `[profile.release]` 优化配置(`opt-level` / `lto` / `incremental` 均未设置)

---

## 3. 量化方案问题(最致命)

### 3.1 AI 评分系统性反向(quant_institutional_review §3.4)

机构评审发现的**最严重数据问题**:

| AI 评分区间 | 实际胜率 |
|---|---|
| ≥ 80 | **0%**(全亏) |
| 70-79 | 13.9% |
| 60-69 | 19.2% |
| < 40 | **~50%,T5 均涨 +2.40%** |

**现状**:已 bypass AI 改用布林+MACD 共振(B 方案)
**问题**:
- `sentiment_score` 仍参与**排序展示**和**回测选股**,系统信号方向性仍有问题
- 评审 P0-3 要求"对 sentiment_score 各因子做 IC/IR 分析"——未做
- 没有解释"为什么 AI 给 80 分的票反而全亏"(是 prompt 错了?是 LLM 推理能力差?是 ground truth 标错了?)

### 3.2 风控定义完备但执行路径断裂(quant_institutional_review §3.1)

| 组件 | 位置 | 实际使用? |
|---|---|---|
| `HardLimits` | `risk/limits.rs` | 仅 monitor 告警 |
| `StopLoss` (ATR) | `monitor/risk.rs` | ❌ `position_tracker` 未用 |
| `PositionSizer` | `monitor/risk.rs` | ❌ `position_tracker` 未用 |
| 四大铁律(硬编码) | `pipeline/position_tracker.rs:73-93` | ✅ 实际在用 |

`pipeline/mod.rs:686-740` 三段核心拦截器**全部被注释**(包括乖离率、空头排列、主力出逃、基本面恶化拦截)。注释自承"解决系统精神分裂问题"。

**根因**:`monitor/risk.rs` 早写但 `position_tracker.rs` 后写,后者用四大铁律(8% 止损 / 20% 止盈 / 跌破 5 日线 / 14 天时间止损)绕开了原 risk 模块。**两个风控体系并存,新体系不接入旧体系**。

### 3.3 多因子回测存在时点错位(准 look-ahead)

`pipeline/backtest_runner.rs:265-329` 回测期内**复用同一批因子分数**,无法反映因子随时间变化。机构评审 §2.2 指出"正确做法应每个再平衡日仅使用截至当日可得数据重算因子"。

**P0.5 修复**声称用 `factor_snapshot` 表 + `get_as_of` 解决,需验证是否**真实接入**所有回测路径(而非仅文档承诺)。

### 3.4 v9 流水线"准实盘"风险

`docs/architecture-v9.1-...` 设计的 7 段流水线 + `dual_score`,以及 `KNOWN_BUGS B-002`(科创板日报缺失)显示:
- 半导体 / 新能源 / AI 等行业的事件驱动信号源**永远拿不到**
- `chain_score` 缺数据,`event_risk_score` 封顶 70(损失 30% 信号)
- v9 AI 调用接入是 P3 推迟项,实际只有 rules-only 版本可跑

**结论**:v9 当前是"**设计完备、实现过半、信号源不足**"的状态。`trade_signal_score` 字段被声明为"无数据 = 0/100"二元化,但**这意味着系统永远把 trade_signal 视为 0**,实盘路径触发条件(`≥75 且胜率≥60% 且样本≥200`)**永远不满足**。

### 3.5 `HybridStrategy` 是空壳(strategy §1.1)

`strategy/mod.rs:165` `HybridStrategy` 只"各策略独立运行,分别返回结果",**从未实现**:
- 无多策略投票
- 无 Kelly 仓位
- 无组合层面净值加权
- 各策略权重固定为构造期传入,无动态调整

P2.4 HybridStrategy 真实加权是 P2 推迟项。

### 3.6 基本面因子仅 5 个,且多因子无 Walk-Forward

`MultiFactorStrategy::StockFactors` 5 因子(`market_cap` / `ROE` / `PE` / `PB` / `turnover_rate`),缺:
- 动量(3 / 6 / 12 月)
- 波动率 / Beta
- 质量(应计利润、杠杆)
- 成长(盈利超预期)
- 情绪(北向、融资融券)

且多因子策略**未实施 Walk-Forward**(评审 §2.1)——这恰恰是最易过拟合的策略。

### 3.7 walk-forward 仍然有 overfit 风险

- 缺 PBO(Backtest Overfit 概率)—— 推迟项
- 缺 DSR(Deflated Sharpe Ratio)、CPCV(Combinatorial Purged CV)
- 缺压力测试(2015 股灾、2018 去杠杆、2020 疫情、2022 封城)
- 缺 Monte Carlo、VaR / CVaR

### 3.8 信号时效与"过期不沿用"覆盖不全

`monitor/data_quality.rs` 有 `FreshnessConfig`(5s 行情 / 30s 持仓 / 86400s 净值),但:
- 评估代码 `validate_quote_freshness` / `validate_position_freshness` / `validate_nav_freshness` 都在 `bin/monitor/main.rs`(1805-1890 行),**与 `monitor/data_quality.rs` 重复定义**
- `bin/monitor/market_data.rs:55-59` 用 `validate_quote_freshness(update_time, ...)` 过滤每条数据,但 `push2delay` 主机延迟大,可能**整批被静默丢弃**而不告警

### 3.9 实盘 / 回测鸿沟

评审 §5.1 指出现状:
- 订单执行 = 直接记录 DB,**无券商 API**
- 成交价 = 当日收盘价,**无滑点模型**(P1.4 动态 σ/ADV 接入是 P3 推迟)
- T+1 = 文字提醒,**无下单拦截**
- 资金 = 环境变量,**无实时查询**
- 持仓对账 = 无

**结论**:这**不是实盘系统,是 paper trading 模拟器**。文档承诺 8/10,实际距实盘至少 6-12 个月工程量。

---

## 4. 代码质量问题

### 4.1 错误处理 100% anyhow / 0% thiserror

- `thiserror` 全文出现 **3 次**(可能仅在 `use` 中)
- 全部 `anyhow::Result<T>`,调用者无法区分"key 用尽"和"HTTP 超时"
- 跨层错误消息字符串泄露

### 4.2 284 处非测试 `.unwrap()` / `.expect()`

评审数据 56 处(可能仅统计了显式 `unwrap()`),本次扫描搜到 284 处 `unwrap()` / `expect()`,`config.rs` 单文件就有 10 处,搜索 provider 12 处。**任何一次 RwLock 中毒都导致进程 panic**(对实盘 = 灾难)。

### 4.3 56 种可变状态散落 8 种机制

评审 F7 量化:
- 6 个全局 static 单例
- 7 个全局 RwLock
- 3 个全局 tokio sync
- ~15 处栈局部可变
- 5 处文件系统状态
- 1 个 AtomicBool
- 4 处内存去重
- 4 处内存计时器

**无可观测的 `MonitorState`**,无法在不动 main.rs 的前提下加新窗口 / 新状态。

### 4.4 依赖硬编码 + 三层配置散落

- `.env`(API key、`STOCK_LIST`)
- `config/*.toml`(SIGHUP 热加载 5 个)
- 代码内 `const` / `Default::default()`(多处)

**问题**:
- B-005 显示 `cross_source_count` / `winrate_pct` 等 const **未完全迁移**到 `risk.toml`
- 调整任一参数需要改代码 + 重编译 + 重新发布
- 无统一配置中心

### 4.5 缺乏 CI / CD

- `.github/workflows` 目录**存在但几乎为空**
- 无 pre-commit 钩子
- 无自动构建 / 发布

### 4.6 测试覆盖不均

| 模块 | 测试数 | 风险 |
|---|---|---|
| `position_tracker.rs` | 30(已补) | ✅ |
| `pipeline/mod.rs` | 6(仅 `normalize_ai_sections`) | 🟡 |
| `risk/limits.rs` | 5 | 🟡 |
| `risk/stop_loss.rs` | 3 | 🟡 |
| `review/equity.rs` | 2 | 🟠 |
| `review/journal.rs` | 3 | 🟠 |
| `pipeline/backtest_runner.rs` | 5 | 🟡 |
| `bin/monitor/main.rs`(1934 行) | **0** | 🔴 |

**bin/monitor 1934 行 0 单测**是**最大的覆盖盲区**。

### 4.7 god-struct 拆分的"半完成"债务

- `AnalysisResult` 130 字段已用注释分组(v3.4)但**未真拆**
- `HybridStrategy` 留"未来扩展"但**未实现**
- 多个 v9 子模块只有 rules-only,**AI 真接入是 P3 推迟项**
- B-003 显示 `live_rolling_sharpe` + `strategy_correlation_matrix` **函数写好但没调用方**(文档承诺债)

### 4.8 文档 / 代码同步债

- `src/notification/mod.rs:7` (修复 v9.3 codex review C-7: 之前引用 `docs/notification/mod.rs:7` 是幻象文件) 文档说"5 渠道",实际 `src/notification/config.rs::NotificationChannel` enum 有 10 个 channel (B-011)
- v9 流水线设计已落地文档,**主实施未动**
- 文档评分 8/10,**实际架构评审 2.3/5**——自我评分严重失真

---

## 5. 数据合规 / 风险红线问题

### 5.1 测试账户隔离机制

`AGENTS.md §2.5` 要求:
- `TEST_CODE` 前缀区分
- 生产拒绝 `TEST_CODE`,测试拒绝真实标的
- 物理隔离

**未验证**:未在 `bin/monitor/main.rs` 看到 `STOCK_ENV_MODE` 检查逻辑,需进一步审计才能确认是否真做硬隔离(可能仅约定)。

### 5.2 过期数据"显式报错"承诺的执行

`bin/monitor/main.rs:1805-1890` 三处 `validate_*_freshness` 函数,`market_data.rs:55-59` 用 `validate_quote_freshness` 过滤数据。但 `push2delay.eastmoney.com` 主机推送延迟常 > 5 秒,可能**整批过滤掉**导致监控静默——这与"显式报错"的红线精神有冲突。

### 5.3 "数据红线" vs "实际 0 panic" 承诺

- 单测 454 串行 100% 通过
- e2e 15 飞书推送 exit=0
- **但是**:284 处 `unwrap()` 任意一处中毒 = 进程 panic = 监控中断
- 5 大数据源(通达信 / 东财 / 腾讯 / 巨潮 / 沪深)任一失效 24h 无重试补偿

### 5.4 假实现禁令的可执行性

`AGENTS.md §2.8` 要求 `verify` / `save` / `notify` / `push` / `sync` / `update_result` / `reconcile` 类函数必须真实操作。**人工审查无法保证**,`tools/compliance/lib/check_fake_impl.sh` 仅拦截 `update_.*result.*0\.0.*false` 模式,覆盖面极窄。

### 5.5 0 mock 残留的可执行性

- 测试用 `TEST_CODE` 前缀 + SQLite 内存库
- 但 `data_provider/` 用 `Handle::try_current().block_on(...)` 在 `spawn_blocking` 内异步调用,测试环境**难以 mock**
- 全局 `static ANALYZER: OnceCell<...>` 单例**无法注入 mock**——无 DI 容器

### 5.6 单笔交易拦截边界

`AGENTS.md §2.6` 要求:
- 单笔 ≤ 账户可用资金,≤ 100 万
- 单笔 ≥ 50 万需二次确认
- 100 股整数倍 + 涨跌停区间

`portfolio/store.rs` 未审计;但 `bin/monitor` **无任何下单路径**(评审 §5.1),意味着此红线**目前无可执行的下游**——是"未实现"而非"实现但有 bug"。

---

## 6. 项目管理 / 流程问题

### 6.1 自我评分 8/10 vs 外部评审 2.3/5 的严重失真

| 维度 | 内部评分 | 外部评分 |
|---|---|---|
| 整体 | 8/10(可生产) | 2.3/5(系统债) |
| 边界 | (未评) | 2/5(普遍泄漏) |
| 数据流 | (未评) | 2/5(无事件总线) |
| 并发 | (未评) | 2/5(多 Runtime) |

### 6.2 P0 → P3 优先级倒挂(部分)

- P0 列 5 项数据真实性修复
- P1 列 9 项**回测严谨性**修复
- 17 项 P2 / P3 **推迟项**中有"v9 业务实施 (1-2 周)"——这是核心机会发现功能,但被排到 P3

### 6.3 文档版本膨胀

`docs/architecture/` 目录 10 个版本(`v2/v3/v4/v5/v5.1/v6/v7/v8/v9/P0 风控`),`docs/architecture-v9.1-...` 又是单独文件。**新人 onboarding 极难**(需要读完 10+ 个版本演进史才能理解当前架构)。

### 6.4 "修复成果"叙事掩盖了系统债

`README.md` 大段渲染 28 项修复,但:
- 同期承认 17 项推迟
- 实际架构评审 2.3/5
- 量化机构评审 P0 阻塞 4 项
- `signal/` 整个上下文是死代码
- `HybridStrategy` 是空壳

---

## 7. 核心问题 Top 10(按 ROI 排序)

| # | 问题 | 影响范围 | 估时 | 紧急度 |
|---|---|---|---|---|
| 1 | `signal/` 死代码删除 / 或接入 Opportunity | 减认知负担 | 半天 | 低 |
| 2 | AI 评分反向根因分析(sentiment IC/IR) | 决定 AI 模块去留 | 1 周 | P0 |
| 3 | `pipeline/mod.rs` 拆分(1765 行 → 5 个子模块) | 减认知负担 + 单元可测 | 1 周 | P1 |
| 4 | `bin/monitor/main.rs` 拆分(1934 → 6 个文件) | 同上 | 1 周 | P1 |
| 5 | 26 处 `block_on` 收敛为统一桥接 trait | 消灭多 Runtime | 2 天 | P0 |
| 6 | `std::sync::Mutex` → `tokio::sync::Mutex`(5 处) | 解决 worker 阻塞 | 1 天 | P0 |
| 7 | HTTP client 共享 + 4 路 `join!`(data_provider) | 性能 5-10x | 1 天 | P0 |
| 8 | `thiserror` 错误枚举(替换 anyhow) | 跨层错误可控 | 2 天 | P1 |
| 9 | 接入 VetoChain 到 `position_tracker`(恢复 686-740) | 修风控执行断层 | 1 天 | **P0 阻塞实盘** |
| 10 | 多因子每日重算 + Walk-Forward | 修真 look-ahead | 1 周 | P0 |

---

## 8. 总结性判断

这个项目**不是"假系统"也不是"实盘系统"**,而是:

- **工程完成度高**: 61K 行、454 测试、434 个细节修复、文档完整度 9/10
- **量化方法学严密度中等**: 关键 look-ahead / 评分方向性 / 因子完备性都有问题
- **架构整洁度低**: 评审 2.3/5,1934 行 main.rs、1765 行 pipeline、130 字段 struct、0 channel、26 处 block_on 是**典型"快速堆叠 + 后期补丁"模式**
- **自我评分严重失真**: 8/10 与 2.3/5 严重脱节
- **实盘可交易性 = 0**: 无券商 API、无实时成交、无持仓对账(评审 §5.1)
- **距离"可生产"还有 1-2 个全职月工程量**:主要是架构债清理 + 风控统一 + 量化方法学补齐(DSR/PBO/Walk-Forward/压力测试)

**最需要警惕的两点**:
1. **AI 评分反向**——如果没做根因分析,这个模块对系统的"解释价值"远大于"决策价值",但仍在污染排序展示
2. **风控注释掉 + 四大铁律硬编码**——这是实盘事故的"火种",一旦切换到 live mode 立即放大风险

**最值得借鉴的两点**:
1. v9.1 `dual_score` 模型(把"风险评估"和"胜率预测"解耦)——这是项目里**唯一有理论支撑**的设计创新
2. AGENTS.md 数据红线 + 受控例外通道——流程治理非常规范,与代码债的对比令人深思

---

## 附:参考文档

- `docs/PROJECT_DESIGN-2026-06-28.md` — 全项目设计文档(内部 8/10 评分基线)
- `docs/architecture-v9.1-opportunity-pipeline-fix-2026-06-28.md` — v9.1 现行架构 + 5 项量化修正
- `docs/KNOWN_BUGS-2026-06-28.md` — 10 个已知 bug(按 P0/P1/P2 排序)
- `docs/reviews/ARCHITECTURE_REVIEW-2026-06-25.md` — 架构评审(外部 2.3/5)
- `docs/reviews/quant_institutional_review-2026-06-21.md` — 量化机构级多维度排查
- `docs/reports/OPTIMIZATION_REPORT-2026-06-22.md` — 性能优化报告(P0-P3 24-task)
