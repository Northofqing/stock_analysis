# 推送 Foundation W13 P01/N02 专用 Conformance Adapter 设计

**状态：** 已实施并通过 W13 fresh 验证；双轴评审 finding 已关闭。保持零生产接线，结果见 `docs/push-system/implementation-w13-results-2026-09-07.md`。

**决策日期：** 2026-09-07

## 1. 目标与验收句

W13 把现有 P01 与 N02 两套高保证状态机适配到 W09 的唯一应用终态合同，不重写其状态机，也不增加第三种 delivery authority。WBS 的精确验收句为：

> P01 同日 claim 不分 render-mode；N02 accepted-window 独立于 N01 critical quota。

本切片依赖 W03、W09、W12。它实现专用 authority 的 exact read、conformance validation 和 `DeliveryResult` 投影；不做 MU-p01/MU-news-flash-aggregate 的 producer 接线、scheduler、business cursor finalization、activation、shadow、physical owner 切换或生产数据库选择。

## 2. 当前代码事实

### 2.1 P01

P01 已有三类入口：scheduled、compensation、startup recovery。真实完成 owner 是 durable SQLite 的：

```text
(business_date, PreopenNewsHot, None, GLOBAL)
  -> one immutable decision
  -> schedule_occurrence_identity = p01:{business_date}
```

`DeliveryEnvelope` 的 source binding 包含 `render_mode=Scheduled|Compensation`，rendered bytes 和 source evidence 也可能随模式变化；但 `business_date_once_claims` 的主键不含 render mode、render hash 或 source hash。因此 adapter 只能按同一业务日 owner 查询，不能把模式当成第二条 application occurrence。

现有 `inspect_business_date_once_claim` 已验证 claim→decision→envelope、Accepted result、disposition、attempt/fence 和 frozen delivery audit，但它只返回运行摘要，不返回 W09 所需的 exact terminal record。W13 增加 crate-private 专用只读模型，复用这些验证函数，不复制发送或恢复状态机。

### 2.2 N02

N02 的现有 NewsFlash authority 以 append-only audit chain 保存：

```text
SinkAttempt(reservation_identity_sha256, attempt_ordinal)
  -> Accepted | DefinitivelyRejected | Uncertain
```

N02 的 completion owner 是每个 `(business_date, window)` 的 accepted-window；四个合法窗口为 09:30、11:30、13:00、15:00。N01 的 critical accepted-event 与 daily quota 是另一完成域。当前 SourceOnly gate 不产生 N01 producer，不能因为 N02 有 Accepted 就改变 N01 quota、accepted event 或 activation。

现有 `reconcile_news_flash_business_date` 会同时恢复 accepted events 和 accepted windows，但 W13 不能把这个混合 snapshot 当作 N02 terminal receipt。它必须对一个 window 精确重读 attempt+terminal 两个 authoritative envelopes，并验证 reservation、ordinal、channel、source evidence、render、typed receipt 和 join hash。

## 3. 方案比较

### 方案 A：重写 P01/N02 到 generic durable coordinator

拒绝。它破坏蓝图“保留已有高保证状态机”的决定，制造数据迁移和双 owner 风险，也无法证明历史 P01/N02 authority 在切换时仍可恢复。

### 方案 B：把现有摘要/布尔结果直接包装为 `DeliveryResult`

拒绝。P01 的 `Pushed/AlreadyDelivered` 摘要和 N02 的 `accepted_windows` 集合都没有携带 W09 全字段 exact binding；这会让本地 bool、集合成员或日志冒充强 receipt。

### 方案 C：两个专用 exact reader + 一个统一 conformance module

采用。P01 reader 读取 durable claim；N02 reader 读取 NewsFlash audit chain。两个 adapter 分别验证领域特有不变量，最后都构造 W09 私有 `AuthorityTerminalRecord` 并调用同一个 `verify_terminal`。这样 source-specific complexity 留在深模块内部，调用方只看到两个验证入口和统一结果。

## 4. 模块与依赖方向

新增：

```text
src/push_foundation/dedicated_transport.rs
src/push_foundation/dedicated_transport_tests.rs
```

扩展：

```text
src/durable_delivery/model.rs
src/durable_delivery/coordinator.rs
src/event/mod.rs
```

依赖方向固定为：

```text
Foundation dedicated conformance
  -> P01 exact read port -> DurableDeliveryCoordinator
  -> N02 exact read port -> AuditDispatcher / authoritative EventEnvelope chain
  -> W09 TerminalAuthorityPort/verify_terminal
  -> W02 DeliveryResult
```

`durable_delivery` 与 `event` 不依赖 `push_foundation`；它们只返回各自领域的只读证据。生产 composition root 不在 W13 修改。

## 5. 外部 seam

