# 推送 Foundation W01--W03 合同内核实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在不接入任何生产 producer 的前提下，完成 W01 稳定身份、W02 应用投递结果/十四态适配、W03 完成与重试策略的可编译、可验证 Rust 合同内核。

**Architecture:** 新增 `stock_analysis::monitor::push_job` 深模块作为唯一应用 seam，公共接口在 `src/monitor/push_job.rs`，identity/delivery/policy 实现保持私有。该模块只做确定性计算；现有 `durable_delivery` 继续拥有物理投递 authority，生产接线留给后续 W04--W21 和逐 Unit 迁移。

**Tech Stack:** Rust 2021、chrono、serde/serde_json、sha2、thiserror、现有 `durable_delivery::DecisionState`、Cargo test/rustdoc/Clippy。

**Spec:** `docs/superpowers/specs/2026-09-06-push-foundation-contract-kernel-design.md`

## Global Constraints

- 开发 worktree 固定为 `codex/push-reliability-20260905`；不得修改根 `master` 的既存冲突。
- 不修改或读取 `.env` 值、`data/**` 内容、生产数据库、持仓正文、消息正文、webhook 或凭据。
- 不修改 `src/bin/monitor/**`、`src/notification/**`、`src/durable_delivery/{coordinator.rs,schema.rs}`、activation/config 或任何 production owner。
- 不启动、停止、重启、重建或热替换当前 release monitor；本切片不做 production runtime acceptance。
- `push_job` 不依赖 rusqlite/Diesel/reqwest/gRPC/provider/LLM/sink/global clock/environment/filesystem。
- canonical preimage 固定为 `<ASCII domain tag><0x00><canonical UTF-8 JSON>`；对象键字典序、Option 显式 null、无 float、无尾换行。
- `OccurrenceId` 只绑定 business date/family/key；source-contract 防碰撞由 `ScheduleOccurrenceId` 与 `IntentId` 保证。
- `VerifiedTerminalRef` 没有 public 构造器、Default、通用 Deserialize 或 compatibility 转换；真实构造和重验证属于 W09。
- `DeliveryResult` 是不透明结果；调用方只能用受校验构造器并通过 `DeliveryResultView` 观察。
- 所有 RED 必须由缺失的目标合同或错误行为触发；不得以语法错误、坏 fixture 或缺外部依赖充当失败。
- 每个 GREEN 后运行该任务目标测试；最终 fresh 运行 fmt、push_job tests、rustdoc、durable tests、strict Clippy 和 diff checks。

---

## 文件结构

| 文件 | 动作 | 唯一职责 |
| --- | --- | --- |
| `src/monitor/mod.rs` | 修改 | 导出 `push_job` 模块；不做 composition |
| `src/monitor/push_job.rs` | 新建 | 唯一公共接口、re-export、公共错误类型 |
| `src/monitor/push_job/identity.rs` | 新建 | W01 值验证、canonical-v1、三种稳定身份 |
| `src/monitor/push_job/delivery.rs` | 新建 | W02 compatibility evidence、不透明结果、十四态 route |
| `src/monitor/push_job/policy.rs` | 新建 | W03 ReasonCode、retry、completion policy 与纯判定 |
| `src/monitor/push_job/tests.rs` | 新建 | 仅通过 `super` 公共 seam 的 W01--W03 行为测试；必要的 authority/verified-empty fixture 走 `cfg(test)` 私有入口 |
| `docs/push-system/implementation-w01-w03-results-2026-09-06.md` | 新建 | acceptance、命令、提交、零生产接线与剩余 W04+ 边界 |
| `docs/superpowers/specs/2026-09-06-push-foundation-contract-kernel-design.md` | 修改 | 最终将状态更新为 W01--W03 implemented/verified；不得宣称整体迁移完成 |

---

### Task 1: W01 RED——固定身份公共合同

**Files:**

- Modify: `src/monitor/mod.rs`
- Create: `src/monitor/push_job.rs`
- Create: `src/monitor/push_job/identity.rs`
- Create: `src/monitor/push_job/tests.rs`

**Interfaces:**

- Consumes: `chrono::NaiveDate`，设计文档 §5。
- Produces tests for: `Namespace`、受校验 ID 值、`BusinessDate`、`UtcMicros`、`Sha256Digest`、三种 identity material 与 derive 函数。

- [ ] **Step 1: 建立模块 scaffold，但不提供 W01 符号**

在 `src/monitor/mod.rs` 增加：

```rust
pub mod push_job;
```

创建 `src/monitor/push_job.rs`：

```rust
//! Application-level push contracts. This module is pure and has no runtime wiring.

mod identity;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PushJobError {
    #[error("invalid {field}: {reason}")]
    InvalidText {
        field: &'static str,
        reason: &'static str,
    },
    #[error("invalid business date: {0}")]
    InvalidBusinessDate(String),
    #[error("invalid sha256 for {field}")]
    InvalidSha256 { field: &'static str },
    #[error("UTC microseconds must be non-negative")]
    InvalidUtcMicros,
}

pub type Result<T> = std::result::Result<T, PushJobError>;

#[cfg(test)]
mod tests;
```

创建 `src/monitor/push_job/identity.rs`，只保留模块说明，使 RED 来自尚未实现的合同符号：

```rust
//! Stable W01 identity contracts.
```

- [ ] **Step 2: 写 W01 公共 seam 测试**

在 `src/monitor/push_job/tests.rs` 写入以下测试骨架和精确断言：

