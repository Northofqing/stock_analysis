# 整体设计剩余工作与开发交接（2026-10-06）

> 核对时间：2026-10-06 21:55–22:02 CST。源码基线：`9a71f6069847a1350480203399264d3b757a1c68`。
> 本文是当前接续入口，历史证据保留在 [PLATFORM_HANDOFF.md](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/PLATFORM_HANDOFF.md)。源码、测试、候选制品、生产接线和自然观察分别记录。

## 1. 结论与范围

**整体设计尚未完成全部上线。** 最近完成的是投资决策提交/回执恢复、普通归因报告、治理 Draft 合同与材料持久化，以及治理/资金材料按保存 ID 冷恢复。正式正向投资决策、批准资金的发行、生产模拟执行、完整治理和 Gate P 仍有缺口。

剩余工作归为七组：

1. 受控构建、原生 provider 和完整 Financial 校验。
2. 代际迁移、原历史保全与生产冷恢复。
3. 真实资金、正向 F2 和正式 paper 接线。
4. 同版 SDK、真实业务 RPC 与上游数据资格。
5. 远端 WORM、签名根、恢复与 Gate P。
6. 推送 Unit、Uncertain、运行和 AI 评价验收。
7. 正式归因、策略治理、必要研究、前瞻观察和 M8 裁定。

此前用户优先开发的“第5项/第7项”分别是**正式投资决策与模拟盘、归因与策略治理**，主要落在路线图 M4 的 F2–F4；它们与里程碑 M5（Gate P）、M7（前瞻观察）的编号不同。

完成标准沿 [完整路线图](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/docs/superpowers/plans/2026-09-28-platform-complete-roadmap.md)：M0–M7 必要工作均有退出证据，M8 有实施或不实施裁定。项目不接券商；真实下单、券商回报和依赖券商的 T-14/T-15 不作为待开发交易能力。v20 的 55 因子、全部回测模式、Web/Redis/Postgres/K8s 仍需需求和测量裁定，不能全部列成必建功能。

## 2. 接手时的准确基线

| 对象 | 本次核对结果 | 证据范围 |
| --- | --- | --- |
| 当前 checkout | `/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis` | 已有隔离工作树 |
| 分支 | `codex/platform-roadmap-implementation-20261002` | 非主分支 |
| 代码 HEAD | `9a71f6069847a1350480203399264d3b757a1c68` | 本文开始前实际 Git HEAD |
| 远端 | `stock_analysis`，GitHub `Northofqing/stock_analysis` | 本次 `git ls-remote` 与代码 HEAD 完整 OID 一致 |
| 未提交状态 | 只有既存 `.replay-build-records/` 未跟踪 | 保留原件；本文新增文档另行提交 |
| Mac 正式根 | `/Users/zhangzhen/.local/share/stock-analysis-runtime` | 本次 launchd 只读检查 |
| Mac monitor / bridge | PID `14998` / `56417`，均 running，program/cwd 指向上述正式根 | 只证明进程身份与运行状态 |
| 实际部署源码 | 新闻修复 `f517f45c91ec9489f63d0156a1b8f9cf45400c3b` | 本次实际制品 SHA 与原部署记录匹配 |
| Windows 正式版 | 最后同版现场证据为 source `4e4995f8d3f2c7cd504d1dec0f238e6d4b4fc02c` / binary `517e0b4c…` | 引用部署准备记录；本次未重新调用 Windows RPC |
| 生产健康 | 最后明确快照为 12:09:17 CST `Frozen/Unsafe`，News 已恢复，缺 Quote/Kline/MoneyFlow/OrderBook，账户指标不完整 | 历史业务快照；本次未刷新 Health 或查询业务库 |
| durable 状态 | 最后发布准备只读记录：`user_version=9`，78 条 `UncertainManualReview` 保持隔离 | 本次未重新统计；切换前必须重查 |
| SDK 窄候选 | Mac R2 `3f0b29cd` 绑定 SDK `eea9cc6…` / binary `29a…`，已构建但未安装 | 不能配后继 f57c 或当前平台 HEAD 使用 |

本次实测生产文件：

