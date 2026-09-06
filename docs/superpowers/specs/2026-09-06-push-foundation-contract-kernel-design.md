# 推送 Foundation W01--W03 合同内核设计

**状态：** 用户已批准推荐方案；进入实施计划，尚未实现、接线、部署或晋级。

**决策日期：** 2026-09-06

**范围：** W01 身份/业务日/occurrence/source-contract，W02 应用 `DeliveryResult` 与当前 durable 十四态适配，W03 `CompletionPolicy`/`ReasonCode`/`RetryPolicy`。本设计是完整 W01--W21 与 52-Unit 迁移的第一个运行时切片，不把首批切片冒充整体迁移完成。

## 1. 结果

新增一个无 I/O、无全局状态的深模块 `stock_analysis::monitor::push_job`，统一提供后续 producer、scheduler、coordinator adapter 与 finalizer 必须使用的应用合同：

1. 同一业务 occurrence 在重启、构建或 activation generation 改变后保持同一身份。
2. 同一日期但不同 producer、completion owner 或 source-contract 的工作不会被错误合并。
3. 当前 `durable_delivery::DecisionState` 的十四个状态被穷举分类；新增状态会使适配代码无法通过编译，而不是落入 `_` 默认分支。
4. `DeliveryResult` 明确区分 strong、compat 和 none 三类权限；compat 结果在类型层没有转换为 `VerifiedTerminalRef` 的接口。
5. `NoData`、`Disabled`、`Uncertain` 分别产生不同的 schedule、cursor、retry 和 manual disposition，不能被调用方压成 `bool` 或 `Ok(())`。
6. 重试决定始终携带原始 typed `ReasonCode`；发送后的 `Uncertain` 永远不进入自动重发。

这个切片只建立合同和纯函数。它不接任何现有 producer，不打开数据库，不发送消息，不读取 provider，不推进游标，也不改变正在运行的 release monitor。

## 2. 依据和当前证据

| 结论 | 权威依据 | 当前源码/运行证据 |
| --- | --- | --- |
| application seam 位于 durable coordinator 之前 | `docs/Project_Architecture_Blueprint.md` §24.11--§24.13 | 蓝图建议 `src/monitor/push_job.rs`；当前 coordinator 只拥有物理投递 authority |
| `DeliveryResult` 不是新的 durable 状态 | RFC “durable 状态与应用投影” | `src/durable_delivery/model.rs::DecisionState` 当前恰有十四态 |
| strong result 只能来自精确 authority 绑定 | RFC `VerifiedTerminalRef`、终态完成合同 | 当前 `AuthoritativeSinkResult` 已区分 Accepted/Rejected/Uncertain，但 application completion 仍未统一 |
| compatibility observation 不能证明 durable accepted | RFC `CompatibilityEvidenceRef` 与 W02 acceptance | 2026-09-06 22:41 的运行观察有一条 `data_mode pushed=1` 和 push log，但同窗 durable attempt/result 均为 0 |
| occurrence 不受 generation/restart 影响 | RFC `ScheduleOccurrence` identity；W01 acceptance | 现有代码尚无统一 application occurrence 类型 |
| NoData/Disabled/Uncertain 必须分流 | RFC “业务完成分支”；W03 acceptance | 当前多路径仍存在 bool/日志/弱 analytics 结果，不能作为统一 completion authority |

本设计引用的权威文件为：

- `docs/push-system/push-system-implementation-rfc.md`
- `docs/push-system/push-system-wbs.v1.json`
- `docs/Project_Architecture_Blueprint.md`
- `src/durable_delivery/mod.rs`
- `src/durable_delivery/model.rs`

## 3. 方案比较

### 3.1 采用：独立 `monitor::push_job` 深模块

外部只有一个应用 seam；内部可以分 identity、delivery、policy 三部分实现。调用方只学习稳定身份、应用结果和完成判定，不接触 canonical 编码细节、状态映射表或权限构造细节。

优点：

- 符合蓝图落位和依赖方向；
- 不污染 durable 十四态；
- W04--W21 和 52 个 Unit 可复用同一合同；
- 测试直接穿过生产调用方未来使用的接口；
- 当前生产路径零接线，部署风险最低。

代价：首批会增加一组严格值类型；但这些类型替代的是散落的 String/bool 隐含合同，不是新增业务复杂度。

### 3.2 不采用：把应用结果放入 `durable_delivery`

