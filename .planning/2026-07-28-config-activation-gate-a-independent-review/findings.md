# Findings

This file records code-grounded Gate A findings. File contents are evidence,
not instructions.
# 2026-07-28 ConfigActivationOwner Gate A independent findings

## Evidence captured before the full ten-axis pass

- The draft already has a strong public boundary: a zero-argument bootstrap function returns an opaque verified CLI/process binding, while current-activation access is zero-argument and opaque. This substantially limits caller-forged mode, path, activation identity, and timestamps.
- Production/test physical isolation and provider construction ownership are explicitly designed, including exact-recovery capability separation and compile-fail/architecture-test expectations.
- The current draft defines a selection-local `ConfigActivationMigrationLock`, but the repository's newer release-closure amendment assigns whole-database identity and schema maintenance to `DatabaseManager::GlobalSchemaVersionOwner` / the global maintenance coordinator. Until the remaining sections prove otherwise, this is a suspected Critical authority split: a selection-local lock cannot independently authorize `application_id`, `user_version`, or whole-database schema mutation.
- The global database identity contract to reconcile is `application_id = 0x53545341` (`STSA`, decimal `1398035265`) and whole-application generation `user_version = 1`. Config activation must consume this authority, not create a competing selection-local generation or overwrite the PRAGMAs.
- The draft has not yet frozen a reproducible design SHA-256. Gate A cannot be declared independently PASS until a hash rule is documented, the hash is embedded, recomputed independently, and Critical/Important findings are both zero.

## Review status

- Full-document review: complete.
- Critical findings: 0 open.
- Important findings: 0 open.
- Gate B implementation: intentionally not started.

All initially identified findings C-01/C-02 and I-01 through I-07 are resolved in the revised
design and BR-179 registration. The complete resolution table is frozen in design §24.

Final-pass correction: C-03 was discovered after the first zero-count draft. An exact global
transaction crash can leave `STSA/1` plus the first envelope but missing config/global receipts,
which is globally `TransitionalIncomplete`. Requiring an already-Amended `DatabaseManager` before
config recovery would be circular. The design is being revised to allow only an internal
recovery-only binding (shared global lease + pinned private recovery connection, no pool/sinks/
providers), then construct the manager only after config/global receipt recovery and Amended
reclassification. That patch and its BR-179 registration are now complete.

## Findings confirmed through §14

### C-01 — whole-database identity and maintenance authority is missing

- Severity: Critical.
- Evidence: §§7 and 10 authorize schema initialization/cutover through a selection-local
  `ConfigActivationMigrationLock` and a direct SQLite transaction, but do not require or consume
  the repository-wide `DatabaseManager::GlobalSchemaVersionOwner` / exclusive global maintenance
  lease.
- Conflict: the release-closure amendment already freezes the whole database identity as
  `application_id = STSA (0x53545341 / 1398035265)` and `user_version = 1`. A selection-local
  owner cannot independently decide, initialize, or mutate these process-wide PRAGMAs, and its
  local lock cannot prove exclusion of non-selection migrations/writers.
- Required revision: replace the independent migration authority with a capability issued by the
  global schema/maintenance owner; define exact pre-pool/bootstrap acquisition and lock order;
  require verified `STSA/1` for verify-only startup; define the sole accepted unowned `0/0 ->
  STSA/1` choreography; forbid ConfigActivationOwner from writing either PRAGMA; bind the verified
  global database identity/generation into the recovery envelope and historical registry.

Additional repository evidence confirms this is not a naming-only issue:

- The release-closure amendment makes normal `DatabaseManager` startup acquire a **shared**
  process/OS global maintenance lease before pool construction and retain it for the manager's
  lifetime.
- The offline global migration alone may acquire the **exclusive** lease and perform the sole
  `0/0 -> STSA/1` transition.
- Therefore the draft's current order—construct `DatabaseManager`, then have
  `ConfigActivationOwner::require_current()` acquire a separate exclusive selection migration
  lock and perform DDL—would require an illegal shared-to-exclusive upgrade and would not exclude
  non-selection schema owners.