```rust
use super::{
    derive_intent_id, derive_occurrence_id, derive_schedule_occurrence_id, AudienceId,
    BusinessDate, CalendarId, CompletionOwnerId, IntentIdentityMaterial, Namespace,
    OccurrenceFamily, OccurrenceIdentityMaterial, OccurrenceKey, ProducerId, RunId,
    ScheduleOccurrenceIdentityMaterial, ScheduleOrTriggerId, Sha256Digest, SourceContractId,
    SourceContractVersion, SubjectId, UnitId, UtcMicros,
};

fn text<T>(value: &str, constructor: fn(String) -> super::Result<T>) -> T {
    constructor(value.to_owned()).expect("valid fixture")
}

fn occurrence_material() -> OccurrenceIdentityMaterial {
    OccurrenceIdentityMaterial::new(
        BusinessDate::parse("2026-09-06").expect("valid date"),
        text("daily", OccurrenceFamily::try_new),
        text("close", OccurrenceKey::try_new),
    )
}

#[test]
fn w01_occurrence_golden_hash_is_stable() {
    let id = derive_occurrence_id(&occurrence_material());
    assert_eq!(
        id.as_str(),
        "5752487f81e81737f173e01b354988cc335ae5cb537aa0cecd05bc613957cb7a"
    );
}

#[test]
fn w01_outer_identities_bind_source_contract_without_changing_raw_occurrence() {
    let occurrence = occurrence_material();
    let raw_id = derive_occurrence_id(&occurrence);
    let schedule = |source: &str| {
        derive_schedule_occurrence_id(&ScheduleOccurrenceIdentityMaterial::new(
            Namespace::Production,
            text("MU-close", UnitId::try_new),
            text("close-scheduled", ProducerId::try_new),
            text("schedule-close", ScheduleOrTriggerId::try_new),
            text("a-share-calendar", CalendarId::try_new),
            occurrence.clone(),
            text("owner-close", CompletionOwnerId::try_new),
            text(source, SourceContractId::try_new),
        ))
    };
    let intent = |source: &str| {
        derive_intent_id(&IntentIdentityMaterial::new(
            Namespace::Production,
            text("MU-close", UnitId::try_new),
            text("owner-close", CompletionOwnerId::try_new),
            text(source, SourceContractId::try_new),
            raw_id.clone(),
            SubjectId::Global,
            text("portfolio-owner", AudienceId::try_new),
        ))
    };

    assert_ne!(schedule("close-v1"), schedule("close-v2"));
    assert_ne!(intent("close-v1"), intent("close-v2"));
    assert_eq!(raw_id, derive_occurrence_id(&occurrence));
}

#[test]
fn w01_restart_and_generation_metadata_cannot_change_occurrence() {
    let before = derive_occurrence_id(&occurrence_material());
    let after = derive_occurrence_id(&occurrence_material());
    assert_eq!(before, after);
}

#[test]
fn w01_identity_value_types_reject_invalid_input() {
    assert!(UnitId::try_new(String::new()).is_err());
    assert!(ProducerId::try_new(" value".to_owned()).is_err());
    assert!(AudienceId::try_new("value\0hidden".to_owned()).is_err());
    assert!(BusinessDate::parse("2026-9-6").is_err());
    assert!(Sha256Digest::parse("payload", "ABC").is_err());
    assert!(UtcMicros::try_new(-1).is_err());
}

#[test]
fn w01_test_namespace_is_bound_to_run_id() {
    let one = Namespace::test(text("run-1", RunId::try_new));
    let two = Namespace::test(text("run-2", RunId::try_new));
    assert_ne!(one, two);
    assert_ne!(one, Namespace::Production);
    assert_eq!(
        text("source-v1", SourceContractVersion::try_new).as_str(),
        "source-v1"
    );
}
```

- [ ] **Step 3: 运行 RED 并保存预期失败**

Run:

```bash
cargo test --lib monitor::push_job::tests::w01_ -- --nocapture
```

Expected: FAIL with unresolved imports for W01 symbols from `super`; `PushJobError`、chrono、serde 或 sha2 不能出现 dependency failure。

- [ ] **Step 4: 提交 W01 RED**

```bash
git add src/monitor/mod.rs src/monitor/push_job.rs src/monitor/push_job/identity.rs src/monitor/push_job/tests.rs
git commit -m "test: specify push identity contracts"
```

---

### Task 2: W01 GREEN——实现 canonical-v1 和三层身份

**Files:**

- Modify: `src/monitor/push_job.rs`
- Modify: `src/monitor/push_job/identity.rs`

**Interfaces:**

- Consumes: Task 1 tests。
- Produces:

```rust
pub fn derive_occurrence_id(material: &OccurrenceIdentityMaterial) -> OccurrenceId;
pub fn derive_schedule_occurrence_id(
    material: &ScheduleOccurrenceIdentityMaterial,
) -> ScheduleOccurrenceId;
pub fn derive_intent_id(material: &IntentIdentityMaterial) -> IntentId;
```

- [ ] **Step 1: 在公共 seam 精确 re-export W01 类型**

在 `src/monitor/push_job.rs` 加入：

```rust
pub use identity::{
    derive_intent_id, derive_occurrence_id, derive_schedule_occurrence_id, AudienceId,
    BusinessDate, CalendarId, CompletionOwnerId, IntentId, IntentIdentityMaterial, Namespace,
    OccurrenceFamily, OccurrenceId, OccurrenceIdentityMaterial, OccurrenceKey, ProducerId, RunId,
    ScheduleOccurrenceId, ScheduleOccurrenceIdentityMaterial, ScheduleOrTriggerId, Sha256Digest,
    SourceContractId, SourceContractVersion, SubjectId, SubjectValue, UnitId, UtcMicros,
};
```

- [ ] **Step 2: 实现受校验字符串和标量类型**

`identity.rs` 使用私有 `ValidatedText` 和 macro 生成语义 newtype。验证规则固定为：1--512 UTF-8 bytes、无 NUL、`trim()` 后非空且必须等于原值；拒绝而不修改输入。

```rust
#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
struct ValidatedText(String);

impl ValidatedText {
    fn try_new(field: &'static str, value: String) -> Result<Self> {
        let valid = !value.is_empty()
            && value.len() <= 512
            && !value.contains('\0')
            && value.trim() == value;
        if !valid {
            return Err(PushJobError::InvalidText {
                field,
                reason: "must be 1..=512 UTF-8 bytes, trimmed, and contain no NUL",
            });
        }
        Ok(Self(value))
    }
}
```

