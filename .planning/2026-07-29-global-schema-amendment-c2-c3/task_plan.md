# Global Schema Amendment C2/C3 Gate-B

## Goal

Implement amendment §4.1 behind the deep
`GlobalSchemaVersionOwner::inspect_selection_with_audit` interface. The owner alone must
acquire and retain the complete lock/pinning/transaction evidence needed to issue a
diagnostic selection snapshot. Production apply remains fail closed; only a
`TEST_CODE` temporary-copy rehearsal may consume owner-issued evidence.

## Scope

- `src/database/global_schema_v1.rs`
- `src/database/global_schema_catalog_v1.rs`
- selection audit locking seam, only as required by the owner
- `src/bin/migrate_selection_v2.rs`
- focused contract tests and the amendment design record
- never write `data/stock_analysis.db`

## Required ordering and evidence

1. Global exclusive authority.
2. Selection audit exclusive lock and full-chain high-water validation.
3. Pinned database identity.
4. `BEGIN IMMEDIATE`.
5. Catalog, 12-table row counts, integrity/FK, audit high-water and pinned-identity capture.
6. Revalidate every retained identity before returning.
7. Database-only evidence is diagnostic and never authorizes migration.

## Phases

### Phase 1 — Gate A recovery and interface inventory

Status: completed

- Locate amendment §4.1, BR-180/BR-182, existing owner, catalog and audit interfaces.
- Record old-module adopt/reject decisions and failure modes.
- Define the smallest owner interface and observable behaviors.

### Phase 2 — TDD tracer: owner-issued diagnostic snapshot

Status: completed

- Add one focused owner-interface contract test.
- Record RED statically while Cargo is intentionally blocked by parallel work.
- Implement only enough owner logic to satisfy the first behavior.

### Phase 3 — Incremental evidence and lock-order contracts

Status: completed

- Add row-count, integrity/FK, audit high-water, pinned-identity and revalidation behavior.
- Make `CatalogSnapshot` private-field, non-`Clone`, owner-issued only.
- Keep database-only state diagnostic.

### Phase 4 — TEST_CODE temporary-copy rehearsal

Status: completed

- Route migration CLI through the owner.
- Permit only owner-issued isolated TEST_CODE copy rehearsal.
- Keep production `--apply` explicitly fail closed.

### Phase 5 — Exact receipt reconciliation and capability

Status: completed

- Generalize the full repository verifier behind a private exact-snapshot
  reader while retaining the Diesel adapter.
- Add a rusqlite adapter that borrows only the owner's retained transaction.
- Issue `VerifiedAmendedSelectionSchema` only after exact closure and final
  owner revalidation.
- Add focused parity, tamper, fail-closed, and lock-lifetime tests.

### Phase 6 — Static verification and handoff

Status: completed

- `rustfmt --check`, `git diff --check`, forbidden-production-write scans.
- Cargo remains unrun until the root explicitly releases the shared artifact lane.
- Report remaining Gate C/D blockers without claiming completion.

## Validation

- Targeted owner and CLI exact tests after release.
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-targets --all-features -- --test-threads=1`
- `bash tools/compliance/check.sh`

## Rollback

Revert owner, snapshot, audit-adapter, CLI and test changes independently. Remove only
TEST_CODE temporary rehearsal artifacts. Never delete or rewrite production database/audit
evidence.

## Errors encountered

| Error | Attempt | Resolution |
|---|---:|---|
| None yet | 0 | — |
