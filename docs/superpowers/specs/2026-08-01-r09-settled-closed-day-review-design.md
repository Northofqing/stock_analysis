# R-09 Settled Closed-Day Review Supporting Contract

**Date:** 2026-08-01
**Rules:** BR-192, BR-194, BR-198, BR-200, BR-202
**Authority status:** supporting Gate-A contract candidate for BR-192 only.

This document does not own an independent BR-198 Gate B, Gate C, implementation
commit, release slice, or prerequisite Gate C. BR-192 Task 8/Gate B is the sole
implementation authority that atomically creates the R-09 gateway, producer,
dependency closure, tests, compliance evidence and checked-in forward rollback
patch. BR-198 therefore must not be used as a prerequisite that blocks creation
of the very BR-192 artifacts needed to implement it.

BR-200 must first land its provider-free occurrence preflight while R-09 remains
disabled. The accepted BR-192 implementation then adopts this complete contract
and preserves the order:

```text
BR-194/BR-198 static date preflight
  -> accepted BR-200 durable occurrence preflight
  -> BR-192 counted-producer permit
  -> R-09 gateway/provider
  -> renderer
  -> durable counted delivery
  -> real sink
```

The fixed factual baseline used to write this contract is
`b4aeee68d2c0259cc968914b3d39e3a89a18a496`. That baseline has the R-09 task
declaration and current-date-only preflight, but no production R-09 producer,
no `src/data_gateway/capital.rs`, no BR-200 occurrence seam and no atomic Magic
release-identity test. Dirty-worktree versions of those artifacts are candidates,
not accepted upstream authority.

## 1. Intent and boundary

`monitor --review` resolves a weekend or holiday invocation to the latest
confirmed trading date. R-09 may request that exact calendar-selected date from
the pinned Provider Top-N route when the upstream response proves that date as
its latest settled session. This is not arbitrary historical replay and adds no
caller-selected date CLI.

This contract only replaces the R-09 `current Shanghai date only` and
`closed-day zero-network` clauses. It does not weaken SourceOnly classification,
BR-200 terminal reuse, counted-producer permits, durable uniqueness, budget,
deduplication, audit, hydration, retry freshness, rendering or sink ownership.

## 2. Shanghai observation authority

R-09 must not derive a business or observation date from host-local timezone
configuration. Production captures time from the trusted system UTC clock and
converts it to the fixed Asia/Shanghai `+08:00` authority before constructing
`ReviewRunContext`. A naive host `chrono::Local::now().naive_local()` value is
not BR-198 authority.

The immutable acquisition window contains:

- `request_started_at`: captured before router construction or provider I/O;
- one closed `ProviderCaptureEvidenceV1` per metric, with the exact field
  manifest `raw_timestamp_bytes: Box<[u8]>`,
  `parsed_timestamp: DateTime<FixedOffset>`, and
  `raw_timestamp_sha256: String` validated as exactly 64 lowercase ASCII hex;
- `capture_completed_at`: captured from the same trusted clock immediately
  after both batches return and before pair admission.

All three trusted boundary observations use fixed `+08:00`. Each provider value
is parsed as complete RFC 3339 while its exact original bytes remain in retained
evidence. Its digest is
`SHA-256("stock_analysis.br198.provider_capture_raw.v1\0" ||
u64_be(raw_timestamp_bytes.len()) || raw_timestamp_bytes)`. The closed pair is
exactly `ProviderTopNPairCaptureBindingV1 { volume_ratio_capture,
main_net_inflow_capture }` and appears as the single
`capture_binding: ProviderTopNPairCaptureBindingV1` field of the canonical
counted binding. Compact serde JSON follows declaration order and encodes boxed
bytes as exact integer arrays; there is no pair-only hash because the containing
binding hash covers both nested values and each raw digest is independently
recomputed on read. Trim, normalization, re-encoding or replacing raw bytes is
forbidden.
The complete pair is rejected unless:

