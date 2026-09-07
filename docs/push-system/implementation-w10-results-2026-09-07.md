# 推送 Foundation W10 实现与验证结果

**结论：** W10 已完成通用业务 Finalizer。接受路径必须先验证 authority 并把业务意图从 `AwaitingAuthority` 推进到 `AwaitingFinalizer`，再二次查询同一 authority 后以同库 CAS + append 原子推进到 `Completed`；人工确认未投递则必须同时消费二次验证的 `ManualConfirmedNotDelivered` 与独立 operator audit capability，才能进入 `NotDelivered`。重复执行只返回已存在的终态事实，真实竞争被隔离到 `ResolutionRequired`，提交确认丢失按精确事件恢复。

**生产边界：** 本切片没有接入 monitor、notification、durable adapter、配置、Cargo 清单或生产数据库，没有修改冻结 DDL，也没有迁移任何 Unit 的领域 cursor。线上仍由根仓库既有 release monitor 执行原推送逻辑；W10 代码目前不会改变实际推送。

**验证日期：** 2026-09-07（Asia/Shanghai）

**分支/worktree：** `codex/push-reliability-20260905` / `.worktrees/push-reliability-20260905`

## 1. WBS 验收与代码证据

W10 的验收句是：“CAS 冲突进入 ResolutionRequired；Accepted 不可撤销，重复 finalize 只推进一次。”实现对应如下：

| 验收点 | 实现结果 | 代码证据 |
| --- | --- | --- |
| Accepted 两阶段完成 | prepare 首查并写 `AwaitingFinalizer`；commit 二查并写 `Completed` | `business_finalizer.rs:352-446,629-800` |
| exact authority 二查 | prepare 使用 `verify_terminal`，commit 使用 `reverify_for_finalization`，稳定 binding 漂移即失败 | `business_finalizer.rs:405-418,740-760` |
| exact lease fence | owner、generation、until 必须全部相同且未过期；终态成功时释放 lease | `business_finalizer.rs:58-72,736-738`、`intent_store.rs:1248-1268,1614-1656` |
| CAS + append 原子性 | 更新 intent、插入 transition、commit 位于同一事务；CAS 零行先 rollback 再返回当前快照 | `intent_store.rs:1397-1565` |
| 冲突隔离 | 非幂等竞争重读 winner，再追加 `finalizer.cas_conflict` 到 `ResolutionRequired`；二次竞争返回当前快照 | `business_finalizer.rs:602-626,802-817`、`intent_store.rs:1360-1395` |
| Accepted 不可撤销 | 已完成但 terminal material 不同只隔离到 `ResolutionRequired`，既有 Completed 事件不删除；NotDelivered 历史门禁拒绝任何 AwaitingFinalizer/Completed 历史 | `business_finalizer.rs:694-717,877-900` |
| 重复 finalize 一次推进 | 完整合法终态直接返回既有 receipt；相同 event ID 只有完全匹配才是 `AlreadyCommitted` | `business_finalizer.rs:359-371,694-710`、`intent_store.rs:1414-1424` |
| commit ack loss 恢复 | 提交结果未知时读取 exact event、current head 和完整链；匹配才返回已提交事实 | `intent_store.rs:1537-1553` |
| authority 失效留证 | AwaitingFinalizer 的二查失败追加同态 `finalizer.terminal_ref_invalid`，错误同时返回 receipt | `business_finalizer.rs:740-759,819-850`、`intent_store.rs:1218-1245` |
| Resolution 解封受控 | opaque clearance 精确绑定 intent、当前 version 和冲突事件 SHA；普通生产代码没有构造器 | `business_finalizer.rs:28-55,374-403` |
| 人工 NotDelivered 受控 | opaque audit 精确绑定 intent、decision、version、audit ref/SHA；两次 authority 查询后才写终态 | `business_finalizer.rs:140-187,448-600` |
| completion policy 保持正交 | Accepted 与 NotDelivered 均重新交给 W03 `evaluate_completion`，不在 finalizer 内自造 schedule/cursor/retry 语义 | `business_finalizer.rs:903-919` |

