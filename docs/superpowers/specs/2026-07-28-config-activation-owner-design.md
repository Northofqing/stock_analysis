# BR-174/BR-177/BR-179 Config Activation Owner and Legacy Cutover Design

**Status:** Gate A independently reviewed — PASS; Gate B not started

**Date:** 2026-07-28

**Business rules:** BR-174, BR-176, BR-177, BR-178, BR-179

**Data red lines:** 2.1, 2.2, 2.3, 2.4, 2.5, 2.7, 2.8, 2.10

`CONFIG_ACTIVATION_OWNER_DESIGN_SHA256 = c2810f2dac736539c9d00db628fda2f1fde4c74c3572e75a932867c8b7682714`

**Production readiness:** **No.** This document defines an implementation and verification contract. It does
not make config activation, schema-v2 selection, or legacy cutover production-ready.

## 1. Decision

Process initialization has exactly one public zero-argument facade. It reads the real process
arguments itself and returns an opaque proof that those arguments were parsed and bound:

```rust
pub fn bootstrap_selection_process(
) -> Result<VerifiedParsedSelectionCli, SelectionProcessBootstrapError>;

pub struct VerifiedParsedSelectionCli {
    _private: (),
}
```

There is no public `bootstrap_with_args`, `bind_mode`, parser constructor, path-bearing database
constructor or conversion from a CLI enum. `VerifiedParsedSelectionCli` exposes only read-only
dispatch predicates/tokens; it does not expose raw arguments, a forgeable mode enum, a database
path or the private binding.

Selection config activation is a second deep module named `ConfigActivationOwner`. Its
zero-argument operation is callable only after the public bootstrap succeeded with either an exact
Amended production/test binding or the one narrowly classified config/global-receipt
recovery-only binding:

```rust
pub struct ConfigActivationOwner {
    _private: (),
}

impl ConfigActivationOwner {
    pub fn require_current(
    ) -> Result<VerifiedCurrentConfigActivation, ConfigActivationOwnerError>;
}

pub struct VerifiedCurrentConfigActivation {
    _private: (),
}
```

There is no public constructor for either type. `require_current` obtains all process identities
from the already installed opaque binding:

- repository root: `env!("CARGO_MANIFEST_DIR")`;
- database access: after bootstrap acquires the lifetime shared
  `GlobalSchemaMaintenanceLease`, `DatabaseManager::GlobalSchemaVersionOwner` supplies either
  (a) the production/test `DatabaseManager` for an exact Amended mode-bound database at
  `application_id=1398035265` (`STSA`) and whole-application `user_version=1`, or
  (b) a pinned private no-pool recovery connection for the exact receipt-only
  `TransitionalIncomplete` state; never by `ConfigActivationOwner`'s caller;
- audit namespace: the matching production or invocation-unique test audit writer stored in that
  same binding;
- database maintenance authority: the non-forgeable shared global maintenance lease stored in
  that same binding for ordinary verification/recovery, or the exclusive lease plus global
  generation-1 migration transaction supplied internally by `GlobalSchemaVersionOwner` for the
  one first-cutover participant path;
- wall clock and UUIDv7 generation: owner-controlled;
- legacy cutover snapshot: captured or loaded and verified by the owner;
- runtime quiesce coordinator: the fixed process `SelectionRuntimeCoordinator`.

Neither public interface accepts arguments, mode, repository root, database path, SQLite
connection, audit root, audit writer/session, timestamp, stage ID, trigger definition or legacy
snapshot. Test processes select isolation only through their real `args_os` containing the strict
test CLI form. Clock/UUID fault injectors remain module-private implementation-test seams; they
cannot select mode or address production paths.

The recovery-only binding contains no pool, provider factory, sink, push/order capability or
general repository handle. After exact config/global receipt roll-forward, the global owner
reclassifies the same pinned database/audit state as Amended and installs the operational
`DatabaseManager` into a private inner one-time cell in the existing bootstrap generation. This is
not a second process bootstrap and cannot be invoked by callers.

`VerifiedCurrentConfigActivation` is an opaque, non-forgeable capability. Its public read-only
accessors may expose only the identity and chronology needed for observability:

```rust
impl VerifiedCurrentConfigActivation {
    pub fn activation_run_id(&self) -> &str;
    pub fn config_hash(&self) -> &str;
    pub fn effective_from(&self) -> DateTime<Utc>;
    pub fn receipt_content_hash(&self) -> &str;
}
```

The source-ingress owner consumes the capability by value through a crate-private accessor to the
verified canonical activation payload. Callers cannot obtain or replace the raw config snapshot,
legacy cutover snapshot, database proof, audit proof, or receipt proof.

This seam is intentionally narrow. Deleting the module would force every caller to understand
quiescence, cross-process locking, SQLite DDL, legacy table watermarks, trigger verification,
activation chronology, crash recovery, audit chaining and receipt validation. Keeping those
concerns behind one operation provides depth, leverage and locality.

## 2. Scope

This design owns:

1. selecting or creating the one current activation for the checked-in config;
2. exact recovery of partial config-activation runs;
3. the one-time v1 selection cutover and its immutable evidence;
4. permanent verification of the frozen legacy graph;
5. gating all normal, review, test and canary selection startup before providers;
6. the private legacy T0/D1 outcome drain capability.

It does **not** own the whole-database schema identity or a subsystem migration lock.
`DatabaseManager::GlobalSchemaVersionOwner` is the sole writer of `PRAGMA application_id` and
`PRAGMA user_version`, using the exact global identity contract from
`2026-07-28-selection-release-closure-amendment-design.md §3`. Config activation consumes a
non-forgeable global maintenance capability and can never write either PRAGMA. Its seven-table
catalog is a selection-cutover projection of the complete global mode catalog, not a second
whole-database schema authority.

For this narrow ownership boundary, §5.1 supersedes the parent evidence-closure design's
caller-created `SelectionStoreMode` construction, and §16 specializes that design's existing
`outcome_claim`/`outcome_run` kinds with two legacy payload schemas. It does not change the five
generic run-kind tokens or schema-v2 report visibility. Gate B must rotate every implementation
design hash that binds the parent or this document in the same code change; this Gate A docs-only
revision claims no hash rotation.

This design does not:

- fetch news, board constituents, quotes or K-lines;
- generate candidates, settle schema-v2 outcomes, push messages or place orders;
- replay pending v1 inbox rows into v2;
- rewrite v1 candidates into v2 samples;
- provide an operator override for production paths, roots, locks, clocks or evidence;
- define the separately gated full-database schema-v2 amendment migration.

## 3. Current Evidence and Readiness Gap

The following implementation is reusable:

| Existing evidence | Location | Decision |
|---|---|---|
| Opaque prepared activation with canonical stage/run/envelope preimages | `src/selection/config_activation_v2.rs:108-140` | Adopt and deepen |
| Deterministic config preparation | `src/selection/config_activation_v2.rs:177-317` | Split into owner-independent material and owner-only finalization |
| Chain config, private verified board artifact and executable revision hashing | `src/selection/config_activation_v2.rs:381-448` | Adopt |
| Strict checked-in activation file and chronology | `src/selection/config_activation_v2.rs:189-237`, `:751-783` | Adopt |
| Fixed production database and audit writer in persistence owner | `src/selection/persistence_v2.rs:62-114` | Adopt and deepen |
| Audit-lock-through-receipt persistence choreography | `src/selection/persistence_v2.rs:195-269` | Adopt through a locked-session internal seam |
| Cross-process audit lock, validation and durable append | `src/selection/audit.rs:187-297`, `:381-458` | Adopt unchanged |
| Config hash reuse and evidence verification skeleton | `src/database/selection_v2_repository.rs:827-846`, `:1202-1237`, `:3882-4038` | Replace the caller-manifest comparison with verified state loading |
| Typed legacy cutover and trigger preimages | `src/selection/schema_v2.rs:633-637`, `:4075-4114` | Adopt and add exact validation |
| Central list of seven legacy tables | `src/database/selection.rs:11-19` | Adopt as one exact constant |

The following facts block production:

1. `ConfigActivationPreparationContext` currently accepts caller-created stage IDs, times and a
   legacy snapshot (`src/selection/config_activation_v2.rs:83-105`).
2. Config preparation and executable hashing accept an arbitrary repository root
   (`src/selection/config_activation_v2.rs:177-187`, `:319-323`).
3. Legacy snapshot validation accepts arbitrary sorted table names and does not require the seven
   exact watermarks or derived counts (`src/selection/config_activation_v2.rs:799-835`).
4. `config_hash_reuse` requires an expected manifest hash that already commits a newly generated
   run/time (`src/database/selection_v2_repository.rs:1202-1237`). It therefore cannot select the
   original activation before manufacturing new chronology.
5. Verified config evidence discards the typed envelope payload and cutover snapshot
   (`src/database/selection_v2_repository.rs:3651-3661`, `:3906-4038`).
6. The v1 candidate schema has no `sample_schema`
   (`src/database/selection.rs:266-287`).
7. Startup creates only UPDATE/DELETE append-only triggers on all seven tables
   (`src/database/selection.rs:344-365`). It has no graph INSERT denial or conditional outcome
   INSERT guard.
8. `DatabaseManager::run_migrations` mutates the v1 trigger set at every startup
   (`src/database/mod.rs:588`). After cutover, this could silently recreate a missing registered
   trigger before verification.
9. Legacy `append_outcome` and query surfaces remain public
   (`src/database/selection.rs:1396`, `:1468`).
10. Monitor initializes the database and proceeds without recovery or activation
    (`src/bin/monitor/main.rs:3356-3400`), then calls the v1 adapter directly
    (`src/bin/monitor/main.rs:6301`; `src/bin/monitor/selection_shadow.rs:41-89`).
11. Runtime argv parsing, store mode and database construction remain caller/bin-orchestrated
    rather than one library-owned process bootstrap, so Rule 2.5 cannot be proven across bin/lib
    boundaries.
12. Selection-reachable provider constructors are not below an activation capability boundary.
13. Generic recovery currently has no config-first root, receipt-verified historical activation
    registry or zero-I/O lazy exact gateway, and the legacy drain has no durable claim/stage crash
    contract.

BR-174 and BR-177 remain Gate A registrations in `docs/business_rules.md:8-10`. None of the
existing pieces constitutes a production activation owner.

## 4. Alternatives Considered

### 4.1 Chosen: one fixed-root domain owner below the global schema owner

One zero-argument production operation owns selection-domain recovery and verification. The
selection-specific first-cutover participant alone constructs the snapshot, trigger projection and
recovery envelope, but it may execute DDL only when invoked inside the sole global generation-1
migration transaction under an exclusive `GlobalSchemaMaintenanceLease`. This composition
prevents callers from forging chronology, storage identity or legacy evidence without creating a
second database schema authority.

### 4.2 Rejected: caller-orchestrated preparation plus generic persistence

Keeping the current preparation context would let crate callers choose stage IDs, timestamps,
roots and legacy snapshots. Opaque fields do not restore trust when the opaque value was prepared
from caller-controlled authority. This approach also cannot atomically bind the first cutover DDL
to its recovery envelope.

### 4.3 Rejected: independent selection cutover command with monitor-only activation

An independent selection command could install triggers while monitor later generated a snapshot.
That creates two selection authorities and a crash window in which the graph is frozen but the
first canonical snapshot is absent. The accepted offline global migration is materially different:
it owns only whole-database exclusion, the complete generation-1 catalog and `STSA/1`; it calls the
config-cutover participant inside the same SQLite transaction, and that participant atomically
installs the legacy projection plus the first config recovery envelope. The participant cannot be
called from monitor startup or any generic migration command.

## 5. Fixed Identities and Internal Modules

`ConfigActivationOwner` contains the implementation behind the selection interface. It may use
the following private modules:

| Private module | Responsibility |
|---|---|
| `SelectionProcessBootstrapOwner` | Read real `args_os`, strict-parse once, choose the opaque mode, obtain the mode-bound lifetime shared global maintenance lease before pool construction, classify exact Amended vs the sole receipt-recoverable TransitionalIncomplete state, and install either the bound manager or no-pool recovery-only binding |
| `FixedConfigMaterialLoader` | Load the exact checked-in chain, typed board artifact, activation file and executable inputs from the manifest root |
| `SelectionRuntimeCoordinator` | Prevent new v1 work, stop v1 producers and wait for all registered v1 writer guards to drain |
| `ConfigActivationCutoverParticipant` | Accept only the private exclusive global generation-1 migration capability and atomically contribute the legacy projection plus first recovery envelope; never acquire locks, commit independently or write PRAGMAs |
| `LegacyCutoverRepository` | Verify/migrate v1 schema, install the trigger registry, capture watermarks and derive drain state |
| `ConfigActivationRepository` | Load verified activation states, persist the first atomic cutover envelope and resume locked durable stages |
| `HistoricalConfigActivationRegistry` | Verify every receipted activation and seal one exact historical capability for each persisted partial |
| `LazyExactRecoveryGateway` | Construct no provider until a persisted partial has resolved its historical activation/request seal |
| `LegacyV1OutcomeSettlementRepository` | Expose the only post-cutover v1 write transaction |

