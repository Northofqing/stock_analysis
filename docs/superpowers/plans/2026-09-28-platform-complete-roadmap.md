# 研究与模拟盘平台完整路线图（2026-09-28）

> 状态：规划稿；源码基线 `master@ac089275`，日期 2026-09-28。本文规划当前欠项、架构验收及其后的产品研究，不表示已实施、已部署或已取得策略收益。每个阶段以其验收证据更新状态，不凭旧设计标题或提交说明晋级。

## 1. 目标与边界

目标是形成一条可核验的链：**可信数据 → 可追溯投资决策 → 有成本的模拟成交与账本 → 归因与策略淘汰 → 可恢复通知与运行 → 前瞻模拟观察**。最终用户看到的信号、净收益、风险、数据来源和失效原因应能追到同一批事实；系统故障和没有有效策略都应明确呈现。

- 交易边界：项目只做模拟记录和人工决策支持。2026-09-21 用户已决定不接券商；v18 文档中的 Gate L、券商 adapter、真实下单和券商回报源不在本计划范围。T-14/T-15 等以真实券商事件为前提的通知需裁定为保持禁用、替换成有真实来源的模拟盘语义，或退役，不能通过虚构 feed 满足验收。
- 当前生产是 launchd 管理的 Desktop 外运行根。代码提交、测试通过、生产制品、activation、生效运行和外部回执是不同证据层。2026-09-29 用户明确以全部上线为最终目标，授权按阶段实施生产切换；每次切换仍按现有运行手册、activation 人工复核和单实例门禁执行，不由规划文件本身替代验收。
- v20 的 55 因子、5 种模式、Polars、Redis/Postgres/K8s/Web 均是候选方案。规划覆盖它们的决策与实施路径，不预设全部必须建设。新架构只能在有 PRD、ADR 和测量证据后取得实现预算。
- 上位约束：[项目架构蓝图 §24–25](../../Project_Architecture_Blueprint.md)、[v18 active 设计](../../v18.x/v18.0-2026-07-16-brainstorming-quant-platform-closure-design-active.md)、[v19 入口](../../v19.x/README.md)、[项目运行说明](../../../CLAUDE.md)及[当前协作规则](../../../AGENTS.md)。蓝图中的 2026-09-02 状态不能直接当成 2026-09-28 现状。

## 2. 当前基线与待核实项

| 能力 | 2026-09-28 可定位事实 | 本计划如何处理 |
| --- | --- | --- |
| 模拟盘成本和账本 | `src/performance/fee_evidence.rs` 已有逐笔费率派生；`src/trading/paper_ledger*.rs` 已有独立账本、裁定与有效投影；卖出卡使用 FIFO 买费分摊。 | 不重复安排 2026-09-21 审计的原始“加成本列”方案；核对生产版本、回放、异常价格及口径一致性。 |
| 账户模式与买入门 | `compute_account_mode_metrics_blocking()` 已读取账户摘要及纸面账本，原“无条件 Err”诊断已过期。 | 验证新鲜度、缺数时关闭、当日状态和跨周末场景；是否扩大模拟买入由策略证据决定。 |
| NewsAI | 已有 counted 接线和业务身份修复提交；不能由此推出生产中所有新闻路径都送达。 | 与其它通知一起做事实、决策、回执、恢复对账。 |
| 推送 Foundation | `src/monitor/push_job/`、`src/push_foundation/` 已有合同、意图、finalizer、readiness、activation、shadow 等代码；机器目录保留 52 个 Unit 身份。 | 逐项证明 production wiring 与 Foundation Ready，随后逐 Unit 迁移；“22 个 counted PushKind”不能换算成 Unit 完成数。 |
| 推送目录 | [current-source v2 增量](../../push-system/push-current-capability-status.v2.md)只修订四个 kind，固定旧源码快照；65-kind/52-Unit 冻结目录仍是历史身份。当前 `notify::PushKind` 源码枚举为 66 个，新增 `NewsAiAnalysis`，沿用 `MU-news-ai` 的业务 owner。 | 重新生成当前源码、部署、真实回执三层状态矩阵；版本化纳入新增 kind 与现有 Unit 的归属，保留旧 v1 字节和来源。 |
| 运行与数据源 | `monitor --health` 为只读局部健康快照；2026-09-28 launchd 迁移完成、Eastmoney GlobalNews 已有真实 `ADMITTED` 样本；R-08 仅证明 `Planned`，`Confirmed` 未证实。启动链审计重复全量验证造成数分钟延迟。 | 先取得新运行根的连续运行证据，分别核对各源、启动耗时与欠账重试；不把单一来源成功投影成全局 Ready。 |
| v18 / v19 / v20 | v18 四模块与 Gate P、v19 运行面及复盘、v20 候选都存在局部能力或设计稿。 | 按下列依赖交付；确切缺口在 M0 重测后锁定。 |

