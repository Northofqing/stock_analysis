# Monitor Notification Liveness Repair Design

**Date:** 2026-07-20

**Scope:** production `monitor` notification governance and Eastmoney announcement detail transport

**Rules:** AGENTS 2.1, 2.2, 2.3, 2.4, 2.7, 2.8, 2.10; BR-105, BR-108, BR-113, BR-115, BR-116, BR-134

## 1. Observed failure

The local private runtime log was inspected only through fixed counters. Across the observation
window the process stayed alive, but notification-producing paths repeatedly reported:

- the governance banner was unavailable;
- the paper risk context was unavailable;
- the announcement detail batch failed with an empty response body.

The production database contains one attested real-account snapshot dated 2026-07-18. Its daily
P&L and account mode are explicitly absent, and the latest ledger row is older. Those facts may be
kept for audit but fail the 30-second action freshness gate and cannot be converted into zero,
`Normal`, or a current-day ledger row.

An official-endpoint protocol probe, which emitted only response sizes and field names, showed:

- the old detail path returns an empty body;
- the current Eastmoney detail path returns a non-empty JSON object;
- the current payload uses `data.notice_content`, not `data.content`.

## 2. Root causes

### 2.1 Banner bootstrap cycle

`LATEST_BANNER` is written only after complete account metrics are assembled. Missing current-day
ledger data makes account assembly return before the banner is stored. DataMode evaluation then
requires that same banner before it can store the latest real data state or send T-02. Every push
therefore fails closed at governance even though the sink is configured.

### 2.2 Initial unsafe state is silent

DataMode considers `prev=None` not to be a transition. A process that starts directly in
`Unsafe` records the state but emits no T-02 notification. In addition, the v14 adapter currently
represents `PushKind::DataMode` as a holding-health event and disables the registered
data-source-down exemption, so an Unsafe/Down context rejects its own recovery warning.

### 2.3 Announcement detail protocol drift

The list endpoint still returns a valid strict batch. High-risk detail requests use an obsolete
host/path and parser field, so every required detail is rejected and the complete announcement
batch is correctly withheld. The failure is transport/protocol drift, not an empty-news fact.

## 3. Chosen design

### 3.1 Explicit incomplete account metrics

`PortfolioMetrics` and `BannerCtx` represent daily P&L, consecutive-loss count, and position ratio
with `Option` values. Complete account evaluation requires all three metrics. If account assembly
fails, the monitor creates an explicit incomplete metric set (all absent), logs the source error,
and evaluates it through the existing account-mode rule:

- an existing `Frozen` state remains `Frozen`;
- otherwise incomplete account data yields `ReduceOnly`;
- no missing number is persisted or rendered as zero.

The account-mode audit writes SQL `NULL` for every missing metric and `data_complete=false`.
The banner renders missing account fields as `缺失`, carries the real DataMode evaluation, and may
govern notification delivery. `paper_risk_context_from_banner` becomes fallible and continues to
reject paper/trading work unless the account metrics are complete, preserving BR-134.

### 3.2 Operational state notifications remain audible

The first real DataMode evaluation is handled as follows:

- `Full`: establish state without noise;
- `Degraded` or `Unsafe`: render `未建立 → <mode>` and attempt T-02 delivery;
- later evaluations retain transition-only behavior.

The v14 adapter constructs DataMode events as `SignalSource::DataSourceDown` with a
`DataSourceDownPayload`, classifies the profile as `DataSource`, and enables the existing
`always_send_on_data_source_down` exemption. AccountMode status messages accept `Down` data
quality because they communicate risk restrictions rather than authorize a trade. All messages
still pass L4 dedup, L5 governance, the real sink, delivery audit, and L7 persistence.

Mode state is committed only after `Pushed` or an authoritative `Deduped` result. A denied or
sink-failed DataMode transition keeps the previous state so the next evaluation retries. An
unpushed `account_mode_log` row is reused for retry instead of being treated as a delivered state
or duplicated on every loop; only successful delivery marks that immutable audit row pushed.

### 3.3 Strict current announcement detail protocol

Production keeps the existing announcement list endpoint and switches only required detail calls
to the current official detail endpoint. The parser requires:

- HTTP 2xx;
- readable, non-empty JSON;
- `success=true`;
- non-empty `data.art_code` equal to the requested identity;
- non-empty `data.notice_content`.

Any list row or required detail failure still rejects the complete source batch. There is no
title-only, stale, cached, PDF, or mock fallback.

## 4. Data flow

```text
account sources ── complete ──> Some(metrics) ──> AccountMode
       │
       └─ unavailable ────────> None metrics ──> ReduceOnly/Frozen
                                                │
real capability success times ──> DataMode ─────┤
                                                ▼
                                     explicit BannerCtx
                                       │           │
                                       │           └─ complete only ─> PaperRiskContext
                                       ▼
                         L4 dedup → L5 governance → real sink → L7 audit

Eastmoney list ─> strict list validation ─> required detail identities
                                              │
current detail endpoint ─> strict identity/content validation ─> complete batch
```

## 5. Failure handling

- Account data remains unavailable: send/retain conservative status, reject paper/trading work,
  and retry real account assembly on the existing schedule.
- DataMode notification fails: do not commit the new latest-notified mode; keep explicit failure
  and retry according to BR-116.
- AccountMode notification fails: retain and reuse the existing `pushed=0` audit row; do not
  insert duplicate transitions or mistake an unpushed row for a completed alert.
- Announcement detail fails: reject the entire batch and retry on the existing poll interval.
- Sink or L7 audit fails: return `SinkError`/`Denied`; never report `Pushed`.
- Locks, invalid persisted modes, bad JSON, identity mismatch, and empty content remain hard errors.

## 6. Old-module decisions

| Module | Decision | Reason |
| --- | --- | --- |
| `risk::account_mode::evaluate` | adopt | already defines conservative incomplete-data behavior |
| `monitor::data_mode::evaluate` | adopt | consumes real capability-success timestamps |
| `LATEST_BANNER` | deepen | retain one state object but make missing numeric facts explicit |
| v14 L4/L5/L7 stack | adopt | status alerts must remain governed and audited |
| obsolete announcement detail path | reject | live protocol evidence shows an empty response |
| stale snapshot / zero / cached detail fallback | reject | violates freshness and missing-data rules |

## 7. Test and release evidence

Tests are vertical RED→GREEN slices:

1. incomplete account facts evaluate conservatively and remain absent in banner/audit rendering;
2. paper risk context rejects an incomplete banner;
3. initial Unsafe DataMode produces an eligible data-source-down event;
4. complete current announcement detail JSON succeeds while old/mismatched/empty payloads fail.

Required gates:

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

After PR merge, restart exactly one master release process. Runtime acceptance uses only fixed
counters: at least one state notification reaches a real sink and its delivery/L7 audit succeeds;
the announcement batch no longer fails because of the obsolete empty detail path. No message
content, account value, security identity, credential, or destination is printed or committed.

## 8. Rollback

Revert the merge commit, rebuild `monitor`, terminate only the current monitor PID, and restart the
previous master release. There is no database migration or threshold change. Existing append-only
account evidence, delivery audit, and the private raw log remain untouched.