1. `request_started_at <= provider_capture <= capture_completed_at` for both
   provider captures;
2. request start, both provider captures and completion belong to one identical
   Asia/Shanghai calendar date;
3. the trusted clock did not move backwards; and
4. both metric batches preserve their independent original capture bytes and
   validated hashes.

This rejects a future same-day provider timestamp, a capture before request
start, malformed time, and a request crossing Shanghai midnight. The caller
clock must never replace provider bytes. Even a byte mutation that parses to the
same instant invalidates the pair before durable prepare/sink. Production constructors install only
the trusted clock; deterministic injection is restricted to `cfg(test)` and
`TEST_CODE` fixtures.

## 3. Static review-date contract

Test/live isolation is evaluated before all date, durable and provider work.
For production R-09, preflight uses the calendar-owned `review_date` and trusted
Shanghai `request_started_at`:

1. `review_date > request_started_at.date_naive()` is non-retryable
   `provider_top_n_future_date` and performs zero durable/provider/sink I/O.
2. Equal date before 15:35 Shanghai time is `ExpectedWait(15:35)`.
3. Equal date at or after 15:35 is runnable.
4. A prior date selected by the review calendar is runnable without the 15:35
   wait; exact provider row dates remain the settlement authority.

`--test` returns `test_environment_external_provider_blocked` before this date
decision and before any external provider or real sink construction.

## 4. Sole source and atomic admission

- The only production route is
  `EastmoneyProviderTopNRankingRouter::new()`.
- Exactly one `VolumeRatio` page and one `MainNetInflow` page are requested for
  the same calendar-selected trading date and fixed limit.
- Every provider row's original `f297` must equal the exact requested trading
  date.
- Both sides must be complete and non-empty, finite, correctly typed and ordered,
  with exact provider/source/metric/unit/filter/request/batch evidence.
- Missing rows, invalid timestamps, request/capture/completion drift, partial or
  verified-empty sides, metric/date/order/provider/evidence drift, or clock
  rollback rejects the whole pair with
  `provider_top_n_invalid_evidence`, zero render, zero durable prepare and zero
  sink.
- No cache, local projection, environment fallback, alternate provider,
  cross-source join, inferred settlement date, current-date substitution,
  fabricated row or partial success is permitted.

The ranked point-in-time facts are not a price series. AGENTS 2.3 price-positive,
adjacent-change, gap/duplicate-series and split/dividend subchecks are scoped
N/A; finite value, exact rank/order and evidence validation apply.

## 5. Atomic Magic release identity

BR-192 Task 8 installs and verifies this identity as one atomic release closure:

- repository: `https://github.com/Northofqing/magic-market-data-rs.git`;
- revision: `5f1ce93656a55854c844065390520cd4aecd9a14`;
- exact package version: `=0.2.0`;
- fourteen direct application dependencies, sorted:
  `magic-baidu-rs`, `magic-cls-rs`, `magic-cninfo-rs`,
  `magic-eastmoney-rs`, `magic-exchange-rs`, `magic-jin10-rs`,
  `magic-market-composition`, `magic-market-core`, `magic-market-router`,
  `magic-sina-rs`, `magic-tdx-rs`, `magic-tencent-rs`,
  `magic-thepaper-rs`, `magic-ths-rs`;
- exactly fifteen Magic lockfile packages: those fourteen plus only transitive
  `magic-market-transport`.

Path dependencies, mixed revisions/versions, another repository, direct
transport use, missing/extra names or a sixteenth lock package fail the BR-192
release gate. Revision `660902ff93a07f18367dc16879cf67732accd25a` is not a
rollback target because it lacks the retained Provider Top-N API.

## 6. Failure dispositions

