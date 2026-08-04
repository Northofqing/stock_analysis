# Exclusive Global Schema Maintenance Gate-B Slice

## Scope

- Extend only `src/database/global_schema_v1.rs`.
- Add private exclusive process/OS maintenance authority and tests.
- Do not open or mutate the production database in tests.
- Do not write PRAGMAs, run DDL, migrate, or edit bootstrap/repository/migration modules.

## Contract

- Shared-to-exclusive upgrade is forbidden with a typed error.
- Process-local shared/exclusive leases are mutually exclusive without races.
- The OS lock is acquired exclusively through the existing pinned namespace and hardened
  descriptor lifecycle.
- Lock contention is explicit and retryable; no unlocked fallback exists.
- Every descriptor remains no-follow, nonblocking and close-on-exec.
- Exclusive capability is non-cloneable, retains namespace/lock/process authority, and releases
  the OS lock before process authority.

## Validation

1. RED tests for same-process upgrade, inverse contention, cross-process shared contention,
   release order and exec inheritance.
2. GREEN exact `database::global_schema_v1::tests` target.
3. Non-test `cargo check --lib`.
4. `rustfmt --check` and scoped `git diff --check`.
5. Independent static review with zero Critical/Important findings.

## Rollback

Remove the exclusive lease/authority types and their tests. Existing shared inspection and all
database files remain unchanged.
