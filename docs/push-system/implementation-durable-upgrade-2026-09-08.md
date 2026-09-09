# 旧库升级与恢复审计接续：实施记录

日期：2026-09-08；更新：2026-09-10。状态：本批迁移修复完成，67项测试、静态检查及独立规格/质量审查通过。隔离分支基线 `1868b3f`，源码提交 `77cc3bc568f8502ea9a06a7e03439b1a2eb65b3d`；以下测试针对自有 TEST_CODE 数据库，没有迁移生产数据库或操作生产 monitor。

## 发现与证据

| 问题 | 实际证据 | 当前处置 |
| --- | --- | --- |
| 历史测试夹具的 self-FK 仍指向已改名临时表 | 91580：7 条违规全部指向 `immutable_audit_outbox_v4_historical`，发生在正式 open 之前 | 修正测试创建时的最终表名引用；增加精确 FK 声明断言，保留全库 FK=0 和目标原始快照 |
| 正式迁移全部 SQL 成功，但 COMMIT 被外键拒绝 | 19131：schema 完成 hook=1、version9、foreign_keys=1、defer_foreign_keys=1、FK违规=[]，失败 phase=commit | v4→v5 全库校验为零后，严格在原事务内重置并恢复延迟检查；不关闭 foreign_keys，不吞提交错误 |
| 升级重排后，实际恢复选择了错误的审计前驱 | 55345：正式升级及原始数据/FK/version检查均通过，FenceRevoked 实际前驱8bd0ab…，原唯一逻辑尾6c35cd… | 两次迁移复制都显式保留原 rowid，保留既有跨任务依赖顺序；最终67项合批通过 |

原始日志：[夹具 FK 诊断](../../.superpowers/sdd/2026-09-08-durable-audit-logical-tail/fk-diagnostic-attempt-2.txt)、[正式提交阶段诊断](../../.superpowers/sdd/2026-09-08-durable-audit-logical-tail/formal-open-phase-attempt-4.txt)、[真实接错前驱反例](../../.superpowers/sdd/2026-09-08-durable-audit-logical-tail/logical-tail-behavior-red-attempt-5.txt)。这三类失败不能混记成同一个反例。

## 原因及修复边界

正式迁移入口是 `DurableDeliveryCoordinator::open`，不是关闭 FK 的测试初始化器。当前 `migrate_schema_v4_to_v5` 两次重建 outbox；`enqueue_audit` 以当前 decision 的最大 rowid 选择前驱。因此迁移不能把真实追加顺序改成审计身份字典序。

提交问题另经同一系统 SQLite 动态库的纯内存实验确认：重建后实际 FK 行为零，但延迟约束状态仍为1；COMMIT返回787。零违规检查后 OFF→ON 清除遗留计数，foreign_keys始终为1，提交成功；此后插入真实坏引用仍使 COMMIT返回787，回滚保留原记录。见[可复现源码](../../.superpowers/sdd/2026-09-08-durable-audit-logical-tail/system-sqlite-deferred-probe.c)和[实测输出](../../.superpowers/sdd/2026-09-08-durable-audit-logical-tail/system-sqlite-deferred-probe.txt)。这是原因证据，不替代项目正式入口和负例验收。

本批迁移实现不改 DDL/版本、canonical/hash、原始审计及 receipt，不改变发送权限或 writer 合同。只有全库 FK 验证明确为零才允许重置计数；任何坏引用、查询错误、PRAGMA错误或后续 COMMIT错误仍失败并回滚。临时回滚诊断日志已在取证后删除。

保留 rowid 的原因是兼容性，而非把 rowid 当新的认证材料：现有 W16 正例包含多个由跨 decision 前驱形成的合法链段，局部图不能推导这些链段的追加先后。不能为了新测试通过而删除该正例、任取一个尾或假定时间戳严格递增。

## 当前验证状态