**状态词：** `Code Ready`＝相关实现和目标测试通过；`Shadow Ready`＝同一份事实的语义差分、无副作用；`Production Verified`＝目标制品、activation、真实输入、回执/账本及观察窗均核实。未取得后一层证据时不得使用后一层名称。

## 3. 里程碑与关键路径

| 里程碑 | 进入条件 | 可观察的退出条件 |
| --- | --- | --- |
| M0 现状重基线与运行稳定 | 本文启动 | 冻结当前源码/制品/配置哈希；对账冻结的 65 kind/52 Unit 与当前源码新增项、各上游能力、生产接收与历史欠账；旧审计已修项有新的正反证。 |
| M1 推送 P0 Production Verified | M0、Foundation 验收 | Foundation 零行为合同通过；首批错误完成 owner 的 Unit 完成 shadow、单 owner 接管、真实回执、观察和旧路径清理。 |
| M2 推送 Program Production Verified | M1 | 所有 required Unit 逐个迁移并观察；禁用/Starved/Opt-in 项保持既定状态；无未裁定 Uncertain、超龄 finalizer、重复物理发送和跨库不一致。 |
| M3 v19 运行与复盘收敛 | M1；与 M2 可交错 | Quiet/Halted、统一健康/错误/指标/每源恢复、日志与测试隔离有目标合同和生产观察；OutcomeTracker 与 AI 评价有稳定证据。 |
| M4 v18 数据—决策—模拟账本闭环 | M1 和 M3 的公共健康/原因合同 | 数据健康、不可变投资决策、统一风控、研究身份、paper order/fill/ledger 和归因共享可追溯身份；旧路径无双 owner。 |
| M5 Gate P | M4 | 远端不可改写审计、保留期及恢复探针、逐日账本对账、point-in-time 和成本后样本外证据、故障演练与模拟观察全部通过。 |
| M6 v20 研究能力决策与必要实现 | M4 的 Decision/Fill owner 稳定；可信数据集 | 每个拟建回测模式/因子/DSL 有独立 PRD、ADR、基准、统计及成本证据；只有被批准的子项实施。 |
| M7 前瞻模拟验证与产品收敛 | M5；候选研究通过 M6 门禁 | 候选策略与既有基准并行观察；每个策略有保留/限制/淘汰裁定和可解释的日/周复盘。 |
| M8 按需扩容 | 经 M7 发现明确需求或实测瓶颈 | 独立产品 PRD 和容量 ADR；完成新旧拓扑的恢复、身份、权限和数据迁移验证。无触发条件则关闭该阶段。 |

```text
M0 ──→ M1 ──→ M2
          ├──→ M3 ──→ M4 ──→ M5 ──→ M7
          └─────────────→ M4 ──→ M6 ──┘
                                    M7 ──(有需求/瓶颈)──→ M8
```

M2 和 M3 可交错，但 v18 不借推送迁移顺手扩大合同；M6 不创建第三套回测/账本 owner。外部来源修复、WORM 设施、交易日自然样本及生产晋级会影响日历时间，不等于编码工作量。

## 4. 分阶段工作包

### M0：证据重基线与立即运行问题

