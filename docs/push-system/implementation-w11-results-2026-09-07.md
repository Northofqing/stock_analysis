# 推送 Foundation W11 实现与验证结果

**结论：** W11 已完成启动恢复协调器。它按 `(business_date, intent_id)` 对全部业务日期执行稳定 keyset 扫描，只处理 `PendingDispatch`、`AwaitingAuthority`、`AwaitingFinalizer` 和 `ResolutionRequired`；缺失或过期 lease 通过同态 CAS 取得新 generation，存活的外部 owner 绝不接管。对于已经发出但尚未完成的意图，协调器只查询 W09 强终态 authority，并复用 W10 两阶段 finalizer；它没有 sink、renderer、provider 或 resend 能力，因此启动恢复不能变成隐式重发。

**生产边界：** 本切片没有接入 monitor、notification、durable delivery concrete adapter、配置、Cargo 清单或生产数据库，没有修改冻结 DDL，也没有迁移任何 Unit。`push_foundation::reconciler` 仍是 crate-private 且未被生产入口调用，现有实际推送逻辑不受 W11 影响。

**验证日期：** 2026-09-07（Asia/Shanghai）

**分支/worktree：** `codex/push-reliability-20260905` / `.worktrees/push-reliability-20260905`

## 1. WBS 验收与代码证据

W11 的验收目标是：启动时扫描全部未完成业务意图，按 lease/fence 安全恢复；只用强终态完成业务状态，`Uncertain` 必须隔离，任何恢复过程都不得产生重发权限。

| 验收点 | 实现结果 | 代码证据 |
| --- | --- | --- |
| 有界配置 | lease 到期、页大小和 fixed-point 次数均在入口校验；页大小上限 1000、迭代上限 100 | `reconciler.rs:55-97` |
| 脱敏恢复报告 | 只暴露日期、intent ID、状态、version、lease generation 和边界，不包含 payload、证据或 SQL | `reconciler.rs:99-170` |
| 脱敏错误 | finalizer 错误映射为稳定 check 名，不把 authority record、snapshot 或 evidence 放入错误 Debug | `reconciler.rs:173-230` |
| 全日期 keyset 扫描 | 以 `(business_date, intent_id)` 排序翻页，不依赖“今天”或单日窗口，只选择四种可恢复状态 | `intent_store.rs:763-775,1088-1146` |
| fixed-point 收敛 | 每轮完整扫描；只有整轮 version 无进展才成功，达到上限仍有进展则 fail closed；计数 checked-add | `reconciler.rs:232-284` |
| 状态路由 | PendingDispatch 只报告边界；Authority/Finalizer 才查终态；ResolutionRequired 保持人工边界 | `reconciler.rs:286-316` |
| lease 接管 | 仅缺失/过期 lease 可同态 CAS 取得新 generation；同 owner 活 lease 复用，外部活 lease 只报告 | `reconciler.rs:674-743`、`intent_store.rs:1148-1197` |
| exact fence | recovery observation 与普通 transition 均复核 state、version、owner、generation、until；SQL CAS 再复核当前 lease | `intent_store.rs:1199-1250,1642-1731` |
| Accepted 恢复 | 调用 W10 prepare + commit，保留两次 exact authority 查询与业务 CAS，成功才到 Completed | `reconciler.rs:318-485` |
| 注册绑定 fail closed | template、completion policy 或 authority allowlist 漂移在查询前终止启动，不伪装成可恢复 blocker | `reconciler.rs:467-495` |
| 非接受处置 | Rejected 只留一次等待授权事实；Uncertain 进入 ResolutionRequired；NotDelivered 等待 W18 审计 | `reconciler.rs:497-563` |
| authority blocker 稳定 | 暂态查询失败或 durable record 无效只追加一次 invalid observation，后续 fixed-point/read restart 不增长链 | `reconciler.rs:565-629` |
| 竞争处理 | CAS 竞争依据重读 snapshot 返回真实边界，不携带 payload，也不把竞争解释为发送许可 | `reconciler.rs:631-672` |

## 2. 启动扫描与 lease 规则

扫描不接受业务日期参数。每页 SQL 只选择四类非终态/隔离态，并按 `(business_date, intent_id)` 升序；下一页 cursor 同时保存两个字段，因此同一日期多条 intent 和跨日期 intent 都不会因上一页写入导致 offset 漂移。每个候选读出后还会再次验证状态并校验完整 transition hash chain；扫描期间若状态被并发改成集合外状态，协调器 fail closed，而不是使用过期快照继续操作。

lease 规则是：

