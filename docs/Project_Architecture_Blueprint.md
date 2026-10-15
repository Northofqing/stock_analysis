# Stock Analysis 项目架构蓝图

> 生成日期：2026-09-03  
> 专项修订：2026-09-02（推送系统审计与演进路线；v18/v19 设计覆盖、冲突裁决与周期）  
> 当前增量同步：2026-10-09（goal-first E1–E4源码工具、已安装版本包与最新monitor、真实运行根；旧九月表格按历史保留）
> 事实基线：当前工作树中的 Rust 源码、Cargo metadata、配置、测试、CI 与 README  
> 生成配置：Rust / Auto-detect / C4-oriented Mermaid / Implementation-Ready  
> 规模快照：514 个 Rust 文件，379,107 行；1 个 library、28 个 binary、41 个 integration-test、1 个 benchmark、1 个 build-script target

## 1. 阅读约定与证据规则

2026-10-09 范围面板优先于旧图中的 `CURRENT`：源码实现、候选包、安装观察、用户价值是四个独立验收。旧定量表/完整catalog/§24–25路线图是2026-09基线，不能用其数量或 proposed 接线描述今天生产。E5 facade/workspace/通用平台工程冻结，未来仍按 seam-first 从测得收益的单模块接口着手。

| 范围 | 当前证据与状态 |
| --- | --- |
| CURRENT_SOURCE | E1描述性registry/三档scorecard，E2独立watchdog/安全archive与候选screen，E4 artifact-only Phase A、SELL preview/Streak observed研究，E3全前缀分页与软SLI。详见[本次交接](handoffs/2026-10-09-goal-first-delivery.md) |
| STAGED | 五个selected release bins与Python3.9.6依赖闭包的候选副本继续保留；正式版本已发布到runtime/tools/goal-first/v20261009-f3761320c |
| INSTALLED_OBSERVED | 10-09正式发布实测：runtime `/Users/zhangzhen/.local/share/stock-analysis-runtime`，monitor PID97621/bridge PID96490、binary inode/SHA、未来activation Enabled、core DB及startup固定点通过；daily21:17/weeklyFriday20:30、新watchdog60s已加载。健康仍Frozen/Unsafe，见[发布记录](handoffs/2026-10-09-goal-first-release.md) |
| EXTERNAL_PENDING | family/version/PIT、独立close/真实lots/fees、可执行history、手机channel、历史monetary/Unknown人工处置、首次真实周报与≥20completed-session用户价值 |
| FROZEN_PROPOSAL | §24统一facade/PhaseScheduler/新catalog迁移及§25全平台路线不在本交付推进 |

图中旧标签保留其历史阅读意义：

| 标签 | 含义 |
| --- | --- |
| `CURRENT` | 当前生产或默认代码路径可达 |
| `CONDITIONAL` | 代码已实现，但受 feature、环境变量、activation、时间窗口或运行模式控制 |
| `COMPAT` | 为旧入口、旧合同或渐进迁移保留的兼容路径 |
| `INACTIVE` | 代码仍在，但生产主循环明确隔离或禁用 |
| `EXTERNAL` | 仓库外的 provider、LLM、Webhook、操作者或部署资产 |
| `INFERRED` | 由代码结构推导出的架构决策；仓库中没有对应正式 ADR 时，不冒充已批准 ADR |
| `PROPOSED` | 已形成并记录的目标架构或迁移约束；尚未宣称生产实现完成 |
| `HISTORICAL` | 历史实现、旧统计或被上位设计替代的记录；不代表当前运行路径 |

证据写作规则：

- `path:line` 指向当前源文件中的起始位置；行号会随代码编辑漂移，路径和符号名是长期锚点。
- 最终2026-10-09 `cargo metadata --no-deps --offline`：41 bins、43 integration-test targets、1 lib、1 bench、1 build-script；附录旧28/41及模块/行数只作2026-09历史inventory，未移除任何target。
- “完整”指架构面完整：运行单元、模块、数据流、状态机、存储、协议、横切约束、测试、部署和扩展点均列出；不逐行复述 379,107 行实现。
- 53-table catalog 是 legacy generation-1 冻结集合，不等于业务数据库所有增量 DDL 的总表数。

## 2. 架构检测结论

### 2.1 一句话结论

这是一个 **Rust 单 package 的大型模块化单体和外部数据服务消费者**。本仓库拥有 monitor/CLI、provider-neutral 领域类型、统一 Data Gateway、typed gRPC client contract、阶段化 selection workflow、SQLite 审计链与持久化投递协调器；生产市场数据宿主是仓库外独立部署的 `EXTERNAL provider-host`。它不是微服务集合，也不是严格单向依赖的 Clean Architecture。

### 2.2 定量与形态

| 维度 | 当前事实 | 代码依据 |
| --- | --- | --- |
| package | `stock_analysis 0.1.2`，单 Cargo package | `Cargo.toml`、Cargo metadata |
| library | 61 个公开顶层模块 | `src/lib.rs`；附录 A |
| production entrypoints | `monitor` 常驻控制面；`stock_analysis` 默认 CLI；provider-host 不属于本 package | `src/bin/monitor/main.rs`、`src/main.rs`、`README.md:28-44` |
| binary targets | 41（2026-10-09），新增goal-first工具为独立离线入口，不是resident owner | Cargo metadata；本次handoff；附录 A旧表历史 |
| integration tests | 43（2026-10-09） | Cargo metadata；附录 B旧表历史 |
| project features | 无 `[features]`；所有生产市场数据路径均为远程 gRPC，不能切换本地 provider | `Cargo.toml`、`README.md:28-29` |
| async runtime | Tokio；CLI/monitor 使用 multi-thread runtime | `src/main.rs`、`src/bin/monitor/main.rs` |
| primary DB | SQLite + Diesel + r2d2 + descriptor attestation | `src/database/mod.rs:419`、`:1893`、`:2442` |
| durable DB | 独立 `data/durable_delivery.sqlite3` + rusqlite | `src/bin/monitor/durable_delivery_runtime.rs:76`、`src/durable_delivery/coordinator.rs` |
| data protocol | tonic/prost typed gRPC；`client-bundle/market.proto` + build-time compatibility extensions | `build.rs:18-36` |
| consumer gRPC catalog | 40 个 production-used operations；provider-host capabilities 必须满足调用 | `src/grpc_contract/ops.rs:77-128` |
| event mechanisms | 通用 EventEnvelope bus + monitor-domain bus，两套不同广播 | `src/event/bus.rs:63`、`src/monitor/event_bus.rs:74` |
| delivery | 同步权威投递/审计 + observation-only bus/JSONL | `src/bin/monitor/main.rs:4744-4748`、`src/event/jsonl_writer.rs:1-12` |

### 2.3 代码热点

下表行数与模块描述是2026-09-03历史快照，未作为当前热点重新统计。

| 代码域 | Rust 行数 | 架构含义 |
| --- | ---: | --- |
| `src/bin` | 75,666 | monitor 编排与 28 个工具/运维入口是最大热点 |
| `src/database` | 73,767 | schema、attestation、repositories、审计链高度集中 |
| `src/data_gateway` | 30,555 | 远程 wire 转换、证据准入与领域网关；本地 provider 实现移除后明显收缩 |
| `src/selection` | 25,967 | 阶段化、receipt 化、可恢复 selection-v2 |
| `src/durable_delivery` | 18,856 | 投递状态机、租约、审计、恢复 |
| `src/performance` | 15,697 | 归因、epoch、回放与报表 |
| `src/pipeline` | 14,893 | 默认 CLI 的宽分析管道 |
| `src/event` | 13,159 | envelope、审计、JSONL、回放 |
| `src/monitor` | 12,853 | 可复用监控领域能力；不含 bin-local 编排 |
| `src/grpc_client` / `grpc_contract` | 3,074 / 930 | 外部 provider-host 的网络、认证、错误和合同接缝 |
| `src/market_domain` | 1,968 | 本仓拥有的 provider-neutral 市场领域类型 |

### 2.4 活性全景:设计态 vs 运行态(2026-10-11 增量审计)

§3–§17 描述**设计态**(系统被设计成什么样);本节标注同一张架构图的**运行活性**(今天实际跑成什么样)。状态词汇沿用 §1 标签定义。证据:2026-10-11 全库四轮源码普查(新闻/技术/交易链路深查 + 全模块活性扫面,逐模块核对生产调用方);修复路线见 [新闻智能层统一设计](plans/2026-10-11-news-intelligence-unified-analysis-design.md)(含决策记录 D1–D3)。

```mermaid
flowchart TB
    subgraph EXT["EXTERNAL"]
        host["provider-host VM\n行情/快讯/公告/资金流/龙虎榜"]
        llm["LLM APIs\nGemini / DeepSeek / 豆包"]
        feishu["飞书 magiclaw"]
    end
    subgraph ING["接入与准入 ~83k 行"]
        grpc["grpc_client 32k\ntyped gRPC · 40 ops"]
        gw["data_gateway 49k\nfail-closed + BatchEvidence"]
    end
    subgraph MDT["行情通道 · fail-closed"]
        qt["实时报价 ExecutionQuote 含涨跌停价\n日K HistoricalBars · 分钟线 · 5档盘口"]
    end
    subgraph ANA["分析域(monitor 常驻)"]
        news["news 8k · 4路快讯+公告\n120s 轮询"]
        trackA["通路A: LLM提取ticker→入池\n9-24 起断流零新增"]
        trackB["通路B: NewsAI 五档评估\n只推送不影响交易(D1已批准修订)"]
        tech["trend_analyzer 技术评分\nRSI口径分裂 · ATR非标准 · 无形态识别"]
        sel["selection admission-v1\n纯量价二元门 · 无 ranking\n北交所/ST 静默缺失"]
        mkt["market_analyzer\n大盘 regime · 板块共振"]
    end
    subgraph DEC["决策与交易(虚拟盘全自动)"]
        tick["intraday_monitor 30s tick\n6 策略打分 ≥6.0\nNewsCatalyst 常数 7.0"]
        paper["PaperLedger 虚拟盘 trading 46k\n买入固定 100 股 · T+1\n卖出: FIFO 账本 + 四大铁律 30s"]
        reg["registry 策略注册表\n注册在跑 · 弱消费"]
        dead["死代码: decide() 864行 · VetoChain · sector_score\nevent_extractor(决策D2删除) · impact.rs\nbus骨架 · paper_engine(BR-211) · alert.rs大半"]
    end
    subgraph REAL["真钱环(人工 · 无券商接口)"]
        human["人看飞书卡片\n手动券商下单"]
        snap["import_*_snapshot CLI\n真实持仓回填"]
        watch["真实持仓监控\nholding_plan · t0做T · T-16 ST预警"]
    end
    subgraph DELIV["投递域 ~200k 行"]
        pushf["push_foundation 140k\nintent store · activation fence · finalization"]
        p456["push_l4 去重 → l5 治理 → l6 sink\nL2 trait 0 实现 · L3 缺失"]
        dur["durable_delivery 60k\n21 表状态机 · fence · 幂等"]
    end
    subgraph REV["复盘与验证"]
        review["review R02-R12\nR05/R06 Disabled"]
        pred["prediction_tracker 76 条\n停更 07-17 · T+1 命中 17/64 = 26.6%"]
        perf["performance 19k\n归因 · 经济头寸"]
    end
    subgraph RES["研究/回测域(CLI 手动)"]
        bt["BacktestEngine + runner 3.6k\n多因子/布林/RSI · R-12 路径研究\nwinrate 模拟(BR-006 关停 7 个 0% 主题)"]
        sreg["signal_registry 6 信号\n全部 info_only/watch · 从未回测"]
    end
    subgraph CLIP["CLI 旁路(非常驻)"]
        app["app + pipeline 30k\n单次分析 · 报告"]
        deep["deep_analyzer + agent 深研\nAI 分析 IC=-0.0775(2026-06 实测)"]
    end
    subgraph STORE["存储与审计"]
        db[("业务 SQLite · database 146k\nDiesel/rusqlite 双栈")]
        ddb[("durable SQLite 独立库")]
        jsonl["JSONL 哈希链\nevent_audit / durable_audit"]
    end
    host --> grpc --> gw
    gw --> qt
    gw --> news
    gw --> sel
    gw --> mkt
    qt ==> tick
    qt ==> tech
    news --> trackA
    news --> trackB
    trackA --> llm
    trackB --> llm
    trackA -. "断流" .-> tick
    sel -. "半接线" .-> tick
    tech --> tick
    mkt --> tick
    tick --> reg
    tick --> paper
    tick --> pushf
    trackB --> pushf
    watch --> pushf
    pushf --> p456 --> dur --> feishu
    feishu -. "人工环" .-> human
    human --> snap --> watch
    paper --> perf
    tick --> review
    review --> pred
    gw --> db
    tick --> db
    dur --> ddb
    dur --> jsonl
    app --> deep
    app --> llm
    app --> bt
    bt --> sreg
    sreg -. "从未回测生产信号" .-> tick
    classDef alive fill:#173627,stroke:#62d7a2,color:#eaf5ff
    classDef degraded fill:#3a2f1a,stroke:#f3b251,color:#eaf5ff
    classDef deadcode fill:#3a1f24,stroke:#ff7a86,color:#eaf5ff
    class host,llm,feishu,grpc,gw,qt,news,mkt,paper,pushf,p456,dur,db,ddb,jsonl,perf alive
    class trackA,trackB,tech,sel,tick,review,pred,reg,app,deep,bt,sreg,watch degraded
    class dead,human,snap deadcode
```

交易循环(行情 → 买入 → 持仓 → 卖出 → 归因)单独成图:

```mermaid
flowchart LR
    subgraph MKT["行情数据 provider-host → data_gateway"]
        quote["实时报价 ExecutionQuote\n含涨跌停价 · fail-closed"]
        kline["日K HistoricalBars\nselection 特征 / trend_analyzer"]
        minute["分钟线 MarketMinutePoint\n60min/15min 入场确认"]
        book["5档盘口 OrderBook"]
    end
    subgraph BUY["买入 · 虚拟盘全自动"]
        pool["pushed_stocks 候选池\n通路A新闻 / selection / D-01"]
        tick["intraday_monitor 30s tick\nv16_4 打分 ≥ 6.0"]
        limit["涨跌停闸门 BR-134\n涨停不买 跌停不卖"]
        risk["risk_adapter 4 硬检\n账户模式 · 单票10% · 现金底15% · 数据模式"]
        buy["PaperLedger.buy\n固定 100 股 · T+1"]
    end
    subgraph SELL["持仓与卖出 · 虚拟盘全自动"]
        ledger["FIFO 批次账本\npaper_lot_ledger"]
        scan["paper_sell 30s 扫描\nPAPER_SELL_DISABLED 逃生口"]
        rules["四大铁律\nATR止损 · 破MA5止盈\n14天换股 · 布林上轨"]
        sell["PaperLedger.sell\n卖出 → 飞书推送 → 归因"]
    end
    subgraph REAL["真钱环 · 人工 · 无券商接口"]
        card["飞书卡片\nD-01 个股卡 / 卖出提醒"]
        human["人手动券商下单"]
        snap["import_*_snapshot CLI\n真实持仓回填"]
        watch["真实持仓监控\nholding_plan 三预案 · t0做T · T-16 ST预警"]
    end
    quote --> tick
    quote --> scan
    pool --> tick
    tick --> limit --> risk --> buy --> ledger
    ledger --> scan --> rules --> sell
    kline --> rules
    minute --> tick
    book -. 仅观测 .-> tick
    sell --> card
    card --> human --> snap --> watch --> card
    classDef alive fill:#173627,stroke:#62d7a2,color:#eaf5ff
    classDef degraded fill:#3a2f1a,stroke:#f3b251,color:#eaf5ff
    classDef deadcode fill:#3a1f24,stroke:#ff7a86,color:#eaf5ff
    class quote,kline,minute,tick,limit,risk,buy,ledger,scan,rules,sell alive
    class pool,book,card,watch degraded
    class human,snap deadcode
```

图例:绿=CURRENT(生产在用),黄=CONDITIONAL(半活/降级),红=INACTIVE(死代码/断流)。

#### 2.4.1 全模块活性普查(按代码量排序)

| 模块 | Rust 行数 | 职责 | 活性 | 证据与备注 |
| --- | ---: | --- | --- | --- |
| `database` | 145,584 | schema/attestation/repository/审计链 | `CURRENT` | Diesel+r2d2 与 rusqlite 双栈并存 |
| `push_foundation` | 139,851 | 推送地基:intent store、activation fence、N02 绑定、finalization SLA | `CURRENT` | monitor 启动与 N02/chain 推送在用;体量中大量为 schema 迁移测试 |
| `bin/` | 115,617 | monitor 编排 + 40 个工具/探针/研究入口 | `CURRENT` | main.rs 10.3k 行单文件过重;bins 分类见下表 |
| `durable_delivery` | 60,584 | 投递状态机(21 表)、fence、幂等、恢复 | `CURRENT` | §12 |
| `data_gateway` | 48,981 | 远程 wire 转换 + fail-closed 证据准入 | `CURRENT` | §6;质量最高的一层 |
| `trading` | 46,547 | PaperLedger 虚拟盘、FIFO 账本、T+1 | `CURRENT` | 2026-10-09 绑定真实持仓;`paper_engine` 被 BR-211 硬禁用 |
| `monitor`(lib) | 40,257 | 可复用监控域能力 | `CONDITIONAL` | `alert.rs` 大半死亡(push_alert/aggregate_alerts/send_alert 零调用) |
| `grpc_client` | 31,917 | 外部 provider-host 网络/认证/错误 | `CURRENT` |  |
| `selection` | 26,960 | 事件驱动选股 admission-v1 | `CONDITIONAL` | 纯量价二元门;无 ranking;outcome 仅 shadow;北交所/ST 静默缺失 |
| `pipeline` | 24,840 | CLI 宽分析管道 | `CONDITIONAL` | 生产 monitor 不经此路,仅 CLI 使用;内含 backtest_runner(多因子/布林/RSI 回测) |
| `performance` | 19,201 | 归因、经济头寸、业绩报告 | `CURRENT` | replay 子路径偏研究/回填 |
| `event` | 15,152 | EventEnvelope bus + History/Replay | `CURRENT` | 三套总线并存:event/bus 与 monitor/event_bus 活,`src/bus` 骨架死 |
| `decision` | 11,544 | 盘中监控 + 决策 | `CONDITIONAL` | `intraday_monitor` 活(30s tick);`decide()` 864 行零调用、`sector_score` 未接线 |
| `strategy` | 11,307 | v16_4 八策略 + 回测策略 | `CONDITIONAL` | 八策略在跑但常数分;`BacktestEngine`(core.rs 1.9k)+`backtest_runner`(1.7k)仅 CLI/研究;**生产 8 信号从未回测(signal_registry 全 watch)**;`model_change_draft_v1` 零调用 |
| `news` | 8,403 | 新闻采集/聚合/分类 | `CONDITIONAL` | 采集强;`impact.rs` 打分链死;快讯事件分类空投影 |
| `opportunity` | 7,198 | 候选机会 | `INACTIVE` | `auction_agent` test-only;`event_extractor` 已决策删除(D2) |
| `review` | 6,378 | 盘后复盘 R02-R12 | `CONDITIONAL` | R05/R06 Disabled;prediction_tracker 停更 2026-07-17;含 R-12 15min 路径研究(`review/backtest.rs` 1.9k,仅研究) |
| `app` | 5,723 | CLI 主流程 bootstrap/modes/schedule | `CURRENT` | monitor 复用其 chain_schedule |
| `notification` | 5,508 | 通知服务(feishu/email/wechat) | `CONDITIONAL` | 仅 CLI 侧;monitor 不经 NotificationService;`send_daily_report` 零调用 |
| `search_service` | 4,643 | 网页搜索(SerpApi/Bocha/Tavily) | `CONDITIONAL` | ResearchOnly;规则 NLP 关键词级 |
| `market_analyzer` | 4,585 | 大盘复盘/板块/涨停池 | `CURRENT` |  |
| `evidence_retention` | 4,349 | 证据留存 outbox | `INACTIVE` | 唯一调用方是两个死模块(paper_funding_review_v1 / model_change_draft_v1) |
| `agent` | 4,304 | Agent 工具带 + 多角色深研 | `CONDITIONAL` | CLI/手动触发;不在自动选股→深研链路 |
| `risk` | 4,156 | 风控 | `CONDITIONAL` | monitor 侧 risk_adapter 活;lib 侧 VetoChain 从未实例化 |
| `offline_products` | 3,489 | sell_reminder + streak_leader 研究 | `CONDITIONAL` | 仅手动 bin,无调度 |
| `analyzer` | 3,241 | GeminiAnalyzer 等 AI 分析 | `CONDITIONAL` | v19.2 确诊 IC=-0.0775 反向指标;三套 LLM 设施并行 |
| `llm` | 2,503 | LLM provider 抽象 + ticker 提取 | `CURRENT` | extract_tickers 无 receipt(待 P1 修复) |
| `data_provider` | 2,256 | 筹码/一致预期/停复牌/涨跌停状态/news_item | `CURRENT` | 本地 provider 移除后明显收缩 |
| `market_domain` | 1,970 | provider-neutral 领域类型 | `CURRENT` | 150+ 文件引用 |
| `indicators` | 1,399 | MACD/KDJ/RSI/背离 | `CONDITIONAL` | RSI 双实现、ATR 非标准、无形态识别 |
| `portfolio` | 1,197 | 持仓计划存储 | `CURRENT` | PositionSizer 不进生产 monitor |
| `push_l1/l4/l5/l6/l7` | 3,624 | 推送分层 | `CURRENT` | L4-L7 活;`push_l2` 316 行 trait 0 实现,L3 目录不存在 |
| `signal` | 585 | market_event + push_recorder | `CONDITIONAL` | 顶层 Signal 类型已删 |
| `breakout` | 510 | 放量启动检测 | `CURRENT` | 盘中/盘后双模式 |
| `bus` | — | v16.4 三总线骨架 | `INACTIVE` | 唯一生产发布方 paper_engine 已硬禁用 |
| `magic_compat` | 0 | 空目录残留 | `INACTIVE` | lib.rs 未声明,可直接删 |
| `gate_d_*`(5 文件) | 475 | Gate-D 验收回归 | `—` | 全部 #[cfg(test)] 挂载,生产零参与(正常) |

#### 2.4.2 src/bin 41 个 binary 分类

| 类别 | bins |
| --- | --- |
| launchd 生产关键 | `monitor`(常驻);`weekly_outcome_review`+`assistant_review`;`backfill_daily`+`backfill_predictions` |
| 生产运维(手动) | import_*×3、paper_account_activate、run_closing_valuation、confirm_daily_change、resolve_stale_uncertain、selection_activation_prepare、migrate_selection_v2、chain_schedule_resolve、day_capture_check、durable_schema14_snapshot_check、attribution_backfill、strategy_attribution、backfill_catalyst_watchlist、paper_adjudication_preview、sell_reminder_producer/preview |
| 诊断探针(手动) | account_metrics/announcement/board_directory/economic_position/gateway_quote/grpc_bundle/grpc_candidate_bundle/grpc_failure/grpc_local_readiness/historical_observed 各 probe、lhb_query |
| 研究/回测 | boll_macd_backtest、rsi_optimize、winrate_simulator、streak_leader_research、produce_winrate_samples、deep_analyze |
| 一次性/开发 | agent_test、v14_e2e |

#### 2.4.3 本次普查新发现的死代码/断流(超出设计文档记录)

- `monitor/alert.rs` 大半死亡:push_alert/aggregate_alerts/send_alert 零调用,仅 format_alert 经 alert_log 活
- `src/bus` 骨架死亡:唯一生产发布方 `paper_engine::run_once` 被 BR-201/BR-211 硬禁用(main.rs:10145 有断言隔离)
- `src/magic_compat/` 空目录残留,lib.rs 未声明
- `strategy/model_change_draft_v1` + `trading/paper_funding_review_v1` 零生产调用 → evidence_retention 断流的根因
- 三套总线并存:`event/bus.rs`(活)、`monitor/event_bus.rs`(活)、`src/bus`(死),替代关系未完成
- `lib.rs` 的 `pub mod cli` 冗余:bin 用各自本地 mod cli,lib 侧零外部调用方

#### 2.4.4 模块详析(2026-10-11 全模块普查,逐模块核对生产调用方)

##### `src/database/`

- **职责:** SQLite 持久化核心——`DatabaseManager` 单例（`OnceCell`, diesel+r2d2 连接池） + descriptor-attested 连接证明（fd/inode 级防伪， BR-001/016/017/050/066/129) + 全库 schema 定义与各业务表存取子模块。
- **内部结构:** `mod.rs`(7037, 连接池/descriptor attestation/单例/retention material proof)、`global_schema_v1.rs`(15583, 主 schema)、`selection_v2.rs`(7194)、`news_ai.rs`(4816, 另有 news_ai/ 子目录）、`selection.rs`(2060)、`kline.rs`(1304)；另有约 20 个 `global_schema_*` 版本化 schema 文件及 attribution/paper/audit 系列子模块。
- **生产调用链:** 入口双侧接线——monitor 启动 `DatabaseManager::init/init_retained_monitor` (src/bin/monitor/main.rs:4514-4516),CLI `stock_analysis` 启动 (src/main.rs:66-69)；运行时各 loop 经 `get()/try_get()` 消费（src/bin/monitor/health.rs:7, market_data.rs:179, push_templates.rs:274/1316/7840, news_aggregator_init.rs:1393, durable_delivery_runtime.rs:6713, holding_plan.rs:188);lib 侧 src/monitor/*、src/data_gateway/*、src/pipeline/*、src/trading/*、src/selection/* 全面引用（308 处 grep 命中）。
- **活性:** CURRENT — monitor 主流程启动即 init，主循环与 CLI 全程可达。
- **问题/缺口:**
  - `DatabaseManager` 两个字段 `selection_connection_source`/`selection_schema_authority` 标 `#[allow(dead_code)]` 保留未消费（src/database/mod.rs:431-434)。
  - `DatabaseAuthorityError::code()` 标 `#[allow(dead_code)]`(src/database/mod.rs:748-749)。
  - `DatabaseManager::get()` 未初始化即 panic("数据库未初始化，请先调用 DatabaseManager::init()", src/database/mod.rs:3475)，依赖 init 时序约定；多数生产调用方用 `try_get` 兜底，但 `get()` 裸调用仍散布在 monitor 多处（如 holding_plan.rs:188, market_data.rs:179)。
  - mod.rs 单文件 7037 行，descriptor attestation、只读快照复制、业务查询、测试 hook 基建混在一起，维护面过大。

##### `src/event/`

- **职责:** 事件基础设施——`DomainEvent`/`EventEnvelope` 信封、有界广播 `EventBus`、BR-091 权威 JSONL hash-chain 审计写（`AuditDispatcher`)、NewsFlash 投递权威（attempt/terminal/failure 追加 + reconcile)、历史查询与 replay。
- **内部结构:** `dispatcher.rs`(2785)、`envelope.rs`(2082)、`push_record.rs`(1984)、`durable_delivery_append.rs`(1841)、`mod.rs`(3536, NewsFlash authority snapshot/reconcile 主体）、`history.rs`(1092)、`replay.rs`+`cli.rs`(453+451, events history/replay 子命令）。
- **生产调用链:** monitor 启动 `spawn_blocking(preflight_runtime_delivery_audit)` 关门（src/bin/monitor/main.rs:5805) → `global_bus()`(main.rs:5922, health.rs:36) → `JsonlWriter::spawn`(main.rs:5937)；投递审计写入 `publish_delivery*`(main.rs:3471, notify.rs:2420/2809, news_ai_shadow.rs:396)、`publish_news_flash_attempt/terminal/failure`(notify.rs:2927/2995, news_aggregator_init.rs:991/1592/1609);`AuditDispatcher` 注入 durable delivery 运行时（durable_delivery_runtime.rs:1066/1164);`EventCommand` 子命令 history/replay 接线于 main.rs:5659/6016-6055。
- **活性:** CURRENT — monitor 启动预检、主循环推送审计、CLI 子命令均实际可达。
- **问题/缺口:**
  - `delivery_settlement.rs` 整文件（47 行， BR-145）是死导出： `settle()`(src/event/delivery_settlement.rs:15）仅被自身测试调用（同文件：40),`DeliverySettlement`/`IdentityAction` 经 mod.rs:22 re-export 后无任何消费方；news_aggregator_init.rs 里的 `gate.settle` 是另一类型的同名方法，不构成调用。
  - NewsFlash reconcile 主体（~1200 行）直接堆在 `mod.rs`(src/event/mod.rs:1279-1545）而非子模块，与 3536 行的 mod.rs 规模不匹配。
  - src/bin/monitor/main.rs:13914 测试注释 "This test will fail until main() wires event::cli::parse_args" 已过期——接线实际存在于 main.rs:5659/6016。

##### `src/evidence_retention/`

- **职责:** 自声明"有界、未验证的历史材料"(src/evidence_retention/mod.rs:1-2)——retention package draft / stored receipt / incomplete daily root 三类载体的纯值编解码与 claim 一致性校验，加本地 outbox 落盘；明确无 I/O、无 owner seal、无签名、不转换为交易/留存权威。
- **内部结构:** `mod.rs`(936, draft/receipt/root 类型与校验器）、`codec_v1.rs`(1027, Work 计量的有界编解码）、`outbox_v1.rs`(1442, 本地 outbox 打开/入队）、`tests.rs`(587)+`outbox_v1_tests.rs`(357)。
- **生产调用链:** `pub(crate)`(src/lib.rs:34)；调用方仅 `trading::paper_funding_review_store_v1`(src/trading/paper_funding_review_store_v1.rs:7)、`strategy::model_change_draft_store_v1`(src/strategy/model_change_draft_store_v1.rs:7)，以及 `database::open_retention_material` 签名（src/database/mod.rs:6992，被 outbox_v1.rs:456 调用）；而这两个 store 模块自身（均为 pub(crate), src/trading/mod.rs:14-15, src/strategy/mod.rs:31）也只被各自 `#[path]` 测试模块引用（src/trading/paper_funding_review_v1.rs:870 位于 tests 子模块内； src/strategy/model_change_draft_v1_tests.rs:1)。src/bin/ 与 monitor 零引用。
- **活性:** INACTIVE — 零生产调用方，整条可达链终点全是 cfg(test)/#[path] 测试。
- **问题/缺口:**
  - 模块自我定位为预留的留存基础设施，尾部注明 "no remote dispatcher or seal issuer"(src/evidence_retention/mod.rs:933-934)，从未接入任何生产 pipeline。
  - 唯一非测试消费方（paper_funding_review_store_v1 / model_change_draft_store_v1）自身也仅测试可达，形成两层死代码链；database/mod.rs:6956-7037 的 retention material proof 代码仅经 outbox_v1.rs:456 服务于此链。
  - 代码质量本身未见问题（有完整有界计量与一致性测试），缺口是"从未接线"而非缺陷。

> **本组总评:** `src/database/` 是生产主体的持久化基石，CURRENT 且调用面最大，主要债是 mod.rs 巨型化与零星 `#[allow(dead_code)]` 残留；`src/event/` 承载 BR-091/192/244 投递审计权威写路径，CURRENT，但 `delivery_settlement.rs` 是孤立死导出且 reconcile 逻辑淤积在 mod.rs;`src/evidence_retention/` 是设计完备但完全未接线的留存草案/outbox 设施，连其仅有的两个 store 调用方也只被测试引用，整体 INACTIVE，属"预留能力"而非在役代码。

##### `src/push_foundation/`

- **职责:** v14.x "推送地基"持久化层 — 业务意图存储（BusinessIntentStore)、激活/就绪/终态/对账原语，外加 N02 快讯预约绑定和盘前/盘后链调度 shadow 观察器。
- **内部结构:** `intent_store.rs` (2993 行） + `intent_store/` 子目录 (89 个文件， 含 `chain_post_close.rs` 4598 行 + v1–v15 SQL 迁移 + `n02_binding/v2.rs` 344 行）; `activation_fence.rs` (1040 行）; `dedicated_transport.rs` (799 行）; `generic_transport.rs` (770 行）; `chain_preopen_shadow.rs`/`chain_post_close_shadow.rs`; 约 40 个体量常超实现的 `*_tests.rs`。
- **生产调用链:** 极窄 — ① monitor 主循环盘前/盘后链调度旁路： `src/bin/monitor/main.rs:10495` `observe_chain_post_close_shadow`、`main.rs:11253` `observe_chain_preopen_shadow`（在 `run_scheduled_chain_analysis` 调用之后， 只打对比日志）; ② 新闻快讯 N02 路径： `src/bin/monitor/news_aggregator_init.rs:292` `N02ReservationBindingV1::try_from_reservation_material` → `src/monitor/push_job/n02_source.rs:19`; ③ `src/app/chain_schedule.rs:447` `project_chain_schedule_snapshot`（同为 shadow 投影）。
- **活性:** CONDITIONAL — 仅 shadow 观察器（只读、不落库）和 N02 绑定类型生产可达； 激活/就绪/对账/终态/传输全套（约占体量 90%）零生产调用方， 仅模块内互调+测试。
- **问题/缺口:**
  - `mod.rs:1` 自述 "Additive push-foundation persistence. No production database is selected or migrated here" — Foundation schema 未接管任何生产库。
  - `BusinessIntentStore` 无任何模块外生产调用方： grep 命中全部位于 `src/push_foundation/**`（如 `reconciler.rs:19`、`finalization_sla.rs:28` 内部 use）及 `*_tests.rs`。
  - shadow 是 log-only: `main.rs:10501` 与 `chain_schedule.rs:462` 日志固定 `foundation_persisted=false`, 不产生影响。
  - pub 导出的 `ActivationManifest`/`inspect_raw_activation_facts`(`mod.rs:49,52`）只在模块内与测试使用， 属死导出。
  - 测试体量远超实现（如 `finalization_sla_tests.rs` 81KB vs `finalization_sla.rs` 22KB), 维护成本高而当前生产价值为零。

##### `src/push_l1/`

- **职责:** v14.2 L1 信号源层 — `SignalEvent`(L1→L4 唯一信号载体）+ `SignalSource`/`SignalPayload` 强类型枚举 + sha256 event_id 派生。
- **内部结构:** `event.rs` (436 行） + `mod.rs` (15 行）。
- **生产调用链:** `src/bin/monitor/v14_adapter.rs:31-38` import(`make_source_fact_event_id` 在 :490/:531/:568/:609/:651/:796/:1417 共 7 处） → `notify.rs:2356` 等 → monitor 推送主链路； `news_aggregator_init.rs:2510`。
- **活性:** CURRENT — 每条生产推送过 `v14_gate` 时都构造 SignalEvent。
- **问题/缺口:**
  - `mod.rs:4-5` 自述 "后续 W3-W4 会加 market_session_emitter, preopen_window_emitter" — emitter 层从未实现； 事件实际由 `v14_adapter.rs:1304/1384` 的 `signal_event_for_kind` 从 PushKind 逆推构造， 方向与"L1 源 → L4"的设计相反。
  - event_id 含 1–10s 时间桶， 同业务事件跨桶 ID 不同， `dispatcher.rs:7-9` 注释自述"按 id 去重形同虚设", dedup 已改用 （kind,code) 键， event_id 仅剩跨层关联用途。

##### `src/push_l2/`

- **职责:** v14.2 L2 模板注册层 — `Template` trait + `TemplateMetadata`/`DataMode`/`RenderedText` 等流通类型。
- **内部结构:** `template.rs` (303 行） + `mod.rs` (13 行）。
- **生产调用链:** 仅类型被 `v14_adapter.rs:35`(DataMode/RenderedText/TemplateMetadata)、`push_l5/governance.rs:19`、`push_l7`、`l6_sink.rs:39` 使用。
- **活性:** CONDITIONAL — DataMode/RenderedText/TemplateMetadata 是生产治理与 analytics 的流通类型； 但 `Template` trait 全仓库无 `impl Template for`（零实现）, `TemplateCategory`/`BannerContext`/`FailureStrategy` 只出现在 `src/bin/v14_e2e.rs` 和 governance 测试。
- **问题/缺口:**
  - `mod.rs:5` 自述 "W3.2 会加 build.rs + TOML 解析 + 静态注册 inventory" — 模板注册表从未建； `src/lib.rs:64` 注释确认 "L3 Render 暂缺， 渲染走 push_templates::render_xxx"。
  - `Template`/`Validate`/`FailureStrategy`(`template.rs:93-112`）与 `BannerContext`(`template.rs:179`）为死代码。

##### `src/push_l4/`

- **职责:** v14.2 L4 仲裁层 — Dispatcher dedup 闭环： reserve/commit/rollback 三段式， 键 (kind, code, sub_kind), 冷却窗口来自 `PushKind::cooldown_secs()`。
- **内部结构:** `dispatcher.rs` (584 行： DashMap 内存 dedup 表 + `DispatcherStats` 原子计数 + `sub_kind_dedup_key`) + `mod.rs` (11 行）。
- **生产调用链:** `v14_adapter.rs:989/991` reserve + `:1085/1087` commit(V14Stack 全局单例， `v14_adapter.rs:45`) ← `notify.rs` 全部推送入口 → monitor 主循环。
- **活性:** CURRENT — 每条推送的速率限制实际在此执行。
- **问题/缺口:**
  - dedup 表是进程内存 `DashMap`(`dispatcher.rs:44`), 重启即丢； 跨重启幂等完全依赖 durable_delivery/push_job 层兜底， L4 自身不持久化。
  - 便捷方法 `dispatch()`(`dispatcher.rs:210`）生产无人调用， 仅 `v14_e2e.rs`/测试使用（生产走 reserve/commit 两段）。

##### `src/push_l5/`

- **职责:** v14.2 L5 治理层 — `GovernanceEngine::check_governance`: 静默期（02:00–06:00)/Frozen 模式/data_mode 门禁/每日上限。
- **内部结构:** `governance.rs` (543 行， 含 Frozen warn 60s 节流 `FROZEN_WARN_THROTTLE_SECS`) + `mod.rs`。
- **生产调用链:** `v14_adapter.rs:37`(V14Stack 单例 `governance` 字段） → gate 内先判 L5(`v14_adapter.rs:877`), ctx 由 `current_governance_ctx()`(:1572）从 `LATEST_BANNER` + 本地时钟构造。
- **活性:** CURRENT — 每条推送过闸都执行。
- **问题/缺口:**
  - data_mode 门禁对多数 kind 实际不拦： BR-249 实测 Full 门恒不可达（`v14_adapter.rs:1453-1458`), 参考级推送 `data_mode_min` 被放宽到 Down(`v14_adapter.rs:1554-1557`, "2026-08-06 用户决策 C 方案"), L5 实际有效约束主要是 quiet_hour/daily_limit/紧急豁免。
  - `mod.rs:6` 注释仍写 "后续 W5.2/W6 会加 monitor_loop 集成 + GovernanceContext 构造" — 该集成已由 v14_adapter 完成， 文档漂移未更新。

##### `src/push_l6/`

- **职责:** v14.2 L6 投递层 — `Sink` trait + `SinkRouter` + `ConsoleSink`; `external_sinks` 提供 Wechat/Feishu/Http sink。
- **内部结构:** `sink.rs` (394 行） + `external_sinks.rs` (536 行， v15.1 起 HttpSink 有真实 reqwest POST) + `mod.rs`。
- **生产调用链:** `src/bin/monitor/l6_sink.rs:40` import, `sink_router()`(:136）单例只注册 ConsoleSink + MagiclawSink ← `notify.rs:2388-2391` 真实投递后 `route(&msg)`。
- **活性:** CURRENT(Sink trait/Router/ConsoleSink 部分）, 但三个外部 sink 生产零注册， 属 CONDITIONAL/部分死代码。
- **问题/缺口:**
  - `WechatSink`/`FeishuSink`/`HttpSink` 从未在生产注册： 扩展入口 `init_with_extra_sinks`(`l6_sink.rs:182`）无生产调用方， 仅 `l6_sink.rs:21-24` 注释里的示例用法。
  - L6 route 是投递后的旁路（`notify.rs:2388` 在 `push_wechat` 成功后才 build/route), 真实外发不走 L6 — L6 不是投递执行层而是留痕/回声层。
  - `HttpSink::health_check` 恒返回 true(`external_sinks.rs:140-142`), 无真实探测。

##### `src/push_l7/`

- **职责:** v14.2 L7 分析层 — `PushAnalytics` 记录 + `AnalyticsStore` trait + `SqliteStore`（持久化到 `data/push_analytics.db`) + `InMemoryStore`。
- **内部结构:** `analytics.rs` (504 行） + `sqlite_store.rs` (658 行） + `mod.rs`。
- **生产调用链:** `v14_adapter.rs:47` `V14Stack.store = Mutex<SqliteStore>` → `v14_record_delivery`(`v14_adapter.rs:1195`）每次投递后 `record()`; daily-limit 读取走 `count_today_pushed_for_user_and_template`(`v14_adapter.rs:906` → `sqlite_store.rs:67`)。
- **活性:** CURRENT — 写路径+每日计数读路径在每条生产推送上执行。
- **问题/缺口:**
  - `SqliteStore::set_global`/`global()`(`sqlite_store.rs:31-44`）全仓库无调用方， 死 API。
  - 分析读 API(`query_by_time_range`/`count_by_governance`/`push_rate`）无生产消费者， 仅测试和 `v14_e2e.rs:308` — analytics 只写不读， 无报表/看板出口。
  - `InMemoryStore`(`analytics.rs:114`）仅 `v14_e2e.rs:256` 与测试使用； `count_today_for_user`(`sqlite_store.rs:47`）亦无生产调用方（生产用带 template 的变体）。

> **本组总评:** v14.2 七层中 L1/L4/L5/L7 已通过 `v14_adapter` 真实接入 monitor 推送主链路（`notify.rs` → `v14_gate` → 真实投递 → `v14_record_delivery`), 是全仓库少数"架构与运行一致"的部分； L2/L6 只落地了类型与骨架， 模板注册表和外部 sink 从未实装， L6 实际只是投递后留痕。push_foundation 是另一极： 约 140K 的激活/就绪/终态/对账持久化体系几乎零生产调用方， 仅 N02 快讯绑定类型和两个 log-only shadow 观察器被 monitor 引用， 属"为 Foundation 未来接管预留、当前纯测试驱动"的大型储备代码。

##### `src/durable_delivery/`

- **职责:** BR-192 持久化计数投递暗核：物理隔离的 SQLite 投递状态机（预留账本、fencing、冻结审计载荷、预算/冷却、复盘终局重放），为 monitor 全部 counted 推送提供恰好一次的决策/回执证据。
- **内部结构:** `coordinator.rs`(10954 行，主状态机 DurableDeliveryCoordinator)、`model.rs`(2416,Envelope/DecisionState/策略目录）、`schema.rs`(1647,SCHEMA_VERSION=14，约 19 张核心表）+ `schema_g5b_cohort.rs`/`schema_p05_unit*.rs` 扩展表、10 个领域子协调器（`coordinator_g5b_physical_seal.rs` 4839、`coordinator_p05_unit_runtime.rs` 2234 等）、`monitor_schema9.rs`(101，生产保留库只读校验）、`tests.rs`(14349)+20 余个专项测试文件。
- **生产调用链:** bin 侧适配器 `src/bin/monitor/durable_delivery_runtime.rs` → monitor 启动序列 `src/bin/monitor/main.rs:5796`(validate_runtime_delivery_mode)、`main.rs:5844`(eager_bind)、`main.rs:6162`(ensure_startup_reconciled)；P-01、复盘 R03-R13/A10/A01、holding_plan、news_ai 等 counted 推送经 `deliver_counted_binding` → MagiclawAuthoritativeSink(`durable_delivery_runtime.rs:1063`)。CLI:`src/bin/resolve_stale_uncertain.rs`、`src/bin/durable_schema14_snapshot_check.rs`。
- **活性:** CURRENT — 生产 monitor 启动即绑定 Production 命名空间并强制完成 startup reconcile(`main.rs:6162`)，生产库以 `open_existing_monitor_schema9` 打开（`durable_delivery_runtime.rs:2268`)，全部 counted 推送主流程实际可达。
- **问题/缺口:**
  1. 头注释过时：`src/durable_delivery/mod.rs:1-6` 仍称"deliberately does not wire a production provider, renderer or sink; production activation is an all-or-nothing follow-up"，但 bin 侧早已接线生产 sink(`durable_delivery_runtime.rs:1063-1160`)并在生产中运行 — 文档漂移。
  2. 库为 Schema14(`schema.rs:9`)，生产保留库锁死 Schema9 且禁迁移/禁修复（`monitor_schema9.rs:12-16`、`durable_delivery_runtime.rs:2268`);schema10-14 承载的 P05 Unit / G5B v2 等平台能力被 `frozen_platform_features_enabled()` 整体冻结（`durable_delivery_runtime.rs:1213-1215`，仅 Test namespace 为 true)，生产只用 v9 子集。
  3. `CountedDeliveryOrigin::InternalDurable` 注释（`durable_delivery_runtime.rs:215-218`）称"no production producer has migrated into it yet"，但生产路径 `main.rs:10087-10093` 已构造 InternalDurable binding，且 `validate_p01_text`(`durable_delivery_runtime.rs:440`）强制要求该变体 — 注释与实际不符。
  4. 巨型文件维护风险：`coordinator.rs` 443KB/10954 行、`tests.rs` 529KB/14349 行（非行为缺陷）。

##### `src/p05_auction_unit.rs`

- **职责:** P05 竞价候选 Unit 的可恢复观察门面：把 09:20-09:25 观察到的一批候选固化为不可变 intent(A-02 竞价重推 + P-05 候选台 + T-08 失效三类子投递）,restore/prepare/resume/finalize 全部经 coordinator CAS；不做源资格认定。
- **内部结构:** 单文件 372 行：`P05AuctionUnit` 句柄、`P05UnitObservation`/`P05ProspectiveObservation` 观察类型、`P05ChildDispatchView`/`P05PreparedChild` 子投递、`pub(crate) read_p05_outcome_unit`(`p05_auction_unit.rs:366`)。
- **生产调用链:** `src/bin/monitor/durable_delivery_runtime/p05_unit.rs` → `src/bin/monitor/push_templates/p05_shared_unit.rs::tick/initialize_before_window` → `main.rs:6578/6581`（常驻服务入口）与 `main.rs:11321`（盘前等待后）；子投递经 `src/bin/monitor/notify.rs:3162 push_p05_unit_child`。
- **活性:** CONDITIONAL — 主循环调用点存在，但 `p05_shared_unit.rs:23/35` 的 `frozen_platform_features_enabled()` 门在 Production namespace 恒为 false(`durable_delivery_runtime.rs:1213-1215`、148-150)，生产只打 "disabled reason=user_scope_frozen" 即返回；新建批次另限 09:20-09:25 时间窗（`p05_shared_unit.rs:126-133`)。全链路仅 Test namespace 实际运行。
- **问题/缺口:**
  1. 生产冻结：initialize/tick 在生产恒 no-op(`p05_shared_unit.rs:22-37`)，旧入口 `dispatch_auction_repush`/`dispatch_candidate_board` 也直接 blocked(`push_templates.rs:9871-9875`、`10442-10446`)→ A-02/P-05/T-08 三类推送生产实际不发。
  2. 模块内唯一生产活跃部分是 `read_p05_outcome_unit`(`p05_auction_unit.rs:366-372`)，经 `prediction_outcome_tracker.rs:512` 在盘后报告中可达（`push_templates.rs:12538`)；写入路径全冻结，生产读到的通常为空。

##### `src/p05_candidate_board_link.rs`

- **职责:** 只读跨库证据对账：把 prediction 库的 v2 候选台冻结与 counted 库的 CandidateBoard 卡片观察逐字段比对，产出 Absent/UnlinkedV1/FrozenOnly/VerifiedV2 链接结论；不做写入、准入或 sink 调用。
- **内部结构:** 单文件 198 行：`pub read_candidate_board_occurrence_link`(:71)、`pub(crate) link_candidate_board_snapshot`(:109)、`pub(crate) frozen_candidate_board_envelope`(:182)、`CandidateBoardOccurrenceLinkV1` 枚举（:32)。
- **生产调用链:** `link_candidate_board_snapshot` → `src/monitor/prediction_outcome_tracker.rs:12`(OutcomeTracker)→ `src/monitor/prediction.rs:23-27` re-export → `src/bin/monitor/durable_delivery_runtime.rs:6697 log_prediction_outcome_report` → `push_templates.rs:12538` 盘后复盘日报（仅非 test);`frozen_candidate_board_envelope` → `src/durable_delivery/coordinator_p05_unit.rs`。
- **活性:** CONDITIONAL — 批量核对路径经盘后 OutcomeTracker 报告生产可达；但 pub 主入口 `read_candidate_board_occurrence_link` 零生产调用方，仅测试引用（`durable_delivery/tests.rs:7724-7933`、`push_templates/p05_counted_producer_tests.rs:238/371`)。
- **问题/缺口:**
  1. pub 入口 `read_candidate_board_occurrence_link`(`p05_candidate_board_link.rs:71`）生产零调用，公共 API 与生产使用脱节。
  2. VerifiedV2 分支需 counted 卡片 + prediction 冻结双证据（`p05_candidate_board_link.rs:130-176`)，而上游 P05 卡片写入在生产被 user_scope_frozen 冻结 → 生产实际只能产出 Absent/UnlinkedV1，对账价值受限。
  3. 头注释（`p05_candidate_board_link.rs:1-5`）明示两次 SQLite 读非原子快照、pending 结论为保守值 — 设计内 caveat，非缺陷。

> **本组总评:** durable_delivery 库本体是生产 counted 投递的现役核心（启动 reconcile + 全量 counted 推送），但库头注释仍停在"未接线生产"的旧叙事，且生产库被锁在 Schema9、Schema10-14 的 P05/G5B 平台能力被 user_scope_frozen 整体冻结。因此 p05_auction_unit 与 p05_candidate_board_link 处于"接线完整、生产禁跑"状态，生产真正触达它们的只有 OutcomeTracker 盘后只读报告这条窄路径。

##### `src/trading/`

- **职责:** 交易执行抽象层——统一的 `TradeExecutionGateway` trait + `SimulatedExecutionGateway`（写 `stock_position`，原子落 `order_audit`)，以及独立虚拟盘账本 PaperLedger 全家桶（paper_ledger / paper_sell / paper_trade / paper_book_v2)。
- **内部结构:**（按规模）
  - `paper_book_v2_execution.rs`(5505)+ 测试（3579)——book v2 成交执行
  - `paper_ledger.rs`(3886)+ `paper_ledger_execution.rs`(1284)+ `paper_ledger_adjudication.rs`(1302)+ 测试（4461)——PaperLedger 账本核心
  - `paper_trade.rs`(2114)——BR-023 虚拟腿，只写 `paper_trades`，零写 `stock_position`;`portfolio_state` 供 bin/lib 共用
  - `paper_sell.rs`(1198)——BR-134 虚拟仓卖出闭环，复用 `pipeline::position_tracker::evaluate_sell_rules_with_net_return`
  - `paper_funding_review_v1.rs`(1260)+ codec/store（共约 3800 行）
  - `mod.rs`(724)——trait 定义 + SimulatedExecutionGateway 实现
- **生产调用链:**
  - monitor 主循环（`src/bin/monitor/main.rs`):`paper_sell` 盘中/盘后扫描（main.rs:10256/10398,import 于 158)、`paper_ledger_runtime` 账户指标与收盘结算（3067/10228/10304，受 env 门控）、`risk_adapter::print_startup_banner`(5875)、`paper_trade::portfolio_state`(push_templates.rs:28085)
  - `decision::intraday_monitor`(monitor 循环 10209–10253 驱动）用 `paper_ledger_runtime` + `paper_trade::PaperSignal`(intraday_monitor.rs:22-24)
  - `SimulatedExecutionGateway` 只在 `pipeline::position_tracker.rs:295` 构造 ← `pipeline/analyze.rs:1292` ← `AnalysisPipeline` ← `app::run_analysis`(src/main.rs:104,CLI)+ `app/schedule.rs:262`;**monitor 主循环不经过此路径**
  - `paper_snapshot_activation` 仅 `src/bin/paper_account_activate.rs`（运维 CLI)
- **活性:** CONDITIONAL——paper_sell 在 monitor 主循环可达但带 `PAPER_SELL_DISABLED` 关闭开关（main.rs:9810-9827）且依赖激活 epoch;`paper_ledger_runtime` 整条账户指标/结算链硬门控于 env `PAPER_LEDGER_ACCOUNT_BINDING`(paper_ledger_runtime.rs:13, main.rs:3067)
- **问题/缺口:**
  - `paper_engine`(927 行）已被 BR-211 从生产循环隔离，main.rs:10145 有断言保证 monitor 循环不含 `paper_engine::run_once(`(main.rs:10200-10206)——仅测试引用
  - `paper_funding_review_v1` 全家（v1+codec+store，约 3800 行）在 `src/trading` 之外**零生产调用方**(grep 全仓仅命中 trading 内部），属死代码
  - `paper_book_v2_execution`(5505 行，全模块最大文件）唯一非测试引用是 `src/performance/paper_decision_outcomes_v1.rs:10`，而后者除 `performance/mod.rs:9` 声明外**无任何生产调用方**——实质未接线
  - `SimulatedExecutionGateway::cancel_order` 是永拒绝桩（mod.rs:376-392,"simulated gateway does not support pending-order cancel")，仅用于审计留痕
  - 买卖路径对同一 `order_safety::validate` 用请求价和执行价重复校验两次（mod.rs:210-231 / 307-328)，逻辑冗余

##### `src/portfolio/`

- **职责:** 系统唯一 Position/Trade/Ledger 定义与纯函数 API——持仓、自选（STOCK_LIST 兼容）、T+1 锁仓、净值快照/权益曲线，底层走 SQLite(store.rs)。
- **内部结构:** `store.rs`(699,SQLite 读写）、`mod.rs`(265，公共类型+API)、`closing_valuation.rs`(134，收盘估值视图）、`user_position_snapshot.rs`(99，用户快照输入）
- **生产调用链:** monitor 主循环大量直连——`get_st_positions`(main.rs:1976)、`is_t1_locked`(12129)、`get_positions`(9378/13092)、`get_trade_history`(13489)、`get_equity_curve_as_of`(13501);`monitor::scanner`(main.rs:49 import,scanner.rs:283-284);`closing_valuation` 供 monitor 收盘估值与 `bin/run_closing_valuation.rs`;`user_position_snapshot` 供 `bin/import_user_position_snapshot.rs` 及 holding_plan;`live_rolling_sharpe` 供 `review/report.rs:166`
- **活性:** CURRENT——monitor 主循环、扫描器、收盘估值、CLI 导入工具均实际调用
- **问题/缺口:**
  - `get_st_positions` 文档注释自称"只返回 code 列表避免 deep clone"，实现却返回 `Vec<Position>`(mod.rs:159-166)——注释与实现漂移；且 is_st/star_st 永远默认 false,"真实来源待 broker 接入"(mod.rs:155-157),ST 提醒数据源实为空的
  - `Position.sector` 默认"其他"(mod.rs:42-44)，板块集中度检查（single_sector_max_pct）在未填板块时实质失效
  - `get_watchlist` 仍走 STOCK_LIST 环境变量兼容路径（mod.rs:117)，与"不再各自读环境变量"的模块头注释自相矛盾

##### `src/broker.rs`

- **职责:** 唯一实时行情网关 seam——`QuoteProvider` trait + `PublicQuoteProvider`(→ `data_gateway` 远端行情 + 涨跌停带校验）,fail-closed；提供 `execution_quote`（含 5s 新鲜度门）、ST 类型推断、`detect_and_register` 启动注册。
- **内部结构:** 单文件 213 行——`BrokerStType`/`BrokerSource`/`ExecutionQuote` 类型、`PublicQuoteProvider` 实现、`QUOTE_PROVIDER` OnceLock 注册点、测试用 `FreshTestQuoteProvider`
- **生产调用链:** monitor 启动时 `detect_and_register`(main.rs:6279,BROKER_SOURCE 仅支持 "public"，其他值硬报错）;monitor 盘中 ST 持仓取价（main.rs:1985);`decision::intraday_monitor` 候选买/卖取价（intraday_monitor.rs:254/546);`trading::paper_sell`、`trading::paper_trade`、`pipeline::position_tracker` 经此取执行价
- **活性:** CURRENT——monitor 主循环启动即注册，盘中取价路径每 tick 可达
- **问题/缺口:**
  - `execution_quote` 的 5 秒硬新鲜度门（broker.rs:151-159）对非连续竞价时段（午间休市、盘前）必然报错，调用方需自行规避时段
  - `BrokerStType::from_name` 纯靠名称前缀启发式（broker.rs:27-35)，与 portfolio 侧 ST 字段"默认 false 待接入"叠加，ST 判定链整体不可靠
  - `QUOTE_PROVIDER` OnceLock 禁止运行时替换（broker.rs:131-135)，上游行情源故障时进程内无法切换，只能重启

> **本组总评:** portfolio 与 broker 是 monitor 主循环的真实活跃底座，接线干净但 ST/sector 等数据面仍是占位实现；trading 体量大（46k 行）却冷热悬殊——paper_sell/paper_ledger_runtime/paper_trade 在 monitor 可达但受 env 门控与激活状态约束，paper_engine 已按 BR-211 隔离，而 paper_funding_review、paper_book_v2_execution+paper_decision_outcomes 约 9k 行实质零生产调用，是最主要的死代码堆积区；旧的 `SimulatedExecutionGateway` 写 `stock_position` 路径只剩 CLI/schedule 入口，与 PaperLedger 新账本的长期并存关系值得在架构层面收敛。

##### `src/selection/`

- **职责:** BR-174 receipt-verified schema-v2 事件级选股上下文 — 生产激活门槛、源接入、审计哈希链、结果（outcome）结算所有权。
- **内部结构:** `schema_v2.rs`(10288，纯校验/preimage 基础库）、`audit.rs`(3366)、`outcome_v2.rs`(2885，结算 owner)、`ingress_v2.rs`(2044)、`config_activation_v2.rs`(1982)、`acquisition_v2.rs`(1695)；另有 `process_bootstrap.rs`(862)、`trading_calendar_v2.rs`(820)。
- **生产调用链:** `src/bin/monitor/main.rs:5619` `bootstrap_selection_process()`（每次 monitor 启动无条件执行，解析 CLI + 评估激活门）→ `main.rs:5745` 得出 `selection_v2_enabled` → 门控三个运行点： 新闻管线初始化（`main.rs:5897`)、Track A 候选入池（`main.rs:9307`)、`OutcomeSettlementOwner::settle_tick`(`main.rs:7550`,post_session_review_scheduler 60s loop)。另有 CLI `src/bin/selection_activation_prepare.rs:156`。
- **活性:** CONDITIONAL — 受 `activation_gate.rs:42` 仓库内激活材料控制；当前 `config/selection/selection_activation.v1.json` effective_from=2026-09-28（已过）、board artifact expires=2026-10-22（未到，见 provider_board_bindings.v1.json)→ 当前应为 Enabled，但 **2026-10-22 后自动 fail-closed**(`activation_gate.rs:65-68`)。
- **问题/缺口:**
  1. 源接入/生成写入链未接入 monitor 运行时： `persistence_v2.rs:105` `commit_source_ingress` 全仓零生产调用方；新闻 loop 的 `candidate_ingest_from_news`(`src/bin/monitor/news_aggregator_init.rs:1302`）只经 `signal::push_recorder` 写旧 `pushed_stocks` 池，不写 selection-v2 表 → 结算 owner 没有新样本来源，outcome 侧实质只有 recovery/空转（与已知"outcome 仅 shadow"一致）。
  2. `database/selection_v2_generation_journal.rs` 仅被 `database/mod.rs:2817` 声明、无任何调用方 → `acquisition_v2.rs` 的生成侧类型（cadence receipt、freeze feed plan 等）无运行时消费者。
  3. `relation.rs`/`model.rs` 零外部调用方（`crate::selection::relation` 全仓仅自引用）;`admission.rs`/`features.rs`/`quality.rs` 仅通过 `config_activation_v2.rs:16-17` 的版本常量进入配置哈希，逻辑无运行时执行路径。
  4. `activation_gate.rs:17-19` 注释自述"生产库目前无 selection-v2 表（BR-180 未发布）是设计内状态" → outcome settle tick 每次大概率只产出空 summary。

##### `src/opportunity/`

- **职责:** 产业链挖掘 + 机会发现的遗留上下文；BR-174 后正式选股迁往 selection,mod.rs 声称只保留"仍有独立生产职责"的子模块。
- **内部结构:** `candidate_panel.rs`(1142)、`bom_kb.rs`(743)、`chain_mapper.rs`(617)、`impact.rs`(592)、`launch_gate.rs`(482)、`auction_agent.rs`(450)；另 `event_extractor/`(5 文件）、`scheduler.rs`(269)。
- **生产调用链:**（分子模块）
  - `launch_gate` → `src/bin/monitor/notify.rs:2327/2490-2492/2627/2853/3086/3144/3195/3254/3314/3347`（每条推送的 stage 门）+ 启动 banner(`main.rs:5868`)→ **CURRENT**。
  - `candidate_panel`/`candidate_state` → `src/durable_delivery/coordinator_p05_unit.rs:5`、`src/p05_auction_unit.rs:9`、`src/bin/monitor/push_templates.rs:4956/8075/17509`(merge/format/require_live_promotion,P-05 推送链）→ **CURRENT**。
  - `chain_mapper` + `real_alpha::Confidence` → `src/monitor/attribution.rs:17-20` → `main.rs:12246` 盘中 detector 扫描 G5a 同步归因 → **CURRENT**。
  - `scheduler` → `main.rs:1881`(`--push` CLI 的 `run_daily_pushes`)+ `manual_push.rs:1` → **CURRENT**(CLI 主流程）。
  - `event_extractor`、`impact`、`auction_agent`、`virtual_reason`、`bom_kb`、`hit_case`、`score`、`winrate`、`discover` → 全部零生产调用方 → **INACTIVE**。
- **活性:** 混合型 — 4 条活调用链与 9 个死子模块共存于同一 `mod.rs`。
- **问题/缺口:**
  1. `mod.rs:3-5` 声明"仅保留仍有独立生产职责的子模块"，实测仍挂 9 个零调用模块，声明与事实不符（event_extractor/impact 死代码为已知结论，此处补齐完整清单）。
  2. `real_alpha.rs` 仅 `Confidence` 类型被 `attribution.rs:20` 使用；`compute_real_alpha`(`real_alpha.rs:15`）零生产调用，只剩自测。
  3. `score.rs:104` `compute_dual_score`、`winrate.rs:90` `load_reason_winrate` 零调用；`winrate_simulator` bin 自行计算、不引用该模块。
  4. `auction_agent.rs:20` import `virtual_reason`，两个死模块互相引用，会在粗粒度 grep 中制造"被使用"的假象；`registry/mod.rs:3` 与 `strategy/v16_4/mod.rs:3` 均注明 VirtualReason 已被替代。

##### `src/signal/`

- **职责:** 市场事件类型底座（`MarketEvent`/`Direction`/`EventType`/simhash)+ `pushed_stocks` 推送入池记录器。
- **内部结构:** `market_event.rs`(343)、`push_recorder.rs`(229)、`mod.rs`(13)。
- **生产调用链:**
  - `market_event` → `src/news/aggregator/{mod,feed,raw_v2,projection_v2,classifier,source_event}.rs` + `src/news/{dispatcher,impact,stock_mapper,sink}.rs` → monitor `news_monitor_loop`(`main.rs:6729` 常驻 spawn)；注意全局新闻管线 init 被 `selection_v2_enabled` 门控（`main.rs:5897`)。
  - `push_recorder` → `src/decision/intraday_monitor.rs:1067-1422`(7 处，盘中 loop `main.rs:10207-10210`)、`push_templates.rs:4624/5151`、`news_aggregator_init.rs:1342` → monitor 常驻盘中 loop 无条件可达。
- **活性:** CURRENT — push_recorder 在盘中 loop 实际执行；market_event 经新闻管线流通（主消费者受 activation 门控）。
- **问题/缺口:**
  1. `mod.rs:9-10` 自述模块名是历史包袱（"market_event 可移到 event/ 顶层"的迁移至今未做）。
  2. `market_event` 的声明消费者之一是死代码 `opportunity::event_extractor`(`event_extractor/mod.rs:9,21,26`)；其生产主路径（新闻聚合器）随 selection-v2 activation 门控 — 2026-10-22 board artifact 过期、激活 fail-closed 后，聚合器不再初始化，`market_event` 将只剩 dispatcher/impact 的类型级引用。

> **本组总评:** selection 是架构上的"正式未来"(BR-174 schema-v2)，激活门当前处于 Enabled 窗口期，但运行时未闭环 — 源接入/生成写入链没接进 monitor，只有 outcome 结算在空转，且 2026-10-22 材料过期后整体 fail-closed。opportunity 是"半活半墓园": 推送门（launch_gate)、P-05 候选（candidate_panel/candidate_state)、调度（scheduler)、G5a 归因（chain_mapper）真活，其余 9 个 pub 子模块零生产调用方，与 mod.rs 自述不符。signal 小而活： push_recorder 是 pushed_stocks 池的实际写入者，market_event 是新闻管线的类型底座，但其最大生产消费者的开关握在 selection 激活门手里。

##### `src/decision/`

- **职责:** 策略决策引擎 — 盘中推送票池消费评分、做T建议、板块分档；外加一套 v1 投资决策 codec/schema 底座。
- **内部结构:** `intraday_monitor.rs`(1473，生产核心） / `t0_advisor.rs`(1142，生产） / `investment_decision_codec_v1.rs`(1617)+`pushed_candidate_scope_v1.rs`(920)+`candidate_scope_observation_v1.rs`(371)+`investment_decision_v1.rs`(341)（纯 schema/codec) / `sector_score.rs`(269，生产） / `action_fact_snapshot.rs`(593)+`trading_fact_denial.rs`(468)（仅自测） / v11-v12 决策台死代码簇（`decision_decide.rs`864、`decision_panel.rs`299、`decision_render.rs`323、`exclusion.rs`525、`pre_trade_filter.rs`363、`rotation.rs`308、`capital_verify.rs`289、`leader.rs`257、`live_plan.rs`327、`holding_plan.rs`、`layers.rs`)
- **生产调用链:** ①`src/bin/monitor/main.rs:10209-10253` monitor_loop 的 intraday_loop → `IntradayMonitor::tick`（每30s)/`evening_review`/`recover_saved_candidate_buys`;②`main.rs:9834` `prepare_t0_messages` ← `main.rs:12415` 主扫描循环 30s 做T分支 → T-05 推送；③`src/bin/monitor/push_templates.rs:3713` `grade_sectors` ← `load_sector_snapshot_real`(3712) ← I-01 盘中 dispatcher(3800-3806)
- **活性:** CONDITIONAL — 三条链都在 monitor 常驻进程内，但 intraday 链被两道闸控制：banner 风险上下文可得（`main.rs:10225-10247`)+ `PAPER_LEDGER_ACCOUNT_BINDING` env(`intraday_monitor.rs:179` `active_binding()` 未激活直接 Err;`recover_saved_candidate_buys` 未激活静默返回0,`:154`);t0/sector_score 链无条件可达
- **问题/缺口:**
  1. **v11/v12 决策台整簇死代码约 4.0k 行（占模块 35%)**:`decision_decide.rs`、`decision_panel.rs`、`decision_render.rs`、`exclusion.rs`、`leader.rs`、`rotation.rs`、`capital_verify.rs`、`pre_trade_filter.rs`、`live_plan.rs`、`holding_plan.rs`、`layers.rs` 外部调用方为 0（全库 grep 仅 mod.rs 声明与自测；注意 `src/bin/monitor/holding_plan.rs` 是 bin 本地同名模块，与 `decision::holding_plan` 无关）
  2. `action_fact_snapshot`/`trading_fact_denial` 零外部调用，`evaluate_production_trading_fact_denial`(`trading_fact_denial.rs:155`）只有自测
  3. 投资决策 codec 底座仅测试可达：`capture_pushed_candidate_scope`(`pushed_candidate_scope_v1.rs:87`）无生产调用方，仅 `_for_test` 变体被 `database/global_schema_*_tests.rs` 使用；`pushed_candidate_scope_v1.rs:268-269` cost/liquidity model 硬编码 `"unavailable_formal_*_not_delivered"`
  4. `risk/action_gate.rs:7` 注释声称 "holding_plan / t0_advisor / live_plan 在产出建议前调 authorize" — 实际三者中仅死代码 `live_plan.rs:13` 调authorize；生产 t0_advisor 不经过 action gate，注释与实现漂移
  5. BINDING_ENV 未设置时 tick 每 30s 刷一条 warn(`main.rs:10253`)，无降噪

##### `src/risk/`

- **职责:** 决策硬约束层 — 账户模式评估、环境隔离、实时否决链、纸交易门控；与 `monitor/risk.rs`（实时计算）并存。
- **内部结构:** `veto_rules_live.rs`(636)+`veto_chain.rs`(403)+`veto_execution_engine_v1.rs`(346)+`veto_execution_report_v1.rs`(607+552测试） / `account_mode.rs`(567) / `action_gate.rs`(260)+`cash_guard.rs`(84) / `limits.rs`(254，死） / `stop_loss.rs`(193) / `env_guard.rs`(132) / `sector_exit.rs`(93，死）
- **生产调用链:** ①`env_guard` 被全库 ~40 处生产调用（`bin/monitor/main.rs:698`、`durable_delivery_runtime.rs:1218`、`event/dispatcher.rs:338` 等）;②`account_mode::evaluate/evaluate_with_reset` ← `main.rs:2794/2924` evaluate_account_mode_hook（常驻 banner 刷新）;③VetoChain ← `src/pipeline/analyze.rs:634-686` `apply_configured_veto` ← `analyze.rs:859-878` ← `AnalysisPipeline` ← `src/app/modes.rs:53`/`schedule.rs:262`(CLI 主流程）;④`action_gate::authorize` ← `trading/paper_ledger_execution.rs:617`（生产函数，非测试）+ `trading/risk_adapter.rs`（其 `pre_trade_check` ← `paper_trade.rs:1221` ← intraday_monitor tick);⑤`stop_loss::check_stops` ← `pipeline/position_tracker.rs:27`(CLI analyze,env `POSITION_TRACKING_ENABLED` 默认开， `analyze.rs:1262`)
- **活性:** CURRENT（核心）；否决链需特别修正：**证据表明它不是死代码** — analyze pipeline 实接线，且 checked-in `config/strategy.toml:43-45` 显式 `[live_veto] enabled=true mode="live"`,`analyze.rs:649-671` 在 live 模式下真实拦截（force_hold 压分到 55/改 BuySignal::Hold);action_gate 链为 CONDITIONAL（随 BINDING_ENV 激活）
- **问题/缺口:**
  1. `limits.rs` 整文件死代码（254行）：唯一真实导入方是死代码 `decision_decide.rs:33`;`limits.rs:38` 注释自述现金底线已迁移 `cash_guard`
  2. `sector_exit.rs`(93行）零外部调用方，仅自测
  3. 双风控链并存无统一入口：CLI analyze 走 VetoChain,monitor 盘中走 risk_adapter 4 项检查（`intraday_monitor.rs:253`),VetoChain 不覆盖 monitor 实时路径
  4. live 否决的 fail-closed 分支（报告构造失败压分到 55,`analyze.rs:668-671`）只在 CLI 路径生效；`veto_execution_report_v1` 另有消费者是死/休眠的 `decision/investment_decision_codec_v1.rs:6`

##### `src/strategy/`

- **职责:** 策略接口与实现层 — K线/基本面策略 trait、回测引擎、盘中 v16_4 八策略打分、反向择时/布林MACD 信号；另含 research v2 与 model-change 草稿两套 codec。
- **内部结构:** `core.rs`(1910,BacktestEngine/Summary) / `rsi/`(2972:standard 1394+precision 769+common 309)+`bollinger_zscore.rs`(772)+`multi_factor.rs`(321) / `v16_4/`(573:mod 209+8 个 33-46 行 impl) / `boll_macd.rs`(460)+`contrarian.rs`(142)+`multi_timeframe.rs`(292)+`lot.rs`(104) / `research_run_descriptor_v2.rs`(952)+`research_portfolio_v2.rs`(604)+`research_fill_v2.rs`(576) / `model_change_draft_v1.rs`(387)+store(576)+tests(897)
- **生产调用链:** ①回测三件套 ← `pipeline/backtest_runner.rs`(`pub(super)` 方法） ← `src/pipeline/mod.rs:610/640/658` 在 `AnalysisPipeline::run` 非 dry_run 时执行 ← CLI `app/modes.rs:53`;②`boll_macd` ← `analyze.rs:744/1416`、`position_tracker.rs:28`、`trading/paper_sell.rs:31`、`offline_products/sell_reminder.rs:5`、bin `boll_macd_backtest.rs:20`;③`contrarian` ← `analyze.rs:1253`;④`multi_timeframe::MinuteBar` ← `pipeline/multi_timeframe.rs:10` → analyze mtf_section;⑤`v16_4` ← `decision/intraday_monitor.rs:17-21`,`register_all()` 在 monitor 启动 `main.rs:6301`;⑥`rsi::common::compute_sma_vec` ← `trend_analyzer.rs:351`
- **活性:** CURRENT — CLI 分析主流程 + monitor 盘中打分均实际可达；v16_4 打分的下游成交受 BINDING_ENV 门控（见 decision 卡）;bin/rsi_optimize、boll_macd_backtest 为手工研究工具
- **问题/缺口:**
  1. **research v2 三件套（2132行）+ model_change_draft 两件套（963行+897测试）零生产调用方** — 全库 grep 仅 strategy 内部互引与自测，无生产 writer/reader 接线
  2. `HybridStrategy` 及 `KlineStrategy`/`FundamentalStrategy` trait object 层（`mod.rs:121-236`）从未被实例化：`HybridStrategy::builder()` 无调用方，backtest_runner 直接用具体类型，聚合器是纯粹脚手架
  3. `register_all()`(`v16_4/mod.rs:41`）在 monitor 启动无条件执行，但 8 策略的 `score` 只在 BINDING_ENV 激活后才会被 tick 消费 — 注册与消费之间存在静默空转窗口
  4. `mod.rs` 头部架构图（3-19行）未提 v16_4/research/model_change 三组后加模块，文档漂移

##### `src/breakout/`

- **职责:** 多维度放量启动识别引擎（盘中 Gateway 事实模式 + 盘后 K 线模式）。
- **内部结构:** `engine.rs`(351) / `signal.rs`(108) / `position.rs`(42)，共 510 行
- **生产调用链:** **无** — `crate::breakout` / `stock_analysis::breakout` 全库 grep 零匹配；仅 `src/lib.rs:15` `pub mod breakout` 导出。注意名字撞车：`strategy::v16_4::breakout::BreakoutStrategy`(34行）是另一个模块，与本引擎无关
- **活性:** INACTIVE — 零生产/零测试调用方，2026-08-16 后无改动（git mtime 与全仓 8 月冻结一致）
- **问题/缺口:**
  1. 510 行完全死代码，`mod.rs:1-5` 自述的"盘中统一引擎"职责实际由 `decision/intraday_monitor.rs` + `strategy/v16_4` 承担
  2. 与 `strategy/v16_4/breakout.rs` 同名易混淆，删除前需确认无外部 crate 依赖（lib.rs 公开导出）

> **本组总评:** 四个模块合计约 27.5k 行，真实生产活水集中在三条链：monitor 的 `intraday_monitor`+`t0_advisor`+`sector_score`(env 门控的纸交易链）、CLI analyze 的 VetoChain+position_tracker+boll_macd/contrarian（实拦截已开 live)、以及 strategy 回测三件套；其余约 40% 是 v11/v12 决策台、research/model_change codec、breakout 引擎等成建制死代码或仅测试可达的休眠底座。两处与既有认知冲突的证据需复核：`risk::veto_chain` 并非死代码（`config/strategy.toml:45` `mode="live"` 真拦截，`analyze.rs:649-671`),`decision::sector_score` 亦非死代码（`push_templates.rs:3713` ← I-01 生产 dispatcher `push_templates.rs:3806`)。建议优先删除/归档 decision 死簇（~4k 行）+ breakout(510 行）+ strategy research/model_change 休眠层（~3.9k 行），并修正 `action_gate.rs:7` 与 `strategy/mod.rs` 头部的漂移注释。

##### `src/news/`

- **职责:** v15.1 业务核心——统一全球新闻采集（raw_v2 四源收敛）、通知投影（projection_v2)、分类（classifier)；BR-166/BR-172/BR-174 体系。
- **内部结构:** `aggregator/raw_v2.rs`(2995行，采集核心）、`aggregator/source_event.rs`(841)、`aggregator/classifier.rs`(652)、`aggregator/analyst_state.rs`(393)、`dispatcher.rs`(185)+`impact.rs`(190)+`sink.rs`(67)+`stock_mapper.rs`(198，均为 v15 旧链路）、`ipo/supply_chain.rs`(191)。
- **生产调用链:** `src/bin/monitor/news_aggregator_init.rs:37` + `src/bin/monitor/main.rs:5898`（启动注册） → `news_monitor_loop`(main.rs:8991）逐 tick 采集 → 消费方为 `src/monitor/news_ai.rs:18`、`src/monitor/push_job/n02_source.rs:15`、`src/selection/ingress_v2.rs:9`；`classifier`/`analyst_state` 经 `src/bin/monitor/v17_sources.rs:19-24` 路由；`ipo::supply_chain` 被 `src/bin/monitor/push_templates.rs:19265` 直接用于 IPO 推送。
- **活性:** CONDITIONAL — `main.rs:5897-5906` 明确仅当 `selection_v2_enabled` 时才初始化 BR-174 采集管线（注释： "only initialize … after selection-v2 activation is released")，开关来自 CLI(main.rs:5745)；`ipo` 与 v17 classifier 部分无条件活跃。
- **问题/缺口:**
  - `dispatcher.rs:62 decide()`、`sink.rs:11 install()`、`stock_mapper.rs`、`impact.rs:73 score_event()` 在 src 内零外部调用方（仅互相引用+测试），即已知死代码，仍 `pub mod` 挂在 mod.rs:15-18 形成"业务核心"假象。
  - `news_aggregator_init.rs:25-28` 模块级 `#![allow(dead_code)]`，自承"BR-183 forbids constructing a production receipt while that capability is disabled"——receipt-gated 投影整段保留未启用。
  - `aggregator/feed.rs` 的 `NewsFeed` trait 已被 mod.rs:50-52 自我标注为"not the BR-174 acquisition scheduler"的兼容遗留，生产仅 raw_v2 直连。

##### `src/announcement.rs`

- **职责:** 公告标题分类（AnnLevel 紧急/重要/利好/跳过）+ CNInfo EventCalendar 批次的保证据投影（BR-059/105/137/168)；采集归 data_gateway。
- **内部结构:** 单文件 219 行：`Announcement`/`AnnouncementBatch` 模型、`project_event_calendar_batch`(:78)、`classify_title`(:108)、`extract_reduction_pct`(:135)。
- **生产调用链:** `src/bin/monitor/main.rs:9499`(`news_monitor_loop` 公告分支，`market_announcements` 拉取后投影） → `src/monitor/news_monitor.rs:9`(NewsMonitor 索引处理） + `src/bin/monitor/v17_sources.rs:325 route_announcement_batch`（归一化路由推送）。
- **活性:** CURRENT — `news_monitor_loop` 在 monitor 主循环无条件 spawn(main.rs:6729)，公告分支不受 selection-v2 门控。
- **问题/缺口:**
  - 投影只保留标题/时间/URL，`name`/`summary`/`content` 恒空（announcement.rs:90-94,CNInfo 不供详情），下游若有按名称/正文匹配的需求会静默落空；`main.rs:9548` 还需 `resolve_code_by_name(&ann.name)` 对空 name 做补救查询。
  - `classify_title` 纯关键词包含匹配（announcement.rs:108-133),`extract_reduction_pct` 用字符切分猜百分比（:135-147)，对"减持不超过0.5%"类表述靠特判豁免，脆弱但已有测试兜底（:213)。

##### `src/search_service/`

- **职责:** 统一新闻/网页搜索聚合（Tavily/SerpAPI/Bocha 经 data_gateway 薄适配），多 provider 轮询、缓存、快讯事实投影。
- **内部结构:** `service.rs`(2184，聚合器+`get_search_service` 单例：1638)、`types.rs`(712)、`macro_news/legacy.rs`(703)+`macro_news/runner.rs`(561)、`providers/general_web.rs`(123,BR-175 无网络适配）、`macro_news.rs`(322)。
- **生产调用链:** `src/pipeline/mod.rs:381` + `src/pipeline/analyze.rs:18`(CLI 单次分析主流程） → `src/pipeline/chain_analysis/mod.rs:751` / `preparation.rs:1569`（产业链扫描，monitor `news_monitor_loop` 8:00-22:00 窗口经 `main.rs:11138 run_chain_analysis`) → `src/market_analyzer/mod.rs:15`（涨停复盘） → `src/agent/tools_news.rs:2`（深度研判工具）。
- **活性:** CURRENT — 生产 monitor 与 CLI 均可达；可用性取决于搜索 key env，缺失时 `pipeline/mod.rs:386` 仅降级提示不阻断。
- **问题/缺口:**
  - `macro_news/runner.rs`+`legacy.rs`(1264行）的生产消费方只有 `push_foundation/intent_store/chain_post_close*.rs`，而 monitor 对 chain_post_close 只有 shadow 观察（`main.rs:10495 observe_chain_post_close_shadow`，日志自承 `foundation_persisted=false`,main.rs:10501),`src/bin/monitor/` 内无非 shadow 调用——这一千多行实质是 shadow-only。
  - `opportunity/event_extractor/*` 引用 `SearchResult`(mod.rs:8）但该模块已知死代码，构成虚引用。
  - `service.rs` 2184 行单文件，缓存/健康度/快讯投影/宏快讯混居，guard 注释（:4-8）显示历史审计债。

##### `src/llm/`

- **职责:** 多 Provider LLM 抽象（DeepSeek/MiniMax,OpenAI 兼容协议），按 role 选择 + 强制 provider 回执（ModelCallReceipt）绑定的证据化模型调用。
- **内部结构:** `bounded.rs`(1155，有界调用）、`mod.rs`(565,trait+receipt 协议）、`providers.rs`(437)、`ticker_extractor.rs`(211)、`registry.rs`(135,env 装配）。
- **生产调用链:** `src/bin/monitor/news_ai_shadow.rs`("BR-172 默认启用，2026-08-11 起取消 env 开关") → `src/monitor/news_ai.rs:13`(news_monitor_loop 内 NewsAI 分析）;`src/bin/monitor/g5b_v2.rs:173`+`src/monitor/g5b_analysis_v2.rs:14`+`src/monitor/attribution_deep.rs:19`(g5b 盘后归因，main.rs:11000 `run_tick`);`src/bin/monitor/news_aggregator_init.rs:1306-1314`(BR-183 候选入池 extract_tickers);`src/durable_delivery/coordinator_g5b_cohort.rs:7`。
- **活性:** CURRENT — NewsAI 默认开启且在 `news_monitor_loop` 每 tick 调度（main.rs:9350);g5b/深度归因走 `g5b_should_analyze` 15:05–15:20 时间窗（main.rs:181-184)，属正常定时窗而非禁用。
- **问题/缺口:**
  - `bounded.rs` 1155 行（全模块最大文件）在 src/monitor、src/bin/monitor 内零引用，唯一生产形态是离线 bin `src/bin/assistant_review.rs`("Offline by default")——对常驻 monitor 而言是离线工具代码。
  - `registry.rs:38-43` 只装配 DeepSeek/MiniMax 两家，与 mod.rs:1 宣称的"DeepSeek / MiniMax / OpenAI / Gemini / Claude 可插拔"不符，扩展承诺未兑现。
  - provider 缺失时各消费方各自 fail-open/fail-closed 语义不一（news_aggregator_init.rs:1308 warn 跳过 vs news_ai ModelUnavailable 错误），无统一降级策略。

##### `src/analyzer/`

- **职责:** AI 个股分析器（Gemini/DeepSeek/豆包三家 HTTP 封装），技术面+消息面生成分析报告；Quick/Deep 双模式路由；宏观新闻荐股。
- **内部结构:** `analyze.rs`(1196，主分析+`run_text_pipeline` 宿主）、`client.rs`(572，三家 HTTP 重试）、`mod.rs`(528,GeminiAnalyzer+env 装配）、`prompts.rs`(447)、`macro_rec.rs`(267，宏观 4 专家）、`types.rs`(231)。
- **生产调用链:** CLI `src/main.rs` → `app/modes.rs:53 AnalysisPipeline` → `pipeline/mod.rs:372` 装配 `ai_analyzer` → `pipeline/analyze.rs:913` 逐股调用；宏观推荐：`app/bootstrap.rs:458 analyze_macro_recommendations`(macro_rec.rs:78)；深度研判复用见 agent 卡片；另 `decision/leader.rs:51`、`opportunity/chain_mapper.rs:226` 直接 `from_env`。
- **活性:** CURRENT — CLI 主流程与 monitor 复盘 AI 均可达。
- **问题/缺口:**
  - **装配门控不一致**: `pipeline/mod.rs:372` 只在 `GEMINI_API_KEY` 存在时才创建 `GeminiAnalyzer::from_env()`，而 from_env 本身支持 DeepSeek/豆包（analyzer/mod.rs:329-341)——只配 DEEPSEEK/DOUBAO key 时 pipeline 整段 AI 分析静默缺失；但 `app/bootstrap.rs:442` 的宏观推荐走 `get_analyzer()` 单例不受此限，同进程两条路径行为分裂。
  - `prompts.rs` 整个模块被 `#[allow(dead_code)]` 保留（analyzer/mod.rs:19-23,"legacy JSON dashboard parser retained")，属明示死代码。
  - `GeminiAnalyzer` 用 `RefCell` 持可变状态（mod.rs:87-88,`!Sync`)，迫使 monitor 复盘 AI 在 `push_templates.rs:11432-11441` 为每只股票 spawn 独立 current-thread runtime，是真实的架构债。

##### `src/agent/`

- **职责:** Agent 工具体系（财务/研报/资金/筹码/板块/新闻 6 工具）+ 多角色分析流水线（6 分析师→多空辩论→仲裁）+ 旧 ReAct 单 Agent 循环。
- **内部结构:** `loop_runner.rs`(715,AgentRunner)、`multi_agent/slices.rs`(1087)+`analysts.rs`(408)+`arbitrator.rs`(285)+`cost_board.rs`(270)+`debate.rs`(189)+`trace.rs`(190)、`tools_*.rs`(6 个工具文件）、`validation.rs`(157)、`auction_agent.rs`(124)。
- **生产调用链:** **multi_agent（活）**: `pipeline/mod.rs:468/475`(`enrich_key_stocks_with_deep_analysis`，门控 main 流程 `ai_analyzer.is_some()` 且非 dry_run,mod.rs:602)+ `src/bin/monitor/push_templates.rs:11438`（盘后复盘 AI,fail-open 于无 key,11402-11406)+ `app/modes.rs:30`/`bin/deep_analyze.rs:34`(CLI)——均经 `deep_analyzer.rs:405 run_multi_agent_analysis` → `build_slices` → `run_text_pipeline`(multi_agent/mod.rs:38);**tools_sector（活）**: `pipeline/chain_analysis/fetchers.rs:1141 render_membership_batch`（产业链扫描）;**loop_runner**: 仅 `bin/agent_test.rs:136`(dev 工具）与 `deep_analyzer.rs:352 run_react_analysis`（自注"旧路径，保留备用"，零生产调用方）。
- **活性:** CONDITIONAL — multi_agent 与工具层真实接线但受 AI key env + 重点股过滤（pipeline/mod.rs:430-440)+ 复盘开关多重门控；`loop_runner`/`auction_agent` 实质闲置。
- **问题/缺口:**
  - **文档与实现矛盾**: `multi_agent/mod.rs:4-6` 头注声称"已从个股分析路径中摘除……待改造为宏观专属后再启用"，且模块级 `#![allow(dead_code)]`(mod.rs:19)，但实际被 pipeline/mod.rs:468 与 push_templates.rs:11438 生产调用——头注是过期信息，会误导后续维护者误判死活。
  - `agent/auction_agent.rs`(v12 MVP-4 §7.8）零调用方（grep 全仓库无 `agent::auction_agent` 引用；活跃竞价逻辑在 `opportunity/auction_agent.rs`),124 行死代码。
  - `AgentRunner`(715 行，最大单文件）生产上只剩 deep_analyzer 的备用路径引用，等于靠 dev bin 续命；`validation.rs`/`context.rs`/`toolbelt.rs` 随之仅在 ReAct 旧路径可达。

> **本组总评:** 六个模块全部真实接线、无一整体闲置，但活性成色分层明显——announcement 与 search_service 主干、llm+NewsAI 是 monitor 每日实跑的核心；news 的 BR-174 采集链与 agent 的 multi_agent 分别受 selection-v2 激活和 AI key env 门控（CONDITIONAL)。共同的债务模式是"新旧链路并存、旧件靠 `#[allow(dead_code)]` 续命":news 的 dispatcher/impact/sink/stock_mapper、llm 的 bounded.rs(1155 行仅离线 bin)、analyzer 的 prompts.rs、agent 的 loop_runner/auction_agent, 合计约 3000+ 行死/半死代码；另有 `multi_agent/mod.rs` 头注与实现直接矛盾、`pipeline/mod.rs:372` 只认 GEMINI_API_KEY 两处会导致维护者误判的具体缺口。

##### `src/data_gateway/`

- **职责:** BR-164 证据保留型生产数据网关，统一持有全部外部金融/新闻获取；gRPC 桥（`grpc_source`）fail-closed 接远端 market-data host，绝不静默回退 library。
- **内部结构:**（49k 行，60+ 文件） `grpc_source.rs`(7055 行，gRPC 桥核心）、`benchmark.rs`(3884)、`review.rs`(3377，统一准入/证据/审计）、`outcome_daily_bars.rs`(2847)、`qualified_trading_facts.rs`(1043)、`board.rs`(1144)；另有 `grpc_source/convert.rs` 与约 15 个 `*_tests.rs`。
- **生产调用链:** 135 个外部文件 import。`src/bin/monitor/main.rs:9878`(`bridge_for("T0Evidence")`)、`main.rs:12493`(BoardDataGateway)、`main.rs:9485`(EventCalendarGateway 新闻 loop)、`main.rs:5354`(refresh_position_chains);`src/monitor/news_monitor.rs`/`scanner.rs`/`data_quality.rs`/`news_ai.rs`;`src/pipeline/data.rs`/`analyze.rs`/`chain_analysis/*` → monitor 主循环 + 各 scheduler loop + CLI 分析链路。
- **活性:** CURRENT — monitor 开盘 readiness(`main.rs:4701` `external_static_opening_diagnostics`）与 T0/资金流/板块采集均实际经此桥，启动 banner(`grpc_source.rs:2458`）在 monitor 启动时打印。
- **问题/缺口:**
  - `grpc_source.rs` 单文件 7055 行/266KB，承载全部 op 桥接+准入+审计，是仓库最大单文件之一，可维护性风险高（`src/data_gateway/grpc_source.rs:1`)。
  - CFFEX 期货交割 live 能力不可用：`cffex_futures_delivery_live_supported()` 在 monitor 启动时走 warn 分支，EventCalendar sink 保持 blocked(`src/bin/monitor/main.rs:5757-5763`)。
  - 公告分页"断点 B-2"未彻底修复：上游 max_pages 未放开，客户端以 limit=300 变通覆盖全天（`src/bin/monitor/main.rs:9486-9491` 注释）。
  - `OutcomeDailyBarsGateway` 仅服务 selection outcome_v2 shadow 结算，`settle_tick` 在 `selection_v2_enabled` 门内运行（`src/bin/monitor/main.rs:7545-7552`，与已知"outcome 仅 shadow"一致）。
  - `benchmark.rs`(3884)/`review.rs`(3372)/`outcome_daily_bars.rs`(2847）均为巨型文件；`benchmark` 的生产消费方主要是回测/归因链路（`pipeline/backtest_runner.rs`、`performance/attribution_replay.rs`)，非盘中主路径。

##### `src/grpc_client/`

- **职责:** gRPC 客户端网络层（合同 grpc-external-api.md),`GrpcMarketClient` 实现 24 个 op 查询、能力准入、错误分类保真与 mTLS/bundle 身份。
- **内部结构:**（32k 行） `client.rs`(2114，客户端+#[path] 子模块挂点）、`errors.rs`(2602,status→错误映射）、`external_query_transport.rs`(1136)、`external_v1.rs`(931)、`candidate_probe.rs`(2069)、`envelope.rs`(431);loopback fixture 与大型 *_tests 均为 `#[cfg(test)]`(`client.rs:41-79`)。
- **生产调用链:** `data_gateway/grpc_source.rs:39,2878`(`GrpcMarketClient::connect` 桥接全部 gateway op);`push_foundation/intent_store/chain_post_close_{macro,board,dragon_tiger}*` → `src/monitor/push_job.rs` → monitor 主循环；`search_service/macro_news/*` → monitor 新闻链路；诊断 bin(`grpc_bundle_probe` 等）。
- **活性:** CURRENT — 生产桥（grpc_source）与 push_foundation 盘后 macro/board RPC 均经此层；`external_decoder`/`historical_external` 在 macro codec 生产路径解码归档 descriptor(`chain_post_close_macro_codec.rs:2`)。
- **问题/缺口:**
  - `candidate_probe.rs` 2069 行+2629 行测试仅服务诊断 bin `grpc_candidate_bundle_probe.rs` 与 client 内部（`client.rs:44`)，属探针工具而非生产采集路径。
  - `errors.rs` 2602 行/99KB，错误分类 taxonomy 过于庞大集中。
  - `archived_external_20260928.rs`/`archived_external_20261001.rs` 为冻结 descriptor 兼容层，仅被 `build_identity.rs:96-101` 与 `external_decoder.rs:35-38` 引用做归档识别，属只增不减的历史包袱。

##### `src/grpc_contract/`

- **职责:** gRPC 合同注册表——方法身份（Local/External 双 profile)、schema 名/版本冻结、请求 params 契约、payload 校验；服务端与客户端共享。
- **内部结构:**（~1.1k 行） `methods.rs`(183,ContractProfile/MethodIdentity)、`schema.rs`(266,24 个生产 op 冻结 schema)、`params.rs`(407，默认值表与 monitor 生产惯例对齐）、`ops.rs`(168)、`validate.rs`(91，未知 schema/version 必须拒绝）。
- **生产调用链:** 被 `grpc_client/envelope.rs:6`、`errors.rs`、`client.rs`、`unary_attempt.rs` 及 `data_gateway/grpc_source.rs:46`(`ExternalMethod`)import → 随 grpc_client 进入 monitor 全部 gRPC 路径；`news/aggregator/raw_v2.rs` 亦直接引用。
- **活性:** CURRENT — 每次 gRPC query 信封构造/响应校验都经 `schema_for`/`validate_payload`，是桥的硬依赖。
- **问题/缺口:** 未发现明显问题（模块小、职责单一；schema 注释自述"后续扩展至 41 op"属计划而非缺陷，`schema.rs:4-6`)。

##### `src/market_domain/`

- **职责:** 纯领域类型层——K 线/证据/标的标识/生命周期/榜单/数值约束类型，无 IO 无逻辑副作用。
- **内部结构:**（~1.9k 行） `evidence.rs`(355,SourceEvidence)、`lifecycle.rs`(346)、`record.rs`(306,DataBatch/Provenance)、`bars.rs`(290)、`market.rs`(217)、`value.rs`(163，受限数值类型）。
- **生产调用链:** 128 个文件 import(`ProviderId`/`InstrumentId`/`LimitPoolEntry` 等遍及 pipeline、monitor、database、grpc_source);monitor 直接消费见 `src/bin/monitor/main.rs:966,10039`。
- **活性:** CURRENT — 是全仓数据流的词汇表，生产与测试共同依赖。
- **问题/缺口:** 未发现明显问题（`tdx.rs` 仅 21 行但 `SecurityBar` 被 grpc_source 使用，非死代码）。

##### `src/data_provider/`

- **职责:** 兼容层——保留 `KlineData` 领域模型与本地派生计算（筹码分布、一致预期投影、涨跌停校验）；外部获取已统一上交 data_gateway,`service.rs` 是进程级 TTL 缓存单飞层。
- **内部结构:**（2.2k 行） `service.rs`(662,`DataFetchService` 缓存）、`limit_status.rs`(519)、`chip_distribution.rs`(470)、`mod.rs`(305,`KlineData`/`AdjustType`)、`halt_status.rs`(158)、`news_item.rs`(50)。
- **生产调用链:** 58 个文件 import。`DataFetchService::service()`(`service.rs:327`)← `src/agent/tools*.rs`、`src/pipeline/extra_context.rs`;`KlineData` 遍布 monitor(`main.rs:909-966`)、pipeline、database → monitor 主循环与 ReAct Agent 工具。
- **活性:** CURRENT — KlineData/缓存层是分析与交易链路的数据载体；但含一个死子模块（下条）。
- **问题/缺口:**
  - `halt_status.rs` 158 行零调用方：全仓库（含测试）除 `mod.rs:8` 声明外无任何 `halt_status`/`HaltStatus` 引用，确认死代码。
  - `mod.rs:120-253` 内置约 130 行 `#[cfg(test)] TestHttpServer` 基建，测试设施堆在模块根部（其他模块已移到独立 fixture 文件）。

> **本组总评:** 五个模块全部活性在线（除 data_provider/halt_status 一个死子模块），构成"gRPC 合同 → 客户端 → 网关 → 领域类型"的完整生产数据链，monitor 开盘 readiness、T0、盘后 push_job 均实际可达。主要风险是规模失控而非失活：grpc_source.rs(7055 行）、review/benchmark/outcome_daily_bars（各 3-4k 行）、errors.rs(2602 行）等巨型文件集中了过多职责；另有 CFFEX 交割、公告分页（断点 B-2）两处上游能力缺口仍靠变通维持。

##### `src/monitor/` (lib 层）

- **职责:** 实盘监控核心库——盘中异动检测/信号状态机/预测闭环/新闻 AI/G5b 归因/数据健康降级，供 `src/bin/monitor` 常驻进程消费。
- **内部结构:** `news_ai.rs`(4546) + `news_ai/`;`attribution_deep.rs`(3093);`data_mode.rs`(1489) 数据健康门；`data_quality.rs`(1139);`news_monitor.rs`(1042);`prediction.rs` 经 `#[path]` 吸收 prediction_samples/verifier/horizons/outcome_tracker 四个姊妹文件；另有 `g5b_*_v2` 系列、`alert_log.rs`(871)、`scanner.rs`(714)、`detector.rs`(669)、`push_job.rs`+`push_job/`。
- **生产调用链:** `src/bin/monitor/main.rs:43-51` import detector/prediction/scanner/signal_state → `monitor_loop`(main.rs:10192)、`news_monitor_loop`(main.rs:8991)、`data_mode_monitor_loop`(main.rs:3623);`attribution::apply_attribution`(main.rs:12246)、`signal_fusion`(main.rs:11375)、`event_bus`(main.rs:6275)、`outcome_data`(main.rs:7603);`alert_log`/`g5b_*` ← `src/durable_delivery/coordinator*.rs`;`push_job` ← `src/notification/service.rs:17`、`src/pipeline/cli_identity.rs:7`;`risk` ← `src/pipeline/position_tracker.rs:26`;`data_quality` ← `src/pipeline/data.rs:21`、`src/bin/monitor/freshness.rs:14`。
- **活性:** CURRENT — 主循环+新闻循环+durable delivery 均实际可达，`news_ai` 经 `news_ai_shadow` BR-172 默认启用（`src/bin/monitor/news_ai_shadow.rs:1`)。
- **问题/缺口:**
  - `alert.rs`(321 行）零生产调用方，仅 `integration.rs:40,101,129`(`#[cfg(test)]`）引用；生产告警走 `detector::AlertEvent` + push_templates 直发。
  - `auction.rs` 唯一非测试引用是 `src/opportunity/auction_agent.rs`，而后者自身零调用方（`auction_agent::` 全仓无外部匹配）,agent/opportunity 链路未接入 bin/monitor。
  - `adaptive.rs`(276 行）与 `checklist.rs`(259 行）全仓零引用，仍挂在 `mod.rs:16,22`。
  - `integration.rs` 是纯 `#[cfg(test)]` 文件（`integration.rs:6`）却占着一个 mod 槽位（`mod.rs:32`)。
  - `mod.rs:1` 头注释宣称 "Phase 0-5 全部实现" 并列出 alert/checklist 等，与上述死代码现状不符。
  - `news_ai.rs` 单文件 4546 行，可维护性差。

##### `src/market_analyzer/`

- **职责:** 大盘指数/涨跌统计/板块榜概览 + 涨停池获取 + 板块/连板监控，供盘中与盘后推送使用。
- **内部结构:** `limit_up.rs`(1351) 涨停池 Gateway 投影；`sector_monitor.rs`(1115) 板块榜；`market_stage_confidence.rs`(410);`review.rs`(374) 模板复盘；`async_overview.rs`(304);`mod.rs`(171) MarketAnalyzer 门面。
- **生产调用链:** `MarketAnalyzer::get_limit_up_stocks` ← `src/bin/monitor/main.rs:11116`（产业链 15:00 断点 A);`get_limit_up_observation` ← main.rs:11728（盘中涨停池）+ `push_templates.rs:8445`（竞价量能）;`sector_monitor` ← `push_templates.rs:2298,3714,18706`;`limit_chain_review` ← `main.rs:13270`(R-03 涨停池装载）+ `push_templates.rs:4337,15173`;`get_market_overview` ← `src/app/modes.rs:95`(CLI `--market_review` 模式）。
- **活性:** CURRENT — 涨停池/板块/连板路径在 monitor 主循环实际可达；但约 1/3 文件为死代码（见下）。
- **问题/缺口:**
  - `async_overview.rs`(304 行）: `mod.rs:41` re-export 两个 blocking 函数，全仓零调用方。
  - `market_stage_confidence.rs`(410 行，v12 MVP4-4.1)、`post_close_review.rs`(165 行，MVP4-4.4)、`performance_feedback.rs`(254 行，MVP5-5.1）三个 v12 规划件均零调用方。
  - `review.rs`(374 行）仅被 `run_daily_review`(mod.rs:165）调用，而 `run_daily_review`/`search_market_news` 全仓零调用方——整条"大盘复盘报告"链已死。
  - BR-164: `north_flow` 恒为 `None` 且每次调用都 warn(mod.rs:85-86)，属已知契约缺口。
  - `sector_monitor` 同时被已知死代码 `src/decision/sector_score.rs`、`src/decision/leader.rs` 引用，不影响活性结论。

##### `src/review/`

- **职责:** 盘后复盘纯消费者——R-03~R-13 各复盘任务的计算/渲染库，读 portfolio + data_provider，输出报告文本。
- **内部结构:** `backtest.rs`(1932, R-12);`journal.rs`(672);`watchlist_tracking.rs`(669, R-13);`catalyst_review.rs`(641, A-10);`factor_ic.rs`(637)+`factor_report.rs`(230);`equity.rs`(286)+`report.rs`(257)；其余 R-xx 小件。
- **生产调用链:** R-12 ← `push_templates.rs:11919,11986`(`dispatch_r12_backtest_outcome`，经 `main.rs:5444` review_batch 调度）+ `src/pipeline/backtest_runner.rs`;R-13 ← `push_templates.rs:12336,12354,19555`;R-07 ← `push_templates.rs:10685`;`catalyst_review` ← `push_templates.rs:8659`(pub use)+ `src/bin/backfill_catalyst_watchlist.rs`。
- **活性:** CONDITIONAL — 只有 R-07/R-12/R-13/catalyst 四个任务面经 review_batch 调度真实可达；R-05/R-06 dispatcher 被 BR-110 显式禁用，journal/equity/report 仅测试可达，其余模块零接线。
- **问题/缺口:**
  - `journal.rs`/`equity.rs`/`report.rs`（合计 ~1215 行）唯一调用方是 `src/bin/monitor/main.rs:13484` 的 `#[cfg(test)] fn build_close_review_report`（注意 main.rs:13483 的 cfg 属性）——生产不可达。
  - `factor_ic.rs:285` `run_diagnostic` 文档自称"用于 --review 复盘路径"，实际全仓零调用方；`factor_ic`↔`factor_report` 互相引用（`factor_ic.rs:288`）形成死岛。
  - R-05 禁用： `push_templates.rs:15942-15947` `dispatch_r05_signal_review_real` 恒返回 false(`disabled=no_signal_..._source`, BR-110) → `signal_review.rs` 死。
  - R-06 禁用： `push_templates.rs:15951-15956` 同构 → `failure_attribution.rs` 死。
  - R-04 dispatcher 直接用 `DragonTigerGateway`(`push_templates.rs:15737-15749`)，从未调用 `lhb_review.rs` → 死。
  - `market_stage.rs`/`limit_chain_review.rs`/`performance_feedback.rs` 仅内部互相引用（`limit_chain_review.rs:5`、`performance_feedback.rs:5`)，无外部入口；`sop.rs` 零调用。死/禁用合计约 3700/6378 行（~58%)。

##### `src/performance/`

- **职责:** 绩效与归因引擎——每日结算快照、epoch 归因计算/重放、经济头寸（净盈亏）、费率策略与证据。
- **内部结构:** `attribution_replay.rs`(10721，占模块 58%);`attribution.rs`(1756);`economic_position.rs`(1662);`attribution_epoch.rs`(1108);`fee_policy.rs`(837)+`fee_evidence.rs`(520);`report.rs`(648)+`report/paper_account.rs`;`snapshot.rs`(449)。`attribution_effective.rs` 不在 mod.rs，经 `#[path]` 被 `attribution_replay.rs:19` 吸收。
- **生产调用链:** `PerformanceEngine::daily_settlement` ← `main.rs:10563`(monitor_loop 内 15:05 cron);`attribution::{compute_epoch_daily,persist_epoch_daily}` + `report::render_*` ← `main.rs:10594-10598`(15:05-15:20 重试窗口）;`attribution_replay::ReplayErrorClass` ← `main.rs:10694`;`attribution_epoch` ← `src/bin/monitor/attribution_epoch_runtime.rs:8`，接线于 `main.rs:6684` 启动校验；`economic_position::compute_economic_position_report` ← `main.rs:3128`;`snapshot` ← `src/bin/monitor/health.rs`、`src/trading/paper_ledger_snapshot.rs`;`fee_policy`/`fee_evidence` ← `src/trading/paper_book_v2*.rs`、`src/strategy/core.rs`、`src/pipeline/position_tracker.rs`。
- **活性:** CURRENT — 15:05 结算+归因 cron、启动 epoch 校验、经济头寸均在 monitor 主流程实际运行。
- **问题/缺口:**
  - `paper_decision_outcomes_v1.rs`(297 行）仅被 `src/trading/paper_book_v2_execution_tests.rs:3376+` 引用，生产零调用，仍 `pub mod` 挂在 `mod.rs:9`。
  - `attribution_replay.rs` 单文件 10721 行，为全 src 最大文件之一，维护风险高。
  - `fee_policy`/`fee_evidence` 的主要消费面在 trading paper_book 链（paper_engine 已被 BR-211 禁用），生产可达性现依赖 strategy research 与 CLI `position_tracker`，属半接线状态。

> **本组总评:** `src/monitor/` lib 是生产主干（检测/新闻/归因/数据健康全部在线），但拖着 `alert`/`auction`/`adaptive`/`checklist` 四块约千行死肉且头注释失真；`market_analyzer` 与 `review` 均有三分之一到一半是 v12 规划后未接线或被 BR-110 显式禁用的死代码（review 的 journal/equity/report 甚至只剩 cfg(test) 调用方）;`performance` 最健康、全部 cron 接线真实，唯需警惕 10.7k 行的 `attribution_replay.rs` 巨型文件和 test-only 的 `paper_decision_outcomes_v1`。

##### `src/pipeline/`(24,840 行，含测试）

- **职责:** 股票分析流程调度器（BR-213)，协调 数据获取 → 趋势分析 → AI 分析 → 通知推送；并承载产业链联动分析 `chain_analysis`。
- **内部结构:** `mod.rs`(1137 行，`AnalysisResult` god-struct + 调度主循环）;`analyze.rs`(~110k，单股分析主逻辑）;`chain_analysis/`(mod 108k + fetchers 64k + preparation 86k + p01_projection 47k + 测试 134k);`backtest_runner.rs`(68k，多因子/布林带/RSI 三回测）;`position_tracker.rs`(51k，模拟持仓+卖出铁律）;`summary_notify.rs`/`reporting.rs`/`score_breakdown.rs`/`veto_rules.rs` 等支撑件。
- **生产调用链:** `src/main.rs:104` → `app::run_analysis` → `src/app/modes.rs:53` `AnalysisPipeline::new` → `run_with_producer`；定时模式 `src/app/schedule.rs:262`;monitor 进程在 `src/bin/monitor/main.rs:11138` 直接调 `pipeline::chain_analysis::run_chain_analysis`,`p01_projection` 被 `src/bin/monitor/p01.rs:15` 引用。
- **活性:** CURRENT — CLI 单次/定时模式与 monitor 产业链 loop 均可达。
- **问题/缺口:**
  - `analyze.rs` 仍 ~2900 行，是遗留巨石；`position_tracker.rs`(51k）为 `pub(super)`(mod.rs:26)，除 analyze.rs 外仅被已禁用的 paper_sell 和离线 sell_reminder 使用。
  - `score_to_advice` 已标注 "AI评分已禁用"(src/pipeline/mod.rs:306-314)，但深度研判优先级 `key_stock_priority`(src/pipeline/mod.rs:319）仍用 ranking_score/veto/涨停打分，sentiment_score 仍在摘要排序（modes.rs:73)，新旧评分体系并存。
  - 三个回测在每次非 dry_run 分析后必跑（src/pipeline/mod.rs:608-671)，失败仅 `error!` 记日志不阻塞，回测开销与主分析耦合。
  - 深度研判预算由 env `DEEP_ANALYSIS_MAX`（默认 15)、`DEEP_ANALYSIS_CONCURRENCY`（默认 3）控制（src/pipeline/mod.rs:421-453)，无配置文件入口。

##### `src/app/`

- **职责:** CLI 应用层：`bootstrap` 启动校验+自选股装配，`modes` 三种运行模式（单次/大盘复盘/龙虎榜）,`schedule` 定时调度，`chain_*` 产业链调度的采集/审计/shadow 指纹。
- **内部结构:** `modes.rs`(45k);`chain_schedule.rs`(44k，调度状态机）;`chain_shadow_input.rs`(38k，只读指纹）;`chain_acquisition.rs`(35k);`bootstrap.rs`(23k);`chain_cli_send_audit.rs`(19k);`schedule.rs`(11k)。
- **生产调用链:** `src/main.rs:80/86/91/99/102/104` → `app::{run_scheduled_analysis, run_chain_analysis_mode, build_stock_list, run_lhb_analysis, run_market_review_only, run_analysis}`;monitor 在 `src/bin/monitor/main.rs:36` 引 `chain_schedule`;`push_foundation/chain_preopen_shadow.rs:11`、`chain_post_close_shadow.rs:6` 同引。
- **活性:** CURRENT(CLI 主入口直达）;`chain_shadow_input` 为 **CONDITIONAL/shadow** — 自述 "Read-only fingerprint … not Foundation shadow parity",`COVERAGE = "incomplete"`(src/app/chain_shadow_input.rs:1,17)。
- **问题/缺口:**
  - 并发默认值不一致：`app::get_max_workers` 默认 6(src/app/mod.rs:30),`PipelineConfig::default` 默认 3(src/pipeline/mod.rs:249)，实际生效取前者，后者成摆设。
  - `schedule.rs` 进程内常驻 cron，但生产 launchd 仅常驻 monitor binary(plist ProgramArguments 仅 monitor),`stock_analysis --schedule` 模式无 plist，生产可达性依赖手工/外部触发。

##### `src/notification/`

- **职责:** 日报 Markdown 生成 + 10 渠道推送（企微/飞书/邮件/Telegram/Pushover/钉钉/Slack/Discord/Custom/Server酱）。
- **内部结构:** `service.rs`(53k,`NotificationService` 主入口）;`report.rs`(45k，日报渲染）;`feishu.rs`(23k);`email.rs`(14k);`wechat.rs`(9.9k);`send_report_tests.rs`(41k，测试）。
- **生产调用链:** `src/pipeline/summary_notify.rs:7`、`src/pipeline/analyze.rs:1360+`（单股推送）、`src/app/modes.rs:103/267`、chain 链路 `src/app/chain_shadow_input.rs:7`;lib 侧 `src/monitor/alert.rs:7`。
- **活性:** CURRENT(CLI 分析推送主链路）；注意 monitor bin 并不 import notification（自有 feishu_delivery 体系），且 `monitor/alert.rs` 的推送扩展无调用方（见下）。
- **问题/缺口:**
  - `send_daily_report`(src/notification/service.rs:960）全仓零调用方，是死的便捷 API。
  - `src/monitor/alert.rs:160` 对 `NotificationService` 的告警级别路由扩展：`push_alert` 全仓 grep 零调用方，属死接线。
  - `report.rs` 单文件 45k，与 service.rs 同为巨石。

##### `src/indicators/`

- **职责:** MACD(6/13/5)/SKDJ(40/5)/RSI/KDJ 计算、顶底背离检测、金叉/死叉共振，输出 0-100 综合评分。
- **内部结构:** `mod.rs`(567 行，`analyze_indicators` 综合入口）;`kdj.rs`/`macd.rs`/`rsi.rs`/`skdj.rs`/`cross.rs`/`divergence.rs`;`multi_period.rs`（日/周/月聚合）。
- **生产调用链:** `src/trend_analyzer.rs:341` `analyze_indicators`;`src/strategy/boll_macd.rs:18`(calc_macd/detect_divergence);`src/strategy/multi_timeframe.rs:15`;`src/agent/multi_agent/slices.rs:7`(calc_skdj)。
- **活性:** CURRENT。
- **问题/缺口:**
  - `kdj.rs::calc_kdj` 全仓零生产调用（仅 mod.rs:41 re-export + 内部测试）——实际生产用的是 SKDJ，注释自述 "藏于只 kdj_* 字段名中以保持下游 API 兼容"(src/indicators/mod.rs:72)，典型兼容包袱+死代码。
  - `analyze_indicators` 在数据 <30 根时静默返回默认 50 分（src/indicators/mod.rs:144-147)，下游无感。

##### `src/trend_analyzer.rs`(1047 行）

- **职责:** 趋势交易分析器：均线多头排列、乖离率（不追高）、量能形态、买卖点信号，输出 `TrendAnalysisResult`。
- **内部结构:** 单文件；`TrendStatus` 七态枚举、`BuySignal`、`StockTrendAnalyzer`、内嵌指标分析调用。
- **生产调用链:** `src/pipeline/mod.rs:51,263`(`Arc<StockTrendAnalyzer>`)+ `src/pipeline/analyze.rs` 多处；次要调用方 `src/trading/paper_sell.rs:33`(BR-211 已禁用）、`src/offline_products/sell_reminder.rs:7`。
- **活性:** CURRENT(pipeline 主链每只股票必经）。
- **问题/缺口:** 次要调用方均非生产主路径（paper_sell 已被 BR-211 禁用；sell_reminder 仅 preview/producer 离线 bin)，模块活性完全系于 pipeline 一处。

##### `src/deep_analyzer.rs`(832 行）

- **职责:** 机构级多智能体深度研判：6 工具取数（财务/研报/资金/筹码/板块/新闻）→ DomainSlices → 6 分析师+多空辩论+仲裁，落盘 `reports/details/`。
- **内部结构:** 单文件；`run_multi_agent_analysis`（主）、`run_multi_agent_analysis_with_seed`（复用主流程种子）、`run_react_analysis`（备用）、`run_and_save`;K 线 freshness gate(BR-010)。
- **生产调用链:** `src/pipeline/mod.rs:468/475`（重点股 enrich);`src/app/modes.rs:30`(`--deep-analysis`);`src/bin/deep_analyze.rs:34`;monitor 复盘 `src/bin/monitor/push_templates.rs:11438`（持仓 AI 研判）。
- **活性:** CURRENT — CLI 与 monitor 复盘双路可达。
- **问题/缺口:**
  - `run_react_analysis`(src/deep_analyzer.rs:352）自述 "保留备用"，全仓零生产调用，死路径。
  - 失败/超时（300s）仅保留标准分析静默降级（src/pipeline/mod.rs:486-493)。
  - monitor 复盘路径为每只持仓单起 current-thread runtime(BR-242 注释，push_templates.rs:11430+),90s 超时，LLM 调用密集。

##### `src/chart_generator.rs`(162 行）

- **职责:** 用 plotters 生成汇总 PNG（评分 TOP15 柱状图 + 操作建议分布条）。
- **内部结构:** 单文件单 struct;`generate_summary_chart` + 两个私有绘制函数。
- **生产调用链:** 唯一调用点 `src/pipeline/summary_notify.rs:89`(CLI 汇总通知）。
- **活性:** CONDITIONAL — 在汇总通知路径上执行，但返回值赋给 `_chart_path` 被丢弃，`chart_path: None`(src/pipeline/summary_notify.rs:88-89,239)，图片不并入任何通知。
- **问题/缺口:** 生成结果无人使用（下划线丢弃）;"sans-serif" 字体在无中文字体环境标题/标签会缺字形；无 env 开关，白跑 IO。

##### `src/sharpe_calculator.rs`(136 行）

- **职责:** 夏普比率计算 + 全系统统一无风险利率常量（默认 3%，可 env 覆盖）。
- **内部结构:** 单文件；`DEFAULT_RISK_FREE_RATE`、`calculate_sharpe_ratio`、近 N 日封装、K 线回填函数。
- **生产调用链:** 生产代码仅 `src/strategy/core.rs:1134` 引用常量；`calculate_sharpe_ratio` 只被本文件内部（:101/:106/:134）调用。
- **活性:** CONDITIONAL — 常量活、计算主体死。
- **问题/缺口:** `calculate_sharpe_ratio` 及 :134 的 K 线 sharpe 回填全仓零外部调用，整个计算能力是死代码；`AnalysisResult.sharpe_ratio` 的取值来源在别处（data_provider),`src/portfolio/store.rs:412` 还另写了一份 rf=0.03 的并行实现。

##### `src/company_financials.rs`(482 行）

- **职责:** BR-115/137/168/210 财务指标域模型与保守投影（缺失字段保持 absent)，含 `assess_quality` 财务质量评估。
- **内部结构:** 单文件；`FinancialPeriod`、`Financials`、`FinancialProjectionEvidence`、`QualityReport`/`assess_quality`、`project_income_statements`。
- **生产调用链:** `src/pipeline/analyze.rs:144/949`、`src/analyzer/analyze.rs:636`、`src/agent/multi_agent/slices.rs:679`、`src/data_provider/service.rs:20`、monitor `src/bin/monitor/v17_sources.rs:1062`。
- **活性:** CURRENT。
- **问题/缺口:** 未发现明显问题（纯域投影，契约清晰）。

##### `src/company_metrics.rs`(41 行）

- **职责:** BR-119/120 行业对标与估值历史的纯值对象（`IndustryBenchmark`/`ValuationHistory`)，不含取数。
- **内部结构:** 单文件两个 struct。
- **生产调用链:** `src/data_provider/mod.rs:22`（装配）→ `src/pipeline/analyze.rs:298/395`(render)、`src/analyzer/analyze.rs`、`src/agent/multi_agent/slices.rs`、`src/pipeline/veto_rules.rs:186`。
- **活性:** CURRENT（纯类型，被主分析链贯穿）。
- **问题/缺口:** 未发现明显问题。

##### `src/capital_flow.rs`(208 行）

- **职责:** BR-118 资金流域投影：`MoneyFlowDay/Summary`、`IntradayShape`、`format_for_prompt`；取数归 `data_gateway`，本模块只做投影。
- **内部结构:** 单文件；两个核心 struct + prompt 渲染。
- **生产调用链:** `src/pipeline/extra_context.rs:37`、`src/pipeline/score_breakdown.rs:9`、`src/pipeline/veto_rules.rs:19`、`src/risk/veto_rules_live.rs:149`、`src/agent/tools_money_flow.rs:2`、`src/data_provider/service.rs:19`。
- **活性:** CURRENT。
- **问题/缺口:** `MoneyFlowDay.pct_chg` 刻意不做 join(capital_flow.rs:20-25)，需要涨跌幅的下游必须另查已审计价格批次——设计性缺口（注释已声明，但 veto 规则等消费方需自行注意）。

##### `src/lhb_analyzer.rs`(202 行）

- **职责:** BR-162 龙虎榜复盘纯打分（`analyze_dragon_tiger_review`)+ CLI 日期解析；不含取数/缓存/DB。
- **内部结构:** 单文件；`LhbAnalysis`、`parse_dragon_tiger_date`、打分函数。
- **生产调用链:** `src/app/modes.rs:1031` `run_lhb_analysis` ← `src/main.rs:99`(`--lhb-mode` 或 `LHB_MODE=true`);`src/bin/lhb_query.rs:12`;`src/lib.rs:98` re-export。
- **活性:** CONDITIONAL — 仅 CLI lhb_mode 可达，非每日主流程，无自动调度。
- **问题/缺口:** 无 launchd/schedule 接线，纯手工触发；打分结果不落库（纯返回 `LhbAnalysis`)。

##### `src/assistant_review.rs`(1487 行）

- **职责:** H16 周度证据复盘的只读 Phase-A 三臂对照（离线模板 / 无历史结果模型 / 含冻结结果模型），有界 LLM 调用 + 回执验证 + 全链路哈希一致性，产出只读 Markdown。
- **内部结构:** 单文件；`FrozenPack`（冻结证据包校验）、`compare`（三臂执行+预算预留）、`markdown`（渲染）、`read_private`/`write_new_private`（私有文件 IO)；约一半为测试。
- **生产调用链:** `src/bin/assistant_review.rs`（唯一调用方）← launchd `com.stockanalysis.weekly-outcome-review` → `run-weekly-outcome-review.sh` 以 `--assistant-binary assistant_review` 间接调用。
- **活性:** CONDITIONAL — 经周度 launchd 间接可达，但 bin 自述 "Offline by default; explicit --model enables reviewed bounded assistant_review role only"(src/bin/assistant_review.rs:1)，无 `--model` 时仅确定性模板臂在跑。
- **问题/缺口:** 花费上限由 CLI 参数 `--ceiling-micro-cny` 决定（assistant_review.rs:26-33)，非配置审计；monitor 主进程无内嵌调用，价值依赖人工是否启用 --model。

##### `src/assistant_review_integration.rs`(244 行）

- **职责:** assistant_review 的跨任务生产者/消费者集成测试（合成 fixture，无生产语义）。
- **内部结构:** 单文件测试；`LocalFake` provider + 跨 weekly_outcome_review 组件的端到端用例。
- **生产调用链:** 仅被 `src/assistant_review.rs:36-37` 以 `#[cfg(test)] #[path] mod integration` 引入。
- **活性:** INACTIVE — 纯 `cfg(test)` 模块，生产构建不编译、零运行入口。
- **问题/缺口:** 未发现明显问题；但文件名 "integration" 易让读者误以为存在生产集成点，实际只是测试。

> **本组总评:** 这一组构成 `stock_analysis` CLI 的分析主链路（main.rs → app → pipeline → trend_analyzer/indicators/company_*/capital_flow → notification)，整体 CURRENT 且被 monitor 进程选择性复用（chain_analysis、deep_analyzer、company_financials)。主要缺口集中在"半死代码":sharpe_calculator 只剩常量活着、chart_generator 出图即弃、indicators/kdj.rs 零调用、monitor/alert.rs 的 NotificationService 扩展零调用、deep_analyzer 的 ReAct 备用路径闲置；另有一批模式（schedule 定时、lhb、assistant_review 模型臂）依赖手工或低频 launchd 触发，活性为 CONDITIONAL。pipeline/analyze.rs 与 notification 双 45k+ 巨石仍是最大的可维护性风险。

###### 顶层支撑散文件

##### `src/calendar.rs`

- **职责:** A股交易日历与时段门控（交易日判断、集合竞价/连续竞价/午休时段、上/下一交易日），并内嵌 BR-194/BR-251 的"已核验 SSE 日历"不可变权威（带 SHA-256 authority hash）供审计回放用。
- **内部结构:** 运行时节假日表 `HOLIDAYS`(:85，含 `config/a_share_market_holidays.csv` + `TRADING_HOLIDAYS` env)；不可变 `VERIFIED_TRADING_CALENDAR`(:128,fail-closed);`MarketSession` 枚举（:41);replay 校准段（:145-214)；测试（:797-1170)。
- **生产调用链:** `stock_analysis::calendar::{is_trading_day,current_session}` → `src/bin/monitor/main.rs:6444,9046,9289`（主循环时段门控）、`src/bin/monitor/health.rs:81-86`（健康检查日期解析）、`src/monitor/scanner.rs:165`、`src/app/bootstrap.rs:142`(CLI)。
- **活性:** CURRENT — monitor 主循环每个 tick 都过 `current_session()`/`is_trading_day()`。
- **问题/缺口:**
  - 双日历并存：可变 `HOLIDAYS`(RwLock，可被 env/运行时注入）与不可变 `VERIFIED_TRADING_CALENDAR` 语义不同，调用方需自选，存在误用面（src/calendar.rs:85 vs :128);
  - `verified_a_share_calendar_authority_hash` 为 `pub(crate)` 且仅服务于紧急回滚（:589)，覆盖面窄；
  - 日历覆盖年仅到 checked-in CSV 所列年份，越界 fail-closed(:574)，跨年需手工更新 CSV。

##### `src/config.rs`

- **职责:** 启动配置加载 — `load_all()` 单次读 `config/strategy.toml`(Risk/Monitor 投影）+ `config/chain.toml`（产业链规则/公告关键词/排除板块/BR-160 合同）,ArcSwap 全局快照分发。
- **内部结构:** `MonitorConfig`(:180，含 news/dq/opportunity/live_veto/position_sizing/factor_feedback/air_refuel/v17_7_earnings);`ChainRuleConfig`(:69);`AccountModeConfig`(:556);5 个 `ArcSwap` 静态（:498-507);`load_all`(:736)。
- **生产调用链:** `config::load_all()` 仅被 `src/bin/monitor/main.rs:5987` 调用；读取方 `get_monitor_config()` 遍布 monitor 主循环（:662,9106,11458,12925)、`src/monitor/news_monitor.rs:124`、`src/search_service/service.rs:14` 等。
- **活性:** CURRENT — monitor 启动即加载，热路径全量消费。
- **问题/缺口:**
  - **stock_analysis CLI 路径从不调 `load_all()`**(grep 全仓仅 monitor main 一处）,CLI 模式下 `app/modes.rs:40`、`app/schedule.rs:200` 拿到的是编译期默认 `MonitorConfig` 而非 strategy.toml 实际值；
  - `load_risk_config` 已 deprecated 但仍导出（src/config.rs:725-731);
  - chain.toml 读取/解析失败时 rules/boards "保留先前快照"(:678-694)，首次启动即失败则规则为空，依赖 `chain_mapper` 端显式 unavailable 报错兜底。

##### `src/auth/`

- **职责:** CLI operator 密码认证（BR-028/BR-074),PAM 验证，3 次失败/无 TTY → exit 1;`MONITOR_AUTH_REQUIRED=1` opt-in 启用，默认禁用。
- **内部结构:** `mod.rs`(6 行，仅导出头注释）;`operator.rs`(240 行，`require_monitor_operator_auth` :79、`load_auth_config` :52、PAM 封装 :138)。
- **生产调用链:** `src/main.rs:30`(CLI 启动闸）+ `src/bin/winrate_simulator.rs:101`。
- **活性:** CONDITIONAL — 默认 `MONITOR_AUTH_REQUIRED` 未设即跳过（operator.rs:81-89)；且**生产 monitor 进程（src/bin/monitor）根本不调用它**,`mod.rs:3` 注释声称 "monitor 启动前调用" 与实际不符。
- **问题/缺口:**
  - 文档漂移：src/auth/mod.rs:3 与 src/auth/operator.rs:4 均写 "monitor … 启动前调用"，实际 grep 仅 main.rs:30 与 winrate_simulator.rs:101 两个手动入口，常驻 monitor 无认证闸；
  - 默认禁用 + eprintln 提醒（:84）依赖 operator 看 stderr,launchd 场景该提醒进 stderr log 易被淹没。

##### `src/bus/`

- **职责:** v16.4 的 SignalBus/TradingBus/SystemBus 三个 broadcast channel 骨架 + UUID 风格 ID 生成器（SignalId/OrderId 等）。
- **内部结构:** 3 个事件枚举（:27-80);5 个 ID 生成器（:96-110);3 个 `OnceCell` 全局 bus(:122-166);6 个单测（:168-271)。
- **生产调用链:** 唯一生产 publish 在 `src/trading/paper_engine.rs:350`(TradingBus)，但 paper_engine 已被 BR-211 禁用；`new_*_id` 被 paper_engine(:337-339）与 v16_4 的 `impl_strategy_id!` 宏（src/strategy/v16_4/mod.rs:88-97）使用；monitor bin 内零引用。
- **活性:** INACTIVE — mod.rs:15-17 自承 "仅落地 3 Bus 骨架，业务接入推 v16.4 #5";SignalBus/SystemBus 零生产 publish/subscribe,TradingBus 唯一发布者已禁用（与已知结论一致）。
- **问题/缺口:**
  - 骨架空转：无消费者，publish 即丢弃（src/bus/mod.rs:133 `let _ = self.tx.send(event)`);
  - v16.4 #5 接入计划（SignalBus→intraday_monitor 等，:16-17）从未落地，intraday_monitor 实际直调 strategy 结构体。

##### `src/registry/`

- **职责:** v16.4 StrategyRegistry — DashMap 动态注册 StrategyMeta(name/version/virtual_reason)，替代 v16.3 硬编码 8 enum 的运行时路由设想。
- **内部结构:** `StrategyMeta`(:21);`StrategyRegistry`(:31,register/lookup/list_active/deactivate);`register_v16_3_strategies`(:118);5 单测。
- **生产调用链:** 注册入口实际是 `src/strategy/v16_4/mod.rs:41 register_all()` → `src/bin/monitor/main.rs:6301`(monitor 启动）；唯一生产读取方 `src/bin/monitor/health.rs:37`(`list_all().len() >= 8` 健康判据）。
- **活性:** CONDITIONAL — 启动时确实注册、健康检查确实读，但 registry 从不参与信号路由：生产 dispatch 在 `src/decision/intraday_monitor.rs:433-440` 用硬编码 match 直接实例化 8 个 strategy，绕过 registry 的 lookup/list_active。
- **问题/缺口:**
  - 注册即终点：`lookup`/`list_active`/`deactivate` 无生产消费者，"动态注册/热加载" 设计目标（:5-6）未实现；
  - `register_v16_3_strategies`(:118）生产零调用（仅自身测试 :180)，与 `register_all` 功能重复的死 API;
  - 健康判据 `len() >= 8`(health.rs:37）只证明注册动作发生过，不证明策略可用。

##### `src/production_root.rs`

- **职责:** 构建期锁定生产数据根（`STOCK_ANALYSIS_BUILD_PRODUCTION_ROOT` env 或 CARGO_MANIFEST_DIR)，运行时 env/CWD 不参与；test/prod 命名空间分流。
- **内部结构:** `select_build_root`(:4，绝对路径+纯 normal component 校验）;`production_root()`(:28);`root_for_mode`(:35);2 单测。
- **生产调用链:** 被 `src/bin/monitor/main.rs`、`src/durable_delivery/coordinator*.rs`、`src/selection/config_activation_v2.rs`、`src/data_gateway/historical_observed_store.rs`、`src/event/dispatcher.rs` 等 ~29 处引用。
- **活性:** CURRENT — monitor 与 durable delivery/selection 激活路径每次启动都经它定位数据根。
- **问题/缺口:** 未发现明显问题（纯函数 + option_env，测试覆盖路径校验齐全）。

##### `src/util.rs`

- **职责:** 三个跨模块小工具：`truncate_chars`(char 级截断）、`strip_html_tags`（简单高亮标签剥离）、`recover_lock_or_warn`(Mutex poison 恢复）。
- **内部结构:** 单文件 45 行，3 个 pub fn。
- **生产调用链:** `src/notification/service.rs`、`src/monitor/news_monitor.rs`、`src/push_l7/analytics.rs`、`src/push_l7/sqlite_store.rs`(`src/opportunity/event_extractor/mod.rs` 是已知死代码）。
- **活性:** CURRENT — 通知发送与新闻监测热路径在用。
- **问题/缺口:** 未发现明显问题。

##### `src/http_client.rs`

- **职责:** 3 个预配置共享 reqwest Client（默认 30s / 通知禁跳转 30s / 快讯 5s)，替代 28 处散落 builder(Top10#7 修复）。
- **内部结构:** 单文件 52 行，3 个 `Lazy<Client>` 静态（:20/:32/:44)。
- **生产调用链:** `src/notification/service.rs`、`src/push_l6/external_sinks.rs`、`src/bin/monitor/webhook_alert.rs`。
- **活性:** CURRENT — 通知/告警外发通道在用。
- **问题/缺口:** 未发现明显问题（.no_proxy() 与禁跳转语义都有注释说明）。

##### `src/models.rs`

- **职责:** Diesel ORM 模型 — stock_daily/lhb_daily/analysis_result/stock_position/trades/ledger 六表的 Queryable/Insertable/AsChangeset 结构 + `MaStatus` 均线形态分析。
- **内部结构:** `StockDaily`/`NewStockDaily`/`UpdateStockDaily`(:26-104);`LhbDaily`(:217);`AnalysisResultRecord`/`NewAnalysisResult`(:242-285);`StockPosition`(:312);`TradeRecord`(:359)。
- **生产调用链:** `src/database/mod.rs`、`src/database/repository.rs`、`src/trading/paper_engine.rs`、`src/pipeline/position_tracker.rs`;lib.rs:94 对外的 `pub use models::{...}`。
- **活性:** CURRENT — 所有 DB 读写路径的载体。
- **问题/缺口:**
  - `trades` 表 schema 已有 commission/stamp_tax/slippage/realized_pnl/strategy_tag/signal_id 六列（src/schema.rs:155-160)，但 `TradeRecord`/`NewTradeRecord`(:359-383）未映射这些列，归因字段在 ORM 层不可达；
  - `paper_trades`/`execution_tracking`/`position_adjustments`/`account_mode_log` 四表在 schema.rs 有定义（:90-191）但 models.rs 无对应结构（散落在各专属模块），双轨制。

##### `src/schema.rs`

- **职责:** Diesel CLI 生成的 10 张表 schema(stock_daily/lhb_daily/analysis_result/stock_position/paper_trades/execution_tracking/position_adjustments/trades/ledger/account_mode_log)。
- **内部结构:** 单文件 201 行，10 个 `table!` + `allow_tables_to_appear_in_same_query!`(:193，仅含 7 表）。
- **生产调用链:** `src/models.rs:13` → database/repository → 全 DB 路径。
- **活性:** CURRENT — Diesel 编译期查询检查的根基。
- **问题/缺口:**
  - `paper_trades`/`execution_tracking`/`position_adjustments` 未加入 `allow_tables_to_appear_in_same_query!`(:193-201)，跨表 join 编译期不可用；
  - 与 `src/database/global_schema_v1.rs` 的手写 SQL schema 并存，两套 schema 事实源。

##### `src/traits.rs`

- **职责:** 两个共享 trait:`ScoreDisplay`(sentiment_score → emoji/评级标签）与 `AiContentGenerator`(AI 生成抽象）。
- **内部结构:** 单文件 95 行，2 个 trait(:31/:83)。
- **生产调用链:** `src/pipeline/mod.rs:50,206`(AnalysisResult impl)→ 分析报告推送；`src/market_analyzer/mod.rs:20` 把 `AiContentGenerator` re-export 为 `AiAnalyzer`。
- **活性:** CURRENT — pipeline 评分展示与大盘复盘 AI 抽象在用。
- **问题/缺口:** 未发现明显问题。

##### `src/types.rs`

- **职责:** 领域 newtype 试探 — `StockCode`(6 位 0/3/6 校验）、`Price`、`Percent`；注释自述 "渐进迁移策略：新代码使用 newtype"。
- **内部结构:** 单文件 69 行，3 个 newtype。
- **生产调用链:** 无。grep `StockCode::new|types::Price|types::Percent` 全仓零命中（除自身）。
- **活性:** INACTIVE — 2026-08-16 引入后迁移从未发生，全仓仍用裸 String/f64。
- **问题/缺口:** 死代码：没有任何生产/测试调用方，"渐进迁移"(src/types.rs:3）从未启动，建议删除或重新立项。

##### `src/enums.rs`

- **职责:** 单一枚举 `ReportType`(Simple/Full，报告格式选择）。
- **内部结构:** 单文件 60 行，1 枚举 + FromStr + display_name + 2 单测。
- **生产调用链:** 无。grep `enums::ReportType` 全仓零命中（除自身）；注释称 "用于 API 触发分析"，但无 API 模块消费。
- **活性:** INACTIVE — 零调用方的孤儿枚举。
- **问题/缺口:** 死代码：注释承诺的 API 消费者不存在（src/enums.rs:9)，建议删除。

##### `src/errors.rs`

- **职责:** 四个具名错误枚举（ProviderError/DbError/SearchError/TradeError),anyhow 渐进迁移的终点设想。
- **内部结构:** 单文件 82 行，4 枚举；`TradeError` 含 6 个交易红线变体（:64-82)。
- **生产调用链:** `TradeError` ← `src/trading/mod.rs:26,130`(DuplicateOrder 实构造）;`DbError` ← `src/database/repository.rs:8`（全文件返回类型）;`ProviderError`/`SearchError` 零调用方。
- **活性:** CONDITIONAL — 一半（DbError/TradeError）在交易/仓储路径实达，一半（ProviderError/SearchError）是死类型。
- **问题/缺口:**
  - `ProviderError`(:9)/`SearchError`(:51）全仓零构造零匹配，死代码；
  - `TradeError` 6 变体中仅 `DuplicateOrder` 被实际构造（trading/mod.rs:130)，其余 5 变体（资金不足/数量非法/价格超限等）无构造点，写入侧红线校验未落在该类型上。

##### `src/market_data.rs`

- **职责:** 大盘数据结构 — `MarketIndex`（指数）、`MarketOverview`（涨跌家数/板块/涨停榜）、`TopStock`、`SectorInfo`，纯数据 + 少量展示辅助。
- **内部结构:** `MarketIndex`(:11，含振幅计算）;`MarketOverview`(:108，含 market_mood);`TopStock`(:141);2 单测。
- **生产调用链:** `src/market_analyzer/*`(indices/async_overview/statistics/review)、`src/pipeline/chain_analysis/*`、`src/bin/monitor/{market_data,intraday_market,auction_input_alerts}.rs`。
- **活性:** CURRENT — 大盘复盘与 monitor 盘中/竞价链路共用。
- **问题/缺口:** 未发现明显问题（north_flow 的 None-缺失语义 :126-128 已按 BR-012 修复）。

##### `src/main.rs`

- **职责:** `stock_analysis` CLI 主入口（default-run)—— env 加载、operator 认证闸、日志、DB 初始化、按参数分派五种模式（schedule/chain_analysis/lhb/market_review/默认分析）,SIGINT 优雅关闭。
- **内部结构:** 单文件 120 行；本地 `mod app; mod cli;`(:15-16)；模式分派 :76-109。
- **生产调用链:** 自身即入口；调度方式 = **仅手动**(launchd 仅装 monitor/verify/weekly 三个 job;runtime `bin/` 目录无 stock_analysis 二进制）。
- **活性:** CURRENT — CLI 主流程完整可达（`cargo run` / 手动二进制），但非常驻生产路径。
- **问题/缺口:**
  - CLI 路径不调 `config::load_all()`（全仓仅 monitor main.rs:5987 调），五种模式下的 MonitorConfig 全是编译期默认值；
  - 认证闸（:30）默认禁用，与 auth 模块注释 "monitor 启动前调用" 的承诺错位（真正常驻的 monitor 反而无此闸）。

##### `src/cli.rs`

- **职责:** clap 参数结构 `Args`(debug/dry_run/stocks/schedule/interval/lhb_mode/chain_analysis/deep_analysis 等 16 项）。
- **内部结构:** 单文件 85 行，1 个 `#[derive(Parser)]` 结构。
- **生产调用链:** `src/main.rs:35` 解析；同时被库内 `src/app/` 复用（lib.rs:23 注释）;monitor bin 有自己独立参数解析，不用它。
- **活性:** CURRENT — CLI 每次启动必解析。
- **问题/缺口:** 未发现明显问题。

###### src/bin/ 全部 binary

##### `src/bin/monitor/` （生产主体）

- **职责:** launchd 常驻实盘监控进程 — 行情扫描、新闻/公告、产业链调度、盘中 intraday 策略、归因、收盘估值、durable delivery、健康检查、飞书推送的总装。
- **内部结构:** `main.rs`(15026 行，含 40+ 内嵌 mod/测试模块）;`closing_valuation_runtime.rs`(:18 `run_closing_valuation_once`);`health.rs`(152 行启动健康检查）;`durable_delivery_runtime/`(7 文件）;`push_templates/`(8 文件）;`news_ai_shadow.rs`/`intraday_market.rs`/`auction_input_alerts.rs` 等。
- **生产调用链:** `scripts/launchd/com.stockanalysis.monitor.plist`(KeepAlive=true，指向 release/monitor)→ `main.rs:5555 fn main` → `config::load_all`(:5987)→ `v16_4::register_all`(:6301)→ `health::health_check`(:6304)→ 各时段 loop（收盘估值 :6447/:7661、intraday tick :10250、chain 调度 :10447+)。
- **活性:** CURRENT — 生产唯一常驻进程。
- **调度方式:** **launchd 常驻**(KeepAlive,ThrottleInterval=60s)。
- **问题/缺口:**
  - main.rs 单文件 15026 行、40+ 内嵌测试模块，可维护性风险集中；
  - 启动健康检查失败仅告警不阻断（main.rs:6316-6321 error + webhook 后继续），降级运行的边界靠各 loop 自行 fail-closed。

##### `src/bin/weekly_outcome_review.rs` (+ weekly_outcome_review/)

- **用途:** H16 每周选股结果描述性复盘（读分离快照，绝不初始化/回填/发送/改配置）。
- **调度方式:** **launchd** `com.stockanalysis.weekly-outcome-review` 周五 20:30 → run-weekly-outcome-review.sh → weekly-outcome-review.py（先 ro 备份再跑 JSON+Markdown 双格式）。
- **活性:** CURRENT — runtime `bin/weekly_outcome_review` 已部署在役。
- **内部结构:** 主文件 233 行 + `paper_account.rs`/`registry.rs`/`report.rs`/`scorecard.rs`/`tests.rs`。
- **问题/缺口:** 未发现明显问题（只读快照 + 私有备份设计完整）。

##### `src/bin/assistant_review.rs`

- **用途:** 周报复盘的离线 AI 对比审阅（默认离线，显式 `--model` 才启用受限 assistant 角色）,483 行 + 库内 `src/assistant_review.rs`(66k 行 FrozenPack 体系）。
- **调度方式:** 设计上是 weekly launchd 的子步骤（repo 版 scripts/run-weekly-outcome-review.sh:9-10 传 `--assistant-binary/--assistant-script`,weekly-outcome-review.py:168-172 条件调用）;**但当前已部署 runtime 的 run-weekly-outcome-review.sh 未带该参数、runtime/bin 也无 assistant_review 二进制** → 实际仅手动。
- **活性:** CONDITIONAL — 仓库侧接线完成，部署侧未安装；且离线默认。
- **问题/缺口:** 部署漂移：repo 脚本与 runtime 已装脚本不一致（runtime 版缺 --assistant-binary),weekly 复盘当前不产 AI 对比件。

##### `src/bin/backfill_daily.rs`

- **用途:** stock_daily 停更时一次性全量回填日线（Magic TDX→腾讯→新浪→百度 fallover),`--outcomes` 模式补 outcome 窗口。
- **调度方式:** **launchd** `com.stockanalysis.prediction-verify` 每日 21:17 → daily_prediction_verify.sh:64（对 prediction_tracker 中尚有 NULL 窗口的证券批量调用）。
- **活性:** CURRENT — 每日定时任务第一棒。
- **问题/缺口:** 脚本只汇总退出码，"exit0 不证明窗口成熟/资格到位"(daily_prediction_verify.sh:21-22)，需人工看 backfill-daily.log。

##### `src/bin/backfill_predictions.rs` (+ backfill_predictions/tests.rs)

- **用途:** 按最近已完成上海交易日扫描全部到期 pending 预测并 verify 回填（旧 days 位置参数已弃用）。
- **调度方式:** **launchd** 同上 21:17 → daily_prediction_verify.sh:76。
- **活性:** CURRENT — 每日定时任务第二棒。
- **问题/缺口:** 失败行保持 pending 且仅 stderr 提示（脚本 :83)，无告警通道。

##### `src/bin/import_real_account_snapshot.rs / import_user_account_summary.rs / import_user_position_snapshot.rs`

- **用途:** 三个一次性人工导入器 — BR-103 真实账户快照（用户举证的 evidence manifest)、BR-146 用户确认账户汇总、BR-146+BR-215 用户确认持仓快照（导入后 reconciliation `stock_position` 投影）。
- **调度方式:** **仅手动**(operator 显式 `--database/--evidence|--summary|--snapshot`)。
- **活性:** CONDITIONAL — 代码在役但依赖人工举证事件触发；属账户事实源的唯一入口。
- **问题/缺口:** 无调度无提醒，账户快照过期靠 monitor 侧 stale 门（dq_position_stale_sec 等）间接暴露。

##### `src/bin/paper_account_activate.rs`

- **用途:** 已确认快照的 paper epoch 激活 — preview/apply 两段式，apply 需携带 prepared source hash(89 行薄壳，逻辑在 `trading::paper_snapshot_activation`)。
- **调度方式:** **仅手动**(operator 仪式）。
- **活性:** CONDITIONAL — 仅激活窗口期使用；paper 主引擎已被 BR-211 禁用，激活后实际驱动面收窄。
- **问题/缺口:** 与 BR-211 禁用状态的关系未在 bin 层注释说明，operator 需自行知道当前 paper 链路哪些还活着。

##### `src/bin/run_closing_valuation.rs`

- **用途:** BR-147 一次性收盘估值 runner（用户确认快照 + 已准入未复权收盘价）,230 行。
- **调度方式:** **仅手动** — 生产收盘估值由 monitor 内嵌 `closing_valuation_runtime::run_closing_valuation_once`(main.rs:6447/:7661）自动执行，该 bin 是独立补跑/审计入口。
- **活性:** CONDITIONAL — 手动补跑工具，与 monitor 内嵌路径同源不同入口。
- **问题/缺口:** 双入口（monitor 内嵌 vs 独立 bin）各自维护调用装配，长期有漂移风险（bin 用 `portfolio::closing_valuation`,monitor 用 closing_valuation_runtime 模块）。

##### `src/bin/confirm_daily_change.rs`

- **用途:** BR-171 离线审阅持久化的日度变更候选，operator 确认/拒绝只绑定原候选与 token，绝不重取 RPC 数据。
- **调度方式:** **仅手动**（交互式 stdin 确认）。
- **活性:** CONDITIONAL — 有候选才需要跑；属每日变更治理的人工闸门。
- **问题/缺口:** 未发现明显问题（证据绑定设计明确）。

##### `src/bin/resolve_stale_uncertain.rs`

- **用途:** 2026-09-21 一次性处置：把卡死的 `UncertainManualReview` HoldingPlan 决议置为 Rejected，释放 cooldown head 让当日周期可重新准入。
- **调度方式:** **仅手动（已执行过的一次性工具）**。
- **活性:** INACTIVE — 头部注释自述为 2026-09-21 单次事件（:1-6)，目的已达成，保留作审计。
- **问题/缺口:** 一次性工具滞留 bin 目录，无清理机制（同类还有 migrate_selection_v2)。

##### `src/bin/selection_activation_prepare.rs`

- **用途:** BR-183/193 selection-v2 激活材料准备仪式（seal-board 封存板块绑定 / print-activation 输出 expected_config_hash + activation JSON，全 src+config 可执行指纹）。
- **调度方式:** **仅手动**(operator 仪式，TDX 不可用即 fail-closed)。
- **活性:** CONDITIONAL — 仅 selection 激活/再激活时使用；激活消费方在 monitor。
- **问题/缺口:** executable_revision 覆盖全 src+config(:16-17)，任何无关代码改动都使 hash 失效，仪式成本高（设计使然，但无增量豁免）。

##### `src/bin/migrate_selection_v2.rs`

- **用途:** BR-180 固定根、fail-closed 的 selection-v2 schema 迁移命令（16 行薄壳，逻辑在 `database::run_selection_v2_migration_command`)。
- **调度方式:** **仅手动（一次性迁移）**。
- **活性:** CONDITIONAL — 迁移完成后即休眠，拒绝重复执行的 fail-closed 设计使其天然一次性。
- **问题/缺口:** 未发现明显问题。

##### `src/bin/chain_schedule_resolve.rs`

- **用途:** 人工检视/解決 chain 报告定时发送的 uncertain 状态（查外部渠道日志后置 delivered/retry),**自身绝不发送通知**。
- **调度方式:** **仅手动**。
- **活性:** CONDITIONAL — 仅当 monitor 的 chain 调度（main.rs:10447+）留下 uncertain 存根时使用。
- **问题/缺口:** 未发现明显问题。

##### `src/bin/day_capture_check.rs`

- **用途:** 有界离线观察 — 读事件 JSONL(≤64MB/10 万行上限）跑止损检查重放，无 publisher/DB/时钟/provider。
- **调度方式:** **仅手动**(goal-first 交付包成员，prepare_goal_first_delivery.py:17 BINS 列表，无 launchd 条目）。
- **活性:** CONDITIONAL — 离线取证工具，按需手动。
- **问题/缺口:** 未发现明显问题。

##### `src/bin/durable_schema14_snapshot_check.rs`

- **用途:** schema14 扩展的只读预检 — 只接受无 WAL/SHM 的隔离封存副本，绝不指向活库、不做迁移/批准。
- **调度方式:** **仅手动**（上线前仪式）。
- **活性:** CONDITIONAL — schema 演进窗口使用。
- **问题/缺口:** 未发现明显问题。

##### `src/bin/attribution_backfill.rs`

- **用途:** BR-255 归因手动补跑 — 生产归因只在 15:05-15:20 窗口自动执行，窗口错过（如上游崩溃）当天永久缺失，本工具事后补算；早于 epoch effective_date 的日期走只读重建。
- **调度方式:** **仅手动**。
- **活性:** CONDITIONAL — 故障补偿工具，2026-09-01 实战使用过。
- **问题/缺口:** 工具自己不激活 epoch(:19-21 注释明确），operator 需严格遵守 "激活归生产 15:35-15:50 tick" 的分工，误用代价是占用唯一激活额度。

##### `src/bin/strategy_attribution.rs`

- **用途:** BR-251 中文买卖策略历史归因 CLI(2849 行，全 bin 目录最大）—— 只读重放 verified 日历范围、HS300 基准对比、epoch 绑定报告。
- **调度方式:** **仅手动**。
- **活性:** CONDITIONAL — 研究/审计按需；依赖 `calendar::resolve_verified_*` 不可变权威。
- **问题/缺口:** 2849 行单文件 CLI，子命令/报告/格式化全堆一处，维护面大。

##### `src/bin/backfill_catalyst_watchlist.rs`

- **用途:** T+1 关注票名单快照一次性回填（取"最早落库"visible batch 忠实还原推送当晚名单，内容哈希幂等）。
- **调度方式:** **仅手动**(R-13 首推前使用）。
- **活性:** CONDITIONAL — 补数工具，71 行薄壳。
- **问题/缺口:** 未发现明显问题。

##### `src/bin/paper_adjudication_preview.rs`

- **用途:** paper adjudication 的只读 operator 预览（apply 裁决是另一个独立动作，Task10)。
- **调度方式:** **仅手动**。
- **活性:** CONDITIONAL — 依赖 paper 链路；BR-211 禁用后适用面收窄。
- **问题/缺口:** 与 BR-211 禁用状态的关系同 paper_account_activate，未在 bin 层标注。

##### `src/bin/sell_reminder_producer.rs / sell_reminder_preview.rs`

- **用途:** 独立盘后卖出提醒 — producer 在 15:02 产观察包（来源未准入即 NotReady，只读无发送）;preview 离线只读预览（显式快照/观察包输入）。
- **调度方式:** **仅手动** — 两者在 goal-first 交付清单（prepare_goal_first_delivery.py:17)，但无任何 launchd/cron 条目；docs/Project_Architecture_Blueprint.md:268 归入 "生产运维（手动）"。
- **活性:** CONDITIONAL — 属 offline_products 体系，等待调度接线或保持手动。
- **问题/缺口:** "15:02 producer" 的时间语义靠人工遵守，无调度器保证；runtime/bin 当前未部署这两个二进制。

##### 11 个 probe（一次性合并卡片）

- **用途与调度:**（全部 **仅手动** 诊断/探针，零 launchd/零脚本引用）
  - `account_metrics_probe.rs`(443 行）— 账户模式 metrics 装配只读探针（2026-09-21 评估 #12)。
  - `announcement_probe.rs`(75 行）— 验证 cninfo 公告批次排序（最新优先 vs 正序）。
  - `board_directory_probe.rs`(41 行）— TDX 概念板块目录打印，A-11 关键词校准。
  - `economic_position_probe.rs`(156 行）— BR-248 经济仓位只读历史探针（READ_ONLY 打开）。
  - `gateway_quote_probe.rs`(38 行）— 验证 TDX→腾讯→新浪 fallover 的 Quote 能力链。
  - `grpc_bundle_probe.rs`(1255 行）— 认证 bundle 的 opening-readiness + 选定原生查询探针（脱敏）。
  - `grpc_candidate_bundle_probe.rs`(561 行 + tests.rs)— 候选 bundle 显式诊断，不得选定生产 provider。
  - `grpc_failure_probe.rs`(371 行）— loopback 单发 LocalBridgeV1 flow + GetHealth 失败诊断。
  - `grpc_local_readiness_probe.rs`(423 行）— 本地 gRPC 通道就绪探针（4 必备 Operation 准入摘要）。
  - `historical_observed_probe.rs`(192 行）— 单次精确窗口历史观察入隔离证据目录。
  - `src/bin/monitor/data_mode_probe.rs`(monitor 内嵌模块，非独立 bin)— BR-148 capability 探针接缝，随 monitor 主进程编译（`main.rs:162 mod data_mode_probe`),`#[allow(dead_code)]` 标注显示生产接线是占位式。
- **活性:** 前 10 个 CONDITIONAL（排障/验收时手动跑，grpc_* 系列近期随 BR-196 交付活跃）;data_mode_probe 是 monitor 的 CONDITIONAL 内嵌件。
- **问题/缺口:** probe 无统一登记处，新 operator 只能从文件名猜用途；grpc_bundle_probe 1255 行已超 "探针" 体量，接近半个验收工具。

##### 研究/回测/开发 bin（合并卡片）

- **用途与调度:**（全部 **仅手动**，无自动化引用）
  - `boll_macd_backtest.rs`(394 行）— 布林+MACD 信号在 136 笔已平仓数据上的胜率回测。
  - `rsi_optimize.rs`(670 行）— RSI 策略 preset 迭代对比，结果追加 reports/rsi_optimization_log.md。
  - `winrate_simulator.rs`(427 行）— 基于 prediction_tracker 已 verify 数据模拟 "关停 X 主题后胜率"（带 operator 认证闸 :101)。
  - `streak_leader_research.rs`(170 行）— StreakLeader 离线观察/研究导出（goal-first 交付包成员）。
  - `produce_winrate_samples.rs`(229 行）— BR-017 从 prediction_tracker 导出已验证胜率样本。
  - `deep_analyze.rs`(39 行）— 单股多角色 AI 深研 CLI，走生产同款 deep_analyzer::run_and_save。
  - `agent_test.rs`(211 行）— Agent 工具链（财务/筹码/资金流/新闻/研报/板块）手动联调。
  - `v14_e2e.rs`(397 行）— v14.2 七层推送架构（L1→L4→L5→L6→L7）端到端实测。
  - `lhb_query.rs`(154 行，不在任务清单但同目录）— BR-162 龙虎榜证据保留查询。
- **活性:** 全部 CONDITIONAL — 研究期活跃、无生产消费者；其中 agent_test/v14_e2e 接近一次性验收工具。
- **问题/缺口:** 回测三件套（boll_macd/rsi/winrate）与样本导出（produce_winrate_samples）口径各自为政，无共享胜率计算库；研究结论靠 reports/*.md 人工沉淀，无回收机制。

> **本组总评:** 顶层支撑文件整体健康且在监控主循环实达（calendar/config/models/schema/market_data/util/http_client/production_root 均 CURRENT)，但存在三块明确的死重：`types.rs` 与 `enums.rs` 零调用方，`errors.rs` 半数变体/枚举从未构造，`bus` 三总线空转、`registry` 注册后无人路由（策略分发仍硬编码）。bin 层的调度事实清晰：launchd 仅三条自动链（monitor 常驻、每日 21:17 的 backfill_daily+backfill_predictions、周五 20:30 的 weekly_outcome_review)，其余 30+ 工具全部手动；值得注意的部署漂移是 repo 版 weekly 脚本已接线 assistant_review 而 runtime 已装版本尚未跟上。

> 备注:agent-14 组内 `# 顶层支撑散文件` 与 `# src/bin/ 全部 binary` 两个分组小标题以六级标题保留;该组 src/bin 各卡片按其任务口径使用「用途/调度方式」等字段(而非统一的五字段卡片),已原样保留;`src/opportunity/` 活性判定词为「混合型」,11 个 probe 与研究 bin 两张合并卡片的活性表述亦非单一判定词,HTML 侧仅对「活性」字段内首个 CURRENT/CONDITIONAL/INACTIVE 判定词着 status-chip,正文中的英文不转换;各 subagent 块的序言客套行、agent-7/agent-13 的组标题行与块内 `---` 分隔线不含事实内容,未纳入。11 组卡片齐全,无缺卡、无格式破损。

**一句话:**数据流入(接入/准入)与推送流出(投递域)是机构级且活得好;中间的价值生产段(新闻理解、技术信号、选股排序、真实交易)要么常数分占位、要么死代码、要么断流;复盘环骨架齐全但多数 Disabled 或停更。能量流向是「采集 → 审计 → 推送」,而不是「分析 → 决策 → 收益归因」。

## 3. C4 Level 1：系统上下文

```mermaid
flowchart LR
    operator["EXTERNAL 操作者\nCLI / monitor 运维"]
    providers["EXTERNAL 金融/新闻数据源"]
    host["EXTERNAL provider-host\n独立构建与部署"]
    llm["EXTERNAL LLM providers\nOpenAI-compatible APIs"]
    sinks["EXTERNAL 通知目标\nFeishu / WeChat / Email / HTTP"]

    system["Stock Analysis System\nRust modular monolith\nconsumer + control plane"]
    businessDb[("Business SQLite")]
    durable[("Durable SQLite + immutable audits")]
    reports[("Reports / JSONL projections")]

    operator -->|commands, config, activation| system
    providers -->|provider-specific acquisition| host
    host -->|versioned typed gRPC\noptional mTLS + Bearer| system
    system -->|model requests| llm
    system -->|governed delivery| sinks
    system --> businessDb
    system --> durable
    system --> reports
```

证据：`README.md:1-44` 明确本仓不含 provider 实现、server target 或本地 fallback；`src/lib.rs:225-232` 只公开 gRPC client/contract 与 `market_domain`；LLM port 在 `src/llm/mod.rs`；通知实现见 `src/notification/`、`src/push_l6/` 与 monitor-local `notify.rs`。

## 4. C4 Level 2：运行容器与进程拓扑

```mermaid
flowchart TB
    host["EXTERNAL provider-host\nhealth + capabilities + data/event RPCs"]

    subgraph MonitorProcess["CURRENT monitor"]
        grpcBridge["data_gateway::grpc_source"]
        producers["market / news / review / selection producers"]
        decisions["analysis / decision / risk / paper"]
        governor["durable delivery coordinator"]
        observation["event bus + JSONL observation"]
        grpcBridge --> producers --> decisions --> governor
        governor --> observation
    end

    subgraph CliProcess["CURRENT stock_analysis default CLI"]
        bootstrap["auth + startup validation + stock-list assembly"]
        pipeline["AnalysisPipeline"]
        cliOutput["DB / reports / NotificationService"]
        bootstrap --> pipeline --> cliOutput
    end

    subgraph ToolProcesses["CURRENT bounded tools / probes"]
        probes["grpc_bundle_probe\ngrpc_local_readiness_probe"]
        maintenance["import / backfill / migration"]
        research["analysis / backtest / attribution"]
    end

    bundle["EXTERNAL client bundle\noptional mTLS + Bearer"]
    businessDb[("Business SQLite")]
    durableDb[("Isolated durable SQLite")]
    immutable[("Hash-chained audit files")]
    extSinks["EXTERNAL sinks"]

    host -->|typed gRPC| MonitorProcess
    host -->|typed gRPC| CliProcess
    host -->|readiness/data calls| ToolProcesses
    bundle --> MonitorProcess
    bundle --> CliProcess
    bundle --> ToolProcesses
    MonitorProcess --> businessDb
    CliProcess --> businessDb
    ToolProcesses --> businessDb
    MonitorProcess --> durableDb
    MonitorProcess --> immutable
    MonitorProcess --> extSinks
    CliProcess --> extSinks
```

关键边界：

- 外部provider-host/VM与本机bridge是不同部署单元：ExternalV1上游 `10.211.55.3:50051`，独立构建的本机 LocalBridgeV1 `127.0.0.1:18082` 由 `com.northofqing.grpc-market-server` 管理；bridge不在本Cargo targets，运行根与主库共用。不从“仓外provider”推断没有本机bridge。
- `tests/support/grpc_fixture` 只在 integration-test crate 中编译，不是 production container。证据：`README.md:85-90`、`tests/support/mod.rs`。
- default CLI 不是 monitor 控制客户端；它直接复用 library pipeline。证据：`src/main.rs` 的 mode dispatch 与 `src/app/bootstrap.rs`。
- `data_provider` 是委托统一 Gateway 的进程级 facade/cache，不是 provider implementation。证据：`README.md:44`、`src/data_provider/service.rs`。

## 5. C4 Level 2：生产部署拓扑

```mermaid
flowchart LR
    subgraph ExternalData["EXTERNAL data-service deployment"]
        providerApis["provider APIs"]
        host["provider-host\nversioned gRPC endpoint"]
        providerApis --> host
    end

    subgraph Host["stock_analysis deployment host · macOS/Linux"]
        bundle["optional client bundle\nCA / cert / key / token"]
        probe["grpc_bundle_probe --opening"]
        monitor["1 × monitor\nexclusive delivery lease"]
        db[("data/stock_analysis.db")]
        ddb[("data/durable_delivery.sqlite3")]
        audit[("data/event_audit\ndata/durable_delivery_audit")]
        jsonl[("daily event JSONL")]

        bundle --> probe
        bundle --> monitor
        monitor --> db
        monitor --> ddb
        monitor --> audit
        monitor --> jsonl
    end

    host --> probe
    host --> monitor
    monitor --> sinks["EXTERNAL Feishu / WeChat / HTTP"]
```

2026-10-09 增量部署事实（旧图的external端点由下图展开）：

```mermaid
flowchart LR
    vm["EXTERNAL VM ExternalV1 10.211.55.3:50051"] --> bridge["launchd local bridge 127.0.0.1:18082"]
    bridge --> monitor["launchd唯一monitor counted owner"]
    daily["已安装daily21:17 backfills"] --> db[("runtime主库")]
    monitor --> db
    weekly["已安装新版weekly Friday20:30"] --> artifacts["private周报artifact"]
    artifacts --> assistant["已安装 bounded artifact-only Phase A"]
    watchdog["已安装独立60s watchdog"] -->|只读health| monitor
    watchdog --> events["local events/self-check；无手机receipt"]
```

Monitor/bridge由Desktop外runtime的正式launchd job管理。daily写回填仍属既有独立任务，不能删掉其bin aliases；新版本weekly复用唯一label、RunAtLoad=false，不增加重复日程。watchdog只调用已安装monitor早期只读 `--health --json`，不重启/启动monitor。本次已核对新job实际加载及自然watchdog运行；首次真实周报仍等20:30日程。

## 6. C4 Level 3：统一数据平面

```mermaid
flowchart LR
    consumer["Business consumer\npipeline / monitor / selection / review"]
    gateway["Typed domain Gateway API"]
    bridge["GrpcSource process-wide lazy client"]
    contract["grpc_contract\noperation + params + schema + validate"]
    client["grpc_client\nchannel + auth + retry + envelope"]
    host["EXTERNAL provider-host\ncapabilities + data RPCs"]
    wire["wire payload / typed failure"]
    convert["wire-to-market_domain conversion"]
    evidence["GatewayBatch<T>\nAvailable | VerifiedEmpty\nBatchEvidence"]
    audit["data acquisition audit"]

    consumer --> gateway --> bridge --> contract --> client --> host
    host --> wire --> client --> convert --> evidence --> audit --> consumer
```

实现约束：

- `bridge_for` 总是注册 process-wide remote `GrpcSource`；地址来自 `GRPC_MARKET_ADDR`，可选安全连接来自 `GRPC_MARKET_CLIENT_BUNDLE`，首次方法调用才连接。证据：`src/data_gateway/grpc_source.rs:1-9,1063-1087`。
- 服务不可达、合同缺失或证据不合格时返回 typed error，绝不切换到本地采集库。证据：`src/data_gateway/grpc_source.rs:5-6` 与 `bridge_unreachable_is_fail_closed`。
- `HOOKED_OPS` 与真实 bridge 调用点由源码扫描测试对账；catalog 是 consumer 使用集合，不是本仓 server 实现清单。
- 业务消费者应接收带 evidence 的 typed batch；`Available` 空记录与 `VerifiedEmpty` 语义不同，多个模块对非法空 batch fail-closed。示例：`src/app/bootstrap.rs:185-226`。

### 6.1 Data Gateway 能力族

| 能力族 | 子模块 | 主要消费者 |
| --- | --- | --- |
| 行情与历史 | `market_data`、`historical_bars`、`index`、`benchmark`、`intraday_shape`、`t0_evidence` | pipeline、monitor、strategy、performance |
| 板块与排名 | `board`、`board_ranking`、`board_runtime`、`market_capabilities` | monitor、opportunity、selection |
| 资金与交易事实 | `capital`、`block_trade`、`dragon_tiger`、`consensus` | decision、CLI、review |
| 新闻与公告 | `global_news`、`sina_instrument_news`、`event_calendar`、`economic_calendar` | news monitor、agent、review |
| 公司与研究 | `company`、`research`、`general_web_research` | analyzer、agent、deep analysis |
| 身份与生命周期 | `instrument_identity`、`security_lifecycle`、`exchange_calendar_authority` | admission、过滤、session gate |
| 产业链与仓位 | `chain_intelligence`、`position_chain`、`review` | opportunity、portfolio、post-session review |
| selection outcome | `outcome_daily_bars` | selection-v2 settlement owner |

完整声明依据：`src/data_gateway/mod.rs`。

## 7. C4 Level 3：gRPC 合同与调用链

```mermaid
sequenceDiagram
    participant M as monitor consumer
    participant G as data_gateway::grpc_source
    participant C as GrpcMarketClient
    participant H as EXTERNAL provider-host

    M->>G: typed gateway request
    G->>C: operation + JSON params + request_id
    C->>H: tonic RPC + optional mTLS/Bearer
    H-->>C: QueryResponse / typed ErrorDetail / tonic Status
    C-->>G: decoded payload / GrpcError
    G-->>M: domain records + preserved evidence
```

合同生成：

1. `build.rs:26-29` 读取 `client-bundle/market.proto`。
2. `build.rs:18-36` 在 OUT_DIR 合并 compatibility extensions，再用 `tonic_prost_build` 生成 client 与 test fixture 所需的 server trait；这不创建 production server target。
3. 合同枚举覆盖 0..=62；61=`ChainBatch`、62=`BenchmarkBars`，`QueryResponse.source=11` 保留来源证据。
4. `src/grpc_contract/ops.rs` 提供 0..=62 全量方法映射与 40 个 production-used consumer operations。
5. 外部 host 的 health/capabilities 必须满足 opening probe 和实际 operation；本仓集成测试使用 `tests/support/grpc_fixture` 验证 roundtrip。

错误与重试：

- typed `ErrorDetail` 在 client decode 后保留 request id、operation、provider、reason code、retryable；`no_current_reports` 等业务状态不会折叠成 `internal`。证据：`src/grpc_client/errors.rs`。
- endpoint timeout 35 秒。Unavailable 走指数退避并复查 health；DeadlineExceeded 有界重试且保留 request id；backoff 上限 60 秒。证据：`src/grpc_client/client.rs:54-96`、`:221-225`、`src/grpc_client/retry.rs:1-58`。
- InvalidArgument、Unauthenticated、PermissionDenied、Unimplemented、FailedPrecondition 不因远端 metadata 被错误升级为可重试。证据：`src/grpc_client/retry.rs:13-43`。

## 8. C4 Level 3：monitor 控制面

### 8.1 启动闸门

```mermaid
flowchart TD
    start["monitor main"] --> cli["library-owned CLI parse"]
    cli --> terminal{"help/version/history/replay/disabled?"}
    terminal -->|yes| short["storage-free or bounded terminal path"]
    terminal -->|no| dry{"production dry-run env?"}
    dry -->|yes| reject["reject before lease/artifacts"]
    dry -->|no| lease["acquire singleton monitor lease"]
    lease --> activation["selection-v2 activation decision"]
    activation --> namespace["production/test durable namespace isolation"]
    namespace --> mode["validate delivery mode"]
    mode --> audit["delivery audit read/write preflight"]
    audit --> bind["eager bind durable artifacts"]
    bind --> jsonl["spawn observation JSONL writer"]
    jsonl --> db["DatabaseManager + config::load_all"]
    db --> reconcile["durable startup reconciliation"]
    reconcile -->|fixed point| producers["activate long-running producers"]
    reconcile -->|failure| blocked["producer activation blocked"]
```

证据窗口：`src/bin/monitor/main.rs:4471-5077`。关键 fail-closed 点为生产 `V10_DRY_RUN_PUSH=1` 拒绝（`:4550`）、lease（`:4564`）、delivery mode（`:4655`）、audit preflight（`:4660`）、artifact bind（`:4681`）、startup reconciliation（`:4954-4982`）。

### 8.2 常驻任务树

```mermaid
flowchart TB
    supervisor["supervise_long_running_lifecycle"]
    supervisor --> mainLoops["Main loops future"]
    supervisor --> background["8 background JoinHandles"]
    supervisor --> signal["SIGINT"]
    supervisor --> writer["JSONL writer health"]

    mainLoops --> p01["P-01 scheduler"]
    mainLoops --> mon["monitor_loop"]
    mainLoops --> news["news_monitor_loop"]
    mainLoops --> dataMode["data_mode_monitor_loop"]
    mon --> intraday["intraday_loop"]
    mon --> market["market_loop"]

    background --> dryreport["dryrun reporter"]
    background --> eventConsumer["MonitorEvent consumer"]
    background --> postNews["post-close news"]
    background --> review["post-session review"]
    background --> reviewBackfill["startup review backfill\nafter durable producer-ready"]
    background --> chain["position-chain refresh"]
    background --> staticReady["opening static readiness"]
    background --> liveReady["opening live readiness"]
```

证据：四个 main loops 位于 `src/bin/monitor/main.rs:5479-5488`；八个 background tasks 位于 `:5496-5539`；`review_backfill` 在 durable `runtime_producer_ready` 后执行；`monitor_loop` 在 `:8796`，最终 `tokio::join!(intraday_loop, market_loop)` 在 `:11296`。

### 8.3 monitor 代码的两层组成

| 层 | 内容 | 证据 |
| --- | --- | --- |
| reusable library `src/monitor/` | adaptive、alert/log、attribution/deep、auction、checklist、data_mode/quality、detector、entity_linker、event_bus、news_ai/monitor、prediction、rate_budget、risk、scanner、signal_fusion/state | `src/monitor/mod.rs` |
| binary-local composition `src/bin/monitor/` | notify、transport、presentation registry、P-01、push templates、review batch/backfill、dryrun report、v13 diagnostics、attribution epoch runtime、blocking/async market data、closing valuation、data-mode probe、intraday market、durable runtime、L6 sink、news aggregator/AI shadow、health/webhook/freshness/v17 sources | `src/bin/monitor/main.rs` 顶部 module declarations 与同目录文件 |

这种拆分说明 monitor 既有可复用领域能力，也有大量只属于进程启动/运维的 composition root；新功能应先判断是否需要被其他 binaries 复用，再决定放 library 还是 bin-local。

## 9. C4 Level 3：默认 CLI 分析管道

```mermaid
flowchart TD
    start["stock_analysis main"] --> auth["operator auth"]
    auth --> validate["AI key + notification config validation"]
    validate --> db["best-effort business DB init"]
    db --> mode{"mode dispatch"}

    mode --> schedule["schedule"]
    mode --> chain["chain analysis"]
    mode --> review["market review"]
    mode --> deep["deep analysis"]
    mode --> lhb["LHB analysis"]
    mode --> normal["single analysis"]

    normal --> pool["stock-list assembly"]
    pool --> manual["manual/env stocks"]
    pool --> macro["macro AI recommendations"]
    pool --> lhbTop["LHB top 10"]
    pool --> limit["limit-up pool"]
    pool --> holdings["open positions"]
    pool --> lifecycle["SecurityIdentity delisted filter"]
    lifecycle --> pipeline["AnalysisPipeline"]
    pipeline --> bars["HistoricalBarsGateway + save"]
    pipeline --> analyze["bounded concurrent per-stock analysis"]
    analyze --> enrich["optional deep/key-stock enrichment"]
    enrich --> persist["analysis/position persistence"]
    persist --> report["reports + summary + notifications"]
```

代码依据：

- mode dispatch：`src/main.rs:73-106`。
- 股票池装配：`src/app/bootstrap.rs:64-282`；deep mode 禁用 macro/LHB/limit-up 自动扩展。
- `AnalysisPipeline` 与宽 `AnalysisResult`：`src/pipeline/mod.rs:61-250`、`:350-735`。
- historical bars 与保存：`src/pipeline/data.rs:25-101`。
- reports/notification：`src/pipeline/summary_notify.rs:42-112`。
- schedule 仅每轮 override `.env`：`src/app/schedule.rs:175-237`；它不会自动重载 TOML snapshot。

CLI 与 monitor 的故障策略不同：CLI 的 business DB 初始化失败会记录“数据不会入库”后继续；monitor 的 delivery audit、artifact binding 与 durable reconciliation 属于 producer 启动前硬闸门。证据：`src/main.rs:64-72` 对比 monitor 启动窗口。

## 10. 业务能力地图

E1–E4新增入口只有离线/描述性权限：`weekly_outcome_review` detached SQLite/query-only→完整JSON/manifest；`assistant_review::FrozenPack`只消费同一period/schema/原registry bytes→三臂comparison。registry action/status是人工描述字段，无自动strategy promotion；family/version join缺失保持null/unavailable，open cycles右删失与monetary dispute不被LLM覆盖。`sell_reminder_preview`公开observed/imported输入无admission，真实lot/fee/close缺失，15:30 expiry；ATR CNY与既有percent规则歧义仍待独立修复。`streak_leader_research`严格区分observed gross价格与private synthetic qualified engine，public可执行样本0，不承诺收益。

```mermaid
flowchart LR
    data["Data & evidence"] --> analysis["Analysis"]
    analysis --> selection["Selection / opportunity"]
    selection --> decision["Decision"]
    decision --> risk["Risk gates"]
    risk --> paper["Paper execution only"]
    paper --> monitor["Monitoring"]
    monitor --> delivery["Governed delivery"]
    paper --> review["Review / attribution"]
    delivery --> review
    review --> analysis

    analysis --- pipeline["pipeline · analyzer · indicators · strategies"]
    selection --- selectmods["selection · opportunity · news"]
    decision --- decmods["decision · portfolio"]
    risk --- riskmods["risk · monitor::risk · trading::risk_adapter"]
    review --- revmods["review · performance · market_analyzer"]
```

| 能力 | 当前职责 | 主要模块 |
| --- | --- | --- |
| 技术/基本面分析 | 指标、趋势、技术报告、财务/新闻分析、score/veto | `pipeline`、`analyzer`、`indicators`、`trend_analyzer`、`company_*` |
| 策略与回测 | Boll/MACD、RSI、multi-factor/timeframe、contrarian、lot、v16.4 | `strategy`、`backtest` re-export、多个 research binaries |
| 机会发现 | 事件提取、产业链映射、候选台、竞价、real alpha、winrate、launch gate | `opportunity` |
| selection-v2 | acquisition、admission、feature、sample、outcome、receipt、recovery | `selection`、`database::selection_v2*` |
| 决策支持 | exclusion、rotation、leader/sector score、capital verify、live/holding plan、T0 | `decision` |
| 风险 | cash/env guard、limits、stop loss、sector exit、veto chain、account/action gate | `risk`、`monitor::risk`、`trading::risk_adapter` |
| 模拟交易 | order safety、paper buy/sell、ledger/audit | `trading`；不含实盘下单路由 |
| 复盘归因 | 日/周复盘、watchlist、failure attribution、factor IC、epoch/replay | `review`、`performance`、`market_analyzer` |
| 通知投递 | legacy NotificationService、push L1-L7、monitor production durable route | `notification`、`push_l*`、monitor-local runtime |

近期已落地的能力扩展：

- BR-249 将 `chain_daily` 主线簇与持仓行业链归属装入 NewsAI v2 evidence context，并新增 `ChainRisk` monitor 告警；它们复用现有 NewsAI/monitor/durable 链，不构成新服务。证据：`src/monitor/news_ai.rs:589-600`、`src/bin/monitor/news_ai_shadow.rs:399-537`、`src/monitor/detector.rs:64,435`。
- BR-250 通过统一证券身份 Gateway 为 NewsAI 卡片补 display-only 名称；名称不进入 identity、prompt、证据 hash 或 DB，恢复推送缺名时只显示代码。证据：`src/monitor/news_ai.rs:120,282-288,1236`、`src/bin/monitor/news_ai_shadow.rs:436-469`。
- BR-255 的 15:05--15:20 归因闭环改用 `HistoricalBarsGateway` 当日收盘价，并新增 `attribution_backfill` 运维入口；epoch 首月窗口截断到 effective date。证据：`src/bin/monitor/market_data.rs:88`、`src/bin/monitor/main.rs:9084`、`src/performance/attribution.rs:542`、`src/bin/attribution_backfill.rs`。

### 10.1 模拟交易边界

- `CURRENT`：`decision::intraday_monitor` 获取 `broker::execution_quote`，保留真实涨跌停标志和 `quote_observed_at`，调用 `paper_trade::simulate`。证据：`src/decision/intraday_monitor.rs:175-235`、`:470-521`。
- `CURRENT`：monitor 盘中/盘后调用 `paper_sell::scan_and_sell*`；FIFO lot ledger 已重建，默认允许模拟卖出，只有 `PAPER_SELL_DISABLED=1` 才显式暂停。证据：`src/bin/monitor/main.rs:8246-8274` 及 `paper_sell_paused` 调用点。
- `INACTIVE`：legacy `paper_engine::run_once` 已从 production loop 隔离，函数自身返回 disabled error。证据：`src/bin/monitor/main.rs:8329-8353`、`src/trading/paper_engine.rs:409`。
- `EXTERNAL/未接通`：真实 broker trade-sync watermark 尚未连接，confirmed account snapshots 只作为 display facts。证据：`src/bin/monitor/main.rs:1800-1801`、`:2414-2418`。

因此本系统当前是分析、监控、受控通知与 paper simulation 系统，不应被架构图标成自动实盘交易系统。

## 11. selection-v2 阶段架构

```mermaid
flowchart LR
    activation["Config activation\nmanifest + activation receipt"]
    acquisition["Source acquisition\nbatch attempts + facts"]
    admission["Admission / quality\ncalendar + identity + evidence"]
    relation["Relation attempts"]
    evaluation["Feature + evaluation attempts"]
    sample["Samples / rejections"]
    claim["Outcome claim\ndue revalidation + lock"]
    provider["OutcomeDailyBars Gateway"]
    outcome["Sample outcomes / attempts"]
    receipt["Commit receipts + run stages"]
    recovery["Recovery envelopes"]

    activation --> acquisition --> admission
    admission --> relation --> evaluation --> sample
    sample --> claim --> provider --> outcome --> receipt
    recovery --> acquisition
    recovery --> relation
    recovery --> evaluation
    recovery --> claim
    receipt --> recovery
```

### 11.1 所有权与顺序

selection-v2 通过 opaque/typed capability 阻止 caller 自造中间状态：

1. activation runtime 读取真实 CLI/env/config evidence，决定 generation 与 outcome capability 是否释放。证据：`src/selection/activation_runtime.rs`、`activation_gate.rs`。
2. acquisition/admission 将 provider batch 与 trading calendar、instrument identity、source evidence 绑定。证据：`src/selection/acquisition_v2.rs`、`admission.rs`、`ingress_v2.rs`。
3. feature/evaluation/relation 构造 stage input，不允许绕过 stage ownership。证据：`features.rs`、`relation.rs`、`model.rs`。
4. `SelectionV2PersistenceOwner` 提交 stage、audit、receipt，并 read-back 验证。证据：`src/selection/persistence_v2.rs`、`src/database/selection_v2_repository.rs`。
5. `OutcomeSettlementOwner` 是生产 outcome 编排唯一 owner：先 drain recovery、fresh revalidate due，再 commit claim，之后调用 provider，最后 commit outcome receipt。证据：`src/selection/outcome_v2.rs:260-294`、`:854-935`、`:1085-1193`。
6. outcome claim 使用 descriptor-relative/no-follow 文件锁确保进程/跨进程同一 logical subject 只有一个 owner。证据：`src/selection/outcome_v2.rs:348-750`。

### 11.2 激活状态

`CONDITIONAL`：selection-v2 代码已完整存在，但 monitor 每次启动计算 `selection_v2_enabled`；disabled 时明确输出 providers=0、database_operations=0、sinks=0、schedulers=0。证据：`src/bin/monitor/main.rs:4523-4612`。generation-only release 与 outcome capability 还能独立关闭，不能用“模块存在”推断“默认生产启用”。

BR-178 又增加一层运行保护：当 production `DatabaseManager` 没有 amended selection schema authority 时，outcome recovery/due tick 返回空 summary 并只告警一次，不再每分钟制造 failed-closed 噪声；authority 接线后才自动放行。证据：`src/database/selection_v2_read_model.rs:231-239`、`src/selection/outcome_v2.rs:1106-1118`。

### 11.3 selection-v2 final catalog

| 对象 | 数量 | 约束 |
| --- | ---: | --- |
| final tables | 12 | 见附录 E |
| explicit indexes | 5 | activation、pending source、generation、outcome attempt、receipt subject |
| static triggers | 17 | lineage、manifest closure、receipt closure |
| stage membership targets | 9 | 对 9 张 stage table 生成 membership constraints |
| symbol constraint targets | 3 | relation、evaluation、sample |
| final payload schemas | 5 | config-v1、ingress-v2、generation-v3、outcome-claim-v2、outcome-v3 |

证据：`src/database/global_schema_catalog_v1.rs:27-146`。

## 12. 持久化投递与权威审计

本次不新增durable owner/schema；CLI不直接取得生产durable SQLite写权限。现有counted `push_counted_with_binding` 保持唯一owner。Unknown/Uncertain只对账/人工处置，不被重发或restore DB“回退”；event bus/JSONL为观测，不是durable queue或receipt。§24统一facade仍冻结。

### 12.1 投递序列

```mermaid
sequenceDiagram
    participant P as producer
    participant C as DurableDeliveryCoordinator
    participant DB as durable SQLite
    participant S as AuthoritativeSinkPort
    participant A as ImmutableAppendPort
    participant B as observation EventBus/JSONL

    P->>C: counted delivery envelope
    C->>DB: reserve decision / cooldown / daily budget
    C->>DB: create AttemptInFlight + fence token
    C->>S: deliver exact rendered bytes
    S-->>C: Accepted | Rejected | Uncertain
    C->>DB: persist typed sink result and pending state
    C->>A: append exact canonical audit bytes + sha256
    A-->>C: immutable audit ref after fsync
    C->>DB: commit terminal/task transition state
    C->>B: publish observation only
```

顺序语义：

- authoritative sink request 绑定 decision id、attempt id、fence token、push kind、stable template id、exact rendered bytes 与 SHA-256。证据：`src/durable_delivery/model.rs:1164-1179`。
- immutable append port 接收 exact canonical bytes 与 expected SHA-256。证据：`src/durable_delivery/model.rs:1183-1192`。
- monitor 明确声明 event bus 不能 acknowledge delivery。证据：`src/bin/monitor/main.rs:4744-4748`。
- event delivery audit 与 durable immutable audit 都使用固定/隔离 authority、retained descriptor、hash chain、sync/fsync；不接受任意 caller path 升级为生产 authority。证据：`src/event/dispatcher.rs:20-44`、`:190-250`，`src/event/durable_delivery_append.rs:1-52`。

### 12.2 DecisionState 状态机

```mermaid
stateDiagram-v2
    [*] --> Reserved
    Reserved --> AttemptInFlight
    Reserved --> RejectedAuditPending
    AttemptInFlight --> AcceptedAuditPending
    AttemptInFlight --> RejectedAuditPending
    AttemptInFlight --> UncertainAuditPending
    AcceptedAuditPending --> AcceptedTaskTransitionPending
    AcceptedAuditPending --> Delivered
    AcceptedTaskTransitionPending --> Delivered
    RejectedAuditPending --> RejectedTaskTransitionPending
    RejectedAuditPending --> RejectedDurable
    RejectedTaskTransitionPending --> RejectedDurable
    UncertainAuditPending --> UncertainTaskTransitionPending
    UncertainAuditPending --> UncertainManualReview
    UncertainTaskTransitionPending --> UncertainManualReview
    UncertainManualReview --> AcceptedAuditPending
    UncertainManualReview --> ManualRejectedAuditPending
    ManualRejectedAuditPending --> ManualRejectedTaskTransitionPending
    ManualRejectedAuditPending --> ManualResolvedRejected
    ManualRejectedTaskTransitionPending --> ManualResolvedRejected
    RejectedDurable --> Reserved
```

14 个状态定义在 `src/durable_delivery/model.rs:800-814`；上图 20 条合法 transition 逐项来自 `src/durable_delivery/coordinator.rs:7123-7150`。任何新增状态必须同时修改 enum、string mapping、schema constraints、transition table、reconciliation 与 tests。

### 12.3 恢复原则

- startup reconciliation 在 producer 启动前到达 durable local fixed point；未过闸不启动生产者。证据：`src/bin/monitor/main.rs:4954-4982`。
- non-expired foreign attempts 不自动 resend；uncertain decisions 留给 manual review。证据：同一启动窗口的日志分支。
- `RejectedDurable -> Reserved` 是唯一显式 retry loop；BR-249 要求先由 `authorize_rejected_retry` 写入授权，再重新取得 lease/fence/reservation，不能盲目重发。证据：`src/durable_delivery/coordinator.rs:3606`、`src/bin/monitor/durable_delivery_runtime.rs:1748`。
- review terminal replay 有 attempt/completion 两张专用表；恢复行为不是简单重放外部 sink。
- startup review backfill 只在 `runtime_producer_ready()` 后扫描历史业务日；缺估值时用 exact business-date closing valuation 回填，避免把上一交易日价格冒充目标日。证据：`src/bin/monitor/main.rs:5498-5526,5953-5966`、`src/database/closing_valuation.rs:145-170`。

## 13. 事件、推送与通知架构

```mermaid
flowchart LR
    producer["Producer"] --> l1["L1 SignalEvent"]
    l1 --> l2["L2 TemplateMetadata"]
    l2 --> render["L3 missing in library\nmonitor::push_templates renders"]
    render --> l4["L4 Dispatcher\ndedup/reserve/commit"]
    l4 --> l5["L5 Governance\nquiet/frozen/DQ/daily limit"]
    l5 --> durable["Production durable coordinator"]
    durable --> l6["L6 Sink / monitor authoritative sink"]
    l6 --> l7["L7 analytics"]
    durable --> audit["synchronous audit"]
    audit --> bus["EventEnvelope bus"]
    bus --> jsonl["non-authoritative JSONL"]
```

当前并存的三套抽象：

| 抽象 | 用途 | 边界 |
| --- | --- | --- |
| `notification::*` | default CLI 报告通知；email/Feishu/WeChat | 较早的 report-oriented service |
| `push_l1/l2/l4/l5/l6/l7` | library push layering、governance、sink、analytics | L3 缺失；L4 是内存 dedup compatibility 层 |
| monitor `notify` + `durable_delivery_runtime` | production counted delivery、receipt、audit、recovery | 当前生产权威路径 |

`push_l6` 提供 ConsoleSink、SinkRouter、HttpSink、WechatSink、FeishuSink；`push_l7` 提供 AnalyticsStore、InMemoryStore、SqliteStore。证据：`src/push_l6/mod.rs`、`src/push_l7/mod.rs`。新增生产 sink 不能只注册到 L6；若它承载 counted delivery，还必须实现 authoritative receipt 语义并接入 durable runtime。

### 13.1 两套 bus 不可混淆

| Bus | Payload | 特性 | 证据 |
| --- | --- | --- | --- |
| `event::bus::EventBus` | `EventEnvelope` | publish outcome、metrics、shutdown；用于 observation/replay | `src/event/bus.rs:43-166` |
| `monitor::event_bus::EventBus` | `MonitorEvent` | 全局 Tokio broadcast；alert/opportunity/order/price/DQ/info | `src/monitor/event_bus.rs:74-107` |

monitor bus lag 会记录丢失条数并继续；它不是 durable queue。通用 replay 默认 dry-run，force 模式生成新 id、设置 `replay_of`、跳过 delivery audit event，并把 publish failure 计为失败。证据：`src/event/replay.rs:108-190`、`:277-451`。

## 14. 数据架构

Goal-first数据流使用detached逻辑backup/query-only与私有exclusive新产物；不运行DatabaseManager::init。manifest/SHA定位loaded bytes和claimed provenance，不能认证历史PIT或重建source authority；quality screen只出候选，R1/R5独立资格缺失保持unavailable。archive与prune是不同显式操作，install/rollback不清理原件。

### 14.1 物理存储拓扑

```mermaid
flowchart TB
    subgraph Business["Business SQLite · Diesel/r2d2"]
        legacy["Generation-1 frozen catalog\n53 tables + 44 indexes + 63 triggers"]
        selection["selection-v2 final\n12 tables + 5 indexes + 17 static triggers"]
        incremental["Module-owned incremental DDL\nattribution / benchmark / catalyst / AI / analytics..."]
        attest["global schema owner + descriptor attestation"]
        attest --> legacy
        attest --> selection
        attest --> incremental
    end

    subgraph Durable["Durable SQLite · rusqlite"]
        delivery["18 durable delivery tables"]
        state["decision state + lease/fence/reservation"]
        recovery["replay/manual resolution/read model"]
        delivery --> state --> recovery
    end

    subgraph Files["Filesystem authorities"]
        eventAudit["data/event_audit\nhash-chain + sync_data"]
        durableAudit["data/durable_delivery_audit\nhash-chain + fsync"]
        eventJsonl["daily event JSONL\nobservation/replay"]
        reports["reports/ + reports/details"]
        locks["production/test lock namespaces"]
    end
```

### 14.2 Business DB owner

`DatabaseManager` 是 `OnceCell` singleton，持有 operational pool、optional attribution pool、descriptor sources、readonly attribution snapshot 与 selection schema authority。字段 drop order 也是安全合同：pool connections 必须先于 owner descriptors 和 global schema maintenance lease 释放。证据：`src/database/mod.rs:419-435`。

连接安全包括：

- Diesel `SqliteConnection` + r2d2 manager；每个新建或重建连接执行固定 PRAGMA 配置。证据：`src/database/mod.rs:1893-2255`。
- descriptor-bound main/WAL/SHM identity、connection attestation token、read-back connection 与 namespace-swap 检测。证据：`src/database/mod.rs:900-2011`。
- selection/attribution authoritative reads 不接受 caller-supplied connection；query-only 与 retained proof 在 owner 内验证。证据：`src/database/mod.rs:2706-2873`。
- global schema application id/generation 与 SQLite version range 被校验。证据：`src/database/global_schema_catalog_v1.rs:153-156`。

### 14.3 三类业务 schema

| 类别 | 定义方式 | 说明 |
| --- | --- | --- |
| legacy generation-1 | frozen TSV catalog + DDL fixture | 53/44/63 的基线，完整表清单见附录 D |
| selection-v2 final | typed catalog plan + canonical DDL | 12/5/17，另有动态 membership/symbol triggers，见附录 E |
| incremental owners | 各模块的 explicit DDL/migration | 不属于 53 frozen 数量；必须按 owner 管理 |

已观察的增量生产表族：

- attribution epoch：`attribution_sample_epoch_receipt(_chain)`、`attribution_legacy_carry_item`、`attribution_epoch_attempt_audit/_chain`、`paper_attribution_epoch_daily/_chain`；owner `src/database/attribution_epochs.rs`。
- attribution report：`attribution_report_epoch_binding/_chain`、`attribution_run_audit/_chain`、`attribution_report_revision/_chain`、`attribution_failure_audit/_chain`；owner `src/database/attribution_reports.rs`。
- benchmark：`benchmark_segment_revision/_chain`、`benchmark_manifest`、`benchmark_manifest_acquisition`、`benchmark_manifest_chain`；owner `src/database/benchmark_segments.rs`。
- catalyst：`catalyst_watchlist_run/daily/outcome`；owner `src/database/catalyst_watchlist.rs`。
- news AI delivery：`news_ai_delivery_event/_chain`；owner `src/database/news_ai.rs`。
- paper inventory audit：`paper_inventory_failure_audit/_chain`；owner `src/database/paper_inventory_failure_audit.rs`。
- closing valuation：`closing_valuation_run`、`closing_valuation_item`；owner `src/database/closing_valuation.rs`，两表都有 no-update/no-delete triggers，run identity 包含 price date/provider/positions。
- newer projections：`daily_change_confirmation_v2/_chain_v2`、selection generation cadence/audit closure 两表、`candidate_trigger_selection`、`holding_plan_daily`、`push_analytics`。

测试表、migration 中间表（例如 `_v3/_v4/_v5`）和 `TEST_CODE_*` 不计入生产目录。

### 14.4 数据质量模型

| 规则 | 表达 |
| --- | --- |
| 来源确认空 | `GatewayBatch::VerifiedEmpty(evidence)`，区别于错误或非法 `Available([])` |
| 时间证据 | `source_at`、`observed_at`、business/trading date、freshness thresholds |
| 批次身份 | provider/source/batch_id/content hash；转换必须保持 identity |
| 不可信数据 | typed GatewayError / rejection / retryable；不得补零或伪造字段 |
| 幂等/唯一 | selection receipt、event id、delivery decision/attempt、business-date-once claim |
| 篡改证据 | chain tables、no-update/no-delete triggers、SHA-256 canonical bytes |

## 15. AI、LLM 与 Agent 子系统

当前Phase A通过 `assistant_review::compare` 的FrozenPack调用bounded provider seam；不使用历史AgentRunner写日志/工具循环路径。默认offline0calls，无credentials要求；只有显式model/pricing/cash ceiling可走两次最多one-POST尝试。report64MiB/manifest2MiB/output8MiB独立有限；CLI input65536/request131072，18+2s、response32000/content16000、output1500与rounded reservation/no-refund保留。缺sources/真实billing与人类效益仍是不同验收。

```mermaid
flowchart LR
    caller["analyzer / news / deep_analyzer / ticker extraction"]
    registry["LlmRegistry\nrole + fallback"]
    provider["dyn LlmProvider"]
    receipt{"receipt required?"}
    model["EXTERNAL OpenAI-compatible endpoint"]
    agent["AgentRunner"]
    tools["Toolbelt\nfinancial/chip/flow/news/research/sector"]
    validators["ValidationEngine"]
    multi["CONDITIONAL multi-agent\nslices → analysts → debate → arbitrator"]

    caller --> registry --> provider --> model
    provider --> receipt
    caller --> agent --> tools
    agent --> validators
    caller --> multi --> provider
```

架构规则：

- `LlmProvider::chat_json_with_receipt` 默认 fail-closed 为 `ReceiptUnavailable`；只有绑定真实 upstream response id/model 的实现可 override。证据：`src/llm/mod.rs:198-225`。
- registry 从 `LLM_ROLE_<NAME>` 与 `LLM_DEFAULT_FALLBACK` 选 provider；无可用 provider 返回 None 并记录。证据：`src/llm/registry.rs:28-109`。
- ticker extractor 的业务降级是空结果，不把推断当事实。证据：`src/llm/ticker_extractor.rs:50-80`。
- AgentRunner 限制重复相同 tool call、清除失败事实、运行 validators，并可切换 model fallback。证据：`src/agent/loop_runner.rs:86-310`。
- `CONDITIONAL` multi-agent 真实存在，但只由明确入口启用，例如宏观路径 `MACRO_AGENT_PIPELINE` 或 CLI `--deep-analysis`；默认 `AnalysisPipeline` 不是全量 agent orchestration。证据：`src/analyzer/macro_rec.rs:78-95`、`src/app/modes.rs:16-35`。
- NewsAI 使用 `NEWS_AI_ANALYSIS_VERSION=news_ai_v2`，产业链字段进入 normalized prompt 与 evidence hash；证券名称严格是 display-only，不参与 dedup identity。链上下文读取失败降级为空字段，不阻塞单条评估，也不得推断缺失事实。证据：`src/monitor/news_ai.rs:35-38,589-600,2157-2165`、`src/bin/monitor/news_ai_shadow.rs:399-537`。

## 16. 依赖方向与已知环

```mermaid
flowchart TB
    entry["Entrypoints\nmain + src/bin"] --> app["Application orchestration\napp / pipeline / monitor-bin"]
    app --> domains["Domain capabilities\nselection / decision / risk / opportunity / review"]
    domains --> marketDomain["Provider-neutral types\nmarket_domain + domain models"]
    domains --> gateways["Ports and gateways\ndata_gateway / grpc_contract / traits"]
    gateways --> infra["In-repo infrastructure\ngrpc_client / DB / HTTP / sinks"]
    infra --> externalHost["EXTERNAL provider-host"]
    domains --> db["database repositories"]
    db -. "existing reverse type dependency" .-> domains
    gateways -. "audit persistence" .-> db
    db -. "tests/helper call" .-> gateways
```

理想方向是 entrypoint → application → domain → gateway/infra，但当前不是严格分层：

- `data_gateway/grpc_source.rs` 依赖 database acquisition audit 与 `DatabaseManager`。
- `database/mod.rs` 构造 selection schema types；database tests 还调用 data_gateway helper。
- pipeline position tracker 同时依赖 database、`monitor::risk`、`risk` 与 data_gateway。
- monitor binary-local 模块承担较多 composition 和部分业务 owner。

这些是当前实现事实，不应画成不存在的纯 hexagonal architecture。边界主要由源码扫描测试而非多 crate 编译边界执行。证据：`tests/unified_data_architecture.rs`、`tests/test_design_contradiction.rs`。

## 17. 横切关注点

### 17.1 认证与秘密

| 场景 | 当前实现 | 代码依据 |
| --- | --- | --- |
| CLI operator | 启动前 `require_monitor_operator_auth`，失败 exit 1 | `src/main.rs:29-34`、`src/auth/` |
| gRPC request | Bearer metadata 可来自环境；authorization 不进入 debug/tracing | `src/grpc_client/auth.rs:1-77` |
| remote bundle | HTTPS + CA + client cert/key + server name + instance Bearer | `src/grpc_client/bundle.rs:68-121`、`client.rs:69-109` |
| secret hygiene | `Zeroizing<String>`；authorization 不进 debug/tracing | `src/grpc_client/auth.rs:29-77` |
| deployment permissions | bundle 0700，token/key/cert/connection 0600 | `README.md:149-161` |

### 17.2 配置与 feature flags

- `.env`：credentials、paths、mode、feature toggles；CLI/monitor 启动加载。
- `config/strategy.toml`：risk + monitor + opportunity projections。
- `config/chain.toml`：chain rules、announce keywords、exclusion boards。
- `config/selection/*.json`：trading calendar、provider board bindings、activation。
- `config/design_contracts.toml`：coverage/governance contracts。
- `config::load_all()` 读取两份 TOML 到 `ArcSwap`/`RwLock` snapshots；解析失败保留前值/默认值。证据：`src/config.rs:492-736`。
- 只有 CLI schedule 每轮 override `.env`；monitor 没有相同的全量热重载承诺。
- Cargo 当前没有项目级 `[features]`；provider 隔离由仓库边界、Cargo dependency gate 和唯一远程 gRPC 路径执行，而不是 feature switch。
- 市场数据连接只接受 `GRPC_MARKET_ADDR` 与可选 `GRPC_MARKET_CLIENT_BUNDLE`；不存在 local/provider 运行时选择开关。
- `PAPER_SELL_DISABLED=1` 是模拟卖出逃生口；selection release 仍受 hash、effective time、executable evidence activation 控制。

### 17.3 错误、韧性与并发

- 边界使用 `anyhow`、`thiserror`、typed domain failures；provider/retryability 尽量保真。
- 同步 Gateway、SQLite、文件或 CPU 边界通过 `spawn_blocking`/专用 runtime 隔离，避免阻塞 Tokio worker。
- Tokio broadcast 的 lag 是可观察丢失，不是 durable guarantee。
- gRPC retry 有分类、有界、指数退避；request/auth/contract errors 不重试。
- monitor 长任务由 supervisor 统一观察；JSONL writer 或 main loop fatal failure 导致 exit 2。
- singleton lease、fence token、reservation、cross-process lock 防止双 owner；测试 namespace 与 production namespace 强隔离。

### 17.4 日志、指标与可观察性

`monitor --health --json` 已有 `runtime_snapshot.version=2`，读取banner-v2/heartbeat与既有production lease；valid unhealthy exit1不是调用失败。独立STAGED watchdog检测process/account/data、600s heartbeat+60s poll、会话calendar和local dedup/recovery；events/self-check不证明手机送达/用户阅读。未安装运行的证据不得从fixtures外推。

- `env_logger` 使用本地毫秒时间、level、target。证据：`src/main.rs:37-51`。
- event bus 暴露 published/no-subscriber/rejected metrics。证据：`src/event/bus.rs:43-58`、`:156-166`。
- L4 dispatcher 与 L7 analytics 有计数/持久化统计，但它们不替代 durable delivery read model。
- health/capabilities、opening static/live readiness、data-mode/account-mode banner、audit preflight、startup reconciliation 都有结构化 reason code/日志。
- `src/bin/monitor/metrics.rs` 与 `prometheus` 依赖存在，但该 module 未被 monitor composition root 接线，也没有 HTTP listener；因此当前没有可达的 Prometheus/OpenTelemetry exporter或集中式 tracing pipeline，应标 `INACTIVE prototype`，不能从文件存在推断 production metrics endpoint。

## 18. Rust 实现模式

### 18.1 Trait ports + `Arc<dyn Trait>`

```rust
pub trait AuthoritativeSinkPort: Send + Sync {
    fn sink_identity(&self) -> &str;
    fn deliver(&self, request: &AuthoritativeDeliveryRequest) -> AuthoritativeSinkResult;
}

pub type AuthoritativeSink = Arc<dyn AuthoritativeSinkPort>;
```

来源：`src/durable_delivery/model.rs:1176-1181`。适合需要 test double、运行时实现选择、且接口比实现更稳定的边界。

### 18.2 Default method 作为 fail-closed capability

```rust
async fn chat_json_with_receipt(
    &self,
    _system: &str,
    _user: &str,
) -> Result<ReceiptBearingJson, LlmError> {
    Err(LlmError::ReceiptUnavailable {
        provider: self.name().to_string(),
        model: self.model().to_string(),
    })
}
```

来源：`src/llm/mod.rs:212-225`。默认实现不伪造高保证 capability，真实 provider 必须显式证明。

### 18.3 显式 capability catalog

```rust
pub fn is_implemented(op: Operation) -> bool {
    implemented_operations().contains(&op)
}
```

来源：`src/grpc_contract/ops.rs:130-132`。proto 中存在不代表本仓 consumer 会使用；40-op catalog 与外部 provider-host capability negotiation 共同决定生产可用性。

### 18.4 单一远程 Lazy bridge

```rust
pub fn bridge_for(_op: &str) -> Result<Arc<GrpcSource>, GatewayError> {
    // process-wide cache; network connection is established on first query
    let source = GrpcSource {
        addr: std::env::var("GRPC_MARKET_ADDR")
            .unwrap_or_else(|_| DEFAULT_ADDR.to_string()),
        external_bundle: std::env::var_os("GRPC_MARKET_CLIENT_BUNDLE")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from),
        // ... lazy clients
    };
    Ok(Arc::new(source))
}
```

来源：`src/data_gateway/grpc_source.rs:1063-1087`。同一个 typed gateway API 只有远程 transport；地址、认证或证据失败均显式返回，不存在 library provider fallback。

### 18.5 Once-initialized process owners

```rust
pub struct DatabaseManager {
    pool: DbPool,
    attribution_pool: Option<DbPool>,
    // descriptor-bound authorities omitted
}

static DB_INSTANCE: OnceCell<DatabaseManager> = OnceCell::new();
```

来源：`src/database/mod.rs:419-435`。项目还使用 `OnceLock`、`LazyLock`、`ArcSwap` 管理 process-scoped bridge、config、bus 与 runtime owner；测试通常提供 local instance 或显式 namespace，避免共享全局状态污染。

### 18.6 显式状态转换白名单

```rust
fn legal_transition(from: DecisionState, to: DecisionState) -> bool {
    matches!((from, to),
        (DecisionState::Reserved, DecisionState::AttemptInFlight)
        // ...其余合法 pair 显式列出
    )
}
```

来源：`src/durable_delivery/coordinator.rs:7123-7150`。状态机用 pair whitelist 而不是“非终态即可迁移”的宽规则。

### 18.7 其他 Rust-specific 选择

- `async_trait` 用于 LLM 与异步 sink/tool ports。
- Tokio `join!` 表达长期并发任务；`spawn_blocking` 隔离同步 provider/SQLite/CPU 边界。
- Diesel 负责业务 DB typed schema/repositories；rusqlite 负责 durable coordinator 的精细事务与 trigger 控制。
- `build.rs` 合并 proto compatibility extensions，并同时生成 client 与 integration fixture 使用的 server trait；生成 server trait不等于拥有 production server。
- `cfg(test)`、`tests/support/grpc_fixture`、TEST_CODE namespace、isolated roots 是测试隔离策略；production provider-host 始终在仓库外。

## 19. 测试架构与质量门禁

2026-10-09局部交付复用各任务已审范围检查，新增actual Rust producer0/76/4096→FrozenPack/fake三臂容量、零调用默认/低ceiling拒绝、resource0400/singlelink、fixed installer/hash/drift/nooverwrite/relocation覆盖。完整commands/失败历史见永久证据；没有全suite/production probe。E3大fixtureRSS下降79.639%，小fixturelatency上涨31.8705%约2.855ms；debug warm-cache观察不是生产speedup，optimized Criterion/three-way builds/真实capture-counting仍未测。

```mermaid
flowchart LR
    unit["Inline unit tests\nmodule invariants"]
    integration["41 integration tests\ncross-module contracts"]
    sourceScan["Source-scanning architecture tests"]
    process["Child-process isolation / crash tests"]
    bench["intraday_tick benchmark"]
    ci["GitHub Actions"]

    unit --> ci
    integration --> ci
    sourceScan --> ci
    process --> ci
    bench -. manual/perf .-> ci
    ci --> fmt["fmt"]
    ci --> clippy["clippy -D warnings"]
    ci --> tests["all targets/features"]
    ci --> compliance["offline compliance + PR evidence"]
    ci --> coverage["llvm-cov Gate C"]
    repoGate["repository static gate\ncheck-no-magic-dependencies"]
    repoGate -. manual / not wired to CI .-> sourceScan
```

### 19.1 测试层次

| 层次 | 当前模式 | 示例 |
| --- | --- | --- |
| module unit | 与实现同文件，覆盖 parsing、state、schema、retry、renderer | `grpc_contract::ops`、`durable_delivery`、`selection_v2` |
| integration | `tests/*.rs`，验证跨模块、E2E、架构边界 | `grpc_bridge_e2e`、`durable_delivery_counted_cutover` |
| source contract | 扫描源码/调用点，防止绕过 gateway 或恢复禁用调用 | `unified_data_architecture`、`test_design_contradiction` |
| process isolation | child binary/process、foreign CWD、crash/restart、help storage-free | `tool_binary_process_isolation`、`monitor_help_isolation` |
| schema/attestation | SQLite catalog、trigger、descriptor identity、read-back proof | database/selection/durable 内联 tests |
| benchmark | Criterion-like Cargo bench target `intraday_tick` | `benches/intraday_tick.rs` |

### 19.2 CI 事实

| Workflow | 当前门禁 | 证据 |
| --- | --- | --- |
| Rust CI | fmt；strict clippy all targets/features；test all targets/features | `.github/workflows/ci.yml` |
| compliance | offline compliance；选择的 unit/e2e；PR commit spec refs | `.github/workflows/compliance.yml` |
| coverage | Rust 1.95.0；cargo-llvm-cov 0.8.7；workspace all-features；diff Gate C | `.github/workflows/coverage.yml` |
| PR template lint | monitor/push/notify 变更要求 v13/v14.2 refs | `.github/workflows/pr-template-lint.yml` |

注意：`scripts/check-no-magic-dependencies.sh all` 当前是仓库内可运行的静态门禁，但未接入上述 workflows；不得写成 CI 已执行。`compliance.yml` 仍引用 Cargo metadata 中不存在的 `--test e2e`，属于待修 CI 配置漂移。架构维护时应验证 workflow 命令真实可解析，而不是只读注释。

## 20. 构建、部署与运行手册

Isolated工具包布局与具体plan-default installer见[交付runbook](ops/goal-first-delivery-runbook.md)。版本发布至runtime/tools/goal-first/<version>，不sync checkout/src/config、不覆盖monitor/bridge/DB/locks。只有manifest中全部monitor-bound输入与binary/prior launcher哈希不变，才无需activation/restart；若改动则拒绝此范围。Activation v2实际绑定所有src/config（除activation文件）、Cargo*.toml/Cargo.lock/build.rs与COMPILED_PUBLIC_CONTRACT_INPUTS，不限于旧src/config概括。记录activation declared expected hash、diskbinary hash与live attestation三者不同。Rollback恢复确切旧launcher/plist bytes，保留local events/reports/archive，禁止恢复DB/重发Unknown/旧Desktopplist。

### 20.1 构建矩阵

| 目标 | 命令 | provider linkage |
| --- | --- | --- |
| default CLI | `cargo build --release --bin stock_analysis` | gRPC consumer；不链接 provider implementation |
| production monitor | `cargo build --release --bin monitor` | gRPC consumer + durable owner |
| client probes | `cargo build --release --bin grpc_bundle_probe --bin grpc_local_readiness_probe` | 只验证已运行的外部 endpoint |
| attribution repair | `cargo build --release --bin attribution_backfill` | 使用 HistoricalBarsGateway + attribution epoch authority |

依据：`README.md:76-115`、Cargo metadata。仓库没有 provider server 构建目标。

### 20.2 启动顺序

```mermaid
sequenceDiagram
    participant O as operator
    participant H as EXTERNAL provider-host
    participant P as grpc_bundle_probe
    participant M as monitor

    O->>H: provision outside this repository
    H-->>O: endpoint health/capabilities ready
    O->>P: --bundle ... --opening
    P->>H: static/live contract and credential checks
    P-->>O: readiness result
    O->>M: MONITOR_ENABLED=true + address/bundle + DB/sink config
    M->>H: typed gRPC data calls
    M->>M: lease + audit + durable reconciliation
```

运行约束：

- provider-host 的 listener、provider credentials 和服务端数据持久化属于外部部署责任；本仓不拥有其生命周期。
- 一个生产 namespace 只能有一个 monitor delivery lease owner。
- monitor 需要 business DB、durable DB、event/durable audit directories、locks、client bundle 和 sink credentials。
- remote bundle 必须先通过 health/capability/opening gates。
- 当前仓库没有自动创建 OS service、容器镜像或集群资源的实现；这些属于部署环境责任。

### 20.3 运行产物

| 产物 | 典型路径 | 权威性 |
| --- | --- | --- |
| business DB | `data/stock_analysis.db` 或 `DATABASE_PATH` | 业务事实/读模型 |
| durable DB | `data/durable_delivery.sqlite3` | 投递状态与恢复 authority |
| event delivery audit | `data/event_audit` | 权威 hash-chain delivery evidence |
| durable immutable audit | `data/durable_delivery_audit` | durable coordinator exact-byte authority |
| event JSONL | daily JSONL base dir | non-authoritative observation/replay |
| reports | `reports/`、`reports/details/` | 人类可读投影 |
| locks | `data/locks/production/...` 或 isolated test roots | ownership/fencing |

## 21. 扩展蓝图

### 21.1 新增一个 Data Gateway capability

1. 在 `src/data_gateway/` 定义 request、record、`GatewayBatch<T>` 返回与 evidence validation；provider-specific 类型不得泄漏给业务层。
2. 在外部 provider-host 仓库实现 provider-specific acquisition；本仓不得重新引入 provider SDK、provider server target 或本地采集 fallback。
3. 明确 `Available`/`VerifiedEmpty`/error/retryability/freshness 语义。
4. 任何新市场数据 capability 都继续执行 21.2 的 gRPC 合同扩展步骤；不存在“仅本地 provider route”。
5. 添加真实 consumer integration test、bridge hook/source scan test、错误分类测试。
6. 更新 `data-sources-inventory`、本蓝图 capability 表与 evidence matrix。

### 21.2 新增一个 gRPC operation

1. 优先更新上游 `client-bundle/market.proto`；只有明确兼容需求才在 `build.rs` 添加 local extension。
2. 更新 `grpc_contract::ops::method_name`；若生产可用，再加入 `implemented_operations()`。
3. 在 params/schema/validate 中定义版本和字段约束。
4. 在外部 provider-host 实现并发布对应 capability；通过版本/health/capability probe 证明可用，本仓不增加 production server handlers。
5. 在 `grpc_client` 增加 RPC dispatch 与 decode；在 `data_gateway::grpc_source` 增加 wire-to-domain bridge method。
6. 将真实 gateway call site 纳入 `HOOKED_OPS`，不允许以 local fallback 绕过合同。
7. 用 `tests/support/grpc_fixture` 添加 client/contract roundtrip、bridge E2E、capability count 与全 enum mapping tests，并对真实外部 host 做独立部署验收。

### 21.3 新增 monitor 常驻任务

1. 判断它是 reusable monitor domain 能力还是 bin-local composition。
2. 定义终止/错误语义：返回 fatal、重试，还是只发 DQ observation。
3. 在 producer 激活前声明所需 startup gates；不得在 audit/durable reconciliation 前产生 counted delivery。
4. 若为主循环，加入 `main_loops`；若为 background JoinHandle，加入带名字的 `background_tasks`。
5. 确保 `supervise_long_running_lifecycle` 能观察它，避免 detached silent failure。
6. 更新 source-scanning lifecycle tests 与本蓝图任务树。

### 21.4 新增 selection feature/stage

1. 在 selection model/features 中定义 canonical input/output 与版本。
2. 通过 owner 构造 opaque prepared stage；不暴露 caller 可自造 receipt/run id 的 API。
3. 更新 canonical DDL/catalog、stage membership/symbol triggers、payload schema allowlist。
4. 更新 recovery envelope 与 ordered drain 逻辑。
5. activation gate 必须决定能力是否 release；disabled branch 要做到 provider/DB/sink/scheduler 全零副作用。
6. 增加 commit + audit + receipt read-back、crash boundary、process isolation tests。

### 21.5 新增 counted PushKind 或 production sink

1. 在 compiled policy catalog 定义 cooldown、daily budget、business-date-once 等策略。
2. 构造稳定 subject/occurrence/evidence/source binding 与 exact rendered bytes。
3. 若新增 sink，实现 `AuthoritativeSinkPort`，返回 Accepted/Rejected/Uncertain 的真实 typed receipt；不可把 HTTP 2xx 之外的模糊状态假定成功。
4. 通过 coordinator reserve → attempt → sink → audit → terminal transition；禁止 bus/L6 shortcut。
5. 更新 durable schema constraints、reconciliation、manual review、task transition payload。
6. 添加 crash-before/after-sink、foreign lease、duplicate、uncertain、audit append fault 和 restart E2E。

### 21.6 新增业务表

1. 先决定 owner：global frozen generation、selection-v2 final、module-owned incremental，或 durable isolated schema。
2. 业务 DB 使用 Diesel/owner migration；durable DB 使用 coordinator schema transaction；不要跨两个 DB 做假原子事务。
3. 对审计/receipt 表定义 canonical bytes/hash、FK、unique 与 no-update/no-delete constraints。
4. 更新 descriptor/global schema attestation 或相应 module catalog。
5. 添加 clean DB、existing DB、partial schema、future generation、namespace swap 与 rollback/fail-closed tests。
6. 更新附录 D/E/F 或增量表族清单。

## 22. 架构决策记录（由代码推导）

这些条目是 `INFERRED` architecture decisions，用于解释当前代码；如需正式治理，应另建 ADR 并记录批准者与日期。

### ADR-I01：provider host 是仓库外系统边界

- 状态：`INFERRED / CURRENT`
- 决策：本仓只保留 provider-neutral domain、gRPC client/contract 与 consumer admission；provider-host 在外部仓库独立构建部署。
- 动机：彻底隔离 provider linkage、credentials、阻塞采集与控制面，避免 monitor/CLI 加载 provider SDK。
- 代价：跨仓合同/能力发布必须协调，服务不可达时只能 fail-closed，不能靠本地 fallback 应急。
- 证据：`Cargo.toml`、`README.md:28-44`、`scripts/check-no-magic-dependencies.sh`、Cargo targets。

### ADR-I02：统一 Data Gateway 是唯一外部数据边界

- 状态：`INFERRED / CURRENT`
- 决策：业务代码只消费 gateway/`market_domain` types；所有市场数据从 external provider-host 进入，再执行本仓 evidence admission。
- 动机：保持 identity、freshness、empty/error、batch evidence 和 fail-closed 语义一致。
- 代价：data_gateway 体量很大，并与 audit database 形成反向依赖。
- 证据：`tests/unified_data_architecture.rs`、`src/data_gateway/mod.rs`。

### ADR-I03：durable delivery 物理隔离

- 状态：`INFERRED / CURRENT`
- 决策：delivery coordinator 使用独立 rusqlite DB 和独立 immutable audit，而非复用 Diesel business pool。
- 动机：租约/fence/恢复/状态机需要独立 authority 和精细事务控制。
- 代价：跨 DB 无原子提交；需要 startup reconciliation 和 payload/hydration bridge。
- 证据：`src/bin/monitor/durable_delivery_runtime.rs:76`、`src/durable_delivery/schema.rs`。

### ADR-I04：权威审计先于 observation bus

- 状态：`INFERRED / CURRENT`
- 决策：synchronous durable audit 才能确认 delivery；bus/JSONL 仅 observation/replay。
- 动机：broadcast 可 lag/no-subscriber，JSONL projection 可失败，均不满足 counted delivery authority。
- 代价：投递路径延迟更高，audit filesystem 成为启动与运行硬依赖。
- 证据：`src/bin/monitor/main.rs:4744-4748`、`src/event/jsonl_writer.rs:1-12`。

### ADR-I05：selection-v2 通过 activation gate 渐进发布

- 状态：`INFERRED / CONDITIONAL`
- 决策：代码可存在，但 generation/outcome capability 必须由 config/receipt/runtime evidence 释放。
- 动机：避免未闭合 schema/provider/outcome/recovery 直接产生生产副作用。
- 代价：activation/recovery/receipt 代码复杂，旧 selection 与 v2 并存。
- 证据：`src/selection/activation_runtime.rs`、monitor `selection_v2_enabled` 分支。

### ADR-I06：兼容层只保留业务/协议兼容，不保留 provider fallback

- 状态：`INFERRED / COMPAT`
- 决策：保留 NotificationService、push L1-L7、legacy selection 和 wire extensions 等业务/协议兼容层；provider SDK、本地 server 与本地市场数据 fallback 已删除。
- 动机：允许增量迁移与回归对照。
- 代价：同一能力存在多套抽象；必须清楚标注 production wiring，避免误接旧 bypass。
- 证据：`src/lib.rs` push 注释、`build.rs` compatibility extensions、`paper_engine` disabled、monitor-local durable runtime、no-Magic gate。

### ADR-I07：安全边界用 descriptor 与源码测试补强单 crate

- 状态：`INFERRED / CURRENT`
- 决策：不拆多个 crates，而用 descriptor attestation、opaque capabilities、source scans、process tests 执行边界。
- 动机：在单 package 内保持开发效率，同时保护关键 authority。
- 代价：编译器不能阻止所有跨模块依赖；source tests 对重命名敏感。
- 证据：`src/database/mod.rs` descriptor manager、`tests/unified_data_architecture.rs`。

## 23. 新开发治理模板

每个架构相关 PR 应回答：

| 问题 | 必填证据 |
| --- | --- |
| 它属于哪个运行单元？ | CLI/monitor/tool 或外部 provider-host；是否改变跨仓合同/部署依赖 |
| 它的 authority 是谁？ | 唯一 owner、lease/fence/receipt/descriptor |
| 数据从哪里来？ | gateway capability、provider、batch/evidence/freshness |
| 空、缺失、过期、模糊结果如何表达？ | typed empty/error/retryable/uncertain，不得补事实 |
| 写到哪里？ | business/durable/filesystem；说明是否可原子提交 |
| 是否产生外部副作用？ | governance、dedup、audit、sink receipt、dry-run isolation |
| 如何恢复？ | restart、partial commit、foreign owner、manual review |
| 哪些模式启用？ | default/feature/env/activation/session/test/compat 状态 |
| 哪些测试执行边界？ | unit、integration、source scan、process/crash、CI |
| 蓝图哪些清单受影响？ | modules/targets/ops/states/tables/tasks/decisions |

### 23.1 禁止的架构捷径

- 在本仓任何位置重新引入 provider SDK、provider-only target 或直接 provider acquisition。
- 以 `Vec::new()` 伪装 provider 确认空批次。
- 将 event bus publish 或 JSONL append 当成 counted delivery 成功。
- 在 production monitor/CLI 增加 library provider fallback，或在远程失败时伪装本地成功。
- caller 自造 selection receipt、due work、run id 或 delivery audit ref。
- 绕过 durable coordinator 直接调用 production sink。
- 把 test namespace、TEST_CODE receipt 或 arbitrary path 提升为 production authority。
- 新增 `DecisionState` 却不更新 transition/schema/recovery 全链。
- 宣称“热重载”却只重载 `.env` 或只更新一份 config projection。

## 24. 推送系统专项架构与演进路线

**HISTORICAL / FROZEN_PROPOSAL（2026-10-09）**：本节旧catalog数字/全量对账是2026-09基线；统一facade、PhaseScheduler、new intent/catalog和平台迁移路线冻结，不作为今天穷举清单或已交付实现。现行counted owner/状态语义按CLAUDE与source；本次仅E1–E4artifact工具，不提前实施E5。

> 状态：`PROPOSED`  
> 审计日期：2026-09-01 至 2026-09-02  
> 范围：65 个 `PushKind`、monitor 定时/事件 producer、presentation、push L1-L7、NotificationService、durable delivery、authoritative sink、业务完成游标与盘前/竞价/盘中/盘后调度。

本节是基于第 8、12、13、16、18、21 与 22 节当前架构事实形成的专项演进方案。它不改变以下既有架构决策：provider-host 位于仓库外、Data Gateway 作为唯一市场数据准入边界、durable delivery 物理隔离、权威审计先于 observation bus，以及业务兼容层的渐进迁移方式。

### 24.1 审计口径、状态与时段边界

本节的“所有推送项”包括两层：`PushKind` enum 的 65 个稳定业务名称，以及绕过 `PushKind` 的真实通知路径。enum 定义、等级与冷却只证明合同存在，不证明生产可达；生产状态以 composition root、scheduler、source capability、presentation gate、durable policy 和 sink 调用图的交集为准。主要证据入口是 `src/bin/monitor/notify.rs:45-197,289-460`、`src/bin/monitor/presentation_registry.rs:42-421`、`src/bin/monitor/main.rs:5464-5525,8796-11295` 与 `src/bin/monitor/push_templates.rs:9546-9818`。

“逐行”在本文中按可执行语义解释：每个时间判断、feature/环境 gate、取数分支、空数据分支、状态写入、渲染、governor、sink、回执、重试和完成游标都落到连续代码行范围；纯 import、括号和派生样板只说明结构作用，不虚构业务含义。每个 65-kind 行都至少带一个当前存在且未越界的 `path:line` 证据。

业务状态标签仅用于本专项清单：

| 标签 | 精确定义 |
| --- | --- |
| `ACTIVE` | 当前生产 composition root 有 producer；仍可能因非交易日、无数据、冷却、权限、证据或 sink 失败而不发送 |
| `STARVED` | 调度和 dispatcher 可达，但当前上游不再产生新的有效输入，只可能消费历史残留 |
| `OPT-IN` | producer 存在，但 checked-in/default 配置关闭；显式开关后才可进入发送链 |
| `INACTIVE` | 无 producer、结构不可达、source 未注册、硬编码 disabled，或 fail-closed 合同阻断 |

`MarketSession` 的真实边界是：盘前 `Closed(<09:15)`、集合竞价 `Auction([09:15,09:25))`、竞价后闭市间隙 `[09:25,09:30)`、早盘 `[09:30,11:30)`、午休 `[11:30,13:00)`、下午盘 `[13:00,15:00)`、盘后 `AfterHours(>=15:00)`；证据为 `src/calendar.rs:508-559`。下表按“主要业务归属”只收录一次，跨时段 producer 会在行内明确标出，因此分类计数不等于 scheduler 数量。

### 24.2 所有 monitor PushKind 共用的逐层逻辑

| 顺序 | 当前代码行为 | 成功/失败语义 | 证据 |
| ---: | --- | --- | --- |
| 1 | producer 从 Data Gateway、业务 DB、事件总线或 owner 文件读取结构化事实；各入口自行决定交易日、时窗、空数据和业务筛选 | 这是最分散、最容易把“计算完成”误当“通知完成”的一层 | `main.rs:8796-11295`；`push_templates.rs:2520-9538` |
| 2 | `PushKind` 给出 level、banner、cooldown 与 scope；`is_deprecated()` 当前永远为 false | enum/metadata 不代表 producer 已启用 | `notify.rs:203-286,289-534` |
| 3 | production presentation 以 `(family, kind, producer, renderer)` 精确签 token；当前 58 个 tuple 覆盖 54 个唯一 kind | token 只证明允许的展示形状，不证明数据源和调度可达 | `presentation_registry.rs:42-421` |
| 4 | generic governor 先拒绝 counted kind，再执行 launch/source/v14 gate、L4 去重与 L5 治理；counted kind 必须携带稳定 occurrence、subject、canonical evidence | `Denied` 不得当成送达；普通周期 helper 却把 `Deduped` 当 confirmed | `notify.rs:2213-2450,2891-2966`；`push_templates.rs:14617-14659` |
| 5 | counted 路径进入 `DurableDeliveryCoordinator`，执行 reserve、attempt、fence、typed sink result、immutable audit、terminal/reconcile | durable catalog 只有 23 个 kind；未列入的 kind 不得伪造 counted binding | `durable_delivery/model.rs:169-226,611-650`；`durable_delivery_runtime.rs:1354-1369,2270-2321` |
| 6 | monitor 物理层默认飞书；webhook 走 HTTP，否则走 magiclaw CLI；显式 `wechat` 才走微信。发送前必须先落 push log，鉴权失败仅重签后重试一次 | dry-run、尝试前拒绝、真实尝试成功/失败是不同结果；不能压成裸 `bool` | `notify.rs:3133-3387,4325-4329,4628-4662` |
| 7 | 物理成功后提交治理去重并写 L7/hash-chain；后置审计失败返回 `SinkError` 但避免盲目重发 | observation bus/JSONL 不是权威 receipt | `notify.rs:3653-3768`；`src/event/jsonl_writer.rs:1-12` |
| 8 | 盘后 `ReviewTask` 另有 delivered/no_data/deferred/disabled/failed 状态和 1/5/15 分钟退避 | 该路径把 `Deduped` 转为不可重试 Failed，与普通周期语义冲突 | `review_batch.rs:829-940,1190-1240,1507-1687` |

### 24.3 盘前推送（5 个 PushKind）

| PushKind / 状态 | 触发、业务逻辑与完成规则 | 代码证据 |
| --- | --- | --- |
| `AccountMode` / `ACTIVE` | 启动时评估，08:30 后当日补做；核对持久化旧状态与本次 evaluation，先写审计 pending，再推“旧→新模式、原因、限制、解除条件”。只有 `Pushed` 才把同一行标记完成；失败保留 pending 并重试。Frozen 新迁移还会附带 `MarketActionAlert`。 | `main.rs:5464-5487,9370-9430`；`push_templates.rs:1817-2069` |
| `DataMode` / `ACTIVE` | 启动及常驻 data-mode loop 评估 Full/Degraded/Unsafe；首次 Full 静默建基线，恶化到 Unsafe 立即推，其他抖动需稳定 5 分钟。Degraded 禁盘口判断，Unsafe 再禁价格建议；只有静默建基线或 `Pushed` 才 confirmed。此项跨盘前、竞价、盘中和盘后持续监测，主归属启动/盘前。 | `main.rs:5464-5525`；`push_templates.rs:14312-14477` |
| `SnapshotStale` / `ACTIVE` | 启动时检查用户确认持仓快照；只有落后最近交易日至少 5 个交易日才提醒。`begin` 建 in-flight，只有 `Pushed/Deduped` 才 `finish` 当日完成，失败可重试。不要与 15:05 复用 `IntradayMarket` 的“超过 6 小时”提醒混为一项。 | `main.rs:1900-1965,9241-9294` |
| `PreopenNewsHot` / `ACTIVE` | P-01 仅交易日 `[09:00,09:15)` 每 30 秒检查；读取上一交易日涨停池 200 条，取产业链前三主线首票并绑定证券身份与逐票新浪新闻；总新闻数为 0 则终态拒绝。先 inspect/resume durable claim，再渲染和 counted send；缺权威终态时即使底层称 Pushed/Deduped 也判失败。 | `p01.rs:19-20,101-150,484-1035,1284-1392,1488-1601`；`push_templates.rs:14744-14811` |
| `CandidateTriggered` / `INACTIVE` | 设计意图为 09:00--09:15 候选转正；实际代码位于“已经等到市场 active”之后却要求 `Closed`，结构无交集。即使人工开关通过，caller 传 `promotion_evidence=None`，且当前无 durable lifecycle transition owner，dispatcher 必须 fail closed。 | `main.rs:9442-9457,9590-9616`；`push_templates.rs:5714-5835,14227-14265` |

补充：09:10 行情预检不是独立 `PushKind`，它复用 `IntradayMarket`；与 `CandidateTriggered` 位于同一不可达分支，而且探测一执行就封当天，外发失败也不重试。证据：`main.rs:9618-9688`。

### 24.4 集合竞价推送（7 个 PushKind）

| PushKind / 状态 | 触发、业务逻辑与完成规则 | 代码证据 |
| --- | --- | --- |
| `AuctionVolume` / `ACTIVE` | 09:20 后每个竞价 tick 读取当日涨停池，按量比降序选未通知 Top10；只有 dispatcher 返回 true 才把 code 写入进程内 `auction_vol_notified`，banner/token/sink 失败可在竞价窗口再试。 | `main.rs:9692-9773`；`push_templates.rs:5935-5984` |
| `AuctionRepush` / `ACTIVE` | 与 CandidateBoard 同一 tick；过滤价格/热度缺失项，Strong 优先、再按热度降序取 Top5，展示首来源、现价、热度。它与 CandidateBoard 都成功才封本轮，否则下一 tick 继续，由各自 cooldown 再防重复。 | `main.rs:9775-9801`；`push_templates.rs:7184-7275` |
| `CandidateBoard` / `ACTIVE` | 合并 P5 文件、持仓和 chain 等候选，输出排序候选台；发送前先做失效 diff、采样并把本轮 code 快照写盘，最后才发送，所以发送失败时 diff 基线已经推进。 | `push_templates.rs:3517-3655,7752-7865` |
| `CandidateInvalidated` / `ACTIVE` | CandidateBoard 将“上一快照存在、本轮消失”的 code 逐票发 T-08，内容是旧状态→Invalidated 与原因；名称可能因本轮已无 entry 回退为 code。它不是独立 scheduler。 | `push_templates.rs:703-743,7752-7865,14268-14286` |
| `LimitBoards` / `ACTIVE` | 只处理存在主力净流数据的涨停股，最多查 40 个板级、排序最多取 50 个；首板/二板/三板分别取 token 并发三张卡，但共享同一 kind 冷却/预算。代码先 `board_notified.insert` 再渲染发送，失败后同日不会重试该票。 | `main.rs:10241-10416` |
| `VirtualWatch` / `STARVED` | Confirm 模式且早盘候选非空、价格全为 0 时初始化；报价按持仓→涨停池→实时行情补齐，正价项先持久化 snapshot 再推，返回值被丢弃且日志无条件写“已推送”。但唯一候选写入来自固定空字符串 `post_close`，当前没有新输入。 | `main.rs:9803-9967,10103-10238` |
| `PaperTrade` / `ACTIVE` | 竞价工作流约 09:15--09:20 每 30 秒消费当日已持久化终态；逐条要求合法 A 股、buy/sell、正价、整百股数量、终态及唯一 order audit/hash chain，SQL 精确联接 plan/source/reason。所有行 Pushed/Deduped 才 confirmed。 | `main.rs:10043-10057`；`push_templates.rs:5319-5693` |

### 24.5 盘中推送（22 个 PushKind，含尾盘集合竞价与事件驱动）

| PushKind / 状态 | 触发、业务逻辑与完成规则 | 代码证据 |
| --- | --- | --- |
| `HoldingEvent` / `INACTIVE` | 仍有紧急级 renderer/metadata，但 legacy summary 在渲染前停用；LimitUp/LimitDown、主力流入流出、放量、炸板等内存告警因没有 durable lifecycle owner 统一 fail closed，不外发。 | `main.rs:9520-9523,10555-10712,11514-11530` |
| `Announcement` / `ACTIVE` | news loop 从 EventCalendarGateway 取当日最多 300 条，关键词分类并用概念/名称补 code；受众为自选+24 小时内确认持仓。每个输入经生命周期、分类、受众、dedup/source-fact gate，只有 disposition=Pushed 才进入后续催化。当前配置可让 news loop 全天运行，主归属事件驱动。 | `main.rs:7293-7454,7464-7528,8000-8124`；`v17_sources.rs:714-787` |
| `SectorTop` / `ACTIVE` | 每小时尝试一次，从 Concept 板块排行生成领涨 Top；构造全局 counted binding。空批按 confirmed empty，投递成功或失败当前都会重置小时 timer。 | `main.rs:10990-11048`；`push_templates.rs:16334-16423` |
| `SectorAnomaly` / `ACTIVE` | 与 SectorTop 同一小时调度，检测量价反向；读取财联社 20 条并取前 10 标题作归因，新闻失败用空归因文本，非空 moves 才构造全局 counted binding。当前无论成功失败也重置 timer。 | `main.rs:10990-11048`；`push_templates.rs:14087-14152,16429-16456` |
| `FundInflow` / `INACTIVE` | enum 仅保留“主力净流入 Top10”元数据；生产源码精确引用扫描没有 dispatcher/caller。当前板块资金展示由 `IntradayMarket`/`SectorTop` 承担，不能把模板名称视为在推。 | `notify.rs:60-61`；`main.rs:10810-10855,10990-11048` |
| `TurnoverTop` / `INACTIVE` | 有真实板块成分加载、换手率排序 Top10 和 renderer，但没有生产 scheduler/dispatcher 调用；因此是“实现存在、无 producer”。 | `push_templates.rs:1053-1161` |
| `NewsRanked` / `INACTIVE` | enum 注释和启动审计语义均为 shadow ranker 无生产调用；只剩 level/适配/测试清单引用。 | `notify.rs:84-91` |
| `HoldingPlan` / `ACTIVE` | 每 30 分钟基于确认持仓+行情逐票生成；空仓静默，缺行情/非法成本跳过；盈亏 >+5% Reduce、<-3% Add、否则 Hold，并给减仓区/支撑/压力/止损。票级 durable occurrence；Pushed/Deduped 后写跨重启一日一票表，全部确认才推进 timer。 | `main.rs:8460-8620,10857-10986` |
| `T0Advice` / `ACTIVE` | 每 30 秒到期，按持仓与 Magic TDX 批次生成逐票建议，展示趋势、均价/ATR、量能/五档、卖出/接回区、数量和失效条件；全批 Pushed/Deduped（空批也算）才推进 timer，失败立即保留重试。 | `main.rs:10735-10804`；`push_templates.rs:518-567` |
| `ForbiddenOps` / `INACTIVE` | T-09 renderer 可输出结论和多条禁因，durable enum 也保留，但当前告警 producer 因缺生命周期 binding 被 `reject_unbound_alert_delivery` 统一阻断，没有生产发送调用。 | `push_templates.rs:703-743`；`main.rs:10555-10712,11514-11530` |
| `PaperSell` / `ACTIVE` | 默认启用，仅 `PAPER_SELL_DISABLED=1` 暂停；盘中每 30 秒随决策 tick、盘后 15:30 再扫，只有真实风险上下文才执行，逐票普通 governor。发送失败只告警，没有 durable counted receipt。 | `main.rs:8246-8274,8816-8876,8928-8967` |
| `CloseCall` / `ACTIVE` | 14:55 后读取确认持仓，只对相对成本跌幅 ≤-3% 的票生成“尾盘跳水”；逐票 `close-call:{date}:{code}` counted binding。只有所有票 confirmed 才封当天，失败保留重试。 | `main.rs:8624-8707,11138-11210` |
| `IntradayMarket` / `ACTIVE` | 主路径每 5 分钟读取 Concept 板块 1 日资金流 Top10，真实空推进 timer、取数失败不推进、仅 confirmed delivery 推进。该 kind 还被 09:10 不可达预检和 15:05 快照新鲜度提醒复用，导致三种业务共享冷却/指标。 | `main.rs:9618-9688,10810-10855,9241-9294`；`push_templates.rs:2520-2664` |
| `NewsCatalyst` / `ACTIVE` | 公告路由本轮至少一条 Important 才触发；优先最新 board_rotation、校验最多 9 只股票，无 rotation 回退 chain_daily，LLM 为可选增强，失败/空回退主题规则。 | `main.rs:8126-8192`；`push_templates.rs:2670-3050,14068-14084` |
| `NewsToIdea` / `ACTIVE` | 与 NewsCatalyst 共享 Important 新闻触发；从统一候选批取第一名，按 source_count 定阶段、涨幅决定 DoNotChase/BuyDip/Observe，可选 LLM 最多 3 条理由。先确认推送，BuyDip 才模拟买 100 股；虚拟买入失败会使本轮 false 且不写 1 小时 memo。 | `main.rs:8126-8192`；`push_templates.rs:3517-3655,3888-4187` |
| `IndustryChainIntraday` / `ACTIVE` | 每 15 分钟调用产业链扩散；空来源 confirmed empty，有候选时可用 LLM 生成 trigger，失败回退原 trigger。只有 confirmed 才推进 timer；成功后保存 pushed_stocks，审计写失败会把周期改为 Failed。 | `main.rs:10857-10926`；`push_templates.rs:3337-3468` |
| `StPriceLimitChanged` / `ACTIVE` | 09:30 后一次性检查 ST 持仓，展示 ST 类型、5%→10% 规则参数、成本/现价与新止盈止损；dispatcher 成功才封当天。 | `main.rs:11098-11123`；`push_templates.rs:5029-5088` |
| `EtfClosingCallAuction` / `INACTIVE` | 语义属于 14:57 尾盘集合竞价，不是开盘 Auction；到点后代码直接记录 `disabled=no_etf_auction_producer` 并封当天，不取数、不渲染、不发送。 | `main.rs:11212-11222` |
| `PolicyHit` / `INACTIVE` | classifier 要求非 research-only、完整 governed evidence、标题/source/发布日期，并允许全局无 code；但生产没有调用 classifier，`SourcePushKind::PolicyHit` 只在测试直接构造。 | `src/news/aggregator/classifier.rs:315-359`；`v17_sources.rs:714-787` |
| `MarketActionAlert` / `ACTIVE` | 事件路径只从 `MonitorEvent::OrderUpdate` 生成，以 code/action/shares 构 identity，内存状态未变化则跳过；AccountMode 进入 Frozen 时还会额外生成一次。按 Emergency 无条件绕过 launch gate。 | `v17_sources.rs:177-220`；`push_templates.rs:2027-2057`；`notify.rs:2440-2450` |
| `NewsFlashCritical` / `INACTIVE` | 虽有专用 typed-receipt 事务和 presentation，但 SourceOnly aggregator 完全不使用 critical threshold，所有合格新闻只进缓冲；测试若产生 N-01 reservation 会 panic，启动也声明无权威 strength provider。 | `news_aggregator_init.rs:136-149,478-609,1475-1476`；`main.rs:7641-7654` |
| `NewsFlashAggregated` / `ACTIVE` | 从不可变 event authority 预检后抓各 feed 20 条；固定在 09:30/11:30/13:00/15:00（首 tick 容差 5 分钟）按 strength 取 Top3。同窗口未决 reservation 只恢复不新建；仅 Accepted 计数，Rejected/Uncertain 精确 settle。 | `main.rs:7685-7817`；`news_aggregator_init.rs:520-605,1057-1145` |

### 24.6 盘后推送（31 个 PushKind）

| PushKind / 状态 | 触发、业务逻辑与完成规则 | 代码证据 |
| --- | --- | --- |
| `DailyReport` / `INACTIVE` | R-01/旧收盘汇总有 renderer 和 durable 映射，但收盘主循环因缺 immutable counted/source contract 明确停用；不能把 `FactorIC/SectorTier/CapitalVerify` 的 sub-kind 映射当作 generic DailyReport producer。 | `main.rs:11242-11276`；`push_templates.rs:1163-1198`；`durable_delivery_runtime.rs:2307-2318` |
| `FactorIC` / `INACTIVE` | 仅保留 `DailyReportSubKind::FactorIC` 稳定映射；没有生产 producer 构造该 `PushKind` 或完整 immutable binding。 | `notify.rs:64-65,228-286`；`durable_delivery_runtime.rs:2307-2316` |
| `SectorTier` / `INACTIVE` | 与 FactorIC 相同，只是 `DailyReportSubKind::SectorTier` 元数据，没有当前 production caller。 | `notify.rs:66-67,228-286`；`durable_delivery_runtime.rs:2307-2316` |
| `CapitalVerify` / `INACTIVE` | 与 FactorIC 相同，只是 `DailyReportSubKind::CapitalVerify` 元数据，没有当前 production caller。 | `notify.rs:68-69,228-286`；`durable_delivery_runtime.rs:2307-2316` |
| `WeeklySOP` / `INACTIVE` | enum/cooldown 保留一周计划语义，生产调用图没有 renderer dispatcher 或 scheduler。 | `notify.rs:70-71,421` |
| `StockPick` / `INACTIVE` | `stock_pick` 仍是 CandidateBoard 的输入来源枚举，不是 `PushKind::StockPick` 投递；该 kind 没有 producer。 | `push_templates.rs:3517-3553`；`notify.rs:72-74` |
| `IndustryChain` / `INACTIVE` | R-03 注册为 LegacyAccountGate；当前账户依赖任务统一产生 `AccountMetricsIncomplete`，不会进入 provider/renderer/sink。注意 09:05/15:30 产业链报告走的是独立 NotificationService，不是此 kind。 | `review_batch.rs:408-505,1570-1687`；`push_templates.rs:9546-9818` |
| `AttributionDaily` / `ACTIVE` | 15:05--15:20 计算持仓报价、日/30 日归因，落 DB 并写 Markdown，再普通 governor 推摘要。代码不论 Pushed/Deduped/Denied/SinkError 都标当天完成，故通知失败不重试。 | `main.rs:8988-9122` |
| `G5bAttribution` / `ACTIVE` | 同一 15:05--15:20 窗口；无告警即封日，有告警但无 LLM 不封；最多分析 `DEEP_ATTRIBUTION_MAX_EVENTS` 条，每条先保存深链结果再推。`done` 统计分析/持久化成功而非确认送达，批次尝试完即封日。 | `main.rs:9143-9238` |
| `ReviewMarket` / `INACTIVE` | R-02 在 13-task catalog 中，但 preflight 明确 Disabled，不调用 provider/renderer/sink。 | `review_batch.rs:408-505,1570-1687` |
| `ReviewLhb` / `ACTIVE` | R-04 自动运行需 21:00（手工可绕过）；取 Eastmoney 龙虎榜 top-five 完整批次，verified-empty 终态 NoData，否则 source-only counted delivery。 | `review_batch.rs:1570-1687`；`push_templates.rs:12732-12932` |
| `ReviewSignal` / `INACTIVE` | R-05 已注册但 preflight 明确 Disabled。 | `review_batch.rs:408-505,1570-1687` |
| `ReviewFailure` / `INACTIVE` | R-06 已注册但 preflight 明确 Disabled。 | `review_batch.rs:408-505,1570-1687` |
| `TomorrowWatch` / `ACTIVE` | R-07 21:00 后聚合 A 档未触发、龙虎榜净买 Top5、涨停链前三、可做 T 持仓；用收盘价派生 ±2% 区间和 -5% 止损，去重后仍必须绑定完整 LHB provider batch。 | `review_batch.rs:1570-1687`；`push_templates.rs:8047-8475` |
| `EventCalendar` / `ACTIVE` | R-08 并发取 CNInfo 公告、CFFEX 交割、海外指数、USD/CNY；允许部分组件降级，但把所有可用批次完整绑定后走 source-only counted gateway。 | `push_templates.rs:10983-11210` |
| `ReviewProviderTopN` / `ACTIVE` | R-09 15:35 后运行；并非全市场榜，而是 Eastmoney 单响应中的量比/主力净流入双 TopN。每行严格校验 metric/unit/source ordinal/date/filter/证券身份并保留 declared total/inspected count。 | `review_batch.rs:1570-1687`；`push_templates.rs:6226-6323,6680-6777` |
| `PositionReview` / `ACTIVE` | R-11 要求用户确认账户摘要和 review_date 收盘估值；空仓也可推“无持仓”，有持仓按行业市值 Top5+其他汇总，可选 deep-analyzer 文本不阻塞，最后 counted delivery。 | `push_templates.rs:8797-9012` |
| `ReviewBacktest` / `INACTIVE` | R-12 虽在 scheduler/durable catalog，但常量 `R12_TECHNICAL_BARS_PUBLISHED=false`，在 loader/provider 前直接 Disabled。 | `push_templates.rs:9020-9110` |
| `WatchlistTracking` / `ACTIVE` | R-13 次日读取 A-10 watchlist 并核对行情，确认推送后落 outcomes；outcomes 落库失败只 warning，不反转已经送达的状态。 | `push_templates.rs:9343-9538` |
| `CatalystReview` / `ACTIVE` | A-10 读取可见产业链批，按成员数/连板数/持续性派生 0..100 分和明日观察点，走 counted；成功后保存 T+1 watchlist，保存失败不反转投递。静默期会延期。 | `review_batch.rs:1570-1687`；`push_templates.rs:13551-13755` |
| `PostFixedPriceOrder` / `INACTIVE` | T-14 dispatcher 每 15 分钟的代码入口存在，但全仓没有生产调用 `register_trade_event_source`；每次在 source 边界失败。业务校验要求合法 A 股、正价、整百股、非空 order_id/status。其语义是盘后固定价格，当前却放在盘中分支。 | `main.rs:11052-11094`；`push_templates.rs:4688-4878` |
| `PostFixedPriceFill` / `INACTIVE` | 与 T-14 共用未注册 source，并要求 fill/`next_session_carry`；注释窗口 15:05--15:30，但调用嵌在 Morning/Afternoon 分支，15:00 后已退出，形成 source+schedule 双重不可达。 | `main.rs:10060-10080,11052-11094,11227-11229`；`push_templates.rs:4688-4878` |
| `BlockTradeIntradayConfirm` / `ACTIVE` | 名称称盘中，实际是 19:00 review side route：按自选+持仓查 BlockTradesGateway，300/301/688 只接受协议大宗、实时确认、合法百股数量/正价。 | `push_templates.rs:5127-5180,7684-7735,9700-9721` |
| `BlockTradePriceRange` / `ACTIVE` | 与上一项同一 19:00 side route；仅 8/4/920 开头北交所证券，要求正当日均价和非空价格区间。 | `push_templates.rs:5182-5215,7684-7735,9700-9721` |
| `PaperReview` / `STARVED` | A-01 可在 13:00--13:04 午盘和 19:00 review 执行，且午盘无论 bool 成败都封日；它读取 VirtualWatch snapshots。由于当前唯一 virtual_observation 生产写入来自固定空 `post_close`，新快照输入枯竭，只可能处理历史残留。 | `main.rs:9007-9022,9803-9967`；`push_templates.rs:13980-14220` |
| `IpoListingApproval` / `INACTIVE` | enum/presentation 元数据保留，启动明确打印 `disabled=no_producer`。 | `main.rs:5648-5654`；`notify.rs:174-178` |
| `IpoProspectus` / `INACTIVE` | 与 IpoListingApproval 相同，明确无 producer。 | `main.rs:5648-5654`；`notify.rs:174-178` |
| `IpoCatalyst` / `ACTIVE` | 盘后 review 的额外 side effect：读取/复用当日公告，按关键词分类和静态供应链表映射，必要时查询板块/成分/证券身份；无公告或 provider failure 静默短路。 | `push_templates.rs:7418-7627,9546-9818` |
| `EarningsBeat` / `OPT-IN` | 15:00 后 earnings provider 生成归一化 source event，但默认因缺报告期/预测年度/口径 binding 关闭；仅 `EARNINGS_BEAT_ENABLED=1` 放行，禁用告警每 30 分钟节流。 | `main.rs:7914-7953`；`v17_sources.rs:811-845` |
| `EarningsMiss` / `OPT-IN` | 与 EarningsBeat 共用 provider、校验与默认关闭 gate，仅分类结果为 Miss。 | `main.rs:7914-7953`；`v17_sources.rs:811-857` |
| `AnalystUpgrade` / `ACTIVE` | 15:00 后按配置轮询评级 provider，状态店追踪；归一化事件要求合法 code、当日发布日期、完整批次 evidence，再走 source-fact gate。 | `main.rs:7914-7953`；`src/news/aggregator/source_event.rs:308-383`；`v17_sources.rs:714-787,942-1015` |

13 个 ReviewTask 的自动 scheduler 只在交易日 19:00 后运行；R-04/R-07 还受 21:00 门，R-09 受 15:35 门，可重试失败按 1/5/15 分钟退避。注册任务不等于必然发送。证据：`main.rs:5786-5790,6252-6460`、`review_batch.rs:1190-1240,1507-1687`。

### 24.7 不经过 PushKind 的通知路径与 helper

| 路径 | 当前业务逻辑 | 问题与证据 |
| --- | --- | --- |
| 09:05 盘前产业链报告 | `[09:05,09:15)` 调 `run_chain_analysis_mode(true)`，先分析并保存报告，再遍历 `NotificationService` 渠道 | `Ok(false)`/`Err` 只记 warning，函数仍 `Ok(())`，外层据此封当天，通知失败不重试；`main.rs:9398-9427`、`src/app/modes.rs:106-182` |
| 15:30 盘后产业链报告 | `[15:30,15:35)` 与盘前路径相同；与 `PushKind::IndustryChain` 的 R-03 不是同一 producer | 同样存在“保存成功=调度完成、通知未确认也封日”；`main.rs:8974-9003`、`src/app/modes.rs:106-182` |
| default CLI 单股分析报告 | pipeline 生成报告后调用 `NotificationService::send`；该 service 顺序遍历已配置的 10 类 channel，任一成功即 `Ok(true)`，无渠道或全失败为 `Ok(false)` | caller 对 `Ok(_)` 一律打印“推送成功”，会把 `Ok(false)` 冒充成功；`src/notification/service.rs:124-272`、`src/pipeline/analyze.rs:1246-1257` |
| default CLI 汇总报告 | summary notifier 使用同一 NotificationService | 同样把 `Ok(false)` 记为成功；`src/pipeline/summary_notify.rs:8-65,107-114` |
| `monitor::AlertManager` | 设计注释称 Emergency 全渠道、Important 微信+飞书、Info 飞书/邮件 | 三个 match 分支实际都调用同一个 `send(text)`，且 `push_alert` 当前无生产 caller；`src/monitor/alert.rs:93-180` |

`NotificationService` 可配置企业微信、飞书、Telegram、邮件、Pushover、Custom、Server酱、钉钉、Slack、Discord 共 10 类渠道；`send` 顺序遍历，任一成功即 `Ok(true)`，无渠道或全失败是 `Ok(false)`。带图消息只有邮件真正携带附件，其他渠道降级成文本。证据：`src/notification/config.rs:3-38,51-132`、`src/notification/service.rs:124-374`。

### 24.8 全量对账结论

| 维度 | 数量 | 对账结果 |
| --- | ---: | --- |
| `PushKind` | 65 | 盘前 5 + 集合竞价 7 + 盘中 22 + 盘后 31 = 65；无 missing/extra/duplicate |
| 生产可达性 | 65 | `ACTIVE` 37 + `STARVED` 2 + `OPT-IN` 2 + `INACTIVE` 24 = 65 |
| presentation | 58 tuples / 54 unique kinds | 展示目录不等于 producer 目录；11 个 kind 无 presentation registration |
| durable counted | 23 kinds | 只代表进入 durable authority 的子集，不代表 23 个全部有可达 producer |
| enum 外通知 | 5 条路径/helper | 09:05/15:30 产业链、CLI 单股、CLI 汇总、AlertManager；其中前四条有实际 caller，AlertManager helper 无 caller |

上述数量由 `notify.rs:45-197`、`presentation_registry.rs:42-421`、`durable_delivery/model.rs:169-226` 的定义集合，与 `main.rs`/`p01.rs`/`push_templates.rs`/`v17_sources.rs`/`news_aggregator_init.rs` 的生产调用点做集合差分得到；测试 manifest 不参与生产状态判定，因为 `br196_test_delivery.rs:706-788` 仍固定 63 项并误标 CandidateBoard/IpoCatalyst。

### 24.9 当前事实与问题基线

推送目录的四个集合当前并不相等：

| 集合 | 当前实测 | 架构含义 |
| --- | ---: | --- |
| `PushKind` enum | 65 | 命名空间，不等于生产启用清单；`src/bin/monitor/notify.rs:45-197` |
| production presentation tuples | 58，覆盖 54 个唯一 kind | 允许的 family/kind/producer/renderer 形状，不证明 producer 可达；`presentation_registry.rs:42-421` |
| durable counted catalog | 23 | 进入 production durable owner 的 counted 子集；`src/durable_delivery/model.rs:169-226` |
| 生产调用图 | 37 有当前 producer；2 路径可达但新输入枯竭；2 默认关闭可 opt-in；24 禁用、阻断或无 producer | 当前运行事实；不能由 enum、模板或测试 manifest 单独推断 |

已确认的高风险问题：

| 问题族 | 当前实例 | 影响 | 主要证据 |
| --- | --- | --- | --- |
| 投递未确认却推进完成 | AttributionDaily、G5bAttribution、PaperReview 午间、15:05 快照提醒、09:05/15:30 产业链报告 | 漏重试、日志假成功、用户实际未收到 | `main.rs:8988-9238`、`:9241-9294`、`app/modes.rs:164-182` |
| 发送前推进通知状态 | LimitBoards、CandidateBoard | sink/token/renderer 失败后仍失去重试或改变差分基线 | `main.rs:10306-10416`、`push_templates.rs:7752-7865` |
| 调度窗口与代码位置矛盾 | CandidateTriggered、09:10 行情预检、PostFixedPriceFill | 代码存在但运行窗口没有交集 | `main.rs:9442-9688`、`:10060-10080`、`:11052-11229` |
| source capability 未接通 | PostFixedPriceOrder、PostFixedPriceFill | 周期入口重复失败；没有真实生产输入 | `push_templates.rs:4688-4878` |
| 新输入链断裂 | VirtualWatch → PaperReview | `post_close` 固定为空，后续只能消费历史残留记录 | `main.rs:9803-9967`、`:10103-10238` |
| 结果语义分叉 | 普通 periodic 把 Deduped 视为 confirmed；ReviewTask 把 Deduped 转为不可重试 Failed | 指标、终态原因和恢复行为不一致 | `push_templates.rs:14617-14659`、`review_batch.rs:829-940` |
| 旧通知路径返回值歧义 | pipeline 对 NotificationService 的 `Ok(false)` 记录成功 | 无渠道或全渠道失败被冒充成功 | `pipeline/analyze.rs:1246-1257`、`pipeline/summary_notify.rs:107-114` |
| catalog 漂移 | BR-196 只登记 63 kind，漏 PaperSell/SnapshotStale，并把 CandidateBoard/IpoCatalyst 错列为 disabled | 测试清单不能证明 runtime 状态 | `br196_test_delivery.rs:706-788` |
| 一个 kind 混合多种业务 | IntradayMarket 同时表示盘中板块、盘前数据源预警、盘后快照提醒 | 冷却、预算、订阅和指标互相污染 | `main.rs:9618-9688`、`:10810-10855`、`:9241-9294` |

### 24.10 目标架构约束

专项演进遵守以下约束：

1. **不新增推送微服务。** 保持 Rust 模块化单体；市场数据继续由 `EXTERNAL provider-host` 通过 gRPC 提供，本仓只运行 monitor/CLI/tools。
2. **Data Gateway 仍是 producer 的唯一外部数据 seam。** monitor 生产构建不得重新直接链接 provider SDK。
3. **DurableDeliveryCoordinator 仍是 monitor 生产投递的唯一 authority。** 不在其旁边新建第四套 counted 状态机。
4. **EventBus/JSONL 只做 observation。** bus publish、subscriber 接收或 JSONL append 均不能 acknowledge delivery。
5. **业务 DB 与 durable DB 保持物理隔离。** 通过稳定 decision id、幂等 finalizer 和 reconciliation 解决跨库完成，不制造假原子事务。
6. **monitor `main.rs` 保持 composition root。** 可复用规则放 library，具体 producer/sink/runtime 绑定留在 bin-local adapter。
7. **兼容路径按 strangler 迁移。** 不一次性重写 65 项；先固化 seam，再逐项切换 production wiring。

### 24.11 目标运行链

```mermaid
flowchart LR
    providers["EXTERNAL providers"] --> host["CURRENT provider host"]
    host -->|typed gRPC| gateway["CURRENT Data Gateway"]

    subgraph Monitor["monitor control plane"]
        catalog["PROPOSED Push Capability Catalog"]
        scheduler["PROPOSED PhaseScheduler"]
        producer["Push Producer\nstructured business facts"]
        l3["PROPOSED L3 Presentation\nstable template + exact bytes"]
        runtime["PROPOSED Push Application Runtime\noccurrence + completion policy"]
        durable["CURRENT DurableDeliveryCoordinator"]
        finalizer["PROPOSED Business Finalizer"]

        catalog --> scheduler
        scheduler --> producer
        gateway --> producer
        producer --> l3 --> runtime --> durable
        durable --> finalizer
    end

    durable --> sink["CURRENT AuthoritativeSinkPort\nAccepted / Rejected / Uncertain"]
    durable --> audit["CURRENT immutable audit"]
    durable --> observation["CURRENT EventBus / JSONL\nobservation only"]
    finalizer --> businessDb[("CURRENT business DB\nnotification cursors")]
    durable --> durableDb[("CURRENT durable SQLite")]
```

该运行链在现有 coordinator 前增加 application-level seam，不修改 coordinator 的 14-state authority。`DeliveryResult` 是应用层对 durable terminal/receipt 的规范化投影，不是新的 `DecisionState`。

### 24.12 模块落位与依赖方向

| 目标模块 | 建议位置 | 职责 | 禁止依赖/行为 |
| --- | --- | --- | --- |
| Push domain contract | `src/monitor/push_job.rs`（`PROPOSED`） | `RunContext`、`JobDecision`、`PreparedPush`、稳定 occurrence/subject | 不依赖飞书、HTTP、SQLite、全局 timer、presentation token |
| Push capability catalog | `src/monitor/push_catalog.rs`（`PROPOSED`） | kind、owner、phase、schedule、source、presentation、delivery class、activation、completion policy | 不执行网络/DB/sink；不与 BR-196 再维护第二份手写清单 |
| L3 presentation | `src/push_l3/`（`PROPOSED`） | 结构化 payload → stable template id、exact rendered bytes、SHA-256 | 不取行情、不推进业务状态、不从渲染文本反向解析事实 |
| Push application runtime | `src/bin/monitor/push_runtime.rs`（`PROPOSED`） | 绑定 producer、catalog、clock/calendar、presentation、coordinator 与 finalizer | 不包含持仓/候选/新闻分类规则；不直接实现 sink |
| PhaseScheduler | `src/bin/monitor/phase_scheduler.rs`（`PROPOSED`） | 交易日/phase/due/retry 调度并受 supervisor 观察 | 不使用 detached task；不自行解释物理 sink 结果 |
| Durable authority | `src/durable_delivery/**`（`CURRENT`） | reserve、attempt、fence、typed sink result、audit、terminal state、reconcile | 不拥有股票筛选、盈亏规则或报告内容 |
| Production adapters | `src/bin/monitor/**`（`CURRENT/COMPAT`） | Data Gateway、AuthoritativeSinkPort、immutable append、business finalizer 的具体绑定 | 不绕过 runtime/coordinator 直接发送 counted 消息 |

依赖目标保持：

```text
monitor entrypoint
    → push application runtime / scheduler
        → monitor domain contracts and producer rules
            → Data Gateway ports
        → presentation
        → durable coordinator ports
            → SQLite / sink / audit adapters
```

### 24.13 统一的应用层结果合同

producer 不返回含义不明的 `bool`，而返回：

```rust
pub enum JobDecision {
    Ready(PreparedPush),
    NoData,
    Disabled { reason: ReasonCode },
    RetryableFailure { reason: ReasonCode },
    PermanentFailure { reason: ReasonCode },
}
```

application runtime 将 durable/sink 结果规范化为：

```rust
pub enum DeliveryResult {
    TransportAccepted { receipt: DeliveryReceipt },
    AlreadyDelivered { decision_id: DecisionId, authority: AcceptanceAuthority },
    ManualConfirmedAccepted { resolution_id: ResolutionId, evidence_sha256: String },
    Rejected { retryable: bool, reason: ReasonCode },
    Uncertain { decision_id: DecisionId, attempt_id: AttemptId },
    NotDelivered { resolution_id: ResolutionId, reason: ReasonCode },
}
```

语义要求：

- `TransportAccepted` 只能由经过验证的 typed remote receipt 产生；HTTP 2xx、业务 `code=0`、CLI exit 0、日志或报告保存都不够。
- `AlreadyDelivered` 表示同一 occurrence 历史上已经由 transport 或人工证据权威确认；允许完成本 occurrence，但指标不得记为本次远端发送，且必须保留原 authority 类型。
- `ManualConfirmedAccepted` 是操作人基于受审查证据作出的独立事实，不得伪造 `DeliveryReceipt`，指标必须与 `TransportAccepted` 分开。
- `Rejected { retryable: true }` 保留由 durable reservation/fence 约束的重试资格。
- `Uncertain` 必须进入 inspect/reconcile/manual review；所有 severity 都禁止盲重发，Emergency 只提高核查频率，只有远端证明未接受或 transport 支持幂等键时才允许重试。
- `NotDelivered` 终止本 occurrence，但不推进“已通知”游标；后续独立 occurrence 仍可按 policy 运行。
- `NoData`、`Disabled`、业务计算完成、报告文件保存完成均不能冒充 delivery success。
- 所有 daily flag、timer 和 notification cursor 必须声明自己的 `CompletionPolicy`；caller 不再自由解释 `PushOutcome`。

### 24.14 跨业务 DB 与 durable DB 的完成协议

业务事实与投递事实分别归属：

| 存储 | 应拥有 | 不应拥有 |
| --- | --- | --- |
| business DB | 分析结果、候选快照、归因、虚拟观察、`latest_evaluated_snapshot`、`last_notified_snapshot`、业务 notification cursor | sink attempt、fence token、remote receipt、durable terminal authority |
| durable DB | decision、occurrence、reservation、attempt、receipt、audit pending/terminal、manual resolution | 股票筛选结果、盈亏规则、LLM 分析正文的业务 owner 状态 |

推荐协议：

1. producer 生成并幂等保存业务结果，绑定稳定 `decision_id`；business DB 同事务写 additive `push_notification_intents`，只保存业务键、finalizer 类型、目标游标、expected version 与 Prepared/Finalized 状态，不复制 sink attempt/receipt。
2. runtime 将 exact rendered bytes 与 evidence 交给 `AuthoritativeDeliveryPort`。generic `DurableDeliveryCoordinator` 是默认实现；P01/N02 通过 conformance 验证的 `DedicatedAuthoritative` adapter 保留，不重写其高保证状态机，也不允许新增第三种 authority。
3. authority 到达 `TransportAccepted`、`ManualConfirmedAccepted` 或发现 `AlreadyDelivered`。
4. business finalizer 以 `decision_id` 在 business DB 事务内幂等推进领域通知游标，并将 intent 标为 Finalized。
5. 如果步骤 4 前崩溃，startup/background reconciliation 查找“authority 已确认、business intent 未 finalize”并补齐；正常目标不超过两个 reconcile interval，硬上限 5 分钟。
6. rollback 只能关闭新 producer/scheduler；已经存在的 authority、finalizer、reconciler 与旧路径 fence 必须继续到 Prepared/Uncertain 全部结清，禁止删除 intent 或 durable decision 后让旧路径重发。

具体修复映射：

- CandidateBoard 同时保留 `latest_evaluated_snapshot` 与 `last_notified_snapshot`；失效差分只对后者计算。
- AttributionDaily 分离“归因计算完成”和“归因通知完成”。
- G5b 分离 analyzed/persisted/delivery_attempted/delivered/failed 计数。
- LimitBoards 只有 finalizer 确认后才推进通知 occurrence；不得在 sink 前写 `board_notified`。

### 24.15 PhaseScheduler 与启动 readiness

`PhaseScheduler` 作为受 `supervise_long_running_lifecycle` 观察的长期任务，统一使用 `calendar::MarketSession` 和交易日历，不依赖散落的 `Instant`、`static Mutex<Option<NaiveDate>>` 或注释时间窗。

所有时间驱动的 `ACTIVE`、`STARVED`、`OPT-IN` producer 都必须登记 PhaseScheduler；事件驱动 producer 可保留在受监督 event loop，但必须登记触发类型和 readiness。`INACTIVE` 只登记元数据和禁用原因，不创建 scheduler，也不得因 catalog 存在而被激活。

启动顺序扩展为：

```text
lease
→ activation
→ namespace
→ delivery mode
→ audit/artifact preflight
→ durable startup reconciliation
→ push capability readiness
→ activate ready producers and schedulers
```

每个启用定义必须验证：

- schedule 与允许的 `MarketSession` 至少有一个交集；
- producer 已绑定且 owner 唯一；
- required Data Gateway/gRPC capability 已实现；
- presentation tuple/stable template 已注册；
- counted kind 已进入 durable policy catalog；
- production sink 满足对应 receipt 强度；
- activation/feature gate 状态明确；
- completion policy 已声明；
- source 未注册时 scheduler 为零副作用 Disabled，而不是周期性失败。

readiness 分为两级：

- `CoreUnready`：namespace、durable DB、audit、typed sink 等共享 authority 前提失效，monitor 不得进入生产。
- `ProducerUnready`：单个 source/schedule/presentation/policy 缺失，只隔离对应 producer；进程可保持存活，但部署门禁失败并持续产生 operational alert。

activation 由单一、带 schema version 的 manifest 管理，每个 MigrationUnit 只能为 `Disabled`、`Shadow`、`Active`、`Draining`；启动时校验并记录完整 manifest hash。当前配置没有热加载，因此一次 promotion 通过受控重启生效，且只允许改变一个 physical owner。

readiness 至少输出：

```text
push_total
ready
conditional
compat
inactive
schedule_unreachable
producer_missing
source_missing
presentation_missing
durable_policy_missing
```

该门禁应在生产者启动前发现 CandidateTriggered/09:10 probe 的窗口矛盾和 T14/T15 的 source/时段问题。

### 24.16 三套通知抽象的兼容迁移

| 当前路径 | 迁移决策 |
| --- | --- |
| monitor `notify + durable_delivery_runtime` | 通过统一 `AuthoritativeDeliveryPort` 保持 production authority；generic coordinator 是默认实现 |
| push L1/L2/L4/L5/L6/L7 | 保留治理、sink、analytics 能力；L4 compatibility dedup 逐项被 durable occurrence 替代；新增 L3 presentation contract |
| `notification::*` | 继续作为 default CLI report-oriented `COMPAT/BestEffort` 路径；返回逐渠道 typed result，但弱回执不得推进 authoritative 通知游标，也不直接竞争 monitor production durable lease |

具体规则：

- 09:05/15:30 产业链定时报告属于 monitor 生产调度，应迁入 PhaseScheduler + durable authority。
- default CLI 单股/汇总报告短期保留 NotificationService，但返回 `BestEffortAccepted`、`PartiallyAccepted`、`NoChannelConfigured`、`AllChannelsFailed` 等明确结果；这些名称不能投影成 `TransportAccepted`。
- 若未来要求 CLI 报告也进入权威投递，应通过 monitor command/inbox seam；禁止 CLI 直接打开 production durable SQLite。
- P01、N02 已有较强 claim/receipt/recovery 合同，先作为 `DedicatedAuthoritative` adapter 进入 catalog/conformance，不重写状态机。

### 24.17 推送体验与语义去重

架构正确性稳定后，再进行内容层优化：

1. 拆分混合 kind：`IntradayMarket` 至少拆为盘前数据源预警、盘中板块资金、持仓快照新鲜度三种 occurrence/订阅语义。
2. 建立跨 kind `topic_key`（股票、板块、事件簇），避免 Announcement、NewsCatalyst、NewsToIdea、LimitBoards、IndustryChainIntraday 等短时间重复表达同一事实。
3. severity 决定即时/摘要和渠道：Emergency 即时多渠道、Important 即时主渠道、Info 进入周期摘要、Research 进入盘后报告。
4. 用户订阅至少支持竞价、持仓、新闻、板块、盘后复盘、研究报告、仅持仓相关、仅 Important/Emergency。
5. `AlertManager` 的级别路由必须真实选择 adapter，不能三个分支调用同一个 `send`。

### 24.18 增量实施顺序与验收

本节是 2026-09-02 Q1--Q55 Grill 决策的正式结果，覆盖并废止此前“C0--C6 横向串行、10--15 个有效开发日”的执行基线。C0--C6 只保留为能力标签；实际拓扑改为 **Foundation → 纵向 MigrationUnit → Final cleanup**。开发由 Codex 单独承担；每次改变 physical owner 的 production promotion 必须由用户或指定操作人在线监督，等待交易窗口、人工裁定、外部 source 与生产观察不计入有效开发日。

**确认后的范围：**

| 集合 | 数量 | 本轮处理 |
| --- | ---: | --- |
| `ACTIVE` | 37 | 全部进入 catalog、completion policy、readiness 与 trigger 对账；不合规 wiring 逐 MigrationUnit 迁移 |
| `STARVED` | 2 | 保持 Starved；修合同/完成语义/readiness，不恢复输入链或暗中激活 |
| `OPT-IN` | 2 | 保持 OptInDisabled；只有显式产品决策后才允许生产灰度 |
| `INACTIVE` | 24 | 只登记禁用原因和能力缺口，不创建 scheduler/producer，不因 enum/catalog 存在而启用 |
| enum 外真实通知 | 4 | 09:05/15:30 产业链进入 monitor authority；CLI 单股/汇总保留 typed BestEffort compatibility |
| enum 外无 caller helper | 1 | AlertManager 只登记 no-producer；修 severity adapter 不冒充生产迁移 |

**Foundation Release（零行为变化）：**

| 产物 | 必须具备 | 零行为证明 |
| --- | --- | --- |
| Domain contracts | `RunContext`、`PreparedFacts`、`JobDecision`、`DeliveryResult`、`CompletionPolicy` | 未接 caller 时无 provider/DB/sink 调用 |
| Business intent | additive `push_notification_intents` 与幂等 finalizer/reconciler | migration 只增表/索引；旧 binary 可忽略；无自动 drop |
| Machine catalog | 每个 MigrationUnit 的 ID、producer、occurrence family、completion owner、当前/目标 authority、trigger、依赖、门禁 | 与 65 enum、41 非 INACTIVE、presentation、durable、source、schedule exact-match |
| Authority seam | `AuthoritativeDeliveryPort`、generic coordinator adapter、P01/N02 DedicatedAuthoritative adapter | 不改 P01/N02 14-state/专用高保证状态机 |
| PhaseScheduler | clock/calendar 注入、supervisor、timer registration | manifest 为 Disabled 时 schedulers=0 |
| Activation manifest | schema version、Unit 的 Disabled/Shadow/Active/Draining、manifest hash audit | 默认状态不改变任何 physical owner |
| Shadow harness | 单次采集 `PreparedFacts`，old/new 共用输入并输出结构 diff | shadow 禁止 sink 与业务 finalize |
| Operator/observability | inspect/resolve CLI、readiness、结构化日志、reconciler 指标 | 不依赖业务 push sink 才能发现其故障 |

Foundation 必须先生成机器可核验 catalog 后才能冻结 MigrationUnit 数量、`requires_physical_promotion` 集和正式工期；下面的 35 只是当前审计下界，不是伪装成事实的最终清单。

**首版 MigrationUnit 下界：**

| 类别 | 暂定数量 | 代表范围 |
| --- | ---: | --- |
| P0/错误完成 owner | 10 | CLI report、盘前/盘后产业链、Attribution、G5b、15:05 快照、PaperReview、Candidate lifecycle、LimitBoards、ReviewTask result semantics |
| 弱 authority/调度/语义 | 18 | Account/Data mode、SnapshotStale、Auction、Announcement、PaperSell、IntradayMarket、新闻映射、产业链、ST、MarketAction、BlockTrade、IPO、评级、VirtualWatch、Earnings |
| 已有强 authority conformance | 7 | P01、PaperTrade、SectorTop/Anomaly、HoldingPlan、T0Advice、CloseCall、NewsFlashAggregated |
| 合计下界 | 35 | Foundation 后逐 Unit 纵向实现；正式数量由 exact catalog 决定 |

MigrationUnit 以 `(producer, occurrence family, completion owner)` 为边界，不以 PushKind 名称或源文件为边界。以下共享 owner 必须原子处理：CandidateBoard + CandidateInvalidated、SectorTop + SectorAnomaly、7 个 ACTIVE ReviewTask 的 result mapping、两类 BlockTrade、EarningsBeat + EarningsMiss。一个 `IntradayMarket` 的盘前预警、盘中板块和 15:05 快照则必须拆成不同 occurrence/subscription 语义。

**第一批风险顺序：**

| 顺序 | MigrationUnit | 先做原因 |
| ---: | --- | --- |
| 1 | CLI report typed BestEffort result | 无持久 notification cursor，用于先验证外部兼容结果合同 |
| 2 | 09:05 产业链 | 当前吞通知 false/error 后封日 |
| 3 | 15:30 产业链 | 与盘前同 bug，但 occurrence/completion owner 独立 |
| 4 | AttributionDaily | 未确认投递也推进日完成 |
| 5 | G5bAttribution | 分析成功数冒充送达数并封日 |
| 6 | 15:05 snapshot occurrence | 混用 IntradayMarket 且失败也封日 |
| 7 | CandidateBoard + CandidateInvalidated | 发送前推进候选 diff 快照 |
| 8 | LimitBoards | 发送前写 `board_notified` |
| 9 | ReviewTask result semantics | Deduped 与普通 periodic 解释相反 |
| 10 | PaperReview-Starved conformance | 修历史残留的完成语义，但不恢复输入或启用新推送 |

**每个 Unit 的纵向交付内容：**

1. 从同一 `PreparedFacts` 构造 old/new decision；provider、LLM 和业务取数只执行一次。
2. shadow 只能 prepare/project；买入、候选快照、通知游标、watchlist/outcome 等副作用只能由当前 physical owner/finalizer 执行一次。
3. audience、occurrence、business key、severity、evidence、suppression、completion policy、template version、rendered bytes 必须 exact-match；只排除 attempt ID、运行延迟、日志时间等纯遥测。业务日期/phase 来自同一 `RunContext`，不得忽略。
4. 基础设施迁移不得夹带模板改文案；模板变化另建 versioned Unit。同一 `decision_id` 永远重放首次保存的 exact bytes。
5. artifact 可包含多个 Disabled/Shadow Unit，但每次 production promotion 只允许一个 Unit 取得 physical ownership。
6. Release N 接管 Unit N；下一次发布以独立 commit 清理 Unit N 的旧路径并可接管 Unit N+1；最后保留一次 tail cleanup。

**authority 与 receipt 强度：**

| 路径 | authority 分类 | 可产生的结果 | 是否可推进 authoritative 游标 |
| --- | --- | --- | --- |
| generic DurableDeliveryCoordinator + typed CLI receipt | GenericAuthoritative | TransportAccepted、Rejected、Uncertain、AlreadyDelivered | 是 |
| P01/N02 现有高保证状态机 | DedicatedAuthoritative | 通过 adapter 投影同一应用结果合同 | conformance 通过后是 |
| 飞书 HTTP/Webhook、邮件、NotificationService 多渠道 bool | COMPAT/BestEffort | BestEffortAccepted、PartiallyAccepted、NoChannelConfigured、AllChannelsFailed | 否 |
| 人工裁定 | Manual authority | ManualConfirmedAccepted 或 NotDelivered | 只按显式 CompletionPolicy；不得伪造 transport receipt |
| EventBus/JSONL/report save/analysis complete | Observation/business fact | 非 delivery 结果 | 否 |

所有 severity 都禁止 blind resend。Emergency 只提高 inspect/人工升级频率；只有远端证明未接受、transport 提供幂等键，或 `Rejected.retryable=true` 且 fence 仍有效时才允许重试。

**Uncertain 与 finalizer SLA：**

| 类别 | 告警/处置 | promotion 门禁 |
| --- | --- | --- |
| Emergency Uncertain | 1 分钟内 operational alert，15 分钟内人工裁定 | 未裁定立即阻止新 promotion |
| Important Uncertain | 5 分钟内 operational alert，4 小时内裁定 | 未裁定阻止新 promotion |
| Info/Research Uncertain | 下一 eligible session 前 ManualConfirmed 或 NotDelivered | 未裁定阻止新 promotion；不得自动改 Accepted |
| AcceptedPending finalizer | 目标不超过两个 reconcile interval，硬上限 5 分钟 | 上一 Unit 不得存在超龄 intent 或 business/durable mismatch |

**shadow、promotion 与观察门禁：**

| 门禁 | 要求 |
| --- | --- |
| shadow 并行度 | 多个 Unit 可同时 shadow；任何时刻每个 occurrence 只有一个 physical owner |
| promotion 频率 | 每个交易日最多 promotion 一个改变 physical owner 的 Unit；开发、shadow、无 owner 变化的部署可继续 |
| 高频定时 Unit | 至少覆盖一个完整 eligible session，且条件允许时不少于 3 个 occurrence |
| 每日/低频 Unit | 确定性 replay + 一次自然 occurrence 或明确授权的专用灰度 |
| Emergency/业务副作用 Unit | 至少观察两个 eligible session |
| transport 灰度 | 每个实际启用的 authoritative CLI channel 各一次真实 TransportAccepted；同 decision 重放必须 AlreadyDelivered 且无第二条消息 |
| 停线条件 | 任一 semantic diff、重复发送、未解释 cursor 前进、超龄 backlog、未裁定 Uncertain、CoreUnready 或跨库不一致 |

rollback 不是恢复旧 binary 后重新发送。新 producer/scheduler 可关闭，但 authority、finalizer、reconciler 和旧路径 fence 必须继续；`Draining` 不创建新 occurrence，只结清既有 Prepared/Uncertain。若新路径已经 Accepted，外部事实不可撤销。

**人工裁定与控制面：**

- promotion 只在用户或指定生产操作人在线的窗口执行；Codex 负责开发、诊断和当次操作，不能被当作无人值守 15 分钟 on-call。
- CLI 必须先 `inspect decision_id`，再使用当前 fence 执行 `resolve`；operator 来自已认证本机/服务身份与 production allowlist，禁止自由文本伪造身份和直接 SQLite 修改。
- resolution 必须带 reason/evidence；audit 保存最小元数据、受保护 evidence URI 与 SHA-256，不保存 token、cookie、Webhook URL 或不必要的持仓/正文。
- operational alert 至少进入结构化本地日志、进程 health/readiness 和可查询 CLI；可选独立 pager 不能被当作业务 receipt，也不能只依赖正在故障的业务 sink。

**数据库、保留与故障演练：**

| 项目 | 正式要求 |
| --- | --- |
| migration | 只做 additive table/index；生产前分别备份 business/durable DB、分别 `integrity_check` 并在 Test namespace 演练恢复；不得宣称两库原子快照 |
| retention | migration 与 Production Verified 后至少 90 天不自动删除 terminal intent、receipt、manual resolution、immutable audit；nonterminal 永不自动清理；后续 retention 另立 ADR |
| failure matrix | sink 前 definitive rejection、sink 开始后 Uncertain、Accepted 后 finalizer 前崩溃、finalizer 幂等重试、AlreadyDelivered 防二次发送、pending intent binary rollback、manual resolution 合法性 |
| production fault policy | 生产只做正常 typed receipt 灰度与防重复重放；不主动断网/杀库制造故障 |
| test isolation | 默认并行 monitor 测试不得保留无法解释的失败；进程级环境测试必须隔离 subprocess/namespace 或进入 CI 强制的明确 serial-only suite |

**工期重新基线：**

此前 W01--W21 已有 98--142 小时，但没有计入约 35 个纵向 Unit 各自的 shadow、activation、diff、cutover、cleanup 与证据门禁。按每 Unit 额外 4--8 小时，增加 140--280 小时；合计原始 238--422 小时，再加 20%--30% 故障定位/回归缓冲为 286--549 小时，即 **36--69 个 8 小时等效开发日**。source 合同扩张或 exact catalog 继续拆分 Unit 时进入更高风险区间，不能通过删除 C6 类验收追回日期。

| 计划维度 | 暂定基线 | 说明 |
| --- | --- | --- |
| 工程投入 | 36--69 个 8 小时等效开发日 | Codex 单独开发；Foundation exact catalog 后冻结正式数值 |
| production rollout | 7--10 个交易周 | 每交易日最多一个 physical promotion；高风险 Unit 需要两个 eligible session |
| Program Production Verified | 约 10--16 周日历跨度 | 开发与 rollout 可重叠；交易窗口、人工审批和外部等待影响日历但不计有效开发日 |
| 原 10--15 日 | 已废止 | 既不满足原 WBS 数学，也不包含逐 Unit 门禁与生产观察 |

**四级里程碑：**

| 里程碑 | 定义 | 不能冒充的状态 |
| --- | --- | --- |
| Foundation Ready | 合同、intent、catalog、reconciler、shadow、manifest、operator control 完成，且零 physical owner 变化 | 不是业务修复完成 |
| P0 Production Verified | 首批 10 个高风险 Unit 完成 promotion、观察与上一单元 cleanup | 不是全量架构迁移完成 |
| Architecture Release Candidate | 全部 Unit 的代码、shadow、测试完成，但仍可能有 Unit 未做生产观察 | 不是 Production Verified |
| Program Production Verified | required Unit 全部 promotion/观察/cleanup，真实渠道与故障门禁全部结束 | 唯一可称专项完成的状态 |

**最终退出标准：**

- 65 PushKind、41 个非 INACTIVE 状态、24 个 INACTIVE 原因和 enum 外路径 exact reconciliation；无 missing/extra/duplicate。
- 全部 required MigrationUnit 完成 shadow、promotion、观察和 tail cleanup；STARVED/OPT-IN/INACTIVE 未被意外激活。
- 零未裁定 Uncertain、零超龄 finalizer backlog、零 business/durable mismatch、零已知重复物理发送。
- 每个启用 authoritative transport 完成真实 TransportAccepted + AlreadyDelivered 灰度；ManualConfirmed、NotDelivered 与 transport 指标完全分栏。
- rollback/fault matrix 全通过；default parallel、明确 serial-only suite、integration、lint/check 均有 fresh receipts，不用偶发复跑通过掩盖 flake。
- 老 physical path 已删除，或只剩有 owner、到期日和禁止竞争 production lease 的明确 COMPAT 例外。
- 蓝图、运行手册、machine catalog、migration log、activation manifest schema、人工处置说明和 HTML/SHA 同步。

### 24.19 专项禁止项

- 不新建推送微服务、Kafka/Redis durable queue 或第二个权威投递数据库。
- 不把 EventBus/JSONL、HTTP 2xx、报告文件保存或分析完成当成 delivery receipt。
- 不把 Webhook/邮件/NotificationService 的弱成功结果命名或投影成 `TransportAccepted`，也不据此推进 authoritative cursor。
- 不把 `ManualConfirmedAccepted` 伪造成 transport receipt；操作人不得直接修改 production SQLite 绕过 inspect/resolve、fence 和 immutable audit。
- 不因 Emergency 等级绕过 `Uncertain` 的核查流程盲目重发；severity 只改变告警与裁定 SLA。
- 不合并 business DB 与 durable DB 来掩盖跨 owner 完成问题。
- 不宣称 business/durable 两库具备原子快照或原子提交；用 intent + finalizer + reconciliation 显式处理边界。
- 不允许 default CLI 直接竞争 monitor 的 production durable lease。
- 不从渲染文本反向提取股票、候选或 occurrence identity。
- 不新增第二份 PushKind/producer/status 手工清单；所有清单必须由唯一 catalog 派生或自动对账。
- 不在 `main.rs` 继续新增独立 static timer/daily flag；新周期任务必须进入 PhaseScheduler 或有明确例外 ADR。
- 不为 `INACTIVE` 创建 scheduler，不因合同修复激活 `STARVED`，不绕过显式产品决策启用 `OPT-IN`。
- 不允许 shadow 二次调用 provider/LLM 或执行买入、游标、候选快照、outcome 等业务副作用；old/new 必须共享一份不可变 `PreparedFacts`。
- 不在同一交易日 promotion 两个改变 physical owner 的 Unit，也不在仍有未裁定 `Uncertain`、超龄 intent 或 semantic diff 时继续 promotion。
- 不通过 rollback 删除 `push_notification_intents`、durable decision、receipt 或 fence；`Draining` 必须先结清已存在的 occurrence。
- 不用“复跑后偶尔通过”接受默认并行 monitor 测试中的未知失败；必须隔离 subprocess/namespace 或标为 CI 强制 serial-only suite。
- 不先重写 P01/N02/durable coordinator 等高保证状态机；先迁移错误封口和无权威 receipt 的旧路径。

## 25. v18.x / v19.x 设计覆盖审计与落位

结论先行：此前蓝图记录了不少被 v18/v19 触及的现有能力，但**没有把两套版本设计逐项纳入，也没有区分“设计稿存在”“被当前代码部分吸收”“已按原合同实现”**。本节补齐这一缺口。核验基线为 `2026-09-02`、代码 `a673043`；状态只能由当前代码、测试和生产证据决定，不能由目录名、文档标题或历史 commit 说明自行升级。

### 25.1 状态词与裁决优先级

| 状态 | 本节含义 |
| --- | --- |
| `CURRENT` | 当前仓库存在对应生产实现和可定位证据；仍不自动等于生产已验证 |
| `PARTIAL` | 目标能力有局部实现，但公开合同、状态机、owner 或验收与设计稿不一致 |
| `PROPOSED` | 可继续作为候选目标；尚未实现，不能画进 CURRENT 运行图 |
| `SUPERSEDED` | 历史事实/方案已被更新的代码审计或上位设计替代；保留用于追溯 |
| `CONFLICT` | 与上位安全边界、当前架构或同文档其他要求冲突；实施前必须先 ADR 裁决 |

冲突时按以下顺序裁决：当前源码与 fresh test/production evidence → 本蓝图 CURRENT 章节 → 同版本权威入口 → companion/strategy/draft。对 v18，`v18.0 量化平台闭环中文整合设计` 高于 v18.1 战略建议；对推送，本文 §24 的 65-kind 审计高于 v19 的 57/59-kind 历史 catalog。设计文档中的示例 Rust/SQL 只说明意图，不是可编译或已迁移证据。

本次 exact negative probe 在 `src/` 与 `tests/` 中对以下声明均为 **0 个定义命中**：`DataEnvelope`、`AuditJournal`、`DecisionRecord`、`PaperExecution`、`BannerSnapshot`、v19 设计的 `CircuitBreaker`、v19 `ErrorCode`、`SignalTracker`。同时 `src/lib.rs:10-96` 没有 `data_contract`、`paper_ledger`、`banner`、`breaker`、`backtest` 顶层模块；`backtest` 目前只是 `strategy::core` 的兼容 re-export（`src/lib.rs:70-73`）。这组反证必须与下面的“局部能力已存在”一起读，不能误解为系统没有回测、模拟交易、熔断或复盘能力。

### 25.2 v18.x 全文档覆盖矩阵

| 文档 | 自报状态/真实版本 | 本次裁决 | 蓝图落位 |
| --- | --- | --- | --- |
| `docs/v18.x/README.md` | v18 入口；明确“尚未开始实现” | `CURRENT` 文档事实 | 本节作为版本边界；不能用 README 的“新增”推导代码已实现 |
| v18.0 中文整合设计 | Active design；明确尚未实现 | `PROPOSED` 上位目标 | §25.3--25.5 记录四模块、Gate P/L 与实施边界 |
| v18.0 四核心模块 | Active companion；仅设计 | `PROPOSED` | 接口意图并入 §25.3；示例类型不进入 CURRENT inventory |
| v18.0 平台评估 | 2026-07-16 snapshot | `SUPERSEDED/PARTIAL` | 定位与风险仍有效；测试数、文件状态和能力百分比必须重新测量 |
| v18.0 实施路线图 | 6 个顺序 workstream | `PROPOSED` | 保留依赖顺序，但执行前必须按当前模块重新拆 MigrationUnit/ADR |
| v18.1 战略差距分析 | Strategic Research | `CONFLICT/PARTIAL` | “人工决策支持、无已证实 broker 实盘”判断保留；其优先接券商/小资金实盘建议被 Gate L 边界否决 |
| `v18.2-backtest-direction.md` | 文件在 v18.x，标题实际为 **v20.x** Strategic Planning | `PROPOSED/CONFLICT` | 移入未来 backtest discovery；不得和 v18 PaperLedger 双重建模 |
| `v18.3-backtest-implementation.md` | 标题实际为 **v20.x** Implementation Design | `PROPOSED` | 约 1,150 行伪实现只作草案；当前无 `src/backtest/`，且示例存在不可编译片段 |
| `v18.4-factor-zoo-design.md` | 标题实际为 **v20.x** Implementation Design | `PROPOSED` | 55 因子/Polars/5 模式是研究候选，不是当前 factor inventory 或性能事实 |
| `v18.5-production-readiness-design.md` | 标题实际为 **v20.0** Draft | `CONFLICT/PROPOSED` | Redis/Postgres/REST/WebSocket/JWT/Docker/K8s/Web UI 属独立平台扩容提案，当前不并入单机生产拓扑 |

因此，“v18 设计已加进来”的准确含义只能是：**目标、边界、现状差距与未来裁决已进入蓝图；四核心合同和 v20 草案没有被宣称已实现。** 文件归档应在后续文档治理变更中把 v18.2--v18.5 移至 v20.x 或在原处增加醒目的 alias/deprecation header，避免目录名继续制造错误版本语义；本轮为保持历史链接稳定不移动文件。

### 25.3 v18 四核心模块：目标与当前吸收程度

v18 的目标链保持：

```text
Data Contract Gate / DataHealthSnapshot
    → CandidateSnapshot / InvestmentDecisionRecord
        → Paper Order / Fill / Ledger
            → Attribution / ModelChangeProposal
                → Gate P evidence
                    → 独立 Gate L 评审（默认关闭）
```

| v18 目标 | 当前可复用能力（正证） | 仍缺的原设计合同 | 裁决 |
| --- | --- | --- | --- |
| Data Contract Gate / `DataHealthSnapshot` | 回测日线通过统一 Gateway 获取并保存 `BatchEvidence`，明确拒绝伪造 index 身份；因子快照按 `<= as_of` 取值，兼容路径显式标注 look-ahead 风险（`src/pipeline/backtest_runner.rs:56-126`） | 没有通用 `DataEnvelope<T>` 五态、唯一 action health authority、全行动路径 5s/30s freshness gate | `PARTIAL` |
| CandidateSnapshot / `DecisionRecord` | `src/decision` 已有排除、分层、资金验证、持仓计划、盘中监控等业务模块（`src/decision/mod.rs:1-19`）；selection 与 durable push 有各自稳定 ID | 没有保存完整 universe disposition 的不可变 investment decision、统一 `correlation_id`、成本/流动性证据与 risk result 链 | `PARTIAL` |
| Paper Order / Fill / Ledger | paper fill 严格解析事实时间，FIFO 重建持仓并拒绝重复、逆序、未来日期与坏价（`src/trading/paper_lot_ledger.rs:1-150`）；`paper_trade` 与真实 `stock_position` 隔离并带 order audit/idempotency | 没有 v18 `PaperExecution` port、完整 parent order/partial fill/no-fill 状态机、可由事件重放的独立 PaperLedger 和通用 WORM `AuditJournal` | `PARTIAL` |
| Attribution / `ModelChangeProposal` | `performance` 已有 attribution、epoch、replay、report/snapshot（`src/performance/mod.rs:1-10`）；`prediction_tracker` 保存 T+1/T+3/T+5 结果并有候选样本聚合（`src/database/mod.rs:3400-3465,3838-3950`） | 没有沿单一 investment `decision_id` 的全链路归因，也没有 Draft→Reviewed→Shadow→PaperChallenger→Promoted 的治理状态机 | `PARTIAL` |
| Research/model registry | 当前回测支持 point-in-time 因子快照与 walk-forward 基础 | 没有不可变 research run/model promotion record、统一成本/容量证据和独立审批/有效期 | `PARTIAL` |
| `AuditJournal` / Gate P | order audit、attribution epoch、selection 与 durable delivery 各有更窄的链/receipt | 没有四模块共享的远端 WORM/Object-Lock authority、五年 retention probe、daily signed Merkle root 与 restore/read Gate P 证据 | `PARTIAL` |

这里有一个必须现在消除的命名碰撞：§12/§24 的 durable delivery `DecisionId` 表示“一次外部投递决策”，v18 的 `DecisionRecord` 表示“一次投资/组合决策”。未来落地必须使用命名空间或显式类型，例如 `PushDecisionId` 与 `InvestmentDecisionId`；两者只能通过业务引用关联，禁止复用 UUID 字段后让发送成功被解释为投资决策或交易成功。

### 25.4 v18 安全边界与实施顺序

以下边界从设计稿直接晋升为蓝图约束，因为它们与当前事实和安全模型一致：

- 当前系统是研究、分析、监控、通知和 paper simulation 平台，不是自动实盘交易系统；当前代码也明确把 legacy paper engine 从 production loop 隔离，见 §10.1。
- 推送送达、`ManualConfirmedAccepted`、人工持仓调整、paper fill 均不等于券商订单接受或真实成交。
- Gate L 默认关闭；只有 Gate P 在约定模拟观察期内持续通过，且另行完成 broker 状态机、查询对账、kill switch、授权与人工审批设计，才允许提出实盘 adapter 变更。
- v18.1 “优先券商 API/小资金实盘”与上述顺序冲突，按上位设计否决；不能用战略建议绕过 Gate P/L。
- 缺数据、过期数据、冲突数据不能变成零值/空集后参与行动；研究兼容路径必须标为 exploratory，不能晋升模型。
- 复盘可以生成 `ModelChangeProposal`，但不能直接写配置、阈值或模型；生效必须走独立版本、审批、shadow 和 rollback。

若未来授权实现 v18，顺序保持但需要按当前代码重做接口设计：

1. **V18-F0 术语与 ADR 收敛：** 冻结 `PushDecisionId`/`InvestmentDecisionId`、DataHealth authority、通用审计与现有四类窄审计的关系。
2. **V18-F1 数据合同：** 先在一个可行动路径建立 envelope/health vertical slice，不一次性重写全部 provider。
3. **V18-F2 决策记录：** 固化完整候选集合与 risk result；通知只渲染已持久化 view。
4. **V18-F3 paper ledger：** 在不改变真实账户 owner 的前提下，迁移一个策略到 order/fill/event/reconcile 闭环。
5. **V18-F4 归因治理：** 只消费 F2/F3 的不可变 ID；先 report，再 proposal，最后才讨论 challenger。
6. **V18-F5 Gate P：** 完成 retention/restore/replay/point-in-time/成本后证据；Gate L 保持独立项目。

该计划不能并入 §24 推送专项的 36--69 日基线。§24 Foundation 只需为未来 `InvestmentDecisionId`、`DataHealthSnapshotRef` 和 report ref 保留 typed optional reference；不得借推送迁移顺便实现 v18 四模块。

### 25.5 v18.2--v18.5 实际是 v20 提案

| 提案 | 可保留的设计意图 | 当前不能接受的假设 | 下一步门禁 |
| --- | --- | --- | --- |
| 事件驱动回测 | signal→order→fill→position、成本、涨跌停、可追溯 | 新建 `src/backtest` 与 v18 PaperLedger/现有 `strategy::core` 三套 engine owner；用当日 close 推导涨跌停；理想化部分成交 | 先写 Backtest/PaperExecution 边界 ADR 与同一 FillModel conformance |
| 55 因子 + Polars | factor metadata、point-in-time、IC/IR、分层、walk-forward | “55 个够了”“快 100 倍”和固定阈值未经 benchmark/统计验证；缺失值直接填 0 会违反数据红线 | 数据集版本、survivorship/corporate-action、multiple-testing、成本/容量和 benchmark receipt 先就绪 |
| 5 种回测模式/策略 DSL | 研究模式可组合、策略规则结构化 | 直接把示例阈值/权重变成业务规则；重复现有 strategy/selection contract | 每种模式独立 PRD、BR/threshold proof、golden fixture 与 OOS gate |
| Production Readiness & Scale | 统一健康、观测、访问控制、容量规划 | 当前没有已批准的 Web 产品或多副本部署需求；Redis/Postgres/K8s 会改变 owner、lease、DB 与安全拓扑 | 独立产品 PRD + ADR；先证明单机瓶颈，再选择扩容形态 |

这些提案的开发周期在完成 ADR、数据范围和产品选择前没有可信基线。任何引用其“11 周”“~2,500 行”“2026-10-31 GA”或覆盖率百分比的计划，都必须先重新测量当前代码和依赖，不能作为本轮承诺。

### 25.6 v19.x 全文档覆盖矩阵

| 文档 | 自报状态/历史基线 | 本次裁决 | 蓝图落位 |
| --- | --- | --- | --- |
| `docs/v19.x/README.md` | 设计阶段、待评审 | `CURRENT` 文档事实 | 版本边界与 10 个运行痛点进入本节；“v19 已完成”不成立 |
| v19.0 Operational Clarity | 11 PR、5--6 周、评审中 | `PROPOSED/PARTIAL` | §25.7 逐 PR 对账；运行原则部分被现有 supervisor/readiness 吸收 |
| Push Template Catalog | `master@97f28b9`、57 kinds | `SUPERSEDED` | 方法论保留；事实清单由 §24 的 65-kind/41 非 INACTIVE 审计替代 |
| v19.1 复盘增强 | `Design`；JSONL SignalTracker + R10 | `PARTIAL/SUPERSEDED` | 当前改由 SQLite `prediction_tracker` + BR-232 side route 部分吸收；没有 R10 |
| v19.2 AI 改进 | `Design`；历史 IC=-0.0775 | `PARTIAL/PROPOSED` | `LlmRegistry` 已存在，但旧 Gemini/multi-agent 路径仍活跃；IC 必须用当前样本重算 |
| v19.3 全天推送工作流 | BR-223、commit 704de84；57→59 kinds | `SUPERSEDED/PARTIAL` | 历史接线变更可追溯；当前时段、状态、producer、问题与方案一律以 §24 为准 |

### 25.7 v19.0 11 个 PR 的当前代码对账

| PR | 设计目标 | 当前证据 | 状态/缺口 |
| --- | --- | --- | --- |
| PR-1 Quiet/Halted RunMode | 休市停非必要轮询、fatal 可停止 | monitor 有交易 phase、shutdown 和长期任务 supervisor（`src/bin/monitor/main.rs:4212-4255`），但没有设计中的 `RunMode` | `PARTIAL`；PhaseScheduler 可承接 due policy，不能宣称 Quiet Mode 完成 |
| PR-2 BannerSnapshot | 单一结构化 health truth | 当前仍是 `Mutex<Option<BannerCtx>>`，缺失时 caller 记录字符串并跳过（`src/bin/monitor/main.rs:1666-1695`） | `PROPOSED`；不能把现有 BannerCtx 重命名后算完成 |
| PR-3 ErrorCode | typed code/retryable/severity/retry_after | exact type 0 命中；当前大量 reason_code 仍分散 | `PROPOSED`；应与 §24 `ReasonCode`、readiness reason schema 合并设计，避免第二套错误 taxonomy |
| PR-4 Log rotation | 日切、50MB、30 日、warn 分流 | 无 `src/log/rotate.rs`/tracing rotation 实现 | `PROPOSED`；保留期与 secrets/redaction policy 要先定 |
| PR-5 `--health` | 人读/JSON health | 已实现早期只读health runtime_snapshot v2；本任务未实测1s/live acceptance | `CURRENT_SOURCE`；STAGED watchdog消费同一schema，无provider/sink |
| PR-6 25+ metrics | operational metrics catalog | 当前 `MonitorMetrics` 明确只有 6 项（`src/bin/monitor/metrics.rs:1-67`） | `PARTIAL`；标签基数、单位和 authoritative/BestEffort 结果须与 §24 对齐 |
| PR-7 per-source breaker | Closed/Open/HalfOpen + recovery | `BackoffState` 已有失败升级、CircuitBreak 和 half-open 检查（`src/monitor/rate_budget.rs:94-230`） | `PARTIAL`；尚未证明每个真实 source 都统一接线或进入 banner；原设计 threshold=5/10 自相矛盾，须先冻结 |
| PR-8 recovery fields | 每源 last successful pull/失败时长 | 当前 BannerCtx 没有统一 per-source map | `PROPOSED`；source identity 应复用 Gateway evidence catalog |
| PR-9 多层健康通知 | heartbeat、可选 HTTP health、可选 webhook | 当前有 data-mode immutable heartbeat 与远端 API health，但不是设计中的独立三层 monitor health control plane | `PARTIAL`；§24 已要求 operational alert 不依赖业务 sink，可作为上位合同 |
| PR-10 test isolation | test banner、强制 dry-run、物理路径隔离 | durable delivery/test side route 已有 namespace 与零副作用测试门禁，BR-232 post-session test 明确阻断 provider/persistence/sink（`src/bin/monitor/push_templates.rs:9687-9699`） | `PARTIAL`；仍需全 monitor 资源矩阵证明，不以一个路径外推全局完成 |
| PR-11 failure tests | 每个 typed error 至少一个反向测试 | 已有大量局部故障测试，但目标 ErrorCode 尚不存在 | `PARTIAL`；待统一 error contract 后生成 exact matrix |

v19.0 文内还有一个必须先解的规范冲突：PR-7 表写“5 次失败后 Open”，风险段又写默认 `failure_threshold=10`。实现前应把阈值、cooldown、按 source/operation 粒度和恢复成功数写入 ADR/配置证据；不能任选一个值编码。

### 25.8 v19.1/v19.2/v19.3 的具体裁决

**v19.1 复盘增强：** 原设计的 `src/review/signal_tracker.rs`、`data/signal_tracker/YYYY-MM-DD.jsonl` 与 `ReviewTask::R10` 没有按原形态落地。当前 ReviewTask 已扩为 13 项但没有 R10（`src/bin/monitor/review_batch.rs:408-468`）；BR-232 在 CandidateBoard 侧把 Strong 样本写入 SQLite `prediction_tracker`，盘后 side route 回填收益，并用样本数/胜率作为 CandidateTriggered promotion evidence（`src/bin/monitor/push_templates.rs:5714-5745,7821-7850,9190-9225,9695-9699`）。因此：

- “信号→未来收益”的最小闭环是 `CURRENT/PARTIAL`；
- “所有成功推送都记录、统一方向、5 个交易日而非自然日、R10 独立 task、日/周报告”没有由原验收证明；
- 当前代码用 `pred_date + Duration::days(5)` 生成 target，却在回填函数按逐日 offset/次日 target 处理，交易日语义需要单独审计，不能直接宣称 v19.1 完成；
- 后续优先把现有 SQLite owner 抽成窄 `OutcomeTracker` port，而不是再建平行 JSONL SignalTracker；是否新增 ReviewTask 必须由 completion owner 决定。

**v19.2 AI 改进：** role-based `LlmRegistry` 已实现 DeepSeek/MiniMax 与 fallback（`src/llm/registry.rs:1-112`），但 `GeminiAnalyzer`、`agent::multi_agent`、`AI_DEBATE_ROUNDS` 和 `news_ai_shadow` 仍有生产或编译调用，故“统一 LLM/清死代码”未完成。历史 `IC=-0.0775` 只能作为重新验证理由，不能作为 2026-09-02 当前性能结论。正确门禁是：冻结 prompt/model/data version → 以 point-in-time outcome 和成本后指标重算 → 与无 AI baseline 比较 → 明确 keep/restrict/retire；不得把“IC 转正”写成会自然实现的目标。

**v19.3 全天推送：** BR-223 的 AuctionRepush、CandidateBoard/Invalidated、BlockTrade 与 IpoCatalyst 接线历史仍有价值，但文档自己的 2026-08-06 记录已承认竞价未推、Quote Unsafe 和新闻早间未命中；当前 enum 也已从其 57/59 增长到 65。其“权威版”标签由本蓝图 §24 取代。任何修订应更新 §24 machine catalog，而不是继续手改第二份全天清单。

### 25.9 与推送专项的合并边界、计划和周期

| 项目 | 与 §24 的关系 | 是否计入 §24 36--69 日 |
| --- | --- | --- |
| v19.3 时段/catalog/接线事实 | 已由 §24 全量覆盖并纠错 | 已计入，不另加 |
| v19 PR-3 typed reason、PR-6 delivery metrics、PR-9 independent health、PR-10 isolation | 与 Push Foundation 有交集；只实现推送所需最小 slice | 交集部分已计入 |
| v19 Quiet Mode、BannerSnapshot、log rotation、通用 health CLI、全 source breaker | platform-wide operational project | 不计入，需独立计划 |
| v19.1 OutcomeTracker / v19.2 AI evaluation | 可为推送语义和模型评价提供 evidence，但不是 delivery authority | 不计入，需独立 research/review plan |
| v18 四模块与 Gate P/L | future research-to-paper architecture；push 只保留 typed refs | 不计入，严禁借推送重构扩 scope |
| v20 backtest/factor/scale 草案 | 独立 discovery/PRD/ADR | 不计入 |

以 Codex 单独开发为人力基线，只有在 exact ADR/catalog 后才冻结正式工期。当前仅用于排期的区间是：v19 通用运行面 15--25 个 8 小时等效开发日，另需至少 5 个真实运行日观察；v19.1/19.2 收敛 8--15 日，另需跨持有窗口的样本等待；v18 F0--F5 45--80 日，Gate P 的模拟观察与 WORM/restore 外部证据另计日历时间；v20 回测/因子/平台扩容不估时，直到产品范围与 ADR 批准。三者不可相加后宣称一个交付日期，因为可并行程度、外部基础设施和样本窗口尚未冻结。

建议顺序不变：先完成 §24 Push Foundation 与高风险 MigrationUnit；其间只复用 v19 的 typed reason/metrics/isolation seam。推送专项 P0 Production Verified 后，再做 v19 通用运行面；随后选择 v18 F0 数据/身份 vertical slice。v20 因子/回测/平台扩容必须等待 v18 的 Decision/Fill owner 收敛，否则会制造第三套回测与账本真相。

## 26. 蓝图维护触发器

发生以下任一变化时必须更新本文：

- `src/lib.rs` 顶层模块增删。
- Cargo target、依赖、生产构建命令或 provider-host 部署合同改变；本项目当前未定义 Cargo feature。
- monitor main/background task 树或 startup gate 改变。
- proto Operation/RPC、consumer-used operation catalog 或外部 provider-host capability 改变。
- frozen/global/selection/durable schema catalog、增量 owner 表族改变。
- `DecisionState`、合法 transition、PushKind/policy catalog 改变。
- provider import prohibition、gateway evidence model、retry/error classification 改变。
- authority storage 路径、bundle authentication、deployment topology 改变。
- CI workflow、coverage policy、integration/source/process tests 改变。
- v18 四核心合同、Gate P/L、v19 operational PR、BR-232 OutcomeTracker 或 v20 backtest/factor/scale 提案的状态改变。
- `DataEnvelope`、`AuditJournal`、investment `DecisionRecord`、`PaperExecution`、`BannerSnapshot`、typed operational `ErrorCode` 等 §25 negative probe 中的类型开始落地。

建议自动对账项：

```text
src/lib.rs public modules                 == 61
Cargo binary targets                     == 28
tests/*.rs integration targets           == 41
grpc consumer-used operations            == 40
merged Operation enum values             == 0..=62
DecisionState variants                   == 14
legacy catalog                           == 53 tables / 44 indexes / 63 triggers
selection-v2 final catalog               == 12 tables / 5 indexes / 17 static triggers
durable delivery CREATE TABLE set        == 18
PushKind enum                            == 65
production presentation tuples          == 58 / 54 unique kinds
durable counted PushKind catalog         == 23
```

网页同步边界（2026-10-09）：此旧main蓝图Markdown为本次窄事实维护面；当前wrapper实际针对 `docs/architecture/current/Project_Architecture_Blueprint.md` 的审计pair，冻结catalog/file-set尚不包含新源码。本次实际 `--draft` 与 `--draft --check` 均exit1 `file_set_mismatch pair=current`，没有生成新HTML；旧HTML不代表本次更新。保持guard与冻结工具范围，另次明确审计同步后才能发布其HTML。历史流程说明：正文事实只在相应Markdown维护；运行 `ruby scripts/render-architecture-blueprint-html.rb --draft` 刷新预渲染正文、目录、指标、Mermaid 图源、内嵌 Markdown 与 SHA-256，再用同一命令的 `--draft --check` 模式验证 HTML 幂等且未过期。HTML仍为provisional draft，不代表冻结平台路线完成。HTML 的 CSS/交互外壳可以独立演进，但不得手改生成区来绕过源文档。

---

## 附录 A：完整代码模块与 binary inventory

### A.1 61 个 `src/lib.rs` 顶层模块

| # | 模块 | # | 模块 | # | 模块 |
| ---: | --- | ---: | --- | ---: | --- |
| 1 | `analyzer` | 22 | `indicators` | 43 | `event` |
| 2 | `announcement` | 23 | `lhb_analyzer` | 44 | `push_l1` |
| 3 | `app` | 24 | `llm` | 45 | `push_l2` |
| 4 | `auth` | 25 | `market_analyzer` | 46 | `push_l4` |
| 5 | `breakout` | 26 | `market_data` | 47 | `push_l5` |
| 6 | `broker` | 27 | `models` | 48 | `push_l6` |
| 7 | `bus` | 28 | `monitor` | 49 | `push_l7` |
| 8 | `calendar` | 29 | `news` | 50 | `strategy` |
| 9 | `capital_flow` | 30 | `notification` | 51 | `traits` |
| 10 | `chart_generator` | 31 | `opportunity` | 52 | `trend_analyzer` |
| 11 | `cli` | 32 | `performance` | 53 | `types` |
| 12 | `company_financials` | 33 | `pipeline` | 54 | `util` |
| 13 | `company_metrics` | 34 | `portfolio` | 55 | `agent` |
| 14 | `config` | 35 | `review` | 56 | `deep_analyzer` |
| 15 | `data_gateway` | 36 | `registry` | 57 | `trading` |
| 16 | `data_provider` | 37 | `risk` | 58 | `http_client` |
| 17 | `database` | 38 | `schema` | 59 | `grpc_client` |
| 18 | `decision` | 39 | `search_service` | 60 | `grpc_contract` |
| 19 | `durable_delivery` | 40 | `selection` | 61 | `market_domain` |
| 20 | `enums` | 41 | `sharpe_calculator` |  |  |
| 21 | `errors` | 42 | `signal` |  |  |

依据：`src/lib.rs`。`grpc_server` 与 `magic_compat` 已删除；provider-neutral 类型由 `market_domain` 拥有。

### A.2 主要模块的完整子模块声明

| 模块族 | 当前子模块 |
| --- | --- |
| `app` | `bootstrap`、`modes`、`schedule` |
| `analyzer` | private `analyze`、`client`、`macro_rec`、`prompts`；public types/re-exports |
| `agent` | `context`、`loop_runner`、`multi_agent`、`state`、`tool`、`toolbelt`、`tools`、`tools_chip`、`tools_money_flow`、`tools_news`、`tools_research`、`tools_sector`、`validation`、`auction_agent` |
| `data_gateway` | private `benchmark`、`board_runtime`；public `block_trade`、`board`、`board_ranking`、`capital`、`chain_intelligence`、`company`、`consensus`、`dragon_tiger`、`economic_calendar`、`event_calendar`、`evidence_time`、`exchange_calendar_authority`、`futures_delivery`、`general_web_research`、`global_market`、`global_news`、`grpc_source`、`historical_bars`、`index`、`instrument_identity`、`intraday_shape`、`market_capabilities`、`market_data`、`outcome_daily_bars`、`position_chain`、`research`、`review`、`security_lifecycle`、`sina_instrument_news`、`t0_evidence` |
| `database` | `factor_snapshot`、`repository`、`attribution_epochs`、`attribution_reports`、`benchmark_segments`、`chain_intelligence`、`concepts`、`daily_change_confirmation`、`data_acquisition_audit`、`execution_tracking`、`news_ai`、`order_audit`、`position_chain`、`account_mode_log`、`account_snapshot`、`catalyst_watchlist`、`closing_valuation`、`position_shares`、`selection`、`selection_v2`、`selection_v2_read_model`、`selection_v2_repository`、`user_account_summary`、`user_position_snapshot`；private `kline`、`lhb`、`positions`、`sqlite_descriptor_attestation` 及 schema/journal internals |
| `monitor` | `adaptive`、`alert`、`alert_log`、`attribution`、`attribution_deep`、`auction`、`checklist`、`data_mode`、`data_quality`、`detector`、`entity_linker`、`event_bus`、`news_ai`、`news_monitor`、`prediction`、`rate_budget`、`risk`、`scanner`、`signal_fusion`、`signal_state`；private `integration` |
| `selection` | `acquisition_v2`、`activation_gate`、`activation_runtime`、`admission`、`audit`、`config_activation_v2`、`features`、`ingress_v2`、`model`、`outcome_v2`、`persistence_v2`、`quality`、`relation`、`schema_v2`、`trading_calendar_v2`；private `process_bootstrap`、`outcome_session_gate` |
| `decision` | `capital_verify`、`decision_decide`、`decision_panel`、`decision_render`、`exclusion`、`holding_plan`、`intraday_monitor`、`layers`、`leader`、`live_plan`、`pre_trade_filter`、`rotation`、`sector_score`、`t0_advisor` |
| `pipeline` | `chain_analysis`、`result_types`、`score_breakdown`、`section_utils`、`technical_report`、`veto_rules`、nested `data`/`analyze`；private `backtest_runner`、`macro_news`、`market_regime`、`reporting`、`summary_notify`、`trade_type`，以及同目录辅助文件 |
| `opportunity` | `auction_agent`、`bom_kb`、`candidate_panel`、`candidate_state`、`chain_mapper`、`discover`、`event_extractor`、`hit_case`、`impact`、`launch_gate`、`real_alpha`、`scheduler`、`score`、`virtual_reason`、`winrate` |
| `review` | `equity`、`factor_ic`、`factor_report`、`journal`、`report`、`sop`、`backtest`、`watchlist_tracking`、`catalyst_review`、`failure_attribution`、`lhb_review`、`limit_chain_review`、`market_stage`、`performance_feedback`、`signal_review`、`tomorrow_watchlist` |
| `performance` | `attribution`、`attribution_epoch`、`attribution_replay`、`economic_position`、`report`、`snapshot` |
| `market_analyzer` | `async_overview`、`limit_chain_review`、`market_stage_confidence`、`performance_feedback`、`post_close_review`、`review`、`sector_monitor`；private `indices`、`limit_up`、`statistics` |
| `risk` | `cash_guard`、`env_guard`、`limits`、`sector_exit`、`stop_loss`、`veto_chain`、`veto_rules_live`、`account_mode`、`action_gate` |
| `trading` | `order_safety`、`paper_engine`、`paper_sell`、`paper_trade`、`risk_adapter`；private `paper_lot_ledger` |
| `strategy` | `boll_macd`、`bollinger_zscore`、`contrarian`、`core`、`lot`、`multi_factor`、`multi_timeframe`、`rsi`、`v16_4` |
| `news` | `aggregator`、`dispatcher`、`impact`、`ipo`、`sink`、`stock_mapper` |
| `event` | `bus`、`cli`、`delivery_settlement`、`dispatcher`、`durable_delivery_append`、`envelope`、`history`、`jsonl_writer`、`push_record`、`replay` |
| `notification` | `config`、`email`、`feishu`、`report`、`service`、`wechat` |
| `grpc_client` | `auth`、`bundle`、`client`、`envelope`、`errors`、`external_v1`、`pb`、`retry` |
| `grpc_contract` | `ops`、`params`、`schema`、`validate` |
| `push` | L1 `event`；L2 template；L4 `dispatcher`；L5 `governance`；L6 `sink`/`external_sinks`；L7 `analytics`/`sqlite_store`；L3 未建 module |
| `portfolio` | `closing_valuation`、`user_position_snapshot`；private `store` |
| `data_provider` | `chip_distribution`、`consensus`、`halt_status`、`limit_status`、`service`、`news_item` |
| `market_domain` | `bars`、`evidence`、`instrument`、`lifecycle`、`market`、`provider_id`、`ranking`、`record`、`tdx`、`value` |
| `indicators` | private `cross`、`divergence`、`kdj`、`macd`、`rsi`、`skdj`；public `multi_period` |
| `signal` | `market_event`、`push_recorder` |
| `breakout` | `engine`、`position`、`signal` |
| `llm` | `providers`、`registry`、`ticker_extractor` |

### A.3 历史：2026-09的28个binary targets

| 类别 | targets |
| --- | --- |
| 核心运行单元 | `stock_analysis`、`monitor` |
| 数据/连接/合同探针 | `announcement_probe`、`board_directory_probe`、`gateway_quote_probe`、`grpc_bundle_probe`、`grpc_local_readiness_probe` |
| 运维、导入、迁移、回填 | `attribution_backfill`、`backfill_catalyst_watchlist`、`backfill_daily`、`backfill_predictions`、`confirm_daily_change`、`import_real_account_snapshot`、`import_user_account_summary`、`import_user_position_snapshot`、`migrate_selection_v2`、`selection_activation_prepare`、`run_closing_valuation` |
| 分析、研究、回测 | `agent_test`、`boll_macd_backtest`、`deep_analyze`、`economic_position_probe`、`lhb_query`、`produce_winrate_samples`、`rsi_optimize`、`strategy_attribution`、`v14_e2e`、`winrate_simulator` |

依据：Cargo metadata 的 `kind=["bin"]` targets；不能仅统计 `src/bin/*.rs`，因为 `monitor` 是目录式 target，`stock_analysis` 在 `src/main.rs`。

## 附录 B：历史2026-09的41个integration-test targets

| # | target | # | target |
| ---: | --- | ---: | --- |
| 1 | `attribution_epoch_integration` | 22 | `monitor_help_isolation` |
| 2 | `board_gateway_consumer_cutover` | 23 | `news_item_test` |
| 3 | `bom_kb_tests` | 24 | `north_flow_option_tests` |
| 4 | `br192_candidate_counted_binding` | 25 | `notification_channels_tests` |
| 5 | `br192_main_fail_closed_counted_producers` | 26 | `opportunity_e2e_tests` |
| 6 | `br192_monitor_test_counted_cleanup` | 27 | `position_tracker_tests` |
| 7 | `br192_paper_trade_counted_binding` | 28 | `ranking` |
| 8 | `br192_paper_trade_quote_freshness` | 29 | `review_timeout_tests` |
| 9 | `br193_selection_activation` | 30 | `rule_filter_benchmark` |
| 10 | `br193_selection_scheduler` | 31 | `score_tests` |
| 11 | `chain_exclusive` | 32 | `selection_process_bootstrap_isolation` |
| 12 | `durable_delivery_counted_cutover` | 33 | `test_coverage_thresholds` |
| 13 | `e2e_dedup` | 34 | `test_data_freshness_check` |
| 14 | `e2e_prediction_verify` | 35 | `test_design_contradiction` |
| 15 | `event_extractor_tests` | 36 | `test_fake_impl_check` |
| 16 | `flash_filter` | 37 | `tool_binary_process_isolation` |
| 17 | `grpc_bridge_e2e` | 38 | `unified_data_architecture` |
| 18 | `grpc_channel_e2e` | 39 | `v11_three_sources` |
| 19 | `holding_summary_tests` | 40 | `v12_p0_3_halt` |
| 20 | `launch_gate_tests` | 41 | `winrate_tests` |
| 21 | `market_event_tests` |  |  |

依据：Cargo metadata 与 `tests/*.rs` 当前文件集。

## 附录 C：gRPC Operation 完整目录

### C.1 40 个 production-used consumer operations

| ID | Operation | ID | Operation | ID | Operation |
| ---: | --- | ---: | --- | ---: | --- |
| 3 | `RealtimeQuotes` | 1 | `HistoricalBars` | 2 | `MinuteData` |
| 5 | `OrderBooks` | 4 | `MoneyFlows` | 8 | `SecurityMetadata` |
| 18 | `Announcements` | 17 | `GlobalNews` | 11 | `EconomicCalendar` |
| 12 | `FuturesDelivery` | 9 | `GlobalIndices` | 27 | `BoardDirectory` |
| 28 | `BoardConstituents` | 36 | `BoardFlows` | 44 | `LimitPools` |
| 45 | `StrongStockReasons` | 46 | `DragonTiger` | 47 | `MarketDragonTiger` |
| 49 | `MarketRankings` | 52 | `ConceptHits` | 32 | `Consensus` |
| 30 | `ResearchReports` | 38 | `BlockTrades` | 43 | `NorthboundDaily` |
| 10 | `ForeignExchange` | 23 | `FinancialStatements` | 24 | `MarketStatistics` |
| 25 | `TechnicalBars` | 26 | `CorporateActions` | 34 | `SemanticSearch` |
| 35 | `FundFlowSeries` | 54 | `ProviderTopNRankings` | 56 | `IndexQuotes` |
| 55 | `InstrumentNews` | 57 | `IntradayShape` | 58 | `T0Evidence` |
| 59 | `OutcomeDailyBars` | 60 | `UpperLimitPoolReview` | 61 | `ChainBatch` |
| 62 | `BenchmarkBars` |  |  |  |  |

### C.2 冻结但未加入 consumer-used 集的 22 个 operations

`Auctions(6)`、`Trades(7)`、`ReferenceRates(13)`、`OfficialFxFixings(14)`、`EconomicSeries(15)`、`CompanyFilings(16)`、`MarketAnnouncements(19)`、`InvestorQuestions(20)`、`PolicyDocuments(21)`、`SecurityProfiles(22)`、`BoardMemberships(29)`、`ResearchDocuments(31)`、`TargetPrices(33)`、`MarginData(37)`、`HolderCounts(39)`、`LockupEvents(40)`、`DividendPlans(41)`、`PostCloseFlows(42)`、`DragonTigerDiscovery(48)`、`MarketBreadth(50)`、`Popularity(51)`、`OptionData(53)`。

此外 `Operation::Unspecified(0)` 只用于合同默认值，不是业务 operation。全量映射证据：`client-bundle/market.proto` + `build.rs` compatibility extensions + `src/grpc_contract/ops.rs:8-73`；consumer-used 集证据：`:77-128`。该集合描述本仓调用面，不代表本仓实现 provider RPC；外部 provider-host 必须满足这些 capability。

## 附录 D：legacy generation-1 冻结业务表（53）

| 领域 | tables |
| --- | --- |
| 账户/持仓/估值 | `account_mode_log`、`real_account_snapshot`、`user_account_summary`、`user_position_snapshot`、`user_position_snapshot_item`、`stock_position`、`position_adjustments`、`position_chain_assignment`、`closing_valuation_run`、`closing_valuation_item` |
| 行情/市场/概念 | `stock_daily`、`lhb_daily`、`board_rotation_daily`、`chain_daily`、`stock_concepts`、`concept_cooccurrence` |
| 分析/信号/预测 | `analysis_result`、`factor_snapshot`、`signal_state`、`prediction_tracker`、`agent_scratchpad` |
| 新闻 | `news_items`、`news_dedup`、`news_ai_assessment`、`news_ai_assessment_chain`、`event_seen_simhash`、`topic_novelty_history` |
| 产业链 intelligence | `chain_intelligence_batches`、`chain_intelligence_input_evidence`、`chain_intelligence_chains`、`chain_intelligence_members`、`chain_intelligence_rejections`、`chain_intelligence_visibility_receipts` |
| 采集与确认审计 | `data_acquisition_audit`、`data_acquisition_audit_chain`、`daily_change_confirmation`、`daily_change_confirmation_chain` |
| 交易与订单 | `trades`、`paper_trades`、`paper_performance_snapshot`、`ledger`、`order_idempotency`、`order_audit`、`order_audit_chain`、`execution_tracking` |
| legacy selection | `selection_event_inbox`、`selection_event_completions`、`selection_runs`、`selection_candidates`、`selection_feature_snapshots`、`selection_outcomes`、`selection_visibility_receipts` |
| 推送记录 | `pushed_stocks` |

冻结形状同时包含 44 个 explicit indexes 与 63 个 triggers。唯一目录证据：`src/database/fixtures/global_schema_legacy_catalog_v1.tsv`；捕获/验证逻辑：`src/database/global_schema_catalog_v1.rs`。

## 附录 E：selection-v2 final tables（12）

1. `selection_source_batch_attempts`
2. `selection_source_facts_v2`
3. `selection_source_fact_attempts`
4. `selection_relation_attempts`
5. `selection_evaluation_attempts`
6. `selection_samples`
7. `selection_rejections`
8. `selection_sample_outcomes`
9. `selection_outcome_attempts`
10. `selection_v2_recovery_envelopes`
11. `selection_v2_run_stages`
12. `selection_v2_commit_receipts`

5 个 indexes：`selection_v2_one_activation_per_config`、`selection_v2_source_facts_pending`、`selection_v2_samples_generation`、`selection_v2_outcome_attempt_run`、`selection_v2_receipt_subject`。

17 个 static triggers：

`selection_v2_batch_lineage`、`selection_v2_fact_lineage`、`selection_v2_fact_attempt_lineage`、`selection_v2_relation_requires_admitted_source`、`selection_v2_evaluation_requires_admitted_source`、`selection_v2_sample_requires_admitted_source`、`selection_v2_rejection_requires_admitted_source`、`selection_v2_manifest_envelope_binding`、`selection_v2_config_manifest_closure`、`selection_v2_ingress_manifest_closure`、`selection_v2_generation_manifest_closure`、`selection_v2_outcome_manifest_closure`、`selection_v2_receipt_manifest_binding`、`selection_v2_config_receipt_closure`、`selection_v2_ingress_receipt_closure`、`selection_v2_generation_receipt_closure`、`selection_v2_outcome_receipt_closure`。

证据：`src/database/global_schema_catalog_v1.rs:27-126`、`src/database/selection_v2.rs:18-31`。

## 附录 F：durable delivery tables（18）

| 组 | tables |
| --- | --- |
| decision/policy/audit | `delivery_decisions`、`delivery_policy_catalog`、`immutable_audit_outbox` |
| reservation/claim | `cooldown_reservations`、`cooldown_heads`、`business_date_once_claims`、`daily_budget_reservations` |
| attempts/results | `delivery_attempts`、`sink_results` |
| review/manual | `review_terminal_replay_attempts`、`review_terminal_replay_completions`、`manual_resolutions` |
| payloads | `delivery_disposition_payloads`、`task_transition_payloads` |
| append-only events | `delivery_state_events`、`delivery_attempt_events`、`cooldown_reservation_events`、`daily_budget_reservation_events` |

证据：`src/durable_delivery/schema.rs:155-520`。

## 附录 G：DecisionState 完整目录（14）

1. `Reserved`
2. `AttemptInFlight`
3. `AcceptedAuditPending`
4. `AcceptedTaskTransitionPending`
5. `Delivered`
6. `RejectedAuditPending`
7. `RejectedTaskTransitionPending`
8. `RejectedDurable`
9. `UncertainAuditPending`
10. `UncertainTaskTransitionPending`
11. `UncertainManualReview`
12. `ManualRejectedAuditPending`
13. `ManualRejectedTaskTransitionPending`
14. `ManualResolvedRejected`

证据：`src/durable_delivery/model.rs:800-814`。

## 附录 H：关键证据矩阵

| 架构结论 | 主要代码证据 | 防漂移测试/配置 |
| --- | --- | --- |
| 单 package、28 binaries | `Cargo.toml`、Cargo metadata | `cargo metadata --no-deps` |
| 仓外 provider-host、本仓 client-only | README、`grpc_source.rs`、Cargo targets | `tool_binary_process_isolation`、`grpc_channel_e2e` |
| 本仓无 provider SDK/生产 server target | `Cargo.toml`、`src/data_gateway/**`、`src/bin/**` | `scripts/check-no-magic-dependencies.sh all`、`unified_data_architecture` |
| 40 个 consumer-used gRPC operations | `grpc_contract/ops.rs` | 同文件 `implemented_set_is_40_and_within_62`（历史测试名，语义为 consumer-used catalog） |
| bridge call-site catalog 不漂移 | `grpc_source.rs::HOOKED_OPS` | `hooked_ops_match_bridge_for_call_sites` |
| selection disabled 零副作用 | activation runtime + monitor startup | `br193_selection_activation`、`selection_process_bootstrap_isolation` |
| durable counted delivery | `durable_delivery/*` + monitor runtime | `durable_delivery_counted_cutover`、多组 BR-192 tests |
| audit before observation | monitor startup、event dispatcher/writer | event delivery observation tests、monitor replay tests |
| DB descriptor authority | `database/mod.rs` | namespace swap、WAL、query-only、attestation inline tests |
| legacy frozen schema 53/44/63 | frozen TSV + catalog | global schema catalog tests |
| selection final 12/5/17 | catalog + selection_v2 DDL | partial/future/extra object rejection tests |
| paper engine disabled、paper trade active | monitor loop、paper_engine、intraday_monitor、paper_sell | BR-211 source test、paper quote/count binding integrations |
| coverage/compliance | `.github/workflows/*`、`config/design_contracts.toml` | `test_coverage_thresholds`、compliance scripts |

## 附录 I：架构非目标与已知债务

2026-10-09新增边界：E1–E4已实现本地工具、release候选包STAGED；installation/source-qualified SELL/可执行Streak/真实provider与human-value pending。E5结构改造继续conditional，未来先从单模块seam、可测收益与public surface收窄开始。见[本次交接](handoffs/2026-10-09-goal-first-delivery.md)与已保存的原v2方案。

- 非目标：微服务拆分、Kubernetes、真实 broker 下单、Exactly-once broadcast、bus 作为 durable queue、全系统 TOML 热重载。
- 已知债务：`bin`/database/data_gateway 超大；单 crate 依赖环；legacy/v2 selection 并存；NotificationService、push L1-L7、production durable route 多套抽象并存；L3 renderer 缺位；22 个 gRPC operations 只冻结合同、未进入 consumer-used 集；`build.rs`/`Cargo.toml` 的少量注释仍把测试 fixture/server trait 描述成历史本地 server；Prometheus metrics 模块尚未接入 monitor composition root 或 HTTP exporter；no-Magic guard 尚未接入 CI；compliance workflow 仍引用已不存在的 `--test e2e` target。
- 风险最高的改动面：monitor composition root、global/selection schema authority、durable coordinator transition/reconciliation、gRPC evidence preservation、provider boundary。
- 推荐演进顺序：先用 tests 固化 seam，再抽深模块接口；优先缩小 monitor/database/data_gateway 的 public surface，而不是只按文件大小机械拆分。

本文到此覆盖当前架构全貌；任何与代码冲突的描述都应以当前实现与可运行验证为准，并触发第 26 节的蓝图更新流程。
