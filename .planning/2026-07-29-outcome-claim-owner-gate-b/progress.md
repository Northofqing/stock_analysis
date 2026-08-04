# Progress

- 2026-07-29: completed read-only preflight and architecture inventory.
- 2026-07-29: root granted exclusive ownership of claim-related repository files.
- 2026-07-29: implemented the typed `outcome_claim` schema, audit phases,
  repository/persistence lifecycle, exact claim-to-outcome receipt binding, and
  zero-provider-attempt `expected_wait` semantics.
- 2026-07-29: added receipt/audit/database-bound due materialization, strict
  exact-payload claim recovery, retained fixed-path per-subject OS-lock seams,
  and the closed `Receipted | LiveOwnedSkip | Superseded` owner result algebra.
- 2026-07-29: wired the production `settle_due` owner in the required order:
  subject lock -> fresh exact due revalidation -> durable claim receipt ->
  market-session/provider evaluation -> durable outcome receipt. Claim due and
  semantic request hashes are rechecked before the preallocated outcome run is
  accepted.
- 2026-07-29: scoped Rust formatting and whitespace validation pass. The last
  coordinated `cargo check --lib` attempt reached an unrelated
  `opportunity/chain_mapper.rs` type error; Cargo validation is paused while
  that lane owns the serialized build window.
- 2026-07-29: completed an independent static self-review, hardened the
  subject-lock leaf with `O_NOFOLLOW` plus post-open dev/inode equality, and
  recorded the exact focused-test inventory and seven remaining blockers in
  `self_review.md`.
- 2026-07-29: documented the required global-owner descriptor-pinned,
  query-only SQLite snapshot seam in
  `pinned_read_only_db_binding_contract.md` without editing either global
  schema source file.
- 2026-07-29: added the shared receipted-claim lifecycle classifier
  (`ClaimPartial | ClaimActive | OutcomeRecovery | Closed`) and made both the
  claim persistence guard and due anti-join consume it. Exact claim/planned-run
  binding, mixed artifacts, cross-claim artifacts, multiple open claims and an
  open non-latest claim all fail closed.
- 2026-07-29: added the eight frozen recovery behavior tests and the production
  recovery-first coordinator. `ClaimActive` reuses the exact claim receipt and
  planned outcome run, while `OutcomeRecovery` replays the durable outcome
  envelope without provider acquisition. The post-session scheduler now calls
  the v2 coordinator before the legacy shadow observation path.
- 2026-07-29: added a distinct receipt/audit-verified persistence-recovery read
  purpose so a crash after Committed audit but before receipt can enter exact
  recovery without weakening ordinary authoritative reads.
- 2026-07-29: scoped `rustfmt --check` passes after the recovery edits. Per
  dispatcher instruction, no Cargo command has been run for this latest slice.
- 2026-07-29: completed the ExpectedWait deadline/API slice. One strict
  `DateTime<FixedOffset>` `+08:00` tick now flows from the monitor scheduler
  through verified due reads, locked due revalidation, claim UUID/timestamp
  construction, market-session gating and Magic TDX admission. A latest
  receipted ExpectedWait is suppressed before
  `15:00:00.000000001 +08:00`, becomes due exactly at that instant, and its
  receipt remains in the next claim's same-subject lineage.
- 2026-07-29: added deadline boundary, restart stability, serial dual-owner,
  wrong-offset and prior-wait-receipt lineage tests. Scoped `rustfmt --check`
  and `git diff --check` pass; no Cargo command was run per dispatcher
  instruction.
- 2026-07-29: no provider I/O, production DB write, or migration apply was
  performed.
