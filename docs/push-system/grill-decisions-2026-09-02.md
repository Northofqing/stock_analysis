# 推送系统 Grill 决策记录

> 状态：已批准的设计输入 / 尚未实现到运行时代码
> 决策时间：2026-09-02 至 2026-09-03
> 决策依据：用户在本项目会话中确认的选择
> 规范实施设计：`push-system-implementation-rfc.md`

本记录固化了约束推送系统文档及后续运行时迁移的选择。选项代表设计决策依据，不代表相应运行时行为已经存在。

## Q1--Q55：首轮推送迁移 Grill

| 问题 | 选择 | 已确认约束 | RFC 落点 |
| ---: | :---: | --- | --- |
| 1 | B | 覆盖活跃及高风险路径的 C0--C6；不激活 24 个 INACTIVE 类型。 | §2, §10 |
| 2 | A | 只有获得可验证且已接受的权威结果后，业务通知游标才可推进。 | §5 |
| 3 | C | 初始按严重级别区分 Uncertain 的处理方式；Q9 随后禁止所有严重级别盲目重发。 | §5, §11 |
| 4 | A | 内部布尔结果迁移为类型化合同期间，保持外部 CLI、配置、订阅和模板兼容。 | §9 |
| 5 | B | 代码完成仅达到 Release Candidate；必须取得受控真实传输证据，才能达到 Production Verified。 | §11, §13 |
| 6 | B | 迁移 P0 调用方前先建立最小结果合同，再增加 finalizer 与 reconciliation。 | §4, §6, §10 |
| 7 | B | 只有能提供类型化、可验证回执的传输通道才具有权威性；其他通道维持 COMPAT/BestEffort。 | §5, §9 |
| 8 | A | 业务库负责通用通知意图与最终化记录；durable DB 保存下游尝试和回执。 | §6 |
| 9 | B | 任何严重级别都不得盲目重发 Uncertain；严重级别只影响检查和升级速度。 | §5, §11 |
| 10 | B | 重放时保持稳定身份和首次渲染字节不变；新的 occurrence 可使用新模板版本。 | §4, §9 |
| 11 | C | 从小版本开始；Q16/Q31 将发布边界从 PushKind 细化为原子 MigrationUnit/晋级。 | §10 |
| 12 | B | CoreUnready 阻断生产；ProducerUnready 隔离单个 producer，同时使部署就绪检查失败并告警。 | §8 |
| 13 | B | 新路径先进入 shadow；同一 occurrence 必须且只能有一个物理 owner。 | §7, §9 |
| 14 | A | 业务迁移只做增量、向前兼容变更；回滚时保留表和证据。 | §6, §7 |
| 15 | B | 每个权威传输通道都要提供受控 Accepted，以及同一 decision 的 AlreadyDelivered/未二次发送证据。 | §11 |
| 16 | B | 原子迁移身份是 producer + occurrence family + completion owner，而不是 PushKind 或源文件。 | §3, §10 |
| 17 | A | 任何 Unit 晋级前，先发布一个不改变物理 owner 的 Foundation。 | §10 |
| 18 | B | 回滚时禁用新 producer/scheduler，但在状态收敛前保留 authority、finalizer、reconciler 和隔离栅栏。 | §7 |
| 19 | B | shadow 精确匹配包括 audience、kind、occurrence、severity、suppression、policy、evidence，初期还包括字节。 | §9 |
| 20 | B | 只有 Accepted、重放、重启、故障、交易时段和 Uncertain 门禁全部通过后，才能在后续版本删除旧路径。 | §7, §10 |
| 21 | A | 编目所有 ACTIVE、STARVED 和 OPT-IN 路径；强路径只做合规校正，仅迁移不合规接线，INACTIVE 保持禁用。 | §2, §10 |
| 22 | C | 优先处理假成功、过早状态变更和语义分裂，再处理 scheduler/template 的用户体验。 | §10 |
| 23 | B | 采用流水线发布：版本 N 清理 Unit N-1，并可晋级 Unit N；最后保留一个尾部清理版本。 | §10 |
| 24 | B | 多个 Unit 可并行 shadow，但每次晋级只能让一个 Unit 获得物理所有权。 | §7, §9 |
| 25 | B | Foundation 阶段完成传输灰盒证明；随后每个 Unit 证明其自然 occurrence，或使用经授权的低频灰盒。 | §11 |
| 26 | B | C0--C6 仅作为能力标签；实施顺序为 Foundation → 垂直 Unit 切片 → 最终清理。 | §10 |
| 27 | B | 只设一个应用端口/结果合同；默认使用通用 coordinator；P01/N02 保留为经过一致性测试的专用 authority。 | §3, §5 |
| 28 | B | 所有定时且非 INACTIVE 的 producer 注册到 PhaseScheduler；事件驱动 producer 注册 trigger/readiness；INACTIVE 不创建 scheduler。 | §8 |
| 29 | B | 只有精确、机器可读的 MigrationUnit 目录完成后，才冻结排期和估算。 | §10, §13 |
| 30 | B | 保留 STARVED 和 OPT-IN 状态；恢复输入或激活必须另做产品决策。 | §2, §8 |
| 31 | B | 一个制品可包含多个 disabled/shadow Unit；一次晋级只能变更一个物理 owner。 | §7, §10 |
| 32 | A | 只有共享原子 completion owner 的路径才可分组，包括已识别的候选、板块、复盘、大宗交易和财报家族。 | §10 |
| 33 | B | 新旧 shadow projection 使用同一份不可变 PreparedFacts，包括已捕获的 LLM 输出。 | §4, §9 |
| 34 | B | 拆分 prepare/project/deliver/finalize；shadow 只能执行 prepare/project。 | §4, §9 |
| 35 | B | 语义差异、重复发送、无法解释的游标移动、陈旧积压、未解决 Uncertain、CoreUnready 或 DB 不匹配均阻断晋级。 | §7, §11 |
| 36 | B | 每个交易日最多晋级一个物理 owner；开发和 shadow 工作可并行。 | §7, §13 |
| 37 | B | 按风险观察：高频路径覆盖完整有效时段/样本；低频路径确定性重放；紧急或有副作用的 Unit 观察两个时段。 | §9, §11 |
| 38 | B | Accepted 到 Finalized 的目标为两个 reconcile 周期，硬上限五分钟；下一次晋级前不得存在超时状态。 | §6, §11 |
| 39 | B | Emergency 告警/解决 SLA 为 1/15 分钟，Important 为 5 分钟/4 小时，Info/Research 须在下一有效时段前完成或标为 NotDelivered。 | §5, §11 |
| 40 | B | shadow 精确比较只排除 attempt ID、延迟和日志时间戳；业务时间取自捕获的 RunContext。 | §4, §9 |
| 41 | A | 保持完整范围，暂估 36--69 工程人日、7--10 个交易周；目录冻结后重新建立基线。 | §13 |
| 42 | B | 分开定义 Foundation Ready、P0 Production Verified、Architecture Release Candidate 和 Program Production Verified。 | §13 |
| 43 | A | 仅在用户或指定操作员在线时晋级；Codex 提供证据和命令，不进行无人值守裁决。 | §7, §11 |
| 44 | A | 按已批准的风险顺序迁移 10 个 P0 Unit，从 CLI BestEffort 和链路报告开始。 | §10 |
| 45 | B | 人工处置使用可审计 CLI，记录决策、结果、已认证操作员、原因和证据；禁止直接修改 DB。 | §5, §7 |
| 46 | B | ManualConfirmedAccepted 与 TransportAccepted 必须区分，前者要求可复核的外部证据及其哈希。 | §5 |
| 47 | B | 操作员身份来自已认证主机/服务身份及生产 allowlist，不得使用自由文本。 | §7 |
| 48 | B | 终态迁移证据至少保留 90 天；非终态证据不得自动清理；更严格策略仍优先。 | §6, §14 |
| 49 | B | 测试命名空间覆盖拒绝、Uncertain、接受后崩溃、幂等 finalizer、重放、回滚和人工处置；禁止破坏性生产故障注入。 | §11 |
| 50 | B | Program Production Verified 要求目录精确、Unit 全部完成、无意外激活/Uncertain/积压/重复、传输证明完备、完成清理并取得新鲜回执。 | §13 |
| 51 | B | 规范 CI 在默认并行模式下不得有无法解释的失败；隔离进程级全局测试，或明确强制串行套件。 | §11 |
| 52 | B | 由唯一、带 schema 版本的 activation manifest 管理 Disabled/Shadow/Active/Draining，并记录其哈希。 | §7 |
| 53 | B | 分别备份和校验 business/durable DB，在 Test 环境演练恢复；不得声称存在跨库原子快照。 | §6, §11 |
| 54 | B | 运行故障通过结构化本地日志、readiness/health 和可查询 CLI 保持可见；外部分页告警与业务回执相互独立。 | §8, §11 |
| 55 | B | 人工证据只保存最小元数据、受保护 URI 和内容哈希；不得保存密钥或非必要的消息/投资组合内容。 | §5, §6 |