These are internal implementation seams, not production interfaces. Test adapters exist for the
runtime coordinator, clock/UUID source, database and audit writer, so the seams are real without
making production authority caller-configurable.

### 5.1 Unforgeable process-mode binding

One library-private module (implemented at `src/selection/process_bootstrap.rs`, not in a binary)
owns all four authorities together:

1. the only call to `std::env::args_os`;
2. the strict CLI parser;
3. `SelectionProcessBootstrapOwner`;
4. the private `OnceLock<BoundSelectionProcess>`.

The library re-exports only `bootstrap_selection_process()` and the opaque
`VerifiedParsedSelectionCli`. The function calls `args_os()` itself, parses the complete real argv
exactly once, and atomically installs one closed `BoundSelectionProcess` state. Operational CLI
states choose a private `SelectionProcessModeBinding`, acquire its shared process/OS global
maintenance lease before pool construction, require the mode-bound database to read back exact
`STSA/1`, and classify the complete database/audit state before it can construct a pool:

1. exact complete mode catalog plus linked global/config receipts is `Amended`; bootstrap may
   construct the binding's `DatabaseManager`, audit/sink/coordinator capabilities;
2. exact complete mode catalog plus first config envelope at `STSA/1`, where the only missing
   evidence is an exact prefix of the linked Prepared/manifest/Committed/config-receipt/global-
   receipt choreography, is `ConfigReceiptRecoveryOnly`; bootstrap installs only the shared lease,
   pinned database/audit identities, private recovery connection and coordinator;
3. any other `TransitionalIncomplete`, mixed identity, catalog drift, missing first envelope or
   receipt conflict is fatal with no manager/pool/provider/sink;
4. the private inner operational-storage cell is installed exactly once after recovery and
   Amended reclassification; it cannot replace the outer process binding or change mode.

The bootstrap cannot upgrade its shared lease. A production database requiring fresh initialization
or the sole `0/0 -> STSA/1` transition returns `OfflineGlobalMigrationRequired`; ordinary
production startup never claims it.
The exact test CLI is the sole exception: before obtaining its shared lifetime lease or
constructing its pool, the private bootstrap asks `GlobalSchemaVersionOwner` to fresh-initialize
the invocation-unique TEST_CODE database under that nonce's exclusive lease, complete test-mode
global catalog, config-cutover participant and `STSA/1` receipt, releases the exclusive lease, then
reopens it under the shared lifetime lease and re-verifies every identity. It cannot address,
inspect or migrate the production database. Help/version installs a terminal parsed-CLI state with
no operational binding or resources; invalid/unsupported argv installs a terminal rejected state
and returns an error, also with no resources. A second call or partial/second installation,
including the same argv/mode, is fatal. `ConfigActivationOwner` accepts only the exact Amended
variant or the exact `ConfigReceiptRecoveryOnly` variant; all other terminal/transitional states
are rejected.

No bin owns a parser, mode switch or singleton. No library or binary caller can supply
`Vec<OsString>`, `args`, mode, root, path, `DatabaseManager`, nonce or capability. A module-private
pure parser helper may accept an argv slice only in unit tests inside
`selection::process_bootstrap`; it is not re-exported, and all executable tests use
`std::process::Command` to set the child process's real argv.

The two closed variants have these physical identities:

| Process binding | Binding-selected storage and other namespace | Accepted symbols |
|---|---|---|
| production (normal, review and live canary) | exact Amended uses private `DatabaseManager::production_bound()` for only the manifest-root production database; exact receipt recovery uses only the pinned no-pool connection until Amended reclassification; audit/global-lock/sinks are the matching production objects, but sinks are absent from recovery-only state | canonical six-digit real A-share codes; reject `TEST_CODE_` |
| test (only exact explicit test CLI parsed from real argv) | private global owner first creates one bootstrap-generated invocation-unique `TEST_CODE_` generation-1 database under its nonce-bound exclusive lease; only after `STSA/1`/receipt read-back does `DatabaseManager::test_bound()` open it under a lifetime shared lease; crash recovery uses the same nonce-bound no-pool state; audit root, global-lock root and no-production-sink namespace share that nonce | `TEST_CODE_[0-9]{6}` only; reject real codes |

The selection-facing `DatabaseManager` has no public path/mode constructor or cross-mode
conversion. Any generic manager retained for unrelated modules cannot be passed into selection.
Mode is not inferred from `cfg(test)`, a database filename, CWD or environment. The test binding
contains descriptor-pinned isolated roots created by the private owner and cannot resolve the
production database, audit, lock or sink roots. Production has no override seam. Child tasks
receive only borrowed opaque capabilities, which are process-lifetime stable and neither `Clone`
nor serializable. `ConfigActivationOwner::require_current()` reads the complete
`BoundSelectionProcess` internally, requires its stored opaque parsed-CLI proof and binding to name
the same bootstrap generation, and fails unbound/mismatched before storage.

This is the Rule 2.5 boundary. Executable integration tests must start separate processes and prove
that production rejects every `TEST_CODE_` order/data write, test rejects every real-symbol
order/data write, their database/audit/lock object identities are disjoint, and test mode cannot
construct or call a production sink even when CWD/environment/path strings are hostile. Compile-
fail/architecture tests also prove every bin/library attempt to call a parser helper, install the
OnceLock, construct a mode-specific manager, or pass caller argv/mode/path fails.

### 5.2 Provider-construction capability boundary

All selection-reachable financial and news provider constructors move below one crate-private
`SelectionProviderConstructionOwner`. Its two non-interchangeable entry points are:

```rust
fn for_new_work(
    current: &VerifiedCurrentConfigActivation,
) -> NewWorkProviderFactory;

fn for_exact_recovery(
    historical: &VerifiedRecoveryConfigActivation,
) -> ExactRecoveryProviderFactory;
```

Both also borrow the installed opaque process/database binding internally. New work accepts only
`VerifiedCurrentConfigActivation`. Recovery accepts only the sealed historical capability bound to
that persisted partial; it cannot accept/current-cast the current capability or reconstruct either
capability from run IDs, hashes or display fields.

Every production constructor under `src/data_gateway/**` that is reachable from selection becomes
crate-private and its executable call graph has this closed allow-list:

```text
ConfigActivationOwner -> VerifiedCurrentConfigActivation + HistoricalConfigActivationRegistry
VerifiedCurrentConfigActivation -> NewWorkProviderFactory
HistoricalConfigActivationRegistry -> VerifiedRecoveryConfigActivation
GlobalSelectionRecoveryOwner -> LazyExactRecoveryGateway
LazyExactRecoveryGateway -> VerifiedRecoveryConfigActivation -> ExactRecoveryProviderFactory
NewSelectionWorkBootstrapOwner -> VerifiedCurrentConfigActivation -> NewWorkProviderFactory
NewWorkProviderFactory | ExactRecoveryProviderFactory -> provider constructors
```

`main.rs`, news/bootstrap registration, review/test/canary dispatch, selection ingress, generation
and legacy settlement may call only these owners. They cannot call `production_*`, `new`,
`from_env`, transport builders or provider factories directly. The same restriction applies to
helper modules and re-exports, so moving a call behind an alias is not a bypass.

An AST/HIR-aware architecture test records the exact allowed files and symbols, follows
multi-line calls, imports, aliases, macros and re-exports, and fails on every new edge. Compile-fail
fixtures prove an arbitrary crate module cannot construct the owner/capability or call a provider
constructor. Grep is diagnostic only and is not the compliance proof.

The global process/OS maintenance lock, owned by `GlobalSchemaVersionOwner`, has the same
path-hardening requirements as the selection audit lock:

- the manifest root and every ancestor are real directories, not symlinks;
- the lock directory is created under the fixed root and synchronously persisted;
- the lock file is a regular file opened without following symlinks;
- the acquired file identity is read back after locking;
- environment variables, CWD and caller input cannot alter the path;
- lock contention is an explicit retryable startup failure, not an unlocked fallback;
- ordinary bootstrap acquires only a shared lease before pool construction and retains it for the
  entire `DatabaseManager` lifetime;
- the offline global migration alone acquires the exclusive lease before opening audit or SQLite;
  shared-to-exclusive upgrade is forbidden.

## 6. Runtime Quiesce Contract

Every v1 acquisition/evaluation write registers an in-flight writer guard with the fixed
`SelectionRuntimeCoordinator`. Registration fails once quiescing begins. The coordinator tracks the
following writer classes:

- event inbox and completion writes;
- selection run, candidate and feature writes;
- visibility receipt writes;
- legacy outcome settlement writes.

`ConfigActivationOwner::require_current` enters the barrier before taking a migration or audit
lock:

1. prevent new v1 acquisition/evaluation tasks from starting;
2. signal existing v1 acquisition/evaluation tasks to stop;
3. wait until their in-flight writer count reaches zero;
4. retain a quiesce guard until activation verification and receipt read-back finish.

At initial monitor startup, the coordinator is initialized before selection tasks and therefore
proves a zero in-flight count. On explicit config reload, it drains already-running tasks. The
legacy outcome writer is also drained during the cutover transaction; after the first activation it
may be restarted only through `LegacyV1OutcomeSettlementRepository`.

Failure to register all old writer call sites, failure to stop a task, timeout waiting for a writer,
or a new registration after barrier entry fails activation before DDL. There is no force-abort path
that could leave an unknown SQLite transaction alive.

## 7. Lock and I/O Order

There is no `ConfigActivationMigrationLock`. The two legal global orders are:

```text
ordinary startup/recovery/new-hash activation:
shared GlobalSchemaMaintenanceLease (acquired before DatabaseManager/pool; retained for lifetime)
  -> SelectionRuntimeCoordinator quiesce guard
    -> LockedSelectionAuditSession
      -> SQLite BEGIN IMMEDIATE transaction

offline fresh/0-0 generation-1 migration:
process stopped or SelectionRuntimeCoordinator proven quiescent
  -> exclusive GlobalSchemaMaintenanceLease
    -> LockedSelectionAuditSession
      -> GlobalSchemaVersionOwner-owned SQLite BEGIN IMMEDIATE transaction
        -> ConfigActivationCutoverParticipant borrowed capability
```

Nested resources are released in reverse order; the ordinary shared lease remains until
`DatabaseManager` is dropped. Code must never acquire/upgrade a maintenance lease after the audit
lock or from inside a SQLite transaction. The cutover participant receives borrowed capabilities
and cannot acquire/release the global lease, start/commit the transaction, or escape them.

No provider or network I/O is permitted while the quiesce guard, audit lock or SQLite transaction
is held; the offline path additionally forbids it for the complete exclusive lease. The checked-in
chain, activation file, executable inputs and typed board artifact are local release inputs, not
provider calls. To keep lock duration bounded, the owner may load them once before quiescence, but
it must re-read and re-hash every fixed input after the relevant global maintenance lease is held.
Any byte or path-set change invalidates the draft and aborts activation.

The production SQLite connection must read back:

```text
PRAGMA foreign_keys = 1
PRAGMA synchronous = 2
PRAGMA integrity_check = 'ok'
```

The first two are checked on every pooled connection. The integrity check is required before the
one-time cutover transaction. Every authoritative operation also reads
`application_id=1398035265` and `user_version=1`; any other matrix fails before selection-domain
work. Config activation may read these values and bind their verified receipt, but cannot set them.

## 8. Exact Legacy Schema Contract

### 8.1 Seven watermark tables

`LegacyCutoverSnapshotPreimage.tables_sorted` contains exactly these seven names in UTF-8 byte
order:

1. `selection_candidates`
2. `selection_event_completions`
3. `selection_event_inbox`
4. `selection_feature_snapshots`
5. `selection_outcomes`
6. `selection_runs`
7. `selection_visibility_receipts`

For each table, under the same `BEGIN IMMEDIATE` snapshot:

```sql
SELECT COUNT(*) AS row_count,
       COALESCE(MAX(rowid), 0) AS max_rowid
FROM <fixed_allow_list_table>;
```

Table names are selected from a compile-time enum and are never interpolated from caller or
database content. SQLite integers must convert losslessly to `u64` for `row_count` and non-negative
`i64` for `max_rowid`. Empty tables have `max_rowid = 0`.

The owner also derives, in that transaction:

```sql
-- pending_inbox_count
SELECT COUNT(*)
FROM selection_event_inbox i
LEFT JOIN selection_event_completions c ON c.event_id = i.event_id
WHERE c.event_id IS NULL;

-- committed_legacy_candidate_count
SELECT COUNT(*)
FROM selection_candidates c
JOIN selection_visibility_receipts v ON v.run_id = c.run_id
WHERE c.sample_schema = 'legacy-v1';

-- legacy_outcome_row_count
SELECT COUNT(*)
FROM selection_outcomes;
```

The owner rejects a derived value greater than its parent watermark and requires
`legacy_outcome_row_count` to equal the `selection_outcomes` watermark count.

### 8.2 `sample_schema` migration

The cutover adds this exact immutable discriminator when it is absent:

```sql
ALTER TABLE selection_candidates
ADD COLUMN sample_schema TEXT NOT NULL
    DEFAULT 'legacy-v1'
    CHECK (sample_schema = 'legacy-v1');
```

