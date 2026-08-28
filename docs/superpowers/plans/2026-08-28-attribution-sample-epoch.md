# Attribution Sample Epoch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在不删除、不改写旧成交且不改变模拟交易仓位的前提下，用不可变高水位纪元重置归因样本，并由 monitor 在完整交易日收盘安全窗口自动、幂等地启用一次。

**Architecture:** 新增纯函数纪元域模块负责 carry 构建和“隔离到首次归零”筛选，新增 SQLite 深模块负责不可变纪元、尝试审计、日归因和报告绑定。CLI、历史 replay 与 monitor 都只能通过该共享证据边界选择 active/legacy/exact epoch；既有 BR-248 FIFO/T+1/费用/benchmark 引擎保持不变，只接收已完成纪元筛选的完整成交行。

**Tech Stack:** Rust 2021, Diesel + SQLite, Clap, Chrono, Serde/serde_json, SHA-256, Tokio, cargo fmt/clippy/test, cargo-llvm-cov, repository compliance scripts.

---

## 实施依据与不可变约束

- 设计：`docs/superpowers/specs/2026-08-28-attribution-sample-epoch-design.md`
- 业务规则：`docs/business_rules.md` 中 BR-255；继续满足 BR-248、BR-251、BR-252。
- 数据红线：2.1、2.2、2.3、2.4、2.5、2.7、2.8、2.10。
- 旧 `paper_trades`、`order_audit`、`order_audit_chain` 和历史归因报告只读；不得执行 `DELETE`、`UPDATE`、时间缩放、数量缩放或费用摊分。
- 首次成功纪元是唯一 active epoch。成功后 monitor 只验证，不创建第二个纪元。
- 有 legacy carry 的代码从边界起隔离所有完整买卖行，直到总数量首次精确归零；terminal sell 仍排除，下一条完整成交才可进入新样本。
- 任一成功纪元尾部、链、trigger、源前缀、高水位或 binding 损坏时整次失败；禁止回退到更早成功行。
- 归因失败不阻断行情与模拟交易循环，但归因输出必须为 typed `Unavailable`/`FailedIntegrity` 并追加失败审计。

## 文件结构映射

### 新增文件

- `src/performance/attribution_epoch.rs`：纯类型、legacy carry 构建、边界后 quarantine 状态机、排除摘要和 canonical manifest。
- `src/database/attribution_epochs.rs`：DDL、canonical trigger、全链验证、高水位纪元事务、尝试审计、共享 epoch-scoped 成交读取、append-only 日归因。
- `src/bin/monitor/attribution_epoch_runtime.rs`：15:35–15:50 薄调度适配器；不解析任意 DB path，不启动 CLI。
- `tests/attribution_epoch_integration.rs`：跨数据库边界和并发/篡改/迟到数据集成测试。

### 修改文件

- `src/performance/mod.rs`：导出 `attribution_epoch`。
- `src/database/mod.rs`：导出并在正常数据库初始化中安装/验证纪元 schema。
- `src/calendar.rs`：添加 fail-closed 的下一 verified A 股交易日解析。
- `src/performance/attribution_replay.rs`：加入 epoch selector、证据 seal、共享筛选、排除摘要与失败摘要。
- `src/database/attribution_reports.rs`：报告提交事务中写入不可变 epoch binding 及其链。
- `src/performance/attribution.rs`：monitor 日归因改读 active epoch，共享筛选并改为 append-only epoch-bound persistence。
- `src/bin/strategy_attribution.rs`：新增 `reset-sample`；scheduled/replay/quarter 默认 active，并支持显式 legacy/exact。
- `src/bin/monitor/main.rs`：注册 runtime 模块，在既有循环中加入一个安全窗口调用；现有交易路径不变。
- `docs/business_rules.md`：实现完成后将 BR-255 状态从 spec-only 更新为实际 Gate 状态，并列全 active paths。

## 统一公共类型

