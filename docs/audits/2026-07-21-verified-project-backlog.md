# 2026-07-21 已核验项目问题与去重待办

## 1. 范围与口径

本报告把本轮 48 项盘点逐项映射到当前代码、设计文档、合规脚本和已保存的运行证据。它是核验结果与工程待办，不替代发布 Gate D、PR 审批或线上审计记录。

- `Confirmed`：当前代码、文档或运行证据足以确认问题存在。
- `Partial`：问题方向成立，但原表述过宽，或仓库已经覆盖其中一部分。
- `Fixed`：本轮核验时已有可定位的修复实现与验证入口。
- `False`：现有证据反驳该表述，或证据不足以把它登记为独立缺陷。
- `Duplicate`：仅用于去重说明；为保留原始裁定，附录中的 #41 仍记为 `Confirmed`，并标注与 #35 重复。

本报告只记录证据路径与结论，不摘录真实账户数值、持仓、成交、净值或截图。运行证据引用 `/private/tmp/stock_analysis_monitor.log` 时，同样不复制其中的敏感内容。

### 1.1 核验汇总

| 状态 | 数量 |
| --- | ---: |
| Confirmed | 23 |
| Partial | 13 |
| Fixed | 3 |
| False | 9 |
| 合计 | 48 |

此前对话中的“24 Confirmed / 13 Partial / 11 Fixed+False”是算术错误；按逐项裁定重算后，应为 **23 / 13 / 12**，总数仍为 48。去重待办只合并 #41 → #35，不改变附录的逐项状态。

## 2. 本轮已修复与待发布实现

BR-138 与 BR-139 的实现已通过独立的代码/规格双轴审查，本报告将二者标记为已修复，不再列入未完成工作：

- **BR-138**：生产路径已使用统一的 `NewsOuterTickCoordinator`，共用即时候选门禁，并隔离不同消费者。证据：`docs/business_rules.md`、`docs/superpowers/specs/2026-07-21-announcement-relevance-design.md`、`src/bin/monitor/main.rs`、`src/bin/monitor/push_templates.rs`。
- **BR-139**：盘后复盘由单一调度所有者驱动，结果运行器严格处理到期与重试。证据：`docs/business_rules.md`、`docs/superpowers/specs/2026-07-21-post-session-review-scheduler-design.md`、`src/bin/monitor/main.rs`、`src/bin/monitor/push_templates.rs`。

BR-140 已在当前功能分支完成逐任务强类型结果、A-01/A-10/R-03 逐标的隔离、R-08 逐组件降级，以及公告主/备用真实协议和逐正文隔离。焦点测试已通过；在全量 Gate D、PR 合并和生产 canary 完成前，本报告只称其为“待发布实现”，不称生产修复完成。证据：`docs/superpowers/specs/2026-07-21-post-session-review-partial-evidence-design.md`、`docs/superpowers/plans/2026-07-21-post-session-review-partial-evidence.md`、`src/bin/monitor/review_batch.rs`、`src/data_provider/announcement.rs`。

发布 Gate D 的长期生产证据仍按发布流程单独积累；它不重新打开上述两项实现缺陷。

## 3. 逐项核验（1–48）

