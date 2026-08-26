# Monitor Operational Recovery Design

Status: Gate A candidate. This document covers three independently reversible slices:
BR-211 legacy paper-exit containment, BR-212 explicit review completion, and BR-213
evidence-preserving upper-limit projection. Gate B starts only after the matching
business-rule rows exist.

## Problem and current-code evidence

The production monitor starts, but three internal contracts prevent timely and truthful
operation.

1. `paper_engine::load_open_positions` rebuilds the complete FIFO ledger and then calls
   `broker::execution_quote` once per open code. Each call creates a new Magic router and runs
   the ordered TDX -> Tencent -> Sina route. The current pinned TDX quote batch lacks a trusted
   provider source time, so it cannot pass the five-second contract and every symbol waits for
   the same failed primary attempt before fallback.
2. `run_review_only` treats any one delivered task as complete. A run where only R-04 is
   delivered therefore prints the same completion banner as a run with no failed runnable task.
3. `market_analyzer::limit_up` is a constant failure even though the unified gateway already
   admits an exact-date upper-limit pool. The old `TopStock` contract incorrectly requires the
   limit-pool batch itself to contain quote names, volume ratio, and main-net flow.

Reproducible inspection commands against the working tree (captured 2026-08-04):

```text
$ nl -ba src/trading/paper_engine.rs | sed -n '180,205p'
   185        let quote = match crate::broker::execution_quote(&code) {
   197        positions.push(PaperPositionSellCheck {
   202            current_price: quote.price,
   203            limit_up_price: quote.limit_up_price,
   204            limit_down_price: quote.limit_down_price,

$ nl -ba src/broker.rs | sed -n '67,94p'
    67    fn get_execution_quote(&self, code: &str) -> Result<ExecutionQuote, String> {
    69        let batch = crate::data_gateway::MarketDataGateway::new()
    70            .realtime_quotes(&requested)
    77        let limit = crate::data_provider::limit_status::LimitStatusCalculator::new().calculate(
    89        Ok(ExecutionQuote {

$ nl -ba src/bin/monitor/review_batch.rs | sed -n '844,853p'
   844    pub fn delivered_count(&self) -> usize {
   851    pub fn has_confirmed_delivery(&self) -> bool {
   852        self.delivered_count() > 0

$ nl -ba src/bin/monitor/main.rs | sed -n '4226,4238p'
  4226            if batch.has_confirmed_delivery() {
  4228                    "[复盘] ======== 盘后分析完成 ({}s) ========",
  4238                Err("[BR-140] 严格盘后复盘没有任何确认投递；逐任务状态已写审计".to_string())

$ nl -ba src/market_analyzer/limit_up.rs | sed -n '15,18p'
    15    pub(super) fn get_limit_up_from_gateway(&self) -> Result<Vec<crate::market_data::TopStock>> {
    16        anyhow::bail!(

$ nl -ba src/data_gateway/review.rs | sed -n '247,250p;310,313p'
   247    pub async fn r03_upper_limit_pool(
   310 pub(crate) fn route_exact_date_upper_limit_pool(

$ nl -ba src/data_gateway/market_data.rs | sed -n '755,767p'
   756    fn br164_tdx_without_complete_evidence_cannot_win_quote_route() {
   763        assert_eq!(evidence.source_at(), None);
   767        assert!(policy.require_source_at());
```

## Slice A — BR-211 legacy paper-exit containment and authorized batch boundary

### Data flow

```text
monitor tick
  -> BR-201 exact trading-session permit and committed Admission
  -> deterministic open-position states
  -> sorted unique open codes
  -> one admitted realtime-quote batch
  -> one source-backed BR-205 DailyPriceLimitState batch for the same codes/session
  -> exact identity/evidence/freshness join
  -> terminal five-second consumption-time validation
  -> paper-exit decision and auditable order attempt
```

The first two arrows are mandatory authority boundaries, not performance optimizations. No
production quote provider, account provider, ledger mutation, proposal, reservation, order, outbox
or sink may run until BR-201 has admitted the exact tick. BR-211 grants neither session nor order
authority and does not supersede BR-201 or BR-205.

The current public `paper_engine::run_once(PaperRiskContext)` path has neither boundary and its
`ExecutionQuote` manufactures daily bounds through `LimitStatusCalculator`. It is therefore not a
production execution path. During this operational-recovery slice the monitor removes that legacy
call from its recurring loop and emits one startup warning that the four-rule paper exit is
fail-closed. The compatibility function remains callable only by isolated tests and returns a
stable zero-I/O unavailable error in production. This stops the N+1 route storm without claiming
that paper exits are restored.

