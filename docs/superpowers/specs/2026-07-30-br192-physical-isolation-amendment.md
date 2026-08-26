# BR-192 Physical Isolation Amendment

**Status:** Gate A amended for macOS WAL/OFD rebinding; Gate B repair and
independent review pending
**Parent:** `2026-07-29-provider-topn-ranking-gateway-design.md`
**Business rule:** BR-192
**Data red lines:** 2.5, 2.7, 2.8, 2.10

## 1. Scope

This amendment closes the physical namespace boundary for the counted-delivery
SQLite store, immutable audit append, and push-log artifacts. It does not change
delivery policy, budget, cooldown, payload, or provider semantics.

The three production roots are fixed below the compile-time repository manifest
root. Runtime CWD, environment path overrides, canonicalized caller paths, and
test-selected production paths are not authorities.

Test roots are invocation-unique `TEST_CODE` namespaces. Tests must never create,
link, replace, truncate, or delete a production main/WAL/SHM, immutable-audit, or
push-log object. The monitor-generated namespace component binds both the
process ID and a per-invocation timestamp nonce; a process ID alone is not an
invocation identity because operating systems may reuse it. Tests inspect and
clean only the exact namespace named by the monitor's binding evidence.

### 1.1 Superseded path-selection clauses

For the three BR-192 authorities in scope, this amendment explicitly supersedes
all earlier `RuntimeArtifactRoots`, caller-provided path, CWD-relative path,
`EVENT_AUDIT_DIR`, and `PUSH_LOG_DIR` selection or compatibility clauses,
including those in the selection-evidence closure and terminal-monitor
lifecycle drafts. Those names may remain only in negative tests that prove the
override is rejected. There is no transition period and no fallback: production
uses the compile-time manifest roots, while tests use the exact invocation-owned
`data/test/TEST_CODE*` roots.

## 2. Authority and Data Flow

### 2.1 Retained namespace chain

Each writer opens an authority from `/` and traverses the compile-time manifest
root and fixed namespace one component at a time with `openat(O_NOFOLLOW)`.
It retains every directory descriptor and its device, inode, type, owner, and
mode. It also retains the observed directory link-count baseline. Directory
link count is a mutation detector, not an immutable identity: a legitimate
child-directory create/remove can change it while the retained directory
remains the same inode. A changed baseline is accepted and refreshed only when
two consecutive complete `openat` chain rebinds observe the same link counts,
every retained and reopened device/inode/type/owner/mode still matches, and no
component changes during either pass. Unstable link counts fail closed. This
permits a legitimate `mkdir` without treating it as an ancestor swap, while an
ancestor identity or exact-child rebind still fails.

The complete chain is rebound and compared:

1. after construction;
2. before an operation;
3. before a database commit or file durability acknowledgement;
4. after the operation and before returning success.

Every file leaf must be regular, owned by the effective user, not group/world
writable, have one link, and retain its device/inode identity.

### 2.2 SQLite

SQLite opens through the fixed manifest-root path only after the full directory
chain and main leaf are pinned. Immediately after `Connection::open`, the exact
main descriptor delta and fixed leaf are attested before any PRAGMA, schema DDL,
or WAL priming mutation.

`PRAGMA journal_mode=WAL` is a controlled bootstrap transition. On macOS the
SQLite VFS may close and reopen the main database open-file description while
performing that transition even though the fixed leaf and device/inode identity
remain unchanged. Therefore the initial main OFD proof is authoritative only
for the pre-WAL bootstrap state. It must not be carried unchanged into the
fully bound coordinator.

After WAL priming, the main proof is rebound once under the bootstrap protocol
in §2.2.1, then exact main/WAL/SHM descriptor identities are attested. SQLite
descriptor numbers are observed and revalidated; they are not duplicated.
Process-shared SHM evidence must be typed and tied to a live, directly attested
connection generation. A normal file anchor is never represented as a SQLite
descriptor. Registry entries are weak/generation-bound and are removed when
their last live connection disappears.

Every connection operation uses one attested boundary:

1. full-chain and main/WAL/SHM pre-validation;
2. database operation;
3. transaction validation before commit;
4. commit or rollback;
5. full-chain and descriptor post-validation before returning.

Descriptor enumeration errors are propagated. Only a descriptor that disappears
during its metadata probe with `ENOENT`/`EBADF` may be ignored.

