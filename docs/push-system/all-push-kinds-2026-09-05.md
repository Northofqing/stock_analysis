# 全量推送审计视图（67 项，2026-09-05）

> PROVISIONAL：由取证脚本从当前 enum 与蓝图 §24.3–§24.6 机械生成，不是新的生产 capability catalog。

阅读顺序：[总报告](comprehensive-reanalysis-2026-09-05.md) → 本清单 → [证据快照](recent-push-evidence-2026-09-05.json)。

“原业务逻辑”是冻结蓝图的被审计断言，不代表本轮将冲突源码逐行验证通过。“校正”优先于原断言；其余仍须在干净源码基线复核可达性。当前 enum 定义行与文件 SHA 可由 JSON 查询；历史代码行号只供追溯，不能当作当前锚点。

原65项状态是 ACTIVE37 / INACTIVE24 / STARVED2 / OPT-IN2；新增PaperBuy/Watchdog均有本地运行记录，但这不证明当前混合工作树就是部署制品。

## 盘前（6项；Watchdog跨时段）

| PushKind / 原状态 | 原业务逻辑及来源 | 本轮校正 |
| --- | --- | --- |
| `AccountMode` / ACTIVE | 启动时评估，08:30 后当日补做；核对持久化旧状态与本次 evaluation，先写审计 pending，再推“旧→新模式、原因、限制、解除条件”。只有 `Pushed` 才把同一行标记完成；失败保留 pending 并重试。Frozen 新迁移还会附带 `MarketActionAlert`。（蓝图源第1133行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `DataMode` / ACTIVE | 启动及常驻 data-mode loop 评估 Full/Degraded/Unsafe；首次 Full 静默建基线，恶化到 Unsafe 立即推，其他抖动需稳定 5 分钟。Degraded 禁盘口判断，Unsafe 再禁价格建议；只有静默建基线或 `Pushed` 才 confirmed。此项跨盘前、竞价、盘中和盘后持续监测，主归属启动/盘前。（蓝图源第1134行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `CandidateTriggered` / INACTIVE | 设计意图为 09:00--09:15 候选转正；实际代码位于“已经等到市场 active”之后却要求 `Closed`，结构无交集。即使人工开关通过，caller 传 `promotion_evidence=None`，且当前无 durable lifecycle transition owner，dispatcher 必须 fail closed。（蓝图源第1137行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `Watchdog` / NOT_IN_65 | 旧65项蓝图未收录；以本轮新增实现和日志为准。（notify.rs第112行） | 新项，主归属盘前/运行健康，跨四阶段；review scheduler 每60秒查 deadline。三个 family 为 news_first_wave/attribution_1505/review_evening，发送无条件 mark_fired，依赖原任务注册。见 F06。 |
| `SnapshotStale` / ACTIVE | 启动时检查用户确认持仓快照；只有落后最近交易日至少 5 个交易日才提醒。`begin` 建 in-flight，只有 `Pushed/Deduped` 才 `finish` 当日完成，失败可重试。不要与 15:05 复用 `IntradayMarket` 的“超过 6 小时”提醒混为一项。（蓝图源第1135行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `PreopenNewsHot` / ACTIVE | P-01 仅交易日 `[09:00,09:15)` 每 30 秒检查；读取上一交易日涨停池 200 条，取产业链前三主线首票并绑定证券身份与逐票新浪新闻；总新闻数为 0 则终态拒绝。先 inspect/resume durable claim，再渲染和 counted send；缺权威终态时即使底层称 Pushed/Deduped 也判失败。（蓝图源第1136行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |

## 集合竞价（7项；Watchdog跨时段）

