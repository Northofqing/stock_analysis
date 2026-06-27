# A股自选股智能分析与量化回测系统

> **版本**：v6.5+ (2026-06-27)
> **架构**：事件驱动的实盘监控 + 规则化量化系统
> **技术栈**：Rust 2021 · Tokio · SQLite · 多数据源 (RustDX/东方财富/腾讯/同花顺) · 飞书推送
> **总规模**：~55,000 行 Rust，160+ 源文件，30+ 模块

---

## 系统是什么

一个**全栈 A 股分析与量化回测**系统，三大职责：

1. **盘后复盘** (`cargo run --bin monitor -- --review`)：对持仓 + 自选股做 AI 深度研判，自动生成可执行报告
2. **盘中监控** (`cargo run --bin monitor`)：涨跌停/异动/资金流/AI 事件实时检测，飞书推送
3. **量化回测** (`cargo run --bin rsi_optimize`)：多策略回测 + Walk-Forward + 多基准对比

**不是**：单日扫描器 / 短线交易系统 / 预测明日涨跌的模型。

---

## 7 大限界上下文（DDD 边界）

| 上下文 | 目录 | 职责 |
|--------|------|------|
| Portfolio | `src/portfolio/` | 单一来源的持仓 / 交易 / 账本 |
| Market | `src/market_analyzer/` + `src/data_provider/` | 行情 / 公告 / 涨跌停 / 资金流 |
| Signal | `src/signal/` | 统一信号数据结构（`MarketEvent` 标准中间件） |
| Opportunity | `src/opportunity/` | 事件→产业链→候选→0~100 评分 |
| Decision | `src/decision/` | 排除引擎 / 板块分层 / 资金核验 / 龙头识别 |
| Risk | `src/risk/` | 硬仓位 / 止损 / 否决链 / VetoChain |
| Review | `src/review/` + `src/review/falsify.rs`（已删） | 复盘 / 业绩归因 / falsification |

---

## 目录结构（实际代码）

```
src/
├── agent/                  # AI 多智能体分析 (单 agent loop + 多 agent 辩论)
├── analyzer/               # 技术分析 (MA/MACD/RSI/KDJ/量能)
├── app/                    # 顶层应用模式 (回测/复盘/扫描)
├── bin/monitor/            # 主入口 binary
├── breakout/               # v6 放量分析
├── config.rs               # SIGHUP 热加载 toml 配置
├── data_provider/          # 5 个数据源 + 涨跌停计算器
│   ├── eastmoney_provider.rs
│   ├── gtimg_provider.rs
│   ├── rustdx_provider.rs   # 通达信 TCP (主源, 最快)
│   ├── north_flow.rs        # P0.4 北向资金
│   └── limit_status.rs      # 涨跌停价 / ST / 新股
├── database/               # SQLite + migrations
├── decision/               # 排除 / 板块 / 轮动 / 龙头
├── enums.rs
├── errors.rs
├── indicators/             # 独立指标
├── market_analyzer/        # 指数 / 涨跌停 / 板块 / 市场概览
├── models.rs
├── monitor/                # 实时检测 + 事件总线
├── notification/           # 飞书推送
├── opportunity/            # 事件→机会 (v9 设计)
├── pipeline/               # 主分析 pipeline
│   ├── mod.rs              # 多智能体 + 决策 + 推送
│   ├── backtest_runner.rs  # 5 策略回测
│   └── market_regime.rs
├── portfolio/              # 持仓 / 交易
├── position_tracker.rs
├── review/                 # 复盘 / equity / journal
├── risk/                   # 风控
├── search_service/         # 8 个搜索 provider
├── signal/                 # 统一信号
├── strategy/               # 6 个策略 + 回测引擎
│   ├── core.rs             # BacktestEngine + Sharpe/Sortino/Calmar
│   ├── rsi/                # RSI v2 (13 预设)
│   ├── bollinger_zscore.rs
│   ├── multi_factor.rs
│   ├── boll_macd.rs
│   ├── multi_timeframe.rs
│   └── contrarian.rs
└── types.rs

config/                     # toml 配置
├── monitor.toml             # 扫描/告警/风控
├── chain_rules.toml         # 产业链关键词
├── exclusion.toml           # 排除板块
├── announce_keywords.toml   # 公告分类
└── risk.toml                # P3.1 集中风险常量
```

---

## 5 个回测策略

| 策略 | 文件 | 核心信号 |
|------|------|----------|
| **多因子选股** | `strategy/multi_factor.rs` | 市值/ROE/PE/PB/换手率 排序 |
| **布林带+Z-Score** | `strategy/bollinger_zscore.rs` | 20日 BB + 偏离均值 2σ 入场 |
| **RSI v2** | `strategy/rsi/` | 5日 RSI 超卖 + 趋势 + 13 预设可调 |
| **BOLL+MACD** | `strategy/boll_macd.rs` | BB 收窄 + MACD 底背离 |
| **多时间框架** | `strategy/multi_timeframe.rs` | 60min/15min 联动确认 |
| **逆势** | `strategy/contrarian.rs` | 情绪 < 40 + 52周低位 + 企稳 |