macro 必须为 `RunId`、`UnitId`、`ProducerId`、`ScheduleOrTriggerId`、`CompletionOwnerId`、`SourceContractId`、`SourceContractVersion`、`CalendarId`、`OccurrenceFamily`、`OccurrenceKey`、`AudienceId` 生成 `try_new(String) -> Result<Self>` 与 `as_str() -> &str`。

`SubjectId` 固定为：

```rust
#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum SubjectId {
    Global,
    Entity(SubjectValue),
}
```

`SubjectValue` 是字段私有的 public validated newtype，仅暴露 `try_new(String)` 与 `as_str()`；它复用相同验证。`SubjectId::entity(String)` 是便捷构造器。这样 public enum 不泄露 private type，同时仍不允许未校验 Entity。

`BusinessDate::parse` 必须 parse 后再 format `%Y-%m-%d` 与输入逐字节相等。`UtcMicros::try_new` 拒绝负数。`Sha256Digest::parse(field, value)` 只接受 64 位小写 hex。

- [ ] **Step 3: 实现私有 canonical writer**

只接受模块自己构造的 `BTreeMap<&'static str, serde_json::Value>`：

```rust
fn canonical_digest(
    domain: &'static str,
    fields: BTreeMap<&'static str, serde_json::Value>,
) -> Sha256Digest {
    debug_assert!(domain.is_ascii() && !domain.contains('\0'));
    let json = serde_json::to_vec(&fields).expect("typed canonical fields serialize");
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(&json);
    Sha256Digest::from_bytes(hasher.finalize().into())
}
```

该私有 `expect` 只处理封闭的 String/integer/null typed map；不得接受外部 Serialize 或 float。若实现选择完全消除 `expect`，则 `derive_*` 可以返回 `Result<Id>`，但必须同步更新全部接口与测试，不得让签名分叉。

- [ ] **Step 4: 实现三种 material 和 derive 函数**

`OccurrenceIdentityMaterial` 保存 `BusinessDate`、family、key；domain `OccurrenceId/v1`。`ScheduleOccurrenceIdentityMaterial` 保存设计 §5.3 十项材料，domain `ScheduleOccurrence/v1`，JSON 同时含 `schema_version:"ScheduleOccurrence/v1"`。`IntentIdentityMaterial` 保存七项材料，domain `PreparedPushIntent/v1`。

`Namespace` canonical JSON 固定为：

```json
{"kind":"Production","run_id":null}
```

或：

```json
{"kind":"Test","run_id":"run-1"}
```

所有 material 字段私有；构造后不提供 setter。excluded metadata 不出现在 struct 或 derive 参数中。

- [ ] **Step 5: 运行 W01 GREEN 和回归**

```bash
cargo test --lib monitor::push_job::tests::w01_ -- --nocapture
cargo test --lib monitor::push_job -- --nocapture
cargo fmt --check
```

Expected: all W01 tests PASS；golden hash 精确为 `5752487f81e81737f173e01b354988cc335ae5cb537aa0cecd05bc613957cb7a`。

- [ ] **Step 6: 提交 W01 GREEN**

```bash
git add src/monitor/push_job.rs src/monitor/push_job/identity.rs src/monitor/push_job/tests.rs
git commit -m "feat: add push identity contracts"
```

---

### Task 3: W02 RED——固定 compatibility、结果权限和十四态 route

**Files:**

- Modify: `src/monitor/push_job/tests.rs`

**Interfaces:**

- Consumes: W01 `IntentId`、`UnitId`、`OccurrenceId`、`Sha256Digest`、`UtcMicros`。
- Produces tests for: `CompatibilityEvidenceRef`、不透明 `DeliveryResult`/view、authority/completion 查询、`DurableStateProjection`。

- [ ] **Step 1: 增加 compatibility fixture**

在 tests 中增加 helper，使用固定 64-hex evidence：

```rust
fn digest(byte: char) -> Sha256Digest {
    Sha256Digest::parse("fixture", &byte.to_string().repeat(64)).expect("valid sha")
}

fn channel(value: &str) -> ChannelId {
    ChannelId::try_new(value.to_owned()).expect("valid channel")
}

fn compat(
    configured: &[&str],
    attempted: &[&str],
    outcomes: &[(&str, WeakOutcomeKind)],
) -> super::Result<CompatibilityEvidenceRef> {
    CompatibilityEvidenceRef::try_new(
        CompatId::try_new("compat-1".to_owned())?,
        derive_intent_id(&IntentIdentityMaterial::new(
            Namespace::Production,
            UnitId::try_new("MU-cli".to_owned())?,
            CompletionOwnerId::try_new("owner-cli".to_owned())?,
            SourceContractId::try_new("cli-v1".to_owned())?,
            derive_occurrence_id(&occurrence_material()),
            SubjectId::Global,
            AudienceId::try_new("operator".to_owned())?,
        )),
        UnitId::try_new("MU-cli".to_owned())?,
        derive_occurrence_id(&occurrence_material()),
        configured.iter().map(|v| channel(v)).collect(),
        attempted.iter().map(|v| channel(v)).collect(),
        outcomes
            .iter()
            .map(|(name, kind)| WeakOutcome::new(channel(name), *kind, digest('a')))
            .collect(),
        digest('b'),
        UtcMicros::try_new(1_788_705_600_000_000)?,
    )
}
```

- [ ] **Step 2: 增加弱结果分支与权限测试**