| PushKind / 原状态 | 原业务逻辑及来源 | 本轮校正 |
| --- | --- | --- |
| `AuctionVolume` / ACTIVE | 09:20 后每个竞价 tick 读取当日涨停池，按量比降序选未通知 Top10；只有 dispatcher 返回 true 才把 code 写入进程内 `auction_vol_notified`，banner/token/sink 失败可在竞价窗口再试。（蓝图源第1145行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `VirtualWatch` / STARVED | Confirm 模式且早盘候选非空、价格全为 0 时初始化；报价按持仓→涨停池→实时行情补齐，正价项先持久化 snapshot 再推，返回值被丢弃且日志无条件写“已推送”。但唯一候选写入来自固定空字符串 `post_close`，当前没有新输入。（蓝图源第1150行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `LimitBoards` / ACTIVE | 只处理存在主力净流数据的涨停股，最多查 40 个板级、排序最多取 50 个；首板/二板/三板分别取 token 并发三张卡，但共享同一 kind 冷却/预算。代码先 `board_notified.insert` 再渲染发送，失败后同日不会重试该票。（蓝图源第1149行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `AuctionRepush` / ACTIVE | 与 CandidateBoard 同一 tick；过滤价格/热度缺失项，Strong 优先、再按热度降序取 Top5，展示首来源、现价、热度。它与 CandidateBoard 都成功才封本轮，否则下一 tick 继续，由各自 cooldown 再防重复。（蓝图源第1146行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `CandidateBoard` / ACTIVE | 合并 P5 文件、持仓和 chain 等候选，输出排序候选台；发送前先做失效 diff、采样并把本轮 code 快照写盘，最后才发送，所以发送失败时 diff 基线已经推进。（蓝图源第1147行） | 当前dispatcher在确认空批时先return，未进入失效diff；非空时又先保存diff基线再发卡。应分别验证“全部消失”和“来源失败”，不能二者混用。见F05。 |
| `PaperTrade` / ACTIVE | 竞价工作流约 09:15--09:20 每 30 秒消费当日已持久化终态；逐条要求合法 A 股、buy/sell、正价、整百股数量、终态及唯一 order audit/hash chain，SQL 精确联接 plan/source/reason。所有行 Pushed/Deduped 才 confirmed。（蓝图源第1151行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `CandidateInvalidated` / ACTIVE | CandidateBoard 将“上一快照存在、本轮消失”的 code 逐票发 T-08，内容是旧状态→Invalidated 与原因；名称可能因本轮已无 entry 回退为 code。它不是独立 scheduler。（蓝图源第1148行） | 失效推送结果被丢弃；本轮候选全空时上层提前return导致不会逐票失效。和CandidateBoard共享completion owner，不应独立切换。见F05。 |

## 盘中（23项；Watchdog跨时段）

