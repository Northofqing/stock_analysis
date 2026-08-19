# P-01 Reachability and Safe Compensation Design

**Status:** Gate A recorded; Gate B/C/D and production acceptance pending.

**Decision date:** 2026-08-18

**Business rule:** BR-241

**Scope:** Make the resident monitor reach P-01 before the market-active wait,
and provide one exclusive P-01 compensation command after a missed window. This
document does not authorize a second monitor, test evidence, stale persisted
rows, another `PushKind`, or an unreceipted notification.

## 1. Outcome

P-01 has exactly one production owner and one delivery identity per A-share
trading day:

- `business_date` is the local trading day on which the P-01 message is due;
- `evidence_date = calendar::verified_prev_a_share_trading_day(business_date)`
  is the most recent completed trading day under the immutable checked-in
  exchange-calendar authority; unavailable coverage is an explicit failure;
- exact `LimitPools` and the `chain_daily` projection derived only from that
  batch are bound to `evidence_date`;
- the exact top-head code set is resolved through one complete
  `SecurityIdentity` batch, and each head is queried through
  `SinaInstrumentNewsGateway::instrument_news_in_range` over the inclusive
  local-date range `[evidence_date, business_date]`;
- scheduled execution is owned by a resident pre-market loop that is already
  running before the market-active wait;
- missed-window execution is owned by the terminal command
  `monitor --compensate=P-01 --business-date=YYYY-MM-DD` while it holds the same
  production monitor lease;
- both modes converge on one `BusinessDateOnce` durable decision and one typed
  Feishu receipt.

For example, the P-01 due on Tuesday 2026-08-18 binds LimitPools and the derived
chain to Monday 2026-08-17, then admits identity evidence for the exact head set
and news published within `[2026-08-17, 2026-08-18]`. A Monday P-01 binds its
completed-day sources to the previous valid trading day, normally Friday; it
must not use Saturday, Sunday, local wall-clock subtraction, or a table's
independent latest row.

Gate A is the only completed gate in this change. No P-01 production receipt is
claimed by this document.

## 2. Pre-flight

### 2.1 Impacted implementation paths

- `src/bin/monitor/main.rs`: resident pre-market owner and terminal command
  integration.
- `src/bin/monitor/p01.rs`: closed P-01 date, schedule, input, result, and error
  types.
- `src/selection/process_bootstrap.rs`: strict compensation CLI grammar.
- `src/database/concepts.rs`: exact-date chain read.
- `src/pipeline/chain_analysis/`: deterministic source-only chain projection
  from the admitted exact-date LimitPools batch, with no notification.
- `src/data_gateway/sina_instrument_news.rs` and
  `src/data_gateway/grpc_source.rs`: existing per-head bounded InstrumentNews
  gateway/ExternalV1 route, adopted without another transport.
- `src/bin/monitor/push_templates.rs`: exact P-01 binding, renderer input, and
  durable dispatcher.
- `src/durable_delivery/model.rs`, `src/durable_delivery/schema.rs`,
  `src/durable_delivery/tests.rs`: P-01 durable kind, policy catalog migration,
  and state-machine tests.
- `src/bin/monitor/durable_delivery_runtime.rs`,
  `src/bin/monitor/notify.rs`: counted binding, authoritative sink, receipt,
  and exact audit join.
- `tests/monitor_help_isolation.rs`,
  `tests/durable_delivery_counted_cutover.rs`: process/lease, isolation,
  exactly-once, and crash recovery.

### 2.2 Triggered mandatory rules

- 2.1: every P-01 source is real; no production test or fabricated fallback.
- 2.2: missing source fields remain an explicit failure.
- 2.3: bad or conflicting source records fail before composition.
- 2.4: daily evidence is tied to the exact completed trading date.
- 2.5: `--test` and `TEST_CODE` can never satisfy compensation.
- 2.7: acquisition, composition, decision, attempt, receipt, and exact join are
  auditable.
- 2.8: the compensation dispatcher and notification must perform their named
  operations.
- 2.10: BR-241 is registered before implementation because the design adds a
  time filter, exclusive owner, stable ordering, source bound, and per-date
  deduplication rule.

### 2.3 Validation commands

Gate B through D use the commands frozen in section 11. Gate A additionally
uses:

```bash
rg -n 'T[B]D|T[O]DO|implement[[:space:]]later|fill[[:space:]]in[[:space:]]details|Similar[[:space:]]to[[:space:]]Task' \
  docs/superpowers/specs/2026-08-18-p01-reachability-compensation-design.md \
  docs/superpowers/plans/2026-08-18-p01-reachability-compensation.md
git diff --check
bash tools/compliance/lib/check_business_rules.sh
```