1. 固定 `git`、生产 monitor/桥接/VM binary、bundle、activation、数据库和目录版本；输出源码/部署/真实回执三栏矩阵，记录每项证据时间及 owner。读取生产数据只做受控、脱敏、只读探针。
2. 对照 [2026-09-21 系统评估](../../audits/2026-09-21-系统评估.md) 的 12 条修复项重新标注 `fixed / partial / open / obsolete`。尤其核对成本、买入门、NewsAI、错误分类、静默丢弃、测试隔离；不能复制旧的“未实现”结论。
3. 将 2026-09-28 运行记录中的重复全链验证耗时、桥接未就绪期间的 `no_verified_batch` 欠账、R-08 `Confirmed` 和各 source 的真实批次单列。优化启动只能复用经过验证的检查点或分阶段校验，不能跳过链完整性。
4. 对 T-14/T-15 和依赖真实券商的 owner 做产品裁决：保留永久禁用、改造为模拟事件，或退役；不得注册伪造来源。T-19 仅在上游有可证明的价格区间时考虑恢复。
5. 产出可追踪 backlog：每行有状态、代码入口、业务风险、前置、验收、部署级别和是否需要外部输入；过期文档加时间戳，不改写历史冻结目录。

M0 退出：能回答“当前生产实际在跑什么、缺什么、为什么缺、已有何种恢复证据”；测试噪音有基线，不能将偶发复跑绿当作通过。

### M1–M2：推送系统完整迁移

1. 核验 Foundation：`RunContext`/`PreparedFacts`/`JobDecision`、`CompletionPolicy`、跨 business/durable DB intent-finalizer-reconciler、authority adapter、PhaseScheduler、activation manifest、shadow、readiness 和 operator CLI。缺的仅补窄 slice；P01/N02 高保证状态机先作 conformance adapter。
2. 以 `(producer, occurrence family, completion owner)` 对 52 Unit 做 current delta。保持同一事实一次采集；shadow 比较身份、时间、evidence、suppression、模板版本和 exact bytes，禁止 provider/LLM 二次取数、sink、买卖和游标副作用。
3. 首批按蓝图顺序处理错误完成 owner：CLI BestEffort 结果、09:05/15:30 产业链、AttributionDaily、G5b、15:05 snapshot、CandidateBoard+Invalidated、LimitBoards、ReviewTask mapping、PaperReview Starved conformance。当前源码若已修复，直接补生产证据和清理，不再重写。
4. 余下 Unit 按实际来源、调度和 authority 分类推进；每个 Unit 提交代码、故障矩阵、shadow diff、activation/接管记录、真实回执与上一 Unit cleanup。共享 owner 原子处理；Starved、Opt-in、Inactive 不因 catalog 存在而自动启用。
5. 同一交易日最多晋级一个改变 physical owner 的 Unit；高风险项观察两个 eligible session。发生语义差异、Uncertain、超龄 intent、跨库不一致或双发即停线。回退以 `Draining` 结清旧事实，不删除 receipt/intent、不盲重发。
6. 最终删除旧 physical path 或登记有 owner/到期日的 COMPAT 例外；冻结的 65 kind、新增 kind、枚举外发送、业务 cursor 和真实渠道逐项对账。MachineCatalog 状态升级走版本化目录，不原地修改冻结 v1。

验证：相关 `cargo test --locked --offline --lib <过滤条件>`、`cargo test --locked --offline --bin monitor <过滤条件>`，按改动运行 `monitor --test --push-dry-run`；目录生成器/校验器检查 exact identity。生产门禁还需真实 `TransportAccepted`、同 decision 重放 `AlreadyDelivered`、finalizer 恢复、人工裁定和单 owner 观察；测试通过不能代替它们。

### M3：v19 运行清晰度、结果追踪和 AI 评价

1. 将休市/停机策略收敛到 scheduler 与监督树，定义 Quiet/Halted 对必需健康探针和待处理业务的影响；保留故障下的可观测性。
2. 用单一运行快照承载账户、数据、各来源状态、待裁定投递和进程身份；`--health` 只读、脱敏、有限时延；统一 typed reason/error 和指标口径，避免与 M1 另建 taxonomy。
3. 为真实 source 配置独立 breaker 和恢复字段；先裁定 v19 文档中 5/10 次失败阈值冲突，再加日志轮转、保留期与敏感信息脱敏。每类 typed error 至少有一个反向故障用例；运维告警不依赖正在故障的业务通知 sink。
4. 保留现有 SQLite `prediction_tracker`，抽窄 `OutcomeTracker`；校正交易日而非自然日窗口，补齐信号方向、成功发送与观察样本的不同分母、日/周报告。不另建平行 JSONL truth。是否增加 R10 由 ReviewTask completion owner 裁定。
5. 冻结当前 prompt/model/data 与无 AI 基准，比较 AI 的增量净效果；对仍活跃的 Gemini、多代理或旧 shadow 路径按 `keep / restrict / retire` 决策。历史 IC 数字只作重测线索，不设“必转正”目标。