最终合批89874：`67 passed / 0 failed / 3263 filtered`，编译51.37s、运行36.01s，exit0。完整命令、输出和终态见[最终测试记录](../../.superpowers/sdd/2026-09-08-durable-audit-logical-tail/lib-final-fixed-validation.txt)。没有执行整个仓库或未经安全核对的 durable 全套测试。

- 同一真实升级场景恢复成功，全部 audit identity→rowid 和原始目标数据映射不变，FenceRevoked连接原逻辑尾；严格 terminal inspector 返回相同attempt的Uncertain。
- 重复恢复无进展/新审计/新sink；释放旧coordinator唯一Arc后重新打开，读取相同终态且原目标记录和追加观察不变。不是启动生产monitor或重启操作系统进程。
- 正式 open 拒绝缺 predecessor 与非 outbox 坏 FK；失败前后逐项比对原版本、未规范化的原始 DDL、全部用户表的类型化原始行及 outbox rowid，坏引用原样保留，没有静默修库。
- 同一迁移事务重置后新增的真实延迟 FK 仍在 COMMIT 以787被拒绝，ROLLBACK恢复全部原始快照。此负例直接测试真实迁移事务，不冒称它也经正式 open 植入提交故障。
- 既有 Pending 与跨decision依赖、提交/回滚故障、恢复终态及 Generic/P01 的 SLA/指标消费者合批通过。

静态检查62551：lib Clippy exit0，159条既有告警，按级别/代码/消息/主位置比对前批基线无新增、无消失，本批修改区间无诊断；见[完整诊断](../../.superpowers/sdd/2026-09-08-durable-audit-logical-tail/clippy-final.jsonl)及[比对结果](../../.superpowers/sdd/2026-09-08-durable-audit-logical-tail/clippy-final-summary.json)。测试编译保留43条既有告警，早期未使用helper告警已因负例实际消费而消失。定向 rustfmt、git diff --check、八输入校验均通过；没有为消除告警添加 allow。

失败历史不覆盖：55345是目标错尾RED；85534是首条正例GREEN（[日志](../../.superpowers/sdd/2026-09-08-durable-audit-logical-tail/order-fix-positive-green.txt)、[当时源码](../../.superpowers/sdd/2026-09-08-durable-audit-logical-tail/order-fix-positive-frozen.diff)）。首轮整批78187为66通过/1失败（[日志](../../.superpowers/sdd/2026-09-08-durable-audit-logical-tail/lib-final-attempt-1.txt)）：HoldingEvent策略未创建cooldown head，坏引用夹具在正式open前更新0行。修正为真实HoldingPlan/Rolling prepare，并先JOIN独立验证恰有1条head；唯一坏FK和完整回滚断言未削弱，随后89874整批通过。

固定 `1868b3f..77cc3bc` 的[独立规格/质量审查](../../.superpowers/sdd/2026-09-08-durable-audit-logical-tail/task-1-review.md)均为 Approved：Critical=0、Important=0；Minor M1仅保留上述既有告警的后续清理，不扩大本轮修改。审查后源码未变化，沿用同一最终源码的67项及静态证据，没有为文档提交重跑测试。该结论仅限本批迁移修复，不把子集当全项目完成。

## 对项目的提升与仍未完成项

这一批在所验证的真实API数据与历史v4表形状夹具中，保障正式 v4→当前版本升级可提交，且升级后继续恢复时不因迁移重排行序破坏审计接续。它不重发不确定投递、不把未知结果提升为成功。夹具不是历史生产备份，验证不代表实际生产迁移完成。

**已经被旧迁移重排的 v5–v9 库仍需独立兼容处理**：完整单链可从原前驱关系推导逻辑尾；多个独立合法链段缺少原始追加顺序时，不能凭当前版本、hash、timestamp或任意rowid恢复权威。该问题仍属于完整目标，未用本批未来升级修复替代。

