# Delivery Audit Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the existing delivery audit chain readable without rewriting history, fail closed before sink initialization, and guarantee a physically delivered message is never resent because its post-delivery audit failed.

**Architecture:** Deepen the existing `event` module instead of creating a second audit stack. `AuditDispatcher` owns chain validation/health and exposes a small preflight interface; a pure delivery-settlement module converts sink/L7/audit facts into a typed outcome that the monitor integrates later.

**Tech Stack:** Rust, serde_json, SHA-256 helpers already in `event`, fs2 locks, std filesystem/fsync, Tokio only at the later monitor integration seam.

---

## File map

- Modify: `src/event/push_record.rs` — exact legacy identity parser; authoritative v2 remains unchanged.
- Modify: `src/event/dispatcher.rs` — preflight, health, recovery canary, existing-chain tests.
- Create: `src/event/delivery_settlement.rs` — pure sink/audit settlement state machine.
- Modify: `src/event/mod.rs` — public exports and runtime dispatcher health/preflight functions.
- Do not modify in this parallel task: `src/bin/monitor/notify.rs`, `main.rs`, `push_templates.rs`, `tests/monitor_help_isolation.rs`.

### Task 1: Exact legacy double-empty compatibility

- [ ] **Step 1: Add one RED public-behavior test for accepted legacy double-empty identity**

Add to `src/event/push_record.rs` tests:

```rust
#[test]
fn br143_legacy_double_empty_identity_is_read_as_absent() {
    let mut envelope = legacy_delivery_envelope();
    envelope.entity_key = Some(String::new());
    envelope.payload["code"] = serde_json::Value::String(String::new());

    let record = PushRecord::try_from(&envelope).expect("exact legacy double-empty is readable");
    assert_eq!(record.code, None);
}
```

- [ ] **Step 2: Run the focused test and verify RED**

```bash
cargo test --lib event::push_record::tests::br143_legacy_double_empty_identity_is_read_as_absent -- --exact --test-threads=1
```

Expected: FAIL with `invalid field value: code`.

- [ ] **Step 3: Implement a private exact legacy identity parser**

Use this interface in `src/event/push_record.rs`:

```rust
fn parse_legacy_code(
    payload_code: Option<&serde_json::Value>,
    entity_key: Option<&str>,
) -> Result<Option<String>, PushRecordError> {
    match (payload_code, entity_key) {
        (Some(serde_json::Value::String(code)), Some(key)) if code.is_empty() && key.is_empty() => {
            Ok(None)
        }
        (None | Some(serde_json::Value::Null), None) => Ok(None),
        (Some(serde_json::Value::String(code)), Some(key))
            if !code.trim().is_empty() && code == key => Ok(Some(code.clone())),
        (Some(serde_json::Value::String(code)), _) if code.trim().is_empty() => {
            Err(PushRecordError::InvalidFieldValue("code".into()))
        }
        (Some(value), _) if !value.is_string() => {
            Err(PushRecordError::InvalidFieldType("code".into()))
        }
        _ => Err(PushRecordError::InvalidFieldValue(
            "payload.code does not match envelope.entity_key".into(),
        )),
    }
}
```

Call it only when `audit_schema_version.is_none()`. Keep the current v2 redaction checks in `try_from_authoritative` unchanged.

- [ ] **Step 4: Add the rejection matrix and run GREEN**

The table-driven test must reject: code empty/entity missing, code missing/entity empty, code one space/entity one space, non-empty mismatch, v2 with either identity field.

```bash
cargo test --lib event::push_record::tests:: -- --test-threads=1
```

Expected: PASS, including existing BR-130/BR-142 tests.

- [ ] **Step 5: Commit the vertical slice**

```bash
git add src/event/push_record.rs
git commit -m "fix(audit): read exact legacy empty identities"
```

### Task 2: Preserve the immutable prefix while extending with v2

- [ ] **Step 1: Add a RED fixture test around the real legacy shape**

In `src/event/dispatcher.rs`, write two legacy records with the exact line-123/124 shape, capture `fs::read(&path)` before dispatch, append one authoritative v2 envelope through `AuditDispatcher::dispatch`, and assert:

```rust
assert!(after.starts_with(&before));
assert_eq!(&after[..before.len()], before.as_slice());
assert_eq!(after.iter().filter(|byte| **byte == b'\n').count(), 3);
```

- [ ] **Step 2: Run the test to verify the parser fix is exercised through the chain**

```bash
cargo test --lib event::dispatcher::tests::br143_real_legacy_prefix_is_byte_stable_when_v2_appends -- --exact --test-threads=1
```

Expected before Task 1: FAIL on legacy code; after Task 1: PASS without production changes in dispatcher.

- [ ] **Step 3: Re-run all chain-corruption tests**

```bash
cargo test --lib event::dispatcher::tests:: -- --test-threads=1
```

Expected: tampered hash, partial tail, v2→legacy downgrade, unknown field and one-sided identity all fail closed.

- [ ] **Step 4: Commit the immutable-prefix regression**

```bash
git add src/event/dispatcher.rs
git commit -m "test(audit): preserve legacy chain bytes"
```

### Task 3: Audit preflight and explicit health

- [ ] **Step 1: Add RED tests for the state transitions**

Tests exercise only the public dispatcher interface:

```rust
assert_eq!(dispatcher.health(), AuditHealth::Unverified);
let receipt = dispatcher.preflight().expect("valid chain");
assert_eq!(receipt.previous_hash.as_deref(), Some("GENESIS"));
assert_eq!(dispatcher.health(), AuditHealth::Healthy);

write_tampered_chain(&path);
assert!(dispatcher.preflight().is_err());
assert!(matches!(dispatcher.health(), AuditHealth::Degraded { .. }));
assert!(matches!(dispatcher.dispatch(valid_v2()), DispatchResult::Failed(_)));
```

