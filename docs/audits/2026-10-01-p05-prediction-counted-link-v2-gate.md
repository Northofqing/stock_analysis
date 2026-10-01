# P-05 prediction row → counted occurrence: v2 cutover gate

This follows the [identity seam](2026-10-01-p05-prediction-delivery-link-seam.md). It is an implementation gate, not a delivered-sample result. No production database was changed; implementation tests use isolated `TEST_CODE` databases.

## Boundaries recorded before the v2 development slices

- `src/bin/monitor/push_templates.rs:10345` (`dispatch_candidate_board`) saves Strong rows before it captures `hhmm` and builds the counted binding at lines 10421–10443. A retry in the same minute can save new rows and render different card bytes.
- `src/monitor/prediction_samples.rs:44,97` saves samples one at a time. `src/database/mod.rs:4149` returns each actual inserted ID only after its SQLite insert transaction commits. A worker failure can leave committed rows whose IDs are unknown to the caller.
- `src/bin/monitor/push_templates.rs:7080` builds `candidate-board-v1` from business date and rendered SHA-256 only. Row IDs are absent. Since source hash participates in the decision identity, adding retry-created IDs to a newly built binding would create another decision identity.
- `src/durable_delivery/coordinator_candidate_board.rs:110,209` reads terminal card evidence in one durable SQLite snapshot but accepts only the three-field v1 source. The returned card observation has no prediction-row membership. `src/durable_delivery/schema.rs:155` keys decisions by decision identity; it has no unique key for P-05 occurrence. `inspect_exact_occurrence_owner` in `src/durable_delivery/coordinator.rs:3072` is a read, not an atomic owner claim.
- `src/database/mod.rs:4439,4635` defines the existing promotion and verified-sample denominators from prediction rows. Neither query uses delivery state. Keep both queries and their labels unchanged.

## Why a direct link is unsafe

Joining by date and stock code, or attaching today's saved IDs to a v1 card after send, can mark a never-delivered row as delivered. An old v1 decision with the same occurrence and card SHA must stay **Unlinked** even after this feature is deployed. The current `PushOutcome::is_pushed()` is a dispatch result, not a row-level receipt. A new link therefore needs a versioned source identity and a durable freeze *before* counted admission. This changes the P-05 producer, prediction storage, and counted reader together; a local getter or post-send table insert is insufficient.

## Minimum safe v2 sequence

1. Capture one canonical business date and `HH:MM` occurrence before Strong sample persistence. Keep existing prediction-row creation and both existing statistical queries; later retry rows remain ordinary samples, never members of the first occurrence link.
2. Obtain the committed IDs from the save report. If any Strong row save is failed or unknown, stop before counted send. In one prediction-DB write transaction, verify each proposed ID's date, code, direction, `candidate-strong` marker and expected target date, then freeze ordered `(row ID, code)` membership, exact rendered bytes/SHA-256, exact occurrence, and canonical `candidate-board-v2` source bytes/SHA-256. A unique occurrence key makes this a first-writer-wins insert, while a unique prediction-row key prevents the same sample belonging to another occurrence. Return a typed frozen record only after commit; any error, ambiguous commit, or inconsistent existing record forbids send.

   The freeze stores the whole checked-in calendar hash as **creation-time provenance** and the exact T0..T+5 trading-date vector. A read recomputes that vector against the currently checked-in calendar. Extending another year or editing calendar comments does not invalidate an old freeze; changing a relevant trading date does. If old calendar coverage is removed, the read fails closed. Independent replay against the original calendar authority would require retaining its versioned source bytes; this frozen record does not provide that historical source by itself.
3. For a same-occurrence retry, read the existing frozen record. Never replace its member IDs, rendered bytes, source bytes, or resulting decision identity with newly saved rows. If the newly rendered card differs, fail closed for that occurrence. If it matches, dispatch only the persisted v2 bytes. Concurrent losers must use the winning frozen record or stop.
4. Extend the counted reader to validate both schemas. V1 stays card-level and **Unlinked**. V2 must validate occurrence, rendered hash, canonical bytes, and the structure of its ordered row IDs; the durable DB alone cannot prove those rows exist in the prediction DB. Before admission, reject an existing v1 or conflicting v2 owner of that occurrence; the final owner rule must be enforced atomically in durable admission, because the current read-only owner inspection cannot close a concurrent old/new producer race.
5. Reconcile a separate delivered-sample view only when the prediction-DB freeze still verifies actual row IDs and the durable observation for the *same occurrence, source hash, rendered hash, and derived decision identity* is terminal `Accepted` with its authoritative receipt. `ManualAccepted`, pending, rejected and uncertain remain separate. Never rewrite historical prediction rows or use this view as the existing sample-hit denominator.

## Crash windows to test

| Last completed step | Safe result |
| --- | --- |
| Some row inserts; no freeze commit | Rows remain old-style samples. No counted send and no delivered link. |
| Freeze commit; no counted admission | Persisted v2 link exists but is Pending/Unlinked to delivery. Exact retry is possible. |
| Counted admission/sink; producer crashes before observing result | Reconcile the same frozen v2 source against the durable terminal read. Do not resend from a fresh card. |
| Counted terminal is `ManualAccepted` or nonaccepted | Do not count physical delivery. |
| Existing v1 occurrence during cutover | Never graft new row IDs onto that decision; fail closed until explicit owner handling is implemented. |

## Development status and remaining cutover

- The prediction DB now persists the first occurrence's exact card, canonical source and ordered `(row ID, code)` members. It rechecks actual rows on read. IDs must be positive and unique; card order does not imply SQLite ID order. This remains producer evidence only.
- Durable schema v11 adds a `CandidateBoard` exact-occurrence insert trigger. Counted `prepare` checks the owner in the same `BEGIN IMMEDIATE` transaction. A repeated identical decision resumes its stored state only while it is the sole owner; a new decision identity for that occurrence is rejected across v1/v2. The trigger also rejects direct `INSERT OR IGNORE/REPLACE` conflicts and absent occurrence text. A v10 database with historical duplicate owners is retained, but reads and retries return an error instead of selecting one.
- The counted read accepts strict canonical v1 and v2 sources in one snapshot. V1 is explicitly `UnlinkedV1`. V2 is `DeclaredV2`: its ordered IDs are only the durable source's claim. This read does **not** verify prediction-DB rows or claim that any row was delivered.
- The dispatcher still uses v1 and saves rows before fixing the occurrence. It must next capture the occurrence before saving, require the verified prediction-DB freeze before counted admission, and replay only persisted bytes. A delivered-sample view then needs a separate cross-DB join with terminal `Accepted` receipt and crash-window tests. Existing sample-hit and promotion denominators remain unchanged.