Bootstrap success has an additional final boundary. After the database parent
directory is synchronously persisted, the coordinator reacquires the global
attestation lease and revalidates the complete ancestor chain, fixed
main/WAL/SHM leaves, SQLite descriptor identities, owner-specific OFD markers,
live SHM connection ownership, and effective `foreign_keys=ON`,
`synchronous>=FULL`, `journal_mode=WAL` state. Failure at this final boundary
returns an error; a partially validated coordinator is never returned.

#### Concurrent close serialization (BR-206)

The bounded open helper accepts the caller's process-global attestation mutex guard as a
private proof parameter. The regression performs repeated concurrent open/use/drop rounds so
SQLite VFS descriptor reuse is exercised inside the committed test suite.

The process-global SQLite attestation mutex also covers coordinator teardown.
The live rusqlite connection and the retained main/WAL/SHM proof binding are
destroyed together while that mutex is held. This closes an FD-ABA window in
which one coordinator could snapshot another coordinator's descriptor, the
other coordinator could close it, and the first coordinator's new connection
could reuse the same descriptor number for the same inode. Such a before/after
pair is observationally identical and therefore cannot prove which connection
owns the descriptor.

Open, attested operations, and close are consequently one total order for
process-descriptor evidence. A poisoned mutex is still an error for open and
operations. Teardown cannot return an error, so it recovers the poisoned guard
only long enough to preserve serialization and close both retained resources;
it does not perform SQL, acknowledge durability, or create business data.
Exact inode, fixed-leaf, OFD, WAL/SHM, owner and repository-root validation
remain unchanged and fail closed.

SQLite's Unix VFS can retain a closed connection's main descriptor in a
per-process reuse pool while another connection to the inode remains live. A
subsequent `Connection::open` can consume that already-open descriptor, making
the process-FD snapshots identical even though coordinator teardown is fully
serialized. Bootstrap handles this case without treating identity equality as
ownership proof: it keeps the no-delta connection alive, executes no SQL or
PRAGMA on it, and retries the exact fixed-path open under the same mutex. The
attempt bound is the initial number of descriptors naming the exact main inode
plus one. Every no-delta connection remains live until one attempt introduces
exactly one new descriptor, so a finite reuse pool must be exhausted within
that evidence-derived bound. Only that uniquely attested connection proceeds;
the unproved connections are closed under the mutex. Exhaustion, snapshot
failure, or multiple candidates fails closed with no schema or business write.

### 2.2.1 Bootstrap-only WAL re-attestation and OFD rebinding

The coordinator constructor uses a private linear lifecycle:

```text
InitialMainAttested
  -> WalMaterializing
  -> MainReattested
  -> MainWalShmBound
  -> Operational
```

Only `InitialMainAttested` may create the non-cloneable
`WalMaterializing` permit. The permit is held while both the process-global
SQLite attestation/open lock and the private `Connection` mutex are held. It
cannot be returned, stored in the coordinator, or used by an ordinary database
operation. No pool, sink, provider, schema, policy, or business-write
capability exists before `MainWalShmBound`.

The exact transition is:

1. Revalidate the complete manifest-to-parent chain, retained main anchor,
   fixed main leaf, initial raw descriptor identity, and the initial
   owner-specific OFD marker. Capture the pre-WAL process-descriptor snapshot.
2. Execute only the fixed WAL materialization sequence: set
   `journal_mode=WAL`, require `synchronous>=FULL`, perform the rollback-only
   WAL/SHM priming transaction, and read back the effective values. Schema DDL,
   policy insertion and business rows are forbidden.
3. Without releasing either lock or running unrelated code, capture the
   post-WAL descriptor snapshot. Revalidate the full directory chain, retained
   main anchor and fixed main leaf against the original
   device/inode/type/owner/mode/link-count evidence. This one validation
   deliberately does not consult the old OFD marker because WAL
   materialization is the only operation authorized to replace that OFD.
4. Select exactly one post-WAL SQLite main descriptor:
   - if exactly one matching descriptor is newly observed, select it even when
     the original descriptor number still names the main inode; the marker
     acquisition in step 5 distinguishes the same OFD from an illicit retained
     distinct OFD;
   - if no new matching descriptor exists and the original descriptor number
     still names the exact main identity, select the original number; this
     covers both a retained OFD and a close/reopen that reused the number;
   - if no candidate remains, or if multiple new candidates exist, fail closed.
     Retained namespace anchors and descriptors unchanged from the pre-WAL
     snapshot are not new SQLite candidates.