The eventual guarded owner must acquire one exact-set admitted quote batch and one exact-set
source-backed `DailyPriceLimitState` batch after BR-201 Admission. It must never construct
`limit_down_price` or `limit_up_price` from code, board, name, previous close or a fixed percentage.
Until that guarded owner and source contract are complete, production paper-exit execution remains
explicitly unavailable. TDX historical bars and other independently supported capabilities are
unchanged.

### Failure modes

- Closed/auction/non-trading session or missing BR-201 Admission: zero provider/order calls.
- Legacy `run_once`: stable fail-closed error and zero provider/order calls.
- Missing source-backed BR-205 bounds: `Unavailable`; no inferred range and no order.
- Future guarded batch empty/duplicate/partial/code-set mismatch: reject before decision/order.
- Quote older than five seconds at final consumption: reject the complete future batch.
- After close: no four-rule provider call; no cost-price or daily-close fallback.

## Slice B — BR-212 explicit review completion

`ReviewBatchOutcome::completion()` returns a closed enum:

- `Complete`: at least one confirmed delivery and no `Failed`, `ExpectedWait`, or
  `DeferredUntil` task.
- `Partial`: at least one confirmed delivery and at least one failed/waiting/deferred task.
- `NoDelivery`: no confirmed delivery.

`NoData` and `Disabled` remain explicit terminal task states and do not by themselves turn a
delivered run into `Partial`. `run_review_only` prints a distinct partial banner with the exact task
lists. Partial-with-delivery keeps exit zero so an operator receives the valid independent reports;
it must never print the full-completion banner. No-delivery retains the existing strict non-zero
behavior. Task audit and sink authority are unchanged.

This slice does not activate R-03 ahead of BR-203/BR-204, does not pretend R-08 is supported, and
does not turn missing account evidence into public-data evidence.

## Slice C — BR-213 upper-limit projection

The exact-date upper-limit batch remains the authority for membership, code, trading date, price,
and change. A single exact-code realtime quote batch supplies only the missing display name. The
composition retains both immutable `BatchEvidence` values through a typed
`LimitUpStockBatch` until the projection boundary, checks an exact code-set join, and emits one
structured audit line containing both batch identities. Before a plain `Vec<TopStock>` may escape,
it also appends a BR-159 tamper-resistant composition row whose `source` retains a versioned,
canonical JSON document for both complete batch identities, the exact trading date, and the record
count; `request_hash` commits to those canonical bytes. Audit failure rejects the projection.

The result is a closed enum, so verified-empty never pretends to have quote evidence:

```rust
pub enum LimitUpStockBatch {
    Available {
        stocks: Vec<TopStock>,
        limit_pool_evidence: BatchEvidence,
        quote_evidence: BatchEvidence,
    },
    VerifiedEmpty {
        limit_pool_evidence: BatchEvidence,
    },
}
```

`volume_ratio` and `main_net_yi` remain `None`; this slice does not fill them across batches.
Consumers that require either field continue to skip that signal explicitly, while upper-limit
membership and board display remain usable. A verified-empty limit pool returns a typed empty
batch without a quote request. Unavailable/partial/conflicting data remains an error.

This is a narrow evidence-preserving exception to BR-164's blanket cross-batch prohibition: only a
source-backed display name may be joined, both batches stay explicit, and neither batch may replace
the other's authoritative fields. Future volume-ratio and fund-flow signals require their own typed
composition rather than mutation of this projection.

## Slice D — BR-196 test-notification isolation correction

The template acceptance transport must resolve its target only from the three exact
`BR196_FEISHU_TENANT_ID`, `BR196_FEISHU_APP_ID`, and
`BR196_FEISHU_CONVERSATION_ID` fields. Missing fields are a typed failure before MagicLaw binary,
home, repository `.env`, target allowlist, process, or network work. The former implicit fallback to
the default MagicLaw/repository configuration is rejected because it can select a production
target while the caller believes it is running an isolated acceptance test.

Immediately before every allowed MagicLaw spawn, the resolved MagicLaw home is mandatory and its
configured `FEISHU_ACCOUNT_ID` and `FEISHU_APP_ID` must exactly match the invocation-bound target
authority. The conversation remains an explicit CLI argument. A missing home, unreadable
configuration, or identity mismatch fails before spawn and is recorded as zero external process
attempts. Raw identifiers and credentials remain absent from logs and audit.