## 2. Accepted 完成路径

`prepare_accepted_finalization` 的顺序是：

1. 从 business DB 读取 intent 与完整 transition chain；
2. 对完整合法 `Completed` 直接返回既有 terminal receipt，不查询 authority；
3. 核对请求 version 与 exact lease fence；
4. `ResolutionRequired` 必须消费与当前冲突事件绑定的 `VerifiedResolutionClearance`；
5. 调用 W09 `verify_terminal` 首次查询 exact decision，只接受 `Accepted` 或 `ManualConfirmedAccepted`；
6. `AwaitingAuthority/ResolutionRequired -> AwaitingFinalizer` 使用同库 CAS + append，lease 保留；
7. 返回不可 Clone 的 pending 值，保存首查强引用、qualified version 与 fence。

`commit_accepted_finalization` 随后重读状态和链：

1. 已有匹配 Completed event 时返回 `AlreadyCommitted`；
2. 非 AwaitingFinalizer、版本漂移或不同 terminal material 不能当幂等成功；
3. 再次校验 fence，调用 W09 `reverify_for_finalization` 做第二次 exact query；
4. 二查结果与首查稳定字段完全一致后，再计算 W03 completion directive；
5. 事务内执行 `AwaitingFinalizer -> Completed`、append terminal event、释放 lease；
6. CAS 竞争转入独立冲突隔离，事务故障整体回滚，ack loss 从已提交事件恢复。

数据库里的 Completed 事件只保存 disposition、terminal ref 和 binding SHA，不复制 receipt/evidence 原文。人工接受保持 `ManualConfirmedAccepted`，没有伪装为 transport accepted。

## 3. ManualConfirmedNotDelivered 路径

NotDelivered 没有复用 Accepted finalizer，也不经过 `AwaitingFinalizer`。它必须同时满足：

- 当前为 `AwaitingAuthority`，或最近一次是由 `AwaitingAuthority + transport.uncertain` 进入的 `ResolutionRequired`；
- 完整历史从未进入 `AwaitingFinalizer` 或 `Completed`；
- opaque operator audit 与当前 intent、decision、version 精确匹配，audit ref 非空、无 NUL、无首尾空格且长度受限；
- 首次和最终 authority 查询都是同一个 `ManualConfirmedNotDelivered` 强终态；
- exact lease fence 与 CAS 均成功。

成功事件保存 terminal disposition、原 decision、operator audit ref/SHA、terminal ref/binding SHA，并释放 lease。返回的 W03 directive 保持 `cursor=Never`；W10 不推进任何领域通知 cursor、不关闭 schedule、不授权重发，也不把人工未投递计作 accepted。

生产 `VerifiedOperatorAuditRef` 签发仍属于 W18。W10 只有 `cfg(test)` fixture constructor，因此测试能证明消费规则，但生产调用方不能自行伪造审计 capability。

## 4. 冲突、幂等与不可撤销

幂等成功要求完整材料一致，而不是只看 state 或 event ID：intent、from/to state、version、actor、reason、occurred_at、lease、terminal ref/binding、decision 与 operator audit 字段都由既有 command 匹配和 canonical chain 验证共同约束。

主 CAS affected rows 为零时，原事务先 rollback，再读取 current snapshot：

- exact event 与 current head 完整匹配：返回 `AlreadyCommitted`；
- 已是合法且匹配的终态：返回原 terminal fact；
- 其余竞争：独立 CAS 到 `ResolutionRequired` 并 append 一条冲突事件；
- 隔离 CAS 再竞争或已经处于 ResolutionRequired：返回带当前 snapshot 的 `ConflictUnresolved`，不循环写事件。