- `ResolutionRequired` 不获取 lease，直接保持人工处理边界；
- 有效且属于本恢复 owner 的 lease 原样复用，不延长、不增加 generation；
- 有效且属于其他 owner 的 lease 不接管，不查询 authority，不写事件；
- lease 缺失或已过期时，以当前 state/version/lease 精确 CAS 做同态转换，并由 store 增加 generation；
- CAS 竞争后必须采用返回的当前 snapshot 重新判断；若另一 owner 已取得活 lease，则立即停在外部 owner 边界。

W11 没有定义新的可复制 lease capability。它从已验证 snapshot 读取 owner、generation、until 和 version，按值构造 W10 crate-private `FinalizerFence`，或把同一组材料交给 store 的受控恢复方法；真正写入仍在事务临界区对完整 fence 做第二次验证。

## 3. 状态与终态处理矩阵

| 当前状态 / authority 结果 | W11 行为 | 持久化结果 | 是否允许重发 |
| --- | --- | --- | --- |
| PendingDispatch | 仅报告 `DispatchPending` | 除必要 lease claim 外不改变业务状态 | 否；W11 无发送接口 |
| AwaitingAuthority / Accepted | W10 prepare 后 W10 commit 二查 | `AwaitingFinalizer -> Completed`，释放 lease | 否 |
| AwaitingFinalizer / Accepted | 从数据库重新构造资格并执行二查 | `Completed`，不复用进程内旧 capability | 否 |
| AwaitingAuthority / Rejected | 追加一次 `transport.rejected` | 保持 AwaitingAuthority，等待显式授权策略 | 否 |
| AwaitingAuthority 或 AwaitingFinalizer / Uncertain | 追加隔离事件 | `ResolutionRequired` | 否 |
| AwaitingAuthority / ManualConfirmedNotDelivered | 报告 `OperatorAuditRequired` | 状态不变，等待 W18 签发审计 capability | 否 |
| AwaitingFinalizer / Rejected 或 NotDelivered | 视为与既有 accepted qualification 冲突 | `ResolutionRequired` | 否 |
| authority 暂不可用/记录无效 | 追加一次 `finalizer.terminal_ref_invalid` | 保持原状态，后续启动只读复核 | 否 |
| 注册 template/policy/authority 漂移 | 启动报错 | 查询前零写入 | 否 |
| ResolutionRequired | 只报告人工处理边界 | 零写入 | 否 |

`Rejected` 不是“可重试”。W11 只持久化已经观察到的处置，真正的 retry authorization 仍必须经过 W03 policy 及未来 Unit 迁移的领域事实。`ManualConfirmedNotDelivered` 也不能被启动器直接完成，因为生产级 operator/audit capability 属于 W18。

## 4. fixed-point、竞争与幂等

一次启动恢复由最多 `max_iterations` 轮全量 keyset 扫描组成。任何 intent 的 version 增长都视为本轮有进展；只有一整轮没有 version 增长才返回成功报告。若最后允许的一轮仍有进展，返回 `IterationLimitExceeded`，不能用“跑过 N 轮”冒充已经收敛。transition 总数使用 `checked_add`，理论溢出同样转成完整性错误。

幂等与竞争规则如下：

- 相同恢复 owner 的 lease 被并发续期或 generation 改变时，旧 fence 不能进入 authority 路径；协调器采用竞争返回的 snapshot，在下一 fixed-point 轮重新判定；
- authority blocker、Rejected observation 和 ResolutionRequired 隔离都通过当前 reason/state 防重复追加；
- 第二次 authority 查询失败时，复用 W10 的 invalid receipt，只持久化一条证据，后续启动保持只读；
- 第二次查询由 Accepted 漂移为其他 disposition 时，不完成业务 intent，而是按实际结果进入对应隔离边界；
- W10 CAS 冲突返回的 snapshot 会被立即脱敏映射成边界，恢复报告不会夹带业务 payload；
- 重复启动在稳定边界上不延长 lease、不增加 version、不增长 transition chain。

## 5. TDD、评审与提交

| 提交 | 内容 | 结果 |
| --- | --- | --- |
| `45c6011` | 冻结 W11 启动恢复设计和逐测试计划 | 明确全日期扫描、零 resend、lease/fence、authority 与 fixed-point 边界 |
| `ea738d3` | 全日期扫描与 lease 第一组 RED | API 缺失按预期失败 |
| `b16b506` | 扫描/lease GREEN | keyset scan、claim、活 lease 保护和 PendingDispatch 边界 |
| `bab3817` | authority 恢复第二组 RED | 固定 Accepted/Rejected/Uncertain/NotDelivered 行为 |
| `44fd21e` | authority/finalizer GREEN | 接入 W09/W10 内核但保持零 concrete adapter/零生产接线 |
| `38f4054` | fixed-point 与重启第三组 RED | 暴露竞争快照、迭代上限、重复启动和二查漂移缺口 |
| `f4f6e28` | fixed-point 冲突修复 | 重读竞争 winner、稳定 blocker 和二查漂移隔离 |
| `a6c525b` | 双轴评审修复 | 注册绑定漂移 fail closed，错误和冲突报告脱敏 |
| `c72bac9` | 计数溢出 fail closed | 引入 checked transition counter；首次编译暴露整数字面量类型不明确 |
| `823b179` | 显式声明计数类型 | 修复上述编译错误，15 条 W11 测试重新全绿 |

