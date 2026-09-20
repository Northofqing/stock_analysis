# Push Foundation W08 实施计划

> 目标：以 TDD 落地 PreparedPush exact-byte outbox、业务 intent 原子 CAS 与 append-only hash-chain；保持零生产接线。

## Task 1：冻结设计与深模块接口

- 固定 W08/W09/W10 权限边界：W08 public API 不能伪造 terminal authority。
- 固定 Ready/NoData/Disabled 初始行、PreparedPush snapshot、namespace/subject storage、transition event identity/hash。
- 固定 `BEGIN IMMEDIATE`、affected-row、rollback、post-commit readback 和 commit-ack-lost 结果语义。

提交：`docs: design W08 outbox transition store`

## Task 2：RED——exact outbox 与首次幂等提交

- 给 PreparedPush 增加同源 canonical snapshot 合同测试。
- 对不存在的 `InitialIntentIdentity`、`InitialIntentDraft`、`BusinessIntentStore` 写 fresh schema 测试。
- 覆盖 Ready exact snapshot/render/hash、NoData/Disabled NULL group、相同 draft 重试、immutable conflict。
- 先运行目标测试，确认 RED 只来自 W08 symbol/behavior 缺失。

提交：`test: specify W08 exact outbox contract`

## Task 3：GREEN——attested store 与 initial insert

- W07 attestation 提取为同连接可复用的 crate-private helper，不放宽 public migration API。
- `BusinessIntentStore::open` 只打开显式 existing v1 DB；启用并读回 foreign key/recursive trigger。
- PreparedPush snapshot 字段清单与 W05 现有 canonical value 同源。
- initial insert 使用 immediate transaction，提交后 exact readback；retry 不覆盖 immutable facts。

提交：`feat: add exact business intent outbox`

## Task 4：RED——连续 transition 与故障边界

- 写三段连续 event 的 version/event_id/previous/canonical golden。
- 写 stale CAS、competing same-version command、append-only UPDATE/DELETE 拒绝。
- 用仅测试 fault point 覆盖 after-CAS、after-append、after-commit-ack。
- 检查每个失败点前后 row/version/event count/bytes 精确不变。

提交：`test: specify W08 transition recovery contract`

## Task 5：GREEN——同事务 CAS、append 与只读恢复

- typed state、lease action、nonterminal command、outcome/receipt/snapshot。
- immediate transaction 内重读、溢出检查、SQL exact CAS、append event；任何错误整体 rollback。
- stable event 已存在时重算并 exact compare，返回 AlreadyCommitted；不同事实返回 Conflict。
- post-commit 总是 readback，不能仅凭 `COMMIT` 返回成功。

提交：`feat: add append-only intent transitions`

## Task 6：双轴 review 与修复

Standards：模块深度、raw connection 封装、错误脱敏、路径/同连接 attestation、事务生命周期、无 panic、测试 hook 隔离。

Spec：exact bytes、version 0、连续链、stable event、affected rows、rollback、ack lost、drift fail-closed、terminal authority 不泄漏、零 production wiring。

提交：`fix: align W08 outbox with review`

## Task 7：Fresh 验证与中文结果

- W08 目标测试及 W01--W08 foundation/push_job 回归；
- rustdoc、`cargo check --lib`、strict/非致命 Clippy；
- 目标文件 rustfmt check（module root 使用 `skip_children=true`）；
- architecture docs 五组验证器、`git diff --check`、production-wiring relative diff；
- 只读 monitor PID/TCP 观测，不重启、不替换。

新增 `docs/push-system/implementation-w08-results-2026-09-07.md`，更新设计与 `.planning` 后提交。

提交：`docs: record W08 implementation evidence`