W13 对 Foundation 内部提供两个 crate-private 深接口：

```rust
verify_p01_dedicated(
    snapshot,
    route,
    policy,
    source,
    verified_at,
) -> Result<DeliveryResult, DedicatedConformanceError>

verify_n02_dedicated(
    snapshot,
    window,
    route,
    policy,
    source,
    verified_at,
) -> Result<DeliveryResult, DedicatedConformanceError>
```

调用者不构造 `VerifiedTerminalRef`、`AuthorityTerminalRecord` 或 observation。测试和未来 production adapter 通过同一 seam。

`DedicatedConformanceRoute` 只包含：

- W09 `TerminalTemplateBinding`；
- 一个非空 `ChannelId` required channel。

它没有 sink、renderer、provider、scheduler、cursor 或 retry 能力。

## 6. P01 查询与绑定

`P01SameDayClaimKey` 由 adapter 从已认证 Ready snapshot 的 `business_date` 派生，规范值固定为：

```text
business_date
push_kind = PreopenNewsHot
sub_kind = None
scope_key = GLOBAL
legacy_occurrence = p01:{business_date}
```

接口和 canonical key 中明确没有：

- execution/render mode；
- render SHA；
- source evidence SHA；
- producer ID；
- 当前 tick 或启动原因。

P01 exact reader 返回 `Missing | PendingSeal | Terminal`。Terminal 必须包含：

- exact legacy envelope canonical bytes 与 SHA；
- legacy decision identity；
- current disposition ref；
- validated attempt binding；
- Accepted/Rejected/Uncertain/manual 的 exact evidence bytes 与 SHA；
- durable schema version。

conformance adapter 重新解析并验证 legacy envelope，然后检查：

1. claim key、business date、kind/subkind/scope、`p01:{date}` 完全匹配；
2. envelope canonical bytes、SHA 和 legacy decision identity 自洽；
3. Foundation Unit 必须是 `MU-p01`，subject 必须是 Global；
4. template ID 必须是 `preopen_news_hot_v1`；
5. legacy rendered SHA、source evidence fingerprint 与 Ready intent 精确一致；
6. source binding schema 为 `P01_SOURCE_BINDING_V1`，mode 只能是 Scheduled/Compensation；mode 仅作为证据验证，绝不进入 claim key或 application occurrence；
7. Accepted receipt channel（含 manual accepted 可选 receipt）必须匹配 required channel；
8. transport terminal 必须有 attempt；只有已验证的 pre-attempt rejection/manual terminal 可以没有 attempt。

适配后的 `decision_id`、intent、Unit、occurrence、audience 等应用字段来自同一已持久化 Ready snapshot；legacy decision/ref/evidence 则来自 exact reader。terminal binding hash 把两侧一次性绑定，finalizer 二查会重新执行整条读取和验证。

## 7. N02 查询与绑定

`N02AcceptedWindowKey` 只包含业务日和 typed window：

```text
09:30 | 11:30 | 13:00 | 15:00
```

它不包含 N01 event ID、critical threshold、accepted-event set、committed count、pending count 或 daily quota。N02 source port 也不暴露修改这些值的方法。

N02 exact reader 返回一个窗口的 `Missing | PendingSeal | Terminal`。同一 reservation 允许在明确 Rejected 后以递增 `attempt_ordinal` 重试，但每个 attempt 只能有一个终态，Uncertain 不得自动打开下一 attempt：

- 没有 SinkAttempt：Missing；
- 最新 attempt 无 terminal：PendingSeal；
- 恰好一个 Accepted：以该 Accepted 为不可撤销 Terminal，较早 Rejected 不改变 accepted-window；
- 尚无 Accepted 且最新 attempt 为 DefinitivelyRejected/Uncertain：映射相应 Terminal；
- 多个 Accepted、terminal 无 attempt、同 attempt 多终态、Rejected 后 ordinal 未递增、Uncertain 后出现新 attempt、链损坏或跨窗口混用：查询失败。

adapter 对 exact `EventEnvelope`/`PushRecord` 重新验证：

1. audit schema 必须为 NewsFlash v5 authoritative schema；
2. kind 固定 `news_flash_aggregated_v1`，decision key 固定 `window:{HH:MM}`；
3. business date/window、reservation identity、attempt ordinal、attempt envelope ID、attempt identity/SHA 完全相同；多 attempt 必须属于同一 reservation，并保持严格递增 ordinal；
4. ordered sources 重新计算为 evidence SHA；render SHA 与 Ready intent 精确一致；
5. template ID 固定 `news_flash_aggregated_v1`，Foundation Unit 固定 `MU-news-flash-aggregate`，subject 固定 Global；
6. Accepted typed remote receipt 的 channel 等于 attempt channel和 required channel；receipt `accepted_at` 只表示 transport acceptance，绝不替代 source `published_at/observed_at`；
7. DefinitivelyRejected 映射 Rejected，Uncertain 映射 Uncertain；二者都不能推进 completion；
8. evidence 保留 exact terminal envelope bytes，并由 hash 绑定；不把 `accepted_windows` 集合成员重新制造成 receipt。