SQLite applies the constant default to existing rows. The migration performs no semantic per-row
UPDATE. If the column already exists, `PRAGMA table_xinfo(selection_candidates)` and the canonical
table SQL must prove the exact `TEXT NOT NULL DEFAULT 'legacy-v1'` and single-value CHECK contract.
Any different type, nullability, default, allowed value or generated/hidden state fails closed.
The v1 table never permits `schema-v2`; v2 samples live only in `selection_samples`.

The legacy due query must explicitly filter `c.sample_schema = 'legacy-v1'`.

### 8.3 Frozen seven-table schema catalog

The owner does not treat “the tables exist” as schema evidence. It builds
`LegacySchemaCatalogPreimage` from `sqlite_master`, `PRAGMA table_xinfo`,
`PRAGMA foreign_key_list` and `PRAGMA index_list/index_xinfo`, sorts every vector by the explicit
keys below, and requires exact equality with one of the two mode-bound golden catalogs. Extra,
missing, partial or reordered semantic objects fail closed.

The seven exact column vectors are frozen below. `PK` is the one-based primary-key ordinal; `NN`
is `NOT NULL`; `default` is the canonical default SQL or `NULL`. All unspecified columns have
`PK=0`, `NN=false`, `default=NULL`, `hidden=0`.

| Table | Exact columns in cid order |
|---|---|
| `selection_event_inbox` | `event_id TEXT PK=1`; `content_hash TEXT NN`; `payload_json TEXT NN`; `provider TEXT NN`; `provider_published_at TEXT`; `provider_published_on TEXT`; `observed_at TEXT NN`; `source_batch_id TEXT NN`; `source_batch_hash TEXT NN`; `evaluation_market_date TEXT NN`; `ingested_at TEXT NN default=(strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))` |
| `selection_event_completions` | `completion_id TEXT PK=1`; `event_id TEXT NN UNIQUE`; `content_hash TEXT NN`; `status TEXT NN`; `reason_code TEXT`; `completed_at TEXT NN`; `recorded_at TEXT NN default=(strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))` |
| `selection_runs` | `run_id TEXT PK=1`; `content_hash TEXT NN`; `evaluation_market_date TEXT NN`; `config_hash TEXT NN`; `magic_tdx_batch_id TEXT NN`; `magic_tdx_batch_hash TEXT NN`; `created_at TEXT NN`; `recorded_at TEXT NN default=(strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))` |
| `selection_candidates` pre-cutover | `candidate_id TEXT PK=1`; `run_id TEXT NN`; `event_id TEXT NN`; `chain_id TEXT NN`; `stock_code TEXT NN`; `stock_name TEXT NN`; `relation_version TEXT NN`; `feature_version TEXT NN`; `ordinal INTEGER NN`; `content_hash TEXT NN`; `evaluation_market_date TEXT NN`; `recorded_at TEXT NN default=(strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))` |
| `selection_candidates` post-cutover | exact pre-cutover vector followed by `sample_schema TEXT NN default='legacy-v1' hidden=0` |
| `selection_feature_snapshots` | `feature_snapshot_id TEXT PK=1`; `candidate_id TEXT NN UNIQUE`; `content_hash TEXT NN`; `payload_json TEXT NN`; `source_batch_id TEXT NN`; `source_batch_hash TEXT NN`; `observed_at TEXT NN`; `recorded_at TEXT NN default=(strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))` |
| `selection_outcomes` | `outcome_id TEXT PK=1`; `candidate_id TEXT NN`; `phase TEXT NN`; `market_date TEXT NN`; `content_hash TEXT NN`; `payload_json TEXT NN`; `observed_at TEXT NN`; `recorded_at TEXT NN default=(strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))` |
| `selection_visibility_receipts` | `receipt_id TEXT PK=1`; `run_id TEXT NN UNIQUE`; `audit_record_hash TEXT NN`; `content_hash TEXT NN`; `published_at TEXT NN`; `recorded_at TEXT NN default=(strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))` |

The exact CHECK vectors, in canonical SQL encounter order, are:

| Table | Exact CHECK expressions |
|---|---|
| inbox | `length(trim(event_id)) > 0`; `length(trim(content_hash)) > 0`; `length(trim(payload_json)) > 0`; `length(trim(provider)) > 0`; `length(trim(source_batch_id)) > 0`; `length(trim(source_batch_hash)) > 0` |
| completions | `status IN ('completed', 'rejected')`; `length(trim(completion_id)) > 0`; `length(trim(content_hash)) > 0`; `status = 'completed' OR (reason_code IS NOT NULL AND length(trim(reason_code)) > 0)` |
| runs | `length(trim(run_id)) > 0`; `length(trim(content_hash)) > 0`; `length(trim(config_hash)) > 0`; `length(trim(magic_tdx_batch_id)) > 0`; `length(trim(magic_tdx_batch_hash)) > 0` |
| candidates | `ordinal >= 0`; `length(trim(candidate_id)) > 0`; `length(trim(chain_id)) > 0`; one mode expression below; `length(trim(stock_name)) > 0`; `length(trim(relation_version)) > 0`; `length(trim(feature_version)) > 0`; `length(trim(content_hash)) > 0`; post-cutover additionally `sample_schema = 'legacy-v1'` |
| feature snapshots | `length(trim(feature_snapshot_id)) > 0`; `length(trim(content_hash)) > 0`; `length(trim(payload_json)) > 0`; `length(trim(source_batch_id)) > 0`; `length(trim(source_batch_hash)) > 0` |
| outcomes | `phase IN ('t0_close', 'd1_settled')`; `length(trim(outcome_id)) > 0`; `length(trim(content_hash)) > 0`; `length(trim(payload_json)) > 0` |
| visibility receipts | `length(trim(receipt_id)) > 0`; `length(trim(audit_record_hash)) > 0`; `length(trim(content_hash)) > 0` |

The mode expression is selected only by the unforgeable runtime binding:

```text
production: length(stock_code) = 6 AND stock_code NOT GLOB '*[^0-9]*'
test:       stock_code GLOB 'TEST_CODE_[0-9][0-9][0-9][0-9][0-9][0-9]'
```

The exact foreign-key vector, sorted by
`(from_table, from_column, to_table, to_column)`, is:

```text
selection_event_completions.event_id       -> selection_event_inbox.event_id
selection_candidates.event_id              -> selection_event_inbox.event_id
selection_candidates.run_id                -> selection_runs.run_id
selection_feature_snapshots.candidate_id    -> selection_candidates.candidate_id
selection_outcomes.candidate_id             -> selection_candidates.candidate_id
selection_visibility_receipts.run_id        -> selection_runs.run_id
```

Every entry is immediate, `ON UPDATE NO ACTION`, `ON DELETE NO ACTION`, `MATCH NONE`, with no
deferrable clause. The exact explicit index vector is:

```text
idx_selection_event_pending       (selection_event_inbox: observed_at ASC, event_id ASC)
idx_selection_completion_event    (selection_event_completions: event_id ASC)
idx_selection_candidate_run_date  (selection_candidates: run_id ASC,
                                    evaluation_market_date ASC, ordinal ASC,
                                    candidate_id ASC)
idx_selection_outcome_due         (selection_outcomes: candidate_id ASC, phase ASC,
                                    market_date ASC)
idx_selection_visibility_run      (selection_visibility_receipts: run_id ASC)
```

The exact constraint-backed unique index vector is:

```text
sqlite_autoindex_selection_event_inbox_1           origin=pk (event_id)
sqlite_autoindex_selection_event_completions_1     origin=pk (completion_id)
sqlite_autoindex_selection_event_completions_2     origin=u  (event_id)
sqlite_autoindex_selection_runs_1                   origin=pk (run_id)
sqlite_autoindex_selection_candidates_1             origin=pk (candidate_id)
sqlite_autoindex_selection_candidates_2             origin=u  (run_id,event_id,chain_id,stock_code)
sqlite_autoindex_selection_feature_snapshots_1      origin=pk (feature_snapshot_id)
sqlite_autoindex_selection_feature_snapshots_2      origin=u  (candidate_id)
sqlite_autoindex_selection_outcomes_1               origin=pk (outcome_id)
sqlite_autoindex_selection_outcomes_2               origin=u  (candidate_id,phase)
sqlite_autoindex_selection_visibility_receipts_1    origin=pk (receipt_id)
sqlite_autoindex_selection_visibility_receipts_2    origin=u  (run_id)
```

Every entry is `unique=1`, `partial=0`; every key term uses `BINARY`, ASC, is a real column, and
the rowid auxiliary term returned by `index_xinfo` is non-key. The five explicit indexes are
`origin=c`, `unique=0`, `partial=0` with the same term rules. No expression index, collation
override, DESC term or extra index is permitted.

### 8.4 SQL canonicalization and golden hashes

`canonical_legacy_schema_sql_v1` is a small lexer, not a regex over arbitrary SQL. It:

1. rejects NUL, comments, invalid/unterminated single/double/backtick/bracket quoting and more than
   one statement;
2. trims leading/trailing ASCII whitespace and removes one terminal semicolon outside quotes;
3. outside quotes, collapses each ASCII-whitespace run to one U+0020;
4. normalizes only the exact leading tokens `CREATE TABLE IF NOT EXISTS` to `CREATE TABLE` and
   `CREATE INDEX IF NOT EXISTS` to `CREATE INDEX`;
5. preserves all other case and bytes, especially identifiers, CHECK/default expressions and
   quoted literals.

Objects sort by `(object_type, name)` UTF-8 bytes. Each object hash is
`SHA-256(canonical_sql UTF-8)`. These hashes are normative golden vectors:

| Object | SHA-256 |
|---|---|
| `selection_event_inbox` | `88e8dc98b3fc714f9d7092d2c4f5c46d5a61eb78ed70c631b50d98ca3da0febf` |
| `selection_event_completions` | `84dd2256eb9639d59b21bd2350861326883c12dba10f871bca586730b23891c9` |
| `selection_runs` | `1f125e2dba9905f857048861c8d5a3d4960c1db8dbf50eeaefdd7bd65e8c5ff1` |
| `selection_feature_snapshots` | `c3fc733ab72fb1c319a4f3c5cbea3141edd1bc20069eacee6bae1f9013033464` |
| `selection_outcomes` | `5aee4358f0021362a7a44b5116220df6be37d5a5386eb69d86a4ab426dc46201` |
| `selection_visibility_receipts` | `ddf237c20242ed614218e07c6c56a5d9888e4fa186012f24675526a4b5259455` |
| candidates production pre/post | `782c20dcfd962bded8c333974b346c89867985d7d718a5d9efe320e076815d41` / `2929c89f74fb83db8a1dfa6edb97d3e216432484cfeb1a80c56d5646318ab88e` |
| candidates test pre/post | `9de00b8c0aae2465ae9b9a9e4217ee4b946e607a1349217314a887b13b67eaa0` / `9bd4fac83363efa5181453afd66b0e1aa67ca8f7a7be2cb1d8a192754e9447ce` |
| `idx_selection_event_pending` | `52ba6017aa92dc7c6b522648073e716bbdebe7b9187d08c3ec39d8a62ced3c66` |
| `idx_selection_completion_event` | `ed68b9b6da4019fb7b129c0b03e3977cf9fa079aedcb0563bc7644803352e616` |
| `idx_selection_candidate_run_date` | `6b710ddc2425dac642c4d290596a03f2afcb0ec08561128f537762d68b27f85b` |
| `idx_selection_outcome_due` | `bbd49a97af11962cc3852c572760fbd5f01b37271ea1bf1d4b91bb81e2833e4a` |
| `idx_selection_visibility_run` | `38d9a74c33e85ca9022eb820ec521af2ff8ea9b567070623f1b09fe28de0ba48` |

The richer `LegacySchemaCatalogPreimage` has
`domain="stock_analysis.br179.legacy_schema_catalog.v1"`, bound process mode, cutover phase, all
column/default/CHECK/FK/index vectors above and the sorted object hashes. SQLite version text is
diagnostic only and is not permitted to relax any exact comparison. The catalog embeds a
separately frozen
SQL digest with this exact field order and lowercase tokens:

```rust
struct LegacySchemaSqlObjectDigestV1<'a> {
    object_type: &'a str, // "index" | "table"
    name: &'a str,
    table_name: &'a str,
    canonical_sql_sha256: &'a str,
}

struct LegacySchemaSqlDigestPreimage<'a> {
    domain: &'a str, // exactly "stock_analysis.br179.legacy_schema_catalog_sql.v1"
    mode: &'a str,   // "production" | "test"
    phase: &'a str,  // "pre" | "post"
    objects_sorted: &'a [LegacySchemaSqlObjectDigestV1<'a>],
}
```

The SQL-digest `sha256_json` golden constants are:

```text
production/pre-cutover  b24620c35f48b10cadc8e2b2239c05108fb26bbfb2e89941b328fcfa0dfa6ca0
production/post-cutover 339cc0bff2ffa4480ec2324e69c78d9aa654de69bf153bdd0a78cd212eeb959e
test/pre-cutover        8640a7bc6ecfd8432d62c0e940873bfffc7fc193d0270bf748302d4ce4ffe7fa
test/post-cutover       7326f2914a01f382c1076a585d891c00217a529a39521044bf34979430ff3e3e
```