- monitor：41,517,456B，SHA-256 `a7376a14cde1f6be3025d6dbbdedf61401c34154f3da928a8e3db30c4950d20b`。
- bridge：20,658,076B，SHA-256 `2546b74d3af6929b5a08de4303f506232c9988b8ff030ec994f45b642cd4b1e4`。
- activation 文件：296B，SHA-256 `3c8b49dba568a9e4530ce5598802ea6e6e6ba7e015ffd0fdcd297ea5c0de16b5`；配置 hash `00f3c35c438a4a7951221255370f7ce9e07d68ffe2e87ad2b645201635e88e2e`，effective_from=`2026-10-06T03:59:53.997478000Z`。

文件 hash、配置 hash、源码 revision 和数据库 schema 是不同身份。Catalog6/7/8 的 owner 资格、durable schema9、平台 schema14 候选也须按各自对象核对，不能混用数字推断兼容。

现场和候选详情见 [SDK 发布准备](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/docs/ops/2026-10-06-sdk-platform-release-preparation.md)、[新闻上线记录](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/docs/ops/2026-10-06-news-critical-score-rollout.md)。本平台分支最近六项源码提交均未部署。

## 3. 最近已经完成并推送的开发

| 提交 | 已完成的可验收切片 | 当轮实际验证 | 当前边界 |
| --- | --- | --- | --- |
| `d9468ed289ebe459497dbb6167753e41cb5e3a4f` | 决策幂等提交；关联父订单、原成交/观察 ID、费用和已实现结果的只读报告 | 新7＋相关3通过 | 消费既有非序列化 ApprovedPaperIntent；没有新增生产批准 issuer |
| `69cceef8294e9976d8e955a07964efa503d1d1c4` | 按账户/epoch/决策引用冷恢复原 sealed 命令回执 | 新1＋相关3通过 | 无新批准、行情、时钟或 SQL mutation；None 不允许新下单 |
| `f73930aaa0275a2c72ec0ab3914e687e9e477420` | 不可变治理 Draft：版本声明、双 paper book、窗口、样本/风险/容量政策、内容身份与严格恢复 | 新5通过 | `DeclaredReferencesOnly`；无 Reviewed/Promoted 或真实注册时间证明 |
| `4167b7c4d82c318d40e59b97daea8743341df23a` | Draft 材料接入原 UnverifiedOutbox；家族冲突、CAS、Unknown/close 失败所有权保留 | 新4＋相关2通过 | 本地普通材料；没有新 ledger/schema 或治理批准 |
| `775a6fd288e3a04cd10eb5a490e85afefd6c1165` | 按 package ID＋Draft ID 从真实只读快照冷载原文 | 新4＋相关6通过 | 保原件/查询/首错；没有找到不允许重建或重发 |
| `9a71f6069847a1350480203399264d3b757a1c68` | 按 package ID＋review ID 冷恢复资金复核材料 | 新3＋相关7通过 | `HistoricalObservationOnly / NotIssued / Unverified`，不发行 B 或可花余额 |

这是各提交时的定向证据，包含跨轮重复回归方法，不能相加成当前源码全量测试数。最后资金读端的相关7包含原资金写入3和治理读端4；当前 harness SHA `09bdcd59ab8fc610da62b139ee995b847821bf458868450a99f48dfacdfff6fb`，929 个源码输入前后相同。治理读端提交时 harness SHA `d7b4a4b61889976d597b62b23f7700cf1080faa86e197a02358d80695fc868a6`。

原资金读端首轮 2PASS/1FAIL 是夹具重排 canonical 字段；修正只保原字段顺序并替换批准值，生产源码保持。原失败日志保留，最终3＋7通过。最近六个切片由本 chat 直接自审，没有新独立 Approved 记录；此前任务的独立审查只适用于其原绑定范围。

其他已有能力：Catalog7 候选观察及 Catalog8 不可变拒绝记录、observed-only 风险矩阵、纸面 parent/partial-fill/no-fill/cancel/expire/FIFO/逐笔费用机制、目标复制/WAL/typed 原件读取及链链接局部校验、本地普通资金材料保存、受控构建工具局部协议、新闻恢复与逐项 outcome 反馈。它们各有历史定向证据，整体 owner、生产资格和上线验收仍按下面任务收尾，不重复建设账本或 OutcomeTracker。