### 2.4 Rollback

Before production acceptance, rollback is a scoped `git revert` of the P-01
implementation PR. After any accepted delivery, rollback must preserve durable
decisions, typed receipts, immutable audits, push logs, and acquisition
evidence. Rollback restores the previous binary but never deletes or rewrites
evidence. If an accepted P-01 exists for the business date, the restored binary
must not be used to resend it.

## 3. Reproduced facts

### 3.1 P-01 is below the market-active wait

Command:

```bash
rg -n -A8 -B4 'while !is_market_active|let mut preopen_pushed|dispatch_preopen_news_hot_daily' \
  src/bin/monitor/main.rs
```

Relevant output:

```text
8155:            while !is_market_active() {
8298:            // v32: P-01 ... 9:00-9:15
8300:            let mut preopen_pushed = false;
8343:                        let preopen_ok = push_templates::dispatch_preopen_news_hot_daily().await;
8359:                            preopen_pushed = preopen_ok && candidate_ok;
```

The inner P-01 branch cannot be entered until `is_market_active()` is true. Its
simultaneous requirements of `09:00 <= now < 09:15` and closed session are
therefore structurally unreachable. Its in-memory completion flag is also
coupled to P-03: an independent P-03 failure can leave P-01 eligible to resend.

### 3.2 No P-01-only command exists

Command:

```bash
sed -n '3255,3279p' src/bin/monitor/main.rs
sed -n '337,349p' src/selection/process_bootstrap.rs
```

Relevant output:

```text
Usage: monitor
       monitor --test [--e2e]
       monitor --review
       monitor --replay=YYYY-MM-DD ...
       monitor --history ...
struct ParsedSelectionCli {
    review: bool,
    e2e: bool,
    v13_diag: bool,
    push: bool,
    push_dry_run: bool,
```

`--push` is not a compensation seam. It selects a group by current wall-clock
window; outside pre-open it can invoke A-01/A-10 and therefore violates the
single-task requirement.

### 3.3 P-01 relies on an unwritten rotation table and has no LimitPools binding

Commands:

```bash
rg -n -A55 'pub async fn dispatch_preopen_news_hot_daily' \
  src/bin/monitor/push_templates.rs
rg -n -A12 'get_latest_chain_clusters_strict|get_latest_board_rotations_strict' \
  src/database/concepts.rs
rg -n 'save_board_rotations' src --glob '*.rs'
```

Relevant output:

```text
get_latest_chain_clusters_strict()
get_latest_board_rotations_strict()
WHERE date = (SELECT MAX(date) FROM chain_daily)
WHERE date = (SELECT MAX(date) FROM board_rotation_daily)
```

The dispatcher currently loads chain and rotation independently and does not
consume `LimitPools`. On 2026-08-18 the inspected production database had
`chain_daily` latest date 2026-08-14 and `board_rotation_daily` latest date
2026-07-16. Those rows cannot authorize the 2026-08-18 P-01, whose required
completed evidence date is 2026-08-17.

The only `save_board_rotations` callers in `src/` are DAO tests; production has
no writer for that table. Designing an exact-date rotation refresh would invent
a new producer rather than close the shortest real path. BR-241 therefore
retires `board_rotation_daily` only from P-01. P-01 derives its chain projection
from admitted LimitPools fields `industry`, `board_name`, and `reason`; gets head
names from the existing `SecurityIdentity` gateway; and gets head news from the
existing `SinaInstrumentNewsGateway`/ExternalV1 route. Other callers of the
rotation table remain unchanged.

The only inspected immutable limit-pool audit was an admitted
`OpeningStatic-UpperLimitPoolReview` for 2026-08-17. It is not the exact
LocalBridge `LimitPools` request and cannot be relabelled as P-01 evidence.

### 3.4 P-01 is outside the durable counted path

Commands:

```bash
rg -n -A45 'fn durable_kind_and_sub_kind_with_override' \
  src/bin/monitor/durable_delivery_runtime.rs
rg -n -A28 'pub enum PushKind' src/durable_delivery/model.rs
rg -n -A20 'async fn deliver_and_record' src/bin/monitor/notify.rs
```

Relevant output:

```text
K::HoldingPlan => ...
K::CatalystReview => ...
_ => return None,
pub enum PushKind {
    HoldingPlan,
    ...
    CatalystReview,
}
async fn deliver_and_record(...) -> PushOutcome
```

