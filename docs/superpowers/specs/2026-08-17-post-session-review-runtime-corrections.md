# Post-session review runtime corrections

Status: Gate A recorded; Gate B/C/D pending.

Rules: BR-239 (bounded/cancellation-safe review runtime), BR-240
(two-batch ProviderTopN evidence preservation), and BR-242 (exact semantic
search provider identity and cancellation), with the BR-238 aggregate evidence
amendment.

## Evidence

The 2026-08-17 production cutover proved five real Feishu deliveries, but the
first review attempt timed out after 300 seconds. The production database and
logs establish four separate defects:

1. R-12 called an unpublished `TechnicalBars` route once per 573 trade rows and
   then again for 357 unique codes. `spawn_blocking` survived cancellation and
   the task completed after the outer review timeout.
2. Counted tasks reconciled the same global immutable-audit outbox concurrently.
   One CAS loser left R-07 Reserved before a sink attempt; later reacquisition
   rendered different dynamic evidence and correctly produced identity
   conflicts.
3. `ProviderTopNRankings` merged two real batches and retained only the first
   batch evidence, making R-09's required distinct identities impossible.
4. ExternalV1 reference/news converters required every record time to equal a
   batch aggregate time. The upstream contract instead uses an aggregate
   oldest/newest time while preserving per-record evidence.

## Required behavior

### ExternalV1 aggregate evidence

Apply BR-238's operation-specific aggregate invariants. Provider and batch
identity stay exact; times are parsed as instants and checked against the
documented aggregate rule. No field is filled or rewritten. Any source after
observation, aggregate mismatch, provider/batch conflict, unsupported encoding,
or request-set mismatch fails before consumption.

### Counted delivery serialization and R-07

One process-wide blocking mutex owns the entire counted-delivery durable
critical section. It must cover prepare, pending-audit reconciliation,
resume/begin-attempt, sink result persistence, and terminal reconciliation.
Network/source acquisition stays outside the lock. Every counted review task
must query the exact task occurrence before acquisition; an existing Reserved,
AttemptInFlight, terminal, conflicted, or hydrated occurrence is handled only
through durable state, never by rendering a replacement envelope.

The existing 2026-08-17 Reserved R-07 row and conflict audits are immutable.
Recovery may append reconciliation evidence and resume an eligible stored
envelope, but must not update/delete the original row, manufacture a new batch,
or resend an accepted result.

### Bounded tasks

R-12 is typed Disabled while production `TechnicalBars` is unpublished and
must call loaders zero times. When that capability is later admitted, each
unique code is loaded at most once per run; success and failure are both cached.
Dropping the outer review future must not permit an overlapping scheduler run
while an uncancelled blocking worker remains.

Under BR-242, every SemanticSearch LocalBridge request contains exactly one of
`Bocha`, `Tavily`, or `SerpApi`, the non-empty query, and a limit in `1..=50`.
The server calls only that provider; the client validates the response provider,
source, record evidence, and record count against the request. A bridge must not
register three names that all call the same provider or silently fall back to a
different provider. Each R-11 stock's 90-second budget runs inside the blocking
worker's current-thread runtime around the real analysis future so expiration
drops that future and ends the worker. Missing ResearchOnly context remains
explicit and does not suppress an otherwise complete R-11 position report.

### ProviderTopN

The LocalBridge response carries each metric's real evidence. The client first
validates metric/evidence consistency and then builds two batches. Same IDs,
missing evidence, mixed providers, record/envelope conflicts, and source-date
violations are non-retryable invalid evidence. Deriving or suffixing a batch ID
is forbidden.

### Known non-code blockers

- R-03 remains unavailable without a real account capture no older than 30
  seconds. A historical screenshot may be stored but cannot authorize it.
- R-08 remains unavailable while official CFFEX evidence is unsupported under
  BR-199. It must not be represented as verified empty or delivered.

## Validation

- Focused RED/GREEN tests for aggregate evidence, counted CAS serialization,
  R-07 preflight/recovery, R-12 zero-call/negative cache, and two-batch R-09.
- `cargo fmt --all -- --check`
- relevant `cargo test --lib` and `cargo test --bin monitor` filters
- `bash tools/compliance/lib/check_business_rules.sh`
- `bash tools/compliance/check.sh`
- release rebuild, single-process cutover, static probe, durable receipt and
  immutable-audit verification.

## Rollback

Revert only the correction commits and restore the retained release binaries.
Do not delete or rewrite account snapshots, durable decisions, conflict audits,
sink receipts, or immutable audit records. A rollback keeps the new runtime
disabled rather than re-enabling overlapping retries.
