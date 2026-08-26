# Global Selection DB/Audit Owner — Gate-B Slice

## Scope and authority

This slice implements BR-180 amendment §4.1 behind
`GlobalSchemaVersionOwner::inspect_selection_with_audit`. The migration binary is
only a rendered-diagnostic client. It cannot choose paths, acquire locks,
construct catalog evidence, retain SQLite handles, or receive migration
authority.

The owner acquires and releases evidence in this order (BR-189):

1. fixed mode-bound global process/OS exclusive lease;
2. no-follow database pin and an exact initial-absence gate for WAL, SHM and
   rollback-journal sidecars;
3. retained-parent SQLite open plus a non-authoritative schema probe that
   materializes the owner's WAL/SHM, followed by exact descriptor/identity
   pins for that pair and a second no-journal check;
4. exclusive `LockedSelectionAuditSession`, then optional audit pin;
5. descriptor-opened SQLite `BEGIN IMMEDIATE`;
6. exact catalog, dependency, PRAGMA, integrity, audit high-water and twelve
   table-count capture;
7. repository-owned exact envelope/manifest/receipt/domain-row ↔ audit
   reconciliation through the same retained transaction;
8. exact recapture plus path/object-identity and owner-sidecar revalidation;
9. SQLite transaction/connection finish, then retained-parent removal (or
   confirmation that SQLite already removed) of only the exact owner-pinned
   WAL/SHM, directory sync and final absence/no-journal recheck;
10. capability/diagnostic issuance. For a verified final database the global
    lease and pinned database/audit objects move into the issued capability;
    diagnostic states release them only after sidecar cleanup succeeds.

The audit session starts only after owner sidecar materialization and covers
every authoritative database/audit read. Its namespace-container mutation
marker is never refreshed or bypassed. Consequently a rename, replacement or
same-inode ABA of the audit namespace during the authoritative snapshot still
fails closed, while the owner's earlier WAL/SHM entry creation cannot be
misclassified as audit mutation. The schema probe before the audit lock is not
authoritative and grants no database/audit conclusion.

`CatalogSnapshot` and `VerifiedSelectionSchemaSnapshot` are private,
non-`Clone` values. Only the global owner can issue the catalog capture token.
Detached DTOs are diagnostic-only. The opaque, non-`Clone`
`VerifiedAmendedSelectionSchema` retains exclusive authority and pinned
objects for its full lifetime.

## Descriptor-bound operational pool

An amended capability may be consumed only by the crate-private
`DatabaseManager::from_verified_amended_selection_schema` constructor. The
constructor accepts no path and performs no DDL. It clones the capability's
already pinned database and parent-directory descriptors into a private r2d2
manager. Linux connections use the retained-parent route
`/proc/self/fd/<parent-fd>/<fixed-leaf>`. macOS connections obtain the current
kernel path of the retained parent with `fcntl(F_GETPATH)` and append only the
owner-fixed leaf. This is the platform equivalent required because macOS does
not permit traversal through `/dev/fd/<directory-fd>/<leaf>`.

The owner's exclusive same-snapshot inspection uses this same internal
retained-parent route instead of `/dev/fd/<database-fd>`. The route is derived
only from the already pinned parent descriptor plus the owner-fixed leaf; no
caller path, environment value, CWD, canonical-path lookup or ordinary-path
fallback is accepted. The database leaf is opened with `openat(O_NOFOLLOW)`
before SQLite starts, its device/inode must match the retained descriptor both
immediately before and after SQLite opens, and the existing final snapshot
revalidation remains mandatory before authority can be issued. This lets the
native macOS VFS place locking/WAL/journal state beside the pinned database
while preserving the owner's inode authority.

Every new connection and every pool checkout must read `PRAGMA database_list`,
require the `main` entry to be the descriptor-derived route, validate that
route against the owner-captured device/inode, and prove that SQLite opened a
new main-database file descriptor with that same identity. A caller path,
canonicalized path, environment value, CWD, or a matching filename is not
binding evidence. Retaining the parent descriptor also makes WAL/SHM and
rollback-journal sidecars resolve beside the pinned database after a namespace
rename instead of under `/dev/fd`.

