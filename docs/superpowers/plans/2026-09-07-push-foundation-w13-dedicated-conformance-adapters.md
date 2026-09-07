# 推送 Foundation W13 专用 Conformance Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 保留 P01/N02 现有高保证状态机，通过 exact requery 和专用不变量校验把它们投影为 W09/W02 的统一强 `DeliveryResult`。

**状态：** 已完成。代码验证 HEAD 为 `971a5fa`；fresh 证据与仍未完成的生产边界见 `docs/push-system/implementation-w13-results-2026-09-07.md`。

**Architecture:** durable coordinator 增加 P01 固定同日 claim 的 crate-private 只读终态；event authority 增加 N02 固定窗口的 crate-private attempt/terminal 只读终态。`push_foundation::dedicated_transport` 隐藏两个 reader 的差异，验证 P01 mode 不进入 claim、N02 不接触 N01 quota/event，再把合格证据交给 W09 `verify_terminal`。

**Tech Stack:** Rust、rusqlite、serde/serde_json、chrono、SHA-256、现有 W02/W07/W09/W12 contracts、现有 NewsFlash authoritative audit chain。

**Spec:** `docs/superpowers/specs/2026-09-07-push-foundation-w13-dedicated-conformance-adapters-design.md`

## Global Constraints

- 不重写 P01、N02 或 generic durable 状态机；不新增第三种 authority。
- P01 query key 固定 `(business_date, PreopenNewsHot, None, GLOBAL, p01:{date})`，接口/canonical bytes 不得含 render mode/hash/source hash。
- N02 query key 只含 business date 和 09:30/11:30/13:00/15:00 typed window，不得含 N01 event/quota。
- source record 必须 exact requery；本地 bool、摘要、accepted-window 集合或日志不能构造 `VerifiedTerminalRef`。
- source 自报 binding hash 不可信；W13 adapter 计算 W09 terminal binding。
- 不修改 monitor composition root、P01/N02 caller、notification/sink/config/Cargo、production SQL/activation/physical owner。
- 每个行为按 RED→GREEN 单独提交；不使用 `cargo fmt -- <file>`，只对目标文件运行 `rustfmt` 或审阅格式 diff。

## File Map

- Create `src/push_foundation/dedicated_transport.rs`: P01/N02 conformance 深模块、source ports、W09 adapter。
- Create `src/push_foundation/dedicated_transport_tests.rs`: W13 两个外部 seam 的行为测试。
- Modify `src/push_foundation/mod.rs`: 私有模块与测试模块注册。
- Modify `src/durable_delivery/model.rs`: P01 专用只读终态类型。
- Modify `src/durable_delivery/coordinator.rs`: P01 固定同日 claim exact query，复用既有 terminal join 校验。
- Modify `src/durable_delivery/mod.rs`: 仅 crate-private 导出 P01 只读类型。
- Modify `src/durable_delivery/tests.rs`: P01 reader 真实 SQLite 合同/损坏测试。
- Modify `src/event/mod.rs`: N02 typed window、exact window query 与链选择规则。
- Modify `src/event/push_record.rs` only if a read-only accessor is required; do not weaken `try_from_authoritative`.
- Modify `docs/push-system/implementation-w13-results-2026-09-07.md`: 中文完成证据。

---

### Task 1: P01 same-day key 与 durable exact reader

**Files:**
- Modify: `src/durable_delivery/model.rs`
- Modify: `src/durable_delivery/coordinator.rs`
- Modify: `src/durable_delivery/mod.rs`
- Test: `src/durable_delivery/tests.rs`

**Interfaces:**
- Consumes: `DeliveryEnvelope`, `business_date_once_claims`, existing disposition/result/attempt/audit validators.
- Produces:

```rust
pub(crate) enum P01DedicatedTerminalQuery {
    Missing,
    PendingSeal { state: DecisionState },
    Terminal(Box<P01DedicatedTerminalRecord>),
}

pub(crate) struct P01DedicatedTerminalRecord {
    pub(crate) legacy_decision_identity: String,
    pub(crate) envelope_canonical: Vec<u8>,
    pub(crate) envelope_sha256: String,
    pub(crate) ref_id: String,
    pub(crate) attempt_id: Option<String>,
    pub(crate) disposition: FoundationTerminalDisposition,
    pub(crate) evidence_bytes: Vec<u8>,
    pub(crate) evidence_sha256: String,
    pub(crate) durable_schema_version: i64,
}

impl DurableDeliveryCoordinator {
    pub(crate) fn inspect_p01_dedicated_terminal(
        &self,
        business_date: &str,
    ) -> Result<P01DedicatedTerminalQuery>;
}
```

