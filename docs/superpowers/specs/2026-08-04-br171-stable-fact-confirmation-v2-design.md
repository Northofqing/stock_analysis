# BR-171 Stable-Fact Confirmation V2 Design

Status: Gate A registered

Rules: AGENTS 2.3, 2.7, 2.10; BR-092, BR-125, BR-147, BR-156, BR-159, BR-171

## 1. Problem and scope

Magic TDX batch identities include acquisition time. Reviewing a pending daily
change and then invoking the explicit confirmation command performs a second
audited acquisition, so byte-identical market and lifecycle facts receive new
daily/lifecycle batch IDs. The BR-171 v1 operator token and v1 database lookup
bind those volatile batch IDs and therefore can never complete that two-step
workflow.

This change does not relax any daily-change admission rule. It changes only the
identity used to recognise the same already-reviewed objective fact across
independent, fully audited acquisitions.

## 2. V2 identity and data flow

The v2 stable fact identity contains:

- canonical security code;
- previous/current trading dates, canonical closes and calculated percentage;
- daily provider and source;
- lifecycle provider;
- optional listing date and corporate-action identity.

Acquisition-time daily and lifecycle batch IDs are deliberately excluded from
the stable fact identity. They remain mandatory, are displayed by the operator
CLI, and the batch IDs present when the confirmation is appended are retained
in the immutable v2 confirmation row. Every acquisition, including later
admission attempts, continues through the existing BR-159 acquisition audit;
v2 does not synthesize, merge, overwrite or delete provider batches.

The CLI hashes the stable fact with domain
`BR171_OPERATOR_REVIEW_FACT_V2`. Confirmation re-acquires data, matches the
dates and v2 fact token, prints the current complete evidence, and only then
appends the decision. A changed date, price, percentage, provider/source,
lifecycle provider, listing date or corporate-action identity produces a
different token and fails closed.

Admission validates both ledgers and succeeds only when either the immutable
v1 exact query exists or the immutable v2 stable fact exists. V1 rows, hashes,
chain domains, schema and exact lookup semantics remain unchanged.

After admission, persistence must keep that authority attached to the records.
`stock_daily` backfill therefore passes the unforgeable `AdmittedDailyBars`
capability directly to a dedicated repository entry point. The repository may
persist its immutable records and provider source, but it must not discard the
capability and rerun the evidence-free legacy validator. The legacy raw-slice
`save_kline_data` API remains fail-closed and continues to reject an
unconfirmed large move; it is not an admission shortcut.

## 3. Persistence and failure modes

V2 uses additive `daily_change_confirmation_v2` and
`daily_change_confirmation_chain_v2` tables. Each v2 row is an immutable alias
from the stable fact identity to the exact v1 decision row that retains the
operator, reason, confirmation time and reviewed daily/lifecycle batch IDs.
The alias also repeats those reviewed batch IDs in its own hashed content so a
reference mismatch cannot silently change their meaning. Rows and chain links
are append-only, guarded against UPDATE/DELETE, and retained for at least five
years. Append uses one immediate transaction, validates the full v1 and v2
chains, appends the exact v1 decision, then appends its v2 alias atomically. A
stable fact is idempotent only for the same canonical operator and reason; a
later acquisition/CLI timestamp does not create a second decision, while a
different operator or reason is an explicit conflict. A hash collision, broken
reference or any ledger mismatch fails closed.

Missing/blank batch evidence, malformed decimals, a non-large move, an audit
failure, or any changed stable fact still returns an explicit error. V2 never
auto-confirms a move and never uses lifecycle context as an implicit approval.
An empty admitted batch, blank target/source/batch identity, or a target/record
identity mismatch blocks persistence before the SQLite transaction. Database
failure rolls back the complete batch.

## 4. Old-module disposition and rollback

| Module | Disposition | Reason |
| --- | --- | --- |
| BR-171 v1 confirmation tables and chain | retain unchanged | immutable five-year audit and exact historical decisions |
| v1 token/query hash domains | retain for v1 validation | changing them would invalidate historical evidence |
| provider acquisition audit | adopt unchanged | owns every concrete Magic TDX batch observation |
| legacy `save_kline_data` raw-slice path | retain fail-closed | callers without an admitted capability cannot claim BR-171 approval |
| backfill persistence | replace with admitted-batch entry | preserves Gateway admission authority through the write boundary |

Rollback reverts only the v2 CLI/lookup path. V2 tables and any appended audit
rows remain readable and immutable; rollback must not delete v1 or v2 evidence.

## 5. Verification

- Same objective facts with different daily/lifecycle batch IDs produce the
  same v2 token and reuse one v2 confirmation.
- Changes to date, price, percentage, daily provider/source, lifecycle
  provider, listing date or corporate-action identity do not reuse approval.
- V1 rows and v1 chain continue to validate unchanged.
- V2 fact/chain immutability and tamper detection are tested.
- `cargo test --bin confirm_daily_change`
- `cargo test --lib database::daily_change_confirmation::tests -- --test-threads=1`
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `bash tools/compliance/check.sh`