5. Unconditionally install a new owner-specific OFD marker on the selected
   descriptor at the same deterministic role/owner range. Reinstallation is
   idempotent when SQLite retained the same OFD. When SQLite legitimately
   closed the old OFD, its old lock is gone and the selected new OFD can acquire
   the marker. If an old marked OFD was illicitly retained while a new
   descriptor became the candidate, the lock conflicts and bootstrap fails.
6. Fully validate the rebound main proof, then attest the exact WAL and SHM
   leaves/descriptors, require three distinct physical identities, verify the
   live SHM ownership proof, and construct `MainWalShmBound`. Only this final
   proof may authorize schema or policy transactions.

This is not a general recovery mechanism. After `MainWalShmBound`, marker loss,
descriptor-number reuse, identity drift, ambiguity, or any later
`journal_mode` mutation is an isolation failure. Runtime operations must never
re-attest or reinstall a marker. Recovery closes and discards the entire
coordinator and restarts the full constructor against the fixed authority.

Connection configuration is correspondingly split. The bootstrap-only
materializer owns the sole `journal_mode=WAL` transition. Post-binding
configuration may set `foreign_keys=ON` and read/verify `journal_mode=WAL`,
`synchronous>=FULL`, and `foreign_keys=ON`; it must not invoke the WAL
materializer or write `journal_mode` a second time.

#### Confirmed macOS evidence (2026-07-30)

The exact focused command was:

```bash
cargo test --lib durable_delivery::tests::policy_catalog_has_fifteen_kinds_and_eighteen_rows -- --exact --nocapture --test-threads=1
```

After the read/write OFD capability probe was corrected, the unannotated
failure was:

```text
IsolationViolation("SQLite main raw descriptor lost owner-specific OFD marker owner-TEST_CODE_BR192_CATALOG_40499_1-0123456789abcdef")
```

With separate context around the existing validations immediately before and
after `materialize_wal_capability`, the same command produced:

```text
IsolationViolation("SQLite main attestation failed after WAL materialization: test/live durable-delivery isolation violation: SQLite main raw descriptor lost owner-specific OFD marker owner-TEST_CODE_BR192_CATALOG_41161_1-0123456789abcdef")
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 2304 filtered out
```

The before-WAL validation returned `Ok`; otherwise the contextual message would
identify the before-WAL boundary. The command exited 101. Separate existence
checks confirmed the production main/WAL/SHM artifacts were absent both before
and after this isolated TEST_CODE run. This evidence authorizes only the
bootstrap transition above; it does not weaken runtime ABA protection.

Static call-site evidence:

```bash
rg -n -C 8 "materialize_wal_capability|configure_connection" \
  src/durable_delivery/schema.rs src/durable_delivery/coordinator.rs
```

At the observed revision this shows `materialize_wal_capability` at
`schema.rs:10-23`, `configure_connection` calling it again at
`schema.rs:26-29`, the controlled bootstrap call at `coordinator.rs:1959`, and
the post-binding `configure_connection` call at `coordinator.rs:1997`. The
second call is a Gate B defect: post-binding configuration must be
non-materializing as specified above.

### 2.2.2 Manual accepted-delivery audit acknowledgement

A human `Accepted` resolution creates two different immutable records: the
operator authorization and a `DeliveryAcceptedAudit`. The latter owns an
independent deterministic identity, canonical hash, `Pending/Appended` state,
and immutable append reference in SQLite. Reconciliation performs:

1. read the exact frozen `Pending` audit bytes;
2. append them through `ImmutableAppendPort`;
3. acknowledge only the same identity/hash with a one-row SQLite CAS;
4. leave the row `Pending` and retryable when append or CAS/commit fails.

`Delivered` is forbidden until the manual accepted audit CAS is `Appended` with
an immutable reference containing at least one byte other than ASCII space,
tab, LF, or CR. Every immutable append return is validated with that exact
predicate before its acknowledgement transaction. Before every state transition
to `Delivered`, including recovery from `AcceptedTaskTransitionPending`, the
coordinator joins the current decision, current manual disposition, and manual
resolution. It recomputes and exactly compares the external evidence hash,
optional receipt canonical hash, resolved timestamp, deterministic
accepted-audit identity, canonical bytes/hash, append state, and immutable
reference. A self-consistent accepted audit belonging to another current
disposition is invalid.

