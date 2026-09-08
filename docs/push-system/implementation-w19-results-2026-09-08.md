# W19 持久化最终化延迟检查：实现与验证记录

日期：2026-09-08。状态：本计划 Task1 已完成；独立审查发现的历史一致性缺口已补反例并修复，修复后78项测试及静态检查通过，限定复审全部关闭。本文件不证明完整 W19 或生产接线完成。

后续验收补充：全量库存任务发现并修复 N02 年度reader在lock缺失时创建文件的问题（8094ffc），以及Completed+Conflict遗漏当前未决年龄的问题（c10ec78）。两个新增反例均先真实失败再不改期望通过；最终107项相关测试、静态检查和限定复审通过。下文78项是此前证据，不覆盖这两个新增场景；新证据见[全量库存指标记录](implementation-w19-inventory-results-2026-09-08.md)。

开发目录：`.worktrees/push-reliability-20260905`，分支 `codex/push-reliability-20260905`。计划：[W19 原始接受时钟与持久化最终化延迟联查](../superpowers/plans/2026-09-08-push-foundation-w19-finalization-sla.md)。原始基线 `35d12d0`，计划提交 `5e0c6b5`，初版源码 `27967be`，历史一致性修复 `6f8713c`。

## 要解决的问题

投递系统已经取得回执，不等于业务侧已经安全完成。这两件事之间可能发生进程崩溃、提交确认丢失或业务版本冲突。如果用查询时间、创建时间或“进入等待最终化状态”的时间代替原始接受时间，会掩盖真实积压，重启后也可能把超时重新计为零。

本次将已有真实持久化来源与业务转换链连起来，回答同一条 intent 何时被实际接受、何时完成、当前是否仍未决以及延迟是否越界。它只报告事实，不发送、不恢复、不删除、不切换完成所有者。

## 数据来源与绑定

| 来源 | 原始接受时刻 | 本次读取要求 |
| --- | --- | --- |
| Generic | 已封存的 exact TypedReceipt.accepted_at | 复用实际 coordinator reader 与 W09 校验；时间从传给同一次 W09 校验的证据提取。 |
| P01 专用投递 | 已封存的 exact TypedReceipt.accepted_at | 保留同业务日 claim、原 envelope、channel 与专用 authority 的既有精确绑定。 |
| N02 聚合窗口 | 已封存 terminal envelope 的 remote receipt.accepted_at | 复用窗口 attempt/terminal 链、reservation、原始字节和 channel 校验；不拿新闻来源时间或窗口时间代替。 |
| 业务最终化 | 完整转换链中最早合法 Completed.occurred_at | 与当前 authority 的 terminal ref、binding hash、disposition 精确一致；不跳过冲突事件挑选有利结果。 |

业务 intent 与完整转换链必须在已有业务连接的同一读取事务内验证，包括版本、前驱 hash、事件合法性与当前链头。来源库与业务库是独立读取，不声称跨库原子快照。既有业务连接的创建过程也不因此变成“纯只读 opener”。

N02 当前查询只支持显式的 `news-flash-window` family 与 `key=window.label()` 局部约定。其他 family 返回“不支持的 occurrence 路由”，已支持 family 的错 key 返回路由不匹配。现有证据只证明此前测试使用此约定，不能将其声称为已注册的生产合同；任意调用方自报 family/key/window 对应关系也不能作为来源证明。完整 N02 迁移仍需真实 producer 的身份构造和路由接线。

## 状态和时间规则

- authority 已 Accepted，即使业务仍为 PendingDispatch 或 AwaitingAuthority，也属于待最终化样本，必须从原始接受时刻计龄。
- 已完成样本取原始接受到最早精确匹配 Completed 的差值；之后查询或追加同态事件不能刷新延迟。
- 人工接受、Rejected、Uncertain、人工确认未投递、Missing、PendingSeal 与正常 transport Accepted 分开；未封存记录不能贡献可信接受时刻。
- 历史 Completed 不能遮蔽当前 ResolutionRequired；Ready 来源后来处于 NoData/Disabled/NotDelivered 而 authority 已接受，是需要报告的矛盾。
- 不只检查 Completed：历史 NotDelivered 的 ref/binding/disposition 及已有 decision 引用也须和当前来源一致；已有 AwaitingFinalizer 接受资格不能与当前非接受来源并存。来源 Missing/PendingSeal 与既有资格或终态历史冲突时，ResolutionRequired 不遮蔽该冲突；正常人工处置仍保留其独立状态。
- 缺失/无法换算的时间、观察时钟早于已有事实、完成早于接受，不能通过负数截零伪装为按时。
- 周期要求为非零、可精确表示为微秒且倍增不溢出。来源时间投影到 UTC 微秒，亚微秒部分向下取整；不按秒四舍五入。