N01 event/quota 数据即使同时存在、为空、耗尽或损坏，也不参与 N02 query key、N02 binding hash 和 N02 completion result。反向同样成立：N02 Accepted 不产生 N01 accepted-event/quota 写入。

## 8. 统一 W09 投影

两个 adapter 的 descriptor 分别固定为：

```text
P01Dedicated / p01-durable-v{schema}
N02Dedicated / news-flash-authority-v5
```

source record 通过专用校验后，adapter 才构造私有 `AuthorityTerminalRecord`。`binding_sha256` 由 adapter 计算，不接受 source 自报。随后 W09 再验证：

- policy allowed authority；
- namespace、application decision、intent、Unit、occurrence、business date、subject、audience；
- template/version、rendered SHA；
- terminal disposition/attempt compatibility；
- evidence exact bytes/SHA；
- canonical terminal binding SHA。

结果统一映射为 TransportAccepted、TransportRejected、TransportUncertain 或 AlreadyTerminal。COMPAT 结果没有进入此 seam 的路径。

## 9. 错误与失败关闭

错误至少区分：

- invalid Ready intent / wrong Unit / wrong subject；
- invalid route/template/channel；
- source unavailable；
- missing/pending terminal；
- P01 claim/envelope/source-binding mismatch；
- N02 window/attempt/terminal/receipt mismatch；
- W09 terminal verification failure。

任何解析失败、未知字段、SHA 漂移、额外终态、跨日期/窗口、错渠道、模式进入 claim identity、N01 数据影响 N02 结果均失败关闭。错误不携带 rendered bytes、receipt 正文或 source 原文。

## 10. TDD seam 与用例

测试只通过两个 `verify_*_dedicated` seam 与 source port，不直接测试私有映射函数。

### P01 tracer bullets

1. Scheduled 与 Compensation 两份不同 envelope 生成同一个 same-day query key；key canonical golden 不含 mode/render/source hash。
2. exact Accepted P01 terminal 通过 W09 映射 TransportAccepted；二查稳定。
3. business date、occurrence、kind、scope、render/source SHA、template、channel 任一漂移失败。
4. missing/pending/rejected/uncertain/manual disposition 完整映射；Uncertain 不得重发。
5. legacy envelope bytes/SHA、result/disposition/attempt 任一损坏失败关闭。

### N02 tracer bullets

1. 四个合法 window 可构造，其他窗口拒绝；key surface 不包含 N01 quota/event。
2. exact attempt+Accepted terminal 映射 TransportAccepted，receipt bytes 保留。
3. N01 accepted-event/quota 的不同状态不能改变同一 N02 query/result，source port 调用也没有 quota 参数。
4. window、decision key、reservation、ordinal、attempt join、evidence、render、channel、receipt time 任一漂移失败。
5. Missing、Pending、DefinitivelyRejected、Uncertain 及 Rejected→更高 ordinal→Accepted 完整映射；多 Accepted、同 attempt 多 terminal、Uncertain 后重试和无 attempt 失败关闭。

### 相邻回归

- W09 terminal authority；
- W12 generic transport；
- durable_delivery；
- event NewsFlash authority/reconcile；
- push_job 全模块；
- rustdoc 与 architecture docs。

## 11. 零生产行为证明

W13 完成时以下路径相对 W12 必须零变更：

- `src/bin/monitor/main.rs`、`p01.rs`、`notify.rs`、`news_aggregator_init.rs`；
- notification/sink/config/Cargo；
- production composition root 与 runtime singleton；
- migration/production SQL；
- activation manifest 与 physical owner。

允许变化仅限 Foundation conformance module、专用只读模型/reader、测试和文档。没有 caller 时不会查询 provider、发送消息、写业务 cursor、改 N01 quota 或改变任何调度。

## 12. 完成定义

W13 只有同时满足以下条件才完成：

1. P01 same-day key 从类型与 golden bytes 上排除 render mode；
2. P01 exact terminal 可经 W09 形成 P01Dedicated 强结果；
3. N02 window key/source port 从类型上排除 N01 quota/event；
4. N02 exact attempt+terminal 可经 W09 形成 N02Dedicated 强结果；
5. 两类 source corruption、wrong binding、missing/pending 和非 Accepted 结果均有 fail-closed 测试；
6. W09/W12/durable/event/push_job 相邻回归通过；
7. production wiring diff 为零，中文结果文档记录 fresh 证据与仍未完成边界。