## 4. 里程碑与剩余范围映射

| 里程碑 | 已有进展 | 尚缺退出条件 | 对应任务 |
| --- | --- | --- | --- |
| M0 重基线与稳定 | 部署身份、若干来源/目录/旧审计核对已有记录 | 当前源码/制品/配置/required Unit/真实回执三层矩阵持续对账，现存欠账及恢复依据完整 | H07、H08、H11、H12 |
| M1 推送 P0 | Foundation 和首批 owner、恢复、归档等局部代码及新闻部署 | 首批各 Unit 同事实 shadow、单 owner 接管、真实回执、观察和旧路径清理 | H10、H11 |
| M2 推送 Program | counted/intent/finalizer 和部分迁移能力已实现 | 所有 required Unit 的逐项 Production Verified；无未裁定阻断、双发或跨库不一致 | H10、H11 |
| M3 v19 运行与复盘 | 健康/原因、source 恢复、OutcomeTracker 和逐项反馈部分可用 | Quiet/Halted 全合同、独立运维告警、统一指标、至少5个真实运行日，AI增量评价/裁定 | H12、H15 |
| M4 v18 数据—决策—账本 | F0–F4 多个代码子段已完成，最近补决策/恢复/普通归因/Draft | 真实来源与 Financial、资金、正向 F2、paper 生产消费及正式策略关联/治理 | H01–H08、H13、H14 |
| M5 Gate P | ADR Proposed、本地有界留存和普通材料链路 | 真实远端1830天保留、签名根、四 owner 恢复/对账、PIT/成本后样本外与故障证据 | H09、H16 |
| M6 研究能力 | 研究成交/组合/精确窗口等局部机制 | 每个必要子项 PRD/ADR、与正式 FillModel/成本口径 parity、样本外门禁 | H15 |
| M7 前瞻与产品收敛 | Draft 已能固定声明的双账本、阈值和时间窗 | 原预注册/同输入资格、自然交易日样本、保留/限制/淘汰裁定和可解释日周报告 | H13–H16 |
| M8 按需扩容 | 沿用实测需求触发原则 | 有证据的实施或不实施裁定；有需求才落实新拓扑迁移/权限/恢复 | H17 |

本轮未找到足以关闭任一整阶段的完整退出证据；“未关闭”包含已编码未接线、待外部资格、待部署和待自然观察，不能等同于全部没有开发。

## 5. 可直接接续的剩余任务卡

### A. 受控构建、财务资格与迁移（H01–H03）

| ID / 优先级 | 剩余动作 | 前置与完成标准 | 入口 / owner |
| --- | --- | --- | --- |
| H01 / P0 | 完成真实受控构建 record、native 编译/归档/consumer 图、选定来源/layout/rules 和同进程 provider 发行 | 当前 Tools/policy 的 RecordingOnly 与协议测试之后，取得精确输入的真实完整构建/consumer/issuer 证据；失败、漂移、资源清理保持原合同 | Mac：`tools/replay_build_owner_v1.py`、`tools/replay_build_pin_v1_manifest.json`；构建 owner |
| H02 / P0 | 真实 SQL/capture/COMMIT/source-tail 接入完整 Financial；重算 audit/event/manifest/projection 内容哈希，完成 Genesis/执行/裁定和经济重放 | 局部 typed 原件与链接校验不能作内容哈希通过；同 owner、累计 Work/资源与既有16MiB路径完整成功/失败证据，消除真实 provider 缺口 | Mac：[replay work](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/src/database/global_schema_replay_work_v1.rs)、`src/trading/paper_replay_{codec,financial_work,shapes}_v1.rs` |
| H03 / P0 | 完成 exact6→8 等目标迁移的最终资格、全部非空历史保全、批准制品绑定、原子交换及 warm/cold 启动恢复；另完成平台 schema14 发布所需迁移/兼容 | 已有复制/WAL/typed 读端为起点；版本对象逐一确认，完整财务重放/对账一致，批准与回退精确绑定，原库保留 | Mac：[additive target](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/src/database/global_schema_additive_target_v1.rs)、[target](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/src/database/global_schema_target_v1.rs) |