实现全过程使用以下名称，避免 CLI、数据库与 replay 出现语义相同但类型不同的 selector 或 receipt：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttributionEpochSelector {
    Active,
    Legacy,
    Exact(String),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LegacyCarryPosition {
    pub code: String,
    pub quantity: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum EpochExclusionReason {
    LegacyCarryOverlap,
    MixedLegacyCarryExit,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct EpochExclusion {
    pub fill_id: i64,
    pub code: String,
    pub direction: String,
    pub quantity: u64,
    pub reason: EpochExclusionReason,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EpochScopedFills {
    pub attributable: Vec<EconomicFillRow>,
    pub exclusions: Vec<EpochExclusion>,
    pub remaining_quarantine: Vec<LegacyCarryPosition>,
    pub released_codes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpochActivationSource {
    Monitor,
    Cli,
}
```

`AttributionEpochSelector` 的 exact 值只接受 64 位小写十六进制 SHA-256。`Legacy` 是显式历史诊断，绝不能成为 active 缺失时的 fallback。

## Task 1：补齐下一 verified trading day authority

**Files:**

- Modify: `src/calendar.rs`
- Test: `src/calendar.rs` 内现有 `#[cfg(test)] mod tests`

- [ ] **Step 1：先写跨周末、休市日和 coverage 边界失败测试**

```rust
#[test]
fn verified_next_a_share_trading_day_skips_weekend_and_closure() {
    assert_eq!(
        verified_next_a_share_trading_day(
            NaiveDate::from_ymd_opt(2026, 9, 30).expect("TEST_CODE date")
        )
        .expect("TEST_CODE next trading day"),
        NaiveDate::from_ymd_opt(2026, 10, 8).expect("TEST_CODE date")
    );
    assert_eq!(
        verified_next_a_share_trading_day(
            NaiveDate::from_ymd_opt(2026, 8, 28).expect("TEST_CODE date")
        )
        .expect("TEST_CODE next trading day"),
        NaiveDate::from_ymd_opt(2026, 8, 31).expect("TEST_CODE date")
    );
}

#[test]
fn verified_next_a_share_trading_day_fails_outside_checked_in_coverage() {
    let error = verified_next_a_share_trading_day(
        NaiveDate::from_ymd_opt(2026, 12, 31).expect("TEST_CODE date")
    )
    .expect_err("TEST_CODE missing next-year authority must fail");
    assert!(error.contains("coverage unavailable"));
}
```

- [ ] **Step 2：运行定向测试，确认因函数不存在而失败**

Run: `cargo test --lib verified_next_a_share_trading_day -- --nocapture`

Expected: compile failure containing `cannot find function verified_next_a_share_trading_day`.

- [ ] **Step 3：实现只依赖 checked-in authority 的 checked-add 循环**

```rust
pub fn verified_next_a_share_trading_day(from: NaiveDate) -> Result<NaiveDate, String> {
    let mut candidate = from
        .checked_add_signed(chrono::Duration::days(1))
        .ok_or_else(|| "A-share trading-calendar next-date overflow".to_owned())?;
    loop {
        if verified_a_share_trading_day(candidate)? {
            return Ok(candidate);
        }
        candidate = candidate
            .checked_add_signed(chrono::Duration::days(1))
            .ok_or_else(|| "A-share trading-calendar next-date overflow".to_owned())?;
    }
}
```

- [ ] **Step 4：运行定向测试与现有 calendar 测试**

Run: `cargo test --lib calendar::tests -- --nocapture`

Expected: all `calendar::tests` pass; no runtime holiday override affects the new function.

- [ ] **Step 5：提交小步 commit**

```bash
git add src/calendar.rs
git commit -m "feat: resolve next verified trading day"
```

## Task 2：实现纯 legacy carry 与 quarantine-until-flat 状态机

**Files:**

- Create: `src/performance/attribution_epoch.rs`
- Modify: `src/performance/mod.rs`
- Test: `src/performance/attribution_epoch.rs`

- [ ] **Step 1：添加纯函数测试 fixture 和关键行为测试**

测试使用 `TEST_CODE_600001`、`TEST_CODE_600002`，覆盖：

1. 边界前 buy/sell 得到按代码排序的剩余 carry；
2. carry 为零代码的边界后完整行直接进入 attributable；
3. carry 非零代码的 buy、carry-only sell、mixed sell、terminal sell 全部排除；
4. terminal sell 后的新 buy/sell 完整进入 attributable；
5. mixed sell 同时产生 `LegacyCarryOverlap` 与 `MixedLegacyCarryExit`，但不复制/拆分 attributable fill；
6. 非正价格、坏方向、非 100 股整数、乱序、重复 ID、累计超卖显式失败；
7. legacy carry 构建不执行 T+1，筛选后的行仍由 BR-248 执行 T+1。

核心断言：

```rust
assert_eq!(scoped.attributable.iter().map(|row| row.id).collect::<Vec<_>>(), vec![7, 8]);
assert_eq!(
    scoped.exclusions.iter().map(|row| row.fill_id).collect::<Vec<_>>(),
    vec![3, 4, 5, 5, 6]
);
assert!(scoped.remaining_quarantine.is_empty());
assert_eq!(scoped.released_codes, 1);
```

- [ ] **Step 2：运行新模块测试，确认模块尚不存在**

Run: `cargo test --lib performance::attribution_epoch::tests -- --nocapture`

Expected: compile failure because `performance::attribution_epoch` is not exported.

- [ ] **Step 3：添加统一类型、canonical 排序与两个纯函数**

```rust
pub fn build_legacy_carry(
    rows: &[EconomicFillRow],
    completed_session: NaiveDate,
) -> Result<Vec<LegacyCarryPosition>, String>;

pub fn scope_epoch_fills(
    rows: &[EconomicFillRow],
    effective_date: NaiveDate,
    carry: &[LegacyCarryPosition],
) -> Result<EpochScopedFills, String>;
```

状态机规则：

```rust
match (state.quarantined, row.direction.as_str()) {
    (false, "buy" | "sell") => attributable.push(row.clone()),
    (true, "buy") => {
        state.total_quantity = state.total_quantity.checked_add(quantity)
            .ok_or("attribution_epoch_quantity_overflow")?;
        exclusions.push(overlap(row));
    }
    (true, "sell") => {
        if quantity > state.total_quantity {
            return Err("attribution_epoch_cumulative_oversell".to_owned());
        }
        let mixed = quantity > state.legacy_remaining && state.legacy_remaining > 0;
        state.legacy_remaining = state.legacy_remaining.saturating_sub(quantity);
        state.total_quantity -= quantity;
        exclusions.push(overlap(row));
        if mixed {
            exclusions.push(mixed_exit(row));
        }
        if state.total_quantity == 0 {
            state.quarantined = false;
            released_codes += 1;
        }
    }
    _ => return Err("attribution_epoch_direction_invalid".to_owned()),
}
```

实现中先统一验证 `(occurred_at, id)` 严格顺序、ID 唯一、代码非空、价格正且有限、数量正且为 100 倍数，再运行状态机。不要修改 `economic_position.rs` 的 FIFO/T+1/费用逻辑。

- [ ] **Step 4：加入 domain-separated SHA-256 manifest helper**

为 carry、exclusion 和 scoped fill ID 使用长度前缀 canonical 编码；代码按字典序、成交按 `(occurred_at,id)` 排序。hash 常量固定为版本化 domain：

```rust
const CARRY_MANIFEST_DOMAIN: &[u8] = b"BR255_ATTRIBUTION_CARRY_V1\0";
const EXCLUSION_MANIFEST_DOMAIN: &[u8] = b"BR255_ATTRIBUTION_EXCLUSION_V1\0";
const SCOPED_FILL_MANIFEST_DOMAIN: &[u8] = b"BR255_ATTRIBUTION_SCOPED_FILL_V1\0";
```

- [ ] **Step 5：运行纯函数测试和 BR-248 回归**

Run: `cargo test --lib performance::attribution_epoch::tests -- --nocapture`

Expected: all new state-machine tests pass.

Run: `cargo test --lib performance::economic_position::tests -- --nocapture`

Expected: all existing BR-248 tests pass without fixture changes.

- [ ] **Step 6：提交小步 commit**

```bash
git add src/performance/mod.rs src/performance/attribution_epoch.rs
git commit -m "feat: scope attribution fills by immutable epoch"
```

## Task 3：建立不可变 epoch、carry、attempt 与 daily schema

**Files:**

- Create: `src/database/attribution_epochs.rs`
- Modify: `src/database/mod.rs`
- Test: `src/database/attribution_epochs.rs`

- [ ] **Step 1：先写 schema/trigger/retention/sequence 失败测试**

用临时 SQLite 文件逐项验证：

- `create_schema` 安装六组表：success receipt/chain、carry item、attempt audit/chain、epoch daily/chain；
- 每张事实表使用 `INTEGER PRIMARY KEY AUTOINCREMENT`；
- success、carry、attempt、daily 都有 canonical `BEFORE UPDATE` 和 `BEFORE DELETE` trigger；
- 任意删除、更新、同名 no-op trigger 替换、chain 断裂、`sqlite_sequence` 回退、created_at 非 canonical UTC、retention 少于 60 自然月均 `FailedIntegrity`；
- 空库 schema 初始化成功，旧库只有 legacy 表时成功安装且不写 success receipt；
- 读取必须验证全部 success rows，坏尾不能返回前一个 epoch。

- [ ] **Step 2：运行定向测试，确认新 module 不存在**

Run: `cargo test --lib database::attribution_epochs::tests -- --nocapture`

Expected: compile failure because `database::attribution_epochs` is not exported.

- [ ] **Step 3：实现固定 schema 与 typed store error**

核心事实表字段固定为：

```text
attribution_sample_epoch_receipt:
  id, epoch_id, cutover_completed_trading_date, effective_trading_date,
  paper_trade_high_water, legacy_filled_manifest_hash,
  terminal_binding_manifest_hash, order_audit_high_water,
  order_audit_tip_hash, calendar_authority_hash,
  legacy_carry_manifest_hash, carry_item_count, carry_total_quantity,
  position_projection_hash, previous_epoch_receipt_hash,
  decision_basis, receipt_hash, created_at, retention_deadline

attribution_legacy_carry_item:
  id, epoch_receipt_id, code, quantity, item_index,
  predecessor_item_hash, item_hash, created_at, retention_deadline

attribution_epoch_attempt_audit:
  id, source, invoked_at, completed_session_date, effective_date,
  outcome, reason_code, retryable, source_summary_hash,
  epoch_id, success_receipt_hash, predecessor_attempt_hash,
  record_hash, created_at, retention_deadline

paper_attribution_epoch_daily:
  id, epoch_id, date, signal_family, payload_json, payload_hash,
  predecessor_daily_hash, record_hash, created_at, retention_deadline
```

成功纪元使用 companion chain 表；attempt 和 daily 虽在事实行保留 predecessor，也建立一对一 chain 表，读取时双向校验。`decision_basis` 只能为 `BR-255`。`UNIQUE(epoch_id,date,signal_family,payload_hash)` 只提供完全相同 payload 的幂等重放，不允许 `INSERT OR REPLACE`。

- [ ] **Step 4：实现全量 canonical validation 与 read API**

```rust
pub struct AttributionEpochStore<'a> {
    database: &'a DatabaseManager,
}

impl<'a> AttributionEpochStore<'a> {
    pub fn new(database: &'a DatabaseManager) -> Self;
    pub fn load_selector(
        &self,
        selector: &AttributionEpochSelector,
    ) -> Result<ResolvedAttributionEpoch, AttributionEpochStoreError>;
    pub fn verify_active(&self) -> Result<AttributionEpochReceipt, AttributionEpochStoreError>;
    pub fn append_attempt(
        &self,
        attempt: AttributionEpochAttemptAppend,
    ) -> Result<AttributionEpochAttemptReceipt, AttributionEpochStoreError>;
    pub fn append_daily(
        &self,
        input: AttributionEpochDailyAppend,
    ) -> Result<AttributionEpochDailyReceipt, AttributionEpochStoreError>;
}
```

公开 receipt 和 resolved selector 固定为：

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AttributionEpochReceipt {
    pub epoch_id: String,
    pub cutover_completed_trading_date: NaiveDate,
    pub effective_trading_date: NaiveDate,
    pub paper_trade_high_water: i64,
    pub legacy_filled_manifest_hash: String,
    pub terminal_binding_manifest_hash: String,
    pub order_audit_high_water: i64,
    pub order_audit_tip_hash: String,
    pub calendar_authority_hash: String,
    pub legacy_carry_manifest_hash: String,
    pub carry_item_count: u64,
    pub carry_total_quantity: u64,
    pub position_projection_hash: String,
    pub previous_epoch_receipt_hash: Option<String>,
    pub receipt_hash: String,
    pub created_at: String,
    pub retention_deadline: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedAttributionEpoch {
    Legacy,
    Epoch(AttributionEpochReceipt),
}
```

`Active`：必须存在且返回唯一首个 success receipt；0 行返回 `attribution_epoch_unavailable`，多行、坏尾或不一致返回 `attribution_epoch_integrity_failed`。`Legacy` 返回显式无边界对象；`Exact` 验证全链后按 epoch ID 精确查找，不查询 latest。

- [ ] **Step 5：接入两条 schema 初始化路径**

1. `DatabaseManager::init` 在 `attribution_reports::create_schema` 后调用 `attribution_epochs::create_schema`；
2. `AttributionDatabaseSession::open(AppendOnly)` 同样安装 schema；`ReadOnly` 继续只设置/验证 `PRAGMA query_only`，绝不 migrate。

- [ ] **Step 6：运行数据库模块与只读 session 回归**

Run: `cargo test --lib database::attribution_epochs::tests -- --nocapture`

Expected: all epoch schema/integrity tests pass.

Run: `cargo test --lib database::attribution_reports::tests::read_only -- --nocapture`

Expected: matching existing read-only tests pass; database bytes/WAL/SHM state remains unchanged.

- [ ] **Step 7：提交小步 commit**

```bash
git add src/database/mod.rs src/database/attribution_epochs.rs src/database/attribution_reports.rs
git commit -m "feat: persist immutable attribution epochs"
```

## Task 4：实现首次 activation 的单事务高水位凭证

**Files:**

- Modify: `src/database/attribution_epochs.rs`
- Test: `src/database/attribution_epochs.rs`
- Test: `tests/attribution_epoch_integration.rs`

- [ ] **Step 1：先写 activation preview/commit/幂等/失败审计测试**

构造含 `paper_trades`、`order_audit`、`order_audit_chain` 的 `TEST_CODE` 数据库，覆盖：

- preview 只返回将要冻结的 completed/effective 日期、全表高水位、Filled manifest 和 carry，不写库；
- preview 遇到纪元表全体不存在时把状态显示为“尚未激活”；遇到部分表存在、schema 漂移或已有坏链时完整性失败，绝不初始化 schema；
- commit 使用 completed verified trading day，并将 effective 绑定到下一 verified trading day；
- success 后源 `paper_trades`、订单审计和模拟持仓投影逐字节/逐代码不变；
- 同一时间和后续时间重试返回同一 epoch receipt，不以新增成交抬高旧 highwater；
- 两线程并发 activation 只产生一个 success receipt；
- source 结构缺失、paper ID 重复/非正、坏时间/价格/数量/方向、累积超卖、terminal binding 缺失、audit 链坏、日历不可用时无 success receipt；
- 非 +08:00、非 verified trading day、15:35 前、15:50 后和 SQLite busy 都不产生 success；busy 映射为 retryable unavailable；
- 失败在独立事务追加 attempt；attempt 写入也失败时返回 `epoch_attempt_audit_unavailable`；
- 成功提交后 read-back 验证失败则调用方看见 `FailedIntegrity`，不能宣称成功。

- [ ] **Step 2：运行 activation 测试并确认 API 尚未实现**

Run: `cargo test --lib database::attribution_epochs::tests::activation -- --nocapture`

Expected: compile failure for missing activation API.

- [ ] **Step 3：实现 preview 与 commit 输入输出**

```rust
pub struct EpochActivationRequest {
    pub source: EpochActivationSource,
    pub invoked_at: DateTime<FixedOffset>,
}

pub struct EpochActivationPreview {
    pub epoch_id: String,
    pub completed_session_date: NaiveDate,
    pub effective_date: NaiveDate,
    pub paper_trade_high_water: i64,
    pub order_audit_high_water: i64,
    pub carry: Vec<LegacyCarryPosition>,
    pub legacy_filled_manifest_hash: String,
    pub terminal_binding_manifest_hash: String,
    pub order_audit_tip_hash: String,
    pub position_projection_hash: String,
}

impl AttributionEpochStore<'_> {
    pub fn preview_activation(
        &self,
        request: &EpochActivationRequest,
    ) -> Result<EpochActivationPreview, AttributionEpochStoreError>;

    pub fn activate_once(
        &self,
        request: EpochActivationRequest,
    ) -> Result<AttributionEpochReceipt, AttributionEpochStoreError>;
}
```

- [ ] **Step 4：在 `BEGIN IMMEDIATE` 内实现完整冻结顺序**

事务内固定顺序：

1. `validate_schema_and_all_chains`；
2. 若 success 已存在，验证被冻结的源前缀、binding、audit tip、carry 和 receipt，追加本次幂等成功 attempt 后返回原 receipt；
3. 验证 `invoked_at` 为 +08:00、日期是 verified trading day、时间位于 15:35–15:50；由 checked-in authority 在 service 内解析 completed date、verified next effective date 和 calendar authority hash，caller 不能提供或覆盖这三个值；
4. 从全部 `paper_trades` 计算 `paper_trade_high_water=MAX(id)`；另读取该高水位内全部 `status='Filled'` row，按 `(occurred_at,id)` 验证并计算 `legacy_filled_manifest_hash`；
5. 全量验证 `order_audit` 与 chain 后取整个审计表的 `order_audit_high_water` 和对应 canonical tip；再对每个 frozen Filled row 验证唯一 terminal binding 并计算 terminal binding manifest；
6. 调用 `build_legacy_carry`，计算 carry manifest；
7. 计算边界前逐代码数量投影 hash；
8. 插入 success receipt、chain、carry item 和绑定该 receipt hash 的 success attempt/chain；
9. 再次查询同一源投影并要求 hash 不变；
10. 提交后重新获取连接并全量 read-back 验证。

`epoch_id` 必须由所有冻结字段的 canonical hash 产生，不包含自增数据库 ID。receipt hash 绑定 previous receipt hash、epoch ID、全部 manifest、`decision_basis="BR-255"`、created_at 和 retention deadline。任何失败先回滚该事务，再由独立事务追加 failure attempt；失败 attempt 也无法可信追加时返回 `epoch_attempt_audit_unavailable`。

- [ ] **Step 5：实现 source prefix drift 与迟到越界验证 helper**

```rust
pub(crate) fn verify_epoch_source_prefix(
    conn: &mut SqliteConnection,
    epoch: &AttributionEpochReceipt,
) -> Result<(), AttributionEpochStoreError>;

pub(crate) fn load_verified_epoch_fills_until(
    conn: &mut SqliteConnection,
    epoch: &ResolvedAttributionEpoch,
    to: NaiveDate,
) -> Result<VerifiedEpochFillSet, AttributionEpochStoreError>;
```

active/exact 的每条候选成交必须同时满足 `paper id > paper_trade_high_water`、`terminal_audit_id > order_audit_high_water`、成交日期 `>= effective_date`。高水位后出现日期早于 effective 的迟到成交必须 `FailedIntegrity`；不允许静默过滤。读取时还必须重算并核对高水位内 Filled manifest，而不是只核对 `MAX(id)`。

- [ ] **Step 6：运行单元与并发集成测试**

Run: `cargo test --lib database::attribution_epochs::tests -- --nocapture`

Expected: all schema, activation, drift and audit tests pass.

Run: `cargo test --test attribution_epoch_integration activation -- --nocapture --test-threads=1`

Expected: one immutable success receipt under concurrent attempts; original ledger projection remains identical.

- [ ] **Step 7：提交小步 commit**

```bash
git add src/database/attribution_epochs.rs tests/attribution_epoch_integration.rs
git commit -m "feat: activate attribution epoch atomically"
```

## Task 5：把 epoch selector 与 quarantine 绑定进 replay evidence seal

**Files:**

- Modify: `src/performance/attribution_replay.rs`
- Modify: `src/performance/attribution_epoch.rs`
- Test: `src/performance/attribution_replay.rs`

- [ ] **Step 1：先写 selector、active 缺失、legacy 显式和 seal 换绑测试**

覆盖：

- `ReplayRequest { epoch: Active }` 在纪元缺失时返回 `attribution_epoch_unavailable`，不读 legacy；
- `Legacy` 保持旧 replay 的真实失败，包括旧 T+1 缺陷；
- `Exact(hash)` 只读指定 receipt，未知 hash 返回 typed unavailable；
- active range 早于 effective date 返回 `attribution_epoch_range_before_effective`，不裁剪请求；
- scoped fills、exclusions、carry manifest、epoch ID 任一换绑都会导致 capability seal 验证失败；
- 有 carry 的代码在归零前完整排除，归零后的 flat-to-flat cycle 正常进入 BR-248；
- fee ledger 对排除 fill 可保留原证据但不进入计算；attributable fill 的费用仍一对一完整验证。

- [ ] **Step 2：运行 runner 定向测试，确认新增字段造成预期编译失败**

Run: `cargo test --lib performance::attribution_replay::tests::epoch -- --nocapture`

Expected: compile failure for missing `ReplayRequest::epoch` and epoch evidence fields.

- [ ] **Step 3：扩展 request、admission、failure summary 与 stage**

```rust
pub struct ReplayRequest {
    pub mode: ReplayMode,
    pub epoch: AttributionEpochSelector,
    pub benchmark_day_manifests: Vec<BenchmarkDayManifest>,
}
```

`AdmittedReplayRequest` 保存 selector。`FailureEvidenceSummary` 增加 selector、resolved epoch ID、epoch receipt hash、carry manifest hash、exclusion manifest hash；`source_summary_hash` 必须绑定这些字段。`ReplayStage` 增加 `Epoch` 并在 CLI 映射成 `epoch`。

- [ ] **Step 4：把数据库共享读取放在 trade evidence 之前并签入 seal**

加载顺序：resolve calendar → resolve/verify epoch → 校验 range/effective → `load_verified_epoch_fills_until` → `scope_epoch_fills` → 校验 scoped fee mapping → stock close/benchmark → BR-248 compute。

`AttributionReplayEvidence::issued` 增加：

```rust
epoch_id: Option<String>,
epoch_receipt_hash: Option<String>,
legacy_carry_manifest_hash: Option<String>,
exclusions: Vec<EpochExclusion>,
exclusion_manifest_hash: Option<String>,
remaining_quarantine: Vec<LegacyCarryPosition>,
released_codes: usize,
```

上述字段和 attributable `ReplayFillEvidence` 全部进入 `replay_capability_seal`。`trade_manifest_hash` 绑定 epoch receipt 与 scoped fill manifest，避免同一成交集合跨 epoch 重用。

- [ ] **Step 5：扩展 computation report 与 canonical result payload**

`AttributionComputationReport` 和 `PreparedAttributionReport` 暴露只读 accessor：epoch ID、effective date、remaining quarantine 代码/股数、overlap buy/sell 数量、mixed exit 数、released code 数、excluded fill 数。排除项不进入 source fill IDs、胜率、收益、费用或 200/84 样本门分母。

结果 JSON 固定加入：

```rust
"epoch": {
    "selector": selector_value,
    "epoch_id": resolved_epoch_id,
    "receipt_hash": epoch_receipt_hash,
    "effective_date": effective_date,
    "legacy_carry_manifest_hash": legacy_carry_manifest_hash,
    "exclusion_manifest_hash": exclusion_manifest_hash,
    "remaining_quarantine": remaining_quarantine,
    "released_codes": released_codes,
    "excluded_fills": exclusions,
}
```

- [ ] **Step 6：运行 replay 与 BR-248 回归**

Run: `cargo test --lib performance::attribution_replay::tests -- --nocapture`

Expected: existing legacy tests pass after explicitly setting `epoch: Legacy`; all new epoch tests pass.

Run: `cargo test --lib performance::economic_position::tests -- --nocapture`

Expected: all BR-248 tests remain green.

- [ ] **Step 7：提交小步 commit**

```bash
git add src/performance/attribution_epoch.rs src/performance/attribution_replay.rs
git commit -m "feat: bind replay evidence to attribution epoch"
```

## Task 6：在报告提交事务中追加 epoch binding

**Files:**

- Modify: `src/database/attribution_reports.rs`
- Modify: `src/performance/attribution_replay.rs`
- Test: `src/database/attribution_reports.rs`
- Test: `src/performance/attribution_replay.rs`

- [ ] **Step 1：先写 binding append/reuse/tamper 测试**

覆盖：

- active/exact 新报告没有 epoch binding 时拒绝 commit；
- legacy 报告显式写入 `Legacy` binding，不把旧历史报告改造成新纪元；
- report revision、epoch ID、receipt hash、effective date、exclusion manifest 一一绑定；
- 相同 report identity + 相同 binding 幂等返回原 revision；
- 相同 report identity + 不同 binding 返回 `FailedIntegrity`；
- binding/chain update/delete、坏尾、trigger 漂移、retention 缩短全部失败；
- 报告主表写入成功而 binding 写入失败时整个 immediate transaction 回滚。

- [ ] **Step 2：运行报告定向测试并确认 binding 尚未实现**

Run: `cargo test --lib database::attribution_reports::tests::epoch_binding -- --nocapture`

Expected: compile failure for missing epoch binding types.

- [ ] **Step 3：新增 companion tables 与 typed append input**

```rust
pub enum AttributionReportEpochBinding {
    Legacy,
    Epoch {
        epoch_id: String,
        epoch_receipt_hash: String,
        effective_date: NaiveDate,
        legacy_carry_manifest_hash: String,
        exclusion_manifest_hash: String,
    },
}

pub struct AttributionReportAppend {
    // existing fields remain
    pub epoch: AttributionReportEpochBinding,
}
```

新增 `attribution_report_epoch_binding` 与 `attribution_report_epoch_binding_chain`。绑定记录以 report revision ID 为外键并使用 AUTOINCREMENT 自身 ID；canonical trigger、sequence、时间、60 月 retention 与完整链验证遵循现有 report store 模式。

- [ ] **Step 4：把 binding 纳入 report identity/evidence identity/series identity**

在 `prepare_report_append` 计算任何 report identity 前 canonicalize binding。commit 的 `immediate_transaction` 内按主 report → binding → binding chain → run audit 顺序写入，复用分支必须重新读取并比较全部 binding 字段。

- [ ] **Step 5：runner success/failure 都携带 epoch 状态**

`commit_with_report` 从 `PreparedAttributionReport` 构造 typed binding。prepare 失败的 `source_summary_hash` 已在 Task 5 绑定 selector/epoch 状态；报告 store 的 failure append 不虚构 success binding。

- [ ] **Step 6：运行报告 store 与 runner commit 回归**

Run: `cargo test --lib database::attribution_reports::tests -- --nocapture`

Expected: all report, failure, retention and new binding tests pass.

Run: `cargo test --lib performance::attribution_replay::tests -- --nocapture`

Expected: preview remains read-only; commit receipts resolve to exact epoch binding.

- [ ] **Step 7：提交小步 commit**

```bash
git add src/database/attribution_reports.rs src/performance/attribution_replay.rs
git commit -m "feat: bind attribution reports to sample epochs"
```

## Task 7：将 monitor 日归因迁移到 active epoch 与 append-only persistence

**Files:**

- Modify: `src/performance/attribution.rs`
- Modify: `src/database/attribution_epochs.rs`
- Test: `src/performance/attribution.rs`
- Test: `src/database/attribution_epochs.rs`

- [ ] **Step 1：先写 active-only reader 与 daily append 测试**

覆盖：

- `compute_daily`/`compute_window` 在 active epoch 缺失时返回 typed reason，不调用旧全表 `FILLS_UNTIL_SQL`；
- active epoch reader 与 replay 使用同一 highwater/effective/terminal binding/prefix drift 验证；
- carry 隔离段不进入日归因，归零后的新周期进入；
- daily 相同 payload 幂等，不同 payload 追加 revision，不覆盖旧行；
- 旧 `paper_attribution_daily` 保持不变；
- daily 链坏、trigger 漂移和源迟到越界时不生成 markdown/push payload。

- [ ] **Step 2：运行日归因测试，确认旧 SQL/replace 断言失败**

Run: `cargo test --lib performance::attribution::tests -- --nocapture`

Expected: new tests fail because production path still calls `query_fills_until` and `INSERT OR REPLACE`.

- [ ] **Step 3：把纯聚合与数据读取拆开**

保留 `fifo_match`、`aggregate_window` 等纯函数。将生产入口改为显式 manager/epoch：

```rust
pub fn compute_epoch_daily(
    database: &DatabaseManager,
    date: NaiveDate,
    prices: &HashMap<String, f64>,
) -> Result<EpochDailyAttribution, AttributionEpochRuntimeError>;

pub fn compute_epoch_window(
    database: &DatabaseManager,
    end: NaiveDate,
    days: u32,
    prices: &HashMap<String, f64>,
) -> Result<EpochWindowAttribution, AttributionEpochRuntimeError>;
```

二者调用 `AttributionEpochStore::load_selector(Active)` 和共享 verified fill reader，再将 attributable 完整行转换成 `AttributionFillRow`。删除生产调用对 `query_fills_until` 的依赖；仅测试纯 fixture 可直接传 rows。

- [ ] **Step 4：替换旧 persistence API**

```rust
pub fn persist_epoch_daily(
    database: &DatabaseManager,
    daily: &EpochDailyAttribution,
) -> Result<AttributionEpochDailyReceipt, AttributionEpochRuntimeError>;
```

该函数 canonical serialize 每个 signal-family payload，经 `AttributionEpochStore::append_daily` 一次事务写入事实和链。移除 `PERSIST_SQL` 的生产使用；不 drop、不 update 旧表。

- [ ] **Step 5：运行日归因与 DB daily 测试**

Run: `cargo test --lib performance::attribution::tests -- --nocapture`

Expected: all pure aggregation and active epoch tests pass.

Run: `cargo test --lib database::attribution_epochs::tests::daily -- --nocapture`

Expected: append/reuse/revision/tamper tests pass.

- [ ] **Step 6：提交小步 commit**

```bash
git add src/performance/attribution.rs src/database/attribution_epochs.rs
git commit -m "feat: scope monitor attribution to active epoch"
```

## Task 8：扩展归因 CLI 的 reset-sample 与 epoch selector

**Files:**

- Modify: `src/bin/strategy_attribution.rs`
- Test: `src/bin/strategy_attribution.rs`

- [ ] **Step 1：先写 Clap 与只读/commit 行为测试**

覆盖：

- 命令集合从六个增为七个，新增 `reset-sample`；
- `reset-sample --db /tmp/TEST_CODE_attribution.sqlite3 --at 2026-08-28T15:40:00+08:00` 默认 preview，数据库/WAL/SHM 字节和 mtime 不变；
- `reset-sample --commit` 使用 AppendOnly，返回 epoch ID、completed/effective 日期、高水位、carry manifest、receipt hash；
- `--at` 必须是 +08:00 且处于 verified trading day 15:35–15:50；盘前、盘中、周末、节假日显式 unavailable；
- scheduled/replay/quarter 不传 `--epoch` 时解析为 active；
- `--epoch legacy` 显式历史；64 位小写 hash 解析 exact；大写、短 hash、任意字符串为 usage error；
- capture/resolve/probe 不接受 epoch 参数；
- reset retry 返回原 receipt，不生成第二个 success。

- [ ] **Step 2：运行 CLI parser 测试，确认第七命令缺失**

Run: `cargo test --bin strategy_attribution parser_exposes -- --nocapture`

Expected: assertion failure because `reset-sample` is absent.

- [ ] **Step 3：实现 CLI 参数与稳定解析器**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
struct EpochSelectorArg(AttributionEpochSelector);

impl FromStr for EpochSelectorArg {
    type Err = String;
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw {
            "active" => Ok(Self(AttributionEpochSelector::Active)),
            "legacy" => Ok(Self(AttributionEpochSelector::Legacy)),
            hash if is_lowercase_sha256(hash) => {
                Ok(Self(AttributionEpochSelector::Exact(hash.to_owned())))
            }
            _ => Err("epoch 必须为 active、legacy 或64位小写sha256".to_owned()),
        }
    }
}
```

Scheduled/Replay/Quarter 增加：

```rust
#[arg(long, default_value = "active")]
epoch: EpochSelectorArg,
```

新增 command：

```rust
ResetSample {
    #[arg(long)]
    db: PathBuf,
    #[arg(long)]
    at: Option<DateTime<FixedOffset>>,
    #[arg(long, default_value_t = false)]
    commit: bool,
    #[arg(long, value_enum, default_value_t)]
    format: OutputFormat,
}
```

- [ ] **Step 4：实现 preview/commit 共享 service 调用与渲染**

CLI 只解析并验证 `--at` 的 RFC3339/+08:00 形式，然后以 `EpochActivationSource::Cli` 调用共享 service；交易日、窗口、completed/effective 和日历 hash 均由 service 再验证并生成。preview 使用 `AttributionDatabaseAccess::ReadOnly`；commit 使用 AppendOnly。JSON/Markdown 都明确显示：`数据库已写入`、epoch identity、冻结源高水位、carry 代码/股数、日历 authority、receipt identity、ResearchOnly 边界。

- [ ] **Step 5：将 selector 传入所有 replay request**

`execute_replay` 新增 `epoch: AttributionEpochSelector` 参数并填入 `ReplayRequest`。不得在 active 失败时重试 legacy。

- [ ] **Step 6：运行 CLI 全部单元测试和 help 快照检查**

Run: `cargo test --bin strategy_attribution -- --nocapture --test-threads=1`

Expected: all CLI tests pass; preview state comparison proves read-only.

Run: `cargo run --quiet --bin strategy_attribution -- reset-sample --help`

Expected: exit 0 and help lists `--db`, `--at`, `--commit`, `--format`.

- [ ] **Step 7：提交小步 commit**

```bash
git add src/bin/strategy_attribution.rs
git commit -m "feat: expose attribution epoch CLI"
```

## Task 9：在 monitor 中加入一次性安全窗口 activation

**Files:**

- Create: `src/bin/monitor/attribution_epoch_runtime.rs`
- Modify: `src/bin/monitor/main.rs`
- Test: `src/bin/monitor/attribution_epoch_runtime.rs`
- Test: `tests/monitor_help_isolation.rs`

- [ ] **Step 1：先写 runtime 决策表测试**

纯决策函数覆盖：

| 情况 | 决策 |
|---|---|
| verified trading day 15:34 | `OutsideWindow` |
| verified trading day 15:35–15:50 且无 epoch | `Activate` |
| verified trading day 15:35–15:50 且已有 epoch | `VerifyOnly` |
| verified trading day 15:51 | `OutsideWindow` |
| 周末/节假日任何时刻 | `OutsideWindow` |
| calendar unavailable | `Unavailable` |
| schema/chain/source verify failure | `FailedIntegrity` |

另写 runtime 集成测试：成功才设置当日 latch；失败时下一个 tick 可重试；既有 epoch 只验证；归因失败返回结构化日志结果，不 panic、不改交易状态。

- [ ] **Step 2：运行 monitor runtime 测试，确认模块不存在**

Run: `cargo test --bin monitor attribution_epoch_runtime -- --nocapture --test-threads=1`

Expected: compile failure because `attribution_epoch_runtime` is not registered.

- [ ] **Step 3：实现薄 runtime API**

```rust
pub enum AttributionEpochTickOutcome {
    OutsideWindow,
    Activated(AttributionEpochReceipt),
    Verified(AttributionEpochReceipt),
    Unavailable { code: String, retryable: bool },
    FailedIntegrity { code: String },
}

pub fn run_attribution_epoch_tick(
    database: &DatabaseManager,
    now: DateTime<FixedOffset>,
) -> AttributionEpochTickOutcome;
```

runtime 只调用 calendar authority 和 `AttributionEpochStore`。不得读取 `DATABASE_PATH`、`MAGICLAW_DB_PATH`、CWD 数据库，不得 spawn `strategy_attribution`。

- [ ] **Step 4：在 main 循环加入窄调用**

在 `install_mode_owned_core_database` 成功并初始化 `DatabaseManager` 后，现有循环中加入：

```rust
if now.hour() == 15 && (35..=50).contains(&now.minute()) {
    match attribution_epoch_runtime::run_attribution_epoch_tick(
        stock_analysis::database::DatabaseManager::get(),
        now.fixed_offset(),
    ) {
        AttributionEpochTickOutcome::Activated(receipt) => {
            log::info!("[attribution-epoch] activated epoch_id={}", receipt.epoch_id);
        }
        AttributionEpochTickOutcome::Verified(receipt) => {
            log::debug!("[attribution-epoch] verified epoch_id={}", receipt.epoch_id);
        }
        AttributionEpochTickOutcome::Unavailable { code, retryable } => {
            log::warn!("[attribution-epoch] unavailable code={code} retryable={retryable}");
        }
        AttributionEpochTickOutcome::FailedIntegrity { code } => {
            log::error!("[attribution-epoch] failed_integrity code={code}");
        }
        AttributionEpochTickOutcome::OutsideWindow => {}
    }
}
```

不要 `return`、`break` 或修改 paper trading latch。旧 15:05 日归因调用改用 Task 7 的 `compute_epoch_daily`、`compute_epoch_window`、`persist_epoch_daily`；active 缺失时只记录归因 unavailable，不推送虚假成功，其他 monitor 工作继续。

- [ ] **Step 5：运行 monitor 定向与隔离回归**

Run: `cargo test --bin monitor attribution_epoch_runtime -- --nocapture --test-threads=1`

Expected: decision table and latch tests pass.

Run: `cargo test --test monitor_help_isolation -- --nocapture --test-threads=1`

Expected: monitor help/test mode isolation remains green.

Run: `cargo build --bin monitor`

Expected: debug monitor build succeeds without warnings promoted by later Clippy.

- [ ] **Step 6：提交小步 commit**

```bash
git add src/bin/monitor/main.rs src/bin/monitor/attribution_epoch_runtime.rs tests/monitor_help_isolation.rs
git commit -m "feat: activate attribution epoch from monitor"
```

## Task 10：补齐端到端负例与真实旧缺陷回归 fixture

**Files:**

- Modify: `tests/attribution_epoch_integration.rs`
- Modify: `src/bin/strategy_attribution.rs`
- Modify: `src/performance/attribution_replay.rs`
- Modify: `src/database/attribution_epochs.rs`

- [ ] **Step 1：建立不含生产数据的 TEST_CODE 端到端 fixture**

fixture 分两个阶段写入：activation 前只存在 legacy 事实；receipt 成功并确认源投影不变后，测试再追加新纪元事实。完整场景固定包含：

- legacy buy id 510、same-day legacy sell id 520，证明 legacy replay 真实报 T+1 缺陷；
- 边界时 `TEST_CODE_600001` carry 300 股；
- activation 成功后追加边界后 buy 200、sell 400、terminal sell 100，整段排除并归零；
- 归零后 buy/sell 构成合法完整闭环；
- 每个 fill 有唯一 terminal audit 与完整 audit chain；
- 合法 stock close、fee evidence、verified benchmark manifest；
- 未达 200/84 门槛，结论必须仍是 `InsufficientSample` / `ResearchOnly`。

- [ ] **Step 2：写端到端 preview → commit → replay → report 断言**

流程：

1. reset preview 不写库；
2. reset commit 产生唯一 receipt，并在此刻断言源表行、内容 hash 和逐代码数量投影未变化；
3. 追加边界后测试成交/audit/close/fee/benchmark 事实，active replay 排除 legacy 和 quarantine 段，只接受归零后闭环；
4. report commit 绑定 exact epoch；
5. activation 自身不改变模拟交易源表；后续测试追加只来自 fixture writer，epoch service 从未更新或删除这些行；
6. legacy replay 明确返回旧 T+1 failure；
7. active 样本不足不借 legacy 补足。

- [ ] **Step 3：写所有 fail-closed 篡改矩阵**

每个测试使用独立临时数据库并只篡改一个目标：

- success receipt bad tail；
- carry item hash；
- attempt chain；
- source frozen prefix；
- highwater 后迟到旧日期成交；
- terminal audit ID 低于/等于 highwater；
- order audit tip/chain；
- report epoch binding；
- daily attribution chain；
- canonical trigger SQL；
- retention deadline；
- SQLite sequence。

所有 active/exact 读取必须失败；任何测试都不得获得更早 epoch 或 legacy fallback。

- [ ] **Step 4：写并发与 retry 矩阵**

两个连接同时 `activate_once`，断言一个插入、另一个在锁释放后验证并返回相同 receipt。成功后追加新交易，再次 tick 仍返回原 receipt，且 source prefix 只验证冻结范围；若新增行日期越界则返回 `FailedIntegrity`。

- [ ] **Step 5：运行端到端测试**

Run: `cargo test --test attribution_epoch_integration -- --nocapture --test-threads=1`

Expected: all activation, replay, quarantine, tamper and concurrency tests pass.

- [ ] **Step 6：运行三个改动二进制/库的集中回归**

Run: `cargo test --bin strategy_attribution -- --nocapture --test-threads=1`

Expected: all CLI tests pass.

Run: `cargo test --bin monitor attribution -- --nocapture --test-threads=1`

Expected: monitor attribution tests pass.

Run: `cargo test --lib performance::attribution_replay::tests -- --nocapture --test-threads=1`

Expected: all replay tests pass.

- [ ] **Step 7：提交小步 commit**

```bash
git add tests/attribution_epoch_integration.rs src/bin/strategy_attribution.rs src/performance/attribution_replay.rs src/database/attribution_epochs.rs
git commit -m "test: cover attribution epoch reset end to end"
```

## Task 11：更新规则状态并执行 Gate C

**Files:**

- Modify: `docs/business_rules.md`
- Verify: all changed production/test files

- [ ] **Step 1：更新 BR-255 active paths 与 Gate 状态**

将 `src/bin/monitor/attribution_epoch_runtime.rs`、`tests/attribution_epoch_integration.rs`、报告 binding 路径补入 BR-255。此时只把 Gate A/B 标为 PASS；Gate C 必须等下面全部命令有新鲜成功输出后再标 PASS。

- [ ] **Step 2：运行格式化并检查无漂移**

Run: `cargo fmt --all`

Expected: exit 0.

Run: `cargo fmt --all -- --check`

Expected: exit 0 with no diff.

- [ ] **Step 3：运行 strict Clippy**

Run: `cargo clippy --workspace --all-targets --all-features -- -D warnings`

Expected: exit 0; no warnings.

- [ ] **Step 4：运行全 workspace tests**

Run: `cargo test --workspace --all-targets --all-features -- --test-threads=1`

Expected: exit 0; zero failed tests.

- [ ] **Step 5：运行离线 PR 合规**

Run: `bash tools/compliance/check.sh --policy pr`

Expected: exit 0. Production freshness remains explicitly not claimed under offline PR policy.

- [ ] **Step 6：生成 coverage 并执行 BR-252 patch/ratchet gate**

Run: `cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json -- --test-threads=1`

Expected: exit 0 and `target/coverage/coverage.json` exists.

Run: `cargo llvm-cov report --lcov --output-path target/coverage/lcov.info`

Expected: exit 0 and `target/coverage/lcov.info` exists.

Run: `python3 tools/coverage/check_thresholds.py --policy pr --report target/coverage/coverage.json --lcov target/coverage/lcov.info --base-ref master`

Expected: exit 0; core patch coverage at least 90%, other production patch coverage at least 85%, global/core counts do not regress below audited baseline.

- [ ] **Step 7：构建 release monitor 与 CLI**

Run: `cargo build --release --bin monitor --bin strategy_attribution`

Expected: exit 0; both binaries exist under `target/release/`.

- [ ] **Step 8：复跑业务规则门禁并检查禁止项**

Run: `bash tools/compliance/lib/check_business_rules.sh`

Expected: exit 0; BR-255 is registered for every active path.

Run: `rg -n "INSERT OR REPLACE INTO paper_attribution_daily|DELETE FROM paper_trades|UPDATE paper_trades|AttributionEpochSelector::Active.*Legacy" src tests`

Expected: no production reset path deletes/updates paper facts, no old replace persistence remains active, no active-to-legacy fallback exists. A test fixture string may match only when the assertion explicitly verifies rejection.

- [ ] **Step 9：在全部证据成功后更新 BR-255 为 Gate C PASS 并提交**

```bash
git add docs/business_rules.md src tests
git commit -m "chore: record attribution epoch gate evidence"
```

- [ ] **Step 10：确认分支工作树与提交范围**

Run: `git status --short --branch`

Expected: clean worktree on `feat/attribution-sample-epoch`.

Run: `git diff --stat master...HEAD`

Expected: only the paths named in this plan plus the approved design/rule documents.

## Task 12：形成 PR 证据并按规则合并

**Files:**

- Create through PR description: no repository file required

- [ ] **Step 1：准备完整 PR 描述**

PR 描述必须逐项填写：

```markdown
### Refs
- spec: `docs/superpowers/specs/2026-08-28-attribution-sample-epoch-design.md`
- plan: `docs/superpowers/plans/2026-08-28-attribution-sample-epoch.md`

### Data-Redlines
- [2.1] 只读取真实 paper/order audit/market evidence，无 production mock fallback
- [2.2] epoch/carry/fee/calendar 缺失均 typed unavailable，不补值
- [2.3] 身份、时间、价格、数量、顺序、超卖、链与 prefix 全量校验
- [2.4] activation 只在 verified 完整交易日 15:35–15:50，effective 为下一 verified trading day
- [2.5] 所有测试证券使用 TEST_CODE，monitor 继续使用 mode-owned database
- [2.7] epoch/attempt/carry/daily/report binding append-only、hash-chained、保留至少五年
- [2.8] activate/verify/persist 均真实操作目标 SQLite 事实，不是 logging-only
- [2.10] BR-255 已登记并覆盖所有 active paths

### OldModules
| module | adopt/reject | reason |
|---|---|---|
| `performance::economic_position` | adopt | 保留 BR-248 FIFO/T+1/费用/样本门，只接收已筛选完整 fills |
| `performance::attribution_replay` | adopt | 复用 sealed evidence、benchmark 与 report pipeline，增加 epoch binding |
| `database::attribution_reports` | adopt | 复用 append-only transaction/chain/retention 模式，增加 companion binding |
| `performance::attribution::query_fills_until` | reject | 旧入口读全表且没有 epoch/audit/prefix 证据 |
| `paper_attribution_daily INSERT OR REPLACE` | reject | 会覆盖历史且不绑定 epoch |

### Threshold-Proof
- 未修改 `config/*.toml` threshold；BR-248 的 200/84 与 BR-252 coverage threshold 均未变。

### Business-Rules
- BR-255
- BR-248
- BR-251
- BR-252

### Validation
- `cargo fmt --all -- --check`: PASS
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: PASS
- `cargo test --workspace --all-targets --all-features -- --test-threads=1`: PASS
- `bash tools/compliance/check.sh --policy pr`: PASS
- `python3 tools/coverage/check_thresholds.py --policy pr --report target/coverage/coverage.json --lcov target/coverage/lcov.info --base-ref master`: PASS
- production freshness: NOT RUN under offline PR policy; Gate D not claimed

### Rollback
- Stop automatic activation by reverting the monitor integration commit through `git revert`.
- Revert the remaining implementation commits in reverse order through `git revert`.
- Keep all already-written epoch/attempt/carry/daily/report-binding rows as immutable audit facts; readers ignore reverted feature tables.
- Do not delete or update `paper_trades`, `order_audit`, epoch facts, carry facts, attempts, daily rows, bindings or chains.
```

- [ ] **Step 2：创建 PR 并等待 Gate C 全部成功**

Run: `gh pr create --base master --head feat/attribution-sample-epoch --title "feat: reset attribution samples with immutable epoch" --body-file /tmp/attribution-epoch-pr.md`

Expected: PR URL returned. The body file must contain exactly the reviewed evidence above with actual coverage counts filled from Task 11.

Run: `gh pr checks --watch`

Expected: every required Gate C check succeeds.

- [ ] **Step 3：仅在 Gate C 与 PR 字段完整后合并**

Run: `gh pr merge --merge --delete-branch=false`

Expected: PR reports merged into `master`. This step is blocked if any check, coverage requirement, evidence field or reviewer requirement is incomplete.

- [ ] **Step 4：明确 Gate D 状态**

合并不等于发布。未执行 production freshness、live-data validation、release coverage 和 auditor sign-off 时，最终状态写为：`Merged / Gate D Release Blocked`，不得声称 production deployment 或策略有效性结论已经完成。

## 实施完成时的验收清单

- [ ] 旧成交、订单审计和模拟持仓没有删除、改写或数量变化。
- [ ] 首次 epoch receipt 可重复验证；monitor 重试不创建第二个 success。
- [ ] active/exact/legacy 三种读取语义明确，active 永不 fallback legacy。
- [ ] legacy carry 代码隔离到首次归零，terminal sell 仍排除，归零后完整新周期可归因。
- [ ] 费用不拆分、不按比例猜测，排除项不进入收益和样本门分母。
- [ ] replay capability、failure summary、result payload、report revision 全部绑定 epoch evidence。
- [ ] monitor 仅在 verified trading day 15:35–15:50 activation/verify，失败不影响交易循环。
- [ ] 旧 `paper_attribution_daily` 不覆盖；新 daily 事实 append-only、hash-chained、epoch-bound。
- [ ] 所有篡改、坏尾、迟到越界和 source drift 测试 fail closed。
- [ ] Gate C 命令与 coverage 新鲜通过，PR 证据完整；Gate D 未完成前保持 Release Blocked。
