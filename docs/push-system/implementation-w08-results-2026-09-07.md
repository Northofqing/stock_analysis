# 推送 Foundation W08 实现与验证结果

**结论：** W08 已实现并验证未接生产的 Business Intent Outbox 与 append-only transition store。Ready intent 会把 W05 `PreparedPush/v1` canonical snapshot 和首次渲染原始字节持久化；NoData/Disabled 不伪造 payload。初始提交与后续转换均使用同一业务库 `BEGIN IMMEDIATE`，状态 CAS 和事件 append 同事务提交；稳定 event ID、连续 version、previous SHA-256 链、提交确认丢失恢复及读取侧 fail-closed 均有行为测试。

**生产边界：** 本切片没有接入 monitor、scheduler、provider、sink、durable authority、finalizer 或生产数据库，也没有修改生产 DDL、配置或 Cargo 清单。W09--W21 与 52 个 Migration Unit 仍未完成，当前线上推送继续由既有 release monitor 执行。

**验证日期：** 2026-09-07（Asia/Shanghai）

**分支/worktree：** `codex/push-reliability-20260905` / `.worktrees/push-reliability-20260905`

## 1. 交付范围

| 能力 | 结果 | 代码证据 |
| --- | --- | --- |
| attested business store | 只接受显式绝对、已存在、非 symlink 普通文件；同一连接通过 W07 schema attestation 后才恢复写能力并读回 safeguards | `intent_store.rs:764-811` |
| exact Ready outbox | 核对 PreparedPush 身份，保存同源 canonical snapshot、首次 rendered raw bytes，并现场绑定两份 SHA-256 | `intent_store.rs:359-400`、`projection.rs:801-808,1137-1175` |
| 非发送决定 | NoData/Disabled 的 prepared/rendered/payload/rendered-SHA 整组为 NULL | `intent_store.rs:402-446` |
| 初始幂等 | 相同不可变材料返回当前 `ExistingIdentical`；状态已合法推进不误报 immutable drift；不同材料不覆盖 | `intent_store.rs:575-596,833-943` |
| 非终态转换 | public command 只允许不需要 terminal authority 的边；W08 不能构造 Completed/NotDelivered 正向终态 | `intent_store.rs:172-284` |
| 原子 CAS + append | 事务内读当前行/完整链、精确 state/version/lease CAS、affected rows=1、append event、commit 后读回 | `intent_store.rs:970-1129` |
| 稳定事件与 hash 链 | event ID 只绑定 intent/expected/result version；canonical SHA 绑定除自身外全部事件字段；第一事件无前驱，其后逐条相连 | `intent_store.rs:1256-1341,1480-1567` |
| 读取侧验真 | 重算 identity/payload/event/hash，检查 edge/reason、terminal group、版本/状态/时间/lease 不变量及 NotDelivered 历史门槛 | `intent_store.rs:1420-1567,1685-1765` |
| crash recovery | initial/transition commit 前故障整体回滚；commit ack lost 由既有 exact fact 恢复，不产生第二写 | `tests.rs:798-878` |

`PreparedPush::canonical_snapshot_bytes()` 与 W05 既有 `prepared_push_value()` 共用唯一字段函数 `prepared_push_fields()`，没有复制第二套字段表。snapshot 内只放外部/渲染原始字节的 length/SHA；首次 rendered raw bytes 另存 `rendered_bytes`，因此既能重放，又不会把原文重复嵌入 canonical JSON。

## 2. 持久化语义

### 2.1 初始 intent

| 决定 | 初态 | reason | payload 组 |
| --- | --- | --- | --- |
| Ready | PendingDispatch | `intent.created` | snapshot bytes、rendered bytes、两个 SHA 全部非空 |
| NoData | NoData | `intent.no_data` | 全部 NULL |
| Disabled | Disabled | `policy.disabled` | 全部 NULL |

所有初始行固定 `version=0`、`previous_state=NULL`、无 lease、`lease_generation=0`、`updated_at=created_at`。`intent_id` 与 durable decision ID 均重新派生；读取时也再次派生并比对，不能只信数据库字符串。

`record_initial()` 的重复调用只比较 DDL 定义的 immutable columns。若 intent 已被 dispatcher 合法推进，仍返回包含当前 state/version 的 `ExistingIdentical`，不把可变字段误判成初始材料冲突；返回前会复验整条 transition 链。若 immutable bytes、identity、evidence、template/source-contract binding 或 created time 任一不同，则返回 typed `ImmutableConflict`，不使用 replace/upsert 覆盖事实。

### 2.2 transition