验证：目标模块测试、monitor 子进程隔离测试、只读 health 时延与故障样本、至少 5 个真实运行日观察；交易日 outcome 还要等待其持有窗口自然结束。

### M4–M5：v18 研究到模拟盘闭环与 Gate P

按一个策略 vertical slice 推进，沿用当前 `data_gateway`、`decision`、`trading::paper_ledger` 和 `performance` owner。

1. **F0 数据与身份 ADR：** 定义事实时间、来源资格、五态数据健康、缺失/迟到/冲突处理及 `InvestmentDecisionId`；与推送的 `PushDecisionId` 分开。为所有投研行动给出同一批次准入/拒绝理由。
2. **F1 数据契约：** 保存数据集版本和健康快照，证明 `as_of`、复权/公司行动、退市/幸存者、市场状态与 benchmark 口径。坏数据拒绝候选和模拟成交，不回退到未标注兼容数据。
3. **F2 不可变投资决策：** 完整 universe disposition、候选证据、成本/流动性、统一 veto 与风险结果、人工裁定及稳定关联 ID 入库；核对现有 `HardLimits`、`VetoChain` 与买入路径的实际阻断效果。推送回执不等于投资或成交确认。
4. **F3 模拟订单与账本：** 在已有独立 PaperLedger 上收敛 parent order、partial fill、no-fill、取消、A 股 T+1/整手/涨跌停/停牌及逐笔费用；只允许一个有效持仓投影 owner，日对账与事件重放给出相同结果。历史坏价走可审计裁定，不覆写原始记录。
5. **F4 归因与模型治理：** 决策→订单→成交→持仓→退出→结果同 ID 可追；研究 run、策略/模型版本和 `Draft → Reviewed → Shadow → PaperChallenger → Promoted/Retired` 状态机保留评审证据及有效期。
6. **F5 Gate P：** 为四模块事实建立远端 WORM/Object Lock authority、至少五年保留验证、daily signed root、恢复读取演练、重放/对账、point-in-time/walk-forward/样本外与成本后证据；故障路径和 CI 门禁逐项有 receipt。对象存储、密钥、预算、保留策略与操作责任先单独 ADR 定案。

验证：数据迟到/改写、双 owner、重复/逆序/坏价/部分成交、重放一致、断点恢复、恢复读取和审计篡改测试；Gate P 还需真实外部保留/恢复证据及约定模拟观察窗。Gate P 通过不自动开放真实交易。

### M6：v20 候选能力的逐项决策与实现

| 候选 | 前置裁决与最小可验收切片 | 扩大的条件 |
| --- | --- | --- |
| 事件驱动回测 | ADR 指定 `strategy::core`、已有 PaperLedger 与回测 runner 的唯一责任边界；共享 FillModel 与同一费用/市场约束，先用历史 paper 样本证明回放结果一致。 | parity 与成本后基准通过后再扩多事件、多策略。 |
| 因子库 | 先选少量有明确假设的因子；记录定义、数据版本、缺失、IC/IR、衰减、分层、容量和多重检验次数。 | 只有独立样本与前瞻证据支持时扩到更多因子；“55 个”和“快 100 倍”须实测。 |
| 回测模式与 DSL | 每种模式各有 PRD、业务规则/阈值证据、golden fixture、样本外门禁；复用现有 selection/strategy contract。 | 同类研究重复需求出现后再做通用 DSL，不以草案示例阈值上线。 |
| 大规模平台与 Web | 先量单机 CPU、I/O、启动时延、查询和用户操作瓶颈；明确产品用户和访问控制。 | 需独立 PRD/ADR、数据迁移与回退演练后才选择 Redis、Postgres、K8s 或 Web；没有需求则保持单机。 |

M6 的工期暂不冻结。先完成 ADR、数据样本范围和一个 working slice 再估总量；旧草案中的 11 周、2500 行、GA 日期不作承诺。

### M7–M8：策略前瞻观察、用户决策与扩容