Startup health webhooks are notification sinks. In `TradingEnv::Test` they are disabled before URL
resolution or HTTP client use, even when `ALERT_WEBHOOK_URL` is inherited from the operator shell.
Production keeps the existing explicit `Disabled` / `Delivered` / error semantics. This makes the
README claim that `--test --push-dry-run` performs no external notification true at the process
boundary; it does not claim that production review or normal monitor avoids real public-data I/O.

### Slice-D failure modes

- Missing any exact BR-196 target field: fail closed before default configuration lookup.
- Allowlisted target but mismatched MagicLaw account/app: fail closed before process spawn.
- Test environment with a configured health webhook: return `Disabled`; zero HTTP request.
- Production health webhook failure: preserve the current explicit transport/HTTP error.

## Slice E — BR-214 daily-review idempotency boundary

### Problem

`ReviewMarket` / `ReviewLhb` / `ReviewSignal` / `ReviewFailure` used `WindowMode::Rolling` with an
86400s window. Rolling anchors `cooldown_heads.blocked_until` at "last delivery instant + 86400s",
so the block drifts later every day. On 2026-08-04 the 21:07 review ran 30 minutes before the
21:37:35 block expired and was rejected, and because every pre-sink rejection is frozen with
`retry_authorized=false`, R-04 stayed terminally rejected for the whole business date.

### Data flow

The idempotency boundary for a daily review is the business date, not a rolling 24h window, so the
four `Review*` policies move to `WindowMode::BusinessDateOnce`. Because `policy_version` is part of
`DecisionIdentityMaterial`, `POLICY_VERSION` moves 1 → 2, and because
`seed_and_verify_policy_catalog` compares every persisted row against the compiled catalog and
aborts startup on `PolicyMismatch`, `SCHEMA_VERSION` moves 5 → 6 with `migrate_schema_v5_to_v6`
deleting `delivery_policy_catalog` so the new catalog is re-seeded.

`inspect_review_task_occurrence` additionally ignores non-delivered decisions whose envelope
`policy_version` differs from the compiled catalog, so a rejection frozen under a retired policy
cannot short-circuit the current business date. `Delivered` decisions are never ignored: a delivery
is an accomplished fact and replaying it would push twice.

### Failure modes

- Retired-policy rejection reused as current evidence → occurrence check filters it out.
- Delivered decision ignored after a policy bump → explicitly excluded; delivery stays authoritative.
- Persisted catalog disagreeing with the compiled catalog → `PolicyMismatch` aborts startup rather
  than silently delivering under stale policy.
- `business_date_once_claims` held by a different decision identity → `PolicyMismatch`, not a
  silent second push.

BR-214 is registered in `docs/business_rules.md`.

## Old-module disposition

| Module | Disposition | Reason |
|---|---|---|
| `broker::execution_quote` | reject as production order authority | It derives price bounds locally and lacks BR-201/BR-205 authority. |
| Legacy recurring `paper_engine::run_once` call | reject/remove | It performs provider/order work before BR-201 Admission and creates N+1 routes. |
| `paper_engine::run_once` compatibility symbol | retain as fail-closed zero-I/O shim | Isolated callers compile but cannot bypass the guarded owner. |
| `MarketDataGateway` Magic Router | adopt | It already owns provider ordering, completeness, evidence, and freshness. |
| `market_analyzer::limit_up` constant bail | remove | A supported upper-limit capability already exists. |
| Legacy `TopStock` auxiliary defaults | retain as explicit `Option::None` only | Missing ratio/flow must not become zero. |
| R-03 `LegacyAccountGate` | unchanged | BR-204 remains separately gated. |
| BR-196 non-production transport | correct exact-target and test-webhook isolation | Remove default target fallback, bind MagicLaw credentials to the permit, and disable test health webhook before network work. |

## Validation and acceptance

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
./target/release/monitor --review
./target/release/monitor --test --push-dry-run
./target/release/monitor
```

Expected live evidence includes zero legacy paper-exit quote/order calls and one containment banner,
no per-code provider route storm, a truthful complete/partial review banner, an admitted
upper-limit projection or an explicit provider error, and durable delivery/audit evidence for any
real push. Restoring production paper exits is not an acceptance claim for this slice.

## Rollback

Each slice is committed independently and rolled back with `git revert <slice-commit>`. Rollback
does not delete databases, market evidence, push logs, audit chains, account snapshots, or paper
ledger rows. If a data-flow invariant fails, return to Gate A and revert the corresponding slice.