当两个调用竞争同一 Accepted 时，winner 只生成一个 Completed event，loser 恢复该事实且不做第三次 authority 查询。若相同 version 但 terminal material 不同，则从 Completed 隔离到 ResolutionRequired；原 Completed terminal event 仍保留在 append-only chain 中，所以外部 accepted 事实没有被撤销，也不会转成 NotDelivered。

## 5. 故障与证据语义

| 故障点 | 结果 | 持久化事实 |
| --- | --- | --- |
| CAS 后注入故障 | typed storage error | 整个事务 rollback，state/version/event 数量不变 |
| append 后注入故障 | typed storage error | 整个事务 rollback，不能出现孤立 state 或 event |
| commit ack lost | 首次调用报告注入故障，重试恢复 exact receipt | 数据库只有一个终态事件，不再查询 authority |
| final authority requery 失败 | `TerminalInvalid { source, receipt }` | 保持 AwaitingFinalizer，append 一条同态 invalid evidence |
| 恢复 prepare authority 失败 | 同上 | 同样留下非终态 invalid evidence，不静默丢失原因 |
| stale version/state | 隔离成功则返回 `ResolutionRequired` | 恰好一条 conflict event；隔离再竞争则返回 current snapshot |
| 已在 ResolutionRequired | `ConflictUnresolved { current }` | 零重复隔离写入 |

生产实现文件中没有 `unwrap(`、`expect(` 或 `panic!`。错误只返回稳定分类、检查名、typed snapshot/receipt，不回显 terminal evidence bytes、渲染正文、SQL 或数据库路径。

## 6. TDD、评审与提交

| 提交 | 内容 | 结果 |
| --- | --- | --- |
| `5e61701` | 冻结 W10 设计与逐测试实施计划 | 明确两阶段 Accepted、NotDelivered、冲突和零生产接线 |
| `7f65d0a` | Accepted 第一组 RED | finalizer API 尚不存在，按预期失败 |
| `6dc19f8` | Accepted GREEN | 两次查询、资格转换、Completed terminal event |
| `ba3d804` | 冲突和故障恢复 RED | 暴露 CAS 隔离与 fault seam 缺口 |
| `1f07bd6` | 冲突隔离 GREEN | stale CAS、重复完成、rollback/ack-loss 路径 |
| `a4f24f3` | capability 与 NotDelivered RED | 固定 clearance/operator audit 受控边界 |
| `fec4ec0` | NotDelivered GREEN | 二查、独立 audit、Never cursor 与历史门禁 |
| `fdc56df` | terminal history/replay 强化 | 不同 terminal、重复 not-delivered 与 replay 反例 |
| `5821aa2` | review 缺口 RED | final requery 失效证据、current snapshot、重复隔离 |
| `6383994` | 双轴 review 修复 | 所有评审发现闭环，新增负路径通过 |
| `a483b7a` | target Clippy 收口 | 大枚举 boxed，W10 strict lint 归零 |

评审后没有遗留 W10 critical/high/medium finding。W11 reconciler、W12 concrete adapter、W18 operator capability 及 Unit cursor 仍保持在各自切片，没有用假 production constructor 或提前接线绕过依赖。

## 7. 18 条 W10 行为测试

测试位于 `business_finalizer_tests.rs:138-1135`，覆盖：

1. Accepted 两查、两转换和 fresh terminal binding；
2. ManualConfirmedAccepted 的独立处置语义；
3. Rejected、Uncertain、NotDelivered 不能走 Accepted 分支；
4. stale CAS 重读并进入 ResolutionRequired；
5. 相同并发 finalize 只产生一个 Completed event；
6. 同 version 不同 terminal 不是幂等成功；
7. CAS 后与 append 后故障完整 rollback；
8. commit ack lost 精确恢复且不做额外查询；
9. ResolutionRequired 缺 clearance 被拒绝；
10. exact clearance 只解封其绑定的当前冲突；
11. NotDelivered 两查并保存独立 audit；
12. ResolutionRequired 只有 exact uncertain 来源可 NotDelivered；
13. 重复 NotDelivered 只返回一个终态事件；
14. audit 缺失、错 intent/version/decision 或 replay 在查询前失败；
15. 已有 Accepted/AwaitingFinalizer 历史与错误 disposition 被拒绝；
16. final requery 失败追加 nonterminal invalid evidence；
17. AwaitingFinalizer 恢复 prepare 失败同样留证；
18. 已有 ResolutionRequired 返回 current，不重复写 isolation event。

