# NewsFlash two-phase source-bound delivery design

**Status:** Corrective Gate A re-approved on 2026-08-18 after final-review findings 3C4I

**Business rules:** BR-082, BR-244

**Scope:** public SourceOnly `NewsFlashAggregated`; `NewsFlashCritical` is explicitly disabled until an authoritative strength provider exists

## 1. Problem and authority

The production news loop currently makes a NewsFlash decision before it knows whether the physical sink and the authoritative immutable audit accepted that decision. It also projects an admitted raw-news record to a bare `MarketEvent`, so an aggregated card cannot prove which ordered provider records produced its Top3. A provider/projection failure is only logged and cannot prove that the failure audit was durably appended.

The corrective authority is deliberately narrow:

- `RawNewsAggregationBatch` remains the only SourceOnly ingress. Only terminal `Available` records with complete `GlobalNewsRecord + BatchEvidence` are admitted.
- `NewsFlashGate` owns selection, event/day deduplication, daily capacity and the four aggregate windows. It does not perform I/O. A startup authority snapshot and a fresh pre-reserve snapshot are inputs to the gate; process memory is never the restart authority.
- A NewsFlash-specific transaction adapter owns presentation, Launch/L4/L5, physical sink, L7 and the source-bound immutable delivery audit.
- The adapter must append and sync a source-bound immutable `SinkAttempt` before invoking the physical sink. Only a typed `Accepted` terminal whose branded audit receipt exactly joins the reserved decision, sink attempt and remote receipt may commit gate state.
- The selection/LLM/candidate branch retains BR-174 authority and runs only after the public SourceOnly NewsFlash branch for the tick.
- Public SourceOnly records are fixed at `Neutral/strength=0`. `NewsFlashCritical` therefore prints `disabled=no_authoritative_strength_provider` at startup and has no production reservation or sink caller. Only N-02 aggregate delivery is enabled.

This design does not authorize fake/default/cached news, keyword-based critical upgrades, candidate facts, market facts, account facts or trades.

## 2. Canonical source evidence

The raw projection returns an opaque `NewsFlashProjectedEvent`, not a bare `MarketEvent`. It contains the validated event plus this canonical source identity:

1. event id;
2. provider id;
3. source contract;
4. provider publication time;
5. provider observation time;
6. raw batch id.

The aggregate binding is the ordered list of these six-tuples in exact displayed Top3 order. The implementation serializes the fields with an explicit schema/domain separator and length framing, then computes SHA-256. Sorting the binding independently of display order, removing a field, substituting the current time, or reconstructing evidence from rendered text is invalid.

The provider identity is the closed `GlobalNewsProvider::wire_name()` value (`Eastmoney`, `Cailianpress`, `Jin10`, `ThePaper`), never `Debug`, a lowercase gateway label or arbitrary caller text. Projection validates both the exact provider and its registered source contract before admission. Provider evidence retains its original wire encoding: the projector must reuse the `global_news` module's provider-specific source/observation time parser and must not assume that every admitted `observed_at` is RFC3339 or rewrite it before hashing. Production rejects any `TEST_CODE` event, batch, provider or source. Monitor tests may construct an opaque test capability only after the runtime test namespace is proven; the capability is required by the fixture constructor and is not forgeable from production.

The reservation identity uses schema/domain `stock_analysis.news_flash_reservation.v2` and explicitly includes push kind, business date/window identity, canonical evidence hash and render hash. A sink-attempt identity additionally binds the reservation, attempt ordinal, physical channel and attempt observation time. Its canonical attempt hash binds push kind, reservation/evidence/render, ordered sources and attempt identity. A terminal delivery audit additionally joins the persisted attempt envelope, typed remote receipt identity/hash and terminal state. String logs, boolean sink results, generic persisted receipts and generic delivery records are never authority.

## 3. Gate state machine

`NewsFlashGate` issues a non-cloneable reservation token for every selected decision. Reservation is an in-memory concurrency exclusion, not delivery success.

```text
eligible buffered evidence
        |
        v
     Reserved ------------------------------+
        |                                    |
        | pre-sink Denied/Deduped/error       | sink was attempted but
        | or definitive sink rejection       | final authority is unknown
        v                                    v
    RolledBack                           Uncertain
        |                                    |
        | eligible on a later tick           | reconciliation only;
        +------------------------------->     no automatic resend

Reserved -- physical Accepted + L7 + exact source-bound audit receipt --> Committed
```

The gate maintains independent committed, pending and uncertain identities. Only committed entries consume the confirmed daily quota. Pending entries consume only the bounded in-process concurrency capacity. An uncertain reservation blocks only that exact reservation identity; it does not consume quota or suppress unrelated events/windows. A rollback releases the reservation without deleting the real buffered evidence, so a later tick can retry even when the provider does not repeat the record. Uncertain is neither success nor rollback: it blocks that exact automatic resend until reconciliation resolves it.