`PreopenNewsHot` is absent from both durable mappings and uses generic boolean
delivery. That path cannot produce the required typed Feishu Accepted receipt
or the durable decision/attempt/result/audit exact join.

## 4. Invariants and non-goals

### 4.1 Required invariants

1. One production monitor lease protects resident and compensation modes.
2. `business_date` is a valid local trading day and equals the local date for a
   production compensation.
3. The sink-side binding validator first requires
   `calendar::verified_a_share_trading_day(business_date) == Ok(true)` and only
   then resolves `evidence_date` through the immutable, fail-closed
   `calendar::verified_prev_a_share_trading_day(business_date)`. A weekend,
   exchange holiday, runtime holiday override, or unavailable coverage cannot
   authorize P-01 and returns the stable
   `counted_p01_calendar_authority_unavailable` reason before sink I/O.
4. LimitPools record dates and the derived chain date equal `evidence_date`.
5. The exact LimitPools request is
   `{kind:"Upper",trading_date:evidence_date,limit:200}`.
6. A configured RPC failure is terminal for the attempt; it cannot fall back to
   `UpperLimitPoolReview`, a library route, cache, another date, or empty rows.
7. The chain projection is built only from that admitted LimitPools batch using
   deterministic provider-owned classification fields and is persisted/read by
   exact date, never `MAX(date)`.
8. `board_rotation_daily` is not read or refreshed by P-01. Every selected head
   must appear in the exact `SecurityIdentity` batch, and every head must have
   an Available or VerifiedEmpty InstrumentNews batch for the inclusive local
   date range `[evidence_date, business_date]`.
9. Rendering requires at least one admitted real InstrumentNews record across
   the selected heads. VerifiedEmpty remains evidence but cannot be relabelled
   as a news item.
10. LimitPools, exact chain, SecurityIdentity, every InstrumentNews batch, all
    per-record exclusion decisions, and the rendered bytes enter the canonical
    P-01 source binding. Any byte change changes its hash and the delivery
    subject hash.
11. `BusinessDateOnce` is authoritative across scheduled execution,
    compensation, restart, and process crash.
12. Only a typed `AuthoritativeSinkResult::Accepted` with non-empty local and
    platform message IDs is `Delivered`.
13. An uncertain sink result is never blindly resent. Reconciliation must
    resolve it before another sink attempt.

### 4.2 Non-goals

- Do not repair P-02, P-03, A-01, A-10, review, or trading behavior in this PR.
- Do not introduce a second monitor, a lease bypass, or an unauthenticated
  resident control socket.
- Do not use `--test`, BR-196 fixtures, dry-run output, DataMode, or another
  `PushKind` as P-01 acceptance.
- Do not change the 09:00 inclusive / 09:15 exclusive scheduled window.
- Do not widen data freshness or RPC admission rules.

## 5. Closed domain model

The implementation introduces closed types rather than more booleans:

```rust
pub struct P01BusinessContext {
    pub business_date: chrono::NaiveDate,
    pub evidence_date: chrono::NaiveDate,
}

pub enum P01ExecutionMode {
    Scheduled,
    Compensation,
}

pub enum P01Due {
    Due(P01BusinessContext),
    NotDue(P01NotDueReason),
}

pub enum P01NotDueReason {
    NonTradingDay,
    BeforeWindow,
    ScheduledWindowClosed,
    CompensationBeforeWindowClosed,
    BusinessDateMismatch,
}

pub enum P01RunOutcome {
    Delivered { decision_identity: String, receipt_sha256: String },
    AlreadyDelivered { decision_identity: String },
    AwaitingReconciliation { attempt_identity: String },
    RetryableFailure(P01Failure),
    TerminalFailure(P01Failure),
}

pub enum P01RenderMode {
    Scheduled,
    Compensation,
}

pub struct P01InputBinding {
    pub context: P01BusinessContext,
    pub canonical_source_bytes: Vec<u8>,
    pub source_evidence_fingerprint: String,
    pub rendered_content_sha256: String,
}
```

`P01Failure` carries a stable reason code, retryability, `business_date`,
`evidence_date` when resolved, stage, safe source identity hashes, and observed
time. It does not carry credentials, full sensitive payloads, or fabricated
provider time.

The canonical source binding is versioned and exact-key validated:

```text
P01_SOURCE_BINDING_V1
  business_date
  evidence_date
  template_id = preopen_news_hot_v1
  schedule_occurrence_identity = p01:<business_date>
  limit_pools = {request_hash, provider, source, source_at, observed_at, batch_id,
                 ordered_record_hashes, record_count, verified_empty}
  chain_daily = {source_at, ordered_row_hashes, record_count, persistence_receipt}
  security_identity = {requested_codes, provider, source, source_at, observed_at,
                       batch_id, ordered_record_hashes}
  instrument_news = [{code, range_start, range_end, provider, source, source_at,
                      observed_at, batch_id, ordered_record_hashes,
                      verified_empty}]
  excluded_limit_pool_records = [{record_hash, reason_code}]
  rendered_content_sha256
```

Rows are ordered by stable domain keys before hashing. A verified-empty
LimitPools batch is valid source evidence but cannot form a P-01 hot-chain card;
an empty derived chain, an incomplete identity set, or zero admitted news rows
is an explicit P-01 source failure under BR-241.

## 6. Selected data flow

```text
resident pre-market owner OR exclusive compensation CLI
                    |
                    v
      resolve business_date/evidence_date from calendar
                    |
                    v
      inspect BusinessDateOnce durable occurrence
        | Delivered -> AlreadyDelivered, zero provider/sink
        | Uncertain -> AwaitingReconciliation, zero resend
        v
 exact LimitPools(evidence_date, Upper, 200) RPC
                    |
 derive + persist exact-date chain from the same LimitPools batch
                    |
 exact SecurityIdentity(head codes)
                    |
 InstrumentNews(head, [evidence_date,business_date]) for every head
                    |
                    v
 validate ranges/exact sets + compose canonical source binding
                    |
                    v
 render preopen_news_hot_v1 + bind rendered_content_sha256
                    |
                    v
 durable prepare -> immutable audit -> authoritative Feishu CLI
                    |
                    v
 typed Accepted receipt -> durable commit -> exact join audit
```

No step may call a non-P01 renderer or sink. The exact-date chain projection is
a pure/source-only seam over the already admitted LimitPools batch. It must not
call a chain-analysis mode that also sends an industry-chain notification.

## 7. Ownership and scheduling

### 7.1 Resident owner

`p01_scheduler_loop` is a sibling future of `market_loop`; it starts only after
static opening readiness and before the application waits for market activity.
Every 30 seconds it evaluates one captured local clock value:

- non-trading day: typed `NotDue`, no source/sink;
- before 09:00 or at/after 09:15: typed `NotDue`, no source/sink;
- `09:00:00 <= now < 09:15:00`: inspect durable occurrence, then run P-01 if
  eligible.

There is no `preopen_pushed` memory flag. Durable authority permits repeated
ticks and process restarts without repeated external delivery.

The legacy P-01 block below `while !is_market_active()` is removed. P-03 remains
outside this P-01 owner and cannot influence P-01 completion.

### 7.2 Compensation owner

The only grammar is:

```text
monitor --compensate=P-01 --business-date=YYYY-MM-DD
```

It is valid only when all of the following hold:

- process mode is production; `--test` is rejected;
- both arguments occur exactly once and no other operational argument exists;
- task value is exactly `P-01`;
- date parses, is a trading day, equals local today, and local time is at or
  after 09:15;
- the process acquires the normal production monitor lease before opening the
  gateway, durable runtime, audit writers, or sink.

If the resident monitor holds the lease, compensation exits with
`monitor_instance_already_running` and performs zero provider/durable/sink
calls. Operational use therefore requires a controlled single-owner cutover:
stop the old resident, run the new terminal compensation under the released
lease, then start the new resident. At no point may two monitors deliver.

Adding an in-process authenticated command inbox is explicitly outside this
slice. It is required only if operations cannot perform a controlled cutover.

Compensation rendering must not impersonate the 09:00 scheduled message. Its
first lines state `盘前热点补发`, the `business_date`, and
`依据前一交易日 <evidence_date>`; the captured execution time remains the real
late time. Scheduled rendering retains the normal P-01 title.

## 8. Durable delivery and receipt authority

Add durable `PushKind::PreopenNewsHot` with:

- stable template ID `preopen_news_hot_v1`;
- `CooldownScope::Global`;
- `WindowMode::BusinessDateOnce`;
- occurrence identity `p01:<business_date>`;
- `DeliverySubKind::None`;
- `counts_against_daily_budget=false`, because P-01 is one required daily
  opening message and late recovery must not be starved by unrelated intraday
  messages;
