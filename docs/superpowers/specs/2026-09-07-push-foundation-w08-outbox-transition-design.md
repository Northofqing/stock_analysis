# 推送 Foundation W08 Outbox 与 Append-only Transition 设计

**状态：** 已批准、待 TDD 实现。W08 只建立未接生产的业务 intent repository；不接 monitor、scheduler、provider、sink、durable authority、finalizer 或生产数据库。

**决策日期：** 2026-09-07

## 1. 目标和证据边界

W08 落实 WBS 的唯一验收句：每个 intent 的版本连续、事件前驱 hash 相连，并且首次 outbox 提交与后续状态转换在逐边界崩溃后可恢复。权威输入是：

- `docs/push-system/push-system-implementation-rfc.md` 的“业务 outbox 字节恢复合同”“业务意图转换”“跨库恢复顺序”和故障矩阵；
- `docs/push-system/push-system-foundation.v1.sql` 中 W07 已冻结的 `push_intents`、`push_intent_transitions`、immutable/CAS/append-only trigger；
- `docs/Project_Architecture_Blueprint.html` 的 business DB 与 isolated durable DB 分离、跨库不原子、业务 finalizer 只在重验 authority 后推进的边界；
- W01--W05 的稳定 identity、canonical-v1、ReasonCode 与 PreparedPush exact-byte 合同。

W08 不构造 `VerifiedTerminalRef`，不读取 durable DB，不发送消息，不执行 Completed/NotDelivered 最终化，不推进通知游标。W09/W10 才能把私有 authority 证据带入受限终态转换；W08 不提供可伪造 terminal 字段的通用公开写口。

## 2. 深模块边界

新增 `src/push_foundation/intent_store.rs`，由 `BusinessIntentStore` 独占 rusqlite connection：

```text
InitialIntentDraft
  -> record_initial()
       BEGIN IMMEDIATE
       compare existing immutable row or INSERT version=0
       COMMIT
       post-commit readback -> Inserted | ExistingIdentical

NonTerminalTransition
  -> apply_nonterminal_transition()
       BEGIN IMMEDIATE
       read current row and previous event
       stable event_id + canonical bytes/hash
       exact state/version/lease CAS (affected rows must be 1)
       INSERT one transition event
       COMMIT
       post-commit readback -> Applied | AlreadyCommitted
```

仓储不暴露 raw `Connection`、任意 SQL、production default path 或 alternate schema。打开时只接受显式绝对、已存在、非 symlink 的普通文件；同一读写 handle 先通过 W07 schema attestation，再开启并读回 `foreign_keys=ON`、`recursive_triggers=ON`，随后才允许写。

## 3. 首次 intent/outbox

`InitialIntentDraft` 只允许三个不可变初始决定：

| 初始决定 | 初态 | reason | PreparedPush/render 组 |
| --- | --- | --- | --- |
| Ready | PendingDispatch | `intent.created` | 全部非空 |
| NoData | NoData | `intent.no_data` | 全部 NULL |
| Disabled | Disabled | `policy.disabled` | 全部 NULL |

Ready 构造器接收 W05 `PreparedPush` 和身份补充材料。实现必须：

1. 用 namespace、Unit、completion owner、source contract、occurrence、subject、audience 重新派生 `intent_id`；
2. 逐项核对 PreparedPush 的 intent、decision、Unit、occurrence、subject、source contract；
3. 将 `PreparedPush/v1` canonical snapshot exact bytes 存入 `prepared_push_bytes`；快照中的外部/渲染原始字节只保存 SHA-256 和长度；
4. 首次 render 原始字节原样存入 `rendered_bytes`；
5. 对两组 bytes 现场重算 `payload_sha256` / `rendered_sha256`，不信任调用方字符串；
6. 固定 version=0、lease_generation=0、无 lease、previous_state=NULL。

NoData/Disabled 从同一稳定身份派生 intent/decision，但不得构造 dummy bytes/hash。`evidence_sha256`、`template_sha256`、`source_contract_sha256` 仍是非空不可变绑定。

同一 exact draft 重试只读返回 `ExistingIdentical`。同一 intent/唯一业务身份但 immutable material 不同，不覆盖、不 `INSERT OR REPLACE`；返回 typed immutable conflict。将既有行隔离到 ResolutionRequired 的自动 CAS 属于转换能力的后续调用，不能在一次“创建”API 内隐藏第二个业务决策。

## 4. 持久字符串与 canonical 合同

SQLite TEXT 冻结为无歧义 representation：

- namespace：`Production` 或 `Test:<run_id>`；
- subject：`Global` 或 `Entity:<subject_value>`；
- job/state/disposition/reason：使用 RFC/DDL 大小写精确值；
- 时间：非负 UTC 微秒；version/lease generation 在写入前检查 i64 溢出。

PreparedPush 快照使用 domain `PreparedPush/v1`；字段与 W05 `prepared_push_value` 完全同源，禁止复制第二套字段清单。`payload_sha256` 是整份 `domain + NUL + canonical JSON` bytes 的 SHA-256。