每个 transition 在一个 business connection 的 immediate transaction 内依次执行：

1. 读取并验真当前 intent 与完整既有链；
2. 检查 from state、expected version、occurred time 与 typed lease action；
3. 计算稳定 event ID、previous SHA 与 canonical SHA；
4. `UPDATE ... WHERE` 同时绑定 state、version、lease generation/owner/until；
5. affected row 必须精确为 1，否则 rollback 后返回当前 winner；
6. 在同一事务 append 一条 event；任一 SQL/trigger/check 失败则 CAS 一并回滚；
7. commit 后按 event ID 读回并重算，再校验 current head/完整链。

`PendingDispatch -> AwaitingAuthority` 的 dispatch claim 必须在提交结果中具有 `lease_until > occurred_at` 的 active lease。取得/接管使 generation 精确加一；未过期 foreign owner 不可抢占；释放只能由当前 owner。冲突快照使用 boxed variant，避免 `TransitionOutcome` 因 Ready payload 造成大枚举栈尺寸。

## 3. event identity 与 canonical 证据

event ID domain 为 `IntentTransitionV1`，唯一材料是：

```text
intent_id + expected_version + result_version
```

event canonical preimage 使用相同 domain，但包含数据库中除 `canonical_sha256` 自身外的所有 transition 字段，包括 actor、reason、occurred_at、nullable terminal 字段与 previous SHA。首条测试独立拼出 exact canonical bytes 后用 `sha2` 重算，而不是调用生产 helper 自证（`tests.rs:659-758`）。三段连续转换证明 result version 为 1/2/3，第二、三条 previous SHA 精确指向前一条 canonical SHA。

冻结的 v1 transition schema 没有 lease mutation 字段，因此历史事件的 canonical hash 不声称能重建当时每个 lease 参数。事件仍绑定业务状态转换；当事件还是当前 head 时，重试还会对 Acquire/Release 的当前 owner/until 结果做精确核对。事件已成为历史后，恢复依据是持久事件本身和完整链，而不是虚构不存在的历史 lease 证据。若未来要求逐次 lease 参数也进入审计 preimage，应通过新 schema 版本显式演进，不能静默改变 W07 的 frozen DDL。

## 4. 崩溃与竞争矩阵

| 场景 | 已验证结果 |
| --- | --- |
| initial COMMIT 前故障 | intent 数仍为 0；重试只创建一次 version 0 |
| initial COMMIT 后确认丢失 | 库内只有一条；重试返回 `ExistingIdentical` |
| CAS 后、event append 前故障 | state/version/lease 和 event count 全部回滚 |
| append 后、COMMIT 前故障 | intent 与 event 同事务回滚 |
| transition COMMIT 后确认丢失 | 同 command 返回 `AlreadyCommitted`；event count 不增加 |
| stale/competing version 0 | winner 保留，loser 返回 `Conflict` 且零写 |
| 同 event、不同 current-head lease material | 返回 `Conflict`，不把不同参数伪装为 ack-loss recovery |
| 已有 event 快路径遇到断链 | 先复验整链，返回 `IntegrityFailed` |
| 哈希重算正确但 edge/reason 非法 | 读取侧仍返回 `IntegrityFailed`，不只信 hash |

测试故障点全部位于 `cfg(test)`，生产 API 不可调用。

## 5. TDD 与提交证据

| 提交 | 内容 | 证据 |
| --- | --- | --- |
| `309581a` | W08 设计与逐测试计划 | 冻结 exact bytes、事务、权限及零接线边界 |
| `39b7d46` | 第一组 RED | W08 outbox/store symbols 尚不存在，按预期编译失败 |
| `b40342d` | initial GREEN | Ready/NoData/Disabled、attested store、initial retry 通过 |
| `4d405a3` | 第二组 RED | transition types/behavior 尚不存在，按预期编译失败 |
| `1ae5e77` | transition GREEN | CAS + append、hash chain、fault recovery 通过 |
| `c2ec76f` | 双轴 review 修复 | 关闭创建重试、lease、existing-event 快路与持久事实验真漏洞 |
| `114cb52` | target lint 修复 | boxed conflict snapshot；清除 W08 attestation 重复借用 lint |

两轮 RED 都由预期 W08 能力缺失导致；没有用外部网络、生产环境或全仓旧失败冒充红灯。

## 6. 双轴 review 结果

### Standards