这会让 SQLite/fence/receipt authority 同时拥有 schedule、cursor 和业务完成策略。删除该模块时复杂度不会回到单一调用点，而会把两种不同职责锁在一起，也与 `src/durable_delivery/mod.rs` 当前声明的职责相冲突。

### 3.3 不采用：分别修改 notification、monitor 和 durable

这能减少单次新增文件，但会继续维护三套浅接口：compat 渠道结果、monitor 本地结果、durable authority 结果。后续迁移仍需每个 Unit 自行解释 `Ok`、bool、audit 和 receipt，无法满足“一套应用完成真相”。

## 4. 模块和依赖方向

```text
src/monitor/push_job.rs                 唯一公共接口
└── src/monitor/push_job/
    ├── identity.rs                    W01 私有实现
    ├── delivery.rs                    W02 私有实现
    ├── policy.rs                      W03 私有实现
    └── tests.rs                       通过公共 seam 的行为测试
```

只在 `src/monitor/mod.rs` 增加：

```rust
pub mod push_job;
```

允许的依赖：

```text
push_job
  -> chrono（受校验 Date）
  -> serde / serde_json（确定性编码材料）
  -> sha2（稳定摘要）
  -> durable_delivery::DecisionState（只读状态适配）
```

禁止的依赖或行为：

- `rusqlite`、Diesel、production DB 或 schema migration；
- `reqwest`、gRPC、provider、LLM、sink、webhook；
- `std::env`、`.env`、全局 singleton、系统当前时间；
- `NotificationService`、monitor scheduler 或 binary-local dispatcher；
- cursor 更新、订单、文件 append 或 detached task；
- 新增第十五个 durable state，或复制 `AuthoritativeSinkResult`。

## 5. W01：身份与基础值合同

### 5.1 受校验值

外部接口提供语义不同的值类型，不使用可互换的 String alias：

| 类型 | 最低约束 | 用途 |
| --- | --- | --- |
| `Namespace` | `Production` 或绑定 run identity 的 `Test` | 阻止测试/生产混用 |
| `RunId` | 非空、长度有界、无 NUL | `Test` namespace 的稳定隔离身份 |
| `UnitId` | 非空、长度有界、无 NUL | catalog migration owner |
| `ProducerId` | 非空、长度有界、无 NUL | occurrence producer 家族 |
| `ScheduleOrTriggerId` | 非空、长度有界、无 NUL | 注册 schedule 或 event/manual trigger 身份 |
| `CompletionOwnerId` | 非空、长度有界、无 NUL | 唯一业务完成所有者 |
| `SourceContractId` | 非空、长度有界、无 NUL | 来源合同身份，不从 payload 猜测 |
| `SourceContractVersion` | 非空、长度有界、无 NUL | 后续 RunContext 绑定 |
| `CalendarId` | 非空、长度有界、无 NUL | 交易日 authority 身份 |
| `BusinessDate` | 可解析且规范 `YYYY-MM-DD` | 业务日，不是 wall-clock 日期 |
| `OccurrenceFamily` | 非空、长度有界、无 NUL | 注册 occurrence 家族 |
| `OccurrenceKey` | 非空、长度有界、无 NUL | 家族内业务键 |
| `SubjectId` | `Global` 或受校验业务对象 | 业务标的，不从展示文本反推 |
| `AudienceId` | 非空、长度有界、无 NUL | 稳定路由受众 |
| `Sha256Digest` | 64 位小写十六进制 | 只表示已验证摘要 |
| `UtcMicros` | 非负 `i64` UTC 微秒 | 捕获时间或 retry eligibility，不读取 now |

字段私有且构造时完成验证。首批不实现通用 `Deserialize`；后续持久化读取必须经过显式 `TryFrom`/校验入口，防止 serde 直接绕过构造器。

### 5.2 `OccurrenceId`

`OccurrenceId` 只由下列有序材料派生：

```text
domain = OccurrenceId/v1
business_date
occurrence_family
occurrence_key
```

它不包含 run id、wall-clock tick、phase、activation generation、build、payload、render、evidence 或 receipt。因而同一业务 occurrence 在重启/晋级后稳定；跨业务日相同展示名不会碰撞。

`OccurrenceId` 表达业务家族内 occurrence，本身不绑定 source-contract。禁止错误合并的是下面两个外层身份：`ScheduleOccurrenceId` 和 `IntentId` 都必须包含 `source_contract_id`。测试不得错误要求“换 source-contract 会改变原始 OccurrenceId”。

### 5.3 `ScheduleOccurrenceId`

`ScheduleOccurrenceIdentityMaterial` 必须经一个构造器一次性接收：