| # | 原问题摘要 | 裁定 | 简短证据与说明 |
| ---: | --- | --- | --- |
| 1 | banner 全局单点 | Partial | `src/bin/monitor/main.rs`、`src/monitor/data_mode.rs`：全局 banner 状态仍是集中单点，但已有显式刷新/指标同步入口，不能概括为完全不可控。 |
| 2 | consensus 无熔断 | Confirmed | `src/data_provider/consensus.rs`、`src/news/aggregator/feed.rs`：存在共识源读取与失败处理，未见独立熔断状态机。 |
| 3 | 24/7 空转 | Partial | `src/bin/monitor/main.rs`、`docs/v19.x/v19.0-operational-clarity-design.md`：部分循环缺少统一交易时段/空闲退避策略；并非所有任务都持续空转。 |
| 4 | 日志不轮转 | Confirmed | `docs/v19.x/v19.0-operational-clarity-design.md`、`/private/tmp/stock_analysis_monitor.log`：运行证据与 v19 设计确认尚无正式轮转/保留实现。 |
| 5 | test/prod 边界 | Fixed | `tests/monitor_help_isolation.rs`、`src/bin/monitor/main.rs`：帮助/测试隔离与生产入口防护已有回归验证。 |
| 6 | 健康失败静默 | Partial | `src/bin/monitor/main.rs`、`docs/v19.x/v19.0-operational-clarity-design.md`：已有部分告警与指标，但某些后台任务仍只记录或降级，缺统一健康失败契约。 |
| 7 | banner / data mode 不同步 | Fixed | `src/bin/monitor/main.rs`、`src/monitor/data_mode.rs`：`refresh_banner_state_with_metrics` 等路径已同步状态与指标。 |
| 8 | EarningsMiss 命中零推 | False | `src/bin/monitor/v17_sources.rs`：`EarningsMiss` 已有生产来源路由与测试，不能由旧的“零推”现象推断当前路径缺失。 |
| 9 | 持仓使用 K 线 close 而非实时价 | Confirmed | `src/pipeline/analyze.rs`、`src/pipeline/position_tracker.rs`：持仓/分析链路仍从 K 线收盘字段构造价格输入。 |
| 10 | track / save 非同事务 | Confirmed | `src/pipeline/analyze.rs`、`src/pipeline/position_tracker.rs`、`src/database/kline.rs`：跟踪与保存结果是两个独立调用，未共享原子事务。 |
| 11 | gateway DB panic | Confirmed | `src/trading/mod.rs`、`src/database/repository.rs`：模拟执行网关依赖全局数据库单例，初始化失败路径仍可能 panic。 |
| 12 | MarketAnalyzer 错误转空 Vec | Confirmed | `src/bin/monitor/main.rs`、`src/bin/monitor/push_templates.rs`：部分分析错误被转换为空集合，丢失“无结果”和“执行失败”的区别。 |
| 13 | 推送吞错 | Partial | `src/bin/monitor/notify.rs`、`src/bin/monitor/push_templates.rs`：底层已有 `PushOutcome`，但若干兼容分发器仍压缩为 `bool`，错误语义未全链路保留。 |
| 14 | MAE/MFE 缺失转 0 | Confirmed | `src/opportunity/news_outcome.rs`：缺失的 MAE/MFE 使用 `unwrap_or(0.0)`，违反缺失数据显式化要求。 |
| 15 | fusion 权重 0 使来源消失 | Partial | `src/monitor/signal_fusion.rs`、`.planning/2026-07-18-v18-ws0-test-inventory/findings.md`：默认零权重可让来源静默失效；已有配置结构，但缺可观测性/硬校验。 |
| 16 | `Local::now` 跨日 | False | `src/news/aggregator/source_event.rs`、`src/bin/monitor/v17_sources.rs`、`docs/business_rules.md`（BR-137）：日期解析与新鲜度规则已有专门修复，宽泛跨日缺陷缺当前证据。 |
| 17 | `limit_up_count` 缺失转 0 | Confirmed | `src/opportunity/news_ranker.rs`：缺失涨停计数使用 `unwrap_or(0)`，把未知值静默变成有效零值。 |
| 18 | metrics 仅 6 个 | Confirmed | `src/bin/monitor/metrics.rs`、`docs/v19.x/v19.0-operational-clarity-design.md`：当前模块只暴露六类核心指标，v19 扩展仍属设计。 |
| 19 | 铁律硬编码 | Confirmed | `src/trading/paper_engine.rs`：通过“铁律”文本和固定原因/阈值分支驱动规则，仍存在硬编码耦合。 |
| 20 | 无 persona/profile | Confirmed | `src/decision/`、`docs/v18.x/v18.0-2026-07-16-brainstorming-quant-platform-closure-design-active.md`：未见策略 persona/profile 领域模型；模板元数据 profile 不是策略人格。 |
| 21 | 规则不可关闭 | Partial | `config/strategy.toml`、`config/chain.toml`、`src/trading/paper_engine.rs`：部分功能有开关，但“铁律”路径缺完整的逐规则启停契约。 |
| 22 | 无亏损归因 | Partial | `src/review/failure_attribution.rs`、`src/bin/monitor/push_templates.rs`：已有归因模块，但 R-06 生产分发仍因缺分类失败来源而禁用。 |
| 23 | 无自救窗口 | Partial | `src/trading/paper_engine.rs`、`src/risk/`：存在重试、去抖和风控规则，但没有显式的“自救窗口”领域契约。 |
| 24 | DataEnvelope 未落地 | Confirmed | `docs/v18.x/v18.0-2026-07-16-codebase-design-four-core-modules.md`、`docs/v19.x/v19.0-operational-clarity-design.md`：仍是设计概念，生产源码无对应核心类型。 |
| 25 | AuditJournal trait 未落地 | Confirmed | 同上；审计日志有零散实现，但设计中的 `AuditJournal` 端口未落地。 |
| 26 | DecisionRecord 未落地 | Confirmed | `docs/v18.x/v18.0-2026-07-16-codebase-design-four-core-modules.md`、`src/decision/`：设计中的统一决策记录模型尚未成为生产边界。 |
| 27 | PaperExecution 状态机不全 | Confirmed | `src/trading/paper_trade.rs`、`src/trading/paper_engine.rs`、v18 四核心模块设计：当前模拟执行未覆盖设计要求的完整生命周期。 |
| 28 | L3 Render 物理位置错误 | Confirmed | `src/bin/monitor/push_templates.rs`、`.planning/2026-07-18-v18-ws0-test-inventory/findings.md`：渲染职责仍集中在二进制模块，规划的独立 L3 边界未落地。 |
| 29 | Diesel + rusqlite 双 DB | Confirmed | `Cargo.toml`、`src/database/`、`src/event/push_record.rs`：两套 SQLite 访问栈并存，事务、迁移和错误语义未统一。 |
| 30 | DDD 边界松散 | Partial | `src/bin/monitor/main.rs`、`src/bin/monitor/push_templates.rs`、`CLAUDE.md`：已有上下文模块，但大量编排与领域决策仍聚集在二进制层。 |
| 31 | Gate P 未量化 | Partial | v18 active 设计、`.planning/2026-07-18-v18-ws0-test-inventory/findings.md`：已有定性 Gate P，但部分晋级阈值和证据周期未形成一致、可执行数值。 |
| 32 | WORM 提供方未选 | Confirmed | v18 active 设计、`.planning/2026-07-18-v18-ws0-test-inventory/findings.md`：长期防篡改审计提供方仍是待决项。 |
| 33 | FillModel 无人认领 | False | `src/trading/paper_trade.rs`、`src/trading/paper_engine.rs`：没有足够证据证明“无人认领”；其有效部分已由 #27 的执行状态机缺口覆盖。 |
| 34 | Champion/Challenger 共享 book 方法瑕疵 | False | `docs/business_rules.md`（BR-042）、v18 active 设计：当前主要是规格概念，尚无已落地共享 book 方法可供确认该实现缺陷。 |
| 35 | v18 §6.3 与 Gate P0 冲突 | Confirmed | v18 active 设计、`.planning/2026-07-18-v18-ws0-test-inventory/findings.md`：WORM/Gate P 要求与本地 `audit_events` P0 提案存在冲突。 |
| 36 | compliance 不扫描完成措辞 | Confirmed | `tools/compliance/check.sh`：现有合规入口不含“完成/已上线”等状态措辞与证据一致性扫描。 |
| 37 | copilot instructions 缺失 | False | `.github/copilot-instructions.md` 文件存在，且已纳入本次预检。 |
| 38 | CLAUDE completion 软规则 | Partial | `CLAUDE.md`、`.github/workflows/compliance.yml`：文档定义为硬规则，但 CI 尚未完全编码生产证据与状态措辞约束。 |
| 39 | Spec Evidence 只靠对话 | Confirmed | `CLAUDE.md`、`.gitignore`、`.planning/2026-07-16-historical-doc-code-audit/findings.md`：部分规格/完成证据未形成可独立复现、可跟踪的仓库证据。 |
| 40 | PR 字段未强制 | Partial | `.github/pull_request_template.md`、`tools/compliance/lib/check_pr_evidence.sh`、`.github/workflows/compliance.yml`：字段存在且 PR CI 会检查，但值语义和证据真实性仍只得到部分强制。 |
| 41 | Gate P0 本地 audit 违反红线 | Confirmed | 与 #35 为同一冲突的另一表述；附录保留原裁定，去重待办只登记一次。证据同 #35。 |
| 42 | 盘点 185 估算错，实际 1748 | False | `.planning/2026-07-18-v18-ws0-test-inventory/findings.md`、`docs/v16.x/v16.x-completion-audit-2026-07-19.md`：185 是受影响范围盘点，不是全仓测试总数。 |
| 43 | fallback 行号漂移 | False | `src/data_provider/fallback.rs`：行号变化不构成功能缺陷，证据应锚定路径与符号。 |
| 44 | decision 行号漂移 | False | `src/decision/decision_decide.rs`：同 #43，未发现由行号漂移造成的行为问题。 |
| 45 | 推送量波动无 SLO | Partial | `docs/v14.x/v14.2-2026-07-11-brainstorming-push-postmortem.md`、`docs/v19.x/v19.0-operational-clarity-design.md`：已有复盘指标/目标草案，但缺正式生产 SLO 与告警闭环。 |
| 46 | test/prod 同结构 | Fixed | `tests/monitor_help_isolation.rs`、`src/bin/monitor/main.rs`：测试与生产入口隔离已实现；保留相似结构本身不是缺陷。 |
| 47 | 7/19–20 无 push log | False | `.planning/2026-07-20-monitor-48h/progress.md`：该时段已有保存的生产推送/交付证据；本报告不复制其具体数值。 |
| 48 | 系统/业务共路径 | Confirmed | `src/bin/monitor/notify.rs`、`src/bin/monitor/v14_adapter.rs`、`src/bin/monitor/push_templates.rs`、WS0 findings：系统状态与业务决策推送仍共享关键路径。 |

