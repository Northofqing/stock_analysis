# Findings

- `migrate_schema_v4_to_v5` creates `immutable_audit_outbox_v5` with its
  predecessor FK aimed at the old `immutable_audit_outbox`, then drops that
  referenced table. A non-NULL predecessor chain therefore fails the drop.
- `migrate_schema_v3_to_v4` has the same rename-table self-FK shape and needs
  the corresponding final-table reference.
- The existing v4 fixture seeds only NULL predecessor values and does not prove
  linked-chain preservation or rollback on corrupt historical FK data.
- Frozen design §4.4.1 permits six Failed replay reasons. The implementation
  added `terminal_replay_classification_failed`; classification infrastructure
  errors semantically belong to `terminal_replay_evidence_unavailable`.
- The compliance checker currently requires the contradictory seventh reason.
  The release verifier validates only Passed replay rows and does not reject
  out-of-contract historical completion reasons.
- The frozen six-reason vocabulary is now enforced in the coordinator and the
  classification-error path persists `terminal_replay_evidence_unavailable`.
  The production verifier scans every historical completion before selecting
  the requested latest attempt, so an old seventh reason fails closed.
- Focused strict Clippy exposed only the pre-existing v4→v5 migration tuple
  complexity; a local type alias removes that compile gate without changing
  SQL or migration semantics.