On startup, and again immediately before every reserve operation, the owner reads a fully validated authority snapshot from the retained BR-091 audit root. For the requested business date it restores every accepted event id/window and every unclosed/uncertain reservation. Same-day accepted event id dedup is event-level: a later batch or changed evidence for that event id cannot authorize a resend. Exact accepted reservations/windows restore committed; an attempt without a valid terminal, and every `Uncertain` terminal, restore uncertain; a closed `DefinitivelyRejected` terminal permits a new attempt ordinal. Reconciliation never deletes or rewrites history. Day rollover clears only process-local old-day state after the new business-date snapshot is applied.

## 4. Aggregate window contract

The four target times remain 09:30, 11:30, 13:00 and 15:00 local exchange time. A window is due only in the half-open interval:

```text
[target, target + 300 seconds)
```

Therefore target+90 seconds and target+91 seconds are due; target+300 seconds is not due. There is no pre-target tolerance. The gate selects the complete admitted real buffer, ordered by existing BR-082 strength order, then takes Top3. An empty buffer makes no reservation and sends nothing. A committed or uncertain window cannot be automatically attempted again; a rolled-back window may retry while still due.

## 5. Dispatch transaction and outcomes

For each enabled N-02 reservation the dedicated adapter performs, in order:

1. validate the reservation and source evidence binding;
2. render through the existing presentation-token path;
3. evaluate Launch/L4/L5 without consuming NewsFlash gate state;
4. require a receipt-capable transport and append/flush/sync one source-bound `SinkAttempt` to the BR-091 hash chain;
5. call the real receipt-capable sink exactly once;
6. append L7 and one source-bound terminal (`Accepted`, `DefinitivelyRejected` or `Uncertain`) joined to the persisted attempt;
7. return one typed settlement to `NewsFlashGate`.

The physical sink result is a closed enum: `PreAttemptRejected`, `DefinitivelyRejected`, `Accepted { remote_receipt }`, or `Uncertain`. `PreAttemptRejected` occurs only before a request can be sent. `DefinitivelyRejected` requires typed transport evidence proving the remote endpoint did not accept the request. Once the CLI request is issued, any nonzero exit, timeout, missing/invalid receipt or receipt parse error is `Uncertain`. The existing Magiclaw CLI receipt path is the only current production transport that yields both local `message_id` and remote `platform_message_id`; boolean V6/HTTP/legacy paths are rejected before the attempt append and before network I/O.

| Observation | Typed settlement | Gate action | Automatic retry |
|---|---|---|---|
| Launch/L4/L5/transport denies before attempt append | `PreAttemptRejected` | rollback | yes, while eligible |
| Reconciliation proves exact or same-event same-day accepted authority | branded `Accepted` | commit | no |
| Typed transport proves the request was not remotely accepted and the terminal audit closes the attempt | `DefinitivelyRejected` | rollback | yes, while eligible |
| Sink returns a typed remote receipt and L7 plus exact terminal audit append succeeds | `Accepted` with `NewsFlashAcceptedReceipt` | commit | no |
| Request was issued but typed receipt is missing/invalid, or terminal/L7 authority is unavailable/mismatched | `Uncertain` | mark exact reservation uncertain | no; reconcile only |

`NewsFlashAcceptedReceipt` is a branded type minted only after authoritative append and exact reread/validation. It contains push kind, reservation/evidence/render hashes, sink-attempt identity/hash, persisted attempt envelope id, remote-receipt identity/hash and accepted terminal envelope id. The gate revalidates every field. An ordinary `PushOutcome::Pushed`, `PersistedDeliveryAuditReceipt`, log line or unrelated audit receipt cannot be wrapped or upgraded to `Accepted`. Existing BR-145/BR-172 callers keep their own semantics; this adapter must not globally change their commit/rollback behavior.

## 6. Provider and projection failure audit

Every `Unavailable` provider terminal and every record projection failure produces an immutable schema-v6 `news.flash.failure.audit` append with:

- stable closed provider wire name when known;
- stage (`provider` or `projection`);
- stable reason code, `diagnostic_code` and a UTF-8 diagnostic bounded to 512 bytes;
- retryable flag;
- observed time;
- exact `source_record_count`;
- available provider, batch and record identity as separate optional fields;
- domain-separated identity hash over every field above, including explicit absent markers.

The append API returns `Result<NewsFlashFailureAuditReceipt, NewsFlashFailureAuditError>`. The typed error distinguishes invalid input, audit authority unavailable, append failure and exact reread mismatch. Failure is propagated explicitly. It must not be replaced with mutable dispatcher logging or ignored. If the required failure append cannot be proven, the public NewsFlash branch fails closed before any sink attempt for that tick. Successfully admitted records from other providers remain available to later ticks and are never replaced with synthetic records.

## 7. Production loop order

The source-level order is fixed and test-visible:

```text
fetch opaque raw batch
  -> evidence-preserving projection
  -> append all required immutable source failure audits
  -> reconcile full business-date NewsFlash authority
  -> enabled N-02 reserve / attempt append / typed dispatch / terminal append / settle
  -> selection ingress receipt / LLM / candidate consumers
```

This ordering prevents a slow or failing selection/LLM path from starving public news delivery. It does not allow SourceOnly NewsFlash to mint or impersonate a selection ingress receipt.

## 8. Failure modes and rollback

| Failure | Required behavior |
|---|---|
| Missing/mixed provider evidence | projection failure audit; no event admission |
| Failure audit append unavailable | explicit tick failure before sink; no fake success |
| Presentation/Launch/L4/L5 denial | reservation rollback; no quota/window consumption |
| Attempt append unavailable | explicit pre-sink failure; no physical request |
| Physical sink definite rejection | append definitive terminal, then reservation rollback |
| Physical attempt with unknown final authority | uncertain; no blind retry |
| Exact audit receipt does not join push kind/reservation/evidence/render/attempt/remote receipt | uncertain; never commit |
| Process exits after attempt append | startup reconciliation restores that exact reservation as uncertain |
| Selection/LLM failure | cannot undo or precede completed public NewsFlash dispatch for the tick |

Rollback of this change is a scoped code revert of the adapter, projection wrapper, gate state and call ordering. Immutable audit rows are retained. Restoring the pre-change immediate gate mutation or generic evidence-losing N02 path is prohibited because it violates BR-082/BR-244.

## 9. Old-module disposition

| Module | Decision | Reason |
|---|---|---|
| `record_to_market_event` | adopt | unique validated semantic conversion; wrap its result with original evidence |
| `NewsFlashGate` threshold/Top3 logic | adopt and correct | retain selection semantics; replace eager mutation with reservations |
| generic `deliver_and_record` | reject for NewsFlash settlement | its BR-145 semantics cannot prove the NewsFlash source binding |
| presentation token, Launch/L4/L5 and L7 | adopt | required production controls, composed by the dedicated adapter |
| `push_via_magiclaw_cli_receipt_blocking` | adopt behind typed adapter | only current production transport that returns local and remote receipt IDs |
| boolean `push_wechat` / V6 `SinkResult` | reject for NewsFlash | cannot distinguish remote acceptance from uncertainty |
| BR-192 counted coordinator | reject | its task/policy/counting semantics do not belong to SourceOnly NewsFlash |
| mutable dispatcher N-01/N-02 log | reject as authority | cannot return an immutable persisted receipt |
| BR-174 selection receipt path | adopt unchanged | separate selection/candidate authority |

## 10. Verification matrix

- N-01: startup banner is exactly `NewsFlashCritical disabled=no_authoritative_strength_provider`; SourceOnly strength remains zero and there is no N-01 production sink call.
- Aggregate: target+90 and target+91 are due; target+300 and pre-target are not due; rollback retries the identical ordered Top3 binding; accepted commits the window; uncertain blocks only the exact reservation.
- Reconciliation: startup and pre-reserve scans recover all same-business-date accepted event IDs/windows and open/uncertain reservations; changed evidence cannot resend an accepted event ID; closed definitive rejection permits the next ordinal.
- Quota: only committed consumes the daily quota; pending consumes only concurrent capacity; unrelated uncertain entries do not consume quota or block another identity.
- Sink: unsupported boolean transports reject before attempt; request-issued nonzero/timeout/missing receipt is uncertain; typed definitive rejection rolls back only after a closing terminal append.
- Evidence: changing push kind, order or any event/provider/source/published/observed/batch/attempt/receipt field changes the canonical hash; branded accepted receipt must exactly join all fields.
- Failure audit: provider and projection failures append every required schema-v6 field; 513-byte diagnostics fail validation; append failure propagates and performs zero sink calls.
- Production tracer: a source-level test proves reconcile/N-02 dispatch appears before selection ingress/LLM/candidate calls and proves the production loop has a real caller.
- Regression: BR-172 typed receipt tests and existing generic BR-145 callers remain unchanged.

Fresh validation evidence is required before any completion claim: focused unit/integration suites, `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --workspace --all-features`, and `bash tools/compliance/check.sh`. A currently busy shared Cargo window is not success evidence.

## 11. Current-code evidence captured at Gate A

The following read-only inspections established the defects before implementation:

```bash
nl -ba src/bin/monitor/news_aggregator_init.rs | sed -n '205,340p'
nl -ba src/bin/monitor/news_aggregator_init.rs | sed -n '365,490p'
nl -ba src/bin/monitor/main.rs | sed -n '6880,6980p'
nl -ba src/bin/monitor/notify.rs | sed -n '2240,2325p'
```

They showed eager `seen_events`/daily-count/`window_fired` mutation, log-only N-01/N-02 failure handling, selection before public dispatch, a generic evidence-losing N02 call, and generic delivery settlement that is not the required NewsFlash transaction authority.
