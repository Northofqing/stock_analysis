# Progress — Global Schema Amendment C2/C3

- Pre-flight emitted with impacted paths, red-line rules, validation and rollback.
- Read repository engineering instructions and the planning-with-files,
  codebase-design and TDD skills.
- Created isolated parallel-task planning files; no shared active-plan pointer changed.
- Read the complete release-closure amendment and the first-activation sections that depend on
  §4.1.
- Recorded the frozen lock order, capture/revalidation evidence, authority matrix and
  old-module dispositions.
## 2026-07-29

- Recovered the isolated C2/C3 plan and amendment §4.1 constraints.
- Inventoried the global exclusive owner, catalog snapshot, audit session and
  migration CLI seams.
- Confirmed the implementation can compose existing global and audit locks;
  no new lock primitive is required.
- Cargo remains intentionally unrun while adjacent agents modify shared
  outcome/review paths.
- Read the concrete audit-session lifecycle and migration CLI. Confirmed the
  current CLI lock order is non-authoritative and the audit session already
  supplies both first capture and finish-time chain revalidation.
- TDD RED (static, Cargo intentionally withheld): added
  `owner_issued_capture_reads_all_twelve_selection_row_counts_from_one_connection`.
  It currently names the missing owner token, capture function and private
  row-count accessor, so the contract cannot pass before the owner-only seam
  exists.
- TDD GREEN (static): added the non-forgeable
  `SelectionCatalogCaptureAuthority`, made `CatalogSnapshot` non-`Clone` with
  private fields, and implemented same-connection capture of runtime identity,
  PRAGMA identity, catalog/dependencies, attached schemas, all legacy counts,
  all 12 selection counts and payload-schema contract. Cargo remains deferred,
  so this is not yet runtime-verified.
- TDD RED (static): added
  `owner_selection_inspection_captures_database_and_audit_under_one_authority`.
  The contract requires an owner-issued TEST_CODE inspection to compose the
  global exclusive guard, real locked audit session and same-connection
  catalog capture, returning only a detached diagnostic after revalidation.
- TDD GREEN (static): implemented the owner composition and retained private
  snapshot. `rustfmt` and `git diff --check` pass. A static production-mutation
  scan found no owner-path DML/DDL/PRAGMA writes; all observed `pragma_update`
  calls are test fixture setup.
- TDD RED/GREEN (static): empty `0/0` catalog capture now preserves the frozen
  legacy and 12-table zero-count registries and classifies only as an absent
  database half.
- TDD RED/GREEN (static): audit object missing now returns
  `DatabaseHalfOnly`; audit-v2 plus database-absent is a typed fail-closed
  contradiction.
- Added the detached authority-state matrix. Final five-payload state remains
  receipt-verification-pending until exact same-transaction receipt
  reconciliation is ported.
- Deleted the old migration binary's independent Diesel/audit/filesystem
  preflight. The binary delegates to the owner façade; production apply fails
  before I/O and test mode consumes an owner-issued TEST_CODE temporary copy.
- Added the Gate-B design note
  `docs/superpowers/specs/2026-07-29-global-schema-selection-owner-gate-b.md`.
- Ran `rustfmt` and `git diff --check`; both passed. Static scans found no
  production DML/DDL/PRAGMA writer and no legacy CLI-owned Diesel/audit/lock
  implementation. Cargo remains intentionally unrun.
- Traced the final receipt verifier through all forward and reverse
  reconciliation paths. The remaining blocker is precisely the Diesel-only
  row reader, not missing canonical/hash logic. Documented the reusable
  verifier pieces, the twelve-table/domain-authority read graph, the required
  repository-owned reader seam and the minimal parity/tamper TDD matrix.
- Rejected an owner-local second connection, raw-copy verifier and
  stored-hash-only shortcut because each would weaken the amendment's
  same-retained-transaction proof. The exact adapter is a cross-file
  repository refactor and was not smuggled into this static owner slice.
- Generalized the complete reconciliation algorithm behind the private
  `ExactSelectionSnapshotReader`; the existing Diesel adapter remains the
  repository path.
- Added `RusqliteExactSelectionSnapshotReader` and
  `verify_database_and_audit_in_rusqlite_snapshot`. The public-in-module entry
  accepts only `&rusqlite::Transaction` plus the locked audit session: no path,
  connection factory, new connection or stored-hash shortcut exists.
- Ported all twelve persistence reads plus the receipted outcome-authority
  joins to the rusqlite adapter. Both adapters share the canonical rebuild,
  domain-row rehash, staged-db rehash and forward/reverse audit reconciliation.
- The owner now calls the exact verifier inside its retained transaction.
  Exact final closure plus unchanged catalog/PRAGMA/integrity/audit/object
  evidence yields the opaque non-`Clone`
  `VerifiedAmendedSelectionSchema`; its exclusive maintenance lease and pinned
  database/audit objects remain live until capability drop.
- Added focused static TDD contracts for Diesel/rusqlite same-snapshot parity,
  rusqlite envelope copied-hash tamper rejection, missing receipt fail-closed,
  amended capability issuance and retained-lock lifetime. Existing repository
  tests continue to cover orphan Prepared evidence, typed generation/outcome
  column tampering and outcome lineage mismatch.
- Ran `rustfmt`, `git diff --check` and no-path/open signature scans after the
  exact integration. Cargo remains intentionally unrun under the root's
  shared-artifact restriction.