- `InternalDurable`-origin `CountedDeliveryBinding`, because no single provider
  truthfully represents the composite source; its canonical bytes contain the
  original LimitPools, exact chain, SecurityIdentity, every per-head
  InstrumentNews identity, exclusions, and the rendered bytes hash without
  relabelling any child evidence.

The policy catalog count/version and schema migration must be updated together.
Migration may replay only the policy catalog; it may not delete or rewrite old
decisions, attempts, receipts, cooldown heads, or audits.

P-01 obtains a production presentation token and calls the existing public
`notify::push_counted_with_binding`; it does not call the private sink adapter
directly. That counted entry reaches the production MagicLaw CLI authoritative
transport. HTTP, daemon-only acceptance, empty output, a local-only ID, or a
missing platform ID cannot become `Delivered`.

Before any P-01 provider call, the runtime uses a new generic
`inspect_business_date_once_claim(business_date, push_kind, sub_kind,
scope_key, occurrence_identity)`. It reads the durable owner for any
BusinessDateOnce kind and must not construct or reuse a `ReviewTask` identity.
The existing review-specific wrapper may delegate to this generic seam.

The immutable delivery audit must expose enough hashed linkage to verify:

```text
P01 source binding hash
  == durable envelope source_evidence_fingerprint
rendered bytes hash
  == authoritative request rendered_content_sha256
decision identity -> attempt identity -> sink result hash
typed receipt hash -> immutable audit record -> committed durable row
```

The exact join contains `receipt_sha256`, `sink_result_sha256`,
`counted_join_hash`, provider batch identities, decision identity, attempt
identity, template ID, business date, and evidence date. It contains no secret
or full private payload.

## 9. Failure classification

| Stage | Stable class | Retry behavior | Sink calls |
| --- | --- | --- | --- |
| Calendar | invalid/non-trading/mismatch | terminal for command | 0 |
| Lease | monitor already running | safe operational retry after cutover | 0 |
| Durable inspect | corrupt/unavailable authority | fail closed | 0 |
| Durable inspect | already delivered | terminal dedup result | 0 |
| Durable inspect | uncertain attempt | reconciliation only | 0 |
| LimitPools RPC | unavailable/timeout | retryable if typed upstream says so | 0 |
| LimitPools admission | partial/wrong date/mixed/duplicate/over-limit | terminal for evidence batch | 0 |
| Chain projection | no usable provider classification or persist/read mismatch | terminal for evidence set | 0 |
| SecurityIdentity | incomplete/wrong-code/mixed evidence | terminal for evidence set | 0 |
| InstrumentNews | request/range/batch failure for any head | typed retryable or terminal failure | 0 |
| Composition | date/range/exact-set mismatch | terminal for evidence set | 0 |
| Render | empty/invalid/missing identity | terminal for evidence set | 0 |
| Sink | typed rejection before acceptance | use typed retry authorization | at most 1 |
| Sink | uncertain | reconcile; no automatic resend | 1 |
| Post-accept commit/audit | incomplete local commit | recovery converges from accepted receipt | 1 total |

Failures are appended to the appropriate acquisition/delivery audit with safe
reason codes. A log line alone is not completion and is not a durable receipt.

## 10. Old module disposition

| Existing module/path | Decision | Reason |
| --- | --- | --- |
| `market_loop` P-01 block | retire for P-01 | unreachable below market-active wait and uses volatile coupled state |
| `OpportunitySchedule::push_window` | retain for manual grouped `--push` only | exact-time grouped routing is not P-01 compensation authority |
| `run_daily_pushes --push` | reject for compensation | can dispatch A-01/A-10 outside the P-01 window |
| `dispatch_preopen_news_hot_daily` | replace with typed exact-date runner | current bool result, independent latest rows, and generic delivery are insufficient |
| `build_preopen_news_hot_from_db` | replace for P-01 | it requires an unwritten rotation table and cannot express compensation provenance |
| `render_preopen_news_hot` | adopt through scheduled/compensation parameters | content shape remains, with explicit late-compensation labeling |
| `get_latest_*_strict` | retain for unrelated callers; reject for P-01 | `MAX(date)` can select stale evidence |
| `board_rotation_daily` | retire only from P-01 | DAO has no production writer; BR-241 replaces its P-01 news/name role |
| `ReviewDataGateway::current_upper_limit_pool` | adopt behind the library P-01 projection seam | it is crate-private and already owns exact LocalBridge LimitPools request/admission |
| `cluster_by_concept` / chain persistence primitives | adopt behind a source-only exact-date seam | chain is derived from the same LimitPools batch and emits no other push |
| `MarketCapabilitiesGateway::security_identities` | adopt | existing exact-set admitted identity route |
| `SinaInstrumentNewsGateway::instrument_news_in_range` | adopt | existing bounded per-instrument news route with ExternalV1 bridge |
| generic `deliver_and_record` | reject for P-01 | boolean delivery has no authoritative typed receipt/exact join |
| `notify::push_counted_with_binding` | adopt | existing public counted-delivery entry |
| review-only occurrence preflight | generalize, then retain as wrapper | P-01 must preflight before providers without pretending to be a review task |
| BR-196 test P-01 | retain as isolated test only | cannot satisfy production evidence |
| production monitor lease | adopt unchanged | prevents a second delivering monitor |