- [x] **Step 1: 写 P01 query surface/golden RED**

在 `durable_delivery/tests.rs` 增加 `w13_p01_same_day_query_ignores_render_mode_but_reuses_one_claim`：分别构造 Scheduled/Compensation source binding 和不同 rendered bytes，证明同一日期最终只存在一个 `business_date_once_claims` owner；调用新 reader 只传日期，断言返回原 legacy decision。测试源码同时断言 query method 没有 mode 参数不是证据，真正证据是运行时第二 envelope 无法取得第二 claim。

- [x] **Step 2: 运行 RED**

Run: `cargo test --lib w13_p01_same_day_query_ignores_render_mode_but_reuses_one_claim -- --exact --test-threads=1`

Expected: FAIL because `inspect_p01_dedicated_terminal`/types do not exist.

- [x] **Step 3: 实现固定 P01 exact reader**

reader 内部固定：

```rust
let occurrence = format!("p01:{business_date}");
let kind = PushKind::PreopenNewsHot;
let sub_kind = DeliverySubKind::None;
let scope = "GLOBAL";
```

在同一 SQLite read transaction/connection 中验证 claim row、stored decision、envelope canonical SHA、`DeliveryEnvelope::canonical_bytes()`、legacy identity、日期/kind/subkind/scope/occurrence。非四种强终态返回 `PendingSeal`。

把 `build_foundation_terminal_record` 的 source-independent terminal join 提取为私有 `build_validated_terminal_evidence(connection, stored, envelope, required_channel)`；Foundation 传 `Some(binding.required_channel())`，P01 传 `None`。不得削弱 W12 required-channel 校验。

- [x] **Step 4: 运行 GREEN 与 W12 回归**

Run:

```bash
cargo test --lib w13_p01_same_day_query_ -- --test-threads=1
cargo test --lib w12_ -- --test-threads=1
```

Expected: W13 target PASS；W12 18/18 PASS。

- [x] **Step 5: 提交**

```bash
git add src/durable_delivery/model.rs src/durable_delivery/coordinator.rs src/durable_delivery/mod.rs src/durable_delivery/tests.rs
git commit -m "feat: expose exact P01 dedicated terminal read"
```

### Task 2: P01 conformance → W09

**Files:**
- Create: `src/push_foundation/dedicated_transport.rs`
- Create: `src/push_foundation/dedicated_transport_tests.rs`
- Modify: `src/push_foundation/mod.rs`

**Interfaces:**
- Consumes: Task 1 reader, W07 `IntentSnapshot::attested_ready_binding`, W09 `verify_terminal`.
- Produces:

```rust
pub(crate) struct DedicatedConformanceRoute {
    template: TerminalTemplateBinding,
    required_channel: ChannelId,
}

pub(crate) trait P01DedicatedTerminalSource {
    fn requery_p01(
        &self,
        business_date: &BusinessDate,
    ) -> Result<P01DedicatedTerminalQuery, DedicatedSourceFailure>;
}

pub(crate) fn verify_p01_dedicated(
    snapshot: &IntentSnapshot,
    route: &DedicatedConformanceRoute,
    policy: &CompletionPolicy,
    source: &dyn P01DedicatedTerminalSource,
    verified_at: UtcMicros,
) -> Result<DeliveryResult, DedicatedConformanceError>;
```

- [x] **Step 1: 写 P01 Accepted RED**

测试建立 `MU-p01`、Global subject、P01 occurrence 的真实 W07 Ready snapshot；fake source 返回 exact Scheduled legacy envelope + Accepted terminal。断言：source 只收到日期，结果是 `TransportAccepted`，authority class 为 `P01Dedicated`，application decision/intent/occurrence 来自 Ready snapshot，evidence SHA 来自 P01 exact terminal。

- [x] **Step 2: 运行 RED**

Run: `cargo test --lib w13_p01_dedicated_maps_exact_accepted_through_w09 -- --exact --test-threads=1`

Expected: FAIL because dedicated module does not exist.

- [x] **Step 3: 最小 GREEN**

实现 `DedicatedConformanceRoute::try_new`、source trait 和 coordinator adapter。`verify_p01_dedicated` 必须检查：

