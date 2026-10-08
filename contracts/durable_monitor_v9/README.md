# Retained monitor Schema9

`schema.sql` is the original Schema9 creation DDL from released source
`f517f45c91ec9489f63d0156a1b8f9cf45400c3b:src/durable_delivery/schema.rs`.
It is an immutable compatibility contract, not the development baseline.
The receiver is developed from current master.

The latest monitor uses this DDL only in memory to verify an existing catalog.
It does not execute it against the production file, seed policy rows, create
platform extensions, migrate a header, or heal altered/missing objects.
Ordinary counted delivery retains its attestation, transactions, immutable
references, budget, cooldown and idempotency checks. Schema14 remains the
default boundary for frozen platform APIs and isolated full-platform tests.
