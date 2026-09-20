# 推送系统全量再分析：代码、蓝图、最近五个交易日

> 状态：PROVISIONAL / 分析报告，不是已实施方案或生产验收书。  
> 日期：2026-09-05；时区：Asia/Shanghai。  
> 主样本：2026-08-31 至 2026-09-04；09-05 仅作周末运行对照。  
> 取证快照：2026-09-05 08:57:02–08:57:10；三个运行数据库分别只读查询，不构成跨库原子快照。  
> 代码：HEAD `a673043`，另有未合并工作树；采集时 160 个 unmerged 路径。

## 1. 结论与阅读入口

整体方向应当保留，但现有方案不能直接按“实施就绪”开工。问题不是单纯消息太多或模板不好看，而是五件事叠在一起：

1. **统计事实不统一。** 业务日期、源数据时间、发送时间混用；布尔成功、远端 Accepted、业务完成混用。
2. **真实业务事件与通知事件混用。** NewsAI 的采集批次可形成新评估/投递身份；模拟成交已经入账，通知却只依赖本次返回的内存列表。
3. **完成条件不统一。** 有的发送失败仍封日，有的无数据仍记失败，有的永久依赖缺口反复重试。
4. **监测依赖被监测对象。** 新哨兵存在，但注册、检查和发送仍有共同故障点。
5. **文档已经落后于变化。** 当前 enum 是 67 项，蓝图仍写 65；NewsToIdea 的多 producer、PaperBuy、Watchdog 没有完整进入既有实施边界。

保留“Foundation → 原子迁移单元 → 清理”的路线；先补事实、身份、回执和完成语义，再单独做摘要/优先级/模板体验。不要重写已经有较强证据的 durable/P01/N02 状态机，也不要为这次重构引入微服务、Redis/Kafka 或第三套投递 authority。

配套文件：

- [67 项四时段全量审计视图](all-push-kinds-2026-09-05.md)：保留每项原业务逻辑，明确本轮校正与尚未证明的部分。
- [机器可读证据快照](recent-push-evidence-2026-09-05.json)：SQL、分日数量、回执标识、成交匹配、源码 SHA、设计来源状态。
- [只读取证脚本](reanalyse-recent-pushes.rb)：可重跑；不运行 monitor，不调用 provider/sink，不写数据库。
- [108 项已批准决策](grill-decisions-2026-09-02.md)：本报告不改写历史答案。
- [原架构 HTML](../Project_Architecture_Blueprint.html) / [Markdown](../Project_Architecture_Blueprint.md)：被审计的输入，不是本轮已修订后的规范。

本轮完成的是重新分析和文档交付。既有[文档硬化计划](push-documentation-hardening-plan.md)中的完整 RFC、正式 capability catalog、symbol manifest、双离线 HTML 和 CI 门禁，仍不能声称已经完成。

## 2. 证据口径：先避免得出错误结论

### 2.1 四种事实不能互相替代

| 层次 | 本轮来源 | 能证明 | 不能证明 |
| --- | --- | --- | --- |
| 当前源码 | HEAD / 工作树 / 文件 SHA | 某分支或合同存在 | 正在运行的 binary 一定来自这些字节 |
| 业务事实 | paper_trades、NewsAI assessment、账户/持仓快照 | 成交模拟、分析、输入已持久化 | 通知已被远端接受 |
| 本地通知声明 | push_analytics.pushed、发送前 Markdown | 本地路径记了成功/准备发送 | 完整远端回执、用户看到了、一次且仅一次 |
| 较强投递证据 | durable sink_results、N02 event authority 内 typed receipt | 本地留存了可定位的远端 Accepted 结果 | 用户已读；本轮未重新验证整条历史审计链的密码学完整性 |

本轮没有查询飞书服务端收件箱，也没有主动发送测试消息。不存在“已核对全部用户已收消息”的证据。

### 2.2 对旧统计和旧分析的修正

- 不能把“legacy pushed=true + durable Delivered”直接叫“用户可见消息数”。本报告使用“本地成功声明/Accepted 记录”的口径。
- `push_analytics.pushed=false` 包含去重、治理拒绝、未发送和取数问题，不是统一的发送失败。
- `RejectedDurable` 是决策终态，不一定产生过 sink 尝试；本期 443 个该终态不能叫 443 次远端发送失败。
- 08-31 发生三条历史业务日补推：08-26 的 PositionReview，08-28 的 TomorrowWatch 和 PositionReview。它们计入 08-31 的 Accepted 时间统计，但不应计入 08-31 的业务 occurrence 数。
- N02 的 analytics 时间不是可靠发送时间。例如其一行记在 08-31 04:23，而权威审计的 09:30 窗口 Accepted 在 09:34:10。不能据 analytics 时间推断凌晨乱发。
- 原“779 条”覆盖较早截点、混用了证据强度和日期口径，不再作为本轮基线。

证据：JSON 的 `queries`、`daily`、`durable_results`、`news_flash_typed_receipts`；N02 权威文件 [2026.jsonl](../../data/event_audit/2026.jsonl) 第 4398/4461/4503 行；时间取自 `news_flash_remote_receipt.accepted_at`。

### 2.3 蓝图与源码基线

| 项目 | 重新核验结果 | 影响 |
| --- | --- | --- |
| HTML 内嵌 Markdown | 与磁盘 Markdown 字节相同；MD SHA `a1acf98ec960880934285d1a71ecf6fba068d809b08174ab51871a91645f2f75` | HTML/MD 没有文本同步漂移，但两者可以一起过时 |
| HTML 离线性 | Mermaid 仍引用 jsDelivr CDN | 文本自包含不等于完全离线渲染 |
| 从零重建 | renderer 先读取既有 HTML 当模板 | 不满足 Q60/Q68 的 Markdown+独立模板+本地资源从零构建 |
| enum | 67；原 65 + PaperBuy + Watchdog | 固定 65 的新验收会漏项 |
| presentation | 58 tuples / 54 unique kinds | 当前 13 个 kind 无 registration；不代表都不可发送，存在直接 governor 路径 |
| durable counted | 仍是 23-kind 的独立目录 | 与 monitor enum 不是同一集合，不能据此自动开启全部 producer |
| 源码状态 | 160 个未合并路径 | 只能发布临时审计，不能声称编译、默认并行测试或部署已通过 |