```rust
attested.unit_id.as_str() == "MU-p01"
attested.subject == SubjectId::Global
route.template.template_id().as_str() == "preopen_news_hot_v1"
legacy.business_date == attested.business_date.as_str()
legacy.schedule_occurrence_identity == format!("p01:{}", attested.business_date.as_str())
legacy.source_evidence_fingerprint == attested.source_evidence_fingerprint.as_str()
legacy.rendered_content_sha256 == attested.rendered_sha256.as_str()
```

解析 P01 source binding 为 closed JSON object，要求 `schema_version=P01_SOURCE_BINDING_V1` 且 `render_mode` 仅 Scheduled/Compensation。mode 只验证、不进入 source query、W09 decision 或 terminal binding。构造 `AuthorityTerminalRecord` 后由 `terminal_binding_sha256` 计算 hash，再调用 `verify_terminal`。

- [x] **Step 4: 运行 GREEN**

Run: `cargo test --lib w13_p01_dedicated_maps_exact_accepted_through_w09 -- --exact --test-threads=1`

Expected: PASS.

- [x] **Step 5: 提交**

```bash
git add src/push_foundation/dedicated_transport.rs src/push_foundation/dedicated_transport_tests.rs src/push_foundation/mod.rs
git commit -m "feat: adapt P01 dedicated authority to W09"
```

### Task 3: P01 disposition 与 corruption matrix

**Files:**
- Modify: `src/push_foundation/dedicated_transport.rs`
- Modify: `src/push_foundation/dedicated_transport_tests.rs`
- Modify: `src/durable_delivery/tests.rs`

**Interfaces:**
- Consumes: Task 1/2 P01 seam.
- Produces: fail-closed P01 conformance for all supported terminal dispositions.

- [x] **Step 1: 写逐字段 RED**

增加表驱动测试：business date、legacy occurrence、kind、subkind、scope、legacy decision、envelope bytes/SHA、source binding schema/mode、source fingerprint、rendered SHA、Unit、subject、template ID、required channel、attempt/disposition 任一漂移均返回 typed conformance/terminal error，不返回强结果。

- [x] **Step 2: 写 disposition RED**

真实 SQLite/fake source 覆盖 Accepted、Rejected、Uncertain、ManualAccepted、ManualNotDelivered、Missing、Pending；断言 Rejected/Uncertain completion eligibility 为 Never，manual result 保留 `AlreadyTerminal`，transport terminal attempt 规则由 W09 复验。

- [x] **Step 3: 运行 RED**

Run: `cargo test --lib w13_p01_ -- --test-threads=1`

Expected: new mutation/disposition cases fail.

- [x] **Step 4: 最小修复并运行 GREEN**

补足 closed-object、receipt channel、attempt/disposition 和 exact hash 校验，不添加 production bypass。

Run:

```bash
cargo test --lib w13_p01_ -- --test-threads=1
cargo test --lib push_foundation::terminal_authority_tests:: -- --test-threads=1
cargo test --lib w12_ -- --test-threads=1
```

- [x] **Step 5: 提交**

```bash
git add src/push_foundation/dedicated_transport.rs src/push_foundation/dedicated_transport_tests.rs src/durable_delivery/tests.rs
git commit -m "test: close P01 dedicated conformance gaps"
```

### Task 4: N02 exact accepted-window reader

**Files:**
- Modify: `src/event/mod.rs`
- Modify: `src/event/push_record.rs` only for read-only access if necessary.

**Interfaces:**
- Consumes: `AuditDispatcher::read_authoritative_year`, `PushRecord::try_from_authoritative`, existing NewsFlash attempt/terminal validators.
- Produces:

```rust
pub(crate) enum NewsFlashWindow {
    H0930,
    H1130,
    H1300,
    H1500,
}

pub(crate) enum NewsFlashWindowTerminalQuery {
    Missing,
    PendingSeal,
    Terminal(Box<NewsFlashWindowTerminalRecord>),
}

pub(crate) struct NewsFlashWindowTerminalRecord {
    pub(crate) attempt: EventEnvelope,
    pub(crate) terminal: EventEnvelope,
}

pub(crate) fn requery_news_flash_window_terminal_with(
    dispatcher: &AuditDispatcher,
    business_date: NaiveDate,
    window: NewsFlashWindow,
) -> Result<NewsFlashWindowTerminalQuery, NewsFlashReconcileError>;
```