- [ ] **Step 2: Run the state test and verify RED**

```bash
cargo test --lib event::dispatcher::tests::br144_preflight_controls_audit_health -- --exact --test-threads=1
```

Expected: compile failure because the interface does not exist.

- [ ] **Step 3: Implement health and preflight behind `AuditDispatcher`**

Use these public types:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditHealth {
    Unverified,
    Healthy,
    Degraded { reason_code: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditPreflightReceipt {
    pub year: i32,
    pub previous_hash: Option<String>,
}

impl AuditDispatcher {
    pub fn health(&self) -> AuditHealth;
    pub fn preflight(&self) -> Result<AuditPreflightReceipt, String>;
    pub fn recover_with_canary(&self) -> Result<AuditPreflightReceipt, String>;
}
```

`preflight()` must acquire the same annual lock used by append, validate the complete chain/trailing boundary, create a unique sibling canary file with `create_new(true)`, write a non-sensitive fixed marker, flush and `sync_data`, then remove only that canary. It must never append a fake `push.delivery.audit` event. Any failure sets `Degraded`; normal dispatch cannot clear it.

`recover_with_canary()` is the only method allowed to replace `Degraded` with `Healthy`, and only after the same validation/canary sequence succeeds.

- [ ] **Step 4: Add filesystem failure tests and run GREEN**

Cover: valid absent chain, valid legacy chain, tampered chain, base path is a file, canary create/write failure, and recovery after restoring a valid isolated fixture.

```bash
cargo test --lib event::dispatcher::tests::br144_ -- --test-threads=1
```

Expected: PASS; original chain bytes remain unchanged.

- [ ] **Step 5: Export runtime health functions**

Add to `src/event/mod.rs`:

```rust
pub fn preflight_runtime_delivery_audit() -> Result<AuditPreflightReceipt, String> {
    runtime_delivery_audit().preflight()
}

pub fn runtime_delivery_audit_health() -> AuditHealth {
    runtime_delivery_audit().health()
}

pub fn recover_runtime_delivery_audit() -> Result<AuditPreflightReceipt, String> {
    runtime_delivery_audit().recover_with_canary()
}
```

- [ ] **Step 6: Run the event module and commit**

```bash
cargo test --lib event:: -- --test-threads=1
git add src/event/dispatcher.rs src/event/mod.rs
git commit -m "feat(audit): add fail-closed delivery preflight"
```

### Task 4: Pure physical-delivery settlement

- [ ] **Step 1: Add a RED test for sink accepted + audit failed**

Create `src/event/delivery_settlement.rs` with tests written first against this interface:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliverySettlement {
    Pushed,
    SinkError { reason_code: String },
    PhysicallyDeliveredAuditFailed { failures: Vec<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityAction {
    Commit,
    Release,
}

pub fn settle_delivery(
    sink_accepted: bool,
    l7_result: Result<(), String>,
    audit_result: Result<(), String>,
) -> DeliverySettlement;
```

Assertion:

```rust
assert!(matches!(
    settle_delivery(true, Ok(()), Err("audit_chain_invalid".into())),
    DeliverySettlement::PhysicallyDeliveredAuditFailed { .. }
));
```

- [ ] **Step 2: Run RED**

```bash
cargo test --lib event::delivery_settlement::tests::sink_acceptance_is_never_reclassified_as_sink_error -- --exact --test-threads=1
```

Expected: compile failure before module implementation/export.

- [ ] **Step 3: Implement the complete truth table**

Rules:

```text
sink=false, any post result -> SinkError
sink=true, l7=ok, audit=ok -> Pushed
sink=true, either post result=err -> PhysicallyDeliveredAuditFailed
```

Preserve both structured post-delivery failure strings when both fail. Do not log or perform side effects in this pure module.

- [ ] **Step 4: Add a settlement instruction for the caller**

Expose:

```rust
impl DeliverySettlement {
    pub fn identity_action(&self) -> IdentityAction {
        match self {
            Self::SinkError { .. } => IdentityAction::Release,
            Self::Pushed | Self::PhysicallyDeliveredAuditFailed { .. } => IdentityAction::Commit,
        }
    }
}
```

Test all truth-table rows with literal expected values.

- [ ] **Step 5: Run and commit**

```bash
cargo test --lib event::delivery_settlement::tests:: -- --test-threads=1
git add src/event/delivery_settlement.rs src/event/mod.rs
git commit -m "feat(audit): separate sink and audit outcomes"
```

## Focused completion checks

- [ ] **Step 1: Run formatting and strict library checks**

```bash
cargo fmt --all -- --check
cargo clippy --lib --all-features -- -D warnings
cargo test --lib event:: -- --test-threads=1
git diff --check
```

Expected: all exit 0.

- [ ] **Step 2: Scan changed code for silent paths**

```bash
git diff HEAD~4 -- src/event | rg 'unwrap_or_default\(|let _ = .*\.await|Err\(.+\) => \{\}'
```

Expected: no unexplained silent fallback.

- [ ] **Step 3: Hand off to the serial integrator**

Required evidence in the handoff:

```text
Upstream debt: BR-142 semantic validation poisoned the chain on exact legacy double-empty rows 123/124.
Rename impact: new DeliverySettlement is additive; PushOutcome integration remains pending.
Production evidence: no production sink was called by this parallel task; main/notify wiring is explicitly pending.
```

Status remains **In Progress** until the serial integration and Gate D evidence complete.
