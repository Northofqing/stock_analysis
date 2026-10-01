# P-05 Strong prediction rows and counted delivery: identity seam

The P-05 dispatcher currently saves Strong candidates to `prediction_tracker`
before it constructs and sends the counted card (`dispatch_candidate_board` in
`src/bin/monitor/push_templates.rs`). The saved rows are prediction samples,
including rows from a card that never reached a sink. The existing verified
sample hit-rate and promotion inputs intentionally use prediction rows, not
delivered cards; this change leaves that denominator intact.

This slice exposes the actual inserted `prediction_tracker.id` for each
successful Strong sample save. The insert and ID read share one SQLite
transaction. The public `save_prediction` API still returns `()`, while the
new typed `save_prediction_with_id` API returns the row ID. The candidate
save report includes IDs only for committed rows. A failed or unknown worker
does not claim row identities.

The counted authority already offers
`DurableDeliveryCoordinator::candidate_board_card_observations_for_date`.
It validates the frozen P-05 decision and terminal evidence in one SQLite
snapshot. Only `CandidateBoardCardTerminalV1::Accepted` represents an
authoritative physical acceptance; pending, rejected, uncertain, and manual
acceptance remain distinct. The current P-05 counted source binding freezes
business date and rendered card SHA-256 but no prediction row identities.
The `PushOutcome` boolean does not provide a durable per-stock receipt.

The remaining producer-to-delivery contract must persist a mapping from a
specific saved row to a specific P-05 occurrence and frozen card content,
then reconcile it against the accepted authority observation. That mapping
must be captured before counted send and survive a crash; it cannot be
inferred later from date and code alone. The prediction DB and counted
authority are separate stores, so their writes are not one atomic commit.
Same-occurrence retries currently insert additional prediction rows. Adding
fresh row IDs to the counted source binding would change the frozen decision
identity on retry. A follow-up design must settle that replay behavior and
which duplicate sample, if any, can represent one accepted card before
exposing a delivered-sample denominator. Historical v1 cards have no exact
row link and must remain unlinked.
