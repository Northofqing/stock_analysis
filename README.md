# stock_analysis

面向 A 股的事件驱动实时监控系统（Rust）。统一采集行情/新闻/公告 → 规则筛选候选 → 风险门 → **虚拟盘纸面交易（含卖出闭环与账户自动估值）** → 微信推送与盘后复盘。

> 这是**研究/纸面交易**系统，不连接券商，不产生真实订单。账户事实来自用户确认的真实账户快照，公共数据统一走 Magic Gateway。

## 核心能力

| 能力 | 说明 |
| --- | --- |
| 📊 统一数据接入 | 行情/日K/指数/公告/新闻/板块/资金流/复盘数据经 `data_gateway` 强类型批次接入（Magic TDX 为主路由），禁止业务模块直连数据源 |
| 🎯 候选发现 | 涨停池、连板识别、新闻 → 产业链 → 候选，按 7 个上下文（Selection/Breakout/Signal 等）筛选 |
| 💰 虚拟盘成交 | 候选经风险门（现金/仓位/账户模式）后模拟买入，写 `paper_trades` + 不可变 `order_audit` 审计链；卖出一侧按**四大铁律**（ATR 止损 / -8% 硬止损 / 三级止损 / 破位减仓）自动扫描，盘中 30s tick + 15:30 收盘各一次 |
| 🏦 账户自估值 | 用户上传券商截图（快照）后，系统以快照为准；**不传则用持仓明细 × 实时行情自动估值**，每日收益 = 今日总资产 − 昨日（连续 5 个交易日无新快照才提醒上传） |
| 📲 推送治理 | 17 类 PushKind 经预算/冷却/去重/审计四层治理投递飞书微信，投递账本不可变 |
| 📋 盘后复盘 | 收盘/晚间复盘 + 交易复盘（R-07/R-11），按 `review_date` 取估值；持仓市值 Top-N 附 deep_analyzer 多角色 AI 研判（报告存 `reports/details/`，`REVIEW_AI_TOP_N` 可调） |
| 🛡️ 风险门 | 硬持仓/仓位/现金限制、账户模式（Frozen/ReduceOnly/Full）、数据模式（Unsafe 时行情依赖推送 fail-closed） |

## 项目结构

```text
src/
├── bin/monitor/        # 生产长驻服务：主循环 + 推送链（notify/push_l1-l7）
├── data_gateway/       # 统一数据接入（唯一允许接触数据源的地方）
├── broker.rs           # 实时行情入口（5 秒新鲜度门）
├── trading/            # 虚拟盘：成交模拟（paper_trade）+ 卖出闭环（paper_sell）
├── portfolio/          # 持仓/账户快照模型
├── decision/           # 盘中监控、风险门、账户口径（intraday_monitor）
├── pipeline/           # 信号链分析、持仓追踪（四大铁律卖出规则）
├── signal/             # 统一 Signal/SignalSet 结构
├── opportunity/        # 新闻 → 产业链 → 候选发现
├── review/             # 盘后复盘与证伪
├── breakout/           # 量能突破分析
├── selection/          # 候选选择（v2 激活门控）
├── risk/               # 硬性仓位/现金/账户限制
├── database/           # SQLite 持久化（schema + 每域访问）
└── bin/                # 工具链（导入快照/确认跳变/回填/探针等 ~30 个）
config/
├── strategy.toml       # 策略参数（启动时读一次）
├── chain.toml          # 产业链配置
└── selection/          # 候选选择配置 + 激活门控
data/
├── stock_analysis.db   # 主业务库：paper_trades/ledger/快照/审计
├── push_analytics.db   # 推送治理统计
├── durable_delivery.sqlite3  # 投递账本（路径编译期固定）
├── push_log/           # 每日推送原文
└── event_bus/          # 事件审计 JSONL
```

## 快速开始

```bash
cp .env.example .env    # 配置 STOCK_LIST（监控列表）、DATABASE_PATH、微信推送脚本
cargo build --release --bin monitor

# 常驻监控（真实数据 + 可能推送）
MONITOR_ENABLED=true ./target/release/monitor

# 全模板冒烟测试（隔离审计，不外发）
./target/release/monitor --test --push-dry-run

# 手动盘后复盘
./target/release/monitor --review

# 个股深度 AI 研判（多角色分析，报告写 reports/details/）
cargo run --release --bin deep_analyze -- 600519
```

`.env` 常用项：`STOCK_LIST` 监控代码、`DATABASE_PATH` 主库路径、`WECHAT_SEND_SCRIPT` 推送脚本、`BROKER_SOURCE` 行情入口（默认 `magic_tdx`）。**不要提交 `.env` 与账户证据。**

## 数据流

```text
Magic 数据源 ──> data_gateway（强类型批次+证据）──> 候选发现 ──> 风险门
                                                          │
用户上传券商截图 ──> 快照（account_summary + 持仓明细）──> 虚拟盘成交（paper_trades）
                                                          │
实时行情 ──> 每 30s：估值/四大铁律卖出扫描 ──> 成交写库 + 审计链 ──> 微信推送
```

## 日志快速参考

正常现象（无需处理）：`[paper_sell] … T+1锁仓无法卖出`（今日买入触发规则但锁仓）、`[paper_valuation] … 估值降级为日K最新收盘价`（单只实时行情超 5s 门）、`[BR-151] SnapshotPaper 使用用户确认持仓`、`[涨停板] N 行缺少主力净流`（能力未接入按设计排除）。

需要关注：`ERROR` 反复出现（数据源故障）、`[DataMode-hook] → Unsafe` 持续（行情能力未建立）。

## 文档

- 工程规则 / 数据合同：`AGENTS.md`、`CLAUDE.md`（含 Completion Rule 与证据规范）
- 业务规则注册表：`docs/business_rules.md`
- 架构细节：`docs/README.md`、`docs/ENGINEERING_RULES_V2.md`
