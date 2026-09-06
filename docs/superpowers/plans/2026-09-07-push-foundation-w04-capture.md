# 推送 Foundation W04 单次事实捕获实施计划

> 按 executing-plans + TDD 逐任务执行；每个 RED 必须由目标合同缺失或行为错误触发，每个 GREEN 必须先通过目标测试再提交。

**目标：** 在不连接任何生产 caller/provider/LLM/DB/sink 的前提下，实现 RFC 完整 `RunContext`、`PreparedFacts`、old/new 同实例 snapshot 与可计数的单次外部采集拒绝。

**架构：** 扩展既有 `monitor::push_job` 深模块。W01 canonical 编码抽到私有共享模块且 golden bytes/SHA 不变；`context.rs` 只负责 catalog-bound 运行上下文；`facts.rs` 只负责已捕获输入、PreparedFacts 与一次性 capture capability。W06 之前没有公开 production factory composition。

**技术：** Rust 2021、`std::sync::Arc`、`sha2`、`chrono`、现有 W01/W03 值类型和错误合同。

**设计：** `docs/superpowers/specs/2026-09-07-push-foundation-w04-capture-design.md`

## 全局约束

- 只在隔离 worktree `codex/push-reliability-20260905` 开发；不触碰根 master 冲突。
- 不修改 `src/bin/monitor/**`、`src/notification/**`、`src/durable_delivery/**`、config、activation、DB schema、Cargo features 或 `.env`。
- 不启动、停止、重建、替换当前 release monitor。
- 不读取系统当前时间；business time 必须由调用参数提供。
- 不把 `RunContext` 未声明的 source-contract ID 加进其 canonical schema；ID 由 `PreparationCapture` 的 catalog binding 私有保存。
- 不重新序列化 `ExactBytes`；facts SHA 只对原始 bytes 计算。
- 不让 acquisition error 产生 `verified_empty=true`。
- 第二次 `capture_once` 必须在调用闭包前失败并增加 rejection count；首次失败或 panic 也不重开 capability。
- old/new 必须克隆同一个 `Arc<PreparedFacts>`，不以“内容相同”替代实例同一。
- 只用定向 `rustfmt --edition 2021 <files>`；不得运行会机械修改全仓的 `cargo fmt -- <paths>`。

## 文件计划

| 文件 | 动作 | 职责 |
| --- | --- | --- |
| `src/monitor/push_job/canonical.rs` | 新建 | canonical-v1 null/string/u64/array/object、domain preimage 和 SHA |
| `src/monitor/push_job/identity.rs` | 修改 | 删除重复 encoder，复用 canonical；W01 bytes/SHA 零变化 |
| `src/monitor/push_job/context.rs` | 新建 | CalendarDate、PhaseEpic、Trigger、GitSha40、RunContext、crate-private catalog factory |
| `src/monitor/push_job/facts.rs` | 新建 | SourceRef/Time、ModelOutputRef、ExactBytes、Captured/PreparedFacts、snapshot、capture state |
| `src/monitor/push_job/policy.rs` | 修改 | 给 `VerifiedEmptyEvidenceRef` 增加显式受校验构造入口 |
| `src/monitor/push_job.rs` | 修改 | 私有模块声明、稳定 re-export、W04 精确错误类型 |
| `src/monitor/push_job/tests.rs` | 修改 | 只通过公共 seam 测 W04 行为；factory 仅用 `pub(super)` fixture |
| `docs/push-system/implementation-w04-results-2026-09-07.md` | 新建 | 验收、命令、提交、review、零接线和未完成边界 |

---

## Task 1：RunContext/canonical RED

**修改：** `push_job.rs`、`tests.rs`；尚不创建生产实现。

- [ ] 添加测试 import：`CalendarDate`、`PhaseEpic`、`TriggerView`、`GitSha40`、`ScheduleId`、`CommandId`、`AuthenticatedOperatorRef`、`TemplateVersion`、`RunContext`、`PreparationCapture`。
- [ ] 通过 `context::scheduled_capture_fixture()` 构造固定 context，并逐项断言 RFC 15 字段：schema=1、run/unit/namespace、两个日期、phase、trigger、occurrence、captured time、generation、build、catalog SHA、source-contract version、template version。
- [ ] 固定完整 `RunContext/v1\0{...}` preimage literal 和独立预计算 SHA literal；测试不得调用被测 encoder 生成期望值。
- [ ] 分别构造 Scheduled/Event/Manual trigger，断言只暴露对应字段；Event source ref 的 source-contract 必须与 catalog binding 一致。
- [ ] 添加无效 CalendarDate、GitSha40、trigger/catalog producer/schedule/source mismatch 的拒绝测试。
- [ ] 运行目标测试并确认 RED 来自 W04 符号缺失：

```bash
cargo test --lib monitor::push_job::tests::w04_run_context -- --nocapture
```