The evidence join that authorizes `Delivered` is executed again from the
current snapshot of the same `BEGIN IMMEDIATE` transaction that performs the
state compare-and-set, immediately before its `UPDATE`. Any earlier read is
only a progress hint and cannot authorize the transition. This applies equally
to decisions with and without task bindings; a current-disposition or evidence
change between the hint and the write must fail closed and leave the state
non-`Delivered`.

Durable-delivery schema v3 installs the same four-byte ASCII-whitespace
predicate in the table and acknowledgement trigger. Migration follows
v1 -> v2 -> v3 or v2 -> v3, rebuilds `manual_resolutions`, and fails closed
when historical blank references or any historical accepted-audit semantic
binding mismatch is present. Fresh v0 initialization, repeated v3
initialization, and unsupported newer versions are independently covered. A
schema-v1 store containing a historical manual acceptance without the v2
acknowledgement also fails closed for controlled audited recovery.

Every successful coordinator database operation additionally scans all
existing v3 append-reference projections. `Appended` outbox, disposition and
task-transition rows require a non-null ref with a byte outside ASCII
space/tab/LF/CR; `Pending` rows require `NULL`. Manual authorization refs,
manual accepted refs, authoritative accepted delivery refs after the delivery
audit boundary, and applied hydration-audit refs use the same predicate.
Historical or directly injected blank/reference-state mismatches therefore
fail closed even for v3 columns whose original table DDL predates this
amendment; this does not introduce a schema-v4 migration.

### 2.3 Immutable audit and push logs

Immutable audit retains its complete manifest-to-base chain, pinned lock and
record identities, and fixed base/child lock ordering. Every retained ancestor
also retains an `nlink` baseline. Legitimate child `mkdir`/`rmdir` drift may
refresh that baseline only after two complete rebind passes observe identical
link-count vectors and every component still resolves to the retained inode.
Replay and new append share the same durability and two-pass revalidation
epilogue.

Stored JSONL records use a closed authoritative field set. Unknown keys are an
invalid envelope even when all known fields and their derived record hash still
match, so adding an unhashed extension field cannot preserve chain validity.

Push logging binds one `PinnedPushLogWriter` when the runtime/sink namespace is
constructed. Replacing a root between two deliveries cannot establish a new
history. Production/test namespace or environment drift is rejected before
opening a log artifact.

New or concurrently existing directory entries are reopened, validated, and
their parent is synchronously persisted. File contents use `sync_all`, followed
by directory `sync_all`, before success.

## 3. Failure Modes

All failures are typed and fail closed:

- CWD or environment path drift;
- symlink, hard-link, ancestor, parent, date-directory, or file-leaf replacement;
- owner/mode/type/link-count mismatch;
- descriptor ambiguity, close/reuse, or process-shared SHM lifetime loss;
- initial main proof loss before WAL materialization;
- zero or multiple post-WAL main candidates;
- failure to acquire the deterministic marker on the selected post-WAL OFD;
- any second `journal_mode` mutation after the fully attested binding exists;
- database identity change before operation, before commit, or after commit;
- partial write, file sync, or directory sync failure;
- unsupported platform before any filesystem mutation.

There is no mock/in-memory/copy fallback and no retry under a newly rebound
namespace.

## 4. Trust Boundary

These checks isolate accidental path drift and other OS principals. Portable
Unix file APIs cannot prevent a malicious process running as the same UID from
renaming or linking between the last validation and a write. Release evidence
must therefore show an exclusive service UID or equivalent directory ACL/process
isolation. Until that deployment evidence exists, same-UID adversarial isolation
must not be claimed.

## 5. Required Tests

All filesystem tests run under unique `TEST_CODE` roots or an isolated child
process. Required coverage:

- startup from a foreign CWD still binds the compile-time production root without
  touching it in a test;
- bidirectional namespace symlink/hard-link rejection;
- ancestor, parent, main, WAL, SHM, date-directory, lock, record, and artifact
  replacement before and during operations;
- descriptor enumeration error, ambiguity, close/reuse, concurrent open/drop,
  process-shared SHM lifetime, crash/reopen, and unsupported targets;
- a macOS WAL-materialization regression proving the initial marker validates
  immediately before the transition, the old proof may become invalid
  immediately after it, and the bootstrap-only re-attestation installs and
  validates the exact replacement proof before schema or policy SQL;
- the post-WAL selector covers same-OFD idempotence, same-fd-number
  close/reopen, one unique new descriptor, missing descriptor, multiple
  descriptors, and a simultaneously live old marked OFD plus new candidate;
  the latter reaches the new candidate but must fail its same-range marker
  acquisition when the two descriptors are distinct OFDs;
