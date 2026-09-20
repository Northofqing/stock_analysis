# Push Foundation W10 实施计划

**目标：** 以 tracer-bullet TDD 实现通用 finalizer、业务 CAS 冲突隔离、terminal transition 与提交确认恢复；保持零生产接线。

## Task 1：冻结设计和接口边界

- 核对 WBS W10、RFC 最终化事务、冻结 DDL、W08 store 和 W09 capability。
- 固定两阶段接受完成、直接不投递终态、exact lease fence、重复/冲突语义。
- 明确 generic Completed 不等于 52 Unit 的领域 cursor 已迁移。

提交：`docs: design W10 business finalizer`

## Task 2：接受完成第一组 RED

- 新增 `business_finalizer_tests.rs`。
- 构造临时 attested business DB、Ready intent、AwaitingAuthority lease 和两次返回相同 Accepted 的 fake authority。
- 先引用尚不存在的 `FinalizerFence`、prepare/commit API 和 terminal outcome。
- 断言两次 query、AwaitingFinalizer/Completed 两个连续事件、terminal ref/binding、lease release 和 completion directive。
- 添加 ManualConfirmedAccepted 与非接受 disposition fail-closed 用例。

运行精确测试，确认 RED 只来自 W10 能力缺失。

提交：`test: specify W10 accepted finalization`

## Task 3：最小接受 GREEN

- 新增私有 `business_finalizer.rs`，实现不透明 pending capability 和 typed outcomes/errors。
- 在 `intent_store.rs` 增加 crate-private authority qualification 与 terminal transition command。
- 复用 W08 单事务 CAS + append、event ID/hash chain、post-commit 读取和完整链验真。
- prepare 调用 W09 首查，commit 消费 pending 后调用 W09 二查；两次均重新验证 policy。
- ResolutionRequired 恢复要求 opaque、绑定当前 intent/version 的 `VerifiedResolutionClearance`；production constructor 留给 W18。
- Completed event 只保存 terminal ref/disposition/binding，不复制 receipt bytes。

运行 W10 目标测试和 W07--W09 回归。

提交：`feat: finalize accepted business intents`

## Task 4：冲突/恢复第二组 RED

- stale version 或 lease 竞争后 commit，要求真实冲突进入 ResolutionRequired。
- 两个相同 pending finalize 竞争，要求只有一条 Completed，第二个返回 exact existing terminal receipt。
- 同 event ID 不同 terminal material 不得返回 AlreadyCommitted。
- CAS 后、append 后必须整体回滚；commit ack lost 必须读取原 terminal event 恢复。
- Completed 后禁止 NotDelivered/回退；隔离时保留原 terminal event。

提交：`test: specify W10 conflict and crash recovery`

## Task 5：冲突/恢复 GREEN

- 主 CAS 零行先 rollback，再查 exact event/current/chain。
- exact duplicate 返回 AlreadyCommitted/AlreadyFinalized。
- 真实冲突以重读 state/version 单独 CAS + append `finalizer.cas_conflict` 到 ResolutionRequired。
- 隔离二次竞争不循环，返回 `ConflictUnresolved`。
- terminal fault injection 只保留在 `cfg(test)`；确认丢失走 exact recovery。

提交：`feat: isolate W10 finalizer conflicts`

## Task 6：NotDelivered RED/GREEN

- 定义 opaque `VerifiedOperatorAuditRef`；production 构造权留给 W18，本切片仅测试 fixture。
- 测试 AwaitingAuthority 与“由 AwaitingAuthority/transport.uncertain 进入的 ResolutionRequired”两个合法来源。
- 测试已有 AwaitingFinalizer/Completed 历史、错误 decision、缺 audit、错误 disposition 全部失败。
- 实现两次 W09 requery 后的直接 NotDelivered terminal CAS，保存 decision/audit/terminal 组并释放 lease。
- 断言 schedule KeepOpen、cursor Never、无重发资格、失败指标语义。

提交：`feat: finalize verified not-delivered intents`

## Task 7：双轴评审与修复

### Standards

- public seam 是否泄露 raw SQLite、任意 terminal parts、receipt bytes 或 production path；
- finalization/pending/operator capabilities 是否可被普通调用方复制或伪造；
- SQL 任一错误是否整体回滚；错误是否泄露业务值；
- event canonical/hash 是否复用唯一实现；生产代码无 panic/unwrap/expect。

### Spec

- 对照 W10 acceptance、RFC transition/terminal/failure matrix 逐条审计；
- 变异 state/version/fence/ref/binding/disposition/actor/time/audit/source history；
- 证明 Accepted history 不会被 NotDelivered、回退或新发送覆盖；
- 证明 compatibility/weak evidence 无入口。

提交：`fix: align W10 finalizer with review`

## Task 8：Fresh 门禁与结果文档

- W10 精确测试；W07--W10 foundation 回归；W01--W08 push_job 回归；
- rustdoc compile-fail、`cargo check --lib`、strict/nonfatal Clippy attribution；
- 定向 rustfmt、diff check、五组 architecture docs 验证器；
- 相对 W09 检查 monitor/notification/durable/config/Cargo/冻结 SQL 零差异；
- 只读确认原 release monitor PID/连接/当日 push log，不重启、不热替换；
- 新增 `docs/push-system/implementation-w10-results-2026-09-07.md`，更新设计和 `.planning`。

提交：`docs: record W10 implementation evidence`
