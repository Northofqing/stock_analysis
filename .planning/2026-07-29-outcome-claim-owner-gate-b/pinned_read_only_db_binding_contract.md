# Pinned Read-only Global Database Binding — Interface Contract Proposal

Status: interface requirements only. This slice MUST NOT edit
`global_schema_v1.rs` or `global_schema_catalog_v1.rs`.

## Problem

The outcome due read model currently proves the configured database path with
`canonicalize` plus metadata. That is useful evidence, but it is not a
descriptor-pinned authority: a path component or the database leaf can be
replaced between inspection and the SQLite read.

BR-178 therefore needs a narrow capability owned by the global schema module.
The outcome module must consume that capability; it must not recreate global
path traversal, maintenance-lock, or database identity logic.

Triggered red lines: 2.3, 2.4, 2.7 and BR-178 under 2.10.

## Required Public Shape

Suggested names are illustrative; the ownership constraints are mandatory.

```rust
impl GlobalSchemaVersionOwner {
    pub(crate) fn with_fixed_production_read_snapshot<T>(
        &self,
        use_snapshot: impl for<'snapshot> FnOnce(
            &mut PinnedGlobalReadSnapshot<'snapshot>,
        ) -> Result<T, GlobalSchemaV1Error>,
    ) -> Result<T, GlobalSchemaV1Error>;
}

pub(crate) struct PinnedGlobalReadSnapshot<'snapshot> {
    // private, non-Clone, non-Serialize, non-Default
}

impl PinnedGlobalReadSnapshot<'_> {
    pub(crate) fn object_binding(&self) -> GlobalDatabaseObjectBinding;

    pub(crate) fn with_query_only_sqlite<T>(
        &mut self,
        query: impl for<'connection> FnOnce(
            &'connection mut diesel::SqliteConnection,
        ) -> Result<T, GlobalSchemaV1Error>,
    ) -> Result<T, GlobalSchemaV1Error>;
}
```

The global owner may choose a different internal API, but callers MUST receive
only a higher-ranked callback capability. It MUST NOT expose a path, raw
descriptor, caller-supplied `SqliteConnection`, or constructible proof DTO as
authority.

## Capability Invariants

1. Production root and `data/stock_analysis.db` are compile-time fixed. No
   environment override or caller path is accepted.
2. Root, parent, database leaf and maintenance lock are traversed
   descriptor-relative with `O_NOFOLLOW`; the database must be one regular
   file.
3. The capability retains the pinned root/parent/database descriptors and the
   shared global-maintenance lease for the complete SQLite transaction and
   caller callback.
4. The SQLite connection is created by the global owner in read-only,
   `PRAGMA query_only=ON` mode and must be proven to address the same database
   object as the pinned descriptor. Reopening only by pathname is insufficient.
5. If WAL/SHM exists, the owner must either bind those sidecars into the same
   descriptor-pinned snapshot or fail closed with an explicit stable error.
   Ignoring live sidecars is forbidden.
6. Before returning the materialized result, the owner revalidates pinned
   descriptor identity, namespace identity, sidecar state and maintenance
   lease. Any change invalidates the whole result.
7. `GlobalDatabaseObjectBinding` contains immutable evidence only:
   scope, canonical fixed relative path, root dev/inode/mode, database
   dev/inode/mode, application ID and schema generation. Outcome-specific
   schema hashes and receipt high-water hashes remain in the outcome read
   model and are calculated inside the pinned SQLite transaction.
8. No capability may escape the callback lifetime or be replayed after the
   lease/descriptor is released.

## Outcome Read-model Adoption

After the global seam exists:

1. Delete local `canonicalize`/`metadata` authority construction from
   `selection_v2_read_model.rs`.
2. Run the complete receipt/audit/due query inside
   `with_fixed_production_read_snapshot` and `with_query_only_sqlite`.
3. Bind the global `object_binding()` evidence, ordered twelve-table schema
   hash, receipt high-water rowid/hash and locked audit high-water into
   `VerifiedOutcomeDueDatabaseBindingPreimage`.
4. Revalidate the exact binding after the per-subject lock and before claim
   persistence. A mismatch returns `Superseded`; it does not call the provider.

## Acceptance Tests Required from the Global Owner

- rejects a symlink in every root/parent/leaf component;
- rejects leaf replacement before, during and after SQLite snapshot creation;
- rejects a descriptor/path inode mismatch;
- rejects a non-regular database object and multi-link test object;
- rejects unbound or incomplete WAL/SHM state;
- proves the shared maintenance lease excludes an exclusive migration lease;
- proves SQL writes fail under the query-only connection;
- proves the callback result is discarded when post-read identity
  revalidation fails;
- compile-time/API test proves callers cannot supply a path, connection or
  object-binding proof;
- test and production namespaces remain physically isolated.

## Rollback

This proposal changes no production code. If the eventual global seam fails
validation, revert that implementation and retain BR-178 outcome settlement
behind the existing fail-closed database-binding blocker.
