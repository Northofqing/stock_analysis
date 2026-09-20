# gRPC 下游数据缺口与验收交接

日期：2026-09-16。项目根 `R=/Users/zhangzhen/Desktop/Quant/stock_analysis`；源码核对树 `W=R/.worktrees/push-reliability-20260905`。

这是一份问题交接，不是协议版本批准、生产修复、服务重启授权、迁移验收或发布证明。结论把“运行不可达”“上游/协议待确认”“客户端投影丢字段”“业务口径不一致”“普通未配置/账户来源缺失”分开，避免把所有现象归咎于 gRPC 服务端。

## 总结

共 13 项：P0 6 项、P1 7 项。GD-001 有窄范围运行观察；其余主要是读取时源码的静态事实或条件反例。最先要解决的是运行入口、竞价量比、连板资金、Consensus 报告投影、EPS 期间/issuer 和 R03 账户输入；其他项用于防止“接口已通”被误认作“数据合同完整”。

| ID | 优先级 | 阻断业务 | 分类 | 证据强度 | 建议责任方 |
| --- | --- | --- | --- | --- | --- |
| GD-001 | P0 | 所有依赖本机 Local gRPC 的读取 | 运行证据缺失 | 运行观察，限 00:47–00:49 | 上游服务 / 账户连接 / 待确认 |
| GD-002 | P0 | P-02 竞价量能 Top10 | 客户端投影 / 协议待确认 | 直接静态反例 | 协议 / 上游服务 / 本项目转换器 |
| GD-003 | P0 | 连板卡、持仓盘中量比/资金信号 | 客户端投影 / 消费过滤 | 直接静态反例 | 本项目转换器 / 消费业务 / 上游服务 |
| GD-004 | P0 | AnalystUpgrade | 客户端投影丢失 | 无条件静态反例 | 协议 / 本项目转换器 |
| GD-005 | P0 | EarningsBeat/Miss | 消费口径不一致 | 直接静态条件反例 | 协议 / 转换器 / 消费业务 |
| GD-006 | P0 | R03 自动、手动新消息 | 普通账户依赖缺失 | 无条件静态阻断 | 账户连接 / 消费业务 |
| GD-007 | P1 | R09 ProviderTopN、全市场排行 | wire 与本地合同分层 | 直接静态合同差异 | 协议 / 上游服务 / 转换器 |
| GD-008 | P1 | 做 T 持仓 | 已有保护、待运行验收 | 静态保护已核；无本次批次 | 上游服务 / 转换器 / 消费业务 |
| GD-009 | P1 | 盘后 Macro 经济日历 | 请求意图与 wire 差异 | 直接静态合同差异 | 协议 / 转换器 |
| GD-010 | P1 | 盘后模型搜索与报告 | 聚合器丢失败/身份 | 直接静态反例 | 搜索聚合器 / 转换器 / 消费业务 |
| GD-011 | P1 | CloseCall 尾盘提示 | 快照时效与 lineage 丢失 | 直接静态条件反例 | 本项目转换器 / 消费业务 |
| GD-012 | P1 | SectorTop | 批次 lineage 丢失 | 直接静态反例 | 本项目转换器 / 消费业务 |
| GD-013 | P1 | SectorAnomaly | 多源/归因 lineage 丢失 | 直接静态反例 | 本项目转换器 / 消费业务 |

证据强度不等于生产发生频率：静态反例证明代码允许或必然形成该结果，不证明某个交易日已实际漏发。

## GD-001：本机 gRPC 运行能力未认证

- **优先级 / 时段 / 阻断业务**：P0；全时段；阻断所有依赖 `127.0.0.1:18082` Local 路由的健康、能力和业务批次验收。
- **所需字段、时间、单位与身份**：成功取得带协议版本、请求 ID、服务观察时间和明确 ready/capability 状态的 HealthResponse；随后按具体业务取得 provider/source/source_at/observed_at/batch_id、记录集合和单位。
- **wire → 客户端投影 → 消费者差异**：运行 GetHealth 检查命令，连接尝试在 dial 阶段超时；未取得请求已到达服务端或已发送任何业务 RPC 的证据，也没有 response wire。不存在可投影的 `ready=false`、鉴权失败、Capabilities 或业务空批次；监听 socket 也不能投影成 RPC 可用。
- **代码/运行证据**：[运行评估](../docs/push-system/grpc-data-readiness-assessment-2026-09-16.md)记录 PID 761 监听、日志中的 `Too many open files (os error 24)`，以及 session56011/session3904 两次 5 秒 dial deadline。
- **证据强度与边界**：真实运行观察，但仅限 00:47–00:49 的本机入口。未认证原运行制品与当前源码逐字相同；不能外推全部 ExternalV1 独立路由不可用，也不能把 EMFILE 与 SQLite 合法 FD 复用误拒绝合并成同一根因。
- **建议责任方**：上游服务 owner 先查进程资源与请求处理能力；账户连接仅在业务另需账户源时参与；根因待确认。
- **最小改动建议**：另行获得生产操作授权后，先恢复能返回 HealthResponse 的入口，再读 Capabilities；不要通过重启、替换制品或改鉴权猜测来“验证”本交接。
- **验收条件**：同一已认证制品上 GetHealth 返回明确响应；Capabilities 返回 repository admission 与 runtime availability；每个受影响业务至少一份真实批次通过字段、时间、单位、身份和集合门。保留命令、响应原字节/摘要、制品身份与时间。