- [ ] 提交 RED：`test: specify W04 run context contract`。

## Task 2：canonical 抽取与 RunContext GREEN

### 2.1 canonical 无行为重构

- [ ] 新建 `canonical.rs`，把 W01 encoder 原样移动，并扩展 `Unsigned(u64)`、`Array(Vec<CanonicalValue>)`。
- [ ] `canonical_preimage`/`canonical_digest`/`raw_digest` 为 `pub(super)`；公共 API 不导出任意 canonical JSON hash。
- [ ] `Sha256Digest::from_bytes` 改成 `pub(super)`；`identity.rs` 复用新模块。
- [ ] 先运行所有 push_job tests，证明 W01 exact preimage 和 golden SHA 不变。

### 2.2 Event 所需的最小 SourceRef 值

- [ ] 在 `facts.rs` 先定义受校验 `SourceRefId`、`SourceProvider`、`ExternalId` 与不可变 `SourceRef`，供 RFC Event trigger 直接持有完整引用；其余 facts 聚合行为留给 Task 4。
- [ ] `SourceRef` 精确保存 source_ref_id/provider/external_id/source-contract ID/content SHA，并只提供只读 getter。

### 2.3 RunContext 值和 factory

- [ ] `context.rs` 定义严格 CalendarDate、GitSha40 与文本 ID；不实现 Default/Deserialize。
- [ ] `Trigger` 字段私有，公开 `TriggerView`；Scheduled/Event/Manual 构造只接收该分支完整材料。
- [ ] `CatalogRunBinding` 和其构造保持 `pub(crate)`；包含 exact namespace/unit/trigger rule/occurrence family/generation/build/catalog/source ID+version/template。
- [ ] `RunContextInput` 只包含每次捕获值，其中使用 `OccurrenceIdentityMaterial` 而不是已散列 ID；factory 验证 occurrence family 后派生 `OccurrenceId`。Test namespace 的内嵌 run ID 必须等于 context run ID。
- [ ] `RunContextFactory::begin_capture` 校验 binding 并返回 capture capability。
- [ ] `RunContext` 暴露只读 getter、`canonical_sha256()`；仅测试 fixture 暴露 exact preimage。
- [ ] Event source ref 与 binding source-contract mismatch 时 fail closed；Scheduled schedule ID、Event producer、Manual allowed kind 不匹配均拒绝。
- [ ] 运行定向 rustfmt 和所有 push_job tests。
- [ ] 提交 GREEN：`feat: add catalog-bound run context`。

---

## Task 3：PreparedFacts/单次捕获 RED

**修改：** `tests.rs`，不先写实现。

- [ ] `w04_exact_bytes_hash_original_payload`：用两组语义相近但字节不同的 JSON，以及包含非 UTF-8 的 bytes，断言 SHA 都按原字节且不同。
- [ ] `w04_source_times_are_total_ordered_and_allow_unknown`：两 source refs 对应 ObservedAt(Some)/AsOf(None)；时间语义保留，缺失、重复、错序、陌生 ID 全部拒绝。
- [ ] `w04_source_and_model_refs_are_frozen`：读取顺序等于输入顺序；重复 source ID、重复 model ref 拒绝。
- [ ] `w04_source_contract_binding_is_exact`：captured source ID 不等于 capture catalog binding、version 不等于 context，均失败且不产生 snapshot。
- [ ] `w04_failure_cannot_be_labeled_verified_empty`：typed acquisition failure 保留原 ReasonCode；没有 snapshot/empty completion evidence。
- [ ] `w04_verified_empty_requires_bound_evidence`：occurrence/source-contract 不匹配拒绝；完全绑定时 `verified_empty()` 才为 true。
- [ ] `w04_active_and_shadow_share_same_snapshot`：clone 后 `shares_instance_with` 为 true，model refs 只读且相同。
- [ ] `w04_second_capture_is_rejected_before_external_call`：两个闭包分别计数，第二个永远不执行；attempt=1/rejected=1/state=Sealed。
- [ ] `w04_failed_first_capture_is_also_single_use`：首次 typed failure 后第二闭包不执行；attempt=1/rejected=1/state=Failed。
- [ ] `w04_capture_unwind_does_not_reopen_capability`：外层 `catch_unwind(AssertUnwindSafe(...))` 后 state=Capturing；再次 capture 不执行闭包且 rejection 增加。
- [ ] 运行 W04 facts 测试并确认 RED 来自目标符号/行为缺失。
- [ ] 提交 RED：`test: specify W04 single-capture facts`。

## Task 4：PreparedFacts/单次捕获 GREEN

### 4.1 事实值对象