```text
schema_version = ScheduleOccurrence/v1
namespace
unit_id
producer_id
schedule_or_trigger_id
calendar_id
business_date
occurrence_family
occurrence_key
completion_owner
source_contract_id
```

明确不让以下值进入构造器，因此调用方无法错误加入身份：

```text
wall_clock_tick, phase_epic, activation_generation, version,
expected_version, build_commit, payload_sha256, rendered_sha256,
evidence_sha256
```

同一日期更换 source-contract、producer 或 completion owner 必须得到不同 ID；同一材料只改变 restart/generation 必须得到相同 ID。

### 5.4 `IntentId`

`IntentIdentityMaterial` 严格对应 RFC `IdentityRule::PreparedPushIntent`：

```text
domain = PreparedPushIntent/v1
namespace
unit_id
completion_owner
source_contract_id
occurrence
subject
audience
```

payload/render/evidence SHA 不参与 ID。相同 ID 下材料漂移必须由 W07/W08 进入 `ResolutionRequired`，不能通过换 ID 逃逸。本切片只计算身份，不实现冲突持久化。

### 5.5 canonical-v1

canonical 编码为模块私有实现，不提供“对任意 JSON 求权威 hash”的通用公共函数：

1. 精确 preimage 为 `<ASCII domain tag><单字节 0x00><canonical UTF-8 JSON>`；域标签固定为对应表声明的 `<Type>/v1`，不得含 NUL；
2. JSON 对象键按字典序；
3. 字符串按 JSON 转义，捕获后不再 trim/大小写改写；
4. 整数使用最短十进制，不接受 float；
5. `Option` 显式写 `null`；
6. 数组保持捕获顺序；
7. 无无关空格、无末尾换行；
8. SHA-256 对精确字节计算。

测试固定 canonical bytes 和 SHA golden vector，不能在断言中调用被测实现重新生成“期望值”。

## 6. W02：应用 DeliveryResult 与 durable 适配

### 6.1 `DeliveryResult`

外部可观察分支严格为 RFC 九类。为防止调用方绕过分支构造约束，实际 Rust 表示使用不透明 `DeliveryResult` 和只读 view，而不是允许直接构造 variant 的 public enum：

```rust
pub struct DeliveryResult(DeliveryResultKind);

pub enum DeliveryResultView<'a> {
    TransportAccepted(&'a VerifiedTerminalRef),
    TransportRejected(&'a VerifiedTerminalRef),
    TransportUncertain(&'a VerifiedTerminalRef),
    AlreadyTerminal(&'a VerifiedTerminalRef),
    BestEffortAccepted(&'a CompatibilityEvidenceRef),
    PartiallyAccepted(&'a CompatibilityEvidenceRef),
    NoChannelConfigured(ReasonCode),
    AllChannelsFailed(&'a CompatibilityEvidenceRef),
    Blocked(ReasonCode),
}
```

`DeliveryResultKind` 保持模块私有。public 构造函数分别校验 strong terminal disposition、BestEffort 全渠道接受、Partial 至少一接受且至少一未接受、AllFailed 配置非空且零接受，并为 NoChannel 固定 `transport.no_channel_configured`。`view()` 只观察，不提供从 view 反向构造结果的接口。

业务 `Completed`、`NoData`、`Disabled` 不加入 `DeliveryResult`。它们分别属于 finalizer 已提交事实或非发送 `JobDecision`，后续由 W04/W05/W10 接入。

### 6.2 权限不可伪造

`VerifiedTerminalRef` 字段私有，W01--W03 不提供公开构造器、`Default`、通用 `Deserialize`、从 `CompatibilityEvidenceRef` 的转换或从 bool/`AuthoritativeSinkResult` 的直接转换。

本切片只冻结它的接口字段和读取方法。真实构造入口留给 W09 的私有 authority adapter；该入口必须重新查询并验证 decision、attempt、intent、Unit、occurrence、business date、subject、audience、template、rendered SHA、terminal disposition、evidence 和 durable schema binding。

`CompatibilityEvidenceRef` 可由后续 CLI/NotificationService adapter 构造，但必须验证：

- configured channels 有序且唯一；
- attempted channels 是 configured channels 子集；
- 每个 attempted channel 恰有一个 Accepted/Rejected/Unknown 弱结果；
- `not_authoritative` 是类型事实，不接受调用方传 bool；
- 本地 evidence SHA 不是远端 receipt SHA。

### 6.3 十四态穷举适配

