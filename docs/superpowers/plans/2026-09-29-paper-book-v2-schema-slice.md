# Paper book V2 staged schema slice

## Outcome

Establish an isolated-test-only, read-verifiable fee-policy manifest for the next paper-book generation under ADR-0001. V1 DDL, fee arithmetic, event bytes, and replay remain frozen. This slice creates no account, event, fill, owner, or production seed capability.

## Sequence

1. Add a separately named V2 fee-policy manifest table and immutable triggers. Its staged installer runs in one SQLite transaction only in tests; the read path compares the exact namespace with same-runtime SQLite DDL and verifies a reviewed `AShareFeePolicyV2` descriptor and hash.
2. In an isolated DB, seed a V1 fixture, install the V2 staged schema, and verify V1 effective reads still match. Reject unknown database identity, extra or altered V2 objects, and changed policy contents. Prove a failed installation rolls back all DDL and data.
3. Run the targeted library tests and `git diff --check`. Commit only when those checks pass.

## Next gates

The global exact catalog currently ends at generation 3. A later slice must register a new global generation, add an account/strategy owner fence and authorized cutover, and only then expose V2 account seed, order, fill, and effective-projection writes. Production migration, monitor wiring, and account cutover are outside this slice.