These four constants are hashes of that exact domain/mode/phase/sorted object-hash vector and are
cross-checked with the richer structural catalog. An implementation change to either preimage
requires an explicit schema-version amendment and new reviewed goldens, never silent rehashing.
Golden fixtures include the full canonical SQL bytes and all structural vectors, so a hash alone
cannot hide a parser disagreement.

### 8.5 Relationship to the complete global managed catalog

The catalog in this section is a typed projection used to capture and permanently verify the
legacy cutover. It is not sufficient to classify the database as globally managed. Its parent
authority is the exact mode-keyed 70-object catalog and same-runtime SQLite receipt defined by
`2026-07-28-selection-release-closure-amendment-design.md §4`.

For every overlapping table, index, trigger, column, CHECK, foreign key and canonical SQL object:

1. the global catalog projection and this legacy projection must have byte-identical canonical
   definitions and identical hashes;
2. the global runtime identity
   (`sqlite3_libversion_number`, `sqlite_source_id`, sorted compile-options hash) must equal the
   runtime identity bound into the generation-1 receipt and config recovery envelope;
3. every non-projection object is scanned under the global dependency rules and may not reference
   a legacy managed table except through the exact global allow-list;
4. production and test projections are compared only against their corresponding complete
   mode-bound global catalogs; a union or cross-mode comparison is forbidden.

`VerifiedLegacySchemaCatalog` can prove only the selection cutover subset. Provider construction,
current activation and historical activation recovery additionally require the non-forgeable
verified global catalog/generation capability. Any disagreement is
`GlobalSelectionCatalogProjectionMismatch`, never a local rehash or repair.

## 9. Exact 21-Trigger Registry

### 9.1 Membership and naming

The six frozen graph tables are:

```text
selection_event_inbox
selection_event_completions
selection_runs
selection_candidates
selection_feature_snapshots
selection_visibility_receipts
```

Each has exactly these three triggers:

```text
trg_<table>_no_insert
trg_<table>_no_update
trg_<table>_no_delete
```

`selection_outcomes` has exactly:

```text
trg_selection_outcomes_no_update
trg_selection_outcomes_no_delete
trg_selection_outcomes_legacy_insert_guard
```

The registry therefore contains `6 * 3 + 3 = 21` entries. Entries sort uniquely by
`trigger_name` UTF-8 bytes. Any missing trigger, additional trigger targeting one of the seven
tables, duplicate name, wrong target table, wrong operation or changed canonical SQL fails startup.

The exact sorted registry names are:

```text
trg_selection_candidates_no_delete
trg_selection_candidates_no_insert
trg_selection_candidates_no_update
trg_selection_event_completions_no_delete
trg_selection_event_completions_no_insert
trg_selection_event_completions_no_update
trg_selection_event_inbox_no_delete
trg_selection_event_inbox_no_insert
trg_selection_event_inbox_no_update
trg_selection_feature_snapshots_no_delete
trg_selection_feature_snapshots_no_insert
trg_selection_feature_snapshots_no_update
trg_selection_outcomes_legacy_insert_guard
trg_selection_outcomes_no_delete
trg_selection_outcomes_no_update
trg_selection_runs_no_delete
trg_selection_runs_no_insert
trg_selection_runs_no_update
trg_selection_visibility_receipts_no_delete
trg_selection_visibility_receipts_no_insert
trg_selection_visibility_receipts_no_update
```

The existing UPDATE/DELETE trigger names and messages from
`src/database/selection.rs:351-363` are retained. For pristine only, the owner creates the complete
14-trigger base set; an existing pre-cutover database with any missing/extra/conflicting base
trigger is orphaned and is not repaired. Cutover adds exactly the six graph INSERT denials and one
outcome INSERT guard. Once the first cutover envelope commits, no startup or migration path may
add, remove, replace or repair any registered trigger.

### 9.2 Canonical SQL bytes

For each frozen graph table and operation, canonical SQL is generated from these exact templates:

```sql
CREATE TRIGGER trg_<table>_no_insert
BEFORE INSERT ON <table>
BEGIN
  SELECT RAISE(ABORT, 'BR-174 <table> frozen after legacy cutover');
END
```

```sql
CREATE TRIGGER trg_<table>_no_update
BEFORE UPDATE ON <table>
BEGIN
  SELECT RAISE(ABORT, 'BR-157 <table> is append-only');
END
```

```sql
CREATE TRIGGER trg_<table>_no_delete
BEFORE DELETE ON <table>
BEGIN
  SELECT RAISE(ABORT, 'BR-157 <table> is append-only');
END
```

The outcome denial definitions are:

```sql
CREATE TRIGGER trg_selection_outcomes_no_update
BEFORE UPDATE ON selection_outcomes
BEGIN
  SELECT RAISE(ABORT, 'BR-157 selection_outcomes is append-only');
END
```

```sql
CREATE TRIGGER trg_selection_outcomes_no_delete
BEFORE DELETE ON selection_outcomes
BEGIN
  SELECT RAISE(ABORT, 'BR-157 selection_outcomes is append-only');
END
```

The sole INSERT whitelist is:

```sql
CREATE TRIGGER trg_selection_outcomes_legacy_insert_guard
BEFORE INSERT ON selection_outcomes
WHEN NOT (
  NEW.phase IN ('t0_close', 'd1_settled')
  AND EXISTS (
    SELECT 1
    FROM selection_candidates c
    JOIN selection_visibility_receipts v ON v.run_id = c.run_id
    WHERE c.candidate_id = NEW.candidate_id
      AND c.sample_schema = 'legacy-v1'
  )
)
BEGIN
  SELECT RAISE(ABORT, 'BR-174 selection_outcomes accepts only committed legacy-v1 T0/D1');
END
```

The existing `UNIQUE(candidate_id, phase)` constraint and repository due-market-date validation
remain mandatory. The trigger does not replace either validation.

`canonical_trigger_sql_v1` applies exactly this lexical normalization to both the checked-in
template and `sqlite_master.sql`:

1. reject NUL, SQL comments, unterminated quotes and more than one statement;
2. trim leading/trailing ASCII whitespace;
3. remove one terminal semicolon outside a quoted token;
4. outside single quotes, double quotes, backticks and bracket identifiers, collapse each
   non-empty ASCII whitespace run to one U+0020;
5. preserve every byte inside quoted tokens and preserve keyword/name case.

The resulting UTF-8 bytes are `LegacyTriggerDefinitionPreimage.canonical_sql`.
`LegacyTriggerSetPreimage` has domain exactly
`stock_analysis.br174.legacy_trigger_set.v1`; its 21 definitions are sorted by trigger name and
hashed with `sha256_json`. The canonicalizer, all templates, sorted registry JSON and hash require
golden vectors.

### 9.3 Exact pre-cutover registry and orphan rejection

When no config-activation envelope exists, the database may be in exactly one of two states:

1. **pristine:** none of the seven tables, five explicit indexes, fourteen legacy triggers,
   `sample_schema` or seven cutover-only triggers exists; or
2. **exact v1 pre-cutover:** the bound-mode pre-cutover catalog in §8.3/§8.4 exists exactly, and
   the trigger set is exactly the UPDATE/DELETE pair for each of the seven tables using the
   BR-157 templates in §9.2.

The exact pre-cutover trigger registry is the 14-name subset of §9.1 ending in `_no_update` or
`_no_delete`. Its canonical set hash is
`854fc01fe91cc0d59aee5ebe3b1c5a3cb916ae0086da50f526a867c53d097bb8`.
The exact post-cutover 21-trigger set hash is
`0400a647fc5297922786717c4fb28c7784ec07af53366530e53018eccc64234e`.
Both are `sha256_json(LegacyTriggerSetPreimage)` using the domain and canonicalization in §9.2;
the full sorted definition JSON and every individual canonical-SQL hash are checked-in golden
fixtures.

Any no-envelope state containing `sample_schema`, any cutover-only `_no_insert` or
`trg_selection_outcomes_legacy_insert_guard`, any post-cutover catalog hash, or any partial,
missing, extra or conflicting legacy object is `OrphanLegacyCutover`. It is fatal and is never
adopted, dropped, completed, repaired or treated as a fresh database. An exact post-cutover graph
without its carrier envelope is also orphaned even if all 21 triggers match. The only automatic
creation path is pristine -> exact base v1 -> atomic cutover+envelope in one transaction; the only
automatic mutation of an existing graph is exact pre-cutover -> atomic cutover+envelope.

## 10. First Activation Protocol

The first activation exists only as a mandatory participant in fresh generation-1 initialization
or the offline operator-owned exact `0/0 -> STSA/1` migration. Ordinary production monitor startup
never performs this protocol. The private exact-test bootstrap may invoke the same global fresh
initializer only for its invocation-unique TEST_CODE namespace before constructing a test pool; it
has no production path capability. If an ordinary `STSA/1` startup has no first recovery envelope,
it fails `MissingGlobalMigrationParticipantEvidence`; it does not infer pristine state or create
DDL.

### 10.1 Pre-lock material

Before opening audit or SQLite, `GlobalSchemaVersionOwner` acquires the exclusive global
maintenance lease and supplies a private, lifetime-bound
`VerifiedGlobalGenerationOneMigration<'lease>`. The internal
`ConfigActivationCutoverParticipant`:

1. loads fixed checked-in config material;
2. computes `config_hash`;
3. strictly verifies the activation file and typed board artifact;
4. prepares no stage UUID, `activated_at`, `enveloped_at` or legacy snapshot yet;
5. verifies the frozen design SHA-256 and executable revision binding;
6. proves the process is stopped or the runtime quiesce barrier is complete;
7. proves the supplied capability names the exact mode, pinned database/audit identities, source
   global identity (`0/0` for offline migration or no file for fresh initialization), candidate
   `STSA/1`, and same-runtime complete global catalog.

The participant cannot construct this capability, choose another database, or retain it.

### 10.2 Locked lookup

After the global owner acquires the selection audit lock and starts its one exclusive
`BEGIN IMMEDIATE`, it lends the participant private borrowed views of the same audit session and
transaction. The participant:

1. initializes and verifies the v2 repository under the same audit session;
2. searches recovery envelopes, manifests and receipts for all config activations;
3. proves that no prior activation or conflicting partial state exists;
4. revalidates the fixed release inputs byte-for-byte;
5. revalidates the source and candidate global identity/catalog preimages and the legacy projection
   relationship from §8.5;
6. creates no UUID and reads no activation clock.

The outer lookup is advisory until this global SQLite write lock exists. A concurrent process,
pre-existing orphan, incomplete whole-database catalog or identity drift must not be hidden by an
identity allocated too early.

### 10.3 Atomic DDL and recovery envelope

The already-owned global `BEGIN IMMEDIATE` transaction performs, in order:

1. repeat the envelope/manifest/receipt lookup and require the same `Absent` state;
2. repeat the full audit-prefix, fixed-input, activation-file and config-hash validation;
3. repeat the global source/candidate identity, complete catalog, runtime and projection checks;
4. require either the exact pristine or exact pre-cutover state from §9.3 and reject every orphan;
5. for pristine only, create the exact bound-mode base seven-table/five-index/14-trigger catalog;
   for pre-cutover, perform no base DDL; in both cases reverify exact pre-cutover catalog,
   constraints, foreign keys and 14-trigger hash;
6. only now read the owner clock once and allocate one UUIDv7 stage ID from that same time;
7. add or verify `sample_schema`;
8. create or verify the exact 21-trigger registry;
9. query the seven watermarks and three derived counts;
10. construct and hash `LegacyTriggerSetPreimage` and `LegacyCutoverSnapshotPreimage`;
11. finalize `ConfigActivationStageInputPreimage` with that transaction-local ID/time, frozen
    design hash, SQLite runtime identity, global catalog hash, candidate `STSA/1` generation and the
    installed process/database binding hashes required by §12.1;
12. serialize and reparse the strict canonical stage payload;
13. immediately recheck current activation absence, config hash, activation-file bytes, audit
    prefix, post-cutover catalog and trigger registry before the INSERT;
14. insert the complete config-activation recovery envelope;
15. read back and re-hash the envelope;
16. return a non-escaping `VerifiedConfigCutoverContribution<'transaction>` to the global owner;
17. the global owner verifies the complete generation-1 catalog, writes the sole `STSA/1` PRAGMAs,
    reads all global and participant evidence back, and commits with `synchronous=FULL`.

The single owner time is both the first durable `activated_at` and
`legacy_cutover_snapshot.captured_at`. UUID/time are never returned, logged, placed in a provider
request, audit file or other durable store before the envelope transaction commits. A rollback
therefore leaves no externally observable allocation, and a later attempt may allocate a new
identity. Exact recovery/reuse never reads the clock or UUID source. A later new-hash activation
uses the same rule: `BEGIN IMMEDIATE`, exact recheck, then and only then allocate its ID/time.

The complete global generation-1 DDL, `STSA/1`, selection cutover DDL and first recovery envelope
are one atomic SQLite commit. A durable frozen graph can never exist without the exact envelope
that records its original watermarks and trigger hash. Conversely, the first envelope cannot exist
unless the complete global catalog, schema identity and trigger registry committed.

The transaction does not write the stage manifest, receipt or audit file. The already-held audit
session then continues the existing durable protocol:

```text
append+sync ConfigActivationPrepared
  -> SQLite FULL domain rows + run manifest
  -> append+sync ConfigActivationCommitted
  -> SQLite FULL commit receipt
  -> exact receipt/manifest/envelope/audit read-back
```

The exclusive global maintenance lease and audit lock remain held until receipt verification
completes and the global migration receipt binds the config receipt. Only then does the offline
command report success. A crash after the global commit but before either receipt leaves an exact
`STSA/1` recoverable activation partial; ordinary startup may roll it forward under its shared
lifetime lease and audit lock but performs zero DDL and zero PRAGMA writes.

### 10.4 Database initialization change

`DatabaseManager::run_migrations` must stop calling the trigger-mutating
`selection::create_schema` before the owner (`src/database/mod.rs:588`). Legacy selection schema
creation/cutover moves behind `ConfigActivationCutoverParticipant`, invoked only by
`GlobalSchemaVersionOwner` under the fixed exclusive protocol.

For a genuinely fresh production database, the offline global generation-1 owner creates the
complete whole-application schema and invokes the participant in the same transaction. The exact
test bootstrap may do the same only for its new invocation-unique isolated database. For a
nonempty production `0/0` database, only the offline operator migration may invoke it after
verifying preservation of every application object and row. Once the transaction commits, all
normal startup is schema-verify-only; config activation recovery/later new-hash activation may
append domain envelopes/manifests/receipts but never DDL. No path may run
`CREATE TRIGGER IF NOT EXISTS` as a repair.

## 11. Crash Recovery

The config activation state is determined from the union of recovery envelopes, run manifests,
receipts and audit records, never from the manifest table alone.

| Durable state | Required action |
|---|---|
| Offline global migration, no envelope/manifest/receipt and exact eligible source state | Invoke the cutover participant inside the same exclusive global transaction |
| Ordinary `STSA/1` startup, no first envelope/manifest/receipt | Fatal `MissingGlobalMigrationParticipantEvidence`; never create DDL |
| Exact envelope only | Reparse typed payload and resume the same stage ID/time with zero DDL recapture |
| Envelope + Prepared only | Resume the same domain stage |
| Envelope + Prepared + manifest | Verify staged rows, append/reuse exact Committed record and receipt |
| Committed audit without receipt | Insert/reuse the exact receipt using the persisted Committed time |
| Exact config receipt, missing global migration receipt | Global owner verifies the linked config receipt/catalog/identity and inserts or reuses the exact global receipt |
| Exact verified config + global receipts | Reclassify Amended, install/reuse the manager, return/reuse the current capability |
| Any orphan, duplicate or content mismatch | Integrity failure; no new run |

On recovery from a first-cutover envelope, the owner verifies:

- the exact `sample_schema` contract;
- exact 21-trigger registry membership, canonical SQL and hash;
- the envelope's typed cutover snapshot and hash;
- the envelope/config/run/audit identities.

It does not recalculate the original watermarks. An exact partial run always retains its original
UUIDv7, `activated_at`, `effective_from`, `enveloped_at`, legacy snapshot and content hashes.

Required crash failpoints are:

1. before the atomic complete global DDL/identity/config-envelope commit;
2. after DDL/envelope commit and before Prepared;
3. after Prepared and before stage;
4. after stage and before Committed;
5. after Committed and before receipt;
6. after receipt and before returning the capability.

The offline command yields the complete source state for failpoint 1. Failpoints 2-6 yield an exact
`STSA/1` activation/global-receipt partial that ordinary startup rolls forward with DML only under
the lifetime shared recovery-only binding. It constructs the pool/manager only after the global
owner verifies both receipts and reclassifies Amended. Recovery never reapplies DDL, rewrites
PRAGMAs or yields a mixed global-catalog/trigger/snapshot state.

## 12. Same-Hash Reuse

The owner computes the checked-in `config_hash` before creating a run ID or activation time, then
calls a verified repository lookup:

```rust
enum VerifiedConfigActivationState {
    Absent,
    RecoverableExact(RecoverableConfigActivation),
    ReceiptedExact(VerifiedCurrentConfigActivation),
}

fn load_by_config_hash_locked(
    conn: &mut SqliteConnection,
    audit: &mut LockedSelectionAuditSession<'_>,
    config_hash: &ConfigHash,
) -> Result<VerifiedConfigActivationState, ConfigActivationRepositoryError>;
```

The lookup:

- examines both envelopes and manifests, so envelope-only recovery is visible;
- requires zero or one activation identity for the hash;
- reparses the strict typed envelope and canonical JSON;
- verifies config snapshot, activation, cutover and envelope hashes;
- verifies Prepared, manifest, Committed and receipt links when present;
- verifies the artifact remains valid and the activation is not expired;
- returns the original IDs and times from durable evidence.

`ReceiptedExact` returns the existing activation without generating a UUID or reading a new
activation clock. `RecoverableExact` resumes the same run. More than one identity for a config hash,
a different payload under the same hash, DB-only proof, audit-only proof, an expired artifact or a
malformed receipt is an integrity failure.

Before returning either a recovered or already receipted current capability, the owner rechecks the
live `sample_schema` contract and exact 21-trigger registry against the cutover hash. Same-hash reuse
is not permission to skip current legacy-graph integrity.

The current activation is selected by exact checked-in `config_hash`, never by `MAX(time)`, latest
row, latest manifest or latest receipt.

### 12.1 Receipt-verified historical activation registry

Every config activation, including the first cutover and every later hash, carries the installed
process binding's `database_binding_hash`, the frozen design hash declared at the top of this
document, the SQLite runtime identity, the exact complete global mode-catalog hash and the verified
`STSA/1` generation receipt in its strict recovery envelope, stage manifest and receipt validation.
The database binding is the domain-separated SHA-256 of the bound mode, manifest-relative database
identity, pinned database object/lineage identity and global schema generation; it excludes caller
paths because none exist. Copying an activation/audit chain into another database/mode/runtime or
building it from a different reviewed design therefore does not produce a valid activation.

The design hash is `SHA-256` of the complete UTF-8 bytes of this file after removing exactly the
single complete line that contains the backtick-delimited
`CONFIG_ACTIVATION_OWNER_DESIGN_SHA256 = <64 lowercase hex characters>` declaration. Removing any
other line, normalizing whitespace, or hashing rendered Markdown is forbidden. Gate B embeds this
value into the executable revision binding and every config activation preimage named above. A
source/design mismatch is `ConfigActivationDesignRevisionMismatch` before storage or provider
construction.

The persistent envelope/stage/receipt corpus is also the historical registry; no mutable “current
config” table or second post-receipt registration transaction exists. Under the selection-audit
lock and one pinned read transaction, `ConfigActivationOwner` builds the sealed
`HistoricalConfigActivationRegistry` from this exact inner join:

```text
config_activation recovery envelope
  -> exact config_activation run stage/manifest
  -> exact selection_v2_commit_receipt
  -> exact Prepared/Committed records in the validated audit prefix
```

The entry preimage has this fixed field order:

```rust
struct HistoricalConfigActivationRegistryEntryPreimage<'a> {
    domain: &'a str, // "stock_analysis.br179.historical_config_activation.v1"
    process_binding_hash: &'a str,
    database_binding_hash: &'a str,
    global_application_id: i64, // exactly 1398035265
    global_schema_generation: i64, // exactly 1
    global_schema_catalog_hash: &'a str,
    global_schema_generation_receipt_hash: &'a str,
    sqlite_runtime_identity_hash: &'a str,
    config_activation_owner_design_sha256: &'a str,
    activation_run_id: &'a str,
    config_hash: &'a str,
    config_snapshot_json_hash: &'a str,
    activation_content_hash: &'a str,
    activation_file_content_hash: &'a str,
    provider_board_artifact_hash: &'a str,
    executable_revision_hash: &'a str,
    legacy_cutover_snapshot_hash: &'a str,
    effective_from_rfc3339_nanos_utc: &'a str,
    recovery_envelope_content_hash: &'a str,
    prepared_audit_hash: &'a str,
    run_manifest_content_hash: &'a str,
    staged_db_content_hash: &'a str,
    committed_audit_hash: &'a str,
    receipt_content_hash: &'a str,
    receipt_committed_at_rfc3339_nanos_utc: &'a str,
}
```

`registry_entry_hash = sha256_json(HistoricalConfigActivationRegistryEntryPreimage)`. Entries sort
by `(receipt_committed_at, activation_run_id)` and require exact-one by `activation_run_id` and
exact-one by `config_hash`. A config activation becomes a registry member at the same atomic
receipt commit that makes it authoritative: an envelope, Prepared record, manifest or Committed
record without the exact receipt remains recoverable activation state but is not historical
authority. Receipt insertion revalidates the envelope/manifest/database binding; registry loading
then revalidates the complete audit chain and every nested hash.

`require_current()` first recovers config-activation partials, then verifies the complete historical
registry, chooses the exact checked-in config hash as `VerifiedCurrentConfigActivation`, and stores
the sealed registry with that same bootstrap/database generation for library-private recovery use.
It publicly returns only the current capability. A malformed older entry, duplicate run/hash,
receipt/audit mismatch or database-binding mismatch is fatal even when the newest activation is
otherwise valid.

### 12.2 Exact historical capability for old partials

Every non-config recovery envelope already persists
`config_activation_run_id`, `config_hash`, its own `stage_run_id` and envelope content hash.
`LazyExactRecoveryGateway` parses that durable typed envelope first, then asks the sealed registry
to resolve those persisted identities:

```rust
fn seal_for_recovery(
    registry: &HistoricalConfigActivationRegistry,
    intent: &VerifiedPersistedRecoveryIntent,
) -> Result<VerifiedRecoveryConfigActivation, HistoricalActivationError>;

pub(crate) struct VerifiedRecoveryConfigActivation {
    _sealed: (),
}
```

`VerifiedPersistedRecoveryIntent` can be created only by strict canonical reparse/read-back of the
database envelope; callers cannot pass strings. Resolution requires:

- exact registry entry for both persisted activation run ID and config hash;
- exact registry-entry, receipt, manifest, envelope, audit and database-binding hashes;
- exact process/bootstrap generation and production/test mode;
- the partial's persisted config snapshot/request hashes to agree with that historical entry;
- the partial subject/stage run ID and envelope hash to be sealed into the returned capability.

The capability contains the verified historical typed config snapshot and is neither public-
constructible, `Clone`, serializable nor convertible to/from `VerifiedCurrentConfigActivation`.
It is valid only for the one sealed partial and its exact persisted provider request. Historical
artifact validity is verified at its original activation/ingress chronology; later expiry or a
new current activation does not rewrite the old snapshot. Current wall-clock validity still
governs whether the persisted operation may legally perform I/O (for example BR-177's prospective
window), and a closed window produces its specified no-provider terminal path.

`SelectionProviderConstructionOwner::for_exact_recovery` accepts only this sealed capability and
rechecks its partial/request seal immediately before construction. New polling, ingress,
generation and claims accept only `for_new_work(&VerifiedCurrentConfigActivation)`. Thus after
activating config B, an unfinished run created under config A can recover only with A's receipt-
verified snapshot, while all new work can use only B. There is no “fallback to current”, latest
activation join, caller-selected historical hash or mixed-field provider configuration.

Under BR-176, config activation recovery and full registry verification occur first. Non-config
partials retain their existing stable ordering; immediately before each partial is resumed, the
lazy gateway resolves its own historical capability. Missing/unreceipted activation authority,
run/hash disagreement, copied database binding, config conflict or capability/request mismatch is
an integrity failure that stops recovery and all new provider work. It is never converted into an
empty batch, retry against current config or new run.

## 13. Later New-Hash Activation

A new activation is permitted only when:

- the computed config hash has no activation or partial run;
- every existing activation is fully verified;
- the checked-in activation file names the new exact hash and has valid prospective chronology;
- the first receipted activation's legacy cutover snapshot can be loaded exactly.

The repository provides:

```rust
fn load_first_cutover_locked(
    conn: &mut SqliteConnection,
    audit: &mut LockedSelectionAuditSession<'_>,
) -> Result<VerifiedLegacyCutoverSnapshot, ConfigActivationRepositoryError>;
```

It:

1. loads all receipted config activations ordered by receipt
   `(committed_at ASC, subject_id ASC)`;
2. verifies every envelope, manifest, Prepared/Committed audit record and receipt;
3. selects the first receipted activation as the source carrier;
4. reparses its complete typed payload;
5. recomputes its cutover snapshot hash;
6. requires every later activation to contain byte-identical canonical snapshot JSON and the same
   hash.

The returned value is opaque and can only be consumed by owner-only activation finalization.
Later activation copies the exact first snapshot bytes/preimage/hash. It never queries new legacy
watermarks and never changes `captured_at`. This remains true after allowed legacy T0/D1 outcome
rows have appended.

The owner still rechecks the live `sample_schema` and exact 21-trigger registry before committing a
new activation. Live graph integrity is current evidence; original watermarks remain historical
evidence.

The later activation also binds the same installed database identity and becomes a new
receipt-verified §12.1 historical-registry entry at its receipt commit. Previous entries remain
byte-identical and queryable by their original run ID/config hash; setting the new checked-in hash
current does not relabel, delete or supersede their authority for exact recovery.

## 14. Repository Locked-Session Interface