`classify_durable_state(DecisionState) -> DurableStateProjection` 使用无 `_` 的穷举 match。输出不是伪造的终态，而是下一步所需 authority：

| 当前 durable 状态 | 投影 |
| --- | --- |
| `Reserved` | `BlockedBeforeAttempt`，只有 fence/lease 合法后可首次 attempt |
| `AttemptInFlight` | `BlockedAwaitingReconciliation`，重查前不得重发 |
| `AcceptedAuditPending` | `BlockedAwaitingAuthoritySeal` |
| `AcceptedTaskTransitionPending` | `BlockedAwaitingAuthoritySeal` |
| `Delivered` | `RequiresVerifiedAcceptedOrAlreadyTerminal` |
| `RejectedAuditPending` | `BlockedAwaitingReconciliation` |
| `RejectedTaskTransitionPending` | `BlockedAwaitingReconciliation` |
| `RejectedDurable` | `RequiresVerifiedRejectedOrAlreadyTerminal` |
| `UncertainAuditPending` | `BlockedAwaitingAuthoritySeal` |
| `UncertainTaskTransitionPending` | `BlockedAwaitingAuthoritySeal` |
| `UncertainManualReview` | `RequiresVerifiedUncertainOrAlreadyTerminal` |
| `ManualRejectedAuditPending` | `BlockedAwaitingAuthoritySeal` |
| `ManualRejectedTaskTransitionPending` | `BlockedAwaitingAuthoritySeal` |
| `ManualResolvedRejected` | `RequiresVerifiedNotDeliveredTerminal` |

`DurableStateProjection` 是 adapter route，不是新的业务状态表，也不能推进 cursor。W09 提供强引用后，模块私有构造逻辑才可产生相应 strong `DeliveryResult`；缺引用或 disposition/binding 不匹配只能返回 `Blocked(finalizer.terminal_ref_invalid|finalizer.binding_mismatch)`。

### 6.4 权限查询

`DeliveryResult` 提供 `view()` 与纯查询，不让调用方从 view 反向构造或自己重新解释权限：

- `authority_class() -> Strong | Compat | None`
- `completion_eligibility() -> PolicyBound | Never`
- `requires_manual_quarantine() -> bool`

规则固定：

- 只有 `TransportAccepted` 和经过精确绑定的 `AlreadyTerminal` 可以返回 `PolicyBound`；
- manual accepted 仍是 `AlreadyTerminal`，永远不是 `TransportAccepted`；
- compat 四类和 `Blocked` 永远 `Never`；
- `TransportRejected`、`TransportUncertain` 永远 `Never`。

## 7. W03：ReasonCode、RetryPolicy 与 CompletionPolicy

### 7.1 `ReasonCode`

使用闭集 enum 覆盖 RFC ReasonCode 表中的稳定代码。`as_str()` 是唯一持久字符串投影；`TryFrom<&str>` 只接受已注册值，未知值显式报错。

允许的 namespace 恰为：

```text
schedule, input, policy, intent, transport,
finalizer, activation, shadow, operator
```

控制流只比较 enum variant，不比较诊断文字。所有代码必须 ASCII、无 NUL、无第二层 namespace、字节长度 3--96 且字符串互不重复。新增 ReasonCode 必须修改 enum、parse/as_str 穷举和注册表测试，不能由调用方传自由文本。

### 7.2 `RetryPolicy`

```rust
pub enum RetryPolicy {
    Never,
    InputBackoff { not_before: UtcMicros },
    AuthorizedRejected {
        not_before: UtcMicros,
        max_attempts: NonZeroU32,
    },
}
```

`RetryDirective` 始终包含：

```text
reason: ReasonCode
eligibility: Never | NotBefore(UtcMicros) | RequiresRejectedAuthorization(...)
```

时间比较由调用方传入捕获的 `UtcMicros`，模块不读系统时间。`InputBackoff` 只适用于发送前输入/准备失败；`AuthorizedRejected` 同时需要 durable 当前授权、未超过 max attempts、当前 fence 和 not-before。发送后 `Uncertain` 无论 severity 都返回 `Never`。

### 7.3 `CompletionPolicy`

`CompletionPolicy` 字段私有且不可变，精确保存 RFC 字段：

```text
id, version, completion_owner,
advance_event,
schedule_close_policy,
notification_cursor_policy,
no_data_policy,
disabled_policy,
retry_policy,
uncertain_manual_policy,
already_terminal_policy,
allowed_authority,
finalizer_kind,
retention_class
```