H01/H02 是资金和真实执行的资格前置；普通材料、纯逻辑和必要 SDK 开发可独立推进。接续不能恢复成整个平台等待原生支线的单一队列。

### B. 资金、正向决策与正式模拟执行（H04–H06，原优先第5项）

| ID / 优先级 | 剩余动作 | 前置与完成标准 | 入口 / owner |
| --- | --- | --- | --- |
| H04 / P1 | 取得真实总资金 B、现金/现有持仓是否计入、初始分配/风险预算、seed/cutover 材料；实现真正批准资金的唯一发行与持久版本 | 用户尚未提供 B；已异步询问总金额及是否含持仓。须先制作精确可审材料，再按资金合同批准；历史一致材料或默认金额不发行可花资金 | 用户提供资料/审阅；Mac：[funding review](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/src/trading/paper_funding_review_v1.rs)、[material store](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/src/trading/paper_funding_review_store_v1.rs) |
| H05 / P1 | 完成正式正向 F2 的来源/风险/资金/人工裁定和唯一非序列化 intent 发行，保完整 universe disposition、成本/流动性及稳定关联 ID | Qualified 来源与实际 namespace、Financial、批准资金具备；缺失仍明确拒绝。保既有不可变负向记录；不将历史 DTO、推送成功或 cfg(test) issuer 转成批准 | Mac：[investment decision](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/src/decision/investment_decision_v1.rs)、[intent boundary](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/src/decision/approved_paper_intent_v1.rs) |
| H06 / P1 | 把正式决策、资金与原 parent order/retained writer 接到 monitor 调度及原 paper owner，完成日对账/恢复 | 沿已通过的幂等提交和 sealed 回执恢复；同决策只产生一个父单，partial/no-fill/cancel/T+1/整手/停牌/涨跌停/费用/持仓投影闭环，故障恢复无双 owner | Mac：[execution](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/src/trading/paper_book_v2_execution.rs)、`src/bin/monitor/` |

本次源码实读：`require_production_approval()` 仍返回 `EvidenceUnavailable`，sole positive issuer 仍仅 `cfg(test)`。H04–H06 未开发/接线完成；新增资金冷读不改变这个事实。Draft 的容量/参与率阈值也不是用户投入总金额 B。

### C. Windows SDK、业务数据与双端发布（H07–H08）

| ID / 优先级 | 剩余动作 | 前置与完成标准 | owner |
| --- | --- | --- | --- |
| H07 / P0 并行 | 修实际 SDK 关键覆盖率缺口和 audit 网络故障；生成合格新 tuple，Mac 最小重绑/构建/复核，再做受审维护窗口切换 | 同 HEAD 原80/95/checker、audit等发布门合格；精确 source/binary/descriptor/原始输入封存；Mac新制品/activation人审；同实例真实业务RPC、桥接及观察 | 原 Windows任务＋Mac发布 owner |
| H08 / P1 并行 | 完成真实 source→gateway→health→消费者资格：MoneyFlows/BoardFlows、公告完整覆盖、exact historical/PIT、D14、D17/D20及 R08 Confirmed；对新 SDK 同版业务路径验收 | request/instrument/provider/时间窗/整数价量/tick-band-halt-lifecycle-liquidity/逐代码终态/修订原件完整；缺失维持 typed unavailable，capability/Health 不作业务证据 | Windows合同/原件，Mac准入/消费；按 WG04–WG12 逐项 |

**最新 SDK 阻断（本次读取共享状态）：**

