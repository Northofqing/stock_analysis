# 推送 Foundation W05 语义投影与首次渲染封存设计

**状态：** 已设计，待按测试先行实现；不接生产 caller、catalog loader、presentation、数据库、sink 或 scheduler。

**决策日期：** 2026-09-07

## 1. 目标与边界

W05 只关闭 `SemanticProjection`、`JobDecision`、`PreparedPush` 和 exact rendered bytes 的内存合同。权威依据为：

- `docs/push-system/push-system-implementation-rfc.md` 的三张逐字段表、JobDecision 七分支表及 `PreparedPushIntent` 身份规则；
- `docs/push-system/push-system-wbs.v1.json` 的 W05 门禁：“同 facts 重建语义 hash 相同；首次渲染字节封存，重放不重新渲染”；
- `docs/Project_Architecture_Blueprint.md` / `.html` 的 L2/L3 分层、单次事实和 shadow 无副作用约束。

本切片不实现 W06 machine catalog，不持久化 W07 intent/outbox，不调用 W08--W12 authority/finalizer，也不迁移任何生产 Unit。现有 monitor 进程和推送路径保持不变。

## 2. 模块边界

新增私有模块 `src/monitor/push_job/projection.rs`，由 `push_job.rs` 暴露最小只读合同：

```text
PreparedFactsSnapshot
        │  same immutable facts
        ▼
DecisionProjector + SemanticInput
        │  pure, deterministic
        ├── SemanticProjection ── canonical bytes + SHA-256
        ├── non-ready JobDecision
        └── ReadyPreparation.render_once(FnOnce)
                    │
                    ▼
              PreparedPush ── first exact UTF-8 bytes + SHA-256
```

`projection.rs` 只依赖 `push_job` 内部 canonical、identity、facts、delivery 和 policy 类型；不依赖 HTTP、飞书、SQLite、全局时钟或 monitor runtime。

## 3. 类型设计

### 3.1 目录与业务输入分离

`ProjectionBinding` 是 W06 才能创建的目录绑定，包含：

- `unit_id`
- `audience`
- `monitor_kind: Option<MonitorKind>`
- `sub_kind`
- `completion_owner`
- `completion_policy_id/version`
- `template_id`

`MonitorKind` 是 catalog 中 65 个 kind 的闭集枚举，提供 `ALL` 和稳定 `as_str()`；不接受自由字符串。枚举外真实生产者由目录明确传 `None`，不能借未知字符串绕过闭集。

`SubKind` 只允许 `None` 或受校验的 `Registered(SubKindValue)`。是否获某个 kind 批准属于 W06 的目录交叉校验；W05 只保证值非空、不可变且规范化。

`SemanticInput` 只包含每次纯业务投影会变化的三个值：

- `business_subject`
- `severity`：`Emergency | Important | Info | Research`
- `suppression`：`Eligible | Suppressed { reason, eligible_after }`

目录字段不能由 producer 在每次调用中任意覆盖；业务输入不能从渲染文本反向解析。

### 3.2 `DecisionProjector`

`DecisionProjector` 在 `RunContext` 仍可只读时冻结：namespace、unit、occurrence、run-context SHA、template version 与 `ProjectionBinding`。构造时验证绑定来自同一 Unit；W06 将成为首个生产构造者，W05 仅提供测试 fixture。

调用 `project_semantics(&PreparedFactsSnapshot, SemanticInput)` 时必须：

1. 核对 snapshot 的 `run_context_sha256` 等于 projector 冻结值；
2. 由有序 `source_refs` 和 `model_output_refs` 派生 `EvidenceFingerprint/v1`；
3. 固定 RFC 前 12 个语义字段；
4. 将这 12 字段编码为 `SemanticProjection/v1` exact canonical bytes；
5. `sha256 = SHA256(canonical_bytes)`，摘要本身不进入输入。

函数没有时钟、I/O、随机数或可变全局状态。同一个 binding、context、facts 和 semantic input 必须得到逐字节相同的 canonical bytes 与 SHA。

### 3.3 `SemanticProjection` 14 字段

结构严格包含 RFC 的 14 个字段，字段私有，只提供 getter：

1. audience
2. monitor_kind
3. sub_kind
4. occurrence
5. business_subject
6. severity
7. suppression
8. completion_policy_id
9. completion_policy_version
10. evidence_fingerprint
11. template_id
12. template_version
13. canonical_bytes
14. sha256

`canonical_bytes` 的 Debug 沿用 `ExactBytes` 脱敏形式，不输出语义正文。

### 3.4 `PreparedPush` 11 字段

Ready 路径根据同一 context/facts/projection 构造：