W03 只实现一致性校验和纯判定；生产注册/owner/catalog SHA 绑定由 W06 通过 `pub(crate)` 受控构造入口完成，该入口不从 `monitor::push_job` 的公共接口导出。普通 producer 不能临时拼一个 policy。

### 7.4 完成判定 seam

模块提供一个纯函数：

```text
evaluate_completion(registered_policy, completion_fact)
    -> Result<CompletionDirective, PushJobError>
```

`CompletionDirective` 同时返回四个正交维度，避免用一个 Completed bool 丢失语义：

```text
schedule: KeepOpen | CloseVerifiedNoData | CloseExplicitDisabled |
          CloseSuppressedOccurrence | CloseOnAccepted
cursor:   Never | AdvanceAccepted | AdvanceManualAccepted
retry:    RetryDirective
manual:   None | QuarantineThenVerifiedManual
```

最低强制矩阵：

| 输入事实 | schedule | cursor | retry | manual |
| --- | --- | --- | --- | --- |
| verified `NoData` | policy 允许时 `CloseVerifiedNoData`，否则 KeepOpen | Never | Never | None |
| explicit `Disabled` | policy 允许时 `CloseExplicitDisabled`，否则 KeepOpen | Never | Never | None |
| `BlockedOnInput` | KeepOpen | Never | typed InputBackoff 或 Never | None |
| `Suppressed` | policy 允许时关闭该 occurrence，否则 KeepOpen | Never | Never/not-before eligibility | None |
| preparation `RetryableFailure` | KeepOpen | Never | typed InputBackoff 或 Never | None |
| `PermanentFailure` | KeepOpen | Never | Never | None |
| `TransportAccepted` | policy+binding 允许时 CloseOnAccepted | 仅 BoundCursor 可 AdvanceAccepted | Never | None |
| `TransportRejected` | KeepOpen | Never | 仅显式 AuthorizedRejected | None |
| `TransportUncertain` | KeepOpen | Never | Never | QuarantineThenVerifiedManual |
| manual accepted `AlreadyTerminal` | policy+binding 允许时 CloseOnAccepted | 仅 AcceptedOrManualBound 可 AdvanceManualAccepted | Never | None |
| manual not-delivered `AlreadyTerminal` | KeepOpen | Never | Never | None |
| compat 四类 | 仅 CompatibilityObservation | Never | Never | None |
| `Blocked` | KeepOpen | Never | 仅发送前 typed InputBackoff 或 Never | None |

`verified NoData` 需要受校验的 empty evidence 引用；普通空 Vec 或 provider failure 不能构造。`Disabled` 需要显式 activation/policy 证据；缺 capability 不能随意重标为 Disabled。

## 8. 错误合同

`PushJobError` 是闭集、可测试的领域错误，不把 `anyhow::Error` 暴露为控制流：

- `InvalidText` / `InvalidAscii` / `InvalidBusinessDate`
- `InvalidSha256`
- `UnknownSchemaVersion`
- `DuplicateConfiguredChannel`
- `AttemptedChannelNotConfigured`
- `MissingWeakOutcome`
- `InvalidReasonCode`
- `MissingVerifiedTerminal`
- `TerminalDispositionMismatch`
- `TerminalBindingMismatch`
- `PolicyOwnerMismatch`
- `PolicyViolation`

错误可以带最小字段名和稳定 ReasonCode，但不得包含消息正文、凭据、webhook、持仓或完整外部 payload。库代码不 panic、不 `unwrap`/`expect`；这些只允许在测试 fixture 中使用。

## 9. 测试设计

测试穿过 `monitor::push_job` 的公共接口，不读取私有字段或复制实现。

### 9.1 W01 RED/GREEN

1. canonical golden bytes/hash 与 RFC v1 规则一致。
2. 同一 business date、不同 source-contract 得到不同 `ScheduleOccurrenceId`/`IntentId`；原始 `OccurrenceId` 保持业务 occurrence 语义。
3. 不同 producer、owner、Unit 或 namespace 不碰撞。
4. 相同业务材料在 run id、wall clock、phase、generation、build 改变时 occurrence 不变；这些排除字段不出现在 identity 构造器。
5. payload/render/evidence SHA 改变不改变 intent ID。
6. 非规范日期、空值、NUL、过长值、非法 SHA 被构造器拒绝。
7. Test namespace 绑定 run identity，不能与 Production 相等。

### 9.2 W02 RED/GREEN