2026-09-09只读预检补充：不能用“已有多段照常封口，后续新增审计一律拒绝”作为完整兼容方案。[W16正例](../../src/durable_delivery/tests.rs#L9524)要求global恢复最终到RejectedDurable，而[恢复最终化](../../src/durable_delivery/coordinator.rs#L5772)经[transition_for_reconcile](../../src/durable_delivery/coordinator.rs#L6145)仍到[record_state_transition](../../src/durable_delivery/coordinator.rs#L8450)，需要新增DecisionStateChanged审计；在enqueue处统一拒绝多段会破坏该正例。这个预检建议已撤回，未实现到代码。后续必须同时解决不猜旧重排顺序、合法多段完整恢复与可信顺序/迁移来源；当前尚无已验证兼容方案，不以较容易通过的拒绝行为替代完整目标。以上为源码路径核对，未运行测试、开库或验证生产备份。

### 追加存储能提供的顺序证据

2026-09-10进一步只读核对了当前 append 实现及直接消费者，源码仍为 `aef7972965f610ed418049593dfff1d55341772e`，未读取实际审计文件或生产数据库。这不是旧 v5–v9 兼容修复或运行验收。

| 证据层 | 当前代码能保证什么 | 尚不能据此推导什么 |
| --- | --- | --- |
| 存储记录 | [StoredAppendRecord](../../src/event/durable_delivery_append.rs#L176)保存 kind、identity、canonical 字节/摘要、previous_hash 和 record_hash；[追加路径](../../src/event/durable_delivery_append.rs#L389)从全局尾计算新记录 | 没有独立 decision/ordinal 字段；全局物理链不能直接当每 decision 的业务前驱链 |
| 具体 append 实现 | [共同收尾](../../src/event/durable_delivery_append.rs#L451)同步文件/目录、复核绑定并读回目标；[逐行检查](../../src/event/durable_delivery_append.rs#L546)核对当前文件的链、唯一 identity 与摘要 | 校验现存文件自洽不等于证明历史未被完整截尾或重写；SHA-256 链本身不是外部认证的历史完整性证明 |
| Port 与 DB 引用 | [ImmutableAppendPort](../../src/durable_delivery/model.rs#L1596)只返回 String；[coordinator](../../src/durable_delivery/coordinator.rs#L5686)只要求非空并 CAS 写入；[落库校验](../../src/durable_delivery/coordinator.rs#L9207)检查状态/非空引用一致性 | 数据库里一段非空文本本身不证明记录确实存在、顺序正确或已被该具体实现读回；不能将接口返回类型误称为顺序 receipt |
| 引用和 decision 关联 | [audit_ref](../../src/event/durable_delivery_append.rs#L1147)是 record hash 的字符串引用；[enqueue_audit](../../src/durable_delivery/coordinator.rs#L8596)的 audit identity 纳入 decision、attempt、kind 和 canonical hash | 当前接口没有执行 append 记录与 outbox 的联合顺序验证；即使未来逐项核对这些字段，也不自动证明全部历史记录完整 |
| 读取接口 | [生产私有扫描](../../src/event/durable_delivery_append.rs#L546)仅返回 tail 与目标记录；[枚举 helper](../../src/event/durable_delivery_append.rs#L1086)仅 cfg(test) | 当前公开 Port/恢复消费者没有读取、枚举并消费经验证全局顺序的能力；不是“底层完全不能读文件” |

下一步兼容设计必须区分现存链自洽、可信历史完整性、record 与 decision 的绑定、以及尚未 append 的 Pending 记录。完整可信的历史文件可能提供额外顺序依据，但本轮没有获得或验证此类外部材料；不能认定它们不存在，也不能仅靠现有非空引用恢复旧顺序。合法跨 decision 恢复及最终化新增审计的正向合同仍须保留。

完整 W15/W16/W17/W19、52 Unit迁移及发布门禁、离线蓝图/HTML/checker/实际CI等继续按[当前证据入口](README.md)推进。实施顺序与限制见[本批计划](../superpowers/plans/2026-09-08-durable-audit-logical-tail.md)。