| 指标 | 判定 | 含义 |
| --- | --- | --- |
| 两周期目标超出 | `elapsed > 2 × reconcile_cycle` | 暴露目标超时；等于目标尚未超出。 |
| 五分钟硬上限到达 | `elapsed >= 300 秒` | 恰好五分钟即报告到达，不延后一微秒。 |
| 超龄未决需要阻断 | 当前未完成且从接受至观察已到硬上限 | 供后续晋级检查消费，不自动执行晋级或回滚。 |

已完成但迟到的样本保留阈值事实；人工处置和当前未决状态不被算作正常完成。时序检查只基于已存事实和显式观察时钟，不证明跨重启可信时钟或防回拨水位已经部署。

## 副作用与披露边界

检查不得获取恢复 lease、执行 finalizer、调用 sink/append、推进业务游标或清理证据。输出只保留身份、状态、版本/链头、不可变证据引用的 hash 与计时事实；Debug/Error 不披露正文、持仓、模型内容、webhook、路径、原始数据库错误或远端 message_id。

## 当前验证状态

- 首轮 session93863 因新测试临时 UnitId 借用导致 E0716 编译失败，测试未运行。原实现代理只改为长寿命局部绑定，保留全部拒绝断言与实际 fixture，不算行为测试 RED。
- 修复后 session13333 exit0：**77 项通过、0 失败、0 ignored**；包括 SLA14、Generic11、Dedicated8、终态验证12、业务最终化18、转换链14。编译3m51s，运行12.10s。43条既有 data_gateway 告警，本批文件无诊断。
- 六份变更 Rust 文件定向 `rustfmt --check --edition 2021 --config skip_children=true` 与 `git diff --check` 通过。
- lib Clippy session21762 exit0，1m22s；完整166条输出记录均可解析，163条既有带位置告警，`src/push_foundation/` 前缀诊断为零。不声称全仓无告警。
- 初版固定原始 `35d12d0..27967be` 独立审查：Spec不通过、Quality Needs fixes，Critical0 / Important1 / Minor1。Important 为只核对 Completed 历史，遗漏 NotDelivered 的终态材料和 AwaitingFinalizer 接受资格与当前来源的矛盾；既有77项测试没有覆盖该缺口。该缺口随后按下方真实RED→GREEN及限定复审关闭。Minor是既有warning，留最终全分支审查。

修复后的权威验证覆盖初版结果：

- 先在实现未改时运行新回归：session6260 编译成功、实际1项失败，17种组合中13个矛盾状态被收集，4个正常组合匹配。使用真实业务writer/finalizer历史、重启和当前实际coordinator读取，失败不是编译问题。
- 原实现代理最小修复后，不改原反例期望，session37922 **78项通过、0失败、0 ignored**（15项SLA+63项相邻），编译56.74s、运行18.10s；43条既有data_gateway warning，本批无诊断。
- 最终 lib Clippy session84362 exit0，1m28s；完整166条记录无坏JSON，163条既有warning，本批目录零诊断。原21762不替代修复后证据。
- 修复源码为6f8713c。固定 `27967be..6f8713c` 独立限定复审：原Important **ADDRESSED**，无新增Critical/Important/Minor、无范围外观察；复核保留正常人工处置/接受路径、完成时间冻结及缺失/未封存冲突优先级。Task1 关闭，既有warning仍交最终全分支审查。

实现及反例入口：`src/push_foundation/finalization_sla.rs:248`（实际联查）、`:353`（全链历史一致性）、`src/push_foundation/intent_store.rs:2002`（单读取事务）、`src/push_foundation/finalization_sla_tests.rs:562`（三来源重启/无写入）、`:622`（微秒边界）、`:1185`（历史矛盾17组合）。这些是当前源码证据位置，不是生产运行证明。

RFC 输入检查已执行：`ruby scripts/architecture-docs/check-rfc-inputs.rb --root .`，exit0、`rfc_inputs_valid`。该检查只证明冻结输入一致，不证明新代码行为。

## 尚未完成的完整范围

W19 仍需指标汇总、生产健康/晋级消费、Uncertain 人工响应 SLA、保留期/法律保留/独立备份完整性与安全审计。监管留存与 WORM 的外部能力不能由这个只读检查代替。

W15/W16 真实身份与来源、完整 W17 业务适配及真实端口接线、W18/W20/W21、52 个 Unit 迁移和实际发布门禁继续保留。没有启动或操作生产 monitor、真实数据库、provider、sink、PAM 或生产部署。