- SDK `f57c114190436e6ec96faa60910c127925ffbcfc`，CI `37446154439`。
- overall `69302/81598=84.93%` 满足80；critical `35372/39817=88.84%` 未达95。分母不变时还需2455条真实生产行覆盖，不是2455个测试。
- audit 因拉取 RustSec advisory DB 的网络 I/O 错误失败，尚无完成的漏洞扫描结论。原失败与重试分别保留；该故障不需要重复已结束的 coverage，源码变化则按新 HEAD 验证。
- 原 thresholds/globs/checker/准入保持。原 SDK R2、Health、离线2061测试或源码审阅不能代替发布门。
- WG07 本地 explicit-window/持久消费机制和 Windows 日期字段映射已有有限代码证据；真实 native range/selection/session、historical availability/as_of/revision/correction、同源完整生命周期/lossless 原件和生产 profile 仍缺。
- 新闻四源/实际模型名与发布时点的修复已部署；N01远端 Accepted、N02自然窗口和现场30秒指标仍需对应真实证据。R08 Planned 与 Confirmed 分开；新官方发布 RPC 的扩建需产品用途裁定。

Windows原任务：**R08 FuturesDelivery 上游合同与部署**，thread=`01a0e0cf-2276-7512-96ee-3a94bdfa8ca5`，host=`remote-control:env_e_6ab6a791c27c832a98417a42584a1a39`。用户已有开发、VM协调和功能分支推送授权。

本次之前的续办消息返回 `Timed out waiting for MCP response to fs/createDirectory`，送达未知。已写 [共享续办交接](/Users/zhangzhen/Desktop/Quant/stock_analysis/client-bundle/MAC_SDK_COVERAGE_CONTINUATION_20261006.md)（SHA `a3388319b67ec17ac968896838b11d99ccf77f1f9a5a70952fcf242e27c9007b`），尚无本次收件/开工确认。先核原任务是否已处理/有现存 CI，再继续同任务；不重复 dispatch 或另建用户 chat。

先读 [Windows最新状态](/Users/zhangzhen/Desktop/Quant/stock_analysis/client-bundle/WINDOWS_SDK_COVERAGE_REPAIR_STATUS_20261006.md)、[WG01–WG14计划](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/docs/superpowers/plans/2026-10-01-windows-grpc-development-plan.md)及原失败包。其旧 running/旧 SHA 段落按当时快照保留，以同 HEAD 终态为准。

### D. Gate P 远端设施（H09）

| ID / 优先级 | 剩余动作 | 前置与完成标准 | owner |
| --- | --- | --- | --- |
| H09 / P2 | 为四实际 owner 实现 seal→upload→exact-version HEAD/GET→保留/字节回执→签名日根→独立冷恢复与日对账 | [ADR-0003](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/docs/adr/0003-gate-p-remote-worm-authority.md)仍 Proposed；待提供方/账户/地区/权限/密钥责任人/费用裁定。真实逐版本≥1830天强制保留、验签/内容/链一致及故障演练；本地 outbox 不满足 Gate P | 用户/设施 owner决策；Mac实现 adapter 与验证 |

不把上传请求、本地 SHA、无版本 key 或材料存在当作已取得远端保留。对象存储、密钥及保留政策按具体可审结果准备后执行。

### E. 推送、Uncertain 与运行（H10–H12）

| ID / 优先级 | 剩余动作 | 前置与完成标准 | owner |
| --- | --- | --- | --- |
| H10 / P1–P2 | 冻结当前 required Unit 矩阵；补 Foundation 同事实 shadow、各 physical owner/intent/finalizer 接管与旧路径清理 | 同一次事实/同 exact bytes，无二次provider/LLM/sink/交易副作用；每Unit真实Accepted、AlreadyDelivered重放、恢复与观察。原52Unit是冻结目录口径，不能用counted kind数量换算完成数 | Mac；推送原业务/完成 owner |
| H11 / P0 发布门 | 保全并逐项人工裁定阻断的 Uncertain；完成跨库、超龄intent/finalizer、重复物理发送和完成游标对账 | 当前最后记录78条隔离；实际切换前重查。人审和确切投递证据决定每条终态，禁止批量盲重发/删除receipt。G5b尝试门与投递完成、P05发送/linked/sample分母仍按各合同 | 用户/操作人裁定；Mac只读材料与恢复 |
| H12 / P1–P2 | 统一M0现场矩阵、Quiet/Halted、typed reason/健康/错误/指标、每源恢复、日志/保留/脱敏及独立运维告警 | 真实required数据与账户快照、故障/恢复记录、至少5个真实运行日；逐项 outcome日/周报告接线与自然成熟证据。PID或News采集成功不关闭整个运行面 | Mac；生产操作与观察 owner |

