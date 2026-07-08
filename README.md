# A股量化系统

> **Rust 写的 A 股分析 + 回测系统。**
> **主要功能**：盘后复盘（AI 研判 + 飞书推送）、盘中监控（涨跌停/资金/异动）、5 个回测策略、事件驱动机会发现 (v9)。
> **最新进展**：[2026-06-28 v9.1 流水线 + event_extractor 落地](#修复成果) —— 28 项量化严谨性修复 + 事件抽取引擎。
> **总规模**：~56,000 行 Rust，165+ 源文件，32+ 模块。

---

## 5 分钟上手

### 1. 跑盘后复盘（最常用）

```bash
# 第一次：复制环境变量
cp .env.example .env
# 编辑 .env，把 STOCK_LIST 改成你的持仓 + 自选股（至少 2-3 只大盘蓝筹）
# 然后：
cargo run --bin monitor -- --review
```

**会做什么**：拉真实数据 → 算北向资金/涨跌停/板块 → AI 深度研判每只票 → 飞书推送 15 条消息。

**没有 .env 也能跑**（用 .env.example 的默认配置）。

### 2. 看回测结果

```bash
# 跑全部 5 个策略 + 13 个 RSI 预设 + Walk-Forward
cargo run --bin rsi_optimize
# 输出：reports/rsi_optimization_log.md + charts
```

### 3. 验证编译/测试

```bash
cargo build                  # 编译
cargo test --lib -- --test-threads=1   # 454 个测试, 串行 100% 通过
```

---

## 这是什么 / 不是

| 是 | 不是 |
|----|------|
| **事件驱动的实盘监控 + AI 研判系统** | 单日扫描器 / 短线交易系统 |
| **5 策略回测框架 + v9 事件驱动机会发现** | 预测明日涨跌的模型 |
| **数据真实性优先**（拒绝 mock/伪造） | 漂亮的演示 dashboard |
| **统计严谨**（Sharpe/Calmar/Sortino/dual_score） | 营销导向的"年化 200%" |
| **AI 事件抽取** (v9.1, 8 provider → 三层漏斗 → MarketEvent) | 噪音标题党

---

## 修复成果（2026-06-27 数据真实性大修 + 2026-06-28 v9.1 流水线）

这次修了 **28 个量化问题**，按风险从高到低：

| 风险等级 | 问题 | 修复前 | 修复后 |
|---------|------|--------|--------|
| 🔴 高 | **北向资金显示永远 0** | AGENTS.md 要求真实数据，违反红线 | 真接口，东财 `kamt/get`，元→亿元 |
| 🔴 高 | **多因子回测用末日截面** | 实际是 look-ahead，胜率虚高 | 日频 `factor_snapshot` 表 + `get_as_of` |
| 🔴 高 | **板块数据是 `name.len() % 3` 伪随机** | 喂给 AI 的数据是幻觉 | 东财 `clist/get` 真实数据 |
| 🔴 高 | **SQL 注入 4 处** | `format!` 拼 SQL | Diesel `?` 占位符 + bind |
| 🟡 中 | **Sharpe 用盘中价** | 60 日 Sharpe 实际是盘中 tick | `KlineData.intraday_price` 分离 |
| 🟡 中 | **17 股伪持仓** | A 股 1 手=100 股 | `round_lot` 整百股 |
| 🟡 中 | **多因子评分默认 50** | 假装中性 = 隐式偏误 | 二元（0 / 真实胜率） |
| 🟡 中 | **RSI 加仓可超仓位上限** | 隐性杠杆 | 硬性 ≤ max_position_pct |
| 🟡 中 | **板块集中度 40% 定义但未执行** | 满仓单板块无风控 | `check_position_limits` 真正算 |
| 🟡 中 | **现金底限 15% 定义但未执行** | 满仓 100% 现金底限 = 0 | 同上 |
| 🟢 低 | **17 股/0.001 滑点写死** | 与市值/波动率无关 | 动态 `BacktestConfig` 框架 |
| 🟢 低 | **winrate 50 占位** | 假装中性 7.5 分系统偏差 | 二元 + data_sufficiency |
| 🟢 低 | **多空分歧标签含义不清** | AI 输出来什么就显示什么 | prompt 显式定义 |
| 🟢 低 | **detector 死分支** | `if/else` 两分支相同 | 移除冗余 |
| 🟢 低 | **市场崩盘阈值写死 -1%** | 牛市/熊市都一样 | ATR(20) 自适应 |
| 🟢 低 | **EV 数学** 75% 错误 | 解析失败给 100 分（反语义） | 改 0 分 + warn |
| 🟢 低 | **静默 fail** | 假装通过 | 显式 warn + 降级 |
| 🟢 低 | **critic JSON 解析失败** | 默认 score=100 | 默认 0 + warn |

详细见 [验收报告](docs/ACCEPTANCE_REPORT_2026-06-27.md)。

**修复前**：评分靠伪数据，胜率虚高，Sharpe 假信号 → **回测数字不可信**。
**修复后**：422 个测试通过，端到端 exit=0, 0 panic, 15 飞书推送 → **可生产 + paper trading 验证**。

---

## 5 个回测策略

| 策略 | 文件 | 核心信号 |
|------|------|----------|
| **多因子选股** | `strategy/multi_factor.rs` | 市值/ROE/PE/PB/换手率 排序 + 日频快照 |
| **布林带+Z-Score** | `strategy/bollinger_zscore.rs` | 20日 BB + 偏离均值 2σ |
| **RSI v2** | `strategy/rsi/` | 5 日 RSI 超卖 + 趋势 + 13 个预设 |
| **BOLL+MACD** | `strategy/boll_macd.rs` | BB 收窄 + MACD 底背离 |
| **多时间框架** | `strategy/multi_timeframe.rs` | 60min/15min 联动确认 |
| **逆势** | `strategy/contrarian.rs` | 情绪 < 40 + 52周低位 + 企稳 |

`cargo run --bin rsi_optimize` 跑全部 5 个策略 + 13 个 RSI 预设 + Walk-Forward + 8 基准对比。

---

## 三大入口

### 盘后复盘（最常用）

```bash
cargo run --bin monitor -- --review
```

**飞书推送清单**（15 条）：
1. **市场概览**（指数/涨跌/北向资金/板块）—— 修复 P1.1
2. 复盘报告（累计收益/胜率/VaR）
3-9. **持仓深度研判**（5-7 只股，AI 多 agent 辩论）
10. 优选候选
11-13. 放量分析（持仓/自选/优选）
14. 风控研判
15. 推送完成

**为什么 --review 重要**：盘后推送，不影响盘中交易。AI 研判 + 实盘数据 + 真实历史 = 可信报告。

### 盘中监控

```bash
cargo run --bin monitor
# 默认日历触发 (15:30 收盘后跑一次, 周五 17:00 加 SOP)
# --test  跳过日历, 立即跑
# --review 盘后复盘模式
```

实时检测：
- **涨跌停**（含 ST 严格正则 + 新股 5 日识别）
- **资金异常**（主力单日净流出 > 5000 万）
- **VetoChain 否决**（高风险事件强制取消买入）
- **AI 事件抽取**（v9 设计，本期未实施）

### 策略回测

```bash
cargo run --bin rsi_optimize
# 输出：
#   reports/rsi_optimization_log.md (13 预设对比)
#   reports/backtest_chart_*.png
#   reports/details/ (审计留痕 CSV)
```

回测指标：**Sharpe / Sortino / Calmar / 最大回撤 / VaR95 / 胜率 / 基准 alpha-beta**

---

## 架构（DDD 限界上下文）

| 上下文 | 目录 | 职责 |
|--------|------|------|
| **Portfolio** | `src/portfolio/` | 持仓/交易/账本 单一来源 |
| **Market** | `src/market_analyzer/` + `src/data_provider/` | 行情/涨跌停/板块/资金流 |
| **Signal** | `src/signal/` | 统一信号（含 `MarketEvent` 标准中间件，v9 设计） |
| **Opportunity** | `src/opportunity/` | 事件→产业链→候选→0~100 评分. v9.1: event_extractor 三层漏斗 + dual_score |
| **Decision** | `src/decision/` | 排除/板块分层/资金核验/龙头识别 |
| **Risk** | `src/risk/` | 硬仓位/止损/VetoChain 否决链 |
| **Review** | `src/review/` | 复盘/业绩归因/falsification |

### 数据源 (Phase 1, review #15 + #16)

| 路径 | 数据源 | 优先级 |
|------|--------|--------|
| **K线 (盘中)** | Sina → 腾讯 (qfq) → 东财 (qfq) → RustDX TCP | P1 → P2 → P3 → P4 (4-way 竞速) |
| **K线 (盘后)** | Baostock → 上面 4-way | Baostock P1, 4-way P2 (盘后专用) |

详见 `docs/sina_baostock_integration.md` 与 `docs/business_rules.md` (BR-014, BR-015)。

---

## 目录结构

```
src/
├── agent/                  # AI 多 agent 辩论
├── analyzer/               # 技术分析 (MA/MACD/RSI/KDJ)
├── bin/monitor/            # 主入口 (--review / --test)
├── breakout/               # 放量分析
├── data_provider/          # 5 个数据源 + 涨跌停/北向计算
│   ├── rustdx_provider.rs  # 通达信 TCP (主, 最快)
│   ├── eastmoney_provider.rs
│   ├── gtimg_provider.rs
│   ├── north_flow.rs       # P0.4 修复
│   └── limit_status.rs     # P0.1 涨跌停/ST/新股
├── database/               # SQLite + migrations
├── decision/               # 排除/板块/轮动/龙头
├── market_analyzer/        # 指数/涨跌停/板块/市场概览
├── monitor/                # 实时检测 + 事件总线
├── notification/           # 飞书推送
├── opportunity/            # 事件→评分 (v9)
│   ├── event_extractor/    # P0-1 事件抽取 (5 文件, 20 测试)
│   │   ├── adapter.rs      # SearchResult → RawNewsItem
│   │   ├── rule_filter.rs  # 规则预筛 (6 discard + 9 keep)
│   │   ├── classifier.rs   # Quick AI 分类
│   │   └── core.rs         # Deep AI + 盘中确定性映射
│   ├── score.rs            # dual_score (P0-1)
│   ├── bom_kb.rs           # BOM 弹性节点 (P0-2)
│   ├── winrate.rs          # winrate 二元化 (P1-2)
│   └── launch_gate.rs      # 上线门槛 (P0-3)
├── pipeline/               # 主分析 pipeline
│   ├── backtest_runner.rs  # 5 策略回测
│   └── market_regime.rs    # 牛/震/熊 分状态
├── portfolio/              # 持仓/交易
├── risk/                   # 风控/止损
├── search_service/         # 8 个搜索 provider
├── strategy/               # 6 个策略 + BacktestEngine
└── signal/                 # 统一信号 (v9 MarketEvent)

config/                     # SIGHUP 热加载 toml
├── monitor.toml             # 扫描/告警/风控
├── chain_rules.toml         # 产业链关键词
├── exclusion.toml           # 排除板块
├── announce_keywords.toml   # 公告分类
└── risk.toml                # P3.1 集中风险常量
```

---

## 数据源（多源 fallback）

| 类型 | 源 | 优先级 | 修复 |
|------|----|----|------|
| 实时 K 线 | 通达信 (RustDX) → 东方财富 → 腾讯 | RustDX 最快 | — |
| 财务指标 | 腾讯财经 | 补充 PE/PB | — |
| **北向资金** | 东财 `kamt/get` | 4 流向 (沪/深/北/南) | P0.4 |
| **涨跌停** | 自算 (`limit_status.rs` 严格正则) | 含 ST + 新股 5 日 | P0.1 + P2.1 + P2.2 |
| **板块** | 东财 `clist/get` | 真实涨跌幅 | P0.3 |
| 搜索 | 东方财富/华尔街见闻/财联社/金十 (免费) → SerpAPI/Bocha/Tavily (付费) | 免费优先 | P1.4 |
| 公告 | 关键词 + 飞书 AI 抽取 | — | — |

---

## 关键设计决策（量化产品经理视角）

### 1. 数据真实性 > 一切

- **不 mock / 不占位 / 不编造**：AGENTS.md 红线，违反 = 假系统
- **缺失 = 显式 None / 0 / warn**，不是 50 假装中性
- **配置纪律**：所有阈值进 `config/*.toml`，缺 toml 时 const fallback
- **SIGHUP 热加载**：改 toml 不重启生效

### 2. 回测严谨性

- **T+1 结算 + 整百股 + 最小佣金 5 元 + 涨跌停拒绝**（P0.2 修复）
- **Sharpe 不再用盘中价**（P1.8 修复）
- **winrate_score 二元**（P1.2 修复，不假装 50）
- **日频因子快照**（P0.5 修复，替代末日截面）
- **板块集中度 + 现金底限真正执行**（P1.6 修复）
- **RSI 加仓硬性上限**（P1.5 修复，防隐性杠杆）
- **跨源软化 + 单源封顶 70**（避免 web 全挂时全灭）

### 3. 实盘可交易性

- **执行前**：decision `is_eligible()`（板/ST/北交所过滤 + 资金核验 + 板块分层）
- **执行后**：risk `check_position_limits`（单票 10% + 板块 40% + 现金 15% + 止损 -10%）
- **VetoChain 否决链**（`catch_unwind` 隔离 + dry-run 默认）

### 4. 性能（不靠 mock 凑 95%）

- P0-P3 性能优化（`OPTIMIZATION_REPORT.md`）
- tokio 多 worker + `spawn_blocking` 隔离 CPU 密集
- diesel 同步 + 异步客户端桥接
- 缓存：`OnceLock` + `RwLock` + `HashMap` 全局

---

## 已知遗留（明确推迟）

| 项 | 原因 | 后续 plan |
|----|------|----------|
| P2.4 HybridStrategy 真实加权 | 需 IC 加权/BMA 设计 | 单独 P2 plan |
| P2.6 幸存者偏差 | 需历史成分股数据 | 单独 P2 plan |
| P3.4 god-struct 完整拆分 | v9.1 分组标记版已做, 完整拆需 50+ 访问点 | v3 扩展 |
| P3.M1 σ/ADV 接入 | 框架已就绪, 数据接入推迟 | v3 扩展 |
| B-002 科创板日报缺失 | 11 个 bug 待修 | `docs/v9-known-bugs-已知bug清单-2026-06-28.md` |
| B-003 函数未接日报/周报 | live_rolling_sharpe + strategy_correlation_matrix 已写, 未接输出 | 同上 |
| v9 AI 调用接入 | event_extractor rules-only 版本已可跑, 真 AI 调用待接 | v9.1 最后一步 |
| `test_ledger_roundtrip` 并发 flaky | SQLite 全局单例 | 串行 100% 通过 |

---

## 开发者必读

### AGENTS.md 5 大约束

1. **数据真实性**：mock / 占位 / 伪造一律不行
2. **环境隔离**：`STOCK_ENV_MODE=test` 用 `TEST_*` 前缀，与实盘硬隔离
3. **测试纪律**：核心交易模块 ≥ 95% 覆盖；CI 门禁 ≥ 60% 起步
4. **失败模式**：silent fail 一律改显式 warn + 数据降级
5. **配置纪律**：magic number 集中到 `config/*.toml`，缺配置时 const fallback

### 验证清单

```bash
cargo build                                              # 编译
cargo test --lib -- --test-threads=1                   # 454 测试, 串行 100% 通过
cargo test --test event_extractor_tests                 # 20 个 event_extractor 测试
cargo clippy --lib                                       # 26 个 warning (不阻塞)
cargo run --bin monitor -- --review                     # 端到端, 15 飞书推送
```

### 文档

- `docs/v9-project-design-全项目设计-2026-06-28.md` — 全项目设计文档（一站式）
- `docs/architecture-v9.1-opportunity-pipeline-fix-2026-06-28.md` — v9.1 现行架构 + 5 项量化修正
- `docs/v9-known-bugs-已知bug清单-2026-06-28.md` — 10 个已知 bug（按 P0/P1/P2 排序）
- `docs/architecture/` — 10 个架构演进版本文档 (v2-v8 + P0 风控)
- `docs/reviews/` — 3 个评审报告（量化机构 38KB + DDD 20KB + 代码审查）
- `docs/reports/` — 1 个性能优化报告 (P0-P3 24-task)
- `docs/plans/` — 6 个项目计划 (v3-v8 + P0 风控)

---

## License

仅供个人量化研究与学习。