1. 固定 benchmark、现金/简单持有基准、成本/滑点、风险预算、样本窗口和策略淘汰规则；再运行候选，不看结果后改阈值。评价按市场状态、换手、最大回撤、容量、假阳性和样本不确定性分层。
2. 少量 challenger 与现有路径同时产生模拟记录，记录未成交、数据缺失和人为覆盖；在预定交易日窗口结束后出 `promote / restrict / retire` 裁定。样本不足延长观察，不以几天收益声称稳定优势。
3. 日/周报告展示净值、回撤、候选与基准差、归因、失效源、策略版本和下一步人工动作；从用户使用事实决定是否优化订阅、检索与界面。
4. 只有 M8 触发条件满足时，实施容量/界面项目；迁移必须保持投资决策、paper ledger、通知 authority 的单一 owner 与可恢复性。

策略失败的结果也是完成一次研究：保留可复现的否定证据，淘汰候选并重新提出假设；不为追求正收益改写历史样本或风险阈值。

## 5. 第一轮可执行任务（M0，单人任务）

| ID | 工作 | 估算 | 完成证据 |
| --- | --- | ---: | --- |
| B01 | 固定 `git`/制品/activation/bundle/目录哈希和运行时间点 | 4h | 一份带路径与时间的基线清单；不含密钥。 |
| B02 | 对冻结的 65 kind/52 Unit 与当前新增项生成源码/部署/回执分层差异 | 8h | 可重跑脚本或命令、差异表及无法判定项。 |
| B03 | 重核 2026-09-21 的 12 项修复并定位剩余 caller | 8h | 每项 `fixed / partial / open / obsolete`，附当前代码与目标测试。 |
| B04 | 读取新运行根最近运行窗口的 health、数据准入、投递与欠账摘要 | 6h | 只读脱敏记录；各 source 独立结论。 |
| B05 | 对重复链验证启动成本做一次 profile 与安全优化 ADR | 6h | 启动耗时分解、可保持完整性的不变量与取舍。 |
| B06 | 对 T-14/T-15、R-08 Confirmed、T-19 形成源事实/产品裁决包 | 6h | 各自前置、现有缺口、停用/替代/接入选择及验收样本。 |
| B07 | 汇总 M1 首批 Unit 的 exact owner、intent、故障矩阵及 rollout 顺序 | 8h | 可逐 Unit 执行的任务卡；无重复 ownership。 |

M0 约 46 小时是**初始估算**，不含生产观察与外部等待。每项落地后再拆 M1 的 2–8 小时任务；先做 B01–B04，随后根据证据调整 B05–B07。

## 6. 工期、资源与节奏

- 资源假设：一名主开发者；生产物理 owner 变更时用户或指定操作人在线，外部 VM/对象存储分别有可联系 owner。没有确定外部 owner 时对应工作标为 blocked prerequisite，不按开发日倒推发布日期。
- 历史粗基线仅作容量提醒：蓝图 §24 的推送专项为 36–69 个 8 小时等效开发日、7–10 个交易周 rollout；其后 [推送 WBS](../../push-system/push-system-wbs.v1.json)还出现更大的 provisional 工时。蓝图 §25.9 的 v19 运行面为 15–25 日、复盘/AI 8–15 日、v18 F0–F5 为 45–80 日，均基于较旧状态，不能直接相加或承诺日期。M0 将按实际剩余 Unit、复用代码和外部条件重新估算，增加 20–30% 未知量缓冲。
- 每周更新一次里程碑证据和下一周 2–8 小时任务；每个物理晋级单独评审并等待自然观察窗。源码开发可与观察交错，不压缩单日一个 owner 晋级上限。
- 计划总完成定义为 M0–M7 全部通过且 M8 已作 `实施 / 不实施` 的有证据裁定；不把“所有 v20 草案都写成代码”当作完成标准。

## 7. 风险、验证与回退

| 风险 | 控制与停线点 |
| --- | --- |
| 旧审计与运行版本漂移 | 每个结论附源码、部署和事实时间；历史文件只作为线索，M0 重基线后更新本计划。 |
| 推送双发或游标提前完成 | 单 owner、同事实 shadow、intent/finalizer/reconciler、真实 receipt；Uncertain 人工裁定，Draining 回退。 |
| 模拟收益失真 | 逐笔费用、FIFO、成交约束、数据资格和历史裁定同口径；回测与 paper 共享 FillModel。 |
| 策略过拟合 | 预注册假设和阈值、保留样本外、记录试验次数、前瞻窗口；允许淘汰所有候选。 |
| 生产启动慢/外部源失效 | 拆来源健康与链校验耗时；不绕过完整性，缺事实时 fail closed；先验证恢复欠账。 |
| WORM/扩容改变安全拓扑 | ADR 明确存储 authority、密钥、恢复与费用；先做可逆的 isolated slice 和读取演练。 |