同一交易日最多晋级一个改变 physical owner 的 Unit；高风险路径至少两个 eligible session。Starved/Opt-in/Inactive 项保持既定状态或有产品裁定，不通过虚构来源启用。

### F. 正式归因、治理与研究（H13–H17，原优先第7项）

| ID / 优先级 | 剩余动作 | 前置与完成标准 | 入口 / owner |
| --- | --- | --- | --- |
| H13 / P1 | 在正式决策/parent intent中持久绑定策略/模型版本、研究run与合格输入/预算；把决策→订单→成交→持仓→退出→结果串到同一真实身份 | H05/H06的版本化合同先固定；经原完整reader/ledger关联，无补造ID。当前报告实读仍为 `ReferenceOnly` / `StrategyVersionEvidenceV1::NotRecorded`；正式费用/结果与账本对账，缺关联样本不计完整策略证据 | Mac：[outcomes](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/src/performance/paper_decision_outcomes_v1.rs)及原 attribution owner |
| H14 / P1–P2 | 完整 GovernanceRepository及 Draft→Reviewed→Shadow→PaperChallenger→人工Promoted/Restricted/Retired，保审阅/证据/有效期/rollback | Draft合同/材料/冷读已做；仍需真实注册时间、审阅人和证据owner、版本/制品、同输入/PIT/完整成本、双book和成熟窗口资格。到期/样本不足限制或延长，不能自动改配置/阈值/模型 | Mac：[Draft](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/src/strategy/model_change_draft_v1.rs)、[store](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/src/strategy/model_change_draft_store_v1.rs)；人审晋级 |
| H15 / P2 | 冻结prompt/model/data及无AI基准，裁定活跃AI路径keep/restrict/retire；为必要事件回测/因子/模式/DSL落实PRD/ADR与最小实现 | M4稳定owner和可信数据集；复用原FillModel/费用，证明parity、经济容量、统计与样本外。局部research portfolio不是正式全模式回测；无需求不扩55因子或通用DSL | Mac研究 owner；产品裁定 |
| H16 / P2–P3 自然窗口 | 完成PIT/walk-forward/样本外/成本后证据、故障演练、基准与challenger前瞻观察，日/周报告及保留/限制/淘汰裁定 | 固定假设/阈值/试验次数/市场状态/容量后开始；T+1/T+5/T+20按交易日自然成熟，保未成交/缺数/人为覆盖；不足延长，有失败也保否定证据 | 研究/生产观察 owner；依M5与M6有效子项 |
| H17 / 最终裁定 | 对M8做实施或不实施决定；有真实需求/容量瓶颈才做Web、存储或分布式迁移 | M7用户使用事实和CPU/I/O/查询/恢复测量；独立PRD/容量ADR，保持决策/ledger/通知单owner、权限、迁移和恢复。不用某次构建封存慢推断整个系统需要扩容 | 产品/架构 owner；必要时实现 |

## 6. 接续顺序

1. 先核 checkout/HEAD/未提交变更、当前执行 owner 和 Cargo；读本文件、最新 tracked交接和VM同HEAD状态。用户已有继续开发/提交授权，常规源码不重问许可。
2. **P0并行：H01–H03真实资格与恢复，H07–H08 SDK/数据。** 普通合同、材料、必要归因源码可继续独立推进，最终能力发行和生产使用按前置汇合。
3. 按用户顺序完成 **H04→H05→H06**，同时固定H13版本关联合同；实际B资料未到就保明确缺失，继续不依赖该数值的源码。
4. 完成 **H13→H14**，将普通Draft子集接成真实治理；source与观察证据各自验收。
5. H09远端设施、H10–H12推送/运行按公共前置交错推进；真实跨库/remote/physical结果不能用隔离测试补齐。
6. M4/M5条件具备后推进H15/H16；窗口结束给裁定，再关闭或实施H17。

