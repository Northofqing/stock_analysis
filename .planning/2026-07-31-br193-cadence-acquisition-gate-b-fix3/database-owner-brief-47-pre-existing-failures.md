# Database Context Owner Brief — 47 Pre-existing lib-test failures

**Document target**: Database context owner (not BR-193 / BR-194 implementer).
**Scope**: Fix 47 pre-existing lib-test failures in
`database::global_schema_v1::tests::*` and
`database::selection_v2_repository::tests::*` so that BR-193 and BR-194 Gate C
(which require `cargo test --workspace --all-targets --all-features --
--test-threads=1` to report `0 failed`) can pass.
**Date prepared**: 2026-08-01
**Branch base**: `feat/event-scoped-selection-shadow` (current)
**Out of scope**: BR-193 / BR-194 implementation; selection logic; push
gate; durable delivery; any production runtime change.

## 1. Constraint recap (do not violate)

The following actions are forbidden per BR-193 spec §13.3:

- Silently fixing, weakening, or `#[ignore]`-ing the 47 pre-existing
  failures as part of BR-193 / BR-194.
- Adding `#[ignore]` to any test.
- Suppressing the assertion via `assert!` -> `let _ = ...` or similar.
- Marking the failing test "expected failure" in `cargo test` harness.
- Reverting any code unrelated to the assertion mismatch (the
  implementation must be brought to match the assertion, not the
  reverse).

The correct path is to bring the production code into a state where
each existing assertion passes, without modifying the assertion.

## 2. Root cause — first 2 failures (verified)

### 2.1 `v2_audit_with_absent_database_half_fails_closed_as_contradictory`

`src/database/global_schema_v1.rs:3755-3775`:

```rust
#[test]
fn v2_audit_with_absent_database_half_fails_closed_as_contradictory() {
    let fixture = TestFixture::new("selection-audit-v2-db-absent", 0, 0);
    let writer = fixture.pinned_audit_writer();
    writer
        .append(SelectionAuditRecord::new(/* V2IngressCommitted */))
        .expect("append contradictory TEST_CODE v2 audit record");
    let error = GlobalSchemaVersionOwner::for_test_code()
        .inspect_selection_with_audit_for_test(&fixture.root, &writer)
        .expect_err("v2 audit plus absent database must fail closed");
    assert!(matches!(
        error,
        GlobalSchemaV1Error::SelectionAuthorityContradiction { .. }
    ));
}
```

The fixture is built by `TestFixture::new` at
`src/database/global_schema_v1.rs:3445-3465`, which unconditionally
calls `Connection::open(&database)` on a path inside a fresh temp root
directory. The Drop impl cleans the temp root.

This means the fixture's database file **always exists** (with
`user_version = 0` and the catalog not installed). The test name and
assertion expect a database that does **not** exist.

`inspect_selection_with_audit_for_test` at line 936 delegates to
`inspect_selection_with_bound_paths` and ultimately to
`inspect_selection_with_optional_pinned_root` at line 972. That function
calls `open_pinned_sqlite_read_write` which opens the existing database
file. With audit present and database present, the inspection does
NOT take the contradiction branch and returns either `Ok(diagnostic)`
or some non-contradiction error, hence the assertion fails.

### 2.2 `missing_audit_returns_database_half_only_and_never_authoritative_absent`

`src/database/global_schema_v1.rs:3731-3752`:

```rust
#[test]
fn missing_audit_returns_database_half_only_and_never_authoritative_absent() {
    let fixture = TestFixture::new("selection-audit-missing", 0, 0);
    let writer = fixture.pinned_audit_writer();
    assert!(!writer.path().exists(), "audit evidence must start absent");
    let diagnostic = GlobalSchemaVersionOwner::for_test_code()
        .inspect_selection_with_audit_for_test(&fixture.root, &writer)
        .expect("missing audit is a diagnostic database half");
    assert!(matches!(
        diagnostic.database_half(),
        DatabaseHalfDiagnostic::AbsentDatabaseHalf(_)
    ));
    ...
}
```

The test name and assertion expect the database to be "absent" but
`TestFixture::new` creates the SQLite file. The inspection therefore
sees a database half (empty catalog) and does not return
`DatabaseHalfDiagnostic::AbsentDatabaseHalf`.

## 3. Root cause — remaining 45 failures (NOT investigated in depth)

The 45 `database::selection_v2_repository::tests::*` failures each
PASS when run individually but FAIL when run in the namespace. Pattern
mirrors the 2 above but the specific failure mode varies (assertion
mismatch on receipted / non-receipted / replayed outcomes). Likely
shared causes include:

- Process-local `OnceLock` / `thread_local!` state shared across tests.
- Temp-file race when `TestFixture::new` directories collide (the
  fixture uses `std::process::id()` + `AtomicUsize::fetch_add`; collisions
  are unlikely but the OS-level cleanup race is plausible).
- Wall-clock dependency (`Utc::now()` in fixture or subject).
- `Connection::open` interference between two fixtures (shared SQLite
  internal state for `unix_excl` or `psow`).

### 3.1 Confirmed cascading root cause

Running the namespace with `--test-threads=1` exposes a cascading
failure pattern:

1. The first test to panic is
   `outcome_receipt_survives_database_close_reopen_and_exact_owner_replay`
   at `src/database/selection_v2_repository.rs:9078`. Its panic message:

   ```
   persist upstream generation envelope: Audit("selection audit path
   invalid: audit namespace container mutated during locked session:
   /private/var/folders/3z/kj5q9h5x50j6zv9sr2zpdhzc0000gn/T/stock-analysis-selection-v2-repository-outcome-owner-file-reopen-35458-0/test")
   ```

   This is a real production path-validation failure in
   `locked_session` at `src/selection/audit.rs:454`, NOT a test-side
   issue. The audit namespace container path was mutated between
   writer creation and `locked_session()`.

2. `locked_session` acquires `process_audit_lock()` (process-global
   `Mutex`) at `src/selection/audit.rs:455`. When the first test
   panics while holding this lock, the `Mutex` becomes poisoned. Every
   subsequent `locked_session()` in the same process returns
   `SelectionAuditError::Lock("process audit mutex is poisoned"...)`
   and every test that depends on it fails with the same poisoned-lock
   error.

3. With `--test-threads=1`, the cascade hits 10 of 39 tests in
   `database::selection_v2_repository::tests`; the other 29 either
   pass or are positioned before the poisoned state propagates. With
   parallel execution (default), the cascade spreads faster and 45
   of the namespace's tests fail.

### 3.2 Two sub-causes, one fix path

The fix has two parts that must both be addressed:

**Part A: Stop the cascade.** `locked_session` at
`src/selection/audit.rs:454` must not let `process_audit_lock` poisoning
propagate across test runs. Either:

- replace the process-global `Mutex` with a per-namespace
  `RwLock`/`Mutex` keyed on the audit namespace path; or
- wrap the lock acquisition in a `catch_unwind` and recover (forbid
  production use of `catch_unwind`; this is test-only); or
- document a `clear_poison` recovery hook that tests call between
  fixture scopes.

**Part B: Fix the original `path mutated` cause.** The audit
namespace container path
`/private/var/folders/3z/kj5q9h5x50j6zv9sr2zpdhzc0000gn/T/stock-analysis-selection-v2-repository-outcome-owner-file-reopen-35458-0/test`
must NOT mutate between writer creation and `locked_session()`. Likely
a `TestAuditRoot::new` race where another test in the same process
deleted or replaced the parent temp directory while this test was
still mid-fixture. Inspect `TestAuditRoot` (in `src/selection/audit.rs`
or a sibling test helper module) for any non-isolated tempdir use,
shared atomic counter with insufficient uniqueness, or cleanup hook
that runs while a sibling test holds the path.

### 3.3 Investigation recipe

1. Run each of the 45 tests individually and record the actual failure
   assertion text. Cluster by failure mode. Expect one or two clusters
   total (most failures will be the cascade above).
2. Identify the FIRST test to panic in the namespace (likely
   `outcome_receipt_survives_database_close_reopen_and_exact_owner_replay`
   or a sibling that mutates the same tempdir).
