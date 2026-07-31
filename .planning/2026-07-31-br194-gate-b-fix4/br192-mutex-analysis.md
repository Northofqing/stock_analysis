# BR-194 ↔ BR-192 Schema 互斥分析

**状态**：已完成
**日期**：2026-07-31
**目的**：在 BR-194 收口 PR 中，证明与 BR-192 v4→v5 计划之间不存在 schema / 命名 / 触发器冲突；固化合并顺序与不变量。
**引用 spec**：
- BR-194：`docs/superpowers/specs/2026-07-30-review-task-dependency-gate-design.md`
- BR-192：`docs/superpowers/specs/2026-07-30-br192-provider-free-retry-design.md`

---

## 1. 互斥结论：0 冲突

BR-192 spec §2.0/§2.2/§2.4/§4 规划 9 张 companion 表：
- `retry_authorizations`
- `retry_authorization_events`
- `retry_authorization_bindings`
- `retry_attempt_bindings`
- `retry_send_ownership`
- `retry_schedules`
- `retry_cycles`
- `retry_cycle_audit_outbox`
- `retry_cycle_failure_payloads`

**当前源码现状**：`grep -RIn "retry_" src/ migrations/` 在仓库里 **0 个存在**。BR-192 仍处于 Gate A（C0/I0/M1，Minor 1 已关闭），Gate B/C/D 待开工。

由于 BR-192 计划表在当前 HEAD 完全缺席，互斥矩阵**平凡成立**：0 vs 0 = 0。

---

## 2. 关键不变量（BR-192 实施时必须遵守）

未来 BR-192 Gate B 实施必须遵守以下 6 条不变量，否则会破坏 BR-194：

### 2.1 BR-192 不得新增 `delivery_decisions` 列

BR-192 spec §2.0 明确禁止 `ALTER TABLE delivery_decisions` 加 `current_retry_authorization_identity` 列；用 `retry_authorization_bindings` companion 表的 unique-active partial index 作为唯一授权权威。

### 2.2 BR-192 不得新增 `AUDIT_KINDS` 条目

BR-192 spec §4 把 cycle events 放在 `retry_cycle_audit_outbox.event_kind`（独立 CHECK），不动 `immutable_audit_outbox.audit_kind` 闭集。`AUDIT_KINDS` 永远保持 14 条（含 BR-194 的 `ReviewTerminalReplayStarted` / `ReviewTerminalReplayCompleted` 为条目 13/14）。

### 2.3 BR-192 必须保留这两个 authority INSERT trigger 名字

BR-192 v4→v5 在 trigger body 里追加 `sha256_hex(canonical_blob)=stored_lowercase_sha256` 字节相等 + 域分隔哈希双断言，但**保持 trigger 名字不变**：

- `validate_review_terminal_replay_attempt_audit_insert`（schema.rs:626-643）
- `validate_review_terminal_replay_completion_audit_insert`（schema.rs:645-662）

### 2.4 `sha256_hex` UDF 保持不变

`src/durable_delivery/schema.rs:46-58` 已注册 `SQLITE_UTF8 | SQLITE_DETERMINISTIC | SQLITE_INNOCUOUS`，自测探针已校 `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`。BR-192 通过 §2.3 的 trigger body 复用同一 UDF，不动 seam。

### 2.5 `delivery_decisions.retry_authorized` 布尔兼容性列保持现状

BR-192 §2.1 把它当"compatibility projection"，唯一权威在 `retry_authorization_bindings` 的 unique `Active` 行。当前列 `schema.rs:148` 是 `INTEGER NOT NULL CHECK(retry_authorized IN (0,1))`。

### 2.6 合并顺序固定

BR-194 已在 v4 baseline 里（v3→v4 migration 已带入 2 张 replay 表 + 6 trigger + 2 audit kind，见 `src/durable_delivery/schema.rs:852-905`）。BR-192 后续 Gate B 实施时，必须在同一 `v4→v5` 事务（`schema.rs:908-963` 的 `migrate_schema_v4_to_v5`）里加 9 张 retry_* 表 + 重写 2 个 BR-194 INSERT trigger。

**禁止**：中间插入 `v4→v4.1` 或 `v5→v6` 的独立 schema 跳档（违反 BR-192 spec §2.0 "must not allocate a competing schema version"）。

---

## 3. 文件级证据（关键不变量引用）

| 不变量 | 引用位置 |
|---|---|
| BR-194 replay attempts table | `src/durable_delivery/schema.rs:303-324` |
| BR-194 replay completions table | `src/durable_delivery/schema.rs:326-354` |
| BR-194 authority INSERT trigger 1 | `src/durable_delivery/schema.rs:626-643` |
| BR-194 authority INSERT trigger 2 | `src/durable_delivery/schema.rs:645-662` |
| BR-194 4 immutable triggers | `src/durable_delivery/schema.rs:664-690` |
| `AUDIT_KINDS` 14 条 | `src/durable_delivery/coordinator.rs:34-49` |
| `retry_authorized` boolean | `src/durable_delivery/schema.rs:148` |
| `sha256_hex` UDF | `src/durable_delivery/schema.rs:46-58` |
| v3→v4 迁移（含 BR-194 baseline） | `src/durable_delivery/schema.rs:852-905` |
| v4→v5 迁移（保留给 BR-192） | `src/durable_delivery/schema.rs:908-963` |

---

## 4. 重新审查触发条件

若任一项发生，须重新跑本互斥分析：

1. BR-192 spec §2.0 / §2.2 / §2.4 / §4 文本改动（即"BR-192 frozen design SHA-256"变更）
2. BR-194 spec 文本改动
3. 任何 `src/durable_delivery/schema.rs` 改动（特别是 `SCHEMA_VERSION`、`migrate_schema_v4_to_v5`、`AUDIT_KINDS`、trigger body）

---

## 5. 简明结论给 PR 描述

```
BR-192 互斥：0 重叠；后续 companion 表落地时共用两个 authority trigger 名（validate_review_terminal_replay_*_insert），
sha256_hex=stored canonical 哈希契约保留。
```