`cargo run --bin rsi_optimize` 跑全部策略 + Walk-Forward + 多基准对比。

---

## 启动

### 复盘（推荐先试）

```bash
# 1. 复制环境变量
cp .env.example .env
# 2. 编辑 .env, 填 STOCK_LIST=你的持仓+自选股
# 3. 跑复盘
cargo run --bin monitor -- --review
```

输出：飞书推送 14+ 条消息（市场概览、持仓深度研判、放量分析、优选候选、风控研判）

### 盘中监控

```bash
cargo run --bin monitor
# 默认日历触发 (15:30 收盘后跑一次, 周五 17:00 加 SOP)
# --test  跳过日历, 立即跑
# --review 盘后复盘模式
```

### 策略回测

```bash
cargo run --bin rsi_optimize
# 输出 reports/rsi_optimization_log.md
# 含 13 预设对比 + Walk-Forward + 多基准
```

### 单策略快速回测

```bash
cargo run --bin boll_macd_backtest
# 用 reports/analysis/closed_positions_with_ai.csv 数据
```

---

## 数据源（多源 fallback）

| 类型 | 源 | 优先级 |
|------|----|----|
| 实时 K 线 | 通达信 (RustDX) → 东方财富 → 腾讯 | RustDX 最快 |
| 财务指标 | 腾讯财经 | 补充 PE/PB/换手率/市值 |
| 北向资金 | 东财 kamt API | P0.4 修复 |
| 涨跌停 | 自算 (`limit_status.rs` 严格正则) | P0.1 修复 |
| 搜索 | 东方财富/华尔街见闻/财联社/金十 (免费) + SerpAPI/Bocha/Tavily (付费) | P1.4 排序优化 |
| 公告 | 关键词 + 飞书 AI 抽取 | — |

---

## 关键设计决策（量化分析师视角）

### 1. 数据真实性是 P0（修复已完成）
- 涨跌停字段全链路：原 `is_limit_up: false` 写死 → 现在数据层实时算
- 北向资金：原永远 0 → 现在真接口
- 多因子回测：原用末日截面（look-ahead）→ 现在日频因子快照
- 板块数据：原 `name.len() % 3` 伪随机 → 现在东财 clist/get

### 2. 回测严谨性（P1 已修）
- T+1 结算 + 整百股取整 + 最小佣金 5 元 + 涨跌停拒绝成交
- Sharpe 不再用盘中实时价（KlineData.intraday_price 分离）
- `winrate_score` 二元（0 / 真实胜率），不假装中性 50

### 3. 实盘可交易性
- 板块集中度 (40%) + 现金底限 (15%) 真正执行
- RSI 加仓硬性 ≤ max_position_pct（防隐性杠杆）
- detector 死分支清理

### 4. 风险常量集中（P3.1）
- `config/risk.toml` 单点维护：commission / slippage / stamp_tax / regime thresholds / alert
- 缺 toml 时 const fallback
- SIGHUP 热加载

---

## 已知遗留（P2/P3 部分推迟）

| 项 | 状态 | 备注 |
|----|------|------|
| P2.4 HybridStrategy 加权聚合 | ⏸ 推迟 | 当前各策略独立运行 |
| P2.6 幸存者偏差 | ⏸ 推迟 | 需历史成分股数据 |
| P3.4 god-struct AnalysisResult (130 字段) | ⏸ 推迟 | 拆分影响面大 |
| P3.7 README 重写 | ✅ 本次 | 旧 README 严重脱节 |
| P3.9 live Sharpe | ⏸ 推迟 | 需 ledger 滚动 |
| P3.10 策略相关性 | ⏸ 推迟 | 需多策略对齐 |

---

## 验证清单（开发者必看）

```bash
# 编译
cargo build

# 测试 (423 个, 0 failed)
cargo test --lib -- --test-threads=1

# clippy (有警告, 不阻塞编译)
cargo clippy --lib

# 复盘 (主入口)
cargo run --bin monitor -- --review
```

---

## 文档

- `docs/QUANT_ANALYST_REVIEW.md` — 量化分析师视角的完整评审（25 项问题 + 修复状态）
- `docs/architecture-v9-opportunity-pipeline.md` — 机会发现流水线（事件→评分）
- `docs/architecture-v9.1-opportunity-pipeline-fix.md` — v9 的 5 项量化严谨性修正
- `docs/ARCHITECTURE_REVIEW.md` — 整体 DDD 评审
- `docs/OPTIMIZATION_REPORT.md` — P0-P3 性能优化
- `docs/old/` — 旧版本架构文档

---

## AGENTS.md 核心约束（开发时必读）

1. **数据真实性**：所有数据必须真实。Mock / 占位 / 伪造一律不行。
2. **环境隔离**：`STOCK_ENV_MODE=test` 用 `TEST_*` 前缀标的，与实盘硬隔离。
3. **测试纪律**：核心交易模块 ≥ 95% 覆盖；CI 门禁 ≥ 60% 起步。
4. **失败模式**：silent fail 一律改为显式 warn + 数据降级。
5. **配置纪律**：所有 magic number 集中到 `config/*.toml`，缺配置时 const fallback。

---

## License

仅供个人量化研究与学习。