## 8. Fresh 门禁

| 门禁 | 结果 |
| --- | --- |
| `cargo test --lib push_foundation:: -- --test-threads=1` | PASS：53 passed / 0 failed（W07 9 + W08 14 + W09 12 + W10 18） |
| `cargo test --lib monitor::push_job::tests -- --test-threads=1` | PASS：52 passed / 0 failed |
| `cargo test --doc` | PASS：16 passed / 0 failed / 4 ignored |
| `cargo check --lib` | PASS；84 个目标外既有 warning；无 W10 warning |
| strict Clippy | 基线阻断：163 个目标外既有 lint，首个在 `src/data_gateway/futures_delivery.rs:15`；W10/push_foundation 目标零错误 |
| nonfatal Clippy | PASS，exit 0；163 warnings；W10 目标零 warning |
| 定向 rustfmt | PASS：Finalizer、store、tests 与模块根无格式差异 |
| production panic 搜索 | PASS：W10 finalizer/store 中 `unwrap/expect/panic` 为 0 |
| architecture docs 五组验证器 | PASS：418 runs / 5023 assertions / 0 failures / 0 errors / 0 skips |
| `git diff --check f9ffe63..HEAD` | PASS |
| production-wiring relative diff | PASS：monitor、notification、durable delivery、config、Cargo manifests、冻结 SQL 相对 W09 均无变化 |

strict Clippy 的 163 项与 W09 记录的全仓既有基线数量完全相同；本切片修复了全部 W10 目标内 lint，没有顺手修改无关旧代码，也没有把 strict 全仓错误描述成通过。

## 9. 生产 monitor 只读观察

2026-09-07 08:36 CST 只读观察：根仓库既有 `./target/release/monitor` 仍为 PID 20162，已连续运行约 9 小时 56 分钟；cwd 与加载的 release 二进制都位于根仓库，不是开发 worktree。进程继续持有 `durable_delivery.sqlite3`、`stock_analysis.db`、`push_analytics.db`，并保持以下两条连接：

- `10.211.55.2:60076 -> 10.211.55.3:50051`：ESTABLISHED；
- `127.0.0.1:54144 -> 127.0.0.1:18082`：ESTABLISHED。

`data/push_log/2026-09-07` 尚不存在；08:36 仍未到周一常规盘前推送窗口，不能仅据无日志判定故障。进程/连接存活也不等于某条业务消息已 accepted，最终状态仍必须以 durable authority 为准。本次 W10 开发和观察没有重启、重建、热替换或修改生产进程。

## 10. 尚未完成

- W11：reconciler、过期 lease takeover 与恢复调度；
- W12：各 durable authority 的具体只读 adapter；
- W13--W17：scheduler/readiness/activation/shadow/cutover；
- W18：operator 认证、inspect/resolve 与 capability 签发；
- W19--W21：指标、告警、发布与完整回滚门禁；
- 52 个 Migration Unit 的逐项 shadow、六门禁、单 owner 晋级、观察和 rollback；
- 生产 business schema migration、真实 Unit activation，以及盘前/集合竞价/盘中/盘后实盘验收。

因此 W10 完成的是“强终态经同库 CAS 只完成一次、冲突可隔离并可恢复”的 Foundation 能力，不代表生产推送改造已经完成。下一开发切片是 W11。