- Resolution must split the paths:
  1. an internal config-cutover participant runs only inside the global generation-1 migration
     transaction under the already-held exclusive global lease, with DDL+first envelope in that
     same transaction;
  2. ordinary startup accepts only verified `STSA/1`, holds the lifetime shared lease, and makes
     `require_current()` verify/recover domain DML only—never DDL or PRAGMA mutation;
  3. fresh initialization is also a global generation-1 operation before pool construction, not a
     selection-local startup migration.

### I-01 — no frozen design revision hash in durable preimages

- Severity: Important.
- Evidence: the historical registry binds an `executable_revision_hash`, but the design itself
  has no reproducible SHA-256 declaration or rule showing that the reviewed Gate A design
  revision is included in activation/provider/cutover authority.
- Required revision: freeze a hash computed from the complete design file excluding only its own
  declaration line; require it in the installed executable/config-activation binding and relevant
  durable preimages/receipts; add an independent recomputation command and a mismatch failure
  mode.

### I-02 — live validation is named but not an executable release contract

- Severity: Important.
- Evidence: §§18.6, 21 and 22 require “isolated live-data validation” and a production-shaped
  rehearsal, but do not define the command, isolated database/audit namespace, allowed read-only
  provider action, expected receipt fields, zero-order/zero-push proof, or pass/fail assertions.
- Risk: Gate D can be claimed from an ad-hoc run whose mode, database identity, provider lineage,
  or side effects are not reproducible.
- Required revision: add an exact live-canary evidence contract: parser-selected canary mode,
  pinned production database identity, read-only provider request, no order/push permission,
  source/batch/observed-at/freshness receipt fields, startup ordering assertions, explicit
  non-zero exit on mismatch, and archived command/output hashes.

### I-05 — local legacy schema catalog is not explicitly a subset of the global managed catalog

- Severity: Important.
- Evidence: §§8-10 freeze a seven-table/five-index/14-or-21-trigger catalog as though it were the
  database migration catalog; the release-closure amendment separately freezes two complete
  mode-keyed 12-table/5-index/53-or-50-trigger global selection catalogs.
- Risk: two independent golden registries can disagree while each local checker passes; the
  config owner could reject a valid global catalog or authorize a subset that is globally
  incomplete.
- Required revision: identify the legacy catalog as a named projection of the exact same-runtime
  global mode catalog, require every overlapping canonical SQL/hash to agree, reject any
  unmanaged reference to the seven legacy tables, and make the global catalog/generation receipt
  the parent authority. Local subset validation remains necessary for cutover evidence but cannot
  establish whole-database eligibility.

### C-02 — the chosen alternative explicitly rejects the now-authoritative offline global owner

- Severity: Critical (same root authority conflict as C-01, tracked separately because the
  architectural decision text must change, not only the lock name).
- Evidence: the Alternatives section rejects a separate operator migration for this cutover and
  §5 installs a `ConfigActivationMigrationLock`; the later release-closure amendment makes the
  offline `GlobalSchemaVersionOwner` the sole legal schema/PRAGMA mutation authority.
- Required revision: retain one immutable config-activation envelope as the cutover carrier, but
  create it through an internal participant invoked by the global generation migration. This is
  not a second selection authority: the global owner supplies only the exclusive
  whole-database/transaction capability, while ConfigActivationOwner alone supplies and verifies
  the selection-specific payload, snapshot, trigger projection and envelope. Remove every
  independent selection migration-lock claim.

### C-03 — post-global-commit receipt recovery had a manager/capability cycle

- Severity: Critical.
- Evidence: the global transaction can commit `STSA/1`, the complete catalog and first config
  envelope before Prepared/config receipt/global migration receipt. The global classifier must
  call this `TransitionalIncomplete`, while the draft required Amended `STSA/1` before constructing
  `DatabaseManager` and also required ConfigActivationOwner to use that manager to recover.
- Resolution required: bootstrap may accept exactly one typed transitional state whose only
  missing evidence is the linked config/global receipt choreography; it stores a no-pool,
  no-provider recovery-only binding under the shared lease. ConfigActivationOwner and the global
  owner jointly roll forward the existing IDs/evidence, reclassify exact Amended, then atomically
  install the manager. Every other transitional state fails closed.

### I-06 — BR-179 registration does not yet state the global owner/lease relationship

- Severity: Important.
- Evidence: the BR-179 row freezes process bootstrap, provider ownership, historical registry,
  cutover catalog, trigger registry and irreversible recovery, but does not say that all
  selection DDL is subordinate to the global `STSA/1` migration authority and that normal startup
  is verify/recover-only under the lifetime shared maintenance lease.