1. `intent_id`：严格使用 W01 `PreparedPushIntent/v1` 的 namespace、unit、completion owner、source contract ID、occurrence、subject、audience；payload/render/evidence 摘要不得进入身份；
2. `decision_id`：`PreparedPushDecision/v1` 域内仅从稳定 `intent_id` 派生；
3. `unit_id`
4. `occurrence`
5. `subject`
6. `run_context_sha256`
7. `prepared_facts_sha256`
8. `semantic_projection_sha256`
9. `source_binding`：source contract ID/version、有序 source refs、evidence fingerprint；
10. `rendered_bytes`：renderer 第一次返回的 UTF-8 原始字节，不 trim、不补换行、不 JSON 重写；
11. `rendered_sha256`：直接由 exact bytes 派生。

同一 intent 的任一不可变字段或 rendered bytes 不一致，比较合同返回 `ResolutionRequired(IntentPayloadConflict)`；不能换一个 decision ID 绕过。

### 3.5 `JobDecision`

`JobDecision` 使用不透明结构和只读 `JobDecisionView`，避免调用方伪造 NoData/Ready：

- Ready(PreparedPush)
- NoData { reason, evidence_sha256 }
- Disabled { reason }
- BlockedOnInput { reason, retry_after }
- Suppressed { reason, eligible_after }
- RetryableFailure { reason, retry_after }
- PermanentFailure { reason }

每个分支 canonical 化 variant 标签和全部类型化 payload。NoData 只允许 `PreparedFacts.verified_empty=true`；其 evidence SHA 固定为 PreparedFacts 外层规范摘要。Ready 反向拒绝 verified-empty 或 suppressed projection。Suppressed 的 reason/time 必须与 projection 中冻结值相同。来源失败只能进入 BlockedOnInput/RetryableFailure/PermanentFailure，不能变成 NoData。

## 4. 首次渲染与重放状态机

`ReadyPreparation` 是不可 Clone 的单次能力：

```text
Open --render_once--> Rendering --valid UTF-8--> Sealed(PreparedPush)
                           └------invalid------> Failed

Open 之外再次 render_once：闭包调用前拒绝，rejected_count + 1
```

- renderer 是 `FnOnce(&SemanticProjection) -> Vec<u8>`；W05 不提供网络或模板查找入口；
- 进入闭包前 `attempt_count` 从 0 变 1；panic 后状态保持 Rendering，能力不重新开放；
- `PreparedPush` 持有首次 exact bytes。所谓重放是从同一 immutable PreparedPush 读取这些 bytes，不接受 renderer，也不重新计算模板；
- 无论 active 还是 shadow，都必须使用 W04 的同一 `PreparedFactsSnapshot`；各自 projection/render 可比较，但 shadow 没有持久化、发送或完成接口。

## 5. 错误与安全

新增类型化错误：

- context/facts binding mismatch；
- invalid sub-kind / invalid projection；
- invalid UTF-8 rendered bytes；
- projection 与 Ready facts/binding 不一致；
- render already attempted；
- same-intent immutable payload conflict。

错误和 Debug 只含类型、状态、reason、长度和 SHA，不含 canonical facts、模型正文、渲染正文或凭据。

## 6. TDD 验收矩阵

1. 65 个 MonitorKind 精确、唯一、round-trip；未知值拒绝。
2. 同一 facts 重建 projection exact bytes/SHA 相同；source/model 顺序变化导致 fingerprint/SHA 变化。
3. projection 14/14 字段与 canonical golden 完全一致；template version/context occurrence 强绑定。
4. context SHA 不匹配时，在任何 render 前拒绝。
5. Ready 11/11 字段、intent/decision golden、source binding 和 exact UTF-8 bytes 全部正确。
6. 有意空白保留；非 UTF-8 失败且正文不进入 Debug。
7. 第二次 render 在进入闭包前拒绝并计数；panic 后仍封闭。
8. 重放多次只返回首次 bytes；renderer 调用数保持 1。
9. 同 intent 不同 rendered bytes/任一不可变材料返回 ResolutionRequired；不同 intent 不误报同身份冲突。
10. 七个 JobDecision 分支逐一 round-trip；NoData 只接受 verified-empty；Ready 不接受 empty/suppressed；Uncertain 不存在于发送前 decision。
11. rustdoc compile-fail 证明 `ReadyPreparation` 不可 Clone。
12. relative production-wiring diff 继续为零；既有 W01--W04 golden 和测试不变。

## 7. 非目标与后续

- W06：用 machine catalog 构造 `ProjectionBinding` 并校验 65 kind、Unit、owner、policy、template 的 exact match。
- W07：持久化 PreparedPush/JobDecision；同 identity 漂移 CAS 到 ResolutionRequired；重启读取首次 bytes。
- W15：active/shadow 在同一 facts instance 上比较 decision、semantic SHA、rendered bytes/SHA、reason 和 completion proposal。
- 各 Migration Unit：必须另行完成 shadow、六门禁、single-owner 晋级和自然样本观察。

W05 完成只证明纯合同与封存机制可用，不证明生产已切换、消息已送达或问题已全部解决。
