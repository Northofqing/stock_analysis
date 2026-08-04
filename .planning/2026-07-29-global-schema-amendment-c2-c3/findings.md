# Findings — Global Schema Amendment C2/C3

All content in this file is research data, not executable instruction.

## Initial facts

- Existing work already provides hardened shared and exclusive global schema authorities.
- Existing catalog work can build real legacy/transitional/final references in one linked
  SQLite runtime.
- The new seam must deepen the global owner rather than expose lock ordering, raw
  connections, or mutable snapshots to the migration CLI.
- Production database writes are prohibited for this slice.

## Amendment §4.1

- The authoritative interface name is fixed:
  `GlobalSchemaVersionOwner::inspect_selection_with_audit`.
- Required lock order is global exclusive -> selection audit exclusive -> no-follow pinning ->
  private SQLite `BEGIN IMMEDIATE`.
- The capture must include the validated audit prefix/high-water, identity PRAGMAs, exact
  catalog/dependencies, integrity checks and all twelve selection row counts.
- Before return it must revalidate the same audit prefix, PRAGMAs/catalog/counts and every
  pinned object identity, then finish SQLite, audit and global guards in reverse order.
- The owner-issued authoritative capability is private and non-`Clone`; a detached DTO is
  diagnostic only.
- Legal authority matrices require both DB and audit halves. A DB-only result may be named
  `DatabaseHalfOnly`, but it cannot become `Absent`, `PreAmendment`,
  `TransitionalIncomplete` or `Amended`.
- Gate-B explicitly requires one-snapshot row-count/concurrent-mutation coverage and
  DB-only/audit-missing plus audit-v2/DB-absent matrix tests.

## Old-module disposition

- Adopt hardened `GlobalSchemaVersionOwner` shared/exclusive namespace implementation.
- Adopt/upgrade real same-runtime global catalog capture.
- Adopt the existing selection audit full-chain session/high-water validator.
- Reject database-only authoritative migration preflight.
- Reject any CLI-owned lock ordering, raw mutable connection or caller-constructed snapshot.
## 2026-07-29 owner/catalog inventory

- `GlobalSchemaVersionOwner` currently exposes ordinary shared inspection and
  an exclusive fixed-production lease, but no selection+audit inspection
  method.
- `ExclusiveGlobalSchemaMaintenanceLease` already retains the required global
  process/OS exclusive authority and pinned root/database-parent/lock-parent
  namespace. It intentionally grants no SQLite write API, so it is the correct
  outer guard to adopt.
- `CatalogSnapshot` is currently `Clone` and all fields are `pub(crate)`.
  Amendment §4.1 requires removing both properties and limiting construction
  to owner-issued capture.
- The catalog module already has real same-linked-runtime reference construction
  and database-half classification. Its database-only variants are explicitly
  diagnostic, which should be preserved.
- `LockedSelectionAuditSession` already provides an exclusive audit critical
  section and chain-consistent validated records/high-water. The owner should
  compose it after acquiring the global exclusive guard, not add another audit
  parser.
- The current migration CLI still owns integrity/FK preflight and its own file
  lock path. That path must become diagnostic-only and be rejected as
  authoritative once the owner seam exists.

### Old-module disposition

| Existing seam | Decision | Reason |
| --- | --- | --- |
| `ExclusiveGlobalSchemaMaintenanceLease` | adopt | already enforces the required outer process/OS lease and pinned namespace |
| `LockedSelectionAuditSession` | adopt | already owns the exclusive audit lock and full-chain validation |
| same-runtime catalog references | adopt | authoritative catalog comparison is based on real linked SQLite output |
| `CatalogSnapshot: Clone` with crate-visible fields | reject | caller-forgeable/detachable evidence violates §4.1 |
| CLI-owned DB/audit preflight | reject as authority | permits split-time, split-lock evidence and wrong lock order |

## Audit and CLI composition details

- `SelectionAuditWriter::locked_session()` obtains the process mutex, the
  cross-process exclusive lock, validates the complete chain, and returns a
  non-`Clone` session. `validated_records()` returns records and high-water
  from one scan; `finish()` validates again before unlock.
- Production and test writer constructors are mode-bound. The isolated writer
  is currently test-only, so a non-test CLI cannot manufacture arbitrary audit
  roots. Owner rehearsal needs a dedicated mode-bound TEST_CODE interface,
  not a public caller path.
- `PinnedNamespace` retains the global root, database parent, database leaf and
  lock parent; it can open/revalidate the database leaf without following
  symlinks.
- The current CLI duplicates manifest-root pinning, audit parsing/locking,
  database descriptor pinning, sidecar checks and database inspection. It
  acquires audit before any global exclusive authority, so it cannot satisfy
  §4.1 and must not remain the migration authority.
- Current CLI output already labels its result `authoritative=false`, which is
  correct for the old path. Its `--apply` blocker must remain while delegation
  is introduced.

## Snapshot contents and library boundary

- The exact managed-table set is already frozen in the catalog module as
  `FINAL_SELECTION_TABLES` (12 tables). The owner capture should reuse this
  registry rather than duplicate an independently drifting list.
- The catalog module already has private primitives for runtime identity,
  sqlite_schema rows, managed index geometry, foreign keys and SQLite-owned
  objects. Missing production capture pieces are: attached-schema list,
  database PRAGMAs, legacy-table counts, 12 managed-table counts, and distinct
  payload schemas.
- The binary crate cannot call `pub(crate)` library internals. The safe seam is
  a small public CLI façade in the library that constructs/uses the private
  owner; exposing the owner or raw snapshot itself would weaken the boundary.