下一最小源码切片可从H02当前目标读取后的内容哈希与完整Financial接线，或H05真实输入能力的版本化合同开始；先核具体owner及可用前置，保持原reader/预算/历史失败。新SDK尚未合格时，发布切片不进入安装。

## 7. 发布与操作边界

- 新源码/配置上线需新精确制品、原始输入/配置hash、未来effective_from、人工审阅的activation及配套回退；旧Wave0/Wave1批准只适用于各原精确候选。
- 双端先完成具体维护窗口/keepalive单实例控制设计。原Windows全局同名keepalive不能随意改名/换端口绕过；Mac正式入口为launchd，沿真实根和原库。
- 切换前重新核PID/lease、两个DB路径/dev/ino/schema、数据来源/水位/时效、账户与78隔离组、候选/回退完整身份。动态门失败保持旧实例与原数据。
- 正常release和受影响dry-run按上线范围执行；取得同版真实业务RPC、bridge实际接纳、业务消费/回执及生产观察后，才将对应条目标Production Verified。
- 本次交接只写文档并读取Git、已有公开/本地回执、launchd身份和三个生产文件hash，没有运行Cargo、业务RPC/Health、业务SQL、安装、重启、资金批准或Uncertain裁定。

## 8. 证据与接手入口

**仓库可带走：**

- [AGENTS.md](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/AGENTS.md)、[CLAUDE.md](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/CLAUDE.md)。
- [架构蓝图](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/docs/Project_Architecture_Blueprint.md)、[完整路线图](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/docs/superpowers/plans/2026-09-28-platform-complete-roadmap.md)。
- [PLATFORM_HANDOFF](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/PLATFORM_HANDOFF.md)最新2026-10-06节；最新六项源码提交及上述代码入口。
- [SDK发布准备](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/docs/ops/2026-10-06-sdk-platform-release-preparation.md)、[新闻上线](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/docs/ops/2026-10-06-news-critical-score-rollout.md)、[Gate P ADR](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/docs/adr/0003-gate-p-remote-worm-authority.md)。

**仅当前主机的详细证据：**

- [.planning/2026-10-06-decision-attribution-priority/task_plan.md](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/.planning/2026-10-06-decision-attribution-priority/task_plan.md)；同目录 `slice-*-receipt.json`、源码快照、原失败/成功logs和remote读回。当前harness仅是本机已封存定向证据，不给新clone补造通过。
- [.planning/2026-10-02-platform-continued-implementation/task_plan.md](/Users/zhangzhen/.codex/worktrees/platform-roadmap-implementation/stock_analysis/.planning/2026-10-02-platform-continued-implementation/task_plan.md)：构建/财务及前期接续背景。
- 原主目录执行计划在 [/Users/zhangzhen/Desktop/Quant/stock_analysis/.planning/2026-09-29-platform-production/task_plan.md](/Users/zhangzhen/Desktop/Quant/stock_analysis/.planning/2026-09-29-platform-production/task_plan.md)。该路径在本工作树不存在；原文件含旧Wave/运行历史，不据其旧“当前状态”覆盖本次现场或最新tracked节。
- Windows `client-bundle/` 原封存源码/CI/失败包及包外Mac ACK；本次仍只认其原范围，f57c包回读不冒充Mac独立Git树重建/SDK运行/发布资格。
- [生产新闻最终followup2](/Users/zhangzhen/.local/share/stock-analysis-news-input-contract-rollout-20261006/final-deployment-followup2-receipt.json)，原SHA `1b1ce343ccc6ccd09c8cbb0a5cd55531c90e5e003bb5a7124b68b6c04480cc27`。

更换机器或checkout后按仓库内同名路径定位源码/设计，另移交所需公开回执；`.planning`、运行根、私有配置和日志不默认随Git交付。不要提交认证、Token、证书/密钥、数据库或大体积封存包。

## 9. 交接文档完成检查

本文以当前Git/源码入口、已存在原回执、最新共享状态和本次生产身份读取核对。仅文档变更：检查本地链接/任务编号/内容一致性及 `git diff --check`；不因交接重新编译或重跑此前已通过用例。整体完成率和全部上线日期暂不给无证据百分比或承诺，按H01–H17的验收逐项减少余项。