The repository and persistence implementation gain crate-private operations that operate under
resources already owned by `ConfigActivationOwner`:

```rust
fn initialize_locked(
    conn: &mut SqliteConnection,
    audit: &mut LockedSelectionAuditSession<'_>,
) -> Result<ConfigActivationRepository, ConfigActivationRepositoryError>;

fn persist_first_cutover_envelope_locked(
    &self,
    conn: &mut SqliteConnection,
    draft: FixedConfigActivationDraft,
) -> Result<PersistedConfigActivationEnvelope, ConfigActivationRepositoryError>;

fn persist_new_config_envelope_locked(
    &self,
    conn: &mut SqliteConnection,
    draft: FixedConfigActivationDraft,
    cutover: VerifiedLegacyCutoverSnapshot,
) -> Result<PersistedConfigActivationEnvelope, ConfigActivationRepositoryError>;

fn resume_to_receipt_locked(
    &self,
    conn: &mut SqliteConnection,
    audit: &mut LockedSelectionAuditSession<'_>,
    envelope: PersistedConfigActivationEnvelope,
) -> Result<VerifiedCurrentConfigActivation, ConfigActivationRepositoryError>;

fn load_historical_activation_registry_locked(
    &self,
    conn: &mut SqliteConnection,
    audit: &mut LockedSelectionAuditSession<'_>,
    binding: &BoundSelectionProcessRef<'_>,
) -> Result<HistoricalConfigActivationRegistry, ConfigActivationRepositoryError>;

fn seal_exact_recovery_activation(
    &self,
    registry: &HistoricalConfigActivationRegistry,
    intent: &VerifiedPersistedRecoveryIntent,
) -> Result<VerifiedRecoveryConfigActivation, ConfigActivationRepositoryError>;
```

These operations do not acquire an audit session internally. This avoids relocking/deadlock while
the cutover owner holds the same session. The public generic persistence methods continue to own
their resources for ingress/outcome callers, but config activation can only use the owner path.
`SelectionV2PersistenceOwner::commit_config_activation` ceases to be public once the owner is
wired.

Repository result types expose typed outcomes, never `Option` or log-only success. Every operation
reads back and hashes its authoritative rows before returning.
`load_historical_activation_registry_locked` accepts only a borrowed private binding installed by
the bootstrap owner, verifies its process/database binding hashes against every registry entry and
returns a sealed value tied to that bootstrap generation. `seal_exact_recovery_activation` accepts
only the strict database-reparsed intent; neither method accepts run IDs, hashes, mode, paths or a
database manager from its caller.

## 15. Monitor Startup Gate

Normal, review, test and canary selection startup use this order:

```text
bootstrap_selection_process()
       (library-private args_os parse -> opaque process mode
        -> shared global maintenance lease before pool
        -> exact mode-bound STSA/1 global classification
        -> Amended manager OR no-pool ConfigReceiptRecoveryOnly binding)
  -> ConfigActivationOwner::require_current()
       (internally recover/reuse config activations and linked global receipt
        -> global owner reclassifies exact Amended
        -> private inner cell installs binding-selected DatabaseManager once
        -> verify historical registry and select current)
  -> require local time >= effective_from
  -> construct LazyExactRecoveryGateway(capability)
       (zero provider/network construction or I/O)
  -> GlobalSelectionRecoveryOwner drains exact non-config partials
       through the lazy gateway in BR-176 stable order, each with its sealed
       historical activation capability
  -> construct/start new-work news, board and market providers
       only from VerifiedCurrentConfigActivation
  -> source ingress
  -> generation
```

The gate is inserted immediately after database initialization around
`src/bin/monitor/main.rs:3356-3400`, before broker/provider/news aggregator construction and before
the call path at `src/bin/monitor/main.rs:6301`.

“Database initialization” here means manager/pool construction only after exact Amended
identity/catalog/receipt verification; it does not mean running subsystem DDL. The narrowly typed
receipt-recovery connection is not a `DatabaseManager` and exposes only config/global receipt
roll-forward. `OfflineGlobalMigrationRequired`, unsupported application ID/generation, any other
incomplete global state, runtime mismatch, missing first envelope or a failure to hold the lifetime
shared lease is fatal before the `DatabaseManager` becomes selection-capable.

`bootstrap_selection_process()` is the only public process-mode/DatabaseManager creation seam.
`ConfigActivationOwner` is the global recovery root: it first classifies and completes every exact
config-activation partial and its linked global migration receipt, consumes the recovery-only
binding into an Amended manager when necessary, verifies the receipt-backed historical registry,
and returns the exact current activation. No generic recovery scan may run before it, because those
scans need the historical registry and provider-construction authority it grants.

After that capability and the internally sealed registry exist, `LazyExactRecoveryGateway` is a
closed, crate-private request router.
Constructing it stores only the process/config capabilities; it performs zero provider or network
construction and zero I/O. `GlobalSelectionRecoveryOwner` then classifies every non-config partial
into exactly one BR-176 recovery set and drains the stable sequence:

```text
ingress envelope-only/manifested
  -> generation envelope-only/manifested
  -> v2 outcome claim/outcome recovery
  -> legacy-v1 drain claim/outcome recovery
```

For a persisted typed request that genuinely requires continuation, the gateway resolves the
persisted activation run ID/hash to the §12.2 sealed historical capability, revalidates the exact
request/config/process/database binding, and calls only `for_exact_recovery`. Staged provider
evidence resumes without a refetch. Malformed, conflicting or lineage-invalid recovery state fails
closed before construction. Recovery must reach a receipted/terminal fixed point before
`for_new_work`, polling, new ingress, generation, new due claim or general provider bootstrap
begins.

If recovery or activation fails:

- monitor emits one structured fatal startup diagnostic with a stable reason code;
- no financial/news provider is constructed or called;
- no source ingress, selection generation, notification, push or order path starts;
- no fallback activation or current-config join is permitted;
- process mode returns its documented non-zero exit status.

If the activation is valid but future-effective, the owner returns a typed
`ConfigNotYetEffective` startup outcome. It does not activate facts early and does not create a new
activation on each retry.

The current v1 adapter in `src/bin/monitor/selection_shadow.rs:41-89` is replaced by the receipted
v2 ingress/generation owner. Its settlement entry remains only as the private legacy drain until
that drain is complete.

## 16. Private Legacy Outcome Drain

After cutover, `append_outcome` is private and reachable only through the owner below. It is not a
generic repository escape hatch:

```rust
pub(crate) struct LegacyV1OutcomeSettlementRepository {
    _process_mode: VerifiedSelectionProcessMode,
    _config_activation: VerifiedCurrentConfigActivation,
    _first_cutover: VerifiedLegacyCutoverSnapshot,
    _schema_catalog: VerifiedLegacySchemaCatalog,
    _verified_trigger_registry: VerifiedLegacyTriggerRegistry,
}
```

The repository can be constructed only after the current startup has verified:

- the exact first cutover carrier and hash;
- exact live 21-trigger registry;
- exact bound-mode post-cutover seven-table catalog, including `sample_schema`;
- production/test symbol isolation;
- the complete audit prefix and a pinned SQLite snapshot of drain state.

It exposes only owner-internal operations for:

1. deriving the next due legacy T0/D1 work;
2. claiming one exact logical subject durably;
3. staging validated provider evidence and appending one outcome durably;
4. recovering the exact claim/stage after every crash point;
5. deriving `Pending | Complete`.

### 16.1 Durable intent and payload variants

The drain specializes the existing BR-176/BR-178 `outcome_claim` and `outcome_run` durable
choreography; it does not add a sixth generic run kind. Its closed payload-schema variants are:

```text
outcome_claim + payload_schema=legacy-v1-drain-claim-v1
outcome_run   + payload_schema=legacy-v1-drain-stage-v1
```

The claim preimage freezes: process-mode binding hash; current activation/run/config hash; first
cutover and live schema/trigger hashes; candidate/run/visibility receipt identities; exact
`legacy-v1` discriminator; phase and stored due date; immutable market-calendar vector/hash;
typed provider request and request hash; claim UUIDv7; and preallocated outcome-run UUIDv7. The
stage preimage references the exact claim receipt/hash and freezes raw provider records, batch
identity/hash, observation/provider times, all Rule 2.3 validation evidence, any required manual
confirmation receipt, the final outcome payload/hash and the original planned run ID. Both parse
with `deny_unknown_fields`, canonical JSON, EOF checks and domain-separated `sha256_json`.

Fresh work uses this lock sequence:

```text
fixed per-logical-subject OS lock
  -> selection-audit OS lock
    -> SQLite FULL transaction
```

The subject lock is acquired before the claim revalidation and remains held through final outcome
receipt read-back. The audit lock spans each local claim/stage choreography but is released,
together with every SQLite transaction, before provider/network I/O. The subject lock alone stays
held during provider I/O. No lease or age-based takeover exists; process death releases the OS
lock, and recovery must reuse the exact durable IDs/request.

Before provider I/O the owner persists and receipts the claim:

```text
claim recovery envelope -> Prepared -> claim manifest -> Committed -> claim receipt
```

After provider I/O it reacquires the audit lock and persists:

```text
stage recovery envelope -> Prepared -> validated outcome + manifest
  -> Committed -> outcome receipt
```

If the stage envelope already contains complete provider evidence, recovery performs zero provider
calls. No database/audit/provider write may occur merely from a log message.

### 16.2 Per-operation revalidation and Rule 2.3

At due-read, inside the claim transaction, immediately before provider construction, after provider
return, and inside the outcome stage transaction, the owner revalidates:

- the unforgeable process-mode identity and symbol contract;
- exact first cutover carrier, current config activation and complete audit prefix;
- exact live post-cutover schema catalog and 21-trigger registry;
- exact committed `legacy-v1` candidate plus visibility receipt;
- phase absence/uniqueness, T0-before-D1 ordering, due date/calendar vector and typed request hash;
- no newer claim/outcome receipt that supersedes the pinned due capability.

Any change invalidates the capability and forces exact reclassification; it never writes using
stale authority. Provider data enters computation only after the complete Rule 2.3 gate:

- every price is finite and greater than zero;
- requested/returned dates are exact, unique, sorted and calendar-continuous; gaps and duplicates
  are explicit errors;
- corporate-action/split/dividend evidence proves expected series continuity;
- an adjacent valid-value move outside ±20% is an alert requiring an immutable BR-171 manual
  confirmation bound to the exact provider batch, symbol, dates and values; absence/mismatch is a
  hard validation failure, not a warning or automatic rejection of a legitimately confirmed move.

Missing source fields remain missing under Rule 2.2. A bad/missing/partial batch produces a typed
retryable or non-retryable attempt only when lineage remains valid; it never fabricates a price,
inserts an outcome or closes an integrity-invalid claim.

### 16.3 Crash recovery matrix

| Last durable state | Exact recovery action |
|---|---|
| Before claim envelope | No attempt exists; fresh scheduling may allocate after revalidation |
| Claim envelope only / partial claim choreography | Resume same claim ID, planned outcome-run ID and request |
| Receipted claim, no stage envelope | Revalidate and continue the same typed request through the lazy exact gateway |
| Stage envelope contains provider evidence | Resume staging with zero refetch |
| Outcome/manifest committed, no Committed audit/receipt | Verify exact rows, append/reuse exact Committed and receipt |
| Exact outcome receipt | Terminal for that candidate/phase; never duplicate |
| Any identity, request, lineage, schema, audit or evidence mismatch | Integrity failure; do not allocate a replacement or close the claim |

`GlobalSelectionRecoveryOwner` runs this matrix after config activation and before new work, in the
stable order defined in §15/BR-176. Exact recovery never reads a new clock/UUID or silently
substitutes the current config/provider request.

### 16.4 Terminal drain state

The as-of-independent terminal anti-join is:

```sql
SELECT COUNT(*)
FROM selection_candidates c
JOIN selection_visibility_receipts v ON v.run_id = c.run_id
WHERE c.sample_schema = 'legacy-v1'
  AND (
    NOT EXISTS (
      SELECT 1
      FROM selection_outcomes o
      WHERE o.candidate_id = c.candidate_id
        AND o.phase = 't0_close'
    )
    OR NOT EXISTS (
      SELECT 1
      FROM selection_outcomes o
      WHERE o.candidate_id = c.candidate_id
        AND o.phase = 'd1_settled'
    )
  );
```

`count > 0` is `Pending`; `count = 0` is `Complete`. A current due set of zero is not completion.
The immutable graph, append-only outcomes, unique phase constraint and unchanged INSERT guard make
`Pending -> Complete` monotonic and `Complete -> Pending` impossible.

At `Complete`, monitor does not start the legacy drain owner. The exact same guard remains installed
and hashed; duplicate/missing-phase rules make every further outcome INSERT fail. No DDL changes
occur at completion.

Pending v1 inbox rows are counted as `legacy_excluded` and are never replayed or marked complete by
v2.

## 17. Failure Model