3. Fix Part B (the first panic's actual cause).
4. Fix Part A (cascade containment) regardless of Part B's success, so
   that any future test panic cannot poison sibling tests.
5. Apply the minimal surgical fix; do NOT add `#[ignore]`.

## 4. Recommended fix approaches

### 4.1 Fix the cascading `process_audit_lock` poisoning first (Part A above)

Wrap `process_audit_lock().lock()` in `src/selection/audit.rs:455` so
that a poisoned lock does not cascade. Minimal pattern:

```rust
let process_guard = match process_audit_lock().lock() {
    Ok(guard) => guard,
    Err(poisoned) => poisoned.into_inner(),
};
```

Then Part B (the original path-mutated cause) can be diagnosed with
the cascade contained.

### 4.2 Fix the `audit namespace container mutated` cause (Part B above)

Inspect `TestAuditRoot::new` for any non-isolated tempdir usage. The
first test to panic (`outcome_receipt_survives_database_close_reopen_and_exact_owner_replay`)
constructs an `OutcomeEnvelope` fixture under
`stock-analysis-selection-v2-repository-outcome-owner-file-reopen-35458-0`
that conflicts with one or more sibling fixtures using the same parent
temp dir + atomic counter.

### 4.3 Add `TestFixture::absent(label)` variant

### Approach A: Add `TestFixture::absent(label)` variant

```rust
impl TestFixture {
    /// Variant that does NOT open or create the SQLite file. Use only
    /// for tests whose premise is that the database half is absent.
    fn absent(label: &str) -> Self {
        let root = /* same isolation logic as new() */;
        fs::create_dir(&root).expect("create isolated TEST_CODE root");
        Self { root }   // no Connection::open, no user_version seed
    }
}
```

Then change `v2_audit_with_absent_database_half_fails_closed_as_contradictory`
and `missing_audit_returns_database_half_only_and_never_authoritative_absent`
to `TestFixture::absent(...)`.

This is the minimal change: production code is unchanged; only the
fixture acquires a new variant and 2 tests switch. The
`TestFixture::new` fixture remains usable for the 47 - 2 = 45 tests
that need a real (empty) database.

### Approach B: Make the production code detect contradiction

In `inspect_selection_with_optional_pinned_root` at line 972, after
the database connection is open, if the database contains no
catalog (no `selection_v2_*` tables) and `audit_session.validated_records()`
returned at least one record, return
`GlobalSchemaV1Error::SelectionAuthorityContradiction`. This adds a
production code path that didn't exist before.

Drawback: introduces new production behavior for a state
(audit present, database file present but empty catalog) that was
previously impossible to construct via production paths. Worth a
separate design review; out of scope for the fix-only PR.

### Recommendation

Use Approach A for the 2 verified failures. Apply a similar minimal
fixture change (or the underlying cause) for the remaining 45 after
clustering their failure modes.

## 5. Mandatory PR template

```text
### Refs
- brief: .planning/2026-07-31-br193-cadence-acquisition-gate-b-fix3/database-owner-brief-47-pre-existing-failures.md
- progress entry: 2026-08-01 §13.3 root cause (47 pre-existing failures)

### Data-Redlines
- 2.1 Applies: no mock data, no silent fix of asserted state
- 2.5 Applies: TEST_CODE/production physical isolation preserved
- 2.7 Applies: audit evidence retained, no truncation

### OldModules
- TestFixture | adopt + add absent variant | isolates tests whose premise is absent DB
- inspect_selection_with_optional_pinned_root | retain | only add contradiction detection if Approach B chosen

### Threshold-Proof
No threshold change.

### Business-Rules
- BR-049, BR-139, BR-140, BR-194, BR-193 (this slice unblocks Gate C)

### Rollback
git revert <commit-sha>; 47 tests will FAIL again as before; BR-193
Gate C will re-block. No production data change.
```

## 6. Machine-checkable acceptance for the fix PR

```bash
cargo test --lib database::global_schema_v1::tests:: -- --test-threads=1
# expected: 0 failed (currently 2 failed)
cargo test --lib database::selection_v2_repository::tests:: -- --test-threads=1
# expected: 0 failed (currently 45 failed)
cargo test --workspace --all-targets --all-features -- --test-threads=1
# expected: 0 failed (currently 47 failed in these two namespaces; BR-194
#             and BR-193 spec tests still all pass)
```

After the fix PR merges, BR-193 Gate C and BR-194's documented Gate D
prerequisite (workspace sweep 0 failed) can both proceed.

## 7. Forbidden patterns

```rust
// FORBIDDEN:
#[ignore]
let _ = error;
assert!(matches!(error, _)); // weakening the original typed assertion
return Ok(());               // swallowing contradiction
panic!("placeholder");        // fake-pass pattern
fn inspect_selection_test_code_rehearsal(...) -> Result<_, GlobalSchemaV1Error> {
    Ok(Default::default())    // short-circuit for any reason
}
```

If the fix requires any of the above, the design must be re-reviewed
and the spec changed accordingly. There is no shortcut.

## 8. Hand-off

On PR merge:
- 47 pre-existing failures must be 0.
- `cargo test --workspace -- --test-threads=1` must report 0 failed.
- BR-193 spec §13.3 can be updated to remove the "Gate C blocker" note.
- BR-194 PR (already pushed to master) becomes Gate D-eligible.

On PR open but pre-review: please cc the BR-193 spec author so they
can confirm the fix does not invalidate §13.3.