## 4. 去重后的未完成工作

优先级依据数据安全、资金安全、故障影响面和发布阻塞程度排序。`Fixed`、`False` 以及 BR-138/BR-139 不进入本节；#41 已并入 #35，#33 的有效部分已并入 #27。

### P0 — 数据真实性、资金安全与生产阻塞

| 待办 | 对应项 | 完成边界 | 证据入口 |
| --- | --- | --- | --- |
| 修复盘后 Eastmoney K 线数据源失败 | 新运行证据 | 数据源失败显式返回并有可执行恢复路径，不回退假数据 | `/private/tmp/stock_analysis_monitor.log`、`src/data_provider/fallback.rs` |
| 完成 R-08 公告源生产验证 | 新运行证据 | 当前分支已实现严格主/备用协议、逐正文隔离和组件降级；仍需 Gate D、合并后真实分发 canary | `/private/tmp/stock_analysis_monitor.log`、`src/data_provider/announcement.rs`、`src/bin/monitor/push_templates.rs` |
| 补齐异常涨跌幅的上市状态/板块规则校验 | 新运行证据 | 计算前按上市状态、板块涨跌停规则验证；异常数据必须阻断并告警 | `/private/tmp/stock_analysis_monitor.log`、`src/bin/monitor/market_data.rs`、`src/monitor/data_quality.rs` |
| 补齐 R-02 指数必需字段 | 新运行证据 | 字段缺失保持未知/错误，不填零；分发前通过字段完整性验证 | `/private/tmp/stock_analysis_monitor.log`、`src/bin/monitor/push_templates.rs` |
| 改用满足新鲜度门槛的持仓估值价格，并禁止缺失值转零 | #9, #14, #17 | 实时价、MAE/MFE、涨停计数均有来源/时间；缺失不参与静默计算 | `src/pipeline/position_tracker.rs`、`src/opportunity/news_outcome.rs`、`src/opportunity/news_ranker.rs` |
| 让 position track/save 原子化，并消除 gateway DB panic | #10, #11 | 同一业务操作共享事务；数据库不可用时显式失败且可测试 | `src/pipeline/analyze.rs`、`src/pipeline/position_tracker.rs`、`src/trading/mod.rs` |
| 禁止 MarketAnalyzer 把错误伪装成空结果 | #12 | `无命中` 与 `分析失败` 使用不同类型/指标/告警，调用方不得吞错 | `src/bin/monitor/main.rs`、`src/bin/monitor/push_templates.rs` |
| 落地四个核心安全契约与完整模拟执行状态机 | #24, #25, #26, #27 | `DataEnvelope`、`AuditJournal`、`DecisionRecord`、`PaperExecution` 成为生产边界并覆盖失败路径 | v18 四核心模块设计、`src/trading/paper_trade.rs`、`src/trading/paper_engine.rs` |
| 选定并验证 WORM 审计提供方，解决 Gate P 冲突 | #32, #35（含重复 #41） | 提供方、保留期、不可篡改证据和迁移/回滚已明确；不得用本地普通表冒充 WORM | v18 active 设计、WS0 findings |
| 分离系统状态与业务决策推送路径 | #48 | 分类、SLO、审计字段和故障隔离独立；系统噪声不得影响交易/决策通知 | `src/bin/monitor/notify.rs`、`src/bin/monitor/v14_adapter.rs`、`src/bin/monitor/push_templates.rs` |