| PushKind / 原状态 | 原业务逻辑及来源 | 本轮校正 |
| --- | --- | --- |
| `HoldingEvent` / INACTIVE | 仍有紧急级 renderer/metadata，但 legacy summary 在渲染前停用；LimitUp/LimitDown、主力流入流出、放量、炸板等内存告警因没有 durable lifecycle owner 统一 fail closed，不外发。（蓝图源第1157行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `Announcement` / ACTIVE | news loop 从 EventCalendarGateway 取当日最多 300 条，关键词分类并用概念/名称补 code；受众为自选+24 小时内确认持仓。每个输入经生命周期、分类、受众、dedup/source-fact gate，只有 disposition=Pushed 才进入后续催化。当前配置可让 news loop 全天运行，主归属事件驱动。（蓝图源第1158行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `SectorTop` / ACTIVE | 每小时尝试一次，从 Concept 板块排行生成领涨 Top；构造全局 counted binding。空批按 confirmed empty，投递成功或失败当前都会重置小时 timer。（蓝图源第1159行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `FundInflow` / INACTIVE | enum 仅保留“主力净流入 Top10”元数据；生产源码精确引用扫描没有 dispatcher/caller。当前板块资金展示由 `IntradayMarket`/`SectorTop` 承担，不能把模板名称视为在推。（蓝图源第1161行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `TurnoverTop` / INACTIVE | 有真实板块成分加载、换手率排序 Top10 和 renderer，但没有生产 scheduler/dispatcher 调用；因此是“实现存在、无 producer”。（蓝图源第1162行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `NewsRanked` / INACTIVE | enum 注释和启动审计语义均为 shadow ranker 无生产调用；只剩 level/适配/测试清单引用。（蓝图源第1163行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `HoldingPlan` / ACTIVE | 每 30 分钟基于确认持仓+行情逐票生成；空仓静默，缺行情/非法成本跳过；盈亏 >+5% Reduce、<-3% Add、否则 Hold，并给减仓区/支撑/压力/止损。票级 durable occurrence；Pushed/Deduped 后写跨重启一日一票表，全部确认才推进 timer。（蓝图源第1164行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `T0Advice` / ACTIVE | 每 30 秒到期，按持仓与 Magic TDX 批次生成逐票建议，展示趋势、均价/ATR、量能/五档、卖出/接回区、数量和失效条件；全批 Pushed/Deduped（空批也算）才推进 timer，失败立即保留重试。（蓝图源第1165行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `ForbiddenOps` / INACTIVE | T-09 renderer 可输出结论和多条禁因，durable enum 也保留，但当前告警 producer 因缺生命周期 binding 被 `reject_unbound_alert_delivery` 统一阻断，没有生产发送调用。（蓝图源第1166行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `PaperSell` / ACTIVE | 默认启用，仅 `PAPER_SELL_DISABLED=1` 暂停；盘中每 30 秒随决策 tick、盘后 15:30 再扫，只有真实风险上下文才执行，逐票普通 governor。发送失败只告警，没有 durable counted receipt。（蓝图源第1167行） | 共 408 张卖出卡与 Filled 成交按日期/证券/方向/数量/分价唯一匹配；批处理后才逐条发卡，9/1 最长 1130 秒到发送前日志。不是据数量认定重复。见 F04。 |
| `PaperBuy` / NOT_IN_65 | 旧65项蓝图未收录；以本轮新增实现和日志为准。（notify.rs第110行） | 新项，main.rs:8970 起只遍历 report.fills；report 的 Filled 收集实现位于 decision/intraday_monitor.rs；工作树存在不代表该制品已验证部署。9/4 有 29 张卡及 29 笔唯一匹配 Filled。无持久通知恢复；不补发旧日期买入卡。见 F04。 |
| `CloseCall` / ACTIVE | 14:55 后读取确认持仓，只对相对成本跌幅 ≤-3% 的票生成“尾盘跳水”；逐票 `close-call:{date}:{code}` counted binding。只有所有票 confirmed 才封当天，失败保留重试。（蓝图源第1168行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `IntradayMarket` / ACTIVE | 主路径每 5 分钟读取 Concept 板块 1 日资金流 Top10，真实空推进 timer、取数失败不推进、仅 confirmed delivery 推进。该 kind 还被 09:10 不可达预检和 15:05 快照新鲜度提醒复用，导致三种业务共享冷却/指标。（蓝图源第1169行） | 同kind至少三个producer语义：盘中资金概览、09:10预检、15:05快照提醒；必须分别定义occurrence/completion owner。见 F07。 |
| `NewsCatalyst` / ACTIVE | 公告路由本轮至少一条 Important 才触发；优先最新 board_rotation、校验最多 9 只股票，无 rotation 回退 chain_daily，LLM 为可选增强，失败/空回退主题规则。（蓝图源第1170行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `SectorAnomaly` / ACTIVE | 与 SectorTop 同一小时调度，检测量价反向；读取财联社 20 条并取前 10 标题作归因，新闻失败用空归因文本，非空 moves 才构造全局 counted binding。当前无论成功失败也重置 timer。（蓝图源第1160行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `NewsToIdea` / ACTIVE | 与 NewsCatalyst 共享 Important 新闻触发；从统一候选批取第一名，按 source_count 定阶段、涨幅决定 DoNotChase/BuyDip/Observe，可选 LLM 最多 3 条理由。先确认推送，BuyDip 才模拟买 100 股；虚拟买入失败会使本轮 false 且不写 1 小时 memo。（蓝图源第1171行） | 存在第二条 NewsAI producer：main::news_monitor_loop → news_ai_shadow::schedule_from_same_tick → assessment/delivery event owner。五日 212 条 NewsAI delivered；不能只用 D-01 的 Top1/20min 规则解释。见总报告 F03。 |
| `IndustryChainIntraday` / ACTIVE | 每 15 分钟调用产业链扩散；空来源 confirmed empty，有候选时可用 LLM 生成 trigger，失败回退原 trigger。只有 confirmed 才推进 timer；成功后保存 pushed_stocks，审计写失败会把周期改为 Failed。（蓝图源第1172行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `StPriceLimitChanged` / ACTIVE | 09:30 后一次性检查 ST 持仓，展示 ST 类型、5%→10% 规则参数、成本/现价与新止盈止损；dispatcher 成功才封当天。（蓝图源第1173行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `EtfClosingCallAuction` / INACTIVE | 语义属于 14:57 尾盘集合竞价，不是开盘 Auction；到点后代码直接记录 `disabled=no_etf_auction_producer` 并封当天，不取数、不渲染、不发送。（蓝图源第1174行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `PolicyHit` / INACTIVE | classifier 要求非 research-only、完整 governed evidence、标题/source/发布日期，并允许全局无 code；但生产没有调用 classifier，`SourcePushKind::PolicyHit` 只在测试直接构造。（蓝图源第1175行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `MarketActionAlert` / ACTIVE | 事件路径只从 `MonitorEvent::OrderUpdate` 生成，以 code/action/shares 构 identity，内存状态未变化则跳过；AccountMode 进入 Frozen 时还会额外生成一次。按 Emergency 无条件绕过 launch gate。（蓝图源第1176行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `NewsFlashCritical` / INACTIVE | 虽有专用 typed-receipt 事务和 presentation，但 SourceOnly aggregator 完全不使用 critical threshold，所有合格新闻只进缓冲；测试若产生 N-01 reservation 会 panic，启动也声明无权威 strength provider。（蓝图源第1177行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `NewsFlashAggregated` / ACTIVE | 从不可变 event authority 预检后抓各 feed 20 条；固定在 09:30/11:30/13:00/15:00（首 tick 容差 5 分钟）按 strength 取 Top3。同窗口未决 reservation 只恢复不新建；仅 Accepted 计数，Rejected/Uncertain 精确 settle。（蓝图源第1178行） | 五日8个带typed receipt的Accepted窗口。analytics.ts可能是源事实时间，按receipt.accepted_at重建实际发送时段。不能把08-31 04:23解析成凌晨物理推送。见 F01/F10。 |