验证按 [AGENTS.md](../../../AGENTS.md) 的改动范围执行：文档只查内容与 `git diff --check`；局部代码跑相应 Cargo target 定向测试；跨模块、投递、交易和持久化边界扩大到故障矩阵与生产观察。已通过的同目标编译结果直接复用。每阶段记录未覆盖的重要部分，不能把 `cargo check`、dry-run 或历史回执当作真实投递/策略有效性证明。

生产回退遵守 [2026-09-28 launchd 迁移记录](../../ops/2026-09-28-monitor-launchd-desktop-tcc-recovery.md)：保持单实例、同一运行根和已有数据库，先检查投递/账本写入与 Uncertain，再决定 binary 或 Unit `Draining`；不能直接切回旧根数据库、覆盖状态或重发不确定消息。

## 8. 更新触发器

发生 Unit 数量/owner、v18 Decision/Fill owner、Gate P 目标、生产运行根、上游来源能力、用户产品方向或策略基准变化时，更新本计划的基线、依赖与估算。每次里程碑完成只将有 fresh evidence 的行标为完成；尚未核验的行保留 `待核实`。

## 9. 2026-09-28 执行进度

| 工作 | 当前状态 | 证据与下一道门 |
| --- | --- | --- |
| M0 B01–B04 | 部分完成 | [当晚事实基线](../../audits/2026-09-28-platform-m0-baseline.md)记录源码、launchd 制品、bundle、目录差异、旧审计复核及部分 route 日志；`scripts/push_catalog_drift.py` 可重跑 65→66 kind 差分。仍缺完整运行窗、真实推送 receipt 与新源码部署层对账。 |
| M0 B05–B07 | 初稿齐备，运行验证待补 | 最近启动的 DB 打开/迁移粗窗约 99 秒，已加阶段耗时日志但未取得新运行样本；[B06 裁决](../../audits/2026-09-28-b06-source-rulings.md)保持 T-14/T-15 禁用、T-19 Starved，T-14/T-15 无来源定时入口已加显式跳过；[B07 首批 Unit 卡](../../push-system/2026-09-28-m1-first-units.md)列出 owner、故障矩阵和顺序，仍待每项 fresh runtime facts。 |
| R-08 全市场公告 | Code Ready，生产未验 | `edc0f144` 完成客户端请求/方法/transport/响应映射，`4a868f91` 约束真实记录 schema；定向 7/7、transport 8/8、method 3/3 通过，只读 mTLS 实连 2026-09-28 `ADMITTED/complete/Cninfo/300`（上游 total 744）。生产 monitor 未换版，缺其真实接收/投递及观察窗。 |
| 模拟费用现行口径 | 纯函数 Code Ready，账本未切换 | [ADR-0001](../../adr/0001-versioned-a-share-fee-schedule.md)冻结旧 `lot-rates-v1`；`4a868f91` 加按成交日的 v2 微元费用与独立研究证据，边界测试 2/2。PaperLedger 新 generation、回测共享计算、旧持仓 cutover 和生产对账未完成。 |
| D14、D17/D20 上游数据 | 等 VM 交付 | 已把 TDX 被拒原始批次、公司行动及权威停复牌事实交虚拟机 Codex；Hithink 正常批次或日线缺口不能替代原始证据。仅收到服务健康反馈，尚无新合同、提交或对应 RPC 样本。 |
| M1–M8 | 未取得本轮退出证据 | 现有局部代码和设计不等于 Foundation、52 Unit、v18/v19/Gate P/v20 或前瞻观察达标。按第 3–4 节前置逐阶段推进，生产晋级与自然观察不能由本轮源码测试替代。 |

当前代码提交不执行 release 部署、模拟账本 seed、推送物理 owner 迁移或业务数据写入。下一开发批次按 B05–B07 和公告生产证据继续，费用 v2 在冻结 v1 回放 fixture 后才进入账本 generation。
