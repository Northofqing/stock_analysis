# Progress

- Pre-flight recorded.
- Frozen owner/amendment design sections re-read.
- Existing database module, dependencies, and no-follow implementation inventoried.
- Root approved the single crate-private module declaration.
- Added typed exact identity classification for `0/0`, exact `STSA/1`, mixed/foreign/negative,
  and future-generation matrices.
- Added fixed production binding plus private isolated `TEST_CODE` test binding.
- Added no-follow regular-file pinning, fixed lock-directory synchronization, process shared
  lease accounting, OS shared lock acquisition, and lifetime retention in the opaque capability.
- Added read-only SQLite identity inspection with before/after descriptor identity checks.
- Added unit and separate-process tests for exact identity, no rewrite, physical isolation,
  symlink rejection, missing DB behavior, lifetime locking, and retryable cross-process
  contention.
- First compile attempt found only a missing `Debug` derive on a test helper; fixed. Cargo was then
  stopped to release the shared artifact lock for the repository lane.
- Independent review reported 2 Critical and 3 Important findings. All five were addressed
  statically:
  - identity now comes from the retained no-follow database descriptor, eliminating path reopen
    ABA/TOCTOU;
  - WAL/SHM are pinned and revalidated, then fail closed with a typed blocker until a
    descriptor-bound sidecar snapshot reader exists;
  - the database descriptor is declared before the lease so it drops first;
  - the production method requires an unforgeable owner value rather than being a callable static;
  - test/production database and lock identities compare device/inode, rejecting hardlink aliasing.
- Added WAL/SHM fail-closed and hardlink-alias negative tests.
- Marked the intentionally not-yet-wired owner entry point as a separate gated bootstrap
  integration point so this isolated slice does not create `dead_code` warnings or pretend the
  runtime already invokes it.
- `rustfmt --edition 2021 src/database/global_schema_v1.rs`: PASS.
- `git diff --check -- src/database/global_schema_v1.rs src/database/mod.rs`: PASS.
- Static production scan found no SQLite path reopen/immutable URI or identity-writing PRAGMA.
- Exact scoped tests: `9 passed; 0 failed; 1 ignored` (the ignored test is the explicit
  cross-process child helper).
- File frozen after validation while the repository lane runs its 25 scoped tests.
- Follow-up review confirmed the original five findings were materially addressed but found one
  remaining Critical and two Important issues:
  - separately pinned absolute lock/database paths do not prove one retained root/data namespace
    across an ancestor rename ABA;
  - isolated test inspection must not open or inspect production paths, even to compare inodes;
  - opening a FIFO/device before `fstat` can block startup.
- Returned to Gate B. Planned correction is descriptor-relative `openat` beneath retained pinned
  root/parent descriptors, single-link TEST_CODE objects without production inspection, and
  `O_NONBLOCK` before strict regular-file checks.
- Implemented retained `PinnedRoot` / database-parent / lock-parent descriptors and opened the
  database, lock, WAL, and SHM only by single-component `openat` beneath those descriptors.
- Added final retained-root/parent revalidation and retained the namespace in the verified
  capability; lock and database can no longer come from independently replaceable absolute
  namespaces.
- Removed all test-mode production-object inspection. TEST_CODE database, lock, and present
  sidecars now require `nlink == 1`; namespace naming remains invocation-isolated.
- Added `O_NONBLOCK` to every descriptor-relative open before strict `fstat` regular-file checks.
- Added deterministic root-rename ABA and database/lock FIFO regression tests.
- Post-fix exact scoped tests: `11 passed; 0 failed; 1 ignored` (explicit child helper).
- Post-fix `rustfmt --check`, scoped `git diff --check`, and forbidden-production-inspection scans:
  PASS.
- Final rereview reduced findings to `0 Critical / 1 Important`: descriptors were not atomic
  close-on-exec and could leak the shared flock into an exec child.
- Added platform-exact `O_CLOEXEC` to every descriptor-relative and absolute-root open.
- Added direct `FD_CLOEXEC` assertions for every retained database/root/parent/lock descriptor plus
  a real `/bin/sh` exec regression proving the child cannot retain the shared flock.
- Final exact scoped tests: `12 passed; 0 failed; 1 ignored` (explicit child helper).
- `cargo check --lib`: PASS. The module has no non-test warnings; unrelated in-progress modules
  still emit repository-wide dead-code warnings.
- Final module SHA-256:
  `735b92edd9386a6a42603ac463b1acba74d79c75978be718564b821da4542d7c`.
