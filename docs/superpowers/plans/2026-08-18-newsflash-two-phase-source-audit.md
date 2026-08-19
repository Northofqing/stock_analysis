# NewsFlash Durable Attempt and Source Audit Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore public SourceOnly N-02 NewsFlash delivery without blind retries by persisting every sink attempt before network I/O and committing the gate only from a branded exact receipt.

**Architecture:** Extend the existing BR-091 audit hash chain with closed NewsFlash attempt/terminal and failure records. A retained-root reconciler rebuilds all same-business-date accepted and unresolved authority at startup and before every reserve; a NewsFlash-only adapter uses the existing receipt-capable Magiclaw CLI and rejects boolean transports before network I/O.

**Tech Stack:** Rust, Tokio `spawn_blocking`, Chrono, SHA-256, serde JSON, BR-091 `AuditDispatcher`, Magiclaw CLI typed receipts, existing Launch/L4/L5/L7.

**Spec:** `docs/superpowers/specs/2026-08-18-newsflash-two-phase-source-audit-design.md`

## Global Constraints

- Business rules are BR-082 and BR-244; BR-145, BR-172 and BR-245 semantics are unchanged.
- SourceOnly GlobalNews remains `Neutral/strength=0`; N-01 is disabled with `disabled=no_authoritative_strength_provider`; only N-02 is enabled.
- No production mock/default/cache/keyword upgrade, no boolean physical acceptance authority, no detached attempt and no audit deletion/rewrite.
- Provider is the closed `GlobalNewsProvider::wire_name()` and source is the registered source contract; production rejects `TEST_CODE`.
- Aggregate windows are exactly `[target,target+300s)`: +90/+91 due and +300 not due.
- Shared-tree staging/commits and Cargo windows are coordinated by the root agent; an implementer must not start Cargo without an explicit window.

---

## File map

- `docs/business_rules.md`: corrective BR-082/BR-244 authority.
- `docs/superpowers/specs/2026-08-18-newsflash-two-phase-source-audit-design.md`: Gate-A contract.
- `src/news/aggregator/raw_v2.rs`: closed provider/source identity, production TEST_CODE rejection, bounded failure evidence and opaque test capability.
- `src/event/envelope.rs`: schema-v5 attempt/terminal and schema-v6 failure canonical fields/hashes.
- `src/event/push_record.rs`: closed parsing and independent hash/join revalidation.
- `src/event/dispatcher.rs`: retained-root exact NewsFlash reconciliation over the validated BR-091 chain.
- `src/event/mod.rs`: typed attempt/failure append APIs, reconciliation snapshot and branded accepted receipt.
- `src/bin/monitor/notify.rs`: receipt-capable physical sink classification and NewsFlash transaction.
- `src/bin/monitor/news_aggregator_init.rs`: authority-aware reservation/quota/settlement.
- `src/bin/monitor/main.rs`: startup/pre-reserve reconcile, N-01 disabled banner and N-02-before-selection orchestration.
- `src/bin/monitor/v14_adapter.rs`: existing NewsFlash governance binding only; do not alter BR-145/BR-172 settlement.

### Task 1: Close provider, source, test and failure evidence

**Files:**
- Modify: `src/data_gateway/global_news.rs`
- Modify: `src/news/aggregator/raw_v2.rs`
- Test: adjacent `br244_provider_contract_*` and `br244_failure_evidence_*`

**Interfaces:**
- Produces: `GlobalNewsProvider::wire_name() -> &'static str` as the only canonical provider text.
- Produces: opaque `NewsFlashProjectionTestCapability` required by `NewsFlashProjectedEvent::test_fixture`.
- Produces: `NewsFlashSourceFailure` accessors for diagnostic, count and optional provider/batch/record identity.

- [ ] **Step 1: Add one RED provider/test-isolation behavior test**

Assert the four wire names as independent literals, reject a mismatched registered source contract, reject production `TEST_CODE`, and prove fixture construction requires a runtime-bound test capability rather than arbitrary strings.

