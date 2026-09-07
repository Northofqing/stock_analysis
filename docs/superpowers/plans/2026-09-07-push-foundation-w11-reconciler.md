# Push Foundation W11 实施计划

**目标：** 以 tracer-bullet TDD 实现全业务日 startup reconciler、exact lease/fence 接管、W09/W10 强终态恢复和 Uncertain 零重发隔离；保持零生产接线。

## Task 1：冻结设计与恢复边界

- 核对 WBS W11、RFC 全日期恢复/故障矩阵/最终化边界、蓝图 startup fixed point 和 W08--W10 API。
- 固定 W11 不依赖 W12、不调用 sink；PendingDispatch 只形成 typed boundary。
- 固定 keyset 全日期扫描、lease takeover、重复阻断事件与 fixed-point 语义。

提交：`docs: design W11 startup reconciler`

## Task 2：扫描与 Lease 第一组 RED

- 新增 `reconciler_tests.rs`，先引用不存在的 recovery scan/config/report API。
- 用多个日期、page size=1 证明完整覆盖及稳定顺序，终态排除。
- 测试无 lease claim、expired takeover、same-owner live reuse、foreign live boundary。
- 断言 generation/version/event chain、旧 owner/until CAS 和 foreign 零 authority query。

提交：`test: specify W11 all-date lease recovery`

## Task 3：扫描与 Lease GREEN

- 在 store 增加私有 `RecoveryCursor`、attested keyset page 和 exact claim 方法。
- 新增 `reconciler.rs` 的受校验 config、fence、typed boundary 与报告。
- 实现单轮全日期扫描、state/version 前后 progress 计算和 fixed-point 外循环。
- PendingDispatch 只返回 DispatchPending；ResolutionRequired 只返回人工边界。

提交：`feat: scan and fence W11 recovery intents`

## Task 4：Authority/Finalizer 第二组 RED

- fake binding port 提供 W09 template/policy/authority，不提供 sink。
- 测试 AwaitingAuthority Accepted 二查完成、AwaitingFinalizer restart 完成。
- 测试 Rejected 只写一次、Uncertain 隔离、NotDelivered 缺 audit 不自动完成。
- 测试 missing/pending/unavailable/binding drift 只写一次 invalid evidence并到 fixed point。

提交：`test: specify W11 terminal reconciliation`

## Task 5：Authority/Finalizer GREEN

- 用 W10 prepare/commit 驱动 accepted；不复制 finalizer 或 terminal verifier。
- 在 store 增加 exact-fence recovery observation transition，只允许 RFC 指定组合。
- Rejected/invalid 当前 reason 相同则只读重查、不追加；Uncertain/资格后处置漂移进入 ResolutionRequired。
- 映射 W10 duplicate、conflict、terminal invalid 和 already-finalized 结果为 typed recovery outcome。

提交：`feat: reconcile W11 terminal authority`

## Task 6：Fixed-point 与竞争强化

- 测试一条 intent 同一 startup 中按 Pending/Awaiting/Completed 状态推进时不会漏扫其他日期。
- 测试 CAS winner 改变 state/version/lease 后旧 fence 不被使用。
- 测试持续 progress 超上限失败关闭；报告按 date/ID 去重、脱敏且保留所有 blocker。
- 证明多轮 Rejected/invalid/ResolutionRequired 不形成无限事件链。

提交：`test: harden W11 fixed-point recovery`

## Task 7：双轴评审与修复

### Standards

- scan 是否有 current-date/lookback 隐含过滤，cursor 是否可跳过前缀；
- store mutation 是否全部绑定 state/version/generation/owner/until；
- reconciler 是否泄露 sink、raw connection、payload、subject 或任意 capability constructor；
- 错误是否 typed/fail-closed，生产代码是否无 panic/unwrap/expect。

### Spec

- 对照 W11 acceptance 与 RFC failure matrix 逐项变异 state/date/owner/until/generation/version/disposition；
- 证明 Uncertain、attempt unknown、Rejected、foreign lease 均无重发入口；
- 证明 AwaitingFinalizer 每次恢复重新查询，不复用 prior memory；
- 证明 ResolutionRequired 不自动 clearance，W11 不推进 Unit cursor。

提交：`fix: align W11 reconciler with review`

## Task 8：Fresh 门禁与结果文档

- W11 精确测试；W07--W11 Foundation 回归；push_job/rustdoc/check；
- strict/nonfatal Clippy attribution、定向 rustfmt、diff check、架构五组验证器；
- 相对 W10 检查 monitor/notification/durable/config/Cargo/冻结 SQL 零差异；
- 只读确认原 release monitor PID/连接/当日 push log，不重启、不热替换；
- 新增中文 W11 实施结果，更新设计和 `.planning`。

提交：`docs: record W11 implementation evidence`
