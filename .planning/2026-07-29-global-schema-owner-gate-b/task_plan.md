# GlobalSchemaVersionOwner Gate B Slice

## Scope

- Add `src/database/global_schema_v1.rs`.
- Add the single crate-private module declaration in `src/database/mod.rs`.
- Keep tests private to the new module.
- Do not edit process bootstrap, selection-v2 modules, repositories, or migration binaries.

## Contract

- Production identity is fixed to manifest-root `data/stock_analysis.db`.
- Test identity is invocation-isolated and `TEST_CODE`-named; no caller-selected production path.
- Authoritative identity is exactly `application_id=1398035265`, `user_version=1`.
- Identity inspection is read-only and never initializes or migrates a database.
- Ordinary runtime obtains a shared process/OS maintenance lease before SQLite inspection and
  retains it in the verified capability for its lifetime.
- Symlinks, non-regular database/lock files, path replacement, lock contention, unmanaged,
  mixed, foreign, negative, and future identities fail with typed errors.

## Validation

1. RED: exact new-module test target fails before implementation.
2. GREEN: exact new-module tests pass.
3. `cargo fmt --check`.
4. Static scan confirms production code contains no identity-writing PRAGMA.
5. Independent review for Critical/Important findings.

## Rollback

Remove the new module/test file and the one module declaration. No database migration, data write,
or configuration rollback is required.