- [ ] **Step 2: Run the focused RED when the root grants Cargo**

```bash
cargo test --lib br244_provider_contract_ -- --nocapture --test-threads=1
```

Expected: FAIL because projection still uses `Debug`, exposes an unbranded fixture and has no production TEST_CODE gate.

- [ ] **Step 3: Implement the minimal closed projection boundary**

Use `registration.provider.wire_name()` only after exact `BatchEvidence.provider` and `registration.source_contract` validation. Make the test fixture require an opaque capability minted only when `runtime_is_test_process()` and a valid `TEST_CODE` durable namespace are both true. Reject `TEST_CODE` event/batch/provider/source before production admission.

- [ ] **Step 4: Add one RED schema-v6 input behavior test, then implement it**

Extend `NewsFlashSourceFailure` with bounded diagnostic and exact optional identities. Its canonical input is:

```rust
pub struct NewsFlashFailureAuditInput {
    pub provider: Option<String>,
    pub stage: String,
    pub reason_code: String,
    pub diagnostic_code: String,
    pub diagnostic: String, // UTF-8 length <= 512 bytes
    pub retryable: bool,
    pub observed_at: DateTime<FixedOffset>,
    pub source_record_count: u32,
    pub batch_id: Option<String>,
    pub record_id: Option<String>,
}
```

Hash every field with explicit absent markers. A 513-byte diagnostic and count overflow return typed validation errors; no truncation or defaulting.

### Task 2: Persist closed attempts/terminals and mint branded receipts

**Files:**
- Modify: `src/event/envelope.rs`
- Modify: `src/event/push_record.rs`
- Modify: `src/event/mod.rs`
- Test: adjacent `br244_news_flash_attempt_*`, `br244_news_flash_terminal_*`, `br244_news_flash_failure_*`

**Interfaces:**
- Produces: `NewsFlashAttemptReceipt`, `NewsFlashAcceptedReceipt`, `NewsFlashFailureAuditReceipt` and `NewsFlashFailureAuditError`.
- Produces: schema-v5 stage `SinkAttempt|Accepted|DefinitivelyRejected|Uncertain` and schema-v6 closed failure parsing.

- [ ] **Step 1: Write one RED source-bound attempt test**

Persist `SinkAttempt` with push kind, business date, reservation/evidence/render, ordered sources, attempt ordinal/observed time/channel, `sink_attempt_identity` and `sink_attempt_sha256`. Assert append returns `NewsFlashAttemptReceipt` only after dispatcher sync; tampering push kind, order or an attempt field fails `PushRecord::try_from_authoritative`.

- [ ] **Step 2: Implement the schema-v5 attempt constructor and append**

```rust
pub fn publish_news_flash_attempt(
    input: NewsFlashAttemptAuditInput,
) -> Result<NewsFlashAttemptReceipt, NewsFlashDeliveryAuditError>;
```

The attempt envelope ID is its final join hash. `SinkAttempt` forbids all terminal remote-receipt fields.

- [ ] **Step 3: Write one RED branded Accepted test, then implement it**

Accepted requires the persisted attempt envelope ID plus a validated typed remote receipt. Store `remote_receipt_identity` and canonical `remote_receipt_sha256`; join them with push kind/reservation/evidence/render/attempt identity/hash. Return only:

```rust
pub struct NewsFlashAcceptedReceipt { /* private exact bindings */ }
```

Do not expose a constructor from `PersistedDeliveryAuditReceipt`. Provide read-only getters for every gate-verified binding.

- [ ] **Step 4: Add terminal rejection/uncertainty and schema-v6 typed failure tests**

`DefinitivelyRejected` and `Uncertain` close the exact attempt without a remote receipt. Failure records require provider/stage/reason/diagnostic_code/diagnostic/retryable/observed/count/optional batch+record identity and return `Result<NewsFlashFailureAuditReceipt, NewsFlashFailureAuditError>`.