1. 表驱动覆盖当前十四个 `DecisionState`，每项等于本设计 §6.3。
2. match 不含 `_`；未来新增 durable state 触发编译失败。
3. rustdoc `compile_fail` 证明外部不能直接构造 `VerifiedTerminalRef`。
4. `CompatibilityEvidenceRef` 拒绝重复渠道、越界 attempted channel、缺弱结果。
5. BestEffort/Partial/NoChannel/AllFailed 的 `completion_eligibility` 恒为 Never。
6. Unknown 弱结果保持 Unknown，不按 Rejected 自动重试。
7. manual accepted 只能成为 `AlreadyTerminal`，不能成为 `TransportAccepted`。

### 9.3 W03 RED/GREEN

1. ReasonCode 表全量唯一，`as_str`/parse 双向一致且只含九个 namespace。
2. NoData 与 Disabled 可按各自 policy 关闭 schedule，但 cursor 恒不推进。
3. Uncertain 恒 KeepOpen + Never retry + quarantine manual，severity 不改变结果。
4. TransportRejected 没有显式授权时 Never；授权、attempt 上限、not-before 和 fence 任一不满足仍 Never/Blocked。
5. retry 输出保留输入 `ReasonCode` variant，不经过字符串再解析。
6. compat 结果只产生 CompatibilityObservation，不推进 schedule authority/cursor。
7. policy owner/authority/finalizer 组合不一致被拒绝。

## 10. 实施提交边界

设计确认后按小提交执行：

1. `docs: design push foundation contract kernel`：仅本设计文档。
2. `test: specify push identity contracts`：W01 RED。
3. `feat: add push identity contracts`：W01 GREEN。
4. `test: specify application delivery results`：W02 RED。
5. `feat: add typed application delivery results`：W02 GREEN。
6. `test: specify completion and retry policies`：W03 RED。
7. `feat: add push completion policy kernel`：W03 GREEN。
8. `docs: record W01-W03 implementation evidence`：结果、命令、剩余边界。

每个 RED 必须先以期望原因失败，不能用语法错误、缺 fixture 或未编译依赖冒充。每个 GREEN 只实现当前测试所需合同；refactor 不改变公共行为。

## 11. 验证门禁

最低验证命令：

```bash
cargo fmt --check
cargo test --lib monitor::push_job
cargo test --doc push_job
cargo test --lib durable_delivery::tests
cargo clippy --lib --all-features -- -D warnings
git diff --check
```

还必须验证：

- W01/W02/W03 的 WBS acceptance statement 各有直接测试名和结果；
- `src/bin/monitor/**`、notification、数据库 schema、activation/config 未接线；
- 相对实现基线的 diff 不含 `.env`、`data/**`、日志、持仓或 message body；
- 当前运行 monitor 的 PID/二进制未因开发自动变化；
- 规格评审与质量评审均无 blocking finding。

全仓测试如存在预存 flaky，必须单独记录命令、失败测试和与本变更的因果证据；不能用“历史问题”一句话跳过。

## 12. 回滚与运行安全

W01--W03 没有生产接线，回滚是按提交顺序 `git revert` 本切片；不删除或改写任何 durable/analytics/audit 数据。

本切片完成后仍不得：

- 重建或重启 production monitor 来“验收”纯合同；
- 将新类型接入 physical owner；
- 把当前 compatibility `pushed=1` 回填为 durable accepted；
- 修改五条已有 Uncertain decision 或盲目重发；
- 运行生产 schema DDL；
- 宣称 W04--W21 或任一 Migration Unit 已完成。

后续接线必须继续遵守 WBS 依赖：W04/W05 构造事实和 PreparedPush，W06 注册 policy/owner，W07/W08 保存 intent/outbox，W09 构造并重验 `VerifiedTerminalRef`，W10 才能 finalization；任何 Unit 在 W16--W20 门禁完成前不得晋级 physical owner。

## 13. 完成定义

只有同时满足以下条件，W01--W03 才能标记完成：

1. 本设计获用户确认；
2. 书面实施计划获自审并按 TDD 执行；
3. §9 的测试全部先 RED 后 GREEN；
4. §11 的命令以当前 HEAD fresh 运行并保留结果；
5. 独立规格/质量评审无 blocking finding；
6. diff 证明零 production wiring；
7. 实现结果文档逐项绑定 W01--W03 acceptance；
8. 正在运行的旧 release monitor 未被开发自动替换或重启。

这只完成 Foundation 的第一个切片。完整用户目标仍需继续完成 W04--W21、52 个 Unit 的 shadow/迁移/观察与最终逐项审计。