## GD-002：竞价量比在 TopStock 投影后缺失

- **优先级 / 时段 / 阻断业务**：P0；09:20–09:25 集合竞价；P-02 竞价量能 Top10 无法从当前涨停池投影得到合格行。
- **所需数据**：逐证券有限正 `volume_ratio`（Multiple），其分子、分母、回看区间、竞价 session、交易日和原始 `source_at`；请求证券集合、缺失/额外/重复集合；与涨停池同批或经批准的版本化组合身份。
- **wire → 投影 → 消费差异**：涨停池与名称投影只形成 code/name/change/price，并固定 `volume_ratio=None`；P-02 准备器只接受 `Some(ratio)` 且 finite、`>0`，缺值只累计 rejection，最终可能 `NoEligibleUnnotifiedRows`。这证明当前客户端路径缺字段，不证明服务端从无量比，也不授权从 MarketStatistics/旧 TopN 跨批拼接。
- **代码证据**：W [limit_up.rs:280](../.worktrees/push-reliability-20260905/src/market_analyzer/limit_up.rs#L280)–297；W [push_templates.rs:6037](../.worktrees/push-reliability-20260905/src/bin/monitor/push_templates.rs#L6037)–6108。
- **证据强度**：直接静态反例；未取得真实竞价批次，也未核远端 provider 量比定义。
- **建议责任方**：协议与上游服务确认正式 operation/语义；本项目转换器保留逐票证据；消费业务确认缺一票排除还是整批失败。
- **最小改动建议**：先定义窄的竞价量比合同，再在同一次采集或明确可审计组合中把字段投影到 P-02；不填 `0`，不拿全市场 TopN 代替指定涨停池全集。
- **验收条件**：真实竞价请求包含精确证券集合；返回逐票 ratio、unit、session/source time 和完整集合结果；正常、部分缺失、重复、额外代码、NaN/Inf/非正值均有可执行用例；消息、入池和通知游标继续绑定同一选中快照。

## GD-003：盘中主力净流与量比在部分消费者前丢失

- **优先级 / 时段 / 阻断业务**：P0；盘中；连板首/二/三板卡和持仓旧 detector 分支被阻断，量比相关盘中信号静默。
- **所需数据**：逐证券 `main_net_yi`（亿元，或原始 Yuan 加明确换算）与 `volume_ratio`（Multiple）；交易日、session、source_at/observed_at、provider/source/batch；与涨停池或行情记录的精确 code 集绑定。
- **wire → 投影 → 消费差异**：涨停池投影固定两字段为 None。持仓 detector 对任一缺失直接 `continue`；连板分支先排除 `main_net_yi=None` 再排序。W 的另一盘中路径会调用 MoneyFlows overlay 并只对覆盖到的 code 恢复净流，量比仍以哨兵使对应信号静默；该 overlay 不能证明前述连板/持仓分支已补齐，更不能证明所有上游 MoneyFlows 不可用。
- **代码证据**：W [limit_up.rs:290](../.worktrees/push-reliability-20260905/src/market_analyzer/limit_up.rs#L290)–297；W [main.rs:9781](../.worktrees/push-reliability-20260905/src/bin/monitor/main.rs#L9781)–9815、[main.rs:10094](../.worktrees/push-reliability-20260905/src/bin/monitor/main.rs#L10094)–10116；overlay 的限定范围见 [main.rs:10252](../.worktrees/push-reliability-20260905/src/bin/monitor/main.rs#L10252)–10309。
- **证据强度**：直接静态反例；“所有连板永远为空”只对该投影和直接过滤组合成立，不外推运行制品或其他 overlay 消费路径。
- **建议责任方**：转换器/消费业务先统一数据组合边界；协议/上游服务确认 MoneyFlows 与量比可用集合、单位和时效。
- **最小改动建议**：为需要资金字段的消费者提供同批或经批准组合后的 typed 记录；字段缺失保持显式，不删除过滤，也不以 `0.0` 冒充来源事实。连板、持仓 detector 和 later overlay 应分别接线验收。
- **验收条件**：用同一真实业务时点覆盖“全量、部分缺失、overlay 无此股、批次不一致”；验证逐消费者不会错误排除有证据行，也不会让缺证据行触发资金/量比信号；单位换算和 code 集可重验。

## GD-004：Consensus 最近报告、日期和目标价被转换器清空

- **优先级 / 时段 / 阻断业务**：P0；盘后 HoldingEarnings；AnalystUpgrade producer 静态为零。
- **所需数据**：issuer code、provider canonical report ID、报告标题、机构、发布日期、评级及原始标签、目标价上下界及货币/单位、report_count/broker_count、provider/source/source_at/observed_at/batch_id；请求窗口和 limit。
- **wire → 投影 → 消费差异**：Consensus gateway 把名义 180 日/50 份写入本地 acquisition request hash，但实际 wire 只有请求 code，不能据客户端代码证明服务端采用该窗口。随后 converter 读取汇总计数、EPS 和评级分布，却无条件把两个目标价、latest_report_date 置 None，并把 recent_reports 置空；下游唯一评级循环遍历 `recent_reports`，因此没有观察项。评级分布不能反造逐报告序列。
- **代码证据**：W [gateway consensus.rs:29](../.worktrees/push-reliability-20260905/src/data_gateway/consensus.rs#L29)–43；实际 wire 与完整链见 [业绩/评级追链](../docs/push-system/earnings-analyst-call-chain-2026-09-16.md)；W [convert.rs:2078](../.worktrees/push-reliability-20260905/src/data_gateway/grpc_source/convert.rs#L2078)–2118；domain 明确期望字段见 W [consensus.rs:9](../.worktrees/push-reliability-20260905/src/data_provider/consensus.rs#L9)–39；消费者见 W [v17_sources.rs:1121](../.worktrees/push-reliability-20260905/src/bin/monitor/v17_sources.rs#L1121)–1185。
- **证据强度**：无条件静态反例；未证明远端当前 wire 已包含这些字段。
- **建议责任方**：协议/上游服务先确认可用逐报告合同；本项目转换器负责无损投影；消费业务负责乱序、身份和状态恢复。
- **最小改动建议**：按真实 wire 增加版本化逐报告字段和 issuer 绑定；保留 provider report ID，不继续用 title 作为唯一 proxy；不能只填 `recent_reports` 就宣布评级链修复。
- **验收条件**：真实批次能重证请求 issuer、窗口与每份报告身份；空报告与不可用分开；同机构升级、同档、降级、乱序旧报告、同标题跨日期分别通过；converter 不再主动丢目标价/日期/报告，且发送失败恢复另行验收。

## GD-005：Earnings EPS 比较没有同期间和 issuer 绑定

- **优先级 / 时段 / 阻断业务**：P0 正确性；盘后；EarningsBeat/Miss gate 当前应保持关闭，避免把不可比数字生成交易结论。
- **所需数据**：请求 issuer；实际 EPS 的 report period、NOTICE_DATE、累计/单季口径、币种/每股单位；一致预期的 fiscal year/period、预测口径、窗口、样本报告身份；两批 request/batch/content identity。
- **wire → 投影 → 消费差异**：财务实际 wire 只有 codes/kind，Consensus wire 只有 codes；两者均未在请求中表达报告期、预测年度或累计/单季口径。客户端分别取得数据后，classifier 只要求实际 report_date 属当前年份，然后把 actual EPS 与 `eps_this_year_avg` 相除；没有证明 6 月末累计 EPS 与全年预测可比。现有携带 evidence 也不能仅凭 batch 重证两份数据属于请求 issuer。
- **代码证据**：W [company.rs:35](../.worktrees/push-reliability-20260905/src/data_gateway/company.rs#L35)–52、[gateway consensus.rs:29](../.worktrees/push-reliability-20260905/src/data_gateway/consensus.rs#L29)–43；W [classifier.rs:137](../.worktrees/push-reliability-20260905/src/news/aggregator/classifier.rs#L137)–205；W [v17_sources.rs:1033](../.worktrees/push-reliability-20260905/src/bin/monitor/v17_sources.rs#L1033)–1074 明确 gate 与期间注释；实际 wire 与完整调用链见 [业绩/评级追链](../docs/push-system/earnings-analyst-call-chain-2026-09-16.md)。
- **证据强度**：直接静态条件反例；不是生产误报次数统计。
- **建议责任方**：协议、财务/Consensus 转换器、消费业务共同负责；issuer 与期间是合同字段，不应只在 renderer 修补。
- **最小改动建议**：先建可比较性判定，只有 issuer、fiscal period、累计/单季、单位和预测年度一致才分类；无法确认时 typed unavailable/skip，保持 gate 关闭。
- **验收条件**：至少覆盖同年同期间、同年不同期间、跨年、累计对单季、issuer 错配、缺单位/缺预测年度；只有可比组合产生 Beat/Miss，边界阈值按现规则保留，失败不推进成“已完成分类”。

## GD-006：R03 缺的不是行情，而是实时账户输入

- **优先级 / 时段 / 阻断业务**：P0；盘后；R03 自动和手动新消息入口均无法进入 provider、renderer 或 sink。
- **所需数据**：完整账户持仓/资产指标，账户/用户身份、快照 ID/evidence hash、effective_at/confirmed_at/observed_at、币种与金额单位、完整/部分/确认空状态；行情批次只能作为独立市场输入。
- **wire → 投影 → 消费差异**：R03 被分为 `LegacyAccountGate`；当前批次实现对全部 account_required 任务直接生成 `AccountMetricsIncomplete`，不调用 R03 来源。历史截图或用户快照入库不自动成为实时账户 gRPC 证据；改写截图时间也不能恢复。
- **代码证据**：W [review_batch.rs:1138](../.worktrees/push-reliability-20260905/src/bin/monitor/review_batch.rs#L1138)–1163；W [push_templates.rs:10072](../.worktrees/push-reliability-20260905/src/bin/monitor/push_templates.rs#L10072)–10096；入口边界见 [R03 调用链](../.worktrees/push-reliability-20260905/docs/push-system/review-r03-call-chain-2026-09-14.md)。
- **证据强度**：无条件静态阻断；不代表既存 immutable envelope 的启动恢复不可用。
- **建议责任方**：账户连接与消费业务；市场数据服务不应承担账户事实。
- **最小改动建议**：接入真实、可验证的账户批次后替换无条件失败，保留缺失/过期/部分输入拒绝；分别保持 auto、manual 和 stored-recovery 的完成权。
- **验收条件**：真实账户可用、缺失、陈旧、部分、确认空分别有 typed 结果；auto/manual 能在合法批次下走实际 dispatcher；stored-recovery 不重取账户、不创建新分析；Terminal 与 Delivered 仍严格区分。

## GD-007：ProviderTopN 的本地合同不是实际远端请求字段

- **优先级 / 时段 / 阻断业务**：P1；盘后 R09 与全市场排行能力声明；风险是把本地 limit/filter evidence 误写为服务端实际接收。
- **所需数据**：metric=`VolumeRatio`/`MainNetInflow`、交易日、limit=20、A 股 filter、排序 ordinal、单位（Multiple/Yuan）、provider/source/batch identity；若全市场能力另行开放，还需完整 universe 与截断语义。
- **wire → 投影 → 消费差异**：R09 实际只发一次 `ProviderTopNRankings {date}`；客户端在本地为两 metric 构造 limit/filter request hash，再从 response 拆两批并校验行。该校验可以证明返回满足客户端口径，却不能证明远端收到了 limit/filter 字段。独立的 full-market ranking 路径仍明确标记 `provider_capability_not_live_admitted`，不能拿 R09 Top20 冒充全市场榜。
- **代码证据**：W [grpc_source.rs:3568](../.worktrees/push-reliability-20260905/src/data_gateway/grpc_source.rs#L3568)–3588；W [capital.rs:171](../.worktrees/push-reliability-20260905/src/data_gateway/capital.rs#L171)–214、[capital.rs:319](../.worktrees/push-reliability-20260905/src/data_gateway/capital.rs#L319)–345；W [market_data.rs:465](../.worktrees/push-reliability-20260905/src/bin/monitor/market_data.rs#L465)–480。
- **证据强度**：直接静态合同差异；没有本次真实 R09 response。
- **建议责任方**：协议/上游服务决定显式 params 或版本化默认；转换器继续校验两侧批次；消费业务保持 TopN 与全市场合同独立。
- **最小改动建议**：优先让实际 wire 明确 metric/limit/filter，或把默认规则绑定到不可歧义的协议/schema 版本并在 response 回显有效请求；不改本地 hash 伪装成远端回执。
- **验收条件**：抓取并解码真实请求/响应；两 metric、limit、filter、date、unit、ordinal 和两个不同 batch ID 可重验；一侧空、缺 metric、顺序错、额外行均显式失败；全市场能力单独验收，不以 Top20 通过替代。

## GD-008：T0 现有时间政策与请求完整性需要运行验收，不是待删除限制

- **优先级 / 时段 / 阻断业务**：P1；盘中做 T；当前缺的是本次真实批次和来源质量证明，不是已发现的静态“请求集合漏检”。
- **所需数据**：精确持仓 code 集；每个 code 恰好一个 record 或 rejection；requested_at/source_at/observed_at、provider/source/batch_id；盘口、日线、完成 5 分钟线、分时均价及单位；`time_untrustworthy` 必须端到端保留。
- **wire → 投影 → 消费差异**：客户端发送显式 codes，并验证 records+rejections 的 outcome 集与请求集完全相等、无重复。T0 专用转换器对 source age 超五秒只置 `time_untrustworthy=true`，但未来时间、observed_at 未来、source_at>observed_at、缺 source_at 仍硬错；消费正文加入“时间不可信”警示。普通实时能力仍走严格 age 门。不能把 `t0_evidence.rs` 的严格 helper 泛化为当前 T0 gRPC 消费规则。
- **代码证据**：W [grpc_source.rs:3746](../.worktrees/push-reliability-20260905/src/data_gateway/grpc_source.rs#L3746)–3782；W [convert.rs:447](../.worktrees/push-reliability-20260905/src/data_gateway/grpc_source/convert.rs#L447)–532、[convert.rs:3784](../.worktrees/push-reliability-20260905/src/data_gateway/grpc_source/convert.rs#L3784)–3903；W [main.rs:8259](../.worktrees/push-reliability-20260905/src/bin/monitor/main.rs#L8259)–8319。严格 helper 只作口径对照：[t0_evidence.rs:168](../.worktrees/push-reliability-20260905/src/data_gateway/t0_evidence.rs#L168)。
- **证据强度**：静态保护已核；本轮仅运行健康检查，没有调用 T0 业务批次。超龄标注是已有明确政策，不在本交接中判为新缺陷。
- **建议责任方**：上游服务提供真实 source time 与完整结果；转换器/消费业务保持现有 typed 校验和警示。
- **最小改动建议**：不改写 `source_at`，不以接收时间代替，不未经批准收紧或继续放宽；仅补足可运行验收、时间字段传递和异常批次测试。
- **验收条件**：真实非空 codes 请求逐项返回 record/rejection；缺失、重复、额外代码硬拒；仅当服务端提供的批级 `time_untrustworthy=false`，且客户端按同一 consumer `now` 复算的批级与每条 record age 均≤5s 时最终不标注；服务端批级标记为 true 时即使 age≤5s 也必须保留，任一批级/record age>5s 必须汇总为 true 且正文保留；当前合同没有 record 自带的 `time_untrustworthy` 字段，不得为验收虚构该字段。未来/倒挂/缺时仍硬拒；普通实时路径继续严格；记录原始时间而非本机覆盖值。

## GD-009：EconomicCalendar 业务意图与实际 `{}` wire 分离

- **优先级 / 时段 / 阻断业务**：P1；盘后 Macro；风险是恢复/审计声称“请求 20 条、country=None”，但原 RPC 字节只表达默认请求。
- **所需数据**：limit=20、country=None 或明确国家、业务观察时刻、协议/schema 默认版本、provider/source/source_at/observed_at/batch_id、实际返回集合。
- **wire → 投影 → 消费差异**：持久计划保存 `economic_limit=20`、`economic_country=None`；`MacroQueryIdentity::EconomicCalendar` 没有参数，payload 和 codec 期望都是 `{}`。因此 wire 只证明使用当时的服务默认，不能单独证明远端按 20/None 执行。
- **代码证据**：W [macro_attempt.rs:13](../.worktrees/push-reliability-20260905/src/grpc_client/macro_attempt.rs#L13)–50；W [chain_post_close_macro_codec.rs:176](../.worktrees/push-reliability-20260905/src/push_foundation/intent_store/chain_post_close_macro_codec.rs#L176)–189、[chain_post_close_macro_codec.rs:368](../.worktrees/push-reliability-20260905/src/push_foundation/intent_store/chain_post_close_macro_codec.rs#L368)–382、[chain_post_close_macro_codec.rs:561](../.worktrees/push-reliability-20260905/src/push_foundation/intent_store/chain_post_close_macro_codec.rs#L561)–562。
- **证据强度**：直接静态合同差异；不是远端返回错误的运行实证。
- **建议责任方**：协议与转换器；由协议 owner 决定显式 params 还是版本化默认，不由恢复层猜测。
- **最小改动建议**：让持久 intent、编码后的 QueryRequest 和服务端有效请求三者表达同一事实；若保留 `{}`，持久化并校验默认规则版本，避免把本地字段冒充 wire 字段。显式参数或默认规则版本只是后续协议选项，不阻断当前 Macro 恢复任务按原合同分别保存本地意图与实际 `{}` wire；当前任务不改变现行请求，也不重写旧事实。
- **验收条件**：解码保存的真实 QueryRequest 可唯一推出 limit/country；服务端 response/audit 回显或绑定有效请求；旧 `{}` 事实按原版本只读，不追溯改写；不同默认版本不复用同一 request identity。

## GD-010：搜索聚合丢原始 provider 失败和调用身份

- **优先级 / 时段 / 阻断业务**：P1；盘后模型搜索与首次报告；空背景无法区分所有 provider 真空、超时、返回 `success=false`、不支持 topic 或未配置。
- **所需数据**：外层 query/limit/stage/ordinal；扩展 query；每个 provider 的 identity、调用序号、timeout、success/error、原始返回或其稳定摘要、结果来源/evidence；聚合和 rerank 所用历史快照身份。
- **wire → 投影 → 消费差异**：`SearchService::search_topic` 对每个扩展 query/provider 调用；timeout、`success=false`、error 和空结果都 `continue`，全空返回 Vec。ProductionIo 又把 Vec 无条件包 `Ok`；外层只把空 Vec 标成 `Unknown`。`is_available` 仅检查任一 provider 可用，早于 topic 支持过滤。数据库历史读写失败也被折叠，结果排序可能漂移却无效果记录。
- **代码证据**：W [service.rs:605](../.worktrees/push-reliability-20260905/src/search_service/service.rs#L605)–607、[service.rs:708](../.worktrees/push-reliability-20260905/src/search_service/service.rs#L708)–773、[service.rs:941](../.worktrees/push-reliability-20260905/src/search_service/service.rs#L941)–1004；W [preparation.rs:1040](../.worktrees/push-reliability-20260905/src/pipeline/chain_analysis/preparation.rs#L1040)–1047、[preparation.rs:1742](../.worktrees/push-reliability-20260905/src/pipeline/chain_analysis/preparation.rs#L1742)–1796。
- **证据强度**：直接静态反例；未运行任何真实搜索/provider。
- **建议责任方**：搜索聚合器提供 typed 逐尝试结果；转换器/盘后消费保存实际序列和最终聚合，不扩张为新的远端身份平台。
- **最小改动建议**：在真实 provider 调用 seam 保存 begin/result 与 provider identity；聚合结果显式区分 VerifiedEmpty、Unavailable、NotConfigured、Partial 和 Unknown。保持外层业务接口，恢复时不重跑搜索或历史写入。
- **验收条件**：用一个成功、一个超时、一个 `success=false`、一个空结果的受控组合验证逐尝试记录；全空不再伪装成普通 `Ok(empty)`；关闭重开后 query、provider 序列、原结果和最终排序逐字节复用，不新增 provider 调用。

## GD-011：CloseCall 快照时效和两次读取破坏输入绑定

- **优先级 / 时段 / 阻断业务**：P1 正确性；尾盘 CloseCall；可能用过期至 24h59m 或未来 effective_at 的持仓 code 集取行情，也可能首读快照与二次读取的 code 集不一致。
- **所需数据**：唯一用户快照 `snapshot_id/evidence_sha256/effective_at/confirmed_at/source`、完整持仓项；精确行情请求 code 集与 BatchEvidence；决策 observed_at；价格/成本/盈亏单位和 instrument identity。
- **wire → 投影 → 消费差异**：`prepare_close_call_messages` 先读一次快照用于成本和遍历，随后 `fetch_position_quotes` 再读“最新快照”决定请求 codes。该 helper 用 `signed_duration_since(...).num_hours() <= 24`：整数小时截断允许 24h59m；没有 `age>=0`，未来快照也通过。行情内部其实保留 `TopStockBatch.evidence`，但 helper 返回 records-only；最终 canonical 只有 code/price/cost/pnl_pct/本机当前时间，缺快照与 quote batch lineage。
- **代码证据**：W [main.rs:8462](../.worktrees/push-reliability-20260905/src/bin/monitor/main.rs#L8462)–8507；W [market_data.rs:175](../.worktrees/push-reliability-20260905/src/bin/monitor/market_data.rs#L175)–216、[market_data.rs:237](../.worktrees/push-reliability-20260905/src/bin/monitor/market_data.rs#L237)–305；快照可用身份见 W [user_position_snapshot.rs:53](../.worktrees/push-reliability-20260905/src/database/user_position_snapshot.rs#L53)–63。
- **证据强度**：直接静态条件反例；没有证明数据库当前存在未来或 24h59m 快照，也没有生产错配观测。
- **建议责任方**：本项目转换器与 CloseCall 消费业务；不是要求行情服务保存账户快照。
- **最小改动建议**：一次读取并固定用户快照；按精确 duration 验证 `0 <= age <= 24h`；用该快照 codes 获取 evidence-preserving quote batch，并把两者身份纳入 immutable decision。
- **验收条件**：边界覆盖 23:59:59、24:00:00、24:00:01、24:59:59 和未来 1 秒；模拟两次读取间新增快照不能改变请求/成本配对；canonical 能重证 snapshot 和 quote batch，缺行情 code 明确隔离而非静默拼错。

## GD-012：SectorTop 丢板块批次 lineage

- **优先级 / 时段 / 阻断业务**：P1 审计/恢复；盘中 SectorTop；展示值可能正确，但 immutable decision 无法证明来自哪次 gRPC 板块批次。
- **所需数据**：fid=`f3`、top_n、排序/截断语义；每行 board code/name/change_pct/main_inflow（Yuan，展示转亿元）及其他实际准入字段；provider/source/source_at/observed_at/batch_id、请求 hash。
- **wire → 投影 → 消费差异**：`BoardRankingGateway::fetch_top` 从 GatewayBatch 只返回 `records().to_vec()`，先丢 BatchEvidence；dispatcher 再投影为 name/change/inflow；counted canonical 只保存展示三字段和 `Local::now()`，没有 board code、fid/top_n 或来源批次。
- **代码证据**：W [board_ranking.rs:28](../.worktrees/push-reliability-20260905/src/data_gateway/board_ranking.rs#L28)–59；W [sector_monitor.rs:215](../.worktrees/push-reliability-20260905/src/market_analyzer/sector_monitor.rs#L215)–241；W [push_templates.rs:16748](../.worktrees/push-reliability-20260905/src/bin/monitor/push_templates.rs#L16748)–16811。
- **证据强度**：直接静态反例；未验证真实板块 provider response。
- **建议责任方**：Gateway/转换器保留 batch；消费业务把真实请求和投影身份写入 decision。
- **最小改动建议**：提供 evidence-preserving `BoardRankingBatch`，SectorTop 沿同一批次排序/渲染/持久化；不要用本机当前时间代替 provider source time。
- **验收条件**：真实 f3 TopN 的请求、顺序、单位和批次可重验；关闭重开复用同一 decision，不重取榜单；batch evidence 缺失/错配、board code 重复、额外/乱序均显式拒绝。

## GD-013：SectorAnomaly 丢两榜与新闻归因身份

- **优先级 / 时段 / 阻断业务**：P1 审计/恢复；盘中 SectorAnomaly；无法从最终 decision 重证“异动但无新闻归因”的输入。
- **所需数据**：f3 与 f62 两份请求/批次及有序行；合并规则；board code、change、volume ratio、资金加速度及触发 reasons；`news_text` 的来源批次/内容 hash/观察窗；配置阈值和业务时点。
- **wire → 投影 → 消费差异**：检测器分别拉 f3/f62，合并 code 后用 news_text 和三类阈值分类；上游 Gateway 已在 `fetch_top` 丢 batch。最终 counted canonical 只写 board name/change/main_inflow 和本机时间，既不含两榜身份、board code、volume ratio/加速度/reasons，也不含新闻文本/来源身份。
- **代码证据**：W [sector_monitor.rs:698](../.worktrees/push-reliability-20260905/src/market_analyzer/sector_monitor.rs#L698)–704、[sector_monitor.rs:760](../.worktrees/push-reliability-20260905/src/market_analyzer/sector_monitor.rs#L760)–855；W [push_templates.rs:14503](../.worktrees/push-reliability-20260905/src/bin/monitor/push_templates.rs#L14503)–14548、[push_templates.rs:16842](../.worktrees/push-reliability-20260905/src/bin/monitor/push_templates.rs#L16842)–16874。
- **证据强度**：直接静态反例；未证明生产已发出无法追溯的消息。
- **建议责任方**：Gateway/转换器保留两份 batch；消费业务保存实际归因输入与配置，不要求新造上游字段。
- **最小改动建议**：构造版本化 `SectorAnomalyInput`，包含两榜原身份、实际合并投影、news content hash/source、阈值和 reasons；从同一输入渲染与完成，不用 Local now 覆盖 source time。
- **验收条件**：两榜一成一败、批次日期不同、同 code 冲突、新闻为空/Unavailable/有覆盖、三种 reason 组合均可执行；重开只读复用首次输入；最终 canonical 可重算相同 moves 和正文。

## 跨问题验收顺序

1. 在单独授权下恢复 GD-001 的可应答健康与能力读取；在此之前所有静态修复都不能称为生产数据满足。
2. 以真实合同确认字段和请求意图：GD-002/003/004/005/007/009。先确定 provider 能给什么，再改协议或转换器。
3. 保持来源类型分离：GD-006 的账户事实不从行情补造；GD-010 的 NotConfigured/Unavailable/VerifiedEmpty 不折叠；GD-008 的显式超龄政策不擅改。
4. 补齐下游 immutable lineage：GD-011/012/013，确保请求、原批次、客户端投影、业务决策和恢复引用同一事实。
5. 每项分别做真实批次、失败分类、关闭重开和业务消费者验收；不要用“返回 0 条”“有监听端口”“已有展示文本”代替。

## 未验证边界

- 健康检查没有取得 HealthResponse 或 Capabilities；本轮没有调用 Consensus、ProviderTopN、T0、Economic、板块榜或搜索业务批次。
- 没有读取生产 token、env、账户凭据、数据库内容或实际渠道配置；没有调用 provider/model/通知，也没有启动、停止或重启服务。
- 没有证明 ExternalV1 全部路由不可用；没有证明当前运行二进制等于 R 或 W 源码。
- 没有证明旧 8 月问题今天仍发生；历史文档只用于定位，经本次实际源码行重新核对后才写入。
- T0 超龄标注是现行明确政策，不是本次要求收紧的缺陷；全市场排行与 R09 TopN 是两个合同；EMFILE 与 SQLite FD 误拒绝根因未合并。
- 这些文档不增加已迁移 Unit 数，不批准新协议版本，也不授权生产修复。

## 读取快照

下表是实际读取版本；`R` 为原项目，`W` 为隔离开发树。共享源码后续可能移动，行号与 SHA 必须一起使用。

### 公开输入文档（R）

| 文件 | 行数 | SHA-256 |
| --- | ---: | --- |
| `docs/push-system/grpc-data-readiness-assessment-2026-09-16.md` | 43 | `d236895de23f54616263e4a436df293218700c8469807bb93f8dd7a50783c1be` |
| `docs/push-system/earnings-analyst-call-chain-2026-09-16.md` | 92 | `a8aa57c92c658bc25098420e8d1bf234cd40efcd48a9d2b34de9806917e83730` |
| `docs/push-system/chain-macro-full-implementation-2026-09-15.md` | 58 | `e327260b34cfb7e1c02f89bfecfea165f8a515f18895adb421a79faa9c0f2f3e` |
| `docs/push-system/chain-models-search-recovery-preparation-2026-09-16.md` | 68 | `0a6d155e5da44d87f0ace5315aea99cd40e906a2ae72940974bb8728f5c567dc` |

### 直接引用源码（W）

| 文件 | 行数 | SHA-256 |
| --- | ---: | --- |
| `src/market_analyzer/limit_up.rs` | 1351 | `60bd9205195f2bf9380b641bf62c03b4bf490ccf7aad4d81153fe75b7e6b495d` |
| `src/bin/monitor/push_templates.rs` | 23450 | `5096e5697f67338b45346d11efa8f9ec988774c17a8f9623b280adeaeb2044f4` |
| `src/bin/monitor/main.rs` | 12747 | `97cebc050729c5b85988c565a294e542a927111d611aeeb80bb46104dc7668f2` |
| `src/data_gateway/grpc_source/convert.rs` | 6862 | `d1bef9d1ed38d5c97e9a59577f9d48bbee84a44a0bdab1bd2e282e51ee82b375` |
| `src/data_gateway/consensus.rs` | 57 | `77d05ba634587c04e6efc373f4c909ab69b317c80844a132aeb061c0494ab4ed` |
| `src/data_gateway/company.rs` | 139 | `31fb05466d46f967a63678361871fdc5b3906e65517a391092fd59b0affbffa2` |
| `src/data_provider/consensus.rs` | 92 | `61d5ea20251b66defd30f8f916216617e21bb9a7579bf69c0e3ea2810d877496` |
| `src/bin/monitor/v17_sources.rs` | 2145 | `65b9b28fab08edc8d2d0ce237d8e2610afa5c17d1728e42966f3bfb0297c8cc0` |
| `src/news/aggregator/classifier.rs` | 652 | `d784d6baee6244a151c032279e8a270b710a04f2d699b17b7e8d36895e911dc1` |
| `src/bin/monitor/review_batch.rs` | 3442 | `8006c7bc6143bd410087b832c81636bc81bb1c047709fd0b0eb100ee77a56b2c` |
| `src/data_gateway/capital.rs` | 570 | `4b803f60bbabe3f67725f0eaf63ceffbe7ef1e1f1220d9d0cb764129632616d0` |
| `src/data_gateway/grpc_source.rs` | 5762 | `4c5a162c4486022b9da0e53d8f32cf5e30f1d392be04ce1f4125bcacaafcf142` |
| `src/grpc_client/macro_attempt.rs` | 453 | `4f3ed5f801124ce37af7f744629d34c87f94526fbb442bf6a7724f54d0cd55f7` |
| `src/push_foundation/intent_store/chain_post_close_macro_codec.rs` | 1273 | `a1317b7c5a2068b03478596812fa93017401426cf76b40a27dd580f61eab4eb8` |
| `src/search_service/service.rs` | 2290 | `c4b6e89c2b5508adaf7f691a04426adf1b49dd568a35c7d6a1ed041bca97b770` |
| `src/pipeline/chain_analysis/preparation.rs` | 1838 | `2fc9e1f329a4d5f07624681da875a6b66bc428bec94f197cdb0edfe8843e800f` |
| `src/bin/monitor/market_data.rs` | 792 | `c110ee3f1e7693d124404d1960bed34f0ac12b2e1ae089130cf52fc0f60b68f5` |
| `src/database/user_position_snapshot.rs` | 258 | `660042ad01eeaad33f08558157de45d4d380502df23b530ae7a12d4db10882eb` |
| `src/market_analyzer/sector_monitor.rs` | 1115 | `474791048cc8d43b2d2953ed875d8b43fdadad9072bbe33454882d876b7e3ba6` |
| `src/data_gateway/board_ranking.rs` | 66 | `1dc93d58411e9c0e6daef162d8a8fa507b7daaa5332016c41dfb3b3aff981b16` |
| `src/data_gateway/t0_evidence.rs` | 535 | `2a43c6b653b810acef37ae2ecc0634c78592afd68abcf0b94df2429e396a706e` |

`src/search_service/types.rs` 被读取用于确认 SearchResult 含 `source/evidence`，但问题证据已由 service/preparation 的实际丢失路径充分表达；其读取版本为 712 行、SHA `6525894f2ef15af39fcb07337ef2f18a9d25c2ae47b36891c17126026d09383d`。