BR-241 supersedes BR-049 only for the P-01 owner placement and supersedes the
P-01 portions of BR-101/BR-225 for content source and date selection. It does
not change their P-03 behavior or any non-P01 caller.

## 11. Validation and production acceptance

### 11.1 Focused RED/GREEN

The implementation plan freezes individual commands. At minimum:

```bash
cargo test --bin monitor p01_ -- --nocapture
cargo test --lib database::concepts::tests::p01_ -- --nocapture
cargo test --lib durable_delivery::tests::p01_ -- --nocapture
cargo test --test monitor_help_isolation p01_ -- --nocapture
cargo test --test durable_delivery_counted_cutover p01_ -- --nocapture
```

Required cases include:

- scheduled window boundaries `09:00:00` and `09:15:00`;
- Tuesday 2026-08-18 -> Monday 2026-08-17;
- Monday -> previous Friday across a weekend;
- holiday predecessor resolution through the admitted calendar;
- sink-side binding rejection for a weekend, exchange holiday, and
  out-of-coverage 2027-01-01 business date, all with the stable
  `counted_p01_calendar_authority_unavailable` reason;
- exact completed date across LimitPools and chain;
- exact SecurityIdentity head-code set and one bounded InstrumentNews result
  per head over `[evidence_date,business_date]`;
- Monday-to-Friday range construction and late compensation labeling;
- stale, future, independently latest, partial, conflicting, duplicate and
  over-limit evidence rejection;
- P-03 failure after P-01 acceptance causes zero additional P-01 sink calls;
- compensation invokes zero A-01/A-10/other `PushKind` paths;
- lease conflict rejects before provider, durable store, audit writer, or sink;
- first accepted run sends once; same-day scheduled/compensation/restart runs
  send zero additional messages;
- crash after remote acceptance and before local completion converges without
  resend;
- uncertain result blocks resend until reconciliation.

### 11.2 Mandatory repository gates

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

### 11.3 Production acceptance

Production acceptance requires a controlled single-owner cutover and all of
the following evidence for the same local date:

1. An acquisition audit proves exact LocalBridge request
   `{kind:"Upper",trading_date:evidence_date,limit:200}` with admitted complete
   provider-backed records.
2. The exact-date chain persistence receipt names the same `evidence_date` and
   is derivably bound to that LimitPools batch.
3. Exact SecurityIdentity and every per-head InstrumentNews batch are admitted
   and included in the canonical binding.
4. The P-01 source binding hash joins all inputs to the scheduled or explicitly
   late-labeled `preopen_news_hot_v1` bytes.
5. Feishu returns typed `Accepted` with non-empty local and platform IDs.
6. The durable decision, attempt, sink result, receipt, immutable audit, and
   push artifact pass the exact join validator.
7. Re-running the compensation command yields durable deduplication and no new
   Feishu message.
8. Starting the resident monitor afterward produces no duplicate P-01 for that
   business date.

`--test`, dry-run, another `PushKind`, a transport handshake, a startup banner,
or the 2026-08-18 DataMode receipt satisfies none of these criteria.

## 12. Release and rollback sequence

1. Complete Gate B/C/D with an independent verifier.
2. Build the release monitor and preserve the old binary for binary rollback.
3. Stop the old resident through the existing controlled service procedure.
4. Confirm the production monitor lease is released.
5. Run exactly one P-01 compensation command for the local date.
6. Validate the real Feishu Accepted receipt and durable exact join.
7. Repeat the command once and prove zero second sink call.
8. Start the new resident and prove the same business date remains deduplicated.
9. If the cutover fails before acceptance, restore the old binary. If remote
   acceptance may have occurred, reconcile first; never resend based only on a
   missing local completion line.
