# Findings

- BR-180 already registers the sole global owner and no-upgrade constraint.
- Existing shared acquisition already pins root/database-parent/lock-parent and opens the lock
  with `O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC`.
- `PROCESS_EXCLUSIVE_LEASE` exists but has no acquisition/RAII owner yet.
- `PinnedNamespace::open` does not open the database, so it can safely bind a fresh TEST_CODE
  namespace before database creation.
- The narrow slice must stop at lock authority; migration/PRAGMA/DDL are later participants.
- Process mutual exclusion uses one atomic exclusive reservation plus the existing shared
  counter. Exclusive acquisition rechecks the counter after its compare-exchange; a racing
  shared acquisition rechecks the exclusive reservation after incrementing and backs out.
- Exclusive field order encodes reverse release: custom `Drop` unlocks the OS lease, followed by
  lock descriptor, pinned namespace descriptors, and the process reservation.