- [x] **Step 1: 写 N02 reader RED**

使用现有 test dispatcher append SinkAttempt/Accepted authoritative envelopes，断言只按 `window:09:30` 读取 exact pair；同时写 Rejected ordinal 1→Accepted ordinal 2 的合法序列，断言最终 Accepted 不可撤销。

- [x] **Step 2: 写非法序列 RED**

覆盖：attempt 无 terminal→Pending、无 attempt terminal、同 attempt 多 terminal、重复 Accepted、ordinal 回退/重复、Uncertain 后更高 attempt、跨日期/跨窗口 join。所有结构损坏必须 `InvalidChain/RecordConflict`。

- [x] **Step 3: 运行 RED**

Run: `cargo test --lib w13_n02_window_reader_ -- --test-threads=1`

Expected: FAIL because window reader/types do not exist.

- [x] **Step 4: 实现选择状态机**

扫描前先复用 authoritative chain validation；只纳入：

```text
kind = news_flash_aggregated_v1
decision_key = window:{typed label}
business_date = requested date
```

每个 attempt 必须唯一并有至多一个 terminal；所有 attempt 的 reservation 相同；ordinal 严格递增；DefinitivelyRejected 才允许下一 ordinal；Uncertain/Open/Accepted 后出现新 attempt 为冲突；Accepted 总数必须 ≤1。返回规则为 Accepted 优先不可撤销，否则最新 attempt 的 Pending/Rejected/Uncertain。

- [x] **Step 5: 运行 GREEN 与现有 NewsFlash 回归**

Run:

```bash
cargo test --lib w13_n02_window_reader_ -- --test-threads=1
cargo test --lib event:: -- --test-threads=1
```

- [x] **Step 6: 提交**

```bash
git add src/event/mod.rs src/event/push_record.rs
git commit -m "feat: requery exact N02 accepted-window authority"
```

### Task 5: N02 conformance → W09 与 N01 分域证明

**Files:**
- Modify: `src/push_foundation/dedicated_transport.rs`
- Modify: `src/push_foundation/dedicated_transport_tests.rs`

**Interfaces:**
- Consumes: Task 4 reader, W09 terminal verification.
- Produces:

```rust
pub(crate) trait N02DedicatedTerminalSource {
    fn requery_n02(
        &self,
        business_date: &BusinessDate,
        window: NewsFlashWindow,
    ) -> Result<NewsFlashWindowTerminalQuery, DedicatedSourceFailure>;
}

pub(crate) fn verify_n02_dedicated(
    snapshot: &IntentSnapshot,
    window: NewsFlashWindow,
    route: &DedicatedConformanceRoute,
    policy: &CompletionPolicy,
    source: &dyn N02DedicatedTerminalSource,
    verified_at: UtcMicros,
) -> Result<DeliveryResult, DedicatedConformanceError>;
```

- [x] **Step 1: 写 N02 Accepted RED**

建立 `MU-news-flash-aggregate`、Global、window occurrence 的 W07 Ready snapshot。source 返回 exact attempt/Accepted terminal，断言统一结果为 `TransportAccepted`、authority class `N02Dedicated`，evidence bytes 等于 exact terminal envelope canonical bytes，receipt `accepted_at` 不等于也不覆盖 sources 的 published/observed time。

- [x] **Step 2: 写 N01 independence RED**

定义 fake source 只实现 `(business_date, window)` 方法；分别改变测试夹具中独立的 N01 accepted-event/quota 观察值，重复验证同一 N02 record，断言 query 参数、terminal binding、result 全部相同且无 N01 写调用。不得给 production trait 增加 quota/event 参数来让测试通过。

- [x] **Step 3: 运行 RED**

Run: `cargo test --lib w13_n02_ -- --test-threads=1`

Expected: adapter tests fail before implementation.

- [x] **Step 4: 最小 GREEN**

验证 authoritative EventEnvelope 的 canonical reserialization、`PushRecord::try_from_authoritative`、schema/kind/decision key/date/window、reservation/ordinal/attempt join、ordered source evidence、render SHA、required channel 和 typed receipt。映射 Accepted/DefinitivelyRejected/Uncertain，计算 W09 binding，不读取 snapshot 中不存在的 N01 状态。

- [x] **Step 5: 运行 GREEN**

Run:

```bash
cargo test --lib w13_n02_ -- --test-threads=1
cargo test --lib push_foundation::terminal_authority_tests:: -- --test-threads=1
```

- [x] **Step 6: 提交**

```bash
git add src/push_foundation/dedicated_transport.rs src/push_foundation/dedicated_transport_tests.rs
git commit -m "feat: adapt N02 window authority to W09"
```

### Task 6: 双轴评审与边界修复

**Files:**
- Modify only W13 target files if review finds a real gap.

**Interfaces:**
- Consumes: complete W13 diff relative to `cdea840`.
- Produces: standards/spec findings with every high/medium issue fixed or explicitly evidenced as out of scope.

- [x] **Step 1: Spec review**

逐条对照 W13 WBS acceptance、RFC adapter conformance 表、设计 §5--§12，确认每条都能指向行为测试。重点反例：P01 mode 不能改变 query/occurrence；N02 N01 quota 不可达；source self-reported SHA 不能跳过重算；Accepted 不可撤销；Uncertain 不重试。

- [x] **Step 2: Standards review**

扫描目标 diff 的 panic、secret/raw payload Debug、自由构造强 authority、生产 public surface、重复状态机、越层依赖和无界集合。命令：

```bash
git diff cdea840 -- src/push_foundation src/durable_delivery src/event src/monitor
git diff cdea840 -- '*.rs' | rg '^\+.*(unwrap\(|expect\(|panic!|unreachable!)'
```

- [x] **Step 3: 先写 RED 再修每个发现**

每个行为缺口先加一个会失败的外部 seam 测试；运行单测确认 RED；最小修复；运行目标与相邻回归确认 GREEN；一个逻辑问题一个提交。

- [x] **Step 4: 零生产接线证明**

Run:

```bash
git diff --name-only cdea840 -- src/bin/monitor src/notification config Cargo.toml migrations
```

Expected: no output.

### Task 7: Fresh 验证与中文结果文档

**Files:**
- Create: `docs/push-system/implementation-w13-results-2026-09-07.md`
- Modify: W13 spec/plan status lines.
- Modify: `.planning/2026-09-06-push-foundation-runtime/{task_plan,findings,progress}.md` (ignored working memory only).

- [x] **Step 1: 定向格式和静态检查**

只对 W13 目标 Rust 文件运行 `rustfmt --edition 2021 <explicit files>`；随后 `git diff --check`、目标 diff panic scan、生产 wiring diff。

- [x] **Step 2: Fresh tests**

Run serially:

```bash
cargo test --lib w13_ -- --test-threads=1
cargo test --lib push_foundation:: -- --test-threads=1
cargo test --lib durable_delivery::tests:: -- --test-threads=1
cargo test --lib event:: -- --test-threads=1
cargo test --lib monitor::push_job::tests:: -- --test-threads=1
cargo test --doc
cargo check --lib
cargo clippy --lib
```

strict Clippy 另跑 `cargo clippy --lib -- -D warnings`；若仍为目标外历史基线，记录首错、总数和 W13 文件命中数，不伪装 PASS。

- [x] **Step 3: 文档门禁**

Run:

```bash
ruby scripts/architecture-docs/check-rfc-inputs.rb --root .
ruby scripts/architecture-docs/check-sources.rb --root .
ruby scripts/architecture-docs/check-rfc.rb --root . --draft
ruby scripts/architecture-docs/render-wbs.rb --root . --check
```

另运行五组 architecture docs tests。catalog current 若仍因 W07--W13 源码 manifest 过期失败，精确记录 NOT CURRENT，W13 不擅自 re-freeze 全目录。

- [x] **Step 4: 写中文结果**

结果文档必须包含：验收矩阵、P01/N02 authority 链、mode/quota 分域证明、exact evidence、状态/失败矩阵、TDD commits、fresh 命令/计数、production zero-wiring、catalog 当前性、实际收益和 W14--W21/Unit 剩余边界。

- [x] **Step 5: 提交并复验**

```bash
git add -f docs/superpowers/specs/2026-09-07-push-foundation-w13-dedicated-conformance-adapters-design.md docs/superpowers/plans/2026-09-07-push-foundation-w13-dedicated-conformance-adapters.md docs/push-system/implementation-w13-results-2026-09-07.md
git commit -m "docs: record W13 implementation evidence"
cargo test --lib w13_ -- --test-threads=1
git status --short
```

Expected: W13 target PASS and worktree clean; next active package is W14, not production cutover.