- 仓储不暴露 raw connection、任意 SQL、默认生产路径或 alternate schema。
- W07 attestation 只提取 crate-private 同连接 helper，public migration API 未放宽。
- 所有业务写都在 `BEGIN IMMEDIATE` 内；错误类型不包含 payload、render、数据库路径或 SQL 原文。
- 生产实现没有 `unwrap`、`expect`、`panic!` 或 `unreachable!`；fault injection 只在测试构建出现。
- review 将大枚举冲突快照改为 `Box<IntentSnapshot>`，W08 target strict Clippy 为零。

### Spec

- 创建重试改为只比较 immutable material，并在返回 existing 前验证完整链。
- existing-event 快路径不再只验单个 event，会先重算完整 version/previous-SHA/state/time 链。
- dispatch 进入 AwaitingAuthority 必须有未来 lease；current-head retry 的 lease owner/until 必须与 Acquire command 一致。
- persisted event 除 hash 外还独立验证 edge/reason、terminal nullable group、actor、intent ID 与 NotDelivered 历史资格。
- version 0 初态/reason/time/lease 精确；非零 version 必须有 previous state；lease generation 不得大于 version。

review 后没有遗留 W08 critical/high/medium finding。W09/W10 所需 authority/finalizer terminal 构造能力仍被刻意排除，不能用 W08 public API 伪造。

## 7. Fresh 验证

| 门禁 | 结果 |
| --- | --- |
| `cargo test --lib w08_ -- --nocapture` | PASS：15 passed / 0 failed（含 1 条 PreparedPush snapshot + 14 条 foundation） |
| `cargo test --lib push_foundation::tests:: -- --nocapture` | PASS：23 passed / 0 failed（W07 9 + W08 14） |
| `cargo test --lib monitor::push_job -- --nocapture` | PASS：52 passed / 0 failed；两条 panic 文本是既有 catch-unwind 反例，测试均 ok |
| `cargo test --doc push_job` | PASS：3 passed / 0 failed / 17 filtered |
| `cargo check --lib` | PASS；84 个目标外既有 dead-code warning；无 W08 warning |
| strict Clippy | 基线阻断：精确 79 个目标外旧 lint；首个仍为 `src/data_gateway/futures_delivery.rs:15`；`push_foundation`/W08 零错误 |
| nonfatal Clippy | PASS，exit 0；79 warnings，均不在 W08 target |
| 定向 rustfmt | PASS：15 个 W01--W08 Rust 文件，module root 使用 `skip_children=true` |
| architecture docs 五组验证器 | PASS：418 runs / 5023 assertions / 0 failures / 0 errors / 0 skips |
| schema identity | W07 DDL SHA `4bac8e58…a953`、signature `dd5f49a1…60ecdd`、25 objects 由 23 项 foundation 回归继续证明 |
| `git diff --check 4b5b03f..HEAD` | PASS |
| production-wiring relative diff | PASS：`src/bin/monitor`、notification、durable delivery、config、Cargo manifests、frozen SQL 均无变化 |

strict Clippy 的 79 项是当前工具链下已存在的目标外基线，本切片没有顺手修改这些不相关文件，也不把 strict 全仓描述为通过。

## 8. 生产 monitor 只读观测

2026-09-07 06:17 CST 只读观察：既有 `./target/release/monitor` 仍为 PID 20162，已运行 `07:35:07`；`10.211.55.2:60076 -> 10.211.55.3:50051` 与 `127.0.0.1:54144 -> 127.0.0.1:18082` 均为 ESTABLISHED。进程仍打开既有 `stock_analysis.db`、`durable_delivery.sqlite3` 及锁文件；本次开发没有重启、重建或热替换它。

当前尚在盘前，`data/push_log/2026-09-07` 尚不存在。最近实际日志为 9 月 6 日 22:27/22:41 两条 Data Unsafe 状态消息及 15:20 attribution 静默哨兵；只读 durable 查询中最近 authoritative Accepted/Delivered 停在 9 月 4 日。9 月 5--6 日为周末，这些事实不能推出 9 月 7 日应已发生业务推送，也不能用 TCP established 代替 typed Accepted/AlreadyDelivered 回执。

## 9. 尚未完成

- W09 durable authority requery 与不可伪造 `VerifiedTerminalRef`；
- W10 business finalizer、Completed/NotDelivered 受限终态；
- W11--W21 recovery、scheduler/readiness、activation、shadow/cutover、operator、metrics/gates；
- 52 个 Migration Unit 的逐 Unit 接线、影子证据、单 owner 晋级和回滚演练；
- 生产 business schema migration、真实 Unit activation 与盘前/竞价/盘中/盘后验收。

因此 W08 是可靠性地基完成，不等于整体推送改造完成，也不会改变当前线上推送行为。下一开发切片是 W09。