## 盘后（31项；Watchdog跨时段）

| PushKind / 原状态 | 原业务逻辑及来源 | 本轮校正 |
| --- | --- | --- |
| `DailyReport` / INACTIVE | R-01/旧收盘汇总有 renderer 和 durable 映射，但收盘主循环因缺 immutable counted/source contract 明确停用；不能把 `FactorIC/SectorTier/CapitalVerify` 的 sub-kind 映射当作 generic DailyReport producer。（蓝图源第1184行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `FactorIC` / INACTIVE | 仅保留 `DailyReportSubKind::FactorIC` 稳定映射；没有生产 producer 构造该 `PushKind` 或完整 immutable binding。（蓝图源第1185行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `SectorTier` / INACTIVE | 与 FactorIC 相同，只是 `DailyReportSubKind::SectorTier` 元数据，没有当前 production caller。（蓝图源第1186行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `CapitalVerify` / INACTIVE | 与 FactorIC 相同，只是 `DailyReportSubKind::CapitalVerify` 元数据，没有当前 production caller。（蓝图源第1187行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `WeeklySOP` / INACTIVE | enum/cooldown 保留一周计划语义，生产调用图没有 renderer dispatcher 或 scheduler。（蓝图源第1188行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `StockPick` / INACTIVE | `stock_pick` 仍是 CandidateBoard 的输入来源枚举，不是 `PushKind::StockPick` 投递；该 kind 没有 producer。（蓝图源第1189行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `IndustryChain` / INACTIVE | R-03 注册为 LegacyAccountGate；当前账户依赖任务统一产生 `AccountMetricsIncomplete`，不会进入 provider/renderer/sink。注意 09:05/15:30 产业链报告走的是独立 NotificationService，不是此 kind。（蓝图源第1190行） | R-03 是固定 LegacyAccountGate，当前无条件返回 account_metrics_incomplete。不能归因于用户今天未补数据，也不能将独立09:05/15:30报告视为该 kind。见 F07。 |
| `AttributionDaily` / ACTIVE | 15:05--15:20 计算持仓报价、日/30 日归因，落 DB 并写 Markdown，再普通 governor 推摘要。代码不论 Pushed/Deduped/Denied/SinkError 都标当天完成，故通知失败不重试。（蓝图源第1191行） | 15:05链已改日K取收盘价，但仍在任何push outcome后推进日完成并satisfy哨兵；报表文件不能替代回执。五日analytics无该类true记录。见 F05。 |
| `G5bAttribution` / ACTIVE | 同一 15:05--15:20 窗口；无告警即封日，有告警但无 LLM 不封；最多分析 `DEEP_ATTRIBUTION_MAX_EVENTS` 条，每条先保存深链结果再推。`done` 统计分析/持久化成功而非确认送达，批次尝试完即封日。（蓝图源第1192行） | 08-31有两份TEST_CODE深链结果和同分钟feishu pushed=true记录；namespace污染已成立，远端实际阅读未证明。top_events_for_deep只排序截断，不做事件聚合。见 F02/F05。 |
| `ReviewMarket` / INACTIVE | R-02 在 13-task catalog 中，但 preflight 明确 Disabled，不调用 provider/renderer/sink。（蓝图源第1193行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `ReviewLhb` / ACTIVE | R-04 自动运行需 21:00（手工可绕过）；取 Eastmoney 龙虎榜 top-five 完整批次，verified-empty 终态 NoData，否则 source-only counted delivery。（蓝图源第1194行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `ReviewSignal` / INACTIVE | R-05 已注册但 preflight 明确 Disabled。（蓝图源第1195行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `ReviewFailure` / INACTIVE | R-06 已注册但 preflight 明确 Disabled。（蓝图源第1196行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `TomorrowWatch` / ACTIVE | R-07 21:00 后聚合 A 档未触发、龙虎榜净买 Top5、涨停链前三、可做 T 持仓；用收盘价派生 ±2% 区间和 -5% 止损，去重后仍必须绑定完整 LHB provider batch。（蓝图源第1197行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `EventCalendar` / ACTIVE | R-08 并发取 CNInfo 公告、CFFEX 交割、海外指数、USD/CNY；允许部分组件降级，但把所有可用批次完整绑定后走 source-only counted gateway。（蓝图源第1198行） | 最近五日259条 dispatcher失败，其中256条invalid_evidence、3条no_verified_batch；R-08 review102条失败均retryable。CFFEX强制依赖失败被字符串化再升级为可重试，非单纯网络抖动。见 F08。 |
| `ReviewProviderTopN` / ACTIVE | R-09 15:35 后运行；并非全市场榜，而是 Eastmoney 单响应中的量比/主力净流入双 TopN。每行严格校验 metric/unit/source ordinal/date/filter/证券身份并保留 declared total/inspected count。（蓝图源第1199行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `PositionReview` / ACTIVE | R-11 要求用户确认账户摘要和 review_date 收盘估值；空仓也可推“无持仓”，有持仓按行业市值 Top5+其他汇总，可选 deep-analyzer 文本不阻塞，最后 counted delivery。（蓝图源第1200行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `ReviewBacktest` / INACTIVE | R-12 虽在 scheduler/durable catalog，但常量 `R12_TECHNICAL_BARS_PUBLISHED=false`，在 loader/provider 前直接 Disabled。（蓝图源第1201行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `WatchlistTracking` / ACTIVE | R-13 次日读取 A-10 watchlist 并核对行情，确认推送后落 outcomes；outcomes 落库失败只 warning，不反转已经送达的状态。（蓝图源第1202行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `CatalystReview` / ACTIVE | A-10 读取可见产业链批，按成员数/连板数/持续性派生 0..100 分和明日观察点，走 counted；成功后保存 T+1 watchlist，保存失败不反转投递。静默期会延期。（蓝图源第1203行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `PostFixedPriceOrder` / INACTIVE | T-14 dispatcher 每 15 分钟的代码入口存在，但全仓没有生产调用 `register_trade_event_source`；每次在 source 边界失败。业务校验要求合法 A 股、正价、整百股、非空 order_id/status。其语义是盘后固定价格，当前却放在盘中分支。（蓝图源第1204行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `PostFixedPriceFill` / INACTIVE | 与 T-14 共用未注册 source，并要求 fill/`next_session_carry`；注释窗口 15:05--15:30，但调用嵌在 Morning/Afternoon 分支，15:00 后已退出，形成 source+schedule 双重不可达。（蓝图源第1205行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `BlockTradeIntradayConfirm` / ACTIVE | 名称称盘中，实际是 19:00 review side route：按自选+持仓查 BlockTradesGateway，300/301/688 只接受协议大宗、实时确认、合法百股数量/正价。（蓝图源第1206行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `BlockTradePriceRange` / ACTIVE | 与上一项同一 19:00 side route；仅 8/4/920 开头北交所证券，要求正当日均价和非空价格区间。（蓝图源第1207行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `PaperReview` / STARVED | A-01 可在 13:00--13:04 午盘和 19:00 review 执行，且午盘无论 bool 成败都封日；它读取 VirtualWatch snapshots。由于当前唯一 virtual_observation 生产写入来自固定空 `post_close`，新快照输入枯竭，只可能处理历史残留。（蓝图源第1208行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `IpoListingApproval` / INACTIVE | enum/presentation 元数据保留，启动明确打印 `disabled=no_producer`。（蓝图源第1209行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `IpoProspectus` / INACTIVE | 与 IpoListingApproval 相同，明确无 producer。（蓝图源第1210行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `IpoCatalyst` / ACTIVE | 盘后 review 的额外 side effect：读取/复用当日公告，按关键词分类和静态供应链表映射，必要时查询板块/成分/证券身份；无公告或 provider failure 静默短路。（蓝图源第1211行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `EarningsBeat` / OPT-IN | 15:00 后 earnings provider 生成归一化 source event，但默认因缺报告期/预测年度/口径 binding 关闭；仅 `EARNINGS_BEAT_ENABLED=1` 放行，禁用告警每 30 分钟节流。（蓝图源第1212行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `EarningsMiss` / OPT-IN | 与 EarningsBeat 共用 provider、校验与默认关闭 gate，仅分类结果为 Miss。（蓝图源第1213行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |
| `AnalystUpgrade` / ACTIVE | 15:00 后按配置轮询评级 provider，状态店追踪；归一化事件要求合法 code、当日发布日期、完整批次 evidence，再走 source-fact gate。（蓝图源第1214行） | 保留原断言作为核验基线；零发送不能自动判故障，metadata/测试引用不能证明生产可达。 |

## enum 外路径与多 producer

09:05/15:30产业链报告、CLI单股/汇总报告均走NotificationService；AlertManager无生产caller证据，不能按第五条已运行消息算量。NewsAI虽复用NewsToIdea，仍有独立分析和完成owner；PaperSell盘中/盘后、IntradayMarket三处以及MarketActionAlert两处也需逐producer核对。参见总报告§3及蓝图§24.7。