| Failure | Required disposition |
| --- | --- |
| Future review date | non-retryable `provider_top_n_future_date`, zero I/O |
| Same-day before 15:35 | `ExpectedWait(15:35)` |
| Test process | typed disabled before durable/provider/sink I/O |
| BR-200 existing terminal/ambiguous/error | provider-free reuse or typed fail-closed |
| Router/provider unavailable | explicit typed failure; no empty/fallback conversion |
| Empty/partial/wrong `f297` pair | retryable atomic gateway failure, zero sink |
| Invalid/out-of-window/cross-midnight capture | `provider_top_n_invalid_evidence`, zero sink |
| Durable or sink uncertainty | retain BR-192 uncertainty/reconciliation semantics |

## 7. Module decisions

| Module | Decision | Reason |
| --- | --- | --- |
| `ReviewRunContext` | deepen under BR-192 | Own a typed Shanghai start observation instead of host-local naive time. |
| accepted BR-200 occurrence preflight | adopt | Must terminate reuse/error before permit/provider. |
| `CapitalDataGateway::provider_top_n_pair` | create under BR-192 | Sole pair acquisition and observation-window admission seam. |
| R-09 producer/renderer/durable binding | create/adopt under BR-192 | One real counted producer, unchanged durable/sink authority. |
| current-date-only branch | replace narrowly | Permit only review-calendar latest-settled prior date. |
| cache/local/alternate provider | reject | Cannot prove the requested settled session. |
| independent BR-198 implementation slice | reject | It creates a circular prerequisite with BR-192. |
| `tools/release/disable_br192_periodic_retry.patch` | create under BR-192 | Forward rollback disables only periodic retry discovery without reverting Task 8. |

## 8. Verification and release ownership

All exact BR-198 behavioral tests named in the paired supporting plan are owned
by BR-192 Task 8/Gate B. BR-192 Gate C owns formatting, full workspace tests and
repository compliance. This contract cannot independently claim Gate B or C.

Gate D is minted only through the independently accepted BR-202 isolated
coverage authority `tools/coverage/run_isolated_gate.sh`. Raw coverage output is
diagnostic. Release additionally requires a real closed-day provider batch,
durable audit join and typed Feishu receipt. Until those exist, status remains
Gate D blocked.

Production evidence is exact:

- `data/push_log/YYYY-MM-DD/*_audit_pending.json`;
- `data/push_log/YYYY-MM-DD/*_committed.json`;
- `data/event_bus/YYYY-MM-DD.jsonl` with
  `event_type="push.delivery.audit"` and `ReviewProviderTopN` identity;
- enabled startup line
  `[BR-192][counted-producer] push_kind=ReviewProviderTopN enabled=durable_binding producer=push_templates::dispatch_r09_provider_top_n_outcome`;
- before BR-192 enablement, the corresponding exact
  `disabled=no_producer reason=capability_unavailable:<reason_code>` line.

## 9. Rollback

Rollback is the BR-192-owned forward-compatible patch
`tools/release/disable_br192_periodic_retry.patch`, created, checked in and
validated during BR-192 Gate B against the accepted release source. It is
applied to that exact release commit; it must not revert the atomic Task-8
commit. The patch has exactly one diff target,
`src/bin/monitor/main.rs`, and may disable only startup of the periodic
provider-free retry runner. It must leave initial and repeated-review R-09
dispatch, the accepted BR-200 occurrence preflight, the complete v6-aware
schema/runtime, the exact 15-row counted-producer catalog, durable/audit
semantics, test isolation and the sole Eastmoney router unchanged.

Rollback must not remove this historical contract, restore host-local time,
restore a fallback, disable R-09, change BR-200/schema/catalog/audit authority,
or change the fourteen-direct/fifteen-lock Magic identity. Gate B freezes the
patch SHA-256 and proves that applying it changes only `src/bin/monitor/main.rs`;
the paired supporting plan owns the executable application and verification
commands. All twelve canonical `br192_br198_*` tests, the BR-200 R-09 tests,
schema/catalog/audit checks and exact release-identity test must pass after the
patch. The BR-192 owner reconciles the BR-198 business-rule status in the same
reviewed rollback PR.