### Task 3: Reconcile the retained BR-091 authority

**Files:**
- Modify: `src/event/dispatcher.rs`
- Modify: `src/event/mod.rs`
- Test: adjacent dispatcher/event `br244_reconcile_*`

**Interfaces:**
- Produces:

```rust
pub fn reconcile_news_flash_business_date(
    business_date: NaiveDate,
) -> Result<NewsFlashAuthoritySnapshot, NewsFlashReconcileError>;
```

- [ ] **Step 1: Write one RED restart-recovery test**

Append accepted event/window records, one open attempt, one uncertain terminal and one definitively rejected terminal. Reopen the dispatcher and assert the snapshot contains all accepted event IDs/windows, exact unresolved reservation IDs, and the next attempt ordinal for retryable closed reservations.

- [ ] **Step 2: Implement retained-root read/reconcile**

Under the existing process mutex and kernel lock, revalidate the complete hash chain, read the year JSONL from the retained root, parse only authoritative schema-v5 records, and group by exact reservation/attempt identity. Reject duplicate/conflicting/tampered terminal records. Do not create, modify or delete any audit record during reconciliation.

- [ ] **Step 3: Add same-event changed-evidence and open-attempt RED cases**

Same business date + accepted event ID remains committed even when a later candidate has a different batch/evidence/reservation. A `SinkAttempt` with no valid closing terminal and an explicit `Uncertain` both return exact unresolved authority and prohibit automatic resend.

### Task 4: Make the gate authority-aware and quota-correct

**Files:**
- Modify: `src/bin/monitor/news_aggregator_init.rs`
- Test: adjacent `br244_gate_*`

**Interfaces:**
- Consumes: `NewsFlashAuthoritySnapshot` before each reserve.
- Accepts: only `FlashSettlement::Accepted(NewsFlashAcceptedReceipt)` for commit.
- Produces: reservation v2 identity including explicit `PushKind` and next attempt ordinal.

- [ ] **Step 1: Write one RED startup/pre-reserve reconciliation test**

Apply an authority snapshot, then prove accepted event IDs/windows do not reserve, open/uncertain block only their exact reservation and definitively rejected permits a new ordinal.

- [ ] **Step 2: Implement snapshot application and reservation-v2 identity**

Rebuild process-local business-date committed/unresolved maps from the snapshot before selection. Include `PushKind::stable_template_id()` in the reservation hash. The gate must compare Accepted receipt getters for push kind, reservation, evidence, render, attempt, remote receipt and accepted audit envelope before commit.

- [ ] **Step 3: Write one RED quota matrix, then implement it**

Assert only committed entries consume daily quota; pending limits only simultaneous reservations; an unrelated uncertain identity does not reduce quota or block another window/event. Rollback retains real buffer evidence.

- [ ] **Step 4: Disable N-01 at the gate boundary**

SourceOnly strength stays zero. Remove the production critical reservation path and expose one startup status value whose exact banner is `NewsFlashCritical disabled=no_authoritative_strength_provider`. Do not add keywords or fallback strength.

### Task 5: Replace boolean delivery with typed physical outcomes

**Files:**
- Modify: `src/bin/monitor/notify.rs`
- Modify: `src/bin/monitor/v14_adapter.rs` only if an exact binding accessor is required
- Test: adjacent `br244_news_flash_sink_*` and `br244_news_flash_transaction_*`

**Interfaces:**
- Produces:

```rust
enum NewsFlashPhysicalSinkResult {
    PreAttemptRejected(TypedNewsFlashRejection),
    DefinitivelyRejected(TypedNewsFlashRejection),
    Accepted { remote_receipt: TypedReceipt },
    Uncertain(TypedNewsFlashUncertainty),
}
```

- [ ] **Step 1: Write one RED physical-outcome classifier test**