transition `event_id` 使用 domain `IntentTransitionV1`，材料严格只有 `(intent_id, expected_version, result_version)`。事件 `canonical_sha256` 使用同一 domain 的独立 canonical event preimage，包含数据库中除 `canonical_sha256` 自身外的所有持久字段，包括 terminal nullable 字段和 `occurred_at`。第一事件 `previous_sha256=NULL`；第 N 个事件必须引用 N-1 的 `canonical_sha256`。

## 5. 非终态转换与 lease

W08 的公开转换只允许不需要 terminal authority 的 RFC 边：

- PendingDispatch -> AwaitingAuthority / NoData / Disabled；
- PendingDispatch、AwaitingAuthority、AwaitingFinalizer、Completed、NoData、Disabled -> ResolutionRequired；
- AwaitingAuthority、AwaitingFinalizer -> ResolutionRequired 的 uncertain/resolution conflict；
- PendingDispatch、AwaitingAuthority、AwaitingFinalizer、ResolutionRequired 的同态 lease 事件；
- AwaitingAuthority/AwaitingFinalizer 的同态阻塞事件。

AwaitingAuthority、AwaitingFinalizer、Completed、NotDelivered 等需要私有 authority 或 finalizer 的正向边不由通用 public API 构造。后续 W09/W10 复用同一个私有事务内核，并传入无法由外部调用者伪造的验证材料。

lease mutation 是 typed enum：保持、取得/接管、释放。取得必须 `lease_until > occurred_at`，generation 精确加一；未过期的其他 owner 不可抢占；释放只能由当前 owner 且 generation/version 均匹配。所有条件最终仍进入 SQL WHERE/trigger，而不是只在事务前用缓存检查。

## 6. 事务、CAS 和恢复结果

每次转换只能使用一个 business connection 的 `BEGIN IMMEDIATE`：

1. 在事务内读取 intent 当前 state/version/lease 与前一事件；
2. 计算 result_version，校验前驱链，计算稳定 event_id/canonical hash；
3. UPDATE WHERE 至少绑定 intent_id、from_state、expected_version，并按 lease action 增加 owner/until/generation 条件；
4. affected rows=0：显式 rollback，不追加事件；事务外重读稳定 event_id 和当前 row；
5. affected rows=1：立即 INSERT transition；任何 CHECK/FK/UNIQUE/trigger/SQL 错误均回滚整笔事务；
6. INSERT 成功后 COMMIT；提交后再次按 event_id 与 result_version 读回并重算全部字段/hash，读回不一致不返回成功。

返回值区分：

- `Applied`：本调用提交并读回 exact event；
- `AlreadyCommitted`：旧请求/提交确认丢失重试，库中已有完全相同的稳定事件；
- `Conflict`：当前状态或同 event_id 事实不同，且本调用零写；
- typed storage/integrity error：SQL 或持久事实不可信，不能冒充冲突或成功。

重试先查 stable event。若 event 完全相同，只返回原 receipt，不再 UPDATE/INSERT，更不触发 provider、render、sink 或 finalizer。若相同 result_version 被不同 actor/reason/time 占用，返回冲突，不能把 winner 改写成 caller 的请求。

## 7. 崩溃矩阵

| 边界 | 提交后事实 | 恢复断言 |
| --- | --- | --- |
| initial INSERT 前/后但 COMMIT 前 | 无 intent | 重试只创建一次 version=0 |
| initial COMMIT 后确认丢失 | exact intent 已存在 | 重试返回 ExistingIdentical，不改 bytes/time |
| transition CAS 后、event INSERT 前 | 事务整体回滚 | 旧 state/version 与事件数不变 |
| event INSERT 后、COMMIT 前 | 事务整体回滚 | 旧 state/version 与事件数不变 |
| transition COMMIT 后确认丢失 | 新 state/version + 一个 event | 同 command 返回 AlreadyCommitted，不追加第二条 |
| stale expected version / competing event | winner 保留 | loser 零写并返回 Conflict |
| stored bytes/hash/chain 漂移 | 原可疑事实保留 | fail closed，不返回 Ready/Applied |

测试 fault point 只在 `cfg(test)` 存在，不进入生产 API。测试数据库全部在独立临时目录；不得触碰生产 business/durable DB。

## 8. 完成门禁

1. Ready snapshot golden bytes/hash，NoData/Disabled NULL group。
2. initial exact retry、immutable conflict、唯一 identity/decision 竞争均零覆盖。
3. 至少三连续转换的 version=1/2/3、previous hash 和 canonical hash 应用重算一致。
4. CAS 后/append 后回滚、commit-ack-lost readback、stale conflict 的逐边界测试。
5. UPDATE/DELETE transition 被 schema 拒绝；仓储读回可识别直接篡改或断链。
6. W01--W07 回归、rustdoc、check、Clippy/rustfmt、架构文档验证器与 diff check。
7. `src/bin/monitor`、notification、durable delivery、config、Cargo manifests 与生产 DDL 相对 W07 零改动；monitor PID 不变。

W08 完成后仍只是零生产接线的 foundation。W09/W10、activation、逐 Unit shadow/cutover 和生产 migration 均未完成。
