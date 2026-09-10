# Durable 运行期数据库版本防护：实施记录

日期：2026-09-10。状态：本 Task 完成；23 项定向测试及静态检查通过，独立 Spec/Quality Approved。固定 BASE：`8daa8bf2d8490807256a5bb6ed92b0a4ea8a5734`；SOURCE：`8ee1e13fa77895fcb2633a2da2d0c6824aba2042`。仅操作隔离树和自有 TEST_CODE 测试数据库，不操作生产 monitor 或迁移真实业务库。

## 解决什么问题

`DurableDeliveryCoordinator::open` 会检查初始化版本，但已打开实例的 `with_connection` / `with_immediate_transaction` 未在 callback 前要求当前版本。`validate_persisted_immutable_references` 对非当前版本直接返回 Ok，本来支持旧库 bootstrap 的跳过规则也进入了运行期。因此数据库版本发生可观察漂移后，原程序可能继续读写而绕过当前形状的引用校验。证据来自上述 BASE 的 [coordinator 源码](../../src/durable_delivery/coordinator.rs)与[版本初始化](../../src/durable_delivery/schema.rs#L88)，尚不等同于生产事故复现。

本批按[专项计划](../superpowers/plans/2026-09-10-durable-runtime-schema-guard.md)，保留正式旧版迁移入口，新增运行期、事务内及 open 最终成功边界检查。版本相同不代表表/trigger 完整；本批不增加 schema 版本或历史顺序来源。

## 当前实现与逐点证据

| 边界 | 实际实现 | 验证依据 |
| --- | --- | --- |
| Bootstrap 与 Runtime | [私有策略](../../src/durable_delivery/coordinator.rs#L385)仅供内部 wrapper；[open 两处调用](../../src/durable_delivery/coordinator.rs#L2497)使用 Bootstrap，普通入口固定 Runtime，无公开豁免开关 | 正式空库、新增测试 Fixture、旧 schema matrix 及真实 v4 升级均通过 |
| 运行期 callback 前 | [连接引擎](../../src/durable_delivery/coordinator.rs#L4536)在 pre-SQL hook 后检查版本，再允许 callback | [初始可见漂移](../../src/durable_delivery/tests.rs#L2673)、[0/4/10 版本矩阵](../../src/durable_delivery/tests.rs#L2728)、[read 前置计数](../../src/durable_delivery/tests.rs#L2767) |
| 获取写锁后 | [BEGIN IMMEDIATE 后二验](../../src/durable_delivery/coordinator.rs#L4630)在事务 callback 前拒绝，并走既有显式回滚 | [外层检查后漂移](../../src/durable_delivery/tests.rs#L2829)：后续 callback checkpoint 计数为 0；恢复版本后真实重试计数为 1 |
| 写事务提交前 | [统一 precommit 检查](../../src/durable_delivery/coordinator.rs#L4673)对两策略均要求当前版本，再校验持久引用；失败不提交 | [事务内漂移回滚](../../src/durable_delivery/tests.rs#L2893)比对业务、事件、审计、预留及版本，原复合 commit/rollback 与六类引用错误测试仍通过 |
| 成功 read 返回前 | callback 成功后，[连接后验](../../src/durable_delivery/coordinator.rs#L4579)先严格校验版本，再做引用及原隔离后验 | [read callback 后漂移](../../src/durable_delivery/tests.rs#L2953)明确拒绝，不把旧值当成功读回 |
| open 最终成功点 | [最终 attested lease](../../src/durable_delivery/coordinator.rs#L2531)中检查版本，并组合 schema 与隔离双重错误 | [初始化提交后、返回前漂移](../../src/durable_delivery/tests.rs#L3113)使 open 拒绝；不声称回滚已提交的初始化 |

[引用校验函数](../../src/durable_delivery/coordinator.rs#L9306)仍保留非当前版本跳过查询的原分支，未将其改写成新的通用认证器。当前调用点由严格 runtime/precommit guard 挡住版本不符；Bootstrap 通用连接阶段暂不执行版本/引用后验，其收敛依赖原 initialize_schema、提交前检查及最终 open 检查。不能写成“已经删除所有跳过分支”，也不把版本数字等同于 schema 完整性。

效果边界有成对验收：[初始可见漂移](../../src/durable_delivery/tests.rs#L2997)时实际 reconcile 没有业务/audit 变化或新 append；[外部 append 已成功](../../src/durable_delivery/tests.rs#L3056)后，在确认事务内注入版本漂移，DB ack 与版本变更回滚，而外部记录保留，重试复用同一记录。这是受控事务故障，不冒充真实旧 writer 并发切换或生产压测。

## 验证进展

- 基线：既有内存 schema 迁移矩阵实际通过 1 项，43 条既有测试编译告警，实际 exit 0；[命令和退出](../../.superpowers/sdd/2026-09-10-durable-runtime-schema-guard/baseline.meta)、[结果](../../.superpowers/sdd/2026-09-10-durable-runtime-schema-guard/baseline.stdout)。这不是新修复的通过证据。
- 实际行为 RED：正式 open/prepare 后将自有 TEST_CODE 库版本从 9 改为 8，真实 `decision_state` 未按要求返回版本拒绝，tests.rs 当时第 2707 行断言失败。编译成功 2m53s、运行 0.22s、0 passed/1 failed、实际 exit 101；[源码摘要](../../.superpowers/sdd/2026-09-10-durable-runtime-schema-guard/red-source.stdout)、[命令/退出](../../.superpowers/sdd/2026-09-10-durable-runtime-schema-guard/red.meta)、[完整结果](../../.superpowers/sdd/2026-09-10-durable-runtime-schema-guard/red.stdout)。同例还调用了 prepare，但 read 断言首先失败，不能把尚未执行的 write 断言计作第二条实证。
- GREEN：同一反例及全部 9 项新增回归通过，编译 2m15s、运行 1.21s、实际 exit 0，保留 43 条既有测试编译 warning；[命令/退出](../../.superpowers/sdd/2026-09-10-durable-runtime-schema-guard/green.meta)、[完整结果](../../.superpowers/sdd/2026-09-10-durable-runtime-schema-guard/green.stdout)。
- 受影响旧行为：按[固定精确测试清单](../../.superpowers/sdd/2026-09-10-durable-runtime-schema-guard/validate-selected-tests.sh)串行运行 14 项，逐条实际 exit 0；覆盖正式升级、坏 FK/延迟约束、隔离/回滚、引用校验、跨 decision Pending 全局恢复及 v7/v8 策略迁移。每项完整日志以 `selected-1` 至 `selected-14` 分文件保存，不是整个仓库通过。
- 最终验证前已记录[源码 SHA-256](../../.superpowers/sdd/2026-09-10-durable-runtime-schema-guard/final-source-before.stdout)，[验证后摘要](../../.superpowers/sdd/2026-09-10-durable-runtime-schema-guard/final-source-after.stdout)完全相同。父线持有唯一 Cargo 队列，源码冻结期间实施 agent 仅补报告。
- Clippy 实际 exit 0、1m45s，809 条 JSON/188 条既有 warning/0 error；实际 lib artifact 为 fresh=false，build success。按 level/code/message/primary 文件及列位置的多重集比对基线，排除本批插入造成的行号位移，新增/消失均为 0；[完整诊断](../../.superpowers/sdd/2026-09-10-durable-runtime-schema-guard/clippy-final.stdout)、[比对结果](../../.superpowers/sdd/2026-09-10-durable-runtime-schema-guard/summary-final.stdout)。不是 strict `-D warnings` 或全 targets/features CI 通过。
- 定向 rustfmt、git diff --check、冻结八输入校验均实际 exit 0；schema.rs/model.rs 无 diff。只读摘要脚本首次因本机 Ruby 不支持 tally 而失败，改用兼容分组后解析同一份原始日志成功，没有重跑 Cargo 或改源。
- 固定 BASE..SOURCE 的[独立规格/质量审查](../../.superpowers/sdd/2026-09-10-durable-runtime-schema-guard/task-1-review.md)均为 Approved，源码 Critical/Important/Minor 均为 0。审查发现一处实施报告 tests.rs 摘要误抄；唯一 writer 仅纠正文档，reviewer 对同一元数据更正完成只读复核。原始验证前后摘要始终一致，未改源或重跑测试。188 条旧 Clippy 告警作为非新增、非阻断 Minor 保留。

验证前静态检查曾发现重复 helper 定义，退回唯一 writer 去重；另增强两条阶段测试，以 callback checkpoint 独立计数证明前置拒绝，而不是仅用事后回滚零行作证明。这些修正均发生在 GREEN 编译前，没有伪记为编译失败或额外 RED，也没有覆盖最初的真实行为 RED。

## 边界

初始已有可见版本漂移时应拒绝实际 prepare/reconcile，不产生该调用的新业务变化或 append。多阶段调用中若外部 append 已经成功，后续版本拒绝只能阻止 DB ack，不能倒退外部效果。该防护也不能撤销旧二进制、完成旧 v5–v9 审计兼容或代替生产切换。完整目标继续保留。