The first connection must likewise prove the exact WAL and SHM descriptors in
the process-fd delta. After that proof, the pool retains a separate SHM
lifetime anchor opened descriptor-relative from the already retained parent
with `openat(O_NOFOLLOW)` and rejects it unless its file-object identity is
exactly the attested SQLite SHM identity. It must not `dup` SQLite's own SHM
descriptor: macOS may guard that descriptor and terminate the process on
duplication. The separate anchor is not evidence that SQLite opened SHM—the
exact fd delta remains that evidence—and later connections may reuse it only
after both its identity and the current main/WAL/SHM leaf set are revalidated.

`DatabaseManager` declares the pool before the amended capability so all
pooled SQLite connections drop before the capability releases its
database/audit pins and GlobalSchema lease.

The Gate B regression exercises WAL creation, simultaneous pooled connections,
pool reopen, parent-directory rename plus replacement, actual SQLite main-fd
identity, and pool-before-capability drop order. Gate D still requires the
normal release validation commands, but there is no deferred file-fd sidecar
design gap.

## State matrix

| Database half | Audit object/chain | Owner result |
| --- | --- | --- |
| any exact database half | audit object missing | `DatabaseHalfOnly` |
| empty `0/0` catalog | present valid chain, no v2 phase | `Absent` |
| exact historical catalog | present valid historical chain, no v2 phase | `PreAmendment` |
| exact four-payload catalog | present valid chain with v2 phase | `TransitionalIncomplete` |
| exact five-payload `STSA/1` database | present valid chain with v2 phase and exact same-snapshot receipt closure | opaque `VerifiedAmendedSelectionSchema` |
| absent/historical database with v2 audit, or current/final database without v2 audit | contradictory; fail closed |

`AmendedReceiptVerificationPending` is an internal preliminary classification
only. It is never returned as authoritative evidence. The owner exchanges it
for `VerifiedAmendedSelectionSchema` only after repository reconciliation and
all final recaptures succeed; any mismatch is an explicit fail-closed error.

The concrete boundary is larger than the receipt table. The verifier must
rebuild and rehash the envelope, canonical payload, actual typed domain rows,
staged-db preimage, manifest, receipt and exact Prepared/Committed audit
records in both directions. The current repository verifier already has those
semantics. Its remaining coupling is only the Diesel row-loading layer.

The implemented interface belongs in `selection_v2_repository`, not in the
global owner:

```text
ExactSelectionSnapshotReader
    -> subject ids
    -> envelope / manifest / receipt rows
    -> typed ingress / generation / outcome rows
    -> outcome authority row

verify_database_and_audit_with_reader(reader, locked_audit_session)
    -> ValidatedAuditChainSnapshot
```

The existing Diesel implementation remains one adapter. The rusqlite adapter
borrows the owner's already-open `&rusqlite::Transaction` and exposes no
path/open/write operation. `global_schema_v1` calls only the repository
verifier and receives the validated audit high-water; it duplicates no table
SQL, canonical preimage, or hash logic.

The focused matrix combines existing repository reconciliation tests with the
new adapter contracts: Diesel/rusqlite parity on the same exact final empty
snapshot; rusqlite envelope-column/hash tampering; typed generation/outcome
domain-column tampering with copied hashes; orphan Prepared audit evidence;
receipt/manifest and Committed-without-receipt checks in the shared verifier;
outcome lineage mismatch; and direct calls proving the rusqlite entry point
accepts only the caller-owned transaction borrow and audit session.

Opening a second production connection, verifying a raw database copy, or
trusting stored row hashes is rejected: none proves that receipt closure was
checked against the same retained SQLite snapshot.

## Missing audit handling

The locked audit session treats a missing data file as an empty chain, while
the owner separately records whether the audit object exists. A missing object
is rechecked as absent through the same pinned parent before return. A present
object retains its descriptor and device/inode identity through revalidation.
The lock file is not treated as audit data.