| Failure | Required result |
|---|---|
| Caller/bin supplies argv, mode, path, manager or attempts a second bootstrap | Compile/architecture failure or fatal before any process resource |
| Process mode absent, rebound, or production/test physical identity overlaps | Fatal before storage/provider work; Rule 2.5 failure |
| Ordinary startup cannot acquire a shared global lease before pool construction | Explicit startup unavailable; no pool/provider work |
| Ordinary startup sees anything except exact Amended or receipt-prefix-recoverable mode-bound complete catalog with `STSA/1` | Fatal unsupported/incomplete global schema; require offline owner where eligible |
| Recovery-only binding exposes pool/sink/provider/general repository, or manager is built before linked config/global receipts reclassify Amended | Architecture/type failure or fatal; release blocked |
| Selection code attempts to write `application_id`/`user_version`, acquire an independent migration lock or upgrade shared lease | Architecture/compliance failure; release blocked |
| Cutover participant is called without the exclusive global generation-1 capability/transaction | Type rejection or fatal authority mismatch; zero DDL |
| Legacy projection differs from the same-runtime complete global mode catalog | `GlobalSelectionCatalogProjectionMismatch`; no rehash/repair |
| Frozen design hash, executable binding or receipt design hash differs | `ConfigActivationDesignRevisionMismatch`; zero provider work |
| Provider constructor appears outside the capability allow-list | Architecture/compliance failure; release blocked |
| Fixed config/root input missing, symlinked, unreadable or changes during activation | Fail before activation; no provider work |
| Typed board artifact/raw release bytes disagree | Fail before activation |
| Runtime coordinator absent, unknown writer class, stop failure or in-flight writer remains | Fail before DDL |
| Global maintenance lease or audit lock unavailable | Explicit retryable unavailable; no unlocked fallback |
| SQLite foreign keys/FULL/integrity check fails | Fail before DDL |
| Legacy table/index/foreign-key shape is partial or unexpected | Integrity failure |
| No envelope plus any partial/post-cutover schema or trigger object | `OrphanLegacyCutover`; never repair/adopt |
| Existing `sample_schema` differs from the exact contract | Integrity failure; no UPDATE repair |
| Existing trigger name/table/operation/SQL conflicts before first cutover | Fail before snapshot |
| Registered trigger is missing/extra/changed after cutover | Fatal startup integrity failure; never recreate it |
| DDL or envelope insertion fails | Whole `BEGIN IMMEDIATE` transaction rolls back |
| Process dies after first DDL/envelope commit | Resume exact envelope/run/time; never recapture |
| Audit append or sync fails | Preserve exact recoverable envelope/stage; no visibility |
| Database becomes FULL or receipt insert fails | Preserve exact recovery state; no capability |
| Same config hash has multiple identities or different payload | Fatal integrity conflict |
| Historical activation registry has missing/duplicate/unreceipted entry or wrong database binding | Fatal before non-config recovery/provider work |
| Persisted partial activation run/hash does not resolve exact-one historical receipt | Integrity failure; never substitute current activation |
| Recovery factory receives current/unsealed/wrong-partial capability or request | Type rejection or fatal seal mismatch; zero provider construction |
| Receipt, manifest, audit, envelope or nested hash differs | Fatal integrity conflict |
| New config activation file is unreviewed, future-invalid or hash-mismatched | Fail closed |
| Activation is valid but not yet effective | Typed wait; zero ingress/provider work |
| Board artifact is expired | Fail closed; do not reuse activation |
| Later activation carries a different cutover snapshot | Fatal integrity conflict |
| Old graph writer attempts a write after cutover | Physical trigger abort plus structured diagnostic |
| Direct legacy outcome writer remains callable | Architecture/compliance failure |
| Legacy outcome does not match visible `legacy-v1` T0/D1 | Repository rejection and physical trigger abort |
| Legacy drain claim/stage lineage or Rule 2.3 evidence differs | Integrity failure; no outcome and no replacement claim |

No failure becomes an empty batch, default configuration, fabricated snapshot, warning-only success
or current-config fallback.

## 18. Test Matrix

### 18.1 Interface and identity

- Compile-fail tests prove callers cannot construct either capability or access its private
  preimages.
- Every selection-capable binary is launched with real child-process argv and proves the only
  public zero-argument bootstrap parses it; architecture/compile-fail fixtures reject
  caller-supplied argv, mode, path, `DatabaseManager`, parser access and `OnceLock` installation.
- Invalid/help/unsupported argv proves no database/audit/lock/sink/provider object was constructed;
  a second bootstrap, including identical argv, fails.
- Architecture tests prove no production operation accepts root/path/connection/audit/time/UUID/
  snapshot arguments.
- CWD and relevant environment mutations cannot change production root, database, audit or lock
  identities.
- Test mode accepts only an isolated TEST_CODE database/audit/lock namespace; production rejects
  `TEST_CODE_`, and test mode rejects real symbols.
- Separate-process tests prove the mode binds once before storage/provider construction, cannot be
  rebound, and test/production database, audit, lock and sink object identities never overlap.
- Cross-process tests prove ordinary bootstrap acquires the mode-bound shared global maintenance
  lease before pool construction, retains it for the manager lifetime, accepts only exact Amended
  or the sole receipt-recovery prefix over the complete mode catalog at `STSA/1`, and cannot
  upgrade the lease.
- Crash after each global commit/config/global receipt substep and prove bootstrap exposes only the
  no-pool recovery binding, reuses exact IDs/times/hashes, installs the manager once only after
  Amended reclassification, and never exposes a sink/provider/order/push capability early.
- Separate-process exact-test fixtures prove the private global owner fresh-initializes only the
  invocation-unique TEST_CODE namespace under its exclusive lease, releases it before acquiring
  the shared lifetime lease/pool, and cannot resolve or mutate the production database/audit/lock
  objects.
- Compile-fail/architecture tests prove selection modules cannot write `application_id` or
  `user_version`, construct a global lease, acquire a subsystem migration lock, or call the
  cutover participant without the borrowed exclusive global generation-1 capability/transaction.
- AST/HIR call-graph tests enumerate every selection-reachable provider constructor and fail unless
  the caller is the exact `SelectionProviderConstructionOwner`/lazy recovery allow-list; alias,
  re-export, macro and multiline bypass fixtures must fail.

### 18.2 Schema, watermark and trigger tests

- Seed all seven tables, including empty and non-contiguous rowid cases, and assert the exact sorted
  `(table_name, max_rowid, row_count)` vector.
- Assert all three derived counts and their watermark cross-checks.
- Migrate an existing candidate table and prove all old rows read `legacy-v1` without a semantic
  UPDATE.
- Reject wrong type/nullability/default/CHECK/hidden-state variants of `sample_schema`.
- Golden-test every exact column, default, CHECK, UNIQUE, FK, explicit/implicit index and
  production/test candidate-code variant in §8.3.
- Golden-test all twelve table/index canonical SQL hashes and all four schema SQL-digest hashes in
  §8.4 against full canonical-byte fixtures.
- Project both complete same-runtime global mode catalogs onto the seven legacy tables/five
  indexes/registered triggers and prove byte/hash equality; alter each overlapping object and prove
  `GlobalSelectionCatalogProjectionMismatch`.
- Golden-test all 21 names, targets, operations, canonical SQL strings, sorted registry JSON and
  hash.
- Prove only exact pristine and exact 14-trigger pre-cutover states are eligible; independently
  seed every orphan `sample_schema`/cutover-trigger/partial object combination and require
  `OrphanLegacyCutover`.
- Independently remove, add or alter every trigger and prove startup fails.
- Reject comments, extra statements, invalid quoting and literal-changing SQL canonicalization
  variants.
- Prove all INSERT/UPDATE/DELETE operations on six graph tables fail after cutover.
- Prove outcome UPDATE/DELETE fail; eligible missing legacy T0/D1 succeeds; wrong phase, invisible
  candidate, non-legacy candidate and duplicate phase fail.

### 18.3 First activation and concurrency

- Seed a production-shaped `0/0` v1 database, invoke the offline global generation-1 owner, and
  prove its one exclusive transaction atomically commits the complete global catalog, `STSA/1`,
  selection cutover DDL and the exact recovery envelope.
- Prove ordinary startup rejects a fresh or nonempty `0/0` database with
  `OfflineGlobalMigrationRequired`, and rejects `STSA/1` without the first envelope, with zero DDL
  and zero PRAGMA writes.
- Inject failure after every DDL/envelope substep and prove either complete rollback or the complete
  atomic state.
- Race two first activations and prove UUID/clock calls occur only after `BEGIN IMMEDIATE` and the
  in-transaction absence/catalog/input recheck; the loser allocates nothing before its lock.
- Race a legacy writer against `BEGIN IMMEDIATE`: a write committed before the lock is included in
  the snapshot; a later graph write is blocked then denied.
- Prove no provider object is constructed or called while the exclusive global
  lease/audit/SQLite locks are held.
- Race an ordinary shared-lease process against the offline command and prove exclusive migration
  cannot start until the process exits; prove no shared-to-exclusive upgrade path exists.
- Prove database startup no longer silently creates or repairs legacy triggers.

### 18.4 Recovery and reuse

- Exercise each crash point in Section 11 and compare final envelope, manifest, audit and receipt
  bytes with a no-crash golden run.
- Prove startup invokes config activation recovery first, constructs a zero-I/O lazy gateway, then
  drains all non-config recovery classes before any new-work provider/poll/ingress path.
- Activate A, persist ingress/generation/outcome partials under A, activate B, then prove each
  partial resolves A's historical registry entry/typed snapshot while all new work accepts only B.
- Prove each receipted activation appears exact-once in the historical registry at its receipt
  commit; envelope/Prepared/manifest/Committed-only activation state is not registry authority.
- Independently alter activation run ID, config hash, registry-entry hash, receipt/audit link,
  database binding, partial subject/envelope hash and request seal; each must fail before provider
  construction and must not fall back to current.
- Compile-fail tests prove `VerifiedCurrentConfigActivation` cannot enter
  `for_exact_recovery`, `VerifiedRecoveryConfigActivation` cannot enter `for_new_work`, and neither
  can be caller-constructed, cloned, serialized or converted.
- Same config restart returns the original run ID, `activated_at`, `effective_from`, receipt and
  snapshot with zero new UUID/clock calls.
- Envelope-only and manifested-unreceipted states recover the same IDs and chronology.
- Duplicate envelope, manifest, receipt, DB-only, audit-only and nested-hash conflicts fail closed.
- A future-effective exact activation returns typed wait without creating a new run.
- Expired artifact and malformed activation evidence fail before provider work.

### 18.5 Later activation and drain

- Append valid legacy T0/D1 rows after first activation, activate a new config and prove it copies
  byte-identical original cutover JSON/hash rather than current counts.
- Give any later activation a different cutover byte/hash and prove startup fails.
- Prove `Pending -> Complete` after the final required phase and prove `Complete -> Pending` is
  impossible.
- Prove the drain is not started at `Complete`, while the trigger registry/hash remains unchanged.
- Prove pending v1 inbox is reported only as `legacy_excluded`.
- Prove v1 due queries cannot see v2 samples and v2 due queries cannot see v1 candidates.
- Exercise every §16.3 drain crash state and prove exact claim/request/run reuse, zero refetch after
  staged evidence, per-operation lineage/schema revalidation and Rule 2.3 rejection/confirmation
  behavior.

### 18.6 Monitor integration

- Run normal, review, test and canary startup fixtures and assert the strict order
  `process bootstrap -> config-activation recovery/registry verification/current selection ->
  non-config recovery -> new-work provider construction`; no provider is constructed before its
  corresponding exact historical or current capability exists.
- Run at least two selection-capable binaries and prove neither can supply a different mode or
  database manager after the library bootstrap owns real `args_os`.
- Activation failure produces the documented non-zero exit and zero provider/ingress/generation
  calls.
- Successful startup passes the opaque capability into ingress and never reloads current config for
  an already-bound fact.
- The old v1 evaluation adapter and public legacy outcome writers fail the executable caller
  allow-list.
- A first envelope followed by a crash before Prepared is accepted only by roll-forward recovery;
  rollback fixtures cannot restore a pre-cutover binary/database once that envelope committed.

### 18.7 Isolated live-canary evidence contract

Gate B must expose exactly one production canary argv form:

```text
monitor --selection-live-canary
```

Gate D runs exactly `./target/release/monitor --selection-live-canary`. The real executable's
strict CLI is parsed only by `bootstrap_selection_process()`; there is no canary alias and it is
not a generic `--database`, `--root` or mode override. The canary:

1. acquires the production shared global maintenance lease before pool construction and verifies
   the pinned production database/audit identities, exact complete production catalog and
   `STSA/1`;
2. runs config-first recovery/registry/current verification and the zero-provider-before-gate
   assertions;
3. issues one allow-listed **read-only** provider validation request through
   `SelectionProviderConstructionOwner::for_new_work`;
4. persists no selection fact, sends no push, and has no order capability; architecture tests make
   those side effects uncallable in canary mode;
5. writes a durable canary evidence artifact outside the selection authority tables containing
   executable SHA, this design SHA, process/database/global-catalog/runtime hashes, activation and
   config IDs, provider/source, request hash, batch ID/hash, provider/observed times, freshness
   verdict, startup-order trace hash, zero-write row-count deltas and exit status;
6. reparses and hashes that artifact, and exits non-zero on any missing field, stale/invalid data,
   identity mismatch, selection-table delta, provider-before-gate edge, push/order attempt or
   receipt mismatch.