- ancestor/main-leaf replacement between WAL materialization and re-attestation
  fails before schema version, table, policy or business-row mutation;
- a retained old OFD causes the new candidate's same-range marker acquisition
  to conflict; the implementation must not choose the old descriptor or
  allocate a different marker range;
- after `MainWalShmBound`, explicit marker removal, same-inode descriptor ABA
  and attempted journal-mode change fail the operation and cannot invoke any
  re-attestation path;
- a static/runtime contract proves post-binding connection configuration does
  not call the WAL materializer and only enables foreign keys plus reads back
  the three required PRAGMAs;
- replay/new append common validation and cross-delivery push-log root retention;
- every append revalidates the complete hash chain from byte zero; no
  process-memory cursor or incremental checkpoint may authorize an append;
- a counted sink result is authoritative only after an `AuditPending` artifact,
  a durable schema-v3 `push.delivery.audit` binding exact
  decision/attempt/artifact/result/receipt hashes, and a matching `Committed`
  artifact; audit failure after remote acceptance is `Uncertain` and never
  automatically resent;
- manual acceptance append failure and acknowledgement-CAS failure leave the
  independent accepted-audit row `Pending`; retry appends the exact same bytes,
  and `Delivered` verification succeeds only after the stored reference is
  identity/hash consistent;
- an append port that durably writes the exact manual accepted audit but returns
  an empty or whitespace-only reference cannot acknowledge it or reach
  `Delivered`; retry with the real reference reuses the same identity and bytes;
- cross-process acknowledgement uses a TEST_CODE file-backed append port whose
  records survive child exit; the parent joins every SQLite identity/hash/ref
  to the exact persisted record. An in-memory append port is not acceptable
  cross-process evidence;
- the cross-process manual-acceptance case specifically joins the
  `DeliveryAcceptedAudit` identity/canonical/hash/reference from the child file
  to the manual resolution and appended manual disposition observed by the
  parent;
- every successful SQLite operation revalidates all six persisted
  append-reference/state projections before commit. Whitespace-only
  space/tab/LF/CR references and hydration state/audit mismatches roll back the
  operation; after the projection is restored, an exact retry succeeds;
- a two-coordinator race changes the current disposition after the outer
  progress hint but before the `BEGIN IMMEDIATE` transition transaction; both
  task-bound and non-task-bound `Delivered` compare-and-sets fail closed, while
  unchanged normal paths reach `Delivered`;
- legitimate TEST_CODE child-directory creation exercises the stable two-pass
  nlink-baseline refresh rule without an isolation false positive;
- integration tests that spawn monitor processes share the
  `durable_physical_isolation` serialization group, because concurrent
  TEST_CODE namespace creation intentionally makes the shared ancestor nlink
  unstable and must continue to fail closed;
- a before/after production identity snapshot proving tests created or removed no
  production object.

## 6. Old Modules

| Module | Decision | Reason |
| --- | --- | --- |
| `database/sqlite_descriptor_attestation.rs` | adopt semantics | Reuse raw-fd identity and typed direct/shared SHM vocabulary; do not copy SQLite fds. |
| durable-delivery initial main proof | adopt as bootstrap-only | Authorizes only the controlled WAL transition; it is consumed by one post-WAL re-attestation and is never an operational proof. |
| `schema::configure_connection` WAL call | split/reject after binding | A second journal-mode mutation can replace an already attested OFD; post-binding configuration may only enable foreign keys and verify effective state. |
| runtime OFD proof repair/rebinding | reject | Reinstalling a lost operational marker would mask fd ABA or descriptor substitution. |
| caller-selected SQLite/push/audit paths | reject | They permit alternate production authorities. |
| CWD-relative fixed paths | reject | A foreign CWD silently creates a second ledger/log chain. |
| per-call push-log rebinding | reject | It forgets the history root between deliveries. |

## 7. Rollback

Rollback is code-only with `git revert <slice-commit>`. Never delete or rewrite a
production database, WAL/SHM, immutable audit record, lock, or push log. Do not
restore CWD-relative roots, path overrides, SQLite-fd duplication, or tests that
touch production paths. Rollback must not restore the old proof after WAL or
introduce runtime marker repair. If bootstrap fails after sidecar
materialization, close and discard the coordinator, preserve all durable
artifacts, and require the ordinary audited startup/recovery path; never delete
WAL/SHM as cleanup.