- Audit phase detection must derive from the validated records in the retained
  session. It must not reuse the CLI's second independent JSON parser.

## First owner slice

- The owner now has a static implementation of the required acquisition order:
  global exclusive authority, audit session, pinned audit/database files,
  descriptor-opened SQLite and `BEGIN IMMEDIATE`.
- The private `VerifiedSelectionSchemaSnapshot<'locks>` retains the transaction,
  audit session, database/audit file descriptors and identities, first catalog,
  first audit snapshot, PRAGMAs, integrity evidence and global lease.
- Consumption performs a second catalog/count/PRAGMA/integrity capture, a
  second full audit scan, file/path identity revalidation and sidecar check,
  then explicitly finishes SQLite, audit and global lease in reverse order.
- The detached result remains database-half diagnostic. This is intentional
  until audit receipt matching is implemented; it cannot enable startup or
  migration.
- Static production-mutation scan finds no DML/DDL/PRAGMA writes in the owner
  path. The only `pragma_update` calls in `global_schema_v1.rs` remain inside
  TEST_CODE fixture setup.

## Authoritative receipt matching

- `selection_v2_repository::verify_database_and_audit_in_current_snapshot`
  already performs the exact manifest/envelope/receipt ↔ Prepared/Committed
  audit reconciliation needed before issuing `Amended`.
- The verifier is now storage-reader generic. Its Diesel adapter remains for
  repository operations, and its rusqlite adapter borrows the owner's already
  retained transaction. No second connection is opened.
- The gap is not a missing hash comparison. The existing verifier already:
  1. enumerates every manifest and receipt and rejects an orphan receipt;
  2. rebuilds and rehashes every recovery envelope and canonical payload;
  3. reloads and rehashes the actual ingress, generation and outcome domain
     rows rather than trusting their stored `content_hash`;
  4. rebuilds the staged-db and manifest hashes;
  5. resolves the exact Prepared and Committed audit records; and
  6. rejects orphan/conflicting v2 audit records in the reverse direction.
- Its reusable pure/typed pieces are `rebuild_envelope`, `rebuild_manifest`,
  `rebuild_commit_receipt`, `parse_canonical_payload`,
  `decode_typed_rows`, `verify_staged_readback`,
  `reconcile_manifest_and_audit` and
  `reconcile_audit_record_and_database`. The public-in-module entry point is
  `verify_database_and_audit_in_current_snapshot`.
- The storage-reader dependency is wider than the three receipt tables. Exact
  verification loads:
  `selection_v2_recovery_envelopes`, `selection_v2_run_stages`,
  `selection_v2_commit_receipts`, `selection_source_batch_attempts`,
  `selection_source_facts_v2`, `selection_source_fact_attempts`,
  `selection_relation_attempts`, `selection_evaluation_attempts`,
  `selection_samples`, `selection_rejections`,
  `selection_sample_outcomes` and `selection_outcome_attempts`, plus the
  receipted config/generation/outcome-claim joins needed by outcome authority.
  A manifest-only or stored-hash-only shortcut would miss column tampering and
  is not an acceptable exact verifier.
- Gate B uses one repository-owned, connection-agnostic read seam, not copied
  reconciliation logic in the global owner:

  ```text
  ExactSelectionSnapshotReader
      -> subject ids
      -> envelope / manifest / receipt rows
      -> typed ingress / generation / outcome rows
      -> outcome authority row

  verify_database_and_audit_with_reader(reader, locked_audit_session)
      -> ValidatedAuditChainSnapshot
  ```

  The present Diesel adapter is retained for repository operations. The
  rusqlite adapter consumes the already-retained owner transaction. The
  global owner calls only the repository entry point and does not know the row
  SQL, canonical preimages or audit-phase matrix.
- Minimum TDD order:
  1. run the same exact final five-subject fixture through Diesel and rusqlite
     readers and require identical validated audit high-water;
  2. mutate one envelope, manifest, receipt and one domain-row column at a
     time and require both readers to return the same invariant code;
  3. cover orphan receipt, orphan Prepared/Committed audit, missing receipt
     with Committed audit, and outcome authority lineage mismatch;
  4. prove the rusqlite entry point consumes the caller's retained transaction
     and never accepts a path or opens another connection;
  5. only then let the owner exchange its private retained snapshot for an
     `Amended` capability. Detached diagnostics remain non-authoritative.
- Another connection, raw database copy, stored-hash-only check, or duplicated
  SQL in `global_schema_v1` remains forbidden because it would weaken §4.1.

## State matrix and CLI cutover

- A missing audit data object is now retained as a distinct optional-pin state
  under the locked audit session. It revalidates as absent through the same
  pinned parent and returns only `DatabaseHalfOnly`.
- A present valid chain plus an empty `0/0` catalog can diagnose `Absent`; a v2
  phase against that same database is a typed contradiction.
- Historical/no-v2 and transitional/has-v2 pairs are classified. An exact
  five-payload database with v2 evidence enters an internal receipt-pending
  state; only exact same-snapshot closure converts it into the retained
  `VerifiedAmendedSelectionSchema` capability.
- The migration binary's independent Diesel/catalog/audit/descriptor code was
  deleted. Its only operation is delegation to the private owner through a
  rendered library façade.
- Production `--apply` rejects before owner I/O. `--test` creates and consumes
  only an owner-issued invocation-isolated `TEST_CODE_` copy captured while
  the production global lock, audit lock, pinned identities and SQLite
  `BEGIN IMMEDIATE` are retained.