## Q56--Q108：文档与实施就绪度整改

| 问题 | 选择 | 已确认约束 |
| ---: | :---: | --- |
| 56 | C | 将当前架构审计与推送实施 RFC 拆分。 |
| 57 | B | 本次改动可涉及文档、生成器、校验器和 CI，但不得改变运行时产品行为。 |
| 58 | A | 纳管覆盖裁决所使用的九份 v18/v19 来源文档。 |
| 59 | B | 证据身份由路径 + symbol + Git 基线/哈希组成；行号仅为派生展示数据。 |
| 60 | A | HTML 必须能从 Markdown、独立模板和本地资源离线重建。 |
| 61 | A | RFC 必须精确到类型、DDL、状态转换、故障矩阵、接口和测试。 |
| 62 | A | 恢复 W01--W21，并补充原子 Unit 的三点估算和依赖。 |
| 63 | A | 保留 55 行原始决策追踪，包含选择、约束、落点和证据。 |
| 64 | A | 证据漂移是失败门禁，不是警告。 |
| 65 | A | 蓝图保留当前事实、65-kind 审计和问题；RFC 负责未来合同与计划。 |
| 66 | A | 人负责业务语义；机器校验 enum、status、symbol、version 和 coverage。 |
| 67 | A | 保持 v18/v19 来源字节不变，在独立目录中保存 SHA 和裁决。 |
| 68 | A | 分别为蓝图和 RFC 构建离线 HTML。 |
| 69 | A | 蓝图是有代码证据的当前快照；RFC 只有通过全部门禁后才能获得 Implementation-Ready 状态。 |
| 70 | A | 正式发布要求干净的 Git commit；脏工作树产物保持 DRAFT/PROVISIONAL。 |
| 71 | C → 已被取代 | Q74 细化了最初的四时段 Unit 方案：时段是 Epic，而非原子切换单元。 |
| 72 | A | PreparedFacts 与 PreparedPush 是分离且以哈希绑定的流水线阶段。 |
| 73 | A | CompletionPolicy 是类型化且由目录管理的合同，不是三值简化或调用方自行解释。 |
| 74 | A | 盘前、集合竞价、盘中、盘后是 Epic，其中包含与原子 owner 对齐的 Unit。 |
| 75 | A | 业务意图状态不照搬 durable 层 14 状态的传输事实。 |
| 76 | A | 业务事务 outbox 配合 lease/CAS 和稳定 decision ID 跨接两个数据库。 |
| 77 | A | expected-version 冲突进入 ResolutionRequired 并阻断晋级，禁止覆盖写。 |
| 78 | A | finalizer 使用重新校验过的 VerifiedTerminalRef，不使用弱化结果枚举或复制的回执。 |
| 79 | A | activation 绑定 build/Git、catalog、两个 schema、template 和 source-contract 版本。 |
| 80 | A | activation 通过 generation/CAS 转换；回滚写入新 generation，恢复上一 owner。 |
| 81 | A | promotion journal 仅追加，记录 manifest、build/schema、Unit、人员、窗口、证据和回滚目标。 |
| 82 | A | 只允许逻辑回滚或 Foundation 兼容的 N-1 回滚；不得删除待处理事实或数据表。 |
| 83 | A | shadow 共享 PreparedFacts、无任何副作用，并比较类型化 decision、hash、reason 和 completion proposal。 |
| 84 | A | 每个 Unit 晋级前必须重新通过 unit/failure/crash/shadow/dedup/rollback 门禁。 |
| 85 | A | CompletionPolicy 包含 owner、推进事件、no-data/disabled/retry 策略和 finalizer kind。 |
| 86 | A | schedule occurrence 关闭与通知游标推进是两个相互独立的事实。 |
| 87 | A | AlreadyDelivered/人工接受只有在精确终态绑定校验通过后才可推进，并使用独立指标。 |
| 88 | A | 按证据类别选择最严格的保留策略；非终态证据绝不自动清理。 |
| 89 | A | intent identity 不含 payload hash；同一身份的不可变 payload/evidence hash 发生变化时必须进入处置流程。 |
| 90 | A | dispatcher claim 使用 lease owner/until/generation CAS 和稳定的 durable 幂等标识。 |
| 91 | A | 提交 JSON evidence manifest，包含 evidence ID、baseline、path、symbol、symbol hash 和派生行号。 |
| 92 | A | strict 检查在 dirty/stale/missing/mismatched 状态下失败；`--draft` 支持临时工作。 |
| 93 | A | 65-kind 目录记录 phase/status/Epic/Unit/producer/schedule/source/authority/policy/evidence。 |
| 94 | A | source catalog 记录 path/SHA/self-version/self-status/ruling/supersession/conflict。 |
| 95 | A | 推送系统制品统一放在 `docs/push-system/`。 |
| 96 | A | 使用一个通用 renderer、独立 template 和本地 Mermaid 构建并检查两份 HTML。 |
| 97 | A | 业务库使用可变 CAS intent 表和仅追加 transition event；两者均不复制回执。 |
| 98 | A | 版本化 manifest 表示期望状态；DB promotion journal 表示已执行事实；启动时审计两者哈希。 |
| 99 | A | Codex 准备证据；用户/指定操作员批准并执行晋级；紧急回滚必须留痕。 |
| 100 | A | 为每个 Unit 生成通用故障案例，并补充业务专属案例，明确恢复和重发预期。 |
| 101 | A | 稳定、带命名空间的 ReasonCode 驱动状态转换、告警、CLI 和测试；说明文字只用于诊断。 |
| 102 | A | 目录冻结后重新计算 Foundation 和 Unit 三点估算，分别给出工程时间、交易时间和日历时间。 |
| 103 | A | 将 v18.2--v18.5 描述为“存放在 v18.x 下、文件自声明为 v20 的版本标签冲突”，不得断言文件放错位置。 |
| 104 | A | 现在构建临时文档；并发来源改动形成干净 commit 后再刷新和发布。 |
| 105 | A | 增加一个本地命令和一项 CI 门禁，检查 baseline/catalog/evidence/source/HTML/decision/WBS。 |
| 106 | A | 纳管 Markdown、template、本地资源和两份生成的 HTML；CI 校验新鲜度。 |
| 107 | A | 将拟议合同、DDL、状态、Unit 和排期移出蓝图；蓝图只保留当前发现和 RFC 链接。 |
| 108 | A | 使用已批准的 `docs/push-system/` 和 `scripts/architecture-docs/` 文件布局。 |

## 决策覆盖规则

- Q9 收窄 Q3：任何严重级别都不得盲目重发 Uncertain。
- Q16 和 Q31 细化 Q11：可部署制品不是晋级边界；原子 MigrationUnit/物理 owner 才是。
- Q74 取代 Q71-C 的字面含义：四个交易时段是规划 Epic，晋级仍按 producer/occurrence/completion owner 保持原子性。
- Q88 收窄 Q48：90 天是推送迁移证据的最低保留期，不得作为全局上限削弱 v18 的五年证据要求。

## 批准记录

用户在 Q108 后确认了完整的综合设计。此后如需变更这些约束，必须新增决策条目；禁止直接改写历史答案。