### P1 — 运行稳定性、策略治理与合规闭环

| 待办 | 对应项 | 完成边界 | 证据入口 |
| --- | --- | --- | --- |
| 建立源级熔断、交易时段调度、健康失败升级与日志轮转 | #2, #3, #4, #6 | 每个后台任务有所有者、退避、熔断、健康状态和保留策略；失败不静默 | `src/data_provider/consensus.rs`、`src/bin/monitor/main.rs`、v19 operational clarity 设计 |
| 扩充核心指标并建立推送量 SLO | #18, #45 | 数据源、队列、调度、分发和送达均有指标；SLO、窗口、告警和复盘动作可执行 | `src/bin/monitor/metrics.rs`、v14 push postmortem、v19 operational clarity 设计 |
| 让融合权重与铁律可配置、可关闭、可审计 | #15, #19, #21 | 零权重和规则关闭必须显式可见；阈值有规格引用、版本与决策记录 | `src/monitor/signal_fusion.rs`、`src/trading/paper_engine.rs`、`config/strategy.toml` |
| 建立 persona/profile、亏损归因和自救窗口的统一领域模型 | #20, #22, #23 | 定义所有者、输入、状态转换、失败处理和审计字段，并接入生产调度 | `src/decision/`、`src/review/failure_attribution.rs`、`src/risk/` |
| 下沉 L3 Render、收敛双 DB、加深 DDD 边界 | #28, #29, #30 | 渲染不再驻留二进制巨模块；数据库端口/事务统一；领域决策与编排分离 | `src/bin/monitor/push_templates.rs`、`Cargo.toml`、`src/database/` |
| 量化 Gate P 晋级/回退标准 | #31 | 阈值、观察周期、样本要求、失败回退和责任人均可机器或人工复核 | v18 active 设计、WS0 findings |
| 把完成措辞、规格证据与 PR 字段纳入强制校验 | #36, #38, #39, #40 | CI 同时校验字段存在、值语义、证据可复现性和完成声明；失败阻止合并 | `tools/compliance/check.sh`、`tools/compliance/lib/check_pr_evidence.sh`、`.github/workflows/compliance.yml` |

### P2 — 可维护性与诊断质量

| 待办 | 对应项 | 完成边界 | 证据入口 |
| --- | --- | --- | --- |
| 移除 banner 全局单点或封装成显式状态端口 | #1 | 所有读写走单一接口，生命周期、同步和失败策略可测试 | `src/bin/monitor/main.rs`、`src/monitor/data_mode.rs` |
| 在推送兼容层保留结构化错误 | #13 | `PushOutcome` 不再被普遍压缩为 `bool`，调用方能区分跳过、限流、失败和成功 | `src/bin/monitor/notify.rs`、`src/bin/monitor/push_templates.rs` |

## 5. 合规与发布提醒

- 数据相关修复必须继续满足 AGENTS 2.1–2.4：真实来源、缺失显式、坏数据阻断、新鲜度可证明。
- 执行和推送改动必须满足 2.6–2.7：资金安全约束不弱化，来源、时间、决策依据可审计。
- 新增去重、过滤、排序、限制或互斥逻辑前，先登记业务规则 ID（2.10）。
- 本文只完成盘点与排期；任何实现项仍须按 Gate A→D、测试覆盖、合规检查和 PR 证据完成后才能声明 Done。