Assert missing target/spawn-before-request is `PreAttemptRejected`; request-issued nonzero, timeout, missing receipt and parse failure are `Uncertain`; success requires nonblank local and platform message IDs. No bool converts to Accepted or DefinitivelyRejected.

- [ ] **Step 2: Implement a receipt-capable NewsFlash sink adapter**

Call the existing `push_via_magiclaw_cli_receipt_blocking` through `spawn_blocking`. Reject V6/HTTP/legacy boolean transports before attempt append and before network I/O with reason `typed_receipt_transport_unavailable`. Preserve the existing BR-192 caller and types.

- [ ] **Step 3: Write one RED transaction-order test, then implement it**

Order is governance -> attempt append -> physical sink -> L7 -> terminal append. Attempt append failure performs zero sink calls. Accepted terminal append failure leaves the attempt open and returns Uncertain. Definitive rejection rolls back only when typed transport evidence and its closing terminal append both succeed.

### Task 6: Wire startup/pre-reserve reconciliation and production order

**Files:**
- Modify: `src/bin/monitor/main.rs`
- Modify: `src/bin/monitor/news_aggregator_init.rs`
- Test: source tracer and adjacent `br244_production_order_*`

**Interfaces:**
- Consumes: startup and per-tick `NewsFlashAuthoritySnapshot`.
- Produces: N-02 reconcile/reserve/attempt/dispatch/terminal/settle before BR-172 selection/LLM/candidate.

- [ ] **Step 1: Write one RED source/order tracer**

Require exact order `projection -> failure audit -> reconcile -> N02 reserve/dispatch/settle -> selection ingress -> LLM -> candidate` and one startup disabled banner before the loop.

- [ ] **Step 2: Implement startup and per-reserve reconciliation**

Fail closed before sink when the snapshot cannot be read or validated. Apply the full business-date snapshot rather than querying only the current reservation. Preserve raw admitted events for later retry; never delete historical audit.

- [ ] **Step 3: Preserve BR-172/BR-145 boundaries**

Keep NewsAI selection receipt and ordinary `deliver_and_record` semantics unchanged. Add static regression assertions that the NewsFlash adapter has no generic receipt or boolean acceptance branch.

### Task 7: Fresh verification and compliance

**Files:** verify only; do not change production process state.

- [ ] **Step 1: Focused RED/GREEN suites in the root-granted Cargo window**

```bash
cargo test --lib br244_ -- --nocapture --test-threads=1
cargo test --bin monitor br244_ -- --nocapture --test-threads=1
cargo test --bin monitor br172_ -- --nocapture --test-threads=1
```

Expected: zero failures. Record each RED before its minimal implementation and the fresh GREEN after it.

- [ ] **Step 2: Scoped formatting and static compliance**

```bash
cargo fmt --all -- --check
git diff --check
bash tools/compliance/lib/check_business_rules.sh
bash tools/compliance/lib/check_design_contradiction.sh
```

- [ ] **Step 3: Gate C/D commands controlled by the root agent**

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

No completion claim is permitted without fresh outputs and independent production evidence. N-01 requires the disabled banner; N-02 requires a real production typed remote receipt and exact schema-v5 audit, which cannot be fabricated during offline tests.

## Self-review

- Spec coverage: typed sink outcomes, pre-sink persistent attempt, full business-date reconciliation, branded receipt, v5/v6 hashes, quota semantics, provider/test isolation and N-01 disabled are each mapped to a vertical task.
- Placeholder scan: no TBD, TODO, mock production implementation or unspecified fallback remains.
- Type consistency: projection feeds reservation-v2; attempt receipt feeds the physical adapter; typed remote receipt feeds branded Accepted; the gate consumes only that branded receipt; reconciliation reads the same closed schema-v5 records.
- Old modules: BR-091 dispatcher and Magiclaw CLI typed receipt are adopted; BR-192 coordinator and boolean sinks are explicitly rejected for this scope; BR-145/BR-172 remain unchanged.