```rust
#[test]
fn w02_compatibility_results_can_never_finalize_authoritatively() {
    let best = DeliveryResult::best_effort_accepted(
        compat(
            &["feishu", "wechat"],
            &["feishu", "wechat"],
            &[
                ("feishu", WeakOutcomeKind::Accepted),
                ("wechat", WeakOutcomeKind::Accepted),
            ],
        )
        .expect("valid compat"),
    )
    .expect("best effort matrix");
    assert_eq!(best.authority_class(), DeliveryAuthority::Compat);
    assert_eq!(best.completion_eligibility(), CompletionEligibility::Never);
    assert!(matches!(best.view(), DeliveryResultView::BestEffortAccepted(_)));

    let none = DeliveryResult::no_channel_configured();
    assert_eq!(none.reason_code(), Some(ReasonCode::TransportNoChannelConfigured));
    assert_eq!(none.completion_eligibility(), CompletionEligibility::Never);
}

#[test]
fn w02_compatibility_evidence_rejects_channel_shape_conflicts() {
    assert!(compat(
        &["feishu"],
        &["wechat"],
        &[("wechat", WeakOutcomeKind::Unknown)]
    )
    .is_err());
    assert!(compat(
        &["feishu", "feishu"],
        &["feishu"],
        &[("feishu", WeakOutcomeKind::Accepted)]
    )
    .is_err());
}

#[test]
fn w02_result_constructor_enforces_weak_outcome_matrix() {
    let mixed = compat(
        &["feishu", "wechat"],
        &["feishu", "wechat"],
        &[
            ("feishu", WeakOutcomeKind::Accepted),
            ("wechat", WeakOutcomeKind::Rejected),
        ],
    )
    .expect("valid evidence");
    assert!(DeliveryResult::best_effort_accepted(mixed.clone()).is_err());
    assert!(DeliveryResult::partially_accepted(mixed).is_ok());
}
```

- [ ] **Step 3: 增加十四态穷举期望表**

```rust
#[test]
fn w02_all_fourteen_durable_states_have_exact_routes() {
    use crate::durable_delivery::DecisionState::*;
    let cases = [
        (Reserved, DurableStateProjection::BlockedBeforeAttempt),
        (AttemptInFlight, DurableStateProjection::BlockedAwaitingReconciliation),
        (AcceptedAuditPending, DurableStateProjection::BlockedAwaitingAuthoritySeal),
        (AcceptedTaskTransitionPending, DurableStateProjection::BlockedAwaitingAuthoritySeal),
        (Delivered, DurableStateProjection::RequiresVerifiedAcceptedOrAlreadyTerminal),
        (RejectedAuditPending, DurableStateProjection::BlockedAwaitingReconciliation),
        (RejectedTaskTransitionPending, DurableStateProjection::BlockedAwaitingReconciliation),
        (RejectedDurable, DurableStateProjection::RequiresVerifiedRejectedOrAlreadyTerminal),
        (UncertainAuditPending, DurableStateProjection::BlockedAwaitingAuthoritySeal),
        (UncertainTaskTransitionPending, DurableStateProjection::BlockedAwaitingAuthoritySeal),
        (UncertainManualReview, DurableStateProjection::RequiresVerifiedUncertainOrAlreadyTerminal),
        (ManualRejectedAuditPending, DurableStateProjection::BlockedAwaitingAuthoritySeal),
        (ManualRejectedTaskTransitionPending, DurableStateProjection::BlockedAwaitingAuthoritySeal),
        (ManualResolvedRejected, DurableStateProjection::RequiresVerifiedNotDeliveredTerminal),
    ];
    assert_eq!(cases.len(), 14);
    for (state, expected) in cases {
        assert_eq!(classify_durable_state(state), expected, "state={state}");
    }
}
```

- [ ] **Step 4: 运行 W02 RED**

```bash
cargo test --lib monitor::push_job::tests::w02_ -- --nocapture
```

Expected: FAIL with unresolved W02 imports/types; W01 tests remain green when run separately。

- [ ] **Step 5: 提交 W02 RED**

```bash
git add src/monitor/push_job/tests.rs
git commit -m "test: specify application delivery results"
```

---

### Task 4: W02 GREEN——实现不透明结果和 durable adapter

**Files:**

- Create: `src/monitor/push_job/delivery.rs`
- Modify: `src/monitor/push_job.rs`
- Modify: `src/monitor/push_job/tests.rs`

**Interfaces:**

- Consumes: W01 identity types、`crate::durable_delivery::DecisionState`。
- Produces:

```rust
pub fn classify_durable_state(state: DecisionState) -> DurableStateProjection;
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

`CompatibilityEvidenceRef` 与 `VerifiedTerminalRef` 的精确字段分别在 Step 1、Step 2 定义；二者所有字段均为 private。`DeliveryResultView<'a>` 在 Step 3 完整列出九个命名分支，不使用 catch-all/Other 分支。

- [ ] **Step 1: 实现渠道和 compatibility evidence**