证据：[notify::PushKind](../../src/bin/monitor/notify.rs#L45)、[presentation registry](../../src/bin/monitor/presentation_registry.rs#L42)、[durable catalog](../../src/durable_delivery/model.rs#L169)、[renderer](../../scripts/render-architecture-blueprint-html.rb#L258)。完整路径/哈希/冲突标识在 JSON `code_sources`。

“逐行证据”的范围必须诚实：本轮覆盖全量 enum、原四时段业务断言、关键调用/状态写入链和最近运行样本；**没有把 160 个冲突文件的混合文本当作可执行程序逐行认证**。全量视图中的原业务说明是审计输入，校正优先；剩余原断言须在干净基线继续验证。引用扫描结果也明确标为 locator，可能含 metadata/测试，不能伪装成生产 caller 清单。

## 3. 重新还原运行架构与业务分类

### 3.1 当前实际不是一条“统一推送管道”

```text
数据 Gateway / 业务账本 / 告警文件
  ├─ counted producers → durable coordinator → typed sink → durable audit/终态
  ├─ P01 → 专用盘前 occurrence/恢复 → counted authority
  ├─ N02 → 独立 reservation/attempt/settlement → typed sink → event authority
  ├─ NewsAI → assessment/delivery event 状态 → 带尝试标记的 sink → 本地完成记录
  ├─ 普通 governor → CLI/HTTP/L6 → bool/Pushed/Deduped → analytics/观察记录
  └─ CLI/产业链报告 → NotificationService 多通道 → bool/部分成功
```

蓝图 §12 的强投递序列适用于相应 counted 路径，不能扩展成所有消息已经拥有该保证。§13 把 L4/L5→durable→L6 画成统一主干，也会掩盖普通 governor、N02、NewsAI 和 NotificationService 的不同所有者。

对外数据仍应由 Data Gateway 准入；provider-host 在仓外。保留业务 DB / durable DB 的所有权隔离。EventBus、JSONL 观察投影不变成队列或回执 authority。v18 的投资决策 ID、paper 成交 ID、推送 decision ID 必须分开引用。

证据：[generic governor 与投递](../../src/bin/monitor/notify.rs#L2246)、[NewsAI sink preflight](../../src/bin/monitor/notify.rs#L2581)、[NewsAI producer](../../src/bin/monitor/news_ai_shadow.rs#L237)、[N02 调度](../../src/bin/monitor/main.rs#L7813)、[蓝图 §12–§14](../Project_Architecture_Blueprint.md#12-持久化投递与权威审计)。

### 3.2 四时段用于看业务，不用于划分原子所有权

| 主归属 | 本轮 enum 视图 | 主要职责 | 不能遗漏的跨时段/独立入口 |
| --- | ---: | --- | --- |
| 盘前 | 6 | 账户/数据模式、快照提醒、P01、候选转正保留项、运行健康哨兵 | DataMode/Watchdog 全天；09:05 产业链报告在 enum 外 |
| 集合竞价 | 7 | 竞价量能、候选板/失效、优选重推、连板、虚拟观察、PaperTrade | 09:25–09:30 是间隙；LimitBoards 实际也进入盘中；PaperTrade 不等于新增 PaperBuy |
| 盘中 | 23 | 持仓计划/T0/尾盘风险、行情/产业链、新闻、模拟买卖、事件型通知 | NewsAI 与 D-01 同 kind 不同 owner；PaperSell 另有盘后扫描；ETF 是尾盘竞价 |
| 盘后 | 31 | ReviewTask、归因、观察回填、公告/评级/IPO、大宗/盘后交易保留项 | 15:30 产业链在 enum 外；手工/启动补推可发生在其他时段 |

共 67；Watchdog 为统计唯一归属放盘前，不意味着它只在盘前运行。原 65 状态仍按“37 ACTIVE / 24 INACTIVE / 2 STARVED / 2 OPT-IN”作为历史对照，新增两项有本地运行记录；不能直接把这些状态相加当成已冻结的当前生产 activation catalog。

完整逐项业务说明见 [67 项视图](all-push-kinds-2026-09-05.md)。真正的迁移目录必须进一步展开 producer：

- NewsToIdea：新闻重要事件触发的 D-01，与同 tick 来源批次触发的 NewsAI 分开。
- IntradayMarket：09:10 预检、盘中资金概览、15:05 快照提醒分开。
- PaperSell：盘中扫描与盘后扫描须证明是否共享同一通知完成 owner，不能只按名字合并。
- MarketActionAlert：OrderUpdate 与账户进入 Frozen 的触发分开检查。
- CandidateBoard/Invalidated：共享 diff owner，不能各自切换后写两套基线。
- CLI 单股/汇总、09:05/15:30 产业链：枚举外也要编目；AlertManager 仅保留无 caller 的 helper 事实。

这延续 Q16/Q32/Q74，不把四时段当成四次大切换。

## 4. 最近五个交易日：到底发生了什么

### 4.1 分日投递记录

| 日期 | analytics true（含 N02） | 其中 N02 typed Accepted | durable Accepted，按发送日 | durable Delivered，按业务日 | 本地成功声明/Accepted 合计* |
| --- | ---: | ---: | ---: | ---: | ---: |
| 08-31 | 44 | 3 | 24 | 21 | 68 |
| 09-01 | 410 | 3 | 31 | 31 | 441 |
| 09-02 | 160 | 1 | 24 | 24 | 184 |
| 09-03 | 74 | 0 | 21 | 21 | 95 |
| 09-04 | 111 | 1 | 19 | 19 | 130 |
| 合计 | 799 | 8 | 119 | 116 | 918 |

\* 合计是两个本地目录的分析口径，不是用户已收到/已读的精确条数。N02 已含在 analytics，不能再加一次。这里区分出 **119 条 durable typed Accepted + 8 条 N02 typed Accepted**；余下 791 条只以 analytics 成功声明计，不能统一升级为权威 receipt。三个历史业务日补推解释 119 与 116 的差额。

同期 analytics false 为 310 条，durable 决策中的 RejectedDurable 为 443 条；两者都不可简单作为发送失败率分母。本轮按观察时间读取到的 durable sink_results 为 119 条 Accepted，未出现 Rejected/Uncertain sink result；这不表示所有准备/准入都成功，也不表示未来不需 Uncertain 恢复。

证据：JSON `daily`、`analytics_by_kind`、`durable_decisions`、`durable_results`，均附 SQL。

### 4.2 消息量主要集中在哪里

| 类型 | 08-31 | 09-01 | 09-02 | 09-03 | 09-04 | 解释 |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| PaperSell | 0 | 254 | 131 | 0 | 23 | 对应模拟成交通知，不是实际券商卖出 |
| NewsAI / NewsToIdea | 0 | 120 | 6 | 48 | 38 | 多目标展开及跨批次评估是主要膨胀来源 |
| PaperBuy | 0 | 0 | 0 | 0 | 29 | 新卡片，之前未发卡不等于之前没有模拟买入 |
| DataMode | 21 | 11 | 10 | 10 | 5 | 高频健康状态消息，需核对状态迁移，不按“每天应一次”裁掉 |
| IntradayMarket | 12 | 14 | 8 | 10 | 9 | 会复用多种 occurrence；不能都解释为固定行情周期 |
| IndustryChainIntraday | 6 | 0 | 1 | 1 | 5 | 取数、候选、治理均影响数量 |
| G5b | 2 | 0 | 3 | 3 | 0 | 08-31 两条含测试标识，见 F02 |
| Watchdog | 0 | 0 | 0 | 1 | 1 | 新增且实际有本地成功记录 |

PaperSell + NewsAI + PaperBuy 为 649 条，占上述混合口径 918 的约 70.7%；09-01 卖出 + NewsAI 为 374/441，约 84.8%。因此后续体验评估必须覆盖这几条真实高流量路径，而不是只改盘后日报。

### 4.3 四时段分布及限制

| 日期 | 盘前 <09:15 | 竞价 Epic 09:15–09:30 | 盘中 Epic 09:30–15:00 | 盘后 ≥15:00 |
| --- | ---: | ---: | ---: | ---: |
| 08-31 | 1 | 1 | 41 | 25 |
| 09-01 | 2 | 20 | 399 | 20 |
| 09-02 | 0 | 0 | 168 | 16 |
| 09-03 | 3 | 4 | 70 | 18 |
| 09-04 | 2 | 5 | 115 | 8 |
| 合计 | 8 | 30 | 793 | 87 |

这张表仅用于诊断时间分布：typed 路径使用 accepted_at；其余只能使用 analytics.ts，可能滞后或来自事实时间，不能当最终 physical-delivery timeline。09:15–09:25 的精确竞价窗口只有 1 条记录；其余 29 条位于 09:25–09:30 间隙。盘中列含午休，五日午休分别为 4/46/3/3/3 条。严格 session 分桶见 JSON `phase_distribution`。

因此，“竞价期有消息”并不证明 AuctionVolume/CandidateBoard 已发。五日上述两层成功记录中未见这两个 kind；但只有在对应日期确认 producer 启用、输入非空、门禁通过、进程覆盖窗口后，才能判定漏发，不能仅凭零计数下结论。

09-05 是周末对照，不混进五日总量。取证时仍有 DataMode 成功声明和 N02 source-failure 日志；说明仍有运行活动，**不等于违规发了交易指令**。是否应在休市停掉这些非必要取数，要按 source/任务类型决定，不能关闭必要恢复和健康检查。

## 5. 具体问题：证据、根因、修订与验收

以下优先级是本轮风险建议，不改写 Q44 已批准的生产迁移顺序。P0 表示优先处理完整性/可靠性，不意味着本轮已授权实施。

### F01 — P0：消息统计和时间语义失真

**证据。** §4 的 119/116 差额、N02 八个 receipt 的 accepted_at，以及同日 analytics 的源时间。发送前 Markdown 在 [push_wechat_with_attempt_marker](../../src/bin/monitor/notify.rs#L3197) 先于 sink 保存，因此“文件存在”甚至不必然代表尝试成功。

**问题。** 现有方案想统一 delivery metrics，但若没有先固定时间和证据强度，降噪前后对比、漏发率、时效 SLA 都会算错。

**修订。** 每条记录分别保存/引用 business_date、source_published_at、facts_observed_at、prepared_at、attempt_started_at、accepted_at、finalized_at；统计分别为 attempted、typed accepted、best-effort reported、no-data、disabled、deferred、rejected、uncertain、finalized。以 intent/decision/attempt join，不能按“时间差不多+文本相似”给强确认。

**验收。** 三个历史补推仍归原业务日；N02 09:30 窗口不再显示凌晨发送；发送前日志、analysis complete 和 governance dedup 都不增加 Accepted 指标。对 918 只标样本观测口径，不宣称“总送达率”。

### F02 — P0：测试数据进入生产归因输入/消息产物

**证据。** [08-31 G5b](../../data/g5b/2026-08-31.jsonl) 第 1–2 行为 `TEST_CODE_000001`；[第一份发送前卡片](../../data/push_log/2026-08-31/150606_000000000000000018d0d203d9c3e9c8_0000c217_0000000000000002.md) 同样含该标识。analytics 同分钟有两条 `g5b_attribution/feishu/pushed=1`。

**根因证据。** [top_events_for_deep](../../src/monitor/attribution_deep.rs#L266) 只按严重级别排序并截断；[append_deep_attribution_row](../../src/monitor/attribution_deep.rs#L280) 硬编码 `data/g5b`；[main G5b](../../src/bin/monitor/main.rs#L9304) 直接读取当日告警。测试数据究竟由哪个测试进程写入，当前冲突中的 alert_log 不能作为已定案生产制品；本轮证明的是污染存在和读取端缺少该隔离，不是完整追责。

**修订。** 将 namespace 贯穿告警写入、读取、LLM、分析结果与发送；生产消费者拒绝测试 namespace，不只在 sink 前检查证券前缀。保留已污染样本作回归 fixture，不清库“修好统计”。

**验收。** 混入测试记录时，production 路径 provider/LLM/sink 为零调用；测试运行不能触达生产 alert/g5b/push_log。测试符号拒绝是防御补充，不能替代存储隔离。不得把本地 feishu 标记进一步称作用户已读的证据。

### F03 — P1：NewsAI 将采集评估身份直接当通知身份

**运行证据。**

| 日期 | delivered 事件 | 唯一新闻项 | 新闻×股票对 | 批次 |
| --- | ---: | ---: | ---: | ---: |
| 09-01 | 120 | 14 | 61 | 26 |
| 09-02 | 6 | 1 | 3 | 2 |
| 09-03 | 48 | 9 | 34 | 13 |
| 09-04 | 38 | 7 | 33 | 8 |

以上是 delivered event 与 assessment 精确联接，不是把所有评估都算发送。例：09-01 财联社 item `2470319` 对 `300413` 有四个不同批次、四个 assessment。JSON 保留这些 ID，方便定位，不复制新闻正文或模型输出。

**根因。** [source identity](../../src/database/news_ai.rs#L652) 和 [core_assessment_id](../../src/database/news_ai.rs#L714) 包含 provider、batch、item、target、analysis_version；同文章新批次天然形成新评估。main 的 [schedule_from_same_tick](../../src/bin/monitor/main.rs#L7971) 是蓝图 D-01 Top1 之外的独立 producer。

**重要限制。** `content_hash` 是完整 assessment 内容哈希，包含来源身份和模型信息，不是原新闻正文哈希。其不同不能证明新闻改版；相同 item 多次也不能证明内容/观点完全重复。因此不能将 120→14 或 120→61 直接设为通过门槛。

**修订。** 保留 BR-172 原评估/审计身份和至少五年保留；另定义通知的 business occurrence，例如“来源项+目标/受众+经批准的有效内容/结论版本”。batch 继续作 lineage，不因换批自动突破通知去重。是否一新闻合多股、何种观点变化触发更新，需要独立版本化体验决策；不能偷偷改既有模板和订阅。

**验收。** 相同事实仅采集批次变化不制造新通知；确有内容更正、方向或风险变化时按明确规则出新 occurrence，并能引用上一版本；重放同 occurrence 必须 exact bytes 且不二次调用模型或 sink。验证跨股票、跨受众、跨交易日和同文章修订，避免过度去重丢掉重要更新。

### F04 — P0：模拟成交与通知断裂，且批量处理拖延卡片

**证据。** 对发送前卡片按本地日期、方向、股票、数量、保留两位的价格，与 `paper_trades.status='Filled'` 联接：408 张卖出、29 张买入全部唯一匹配，详见 JSON `paper_message_matches`。此匹配证明样本与成交一致，不代替 intent/receipt 强关联。

| 日期/方向 | 卡片数 | 无匹配/多匹配 | 成交到发送前文件：最小 / 中位位置值 / 最大 |
| --- | ---: | ---: | --- |
| 09-01 卖出 | 254 | 0 | 18 / 535 / 1130 秒 |
| 09-02 卖出 | 131 | 0 | 49 / 317 / 621 秒 |
| 09-04 卖出 | 23 | 0 | 147 / 412 / 533 秒 |
| 09-04 买入 | 29 | 0 | 0 / 24 / 40 秒 |

文件时间精确到秒且先于物理发送；上述不是远端 Accepted 延迟。09-01 最大 18 分 50 秒，已足以说明“30 秒 tick”不能等价“30 秒内通知”。

**根因。** [scan_and_sell_inner](../../src/trading/paper_sell.rs#L367) 顺序扫描全部持仓、形成 `Vec` 后才返回；[main](../../src/bin/monitor/main.rs#L9000) 再逐条 await 发卡。某笔成交一旦落账，通知失败只 warning；下一轮可能因已成交/幂等规则不再返回它。新增 [PaperBuy](../../src/bin/monitor/main.rs#L8970) 同样只消费本次 `report.fills`。

**修订。** 以稳定 fill ID 建立通知意图，并让通知消费者独立追赶已提交成交；在业务 owner 的可证明事务位置写 outbox。发送恢复不得重跑交易模拟、不得反向把通知失败改成成交失败。若逐笔即时输出或批次汇总改变用户体验，另设变更单元；先保证每笔成交可追溯、无重复和无永久漏通知。

**验收。** fill 已提交后进程崩溃、第一张卡发送失败、批中部分成功均能恢复；恢复不再生成买卖。记录 fill→intent→accepted→finalized 各段延迟并设业务可接受时限。增加模拟账户来源/命名空间，避免用户把数百只模拟持仓误认为截图里的六只真实持仓。

**不做的事。** 不因为 254 条很多就删除成交、不把数量直接定性为重复、不自动补发历史所有 PaperBuy 卡。

### F05 — P0：完成游标早于确认，分析成功冒充通知成功

| 路径 | 当前状态写入 | 风险与建议 |
| --- | --- | --- |
| AttributionDaily | `push_governor_v3` 返回任何 outcome 后设置 `ATTRIBUTION_LAST_RUN` | 报告持久化与通知确认分开；收盘价改成日K解决的是取数，不是完成问题 |
| G5b | 分析/保存成功后 `done += 1`；批次尝试结束即封日 | 保存模型结果后从相同字节重试通知，不能重跑模型计费；NoData 单独终结 |
| CandidateBoard/Invalidated | diff 基线先写后发；候选批为空时在 diff 前直接返回 | 同一 owner 原子管理待通知失效与基线推进；全部候选消失也要区分“确认空”和“来源不可用” |
| LimitBoards | `board_notified.insert` 先于发卡 | 只有满足明确 completion policy 才提交通知去重 |
| 09:05/15:30产业链 | NotificationService false/error 仅记日志，外层以报告流程成功封日 | 区分 report saved 与 delivery completion |
| 15:05快照提醒、PaperReview | 本地日标记存在与通知返回脱钩 | 不应与同kind其他周期共享完成事实 |

证据：[归因完成写入](../../src/bin/monitor/main.rs#L9269)、[G5b done/封日](../../src/bin/monitor/main.rs#L9372)、[连板提前标记](../../src/bin/monitor/main.rs#L10471)、[候选 dispatcher](../../src/bin/monitor/push_templates.rs#L7843)；产业链/其他两项的原链在蓝图 §24.6–§24.7，涉及冲突模块时保留原断言，须以最终 clean source 复核。

**共同验收。** provider NoData 可关闭调度 occurrence，但不可推进“已通知”游标；Accepted 之后 finalizer 崩溃只恢复 finalization，不重发；`Deduped` 必须查询精确原 decision 的可验证终态，不能一概转成功或失败。候选从非空变成确认空时，失效事件不应被提前return吞掉；来源失败则不能批量宣告候选失效。

### F06 — P0：Watchdog 能出声，但无法证明全链路“不漏报”

**代码事实。**

- [检查器](../../src/bin/monitor/main.rs#L6253) 在 review scheduler 的 60 秒 loop 内。
- [发送后标记](../../src/bin/monitor/main.rs#L6277) 不检查 outcome，即调用 mark_fired。
- [news 注册](../../src/bin/monitor/main.rs#L7802) 只在 09:30–09:44；任务没跑或晚启动可能根本没有 deadline。
- [news 满足](../../src/bin/monitor/main.rs#L7935) 只看 flash Accepted 数；NewsAI 发了消息也不会满足这个 family。
- [review 满足](../../src/bin/monitor/main.rs#L6467) 是一轮 attempt 返回，Err 也 satisfy；[归因满足](../../src/bin/monitor/main.rs#L9274) 也是发起 governor 后即记。

**实际记录。** 09-03 review_evening deadline=19:40，21:13:42 才 fired；09-04 news_first_wave 在 09:45:08 fired；09-04 review_evening 到 22:45:17 才 satisfied，fired 仍空。09-03 晚间 Accepted 记录实际在 19:00–19:02 已有多项，说明新哨兵自己的 deadline 记录并不等于历史完整推送事实。新部署/启动覆盖差异可能解释其中部分，但没有 binary/进程时间证据不能擅自选一个原因。

**修订。** deadline 由独立期望计划产生，明确 startup catch-up/停机窗口；分开“任务有进展”“发生外发尝试”“拿到Accepted”。持久化 `observed_overdue / alert_attempted / alert_accepted`，不将一个 fired 位代表三件事。检查执行不受慢 review/provider 阻塞；基础 health/CLI 能在业务 sink 故障时独立查到超期。

**文案问题。** 09-04 09:45 新闻首波卡称“至今全静默”，但 09:26 起 NewsAI 有记录；准确含义是“09:30 聚合窗口尚未确认”，不是全部新闻推送静默。轨道名称和完成标准必须进入文案。

**验收。** 晚启动、原任务未执行、正常NoData、全部治理拒绝、远端Uncertain、watchdog sink失败、跨日遗留、复盘长耗时都要覆盖。哨兵发送失败不得永久封口；Uncertain 仍不得盲重发。

### F07 — P1：调度能力缺口被当成普通失败重复执行

**证据。** 五日 T-14/T-15 分别 449/467 条 `TradeEventSource is not registered`；R-03 有 102 条 account_metrics_incomplete review transition。CandidateTriggered 仍处于市场 active 后却要求盘前 Closed 的分支。[盘前辅助分支](../../src/bin/monitor/main.rs#L9751)、[盘中分支](../../src/bin/monitor/main.rs#L10218)。

**R-03 精确根因。** [dependency catalog](../../src/bin/monitor/review_batch.rs#L502) 定为 LegacyAccountGate；[dispatcher](../../src/bin/monitor/push_templates.rs#L9799) 对 account_required 无条件构造 AccountMetricsIncomplete，没有读取今天的 real_account_snapshot 再作决定。此前直接将其归因于截图未补齐不成立。

**修订。** catalog 明确 NoProducer / SourceUnregistered / ContractUnavailable / Starved / OptInDisabled；初始化可见、运行时不空转。依赖恢复按显式 capability 变化唤醒，仍保留定时核查；永久不具备的能力不能每分钟当 transient retry。Q1/Q30 仍生效：不借修调度把已禁用、缺输入或需 opt-in 的业务激活。

**验收。** INACTIVE 无 scheduler/provider/sink；STARVED 无新增输入时不触发伪消息；已批准的 ACTIVE 任务失败有 next eligible time/原因。盘前、竞价后间隙、午休、尾盘竞价、盘后分别用注入时钟测试。重新启动不重置永久缺口为立即反复扫描。

### F08 — P1：类型化源错误经过字符串转换丢失 retryable

**证据。** 五日 R-08 dispatcher 259 条失败：256 条 `invalid_evidence`、3 条 `no_verified_batch`；review 102 条失败却统一标 retryable=true。09-04 第一次 dispatcher 的 CFFEX 错误明确 `retryable=false`。

**根因。** [R-08 loader](../../src/bin/monitor/push_templates.rs#L11150) 将 GatewayError 转成字符串；[prepare 失败分支](../../src/bin/monitor/push_templates.rs#L11192) 固定 `ReviewTaskOutcome::failed(true, error)`；[review_reason_category](../../src/bin/monitor/review_batch.rs#L157) 又按字符串含“请求/transport”等词归类。永久证据缺口被读成网络失败并持续退避重试。

**修订。** 将 source、operation、reason namespace、retryability、retry trigger 一起携带；说明文字不能驱动状态转换。CFFEX 是当前所需合同，不能为了让卡发出去直接删掉、填空或把错误改 NoData。需由外部 provider-host 修复能力/证据，本仓保留清晰的不可用状态。

**验收。** 原 invalid_evidence fixture 贯穿 Gateway→dispatcher→scheduler 后仍为不可自动定时重试；短暂 transport failure 才走有上限退避；能力版本修复可明确重新允许尝试。按错误家族设 metrics，hash/长文本只放日志，避免高基数标签。

**另一处统计问题。** A-11 五日 false 共341，其中318明确 `no IPO announcements today`、23空原因。不能把341全部归为NoData，更不能全部叫错误。8条true也不能直接当8个独立IPO阶段变更，需要事件身份核验。

### F09 — P0：底层有校验，不代表上层保留了可重放回执

**正证。** 普通 CLI 分支不是只看 exit=0：[notify.rs:4565](../../src/bin/monitor/notify.rs#L4565) 还验证 message_id/platform_msg_id，拒绝缺失/占位回执。这比“裸进程成功”强，旧评价需要保留这一事实。

**缺口。** 它最终返回 `Attempted(true)`；上层 [push_wechat](../../src/bin/monitor/notify.rs#L3167) 再返回 bool。HTTP/CLI/L6、尝试前拒绝/尝试后失败没有统一保留 exact receipt。NewsAI 有自身 immutable assessment/delivery event 和 sink-start 标记，但本地 audit receipt 不自动等价远端 typed receipt。

**修订。** 一个应用投递 interface，对 generic、P01/N02 和兼容路径使用不同 adapter；先按实际保存的证据分类。NewsAI 先做 conformance/receipt gap 评估，不仅凭“有状态机”就新增第三个 DedicatedAuthoritative。若必须保留第三类独立 authority，需要新增决策，而不是偷偷突破 Q27。

**验收。** Accepted、BestEffortReported、Partial、PreAttemptRejected、Uncertain 不互转；unknown/sink-start 后异常不假定未发送；finalizer 只接收重新验证的 VerifiedTerminalRef。manual confirmation 有独立审计与指标，不伪造 transport receipt。

### F10 — P1：窗口缺消息与“没有数据”的解释不完整

**证据。** N02 五日只见8个Accepted窗口：08-31为09:30/13:00/15:00，09-01为09:30/11:30/15:00，09-02仅15:00，09-03无，09-04仅11:30。P01 仅09-03/04各1个Accepted；竞价量能/候选板在本期成功记录中未见。

**结论边界。** 不能从“4窗口×5日=20，仅8条”直接算60%漏发：进程可能未覆盖窗口、输入可能真空、readiness可能不满足。必须补齐 expected→eligible→prepared→denied/no_data/accepted 的每窗口状态，才能裁定具体责任。

**修订/验收。** PhaseScheduler 为每个启用任务生成可查询的期望 occurrence；NoData 必须带来源确认和时间，不允许空Vec/取数失败冒充。错过窗口提供明确 missed/deferred 原因，而不是自动补发时效已失效的行情。P01/N02 现有强恢复能力保留，主要补日程解释和运行可观测性。

### F11 — P1：真实账户、观察建议、模拟账户的语义还需分离

**只读回查。** 这段会话截图对应 `user_position_snapshot.id=25`，effective_at=09-03 19:14，6项；`user_account_summary.id=27` 同时间。real_account_snapshot 最新仍为09-02。不能将用户较早说的“今天”重新解释成09-05，再写一遍相同截图。

截图总资产、市值、可用现金存在16,260.23差额；未知资产分项不能靠总资产减市值猜成可用现金。部分字段可作为用户确认的展示事实，不应自动升级成真实可交易账户能力。R-03 固定gate与这个差额不是同一根因。

9/4 一份 T0 卡同时标了 Frozen、时间不可信、观察价格区间以及“执行前另取≤30秒券商可用持仓并校验T+1”。这说明现有文本已做部分研究/执行区分，不能仅凭出现价格就认定绕过交易风控；但标题、风险条、事实日期和可执行状态需从同一结构化快照生成，防止读者误解。

证据：上述三张表只读查询；[9/4 T0 发送前记录](../../data/push_log/2026-09-04/83844e69e7c7c83f39aeb5359c52984abf982d1f7789d23955c1d081fddbc372_73b40e25b60f4988d990f2cd343a87fb467a6aa7b8fbd1f9e330ee2b7b3ff519_audit_pending.json)。本轮未补录账户、未修改业务能力门禁。

**验收。** account_snapshot_ref、position_snapshot_ref、valuation_date、execution_permission、paper_account_ref 分列。通知成功不改变账户状态，paper fill 不改变真实持仓；快照更新只影响明确声明依赖它的 producer。

### F12 — P0（实施前置）：文档“就绪”标签超过实际交付

**证据。** docs/push-system 此前只有决策表与硬化计划，没有其所引用的实施 RFC/capability catalog/evidence manifest。九份被治理要求覆盖的 v18/v19 来源仍 ignored/untracked；JSON `design_sources` 列出具体路径与SHA。蓝图 §24.10–§24.19 仍混放未来设计，§26 固定65，renderer还硬编码 Implementation-ready。

**修订。** 本报告作为临时复核结果；按 Q56/Q107 将“当前审计”和“未来RFC”分离。保留原108题答案及来源字节，新增一版当前目录，不把Q93历史65字面值改成当前永恒常量。strict 发布必须校验 clean baseline、source SHA、symbol SHA、catalog、模板/本地资源、HTML、决策与 WBS；draft 也不能豁免内容漂移。

**验收。** 从干净checkout能离线生成文档；删测试目录内生成HTML后能重建；任何kind/source/symbol/hash变化使strict失败。当前160个冲突不在本轮分析授权内，不为通过检查而处理它们。

## 6. 结合 v18/v19：哪些一起做，哪些不能顺便做

| 设计 | 本轮实际证据对应 | 对整体方案的影响 | 范围边界 |
| --- | --- | --- | --- |
| v18 DataHealth / Data Contract | N02源错误、T0 freshness提示、模式频繁变化 | 通知带结构化数据健康引用，区分缺失/过期/确认空 | 不重写全部Gateway；不把展示健康当执行许可 |
| v18 DecisionRecord | NewsAI跨批身份、同kind多producer | InvestmentDecisionId/AssessmentId/PushDecisionId分开，保留关联 | 不把推送ID作为投资决策ID |
| v18 PaperLedger | 成交很多、发卡延迟、成交后通知可能丢 | fill是业务真相；notification intent消费它 | 不造第二套账本；不启用实盘Gate L |
| v18 Attribution/ModelChange | G5b同类输入与测试污染 | 模型结果持久化、后续通知重放不重算；归因引用原决策/成交 | 不让复盘自动改模型/阈值 |
| v18 AuditJournal/Gate P | 现有多类窄审计 | 按证据类别保留最严格策略；NewsAI至少五年 | 不把本地hash-chain等同远端WORM，不统一90天删除 |
| v19 PR-3/6/9/10 | 错误语义丢失、口径混合、哨兵盲区、测试污染 | 推送所需reason/metrics/health/isolation必须作为Foundation和切片门禁 | 复用一个合同，不平行建设另一套状态/错误词典 |
| v19 Quiet/Banner/breaker | 周末仍取数、数据模式消息多 | 任务级运行策略/共享快照/按source-operation熔断值得实施 | 不把所有后台恢复在休市关闭；平台级部分另估范围 |
| v19.1 OutcomeTracker | 仅知道发了不等于知道分析对了 | 建立通知/分析样本到T+N结果的关联，复用现有SQLite owner | 未观测完窗口不提前评分，不新建平行JSONL真相 |
| v19.2 AI评价 | 212条NewsAI带来输出量，但无本轮效果验证 | 冻结prompt/model/data，比较无AI基线、成本后收益与有效样本 | 旧IC=-0.0775不是当前结论；不以“加模型/加agent”承诺变好 |
| v19.3 全天工作流 | 57/59→65→67，窗口与producer漂移 | 历史设计保留，当前目录机器派生/对账 | 不继续维护多份相互矛盾的“权威推送清单” |
| v18.2–v18.5 自声明v20 | 因子/回测/扩容草案 | 记录版本标签冲突并保持来源不变 | 不认定文件放错，不把K8s/Redis/券商接入加入本次 |

原始证据：[v18上位设计](../v18.x/v18.0-2026-07-16-brainstorming-quant-platform-closure-design-active.md#L42)、[v19运行面PR表](../v19.x/v19.0-operational-clarity-design.md#L100)、[v19.1](../v19.x/v19.1-review-enhancement.md)、[v19.2](../v19.x/v19.2-ai-analysis-improvement.md)、蓝图§25。九份ignored来源为v18.1–v18.5及v19 catalog/19.0/19.1/19.2；本轮只捕获SHA，没有移动、纳管或改写原文。

v19 文内 breaker 的“5次失败”与“默认10次”仍需显式裁定；不能通过随便选择阈值宣称完成设计。v20草案所谓“v19已解决运行问题”也不能作为当前事实依据。

## 7. 应如何修订整体方案

### 7.1 延续已批准的硬约束

| 已批准事项 | 本轮裁决 |
| --- | --- |
| Q2/7/9/78：可验证终态、弱结果分栏、Uncertain不盲重发 | 保留；F01/F05/F09进一步证实必要性 |
| Q16/32/74：按producer/occurrence/completion owner原子迁移 | 保留；不能按67个kind或四时段粗切 |
| Q17/33/34：Foundation先行，共享PreparedFacts，shadow无副作用 | 保留；Paper路径尤其不得二次模拟成交，G5b/NewsAI不得二次调LLM |
| Q4/10/19/22：兼容/精确字节、正确性先于体验 | 保留；强制汇总/改订阅/换模板不是零行为重构 |
| Q27：一个应用投递端口，默认generic，P01/N02专用adapter | 保留；NewsAI首先做conformance，不自动批准新authority |
| Q1/21/30：不激活Inactive/Starved/Opt-in | 保留；修错分支并不等于允许恢复业务 |
| Q36/37/43/99：每日最多一次owner晋级、风险观察、用户监督 | 保留；Codex不是隐含24小时值守人 |
| Q70/88/104：临时产物、严格保留、干净基线再发布 | 保留；本轮可以写报告，不假称生产就绪 |

### 7.2 需要补进下一版 RFC 的四个工作面

这些是基于新数据的建议，尚未替换历史批准顺序或冻结Unit数量：

1. **NewsAI 通知身份切片。** 明确保留assessment与delivery历史，新增稳定业务通知身份；判别内容修订而非仅批次变化；补真正远端receipt保留。
2. **Paper fill→notification切片。** 以既有成交ID驱动outbox/补偿；成交和发送解耦；延迟指标覆盖扫描等待，买卖卡都进入目录。
3. **期望任务/哨兵切片。** 独立注册、启动追赶、进展/尝试/Accepted三个层次，失败不封口；不依赖慢provider调用才能检查自己。
4. **错误处置切片。** typed ReasonCode/RetryPolicy贯穿取数到调度，能力不可用与临时故障分开；清理T-14/T-15/R-03类无意义循环，不激活它们。

测试隔离不是新的大型平台项目：F02应进入Foundation前的隔离验证及各切片验收。若用户选择将以上高流量路径提前到原Q44首批之前，需要追加批准记录；本报告仅建议优先调研/回放/设计，生产晋级顺序仍按Q44。

### 7.3 模块和接口应怎样收敛

按 codebase-design 的深模块原则，把重复责任收进少数接口，避免继续给 main.rs 增加计时器和flag：

- `PushIntentRuntime`：对调用者隐藏lease/CAS、prepared payload、恢复、terminal-ref复核和finalizer幂等。调用者提供业务事实与completion proposal，不自行解释Deduped。
- `DeliveryPort`：同一应用结果合同，generic/P01/N02/compat为adapter；本地analytics和observation只能消费结果，不能制造Accepted。
- `TaskSchedule`：封装eligible/missed/deferred/readiness/next wake-up；四时段只是调度输入。期望计划与实际执行事实可以分别检查。
- `OperationalSnapshot`：任务期望、源健康、最后进展、告警状态由同一事实投影提供给本地CLI/日志，业务消息不是唯一查询入口。

以上名称是拟议接口说明，不是宣称源码已存在。内部可有存储/时钟/transport测试seam；不需要为了每个名词再建一个微服务。业务账本owner保持原位，runtime只消费可验证引用。

### 7.4 必须补齐的合同，不能只留概念图

| 合同 | 要回答的精确问题 |
| --- | --- |
| RunContext / PreparedFacts | 哪个business_date/session、来源版本、事实时间、模型输出被捕获；重放哪些字段不变 |
| intent identity | 不含payload hash；同身份内容冲突如何进入ResolutionRequired；多目标/多受众如何区分 |
| CompletionPolicy | owner、推进事件、NoData/Disabled/Retry行为、finalizer类别、expected version |
| VerifiedTerminalRef | 重新验证decision/occurrence/subject/audience/template/expected-version及authority namespace |
| 跨库outbox | 业务提交、durable预留、sink开始、Accepted、业务finalize每个崩溃点如何恢复，何时绝不重发 |
| activation | build/Git/catalog/两库schema/template/source-contract版本，generation CAS、journal、Draining |
| rollback | 仅Foundation兼容逻辑回滚；既有Accepted/未完成intent不删除，旧owner受fence隔离 |
| retention | 非终态不清理；按最严格证据政策保留；不复制密钥和非必要持仓正文 |

本报告没有用几行伪DDL替代Q61要求的完整实施规格，也不把当前30条Applied review hydration当成新business intent finalizer的验收。前者是既有调度状态恢复，后者尚未实现；不能直接拿其延迟证明Q38 SLA已达标。

## 8. 改动顺序、周期与人力

### 8.1 建议执行顺序

| 阶段 | 交付 | 前置条件 | 不应宣称的状态 |
| --- | --- | --- | --- |
| A：文档/证据收敛 | 本报告、67-kind视图、runtime样本、既有108题差异；随后完成正式RFC/catalog/WBS/离线构建 | 当前允许PROVISIONAL | 不是运行问题已修复 |
| B：可发布Foundation | 类型/intent/outbox/terminal-ref/manifest/shadow/operational最小切片 | 用户混合改动形成可编译clean baseline，精确目录就绪 | 不是全部消息已迁移 |
| C：首批原子切片 | 按Q44逐单元验证CLI/链报告/归因/候选等；新工作面先回放设计，变更顺序另记录 | 每单元故障/回放/自然occurrence门禁 | 代码通过不等于生产验收 |
| D：其余不合规路径 | NewsAI、Paper买卖、Watchdog、错误处置等按目录纳入 | 单owner、正式接口、依赖能力 | 不顺便激活缺源业务 |
| E：体验改进 | 汇总/分层/文案/受众与效果评估 | 正确性完成，显式版本化语义 | 不算零行为兼容迁移 |
| F：全量退出 | cleanup、默认并行测试、恢复演练、真实Accepted/重放证明、文档一致 | 所有required Units完成观察 | 无证据不能标Program Production Verified |

### 8.2 工期不能再报一个无法复算的总数

历史Q41批准的是**暂估36–69个8小时等效工程日、7–10交易周**，不是冻结承诺。该数以“W01–W21基础98–142小时+约35单元额外工作+缓冲”推出，但正式文档已找不到W01–W21明细；35也是下界。现在又出现新的producer和业务完成切片，不能继续按旧数量承诺，更不能随意改成另一个未经WBS支持的区间。

下一版需要恢复原WBS，并对实际目录逐项填三点估算：

```text
单任务期望小时 = (乐观 + 4×最可能 + 悲观) / 6
工程投入 = Foundation各任务 + 每个Unit的实现/故障/回放/灰度准备/清理 + 明示缓冲
最少晋级交易日 ≥ 需要变更physical owner的Unit数量（每日最多一个）
日历工期 = 工程与合规观察可重叠后的关键路径 + 用户窗口/依赖发布/交易与样本等待
```

不要把AI写代码速度等同交易窗口速度。阶段A文档可以继续推进；全量可编译和正式生产门禁受当前源码基线影响；R-08之类外部provider合同需要独立交付证据，但不必阻止无依赖单元的设计与shadow准备。这个依赖隔离比给总工期统一增加一个百分比更有效。

### 8.3 人力基线

开发基线仍是 Codex 承担实现、测试、取证、文档和候选操作命令，不要求另外配“前端/后端/测试各一人”。人的不可替代职责是：用户或指定操作人裁定产品语义、提供/确认账户证据、在线监督生产owner晋级及人工Uncertain处理。外部provider-host能力缺失时需要该系统的维护权限或负责人配合。

本轮不把这些职责折算成已存在的24小时值班人力，也不假设每次15分钟内都有人。若要无人值守故障处置，应另行设计值班/权限/升级链，不能把Codex默认当on-call。

## 9. 从真实样本生成验收，而不是只测试“能发送”

| 样本/场景 | 必须证明 |
| --- | --- |
| 08-31历史复盘补推 | 业务日期保留原日，发送日可不同；同decision重放不二次发送 |
| N02 analytics时间错位 | acceptance timeline取receipt，不取source采样时间 |
| 08-31 TEST_CODE G5b | 生产读取/模型/发送均隔离；污染样本不删而转回归fixture |
| NewsAI同item同target跨4批 | 新批不自动产生新通知；有效修订仍可出新版本；不破坏五年原审计 |
| 09-01 254笔卖出 | 全部fill可追踪到intent；批中失败/崩溃不重下单、不丢通知；延迟逐段量化 |
| 09-04 29笔买入 | 只对Filled生成执行卡；NotFilled保留明确业务状态，不伪造成交 |
| Attribution/G5b成功后sink失败 | 复用已保存结果，只恢复通知；不封错游标、不二次模型计费 |
| R-03固定依赖不可用 | 不轮询假装等用户补数据；显式不可用且不会新增producer |
| R-08 invalid_evidence/nonretryable | retry语义从Gateway到scheduler不丢失；修复能力后才显式重新允许 |
| Watchdog晚启动/慢review/无任务注册 | 期望任务仍可审计；检查不被同一路径卡住；告警失败不永久fired |
| NoData/Disabled/冷却/Uncertain | 调度完成、通知完成、人工处置分别计数，任何强度不互相冒充 |
| 两库/manifest冲突与回滚 | expected-version/CAS冲突进入ResolutionRequired，不覆盖写，不回滚外部Accepted事实 |

测试先在Test namespace完成，生产只做经批准的正常typed receipt与幂等重放灰度；不主动断网、杀库或制造交易行为。完整默认并行Rust测试须在冲突解除后重新执行，本轮没有复用历史绿灯作结论。

## 10. 本轮交付与后续边界

本轮新增中文报告、全量审计视图、只读SQL/文件采集工具和证据JSON；报告将蓝图的已批准方向与新增建议分开，并纠正了旧统计、NewsAI身份、Paper延迟、R-03、Watchdog和receipt强度的误判。

没有修改产品Rust代码、运行配置、数据库、已有v18/v19原文或用户的冲突；没有发送消息、运行交易模拟、提交或推送Git。新产物位于用户要求的 `docs/push-system/`，但该目录当前仍受ignore规则影响；“本机已落盘”与“已纳入干净Git发布”是两回事，需在既有文档治理任务中处理，不能偷偷强制暂存。

复查命令：

```bash
ruby -c docs/push-system/reanalyse-recent-pushes.rb
ruby docs/push-system/reanalyse-recent-pushes.rb --write
ruby docs/push-system/verify-reanalysis.rb
```

`--write` 仅重生成本目录JSON和全量审计视图；它读取的是运行中的最新状态，采集时间、周末计数或迟到补推可能变化，不能要求跨时间输出字节相同。校验器使用本报告五日快照的冻结断言；运行证据变化时应重新分析、更新报告，不应直接放宽断言。要发布长期golden fixture，应另保存经过授权、去敏且一致的冻结证据集。

最终判断：**先让系统准确知道“该发什么、为什么没发、是否被接受、业务是否完成”，再优化“一天看多少张卡”。** 最近数据支持的是补齐这些闭环，不是再增加一层发送封装或直接压缩所有消息数量。