评审确实发现并关闭了三个问题，而不是只做形式检查：

1. 初版冲突路径可能使用旧 snapshot，且错误类型可能把完整业务 snapshot 带入 Debug；新增 RED 后改为重读 winner，并把错误/报告限制为脱敏边界材料。
2. 初版把注册 template/policy/authority 漂移也当成普通 authority blocker，可能掩盖错误部署；现改为查询前启动失败、零写入。
3. 报告计数最初用普通加法；改成 checked-add 后又真实触发 Rust 类型推断编译错误，随后用显式 `usize` 修复并重新执行门禁。

## 6. 15 条 W11 行为测试

测试位于 `reconciler_tests.rs:347-1164`，覆盖：

1. 跨全部业务日期 keyset 翻页，终态排除且没有 dispatch seam；
2. 只接管缺失/过期 lease，同 owner/外部 owner 活 lease 分别复用/保护；
3. Accepted authority 经 W10 两查推进到 Completed；
4. 重启从 AwaitingFinalizer 重新查询，不复用内存资格；
5. Rejected 只记录一次且不形成 retry permission；
6. Uncertain 隔离后不再查询；
7. ManualConfirmedNotDelivered 没有 operator audit 时保持 pending；
8. authority 暂态/无效 blocker 只追加一个事件；
9. 扫描中有 intent 完成时仍保留跨日期边界；
10. 同 owner fence renewal 竞争后先重读，再允许 authority 使用；
11. 达到迭代上限仍有进展时 fail closed；
12. 重复启动不延长稳定 blocker 或 resolution chain；
13. 第二次 authority 查询失败被持久化并稳定；
14. 第二次查询 disposition 漂移被隔离而不完成；
15. 注册 template 不匹配时启动失败，authority 零调用、数据库零写入。

## 7. Fresh 门禁

| 门禁 | 结果 |
| --- | --- |
| W11 定向测试 | PASS：15 passed / 0 failed |
| `cargo test --lib push_foundation:: -- --test-threads=1` | PASS：68 passed / 0 failed（W07 9 + W08 14 + W09 12 + W10 18 + W11 15） |
| `cargo test --lib monitor::push_job::tests -- --test-threads=1` | PASS：52 passed / 0 failed |
| `cargo test --doc` | PASS：16 passed / 0 failed / 4 ignored |
| `cargo check --lib` | PASS；84 个目标外既有 warning；无 W11 warning |
| strict Clippy | 基线阻断：lib 163、lib-test 127 个目标外既有 lint；首个在 `src/data_gateway/futures_delivery.rs:15`；无 W11 reconciler 错误 |
| nonfatal Clippy | PASS，exit 0；163 warnings；push_foundation 仅有 W08/W09 既有 warning，无 W11 warning |
| production panic 搜索 | PASS：`reconciler.rs` 中 `unwrap/expect/panic` 为 0 |
| architecture docs 五组验证器 | PASS：418 runs / 5023 assertions / 0 failures / 0 errors / 0 skips |
| production-wiring relative diff | PASS：monitor、notification、durable delivery、config、Cargo manifests、冻结 SQL 相对 W10 均无变化 |

strict Clippy 的全仓失败是已记录的旧基线，本次只把 W11 目标范围验证为零新增错误；没有把全仓 strict lint 描述成通过。

## 8. monitor 观察边界

按用户最新要求，独立 monitor 观察任务已在 W11 开发期间停止。本次收口没有读取、改写、重启或替换生产进程，也没有把运行状态作为 W11 完成证据。W11 仍保持零生产接线；“代码门禁通过”不等于“生产消息已经正常推送”。

## 9. 尚未完成

- W12：durable authority 的具体只读 adapter；
- W13--W17：scheduler、readiness、activation、shadow 和 cutover；
- W18：operator 认证、inspect/resolve 与 capability 签发；
- W19--W21：指标、告警、发布和完整回滚门禁；
- 52 个 Migration Unit 的逐项 shadow、六门禁、单 owner 晋级、观察和 rollback；
- 生产 business schema migration、真实 Unit activation，以及盘前/集合竞价/盘中/盘后实盘验收。

因此 W11 完成的是“启动后可以在不重发的前提下，安全收敛已有业务意图”的 Foundation 能力。下一开发切片是 W12 concrete transport authority adapter。