## CLI and TEST_CODE rehearsal

`migrate_selection_v2` delegates all work to the library façade.

- Default mode runs the fixed production diagnostic.
- Production `--apply` fails before owner/database/audit I/O.
- `--test` asks the owner to create an invocation-isolated `TEST_CODE_`
  temporary root with an OS-random owner nonce. Root creation, database/audit
  copy and inspection hand-off use retained directory descriptors plus
  no-follow `mkdirat/openat(O_EXCL)`. The owner copies the pinned production
  database and optional audit while holding the production
  global/audit/SQLite snapshot and inspects only that copy.
- Successful test output is withheld until the outcome/capability is dropped
  and explicit `finish()` cleanup succeeds. `Drop` is a loud error-logging
  fallback only; cleanup failure cannot be reported as a successful rehearsal.
- `--test --apply` remains a no-mutation rehearsal; it cannot authorize or
  modify production.
- No root, database, audit, lock, backup, or output path override exists.

## Failure modes and rollback

- Catalog, runtime, identity, attached-schema, integrity, FK, row-count, audit
  prefix or pinned-object drift returns an explicit error.
- BR-184 descriptor snapshots may ignore only unrelated descriptor numbers
  that disappear with `ENOENT` or `EBADF` between `/proc/self/fd` or
  `/dev/fd` enumeration and inspection. The pre-pinned SQLite object set must
  still appear as an exact three-object delta; missing, duplicate or ambiguous
  main/WAL/SHM evidence fails closed. Once a descriptor is attested, later
  close, reuse or identity drift remains an explicit identity error.
- A pool-lifetime SHM anchor may be opened only from the retained parent
  descriptor with no-follow semantics after exact SQLite SHM fd admission; a
  leaf identity mismatch or later anchor drift fails closed. Direct duplication
  of SQLite's own SHM descriptor is prohibited on macOS.
- BR-185 TEST_CODE audit roots use the retained directory descriptor as
  authority; the supplied path is diagnostic-only. A rename/replacement of
  that diagnostic path cannot redirect the session. Revalidation opens the
  same namespace relative to the retained root, requires its exact identity,
  verifies the root marker is unchanged across that check, and separately
  verifies namespace/audit/lock identities and the hash chain. It deliberately
  does not compare the root's current ctime with the construction-time marker,
  because renaming the same pinned inode changes ctime without changing
  authority.
- BR-186 catalog inspection runs connection-initializing integrity probes
  before freezing its first catalog snapshot. `PRAGMA database_list` must then
  contain `main` and may additionally contain only SQLite's built-in,
  connection-local `temp` schema. `temp` is not an external authority and
  cannot replace `main`; any other attached schema, duplicate or missing
  `main` remains an explicit `AttachedSchemaMismatch`.
- WAL/SHM/journal present before the owner starts fail closed and are never
  auto-cleaned. During inspection only the exact owner-created, descriptor
  pinned WAL/SHM pair is legal; an extra journal, identity drift or unexpected
  disappearance fails closed. After the connection closes, cleanup is scoped
  through the retained parent and may remove only that exact pair. Failure to
  delete, sync or prove final absence blocks capability issuance. A crash may
  leave sidecars; the next startup treats them as unknown pre-existing
  evidence and fails closed rather than cleaning them.
- Audit-v2 plus database-absent and other contradictory halves fail closed.
- Temporary copy failure removes only the exact owner-issued `TEST_CODE_`
  directory. Production database/audit contents are never modified.
- Rollback is a source revert of the owner/catalog/facade files; no data
  rollback is required.

## Static validation for this slice

- `rustfmt` on changed Rust files.
- `git diff --check` on changed paths.
- production-mutation scan for DML/DDL/PRAGMA writers.
- old CLI ownership scan confirming the binary has no Diesel, filesystem,
  audit parser, lock or descriptor implementation.

Cargo and live-data checks remain Gate C/D work and must not be claimed by this
static parallel slice.