定义 `CompatId`、`ChannelId`、`TerminalRefId`、`DecisionId`、`AttemptId`、`TemplateId`、`TemplateVersion`、`DurableSchemaVersion` 为私有字段 newtype，复用 W01 的 `pub(super) validate_text`。

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum WeakOutcomeKind {
    Accepted,
    Rejected,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WeakOutcome {
    channel: ChannelId,
    kind: WeakOutcomeKind,
    local_evidence_sha256: Sha256Digest,
}
```

`CompatibilityEvidenceRef::try_new` 用 `BTreeSet<&str>` 验证 configured/attempted/weak-outcome channel：无重复、attempted 为 configured 子集、outcome channel 恰好等于 attempted 集合。保留输入 Vec 顺序；不得排序后掩盖调用方重复或顺序变化。

- [ ] **Step 2: 定义不透明 terminal ref**

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum AuthorityClass {
    GenericCounted,
    P01Dedicated,
    N02Dedicated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum TerminalDisposition {
    Accepted,
    Rejected,
    Uncertain,
    ManualConfirmedAccepted,
    ManualConfirmedNotDelivered,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedTerminalRef {
    ref_id: TerminalRefId,
    authority_class: AuthorityClass,
    namespace: Namespace,
    decision_id: DecisionId,
    attempt_id: Option<AttemptId>,
    intent_id: IntentId,
    unit_id: UnitId,
    occurrence: OccurrenceId,
    business_date: BusinessDate,
    subject: SubjectId,
    audience: AudienceId,
    template_id: TemplateId,
    template_version: TemplateVersion,
    rendered_sha256: Sha256Digest,
    terminal_disposition: TerminalDisposition,
    evidence_sha256: Sha256Digest,
    durable_schema_version: DurableSchemaVersion,
    verified_at: UtcMicros,
    binding_sha256: Sha256Digest,
}
```

只提供 getters。W02 不提供任何 production 构造器，也不定义可由 caller 填写的 `VerifiedTerminalParts`。`#[cfg(test)] pub(super) fn verified_terminal_fixture(...)` 只能在模块测试中构造固定 ref，供 W03 completion 矩阵使用。W09 将在接入 durable authority requery 时新增 production 构造器并校验 TerminalBinding SHA；在此之前 production code 不可能构造该类型。

增加 rustdoc：

```rust
/// Compatibility evidence cannot be upgraded into an authoritative terminal.
///
/// ```compile_fail
/// use stock_analysis::monitor::push_job::{
///     CompatibilityEvidenceRef, VerifiedTerminalRef,
/// };
/// fn forbidden(weak: CompatibilityEvidenceRef) -> VerifiedTerminalRef {
///     weak.into()
/// }
/// ```
```

把上面的 rustdoc 直接放在本 Step 已列出的实际 `VerifiedTerminalRef` 定义之前，不再声明第二个同名类型。

- [ ] **Step 3: 实现不透明 DeliveryResult**

W02 的公共结果需要携带 typed reason，因此先在 `delivery.rs` 定义仅覆盖本层分支的临时闭集 `ReasonCode`：`TransportRejected`、`TransportUncertain`、`TransportNoChannelConfigured`、`TransportAllChannelsFailed`、`TransportPartiallyAccepted`、`FinalizerTerminalRefInvalid`、`FinalizerBindingMismatch`，并提供对应 `as_str()`。W03 RED 会先用数量断言证明该闭集不完整；W03 GREEN 再将它迁到 `policy.rs` 并扩成最终 52 项。最终源码中只保留 `policy::ReasonCode` 一个定义。

模块私有 `DeliveryResultKind` 保存九个逻辑分支。public 构造器为：

```rust
impl DeliveryResult {
    pub fn best_effort_accepted(evidence: CompatibilityEvidenceRef) -> Result<Self>;
    pub fn partially_accepted(evidence: CompatibilityEvidenceRef) -> Result<Self>;
    pub fn no_channel_configured() -> Self;
    pub fn all_channels_failed(evidence: CompatibilityEvidenceRef) -> Result<Self>;
    pub fn blocked(reason: ReasonCode) -> Self;
    pub fn view(&self) -> DeliveryResultView<'_>;
    pub fn authority_class(&self) -> DeliveryAuthority;
    pub fn completion_eligibility(&self) -> CompletionEligibility;
    pub fn requires_manual_quarantine(&self) -> bool;
    pub fn reason_code(&self) -> Option<ReasonCode>;
}
```

`from_verified_terminal` 保持 `pub(super)`：Accepted→TransportAccepted，Rejected→TransportRejected，Uncertain→TransportUncertain，两个 manual disposition→AlreadyTerminal。compat constructors 分别验证 accepted count 与 configured count，不允许直接指定逻辑 branch。

- [ ] **Step 4: 实现十四态无 wildcard match**

```rust
pub fn classify_durable_state(state: DecisionState) -> DurableStateProjection {
    match state {
        DecisionState::Reserved => DurableStateProjection::BlockedBeforeAttempt,
        DecisionState::AttemptInFlight => {
            DurableStateProjection::BlockedAwaitingReconciliation
        }
        DecisionState::AcceptedAuditPending
        | DecisionState::AcceptedTaskTransitionPending => {
            DurableStateProjection::BlockedAwaitingAuthoritySeal
        }
        DecisionState::Delivered => {
            DurableStateProjection::RequiresVerifiedAcceptedOrAlreadyTerminal
        }
        DecisionState::RejectedAuditPending
        | DecisionState::RejectedTaskTransitionPending => {
            DurableStateProjection::BlockedAwaitingReconciliation
        }
        DecisionState::RejectedDurable => {
            DurableStateProjection::RequiresVerifiedRejectedOrAlreadyTerminal
        }
        DecisionState::UncertainAuditPending
        | DecisionState::UncertainTaskTransitionPending
        | DecisionState::ManualRejectedAuditPending
        | DecisionState::ManualRejectedTaskTransitionPending => {
            DurableStateProjection::BlockedAwaitingAuthoritySeal
        }
        DecisionState::UncertainManualReview => {
            DurableStateProjection::RequiresVerifiedUncertainOrAlreadyTerminal
        }
        DecisionState::ManualResolvedRejected => {
            DurableStateProjection::RequiresVerifiedNotDeliveredTerminal
        }
    }
}
```

不得合并 `AttemptInFlight` 与 audit-pending；它们的恢复 authority 不同。

- [ ] **Step 5: 运行 W02 GREEN、rustdoc 和 W01 回归**

```bash
cargo test --lib monitor::push_job::tests::w01_ -- --nocapture
cargo test --lib monitor::push_job::tests::w02_ -- --nocapture
cargo test --doc push_job
cargo fmt --check
```

Expected: W01/W02/rustdoc PASS；compile_fail 示例被 rustdoc 判定为预期不可编译。

- [ ] **Step 6: 提交 W02 GREEN**

```bash
git add src/monitor/push_job.rs src/monitor/push_job/delivery.rs src/monitor/push_job/tests.rs
git commit -m "feat: add typed application delivery results"
```

---

### Task 5: W03 RED——固定 ReasonCode、retry 和 completion 矩阵

**Files:**

- Modify: `src/monitor/push_job/tests.rs`

**Interfaces:**

- Consumes: W01 types、W02 `DeliveryResult`。
- Produces tests for: 52 个 ReasonCode、`RetryPolicy`/`RetryDirective`、`CompletionPolicyRegistration`、`evaluate_completion`。

- [ ] **Step 1: 写 ReasonCode 完整注册表测试**

测试中的 expected 数组必须逐字包含 RFC 的 52 个代码：

```rust
#[test]
fn w03_reason_code_registry_is_exact_and_round_trips() {
    let expected = [
        "schedule.not_trading_day", "schedule.window_not_open",
        "schedule.window_expired", "schedule.occurrence_closed",
        "schedule.occurrence_conflict", "schedule.window_open", "schedule.deferred",
        "input.source_recovered", "activation.ready", "input.source_unavailable",
        "input.source_unready", "input.evidence_invalid", "input.no_verified_batch",
        "input.account_snapshot_missing", "input.namespace_violation", "policy.disabled",
        "policy.starved", "policy.opt_in_disabled", "policy.cooldown_active",
        "policy.daily_budget_full", "policy.suppressed", "intent.payload_conflict",
        "intent.expected_version_conflict", "intent.lease_held",
        "intent.transition_conflict", "transport.rejected", "transport.uncertain",
        "transport.no_channel_configured", "transport.all_channels_failed",
        "transport.partially_accepted", "finalizer.terminal_ref_invalid",
        "finalizer.binding_mismatch", "finalizer.cas_conflict",
        "finalizer.deadline_exceeded", "finalizer.transition_append_failed",
        "activation.manifest_mismatch", "activation.generation_conflict",
        "activation.owner_conflict", "activation.core_unready",
        "activation.producer_unready", "shadow.semantic_diff",
        "shadow.side_effect_attempted", "operator.not_delivered",
        "operator.unauthorized", "operator.evidence_invalid",
        "operator.resolution_conflict", "intent.created", "intent.no_data",
        "intent.dispatch_claimed", "intent.authority_verified", "finalizer.completed",
        "activation.applied",
    ];
    assert_eq!(ReasonCode::ALL.len(), 52);
    assert_eq!(
        ReasonCode::ALL.iter().map(|code| code.as_str()).collect::<Vec<_>>(),
        expected
    );
    for code in ReasonCode::ALL {
        assert_eq!(ReasonCode::try_from(code.as_str()), Ok(code));
        assert!(code.as_str().is_ascii());
        assert!(!code.as_str().contains('\0'));
    }
    assert!(ReasonCode::try_from("transport.accepted").is_err());
}
```

先只加入这个测试并运行：

```bash
cargo test --lib monitor::push_job::tests::w03_reason_code_registry_is_exact_and_round_trips -- --nocapture
```

Expected: 测试可编译，但明确失败为 `left: 7, right: 52`；这证明 RED 来自 W02 临时 reason 闭集尚未扩展，而不是语法或依赖错误。

- [ ] **Step 2: 写 typed retry 测试**

```rust
#[test]
fn w03_retry_directive_preserves_reason_and_never_retries_uncertain() {
    let now = UtcMicros::try_new(100).expect("valid time");
    let retry = RetryPolicy::InputBackoff {
        not_before: UtcMicros::try_new(200).expect("valid time"),
    }
    .evaluate(now, 0, false, ReasonCode::InputSourceUnavailable);
    assert_eq!(retry.reason(), ReasonCode::InputSourceUnavailable);
    assert!(matches!(retry.eligibility(), RetryEligibility::NotBefore(value) if value.get() == 200));

    let eligible = RetryPolicy::InputBackoff {
        not_before: UtcMicros::try_new(200).expect("valid time"),
    }
    .evaluate(
        UtcMicros::try_new(200).expect("valid time"),
        0,
        false,
        ReasonCode::InputSourceUnavailable,
    );
    assert_eq!(eligible.eligibility(), RetryEligibility::EligibleInputRetry);

    let uncertain = RetryPolicy::Never.evaluate(
        now,
        1,
        true,
        ReasonCode::TransportUncertain,
    );
    assert_eq!(uncertain.eligibility(), RetryEligibility::Never);
    assert_eq!(uncertain.reason(), ReasonCode::TransportUncertain);
}
```

- [ ] **Step 3: 写 NoData/Disabled/Uncertain completion 正交矩阵**

使用 `cfg(test)` verified fixture；fixture 只能构造测试 evidence/ref，不能导出到 production：

```rust
#[test]
fn w03_no_data_disabled_and_uncertain_have_distinct_completion() {
    let policy = policy_fixture(
        NoDataPolicy::CloseVerifiedOccurrence,
        DisabledPolicy::CloseDisabledOccurrence,
        CursorPolicy::AcceptedBoundOnly,
        RetryPolicy::Never,
    );

    let no_data = evaluate_completion(
        &policy,
        CompletionFact::VerifiedNoData(verified_empty_fixture()),
    )
    .expect("valid no-data decision");
    assert_eq!(no_data.schedule(), ScheduleDirective::CloseVerifiedNoData);
    assert_eq!(no_data.cursor(), CursorDirective::Never);

    let disabled = evaluate_completion(
        &policy,
        CompletionFact::ExplicitDisabled(disabled_fixture(ReasonCode::PolicyDisabled)),
    )
    .expect("valid disabled decision");
    assert_eq!(disabled.schedule(), ScheduleDirective::CloseExplicitDisabled);
    assert_eq!(disabled.cursor(), CursorDirective::Never);

    let uncertain = evaluate_completion(
        &policy,
        CompletionFact::Delivery(uncertain_delivery_fixture()),
    )
    .expect("valid uncertain decision");
    assert_eq!(uncertain.schedule(), ScheduleDirective::KeepOpen);
    assert_eq!(uncertain.cursor(), CursorDirective::Never);
    assert_eq!(uncertain.retry().eligibility(), RetryEligibility::Never);
    assert_eq!(uncertain.manual(), ManualDirective::QuarantineThenVerifiedManual);
}
```

再加入：compat 全部 `CompatibilityObservation`/cursor Never；Rejected 只有 `AuthorizedRejected` 且 authorization=true、attempt<max、now>=not_before 才 eligible；policy owner/finalizer/authority 不一致构造失败。

- [ ] **Step 4: 运行 W03 RED**

```bash
cargo test --lib monitor::push_job::tests::w03_ -- --nocapture
```

Expected: 加入 Step 2--3 后 FAIL with unresolved W03 policy symbols；Step 1 已留下独立的 7→52 行为 RED 证据；W01/W02 tests 保持 green。

- [ ] **Step 5: 提交 W03 RED**

```bash
git add src/monitor/push_job/tests.rs
git commit -m "test: specify completion and retry policies"
```

---

### Task 6: W03 GREEN——实现闭集原因和纯完成判定

**Files:**

- Create: `src/monitor/push_job/policy.rs`
- Modify: `src/monitor/push_job.rs`
- Modify: `src/monitor/push_job/delivery.rs`
- Modify: `src/monitor/push_job/tests.rs`

**Interfaces:**

- Consumes: W03 RED tests、W02 result views。
- Produces:

```rust
pub fn evaluate_completion(
    policy: &CompletionPolicy,
    fact: CompletionFact<'_>,
) -> Result<CompletionDirective>;
```

- [ ] **Step 1: 将 W02 临时 ReasonCode 迁移并扩展为 52 variants**

variant 使用稳定 Rust 名称，例如：

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum ReasonCode {
    ScheduleNotTradingDay,
    ScheduleWindowNotOpen,
    ScheduleWindowExpired,
    ScheduleOccurrenceClosed,
    ScheduleOccurrenceConflict,
    ScheduleWindowOpen,
    ScheduleDeferred,
    InputSourceRecovered,
    ActivationReady,
    InputSourceUnavailable,
    InputSourceUnready,
    InputEvidenceInvalid,
    InputNoVerifiedBatch,
    InputAccountSnapshotMissing,
    InputNamespaceViolation,
    PolicyDisabled,
    PolicyStarved,
    PolicyOptInDisabled,
    PolicyCooldownActive,
    PolicyDailyBudgetFull,
    PolicySuppressed,
    IntentPayloadConflict,
    IntentExpectedVersionConflict,
    IntentLeaseHeld,
    IntentTransitionConflict,
    TransportRejected,
    TransportUncertain,
    TransportNoChannelConfigured,
    TransportAllChannelsFailed,
    TransportPartiallyAccepted,
    FinalizerTerminalRefInvalid,
    FinalizerBindingMismatch,
    FinalizerCasConflict,
    FinalizerDeadlineExceeded,
    FinalizerTransitionAppendFailed,
    ActivationManifestMismatch,
    ActivationGenerationConflict,
    ActivationOwnerConflict,
    ActivationCoreUnready,
    ActivationProducerUnready,
    ShadowSemanticDiff,
    ShadowSideEffectAttempted,
    OperatorNotDelivered,
    OperatorUnauthorized,
    OperatorEvidenceInvalid,
    OperatorResolutionConflict,
    IntentCreated,
    IntentNoData,
    IntentDispatchClaimed,
    IntentAuthorityVerified,
    FinalizerCompleted,
    ActivationApplied,
}
```

`ALL` 顺序与 Task 5 expected 完全一致；`as_str` 和 `TryFrom<&str>` 都使用无 wildcard 的完整 match（unknown parse 在 match 的最终 `other` 返回 `PushJobError::InvalidReasonCode(other.to_owned())`）。

从 `delivery.rs` 删除临时 7-variant 定义，改为引用 sibling `policy::ReasonCode`；公共 seam 只 re-export `policy::ReasonCode`，确保最终不存在双重 reason vocabulary。

- [ ] **Step 2: 实现 RetryPolicy 和 typed directive**

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RetryPolicy {
    Never,
    InputBackoff { not_before: UtcMicros },
    AuthorizedRejected {
        not_before: UtcMicros,
        max_attempts: NonZeroU32,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetryEligibility {
    Never,
    NotBefore(UtcMicros),
    EligibleInputRetry,
    EligibleAuthorizedRejected,
    RejectedAuthorizationRequired,
    AttemptsExhausted,
}
```

`evaluate(now, attempts, rejected_authorized, reason)`：Never 恒 Never；InputBackoff 在 `now < not_before` 时返回 NotBefore，到时或过时返回 `EligibleInputRetry`；AuthorizedRejected 依次检查授权、attempt limit、not-before，满足才 `EligibleAuthorizedRejected`。函数始终原样保存 ReasonCode，本模块只返回判定，不直接执行投递。

- [ ] **Step 3: 实现 policy 值和受控 registration**

定义设计 §7.3 的枚举：`AdvanceEvent`、`ScheduleClosePolicy`、`CursorPolicy`、`NoDataPolicy`、`DisabledPolicy`、`UncertainPolicy`、`AlreadyTerminalPolicy`、`FinalizerKind`、`RetentionClass`、`CatalogOwnerRef`。

`CompletionPolicyRegistration` 与 `CompletionPolicy::register` 均为 `pub(crate)`；字段类型化，不接受字符串 enum。验证：

- `BoundCursor` 只能配 AcceptedBoundOnly 和至少一个 strong authority；
- `CompatibilityObservation` 必须 Cursor Never 且 allowed_authority 为空；
- authority 列表无重复；
- policy completion owner 与 CatalogOwnerRef 一致；
- UncertainPolicy 固定 QuarantineThenVerifiedManual；
- AlreadyTerminalPolicy 固定 RequeryExactBinding。

- [ ] **Step 4: 实现 verified/non-ready fact 和 completion directive**

`VerifiedEmptyEvidenceRef`、`DisabledEvidenceRef` 字段私有，无 public 构造器。W04/W06 将来使用 `pub(super)` 构造；当前只提供 `cfg(test)` fixture。

```rust
pub enum CompletionFact<'a> {
    VerifiedNoData(&'a VerifiedEmptyEvidenceRef),
    ExplicitDisabled(&'a DisabledEvidenceRef),
    BlockedOnInput { reason: ReasonCode, retry_after: Option<UtcMicros> },
    Suppressed { reason: ReasonCode, eligible_after: Option<UtcMicros> },
    RetryableFailure { reason: ReasonCode, retry_after: Option<UtcMicros> },
    PermanentFailure { reason: ReasonCode },
    Delivery(&'a DeliveryResult),
}
```

`CompletionDirective` 四个字段及 getters 精确为 `ScheduleDirective`、`CursorDirective`、`RetryDirective`、`ManualDirective`。`evaluate_completion` 使用设计 §7.4 矩阵：

- NoData/Disabled 可按各自 policy 关闭 schedule，cursor Never；
- input/preparation failure 只使用 typed InputBackoff；
- accepted 只有 Strong + allowed authority + policy-bound 才能 Close/Advance；
- rejected 只计算 AuthorizedRejected eligibility；
- uncertain 固定 KeepOpen/Never/Quarantine；
- compat 固定 CompatibilityObservation/Never/Never；
- blocked 固定 KeepOpen/Never，不能从错误文本推断成功。

- [ ] **Step 5: 运行 W03 GREEN 和全部模块测试**

```bash
cargo test --lib monitor::push_job::tests::w01_ -- --nocapture
cargo test --lib monitor::push_job::tests::w02_ -- --nocapture
cargo test --lib monitor::push_job::tests::w03_ -- --nocapture
cargo test --lib monitor::push_job -- --nocapture
cargo test --doc push_job
cargo fmt --check
```

Expected: all PASS；ReasonCode count=52；NoData/Disabled/Uncertain 四维输出逐项不同。

- [ ] **Step 6: 提交 W03 GREEN**

```bash
git add src/monitor/push_job.rs src/monitor/push_job/delivery.rs src/monitor/push_job/policy.rs src/monitor/push_job/tests.rs
git commit -m "feat: add push completion policy kernel"
```

---

### Task 7: Fresh 验证、零接线证明和实现结果

**Files:**

- Create: `docs/push-system/implementation-w01-w03-results-2026-09-06.md`
- Modify: `docs/superpowers/specs/2026-09-06-push-foundation-contract-kernel-design.md`
- Verify: all W01--W03 source files and existing durable tests。

**Interfaces:**

- Consumes: Tasks 1--6 final HEAD。
- Produces: W01/W02/W03 acceptance evidence、零生产接线证明、后续 W04 起点。

- [ ] **Step 1: 运行格式和目标测试**

```bash
cargo fmt --check
cargo test --lib monitor::push_job -- --nocapture
cargo test --doc push_job
```

记录测试数量、通过数量和命令 exit code。

- [ ] **Step 2: 运行相邻 authority 回归和 strict Clippy**

```bash
cargo test --lib durable_delivery::tests -- --nocapture
cargo clippy --lib --all-features -- -D warnings
```

若出现预存失败，先在未改基线复现或用 git blame/独立测试证明因果；没有证据不得标成历史问题。

- [ ] **Step 3: 证明零 production wiring**

```bash
git diff aad7ac1..HEAD --name-only
git diff aad7ac1..HEAD -- src/bin/monitor src/notification config Cargo.toml Cargo.lock
rg -n "rusqlite|diesel|reqwest|grpc|std::env|NotificationService|tokio::spawn|File::|OpenOptions" \
  src/monitor/push_job.rs src/monitor/push_job
```

Expected:

- production wiring diff 为空；
- dependency scan 为空；只允许文档/测试文字中的禁用说明，不允许源实现调用；
- `Cargo.toml`/`Cargo.lock` 无变更。

- [ ] **Step 4: 运行文档和 diff 门禁**

```bash
ruby scripts/architecture-docs/test/rfc_input_spec_test.rb
ruby scripts/architecture-docs/test/source_catalog_spec_test.rb
ruby scripts/architecture-docs/test/catalog_spec_test.rb
ruby scripts/architecture-docs/test/rfc_spec_test.rb
ruby scripts/architecture-docs/test/wbs_spec_test.rb
git diff --check aad7ac1..HEAD
```

RFC strict 仍只允许既有 PROVISIONAL 发布阻断；本切片不能擅自改成 ACCEPTED。

- [ ] **Step 5: 写中文实现结果文档**

结果文档固定包含：

```markdown
# W01--W03 推送合同内核实现结果

## 范围与非声明
## 提交与文件
## W01 acceptance 证据
## W02 acceptance 证据
## W03 acceptance 证据
## Fresh 验证命令与结果
## 零生产接线证明
## 实际 monitor 隔离状态
## 已知限制与 W04--W21/52 Unit 剩余工作
```

每个 acceptance 表必须给测试名、源码符号、命令和结果；不得复制生产消息正文、持仓、receipt ID 或密钥。设计文档状态只更新为“W01--W03 implemented/verified”，结尾继续声明整体目标未完成。

- [ ] **Step 6: 自审并请求双轴代码评审**

自审清单：

- 设计 §5--§9 每项有代码或测试；
- public re-export 与计划签名一致；
- 无未使用 public escape hatch；
- 无 wildcard durable match；
- 无 compat→strong conversion；
- 无系统时间/I/O/global state；
- W01--W03 acceptance 与结果文档一一对应。

使用仓库 review/code-review 规则做 Standards 与 Spec 双轴评审；任何 blocking finding 修复后重新运行受影响测试和全部 fresh 门禁。

- [ ] **Step 7: 提交结果文档**

```bash
git add -f docs/push-system/implementation-w01-w03-results-2026-09-06.md \
  docs/superpowers/specs/2026-09-06-push-foundation-contract-kernel-design.md
git commit -m "docs: record W01-W03 implementation evidence"
```

- [ ] **Step 8: 最终当前切片审计**

```bash
git status --short --branch
git log --oneline aad7ac1..HEAD
git diff --check aad7ac1..HEAD
```

只有 W01--W03 全部证据完备时，才把本地总计划阶段 4--7 标记 complete。总目标保持 active，下一步按 WBS 进入 W04/W05/W06；不得调用 overall goal complete。

---

## 执行方式

本计划在当前隔离 worktree 内采用 **Inline Execution**：逐 Task 执行，Task 间做状态/测试检查点。原因是当前多代理槽位曾专用于实际 monitor 只读观察，且没有新的用户授权把运行时代码写入委派给其他代理；这不改变 TDD、独立评审或提交粒度。