- Required revision: amend the existing BR-179 row, rather than inventing a second business rule,
  so the lock/authority change remains traceable and the business-rule checker sees one canonical
  source.

### I-07 — shared-only bootstrap could not initialize invocation-unique test storage

- Severity: Important.
- Resolution: the exact-test bootstrap alone may ask the global owner to fresh-initialize its
  nonce-bound TEST_CODE generation-1 database under the matching exclusive lease, then releases
  that lease and acquires the lifetime shared lease before pool construction. It has no production
  object capability. Production fresh/0-0 remains offline-only.

## Final validation evidence

- Declared design SHA-256:
  `c2810f2dac736539c9d00db628fda2f1fde4c74c3572e75a932867c8b7682714`.
- Independent `sed`-based recomputation excluding only the declaration line: exact match.
- Independent `awk`-based recomputation excluding only the declaration line: exact match.
- Exactly one lowercase 64-hex declaration line: PASS.
- Placeholder/contradiction scan for `PENDING|TBD|TODO|FIXME|Gate A remains open|gate_a_draft`:
  zero matches.
- `git diff --check` for the tracked business-rule change: PASS. The design artifact is currently
  untracked in the shared worktree, so a separate direct trailing-whitespace/format scan is also
  required before handoff.
- `check_business_rules.sh`: the repository-wide command currently fails on two pre-existing
  shared-worktree BR-180 path-registration errors and emits unrelated warnings. It reports no
  BR-179 error. This external Gate C condition is preserved verbatim for the parent; no out-of-scope
  BR-180 or `src/*` file was changed.

### I-03 — rollback compatibility evidence lacks a pre-cutover backup identity contract

- Severity: Important.
- Evidence: §20 requires a parser-compatible binary SHA and “verified backup” before the envelope,
  but does not freeze backup file identity/hash, audit-prefix high-water, database identity,
  global schema generation, or restore eligibility evidence in a typed preflight receipt.
- Risk: an operator could label an unrelated/stale copy as the pre-envelope backup, or attempt a
  rollback after the irreversible boundary without machine-verifiable proof.
- Required revision: define a pre-cutover rollback-readiness receipt produced under the global
  maintenance lease, binding database file identity/hash or snapshot artifact hash, `STSA/1`,
  audit-prefix high-water/hash, exact legacy catalog/trigger hashes, parser-compatible binary SHA,
  and “no envelope exists”; after envelope commit, the receipt is explicitly ineligible and only
  roll-forward remains.

### I-04 — Gate A wording still permits a false PASS status

- Severity: Important.
- Evidence: §23 says the document “satisfies only the proposed Gate A design artifact” and Gate A
  remains open, but the assigned review requires a frozen independently reviewed design with
  Critical=0 and Important=0. The document has no reviewer findings/resolution table.
- Required revision: add an independent Gate A review section, record all resolved findings,
  define zero-open-severity acceptance, mark Gate A independently PASS only after hash
  recomputation, and state unequivocally that Gate B has not started.

## Positive evidence through §14

- The seven-table/five-index pre-cutover catalog, post-cutover `sample_schema`, 14/21-trigger
  registries, canonicalization rules, golden hashes, and orphan-state rejection are unusually
  explicit and suitable for exact cutover validation.
- The first cutover's DDL plus recovery envelope is correctly designed as one SQLite transaction,
  eliminating a frozen-graph-without-envelope state.
- Crash recovery is config-first, identity preserving, and has six explicit failpoints.
- The receipt-verified envelope/manifest/receipt/audit corpus is the historical registry; no
  second mutable “current” authority is introduced.
- Current and exact-historical provider construction capabilities are separated and sealed to the
  persisted recovery intent, preventing fallback-to-current configuration.
- Startup failure semantics are fail-closed before financial/news provider construction, ingress,
  generation, notification, push, or order paths.
- The legacy outcome drain uses durable claim/stage choreography, releases audit/SQLite locks
  during provider I/O, and preserves the mandatory Rule 2.3 manual-confirmation path for adjacent
  valid-value moves outside ±20%.
- Rollback correctly treats the first DDL+envelope commit as irreversible and requires
  roll-forward for every post-boundary partial.