- [ ] 在 Task 2 的 SourceRef 基础上定义受校验 `ModelId`、`ModelVersion`、`ProtectedRef`；`SourceTime` 固定 source ref ID + `Option<UtcMicros>`。
- [ ] `ModelOutputRef` 固定 model/version/input SHA/output SHA/protected ref。
- [ ] `ExactBytes` 保存 `Vec<u8>` 并预先派生 SHA；只给只读 bytes/len/SHA。
- [ ] `VerifiedEmptyEvidenceRef::new` 显式接已经受校验的 occurrence/source ID/evidence SHA/time；不伪造失败分支，不提供 transport terminal 转换。
- [ ] `CapturedFacts::try_new` 检查 source ID 唯一、有序 time 一一对应、source contracts 相等、model ref 无重复、empty evidence 局部绑定。

### 4.2 PreparedFacts 和 snapshot

- [ ] 构造 `PreparedFacts` 时验证 captured source ID 等于 capture 私有 expected ID、version 等于 context version、empty evidence occurrence 等于 context occurrence。
- [ ] 字段精确对应 RFC 9 行；facts SHA 来自 exact bytes；只读 getters 不暴露内部 Vec 可变引用。
- [ ] `PreparedFactsSnapshot(Arc<_>)` 实现 Clone、`facts()` 和 `shares_instance_with()`；snapshot equality 等同 `Arc::ptr_eq`，PreparedFacts 自身不可 Clone。

### 4.3 capture 状态机

- [ ] `PreparationCapture` 初态 Open、attempt/rejected=0；预期 source ID 只存私有字段。
- [ ] `capture_once` 先检查 Open；非 Open 时 rejection+1 并返回 `AlreadyAttempted`，闭包未被调用。
- [ ] 真正 attempt 前 state=Capturing、attempt+1；success→Sealed(snapshot)，任意 returned error/validation error→Failed。
- [ ] 不 catch panic；unwind 后自然保留 Capturing，禁止第二次 attempt。
- [ ] `CaptureStateView` 只暴露四态；不暴露从外部把状态改回 Open 的入口。
- [ ] 定向 rustfmt，运行全部 push_job tests，提交 GREEN：`feat: add single-capture prepared facts`。

---

## Task 5：双轴 review 与修复

- [ ] Standards 轴：检查 public/private seam、panic/expect、mutable escape、secret/payload Display、过深浅模块、无关 diff、Clippy warning。
- [ ] Spec 轴：逐 RFC RunContext 15 行、PreparedFacts 9 行、W04 acceptance、蓝图 zero behavior 和本设计 TDD 表对照源码/测试。
- [ ] 特别反例：同值不同 Arc 不算共享；第一次失败后不得重采；Event source ID 漂移；unknown time 不得被 now 填充；verified empty 不得由 error 生成。
- [ ] review 发现行为问题时先写回归 RED，再修 GREEN；提交 `fix: align W04 capture contracts with review`（无修复则不制造空提交）。

## Task 6：fresh 验证与中文结果文档

- [ ] 定向格式检查：

```bash
rustfmt --edition 2021 --check \
  src/monitor/push_job.rs \
  src/monitor/push_job/canonical.rs \
  src/monitor/push_job/identity.rs \
  src/monitor/push_job/context.rs \
  src/monitor/push_job/facts.rs \
  src/monitor/push_job/policy.rs \
  src/monitor/push_job/tests.rs
```

- [ ] fresh 目标测试：

```bash
cargo test --lib monitor::push_job -- --nocapture
cargo test --doc push_job
cargo check --lib
```

- [ ] 非致命 Clippy 归因；如果 strict 被未修改基线阻断，记录首个目标外失败和 push_job 是否有 warning。
- [ ] `git diff --check`；确认以下路径相对 `c51115e` 无 diff：`src/bin/monitor`、`src/notification`、`src/durable_delivery`、config、Cargo files。
- [ ] 五项 architecture docs 验证器 fresh 运行并记录 runs/assertions。
- [ ] 只读确认 release monitor 仍是原 PID/命令；不将存活状态误当推送 authority。
- [ ] 新建中文 W04 结果文档，逐条记录 acceptance→代码行→测试→命令证据，并明确 W05--W21/52 Units 未完成。
- [ ] 更新 W04 设计状态为 implemented/verified only after fresh checks。
- [ ] 提交结果文档：`docs: record W04 implementation evidence`。

## 完成定义

W04 只有同时满足以下条件才可标记完成：

1. RunContext 15 字段和 PreparedFacts 9 字段都有受校验构造、只读访问和 RFC 对照；
2. W01 golden bytes/SHA 不变；
3. old/new 同一个 Arc snapshot，有明确实例同一测试；
4. success/failure/panic 后第二次闭包均未调用，且 rejection count 可观察；
5. typed source failure 永不变成 verified empty；
6. 目标测试、rustdoc/check/定向格式/diff 门禁有 fresh 输出；
7. production wiring 路径无修改，release monitor 未被替换；
8. 中文结果文档只声明 W04 切片完成，剩余 W05--W21 与 52 Unit 保持未完成。