The release PR records the exact checked-in canary command exposed by Gate B, start/end database
and audit high-water hashes, artifact path/hash, stdout/stderr hash and non-secret provider
receipt. Production data is never copied into the TEST_CODE database, and test evidence cannot
satisfy this live contract. Until the command exists and this evidence passes, Gate D remains
blocked.

## 19. Old Modules

| Module | Decision | Reason |
|---|---|---|
| bin-owned CLI/mode/database bootstrap | Reject and replace | Real `args_os`, opaque binding, manager selection and `OnceLock` must share one private library owner |
| `src/selection/config_activation_v2.rs` | Adopt and deepen | Keep deterministic typed hashing; split fixed material from owner-only identity/snapshot finalization |
| `src/selection/persistence_v2.rs` | Adopt and deepen | Reuse durable choreography through an already-locked internal path; remove public config commit |
| `src/database/selection_v2_repository.rs` | Adopt and deepen | Add envelope-aware verified lookup, receipt-backed historical activation registry, first-cutover carrier loading and locked resume |
| `src/database/selection_v2.rs` | Adopt | Reuse v2 envelope/manifest/receipt schema and invariants |
| `src/selection/schema_v2.rs` | Adopt and tighten | Keep preimages; require exact seven-table and 21-trigger validation/golden vectors |
| `src/selection/audit.rs` | Adopt and bind | Preserve lock/session integrity, but obtain the fixed production or invocation-unique test audit root only from the installed process binding |
| `src/database/selection.rs` | Adopt, freeze and narrow | Move schema cutover to the owner; add `sample_schema`; make outcome drain private |
| `src/database/mod.rs` | Adopt and change migration order | Stop pre-owner trigger mutation; expose fixed maintenance/quiesce ownership internally |
| `src/bin/monitor/main.rs` | Adopt and reorder | Recovery and activation must gate provider construction and every selection mode |
| `src/bin/monitor/selection_shadow.rs` | Replace acquisition adapter; retain private drain owner | Current path writes v1; only legacy T0/D1 settlement survives cutover |
| `src/selection/pipeline.rs` v1 write owner | Freeze and delete after caller proof | New facts enter schema-v2 only |
| `src/selection/outcome.rs` | Preserve only behind private legacy drain | Existing committed v1 candidates still require T0/D1 |
| `src/opportunity/news_outcome.rs` | Reject and delete after caller audit | It must not retain a second legacy outcome owner |
| `src/bin/selection_backtest.rs` legacy query | Replace with receipt-verified v2 report | Public v1 visibility query would bypass cutover isolation |

## 20. Rollback

The irreversible boundary is the first config-activation recovery-envelope commit, not its later
Prepared record, manifest or receipt.

Before that envelope commit, ordinary rollback is allowed only after locked verification that the
database is exactly pristine or exact pre-cutover (§9.3), the transaction rolled back, and no
cutover envelope/audit record/manifest/receipt exists. A transaction failure before commit restores
that exact prior catalog; code rollback may then select the parser-compatible prior binary and its
verified backup.

Before the offline command becomes eligible to mutate anything, the exclusive global owner creates
and durably verifies a `PreCutoverRollbackReadinessReceipt`. Its fixed preimage binds:

- pinned source database device/inode/size identity and the immutable backup/snapshot artifact
  SHA-256;
- source `application_id=0`, `user_version=0`, the complete source global catalog/runtime hash,
  all application row counts and the exact legacy catalog/14-trigger hashes;
- validated audit-prefix high-water/last-record hash and audit backup artifact SHA-256;
- proof that no config envelope, Prepared/Committed record, manifest or receipt exists;
- parser-compatible prior binary SHA and restore tool/version;
- approver, creation time, expiry and the exact offline migration invocation hash.

The global owner revalidates every field immediately before `BEGIN IMMEDIATE`. The receipt
authorizes restore only while the first envelope/global transaction has not committed and all
database/audit high-waters still match. It is automatically ineligible at the irreversible commit;
there is no operator flag that can revive it.

After the first cutover envelope commits, destructive rollback is prohibited:

- do not drop or replace any of the 21 triggers;
- do not remove `sample_schema`;
- do not delete or rewrite the cutover envelope, audit records, manifest or receipt;
- do not reopen v1 acquisition/evaluation;
- do not select a legacy binary that cannot parse all new permanent audit phases.

This prohibition applies even if the process died before Prepared or receipt: the only permitted
action is exact roll-forward recovery from that envelope. Behavior rollback disables new v2
ingress and generation, retains verified read/recovery support, and continues only the private
legacy T0/D1 drain after global recovery. A file restore, trigger removal, column removal or old
binary at/after the first envelope would erase or misparse durable intent and is not an ordinary
rollback. Any extraordinary full-file restoration requires the separately gated controlled
exception/database rollback protocol, independent approver, complete audit proof and proof that no
post-envelope external or exchange-visible write occurred.

Release rollback instructions must name the last parser-compatible binary SHA and the operator
steps to verify global `STSA/1`, complete mode catalog, trigger registry, audit chain and receipt
high-water before restart.

## 21. Validation Commands

This Gate A document is validated without compiling:

```bash
test "$(grep -Ec '^[[:space:]]*`CONFIG_ACTIVATION_OWNER_DESIGN_SHA256 = [0-9a-f]{64}`[[:space:]]*$' \
  docs/superpowers/specs/2026-07-28-config-activation-owner-design.md)" -eq 1
sed '/^[[:space:]]*`CONFIG_ACTIVATION_OWNER_DESIGN_SHA256 = /d' \
  docs/superpowers/specs/2026-07-28-config-activation-owner-design.md \
  | shasum -a 256
git diff --check -- \
  docs/superpowers/specs/2026-07-28-config-activation-owner-design.md \
  docs/business_rules.md
bash tools/compliance/lib/check_business_rules.sh
```

The independently computed `shasum` value must byte-equal the declaration at the top of this
document. The reviewer runs the computation twice in separate commands and records both outputs;
the declaration line is the only excluded line.

Gate B/C/D implementation is not complete until all of the following pass:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
cargo test --test selection_config_activation_owner -- --test-threads=1
cargo test --test selection_legacy_cutover_cross_process -- --test-threads=1
cargo test --test selection_v2_crash_recovery -- --test-threads=1
bash tools/compliance/check.sh
```

Gate D additionally requires the repository coverage thresholds, the exact §18.7 isolated
live-canary evidence, auditor sign-off and an offline production-shaped global generation-1
cutover rehearsal with no provider calls before the gate.

## 22. PR Evidence Contract

The implementation PR must contain:

```markdown
### Refs
- spec: `docs/superpowers/specs/2026-07-28-config-activation-owner-design.md`
- parent spec: `docs/superpowers/specs/2026-07-28-selection-evidence-closure-design.md §5.1, §7.3`

### Data-Redlines
- [2.1] No mock/fallback activation or cutover evidence in production
- [2.2] Missing schema/evidence remains an explicit failure
- [2.3] Legacy drain validates price, continuity, calendar and corporate-action evidence; >20%
  adjacent moves require exact immutable manual confirmation
- [2.4] Activation/artifact chronology and expiry verified before ingress
- [2.5] One unforgeable process binding physically isolates production/test storage, locks, symbols
  and sinks; the library-private owner alone reads real args and selects `DatabaseManager`; the
  global schema owner alone supplies shared/exclusive maintenance capabilities and owns `STSA/1`
- [2.7] Prepared/Committed audit and receipt chain retained
- [2.8] Quiesce, verify, persist and recover operations perform their named effects
- [2.10] BR-174/BR-177 exact trigger, filter, ordering and reuse rules

### OldModules
| module | adopt/reject | reason |
|---|---|---|
| `src/selection/config_activation_v2.rs` | adopt/deepen | deterministic typed config material |
| `src/database/selection.rs` | adopt/freeze | immutable v1 cutover and private drain |
| `src/bin/monitor/selection_shadow.rs` | replace/retain drain only | remove v1 acquisition |

### Threshold-Proof
- No numeric strategy/risk threshold changed.
- Lock waiting and startup failure semantics are safety choreography, not trading thresholds.

### Business-Rules
- BR-174
- BR-176
- BR-177
- BR-178
- BR-179

### Validation
- Exact command output and pass counts for every command in §21
- Frozen design SHA recomputed twice and matched to executable/envelope/receipt bindings
- Global `STSA/1`, complete mode catalog/runtime receipt and legacy-projection equality matrix
- Shared-before-pool lifetime lease, offline-exclusive cutover and no-upgrade evidence
- Golden seven-table watermark vector
- Golden bound-mode seven-table schema catalogs, twelve SQL object hashes and orphan-cutover matrix
- Golden 21-trigger registry JSON/hash
- Crash matrix and cross-process writer-race evidence
- Monitor config-first recovery, lazy exact gateway and zero-provider-before-gate evidence
- Cross-bin real-argv bootstrap/manager isolation and caller-forgery compile-fail evidence
- Historical A -> current B partial-recovery matrix with registry/receipt/database/request seals
- Legacy drain claim/stage crash and Rule 2.3 evidence matrix
- §18.7 live-canary artifact/hash, zero selection-write delta and zero push/order capability proof

### Rollback
- Parser-compatible binary SHA
- Pre-envelope `PreCutoverRollbackReadinessReceipt` and restore procedure
- First-envelope-and-later disable-generation plus mandatory roll-forward procedure
- Trigger/audit/receipt verification commands
```

## 23. Gate Progression

This document is the independently reviewed Gate A design artifact. Gate A is PASS because the
review recorded in §24 confirms:

- the zero-argument owner is the sole production seam;
- the one public zero-argument process bootstrap owns real `args_os`, the private binding/
  `DatabaseManager` selection and cross-bin production/test isolation, and obtains a shared global
  maintenance lease before pool construction;
- the sole global owner controls the complete mode catalog and `STSA/1`; first cutover is an
  internal participant in its exclusive transaction, while ordinary startup is DDL/PRAGMA-free;
- provider constructors are below the closed capability/call-graph boundary;
- the exact DDL/schema/trigger/watermark/orphan contract has no ambiguity;
- DDL+envelope atomicity is implementable with the existing SQLite/audit choreography;
- all old writer callers are accounted for;
- config activation is the global recovery root and the monitor gate precedes every provider path;
- every old partial resolves its own receipt-verified historical activation/database/request seal,
  while new work can consume only the current activation;
- the private legacy drain has durable claim/stage recovery and per-operation Rule 2.3 validation;
- rollback becomes roll-forward-only at the first envelope commit;
- no blocking objection remains.

Gate B requires implementation plus explicit failure paths and the complete test matrix. Gate C
requires all compliance checks. Gate D requires coverage, production-shaped rehearsal, live-data
validation and auditor sign-off. Gate B has not started. Until Gate D, logs and documentation must
say `config_activation_owner=gate_a_pass` or the actual reached gate, never `production_ready`.

## 24. Independent Gate A Review

The independent review covered interface authority, whole-database identity, physical mode
isolation, legacy catalog/cutover, crash recovery, historical activation authority, provider
ownership, lock order, live validation and rollback.

| Finding | Initial severity | Resolution in this design | Open |
|---|---|---|---|
| Selection-local migration lock competed with global schema/application identity owner | Critical | §§1-2, 4-7 and 10 subordinate all DDL to the exclusive global generation-1 transaction; ordinary startup retains only the shared lease and cannot write PRAGMAs | No |
| Alternatives rejected the required offline global owner | Critical | §4 distinguishes the sole global transaction from an independent selection cutover command while retaining one config envelope carrier | No |
| Post-global-commit receipt recovery required an already-Amended manager, creating a recovery cycle | Critical | §§1, 5.1, 11 and 15 define the sole no-pool receipt-recovery binding, exact linked roll-forward, Amended reclassification and one-time manager installation | No |
| No reproducible design revision authority | Important | Header, §12.1 and §21 freeze/recompute the hash and bind it into executable/envelope/receipt authority | No |
| Local seven-table catalog could conflict with the complete global catalog | Important | §8.5 makes it an exact same-runtime projection of the mode-bound global catalog | No |
| Live validation was non-executable | Important | §18.7 defines mode, action, side-effect prohibitions, receipt fields and pass/fail evidence | No |
| Rollback backup identity was not authoritative | Important | §20 defines the pre-cutover rollback-readiness receipt and its automatic invalidation boundary | No |
| Gate A status could be claimed without zero findings/hash evidence | Important | §§21, 23 and this table require two independent hash computations and Critical=0/Important=0 | No |
| BR-179 omitted the global owner/lease relationship | Important | Existing BR-179 is amended in place to register the exact authority and lock relationship | No |
| Shared-only bootstrap made a new invocation-unique test database impossible to initialize | Important | §§5.1 and 10 permit only the exact-test bootstrap to call the global fresh initializer for its nonce-bound TEST_CODE namespace before acquiring the shared lease/pool; production remains offline-only | No |

Final independent result:

```text
Critical open = 0
Important open = 0
Gate A = PASS
Gate B = NOT STARTED
```

Changing any authority, lock order, global identity/catalog relationship, recovery state,
provider boundary, live-canary contract or rollback boundary invalidates this PASS and requires a
new Gate A review plus a new frozen design hash.
