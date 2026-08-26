# Position Chain Assignment from Magic TDX

**Status:** Gate A refined for atomic pre-trade ownership
**Parent design:** `2026-07-25-unified-data-final-cutover-design.md`
**Data red lines:** 2.1, 2.2, 2.4, 2.5, 2.6, 2.7, 2.8, 2.10
**Business rule:** BR-170

## 1. Outcome

Production position and concentration logic must not derive an industry chain
from `src/data_provider/chain_registry.rs` or any other hand-maintained code
table. It must consume a complete `BoardDataGateway::memberships` batch from
Magic TDX, persist the selected assignment with its source evidence, and keep
the position chain empty when that evidence is unavailable.

The change covers existing open positions, the explicit
`--backfill-chain-name` command and new pre-trade candidates. It does not infer
industry from a company name, news text, price action or a stale local cache.

## 2. Current failure

Three production paths still call the static registry:

```text
portfolio::store::get_positions
pipeline::position_tracker::query_chain_exposure
DatabaseManager::backfill_chain_name
```

Those lookups have no provider, source time, observed time, batch identity or
acquisition audit. Values already written to `stock_position.chain_name`
remain indistinguishable from a real provider assignment after the source code
is deleted. This violates Data Redlines 2.1 and 2.7.

## 3. Considered approaches

1. **Delete the fallback and leave every miss as `NULL`.** This removes false
   data but does not complete the requested position enrichment.
2. **Call Magic TDX on every position read.** This preserves source truth but
   introduces blocking network work inside account/risk reads and makes one
   provider outage stall unrelated consumers.
3. **Selected: acquire asynchronously, persist atomically, read locally.**
   A dedicated Gateway orchestrator fetches complete membership batches and
   chooses a deterministic primary assignment. Existing-position refresh
   commits the evidence and current position projection together. A new
   candidate carries the complete validated assignment into the execution
   boundary, where the order audit, position insert, assignment append and
   projection link commit in one transaction. Read paths never own transport.

Multi-position refresh deduplicates and sorts exact codes before acquisition,
uses at most four concurrent Magic TDX membership calls, then restores stable
code order before applying per-code outcomes. One failed code never suppresses
a successful assignment for another code.

## 4. Data flow

```text
open-position code set
  -> PositionChainGateway
  -> BoardDataGateway::memberships(code)
  -> Magic TDX BoardMembershipProvider
  -> complete GatewayBatch + BR-159 acquisition audit
  -> validate exact instrument and canonical membership facts
  -> deterministic primary-board selection
  -> one SQLite IMMEDIATE transaction
       append position_chain_assignment
       update stock_position.chain_name + chain_assignment_id
  -> portfolio/risk readers consume only the linked projection

new candidate code
  -> async analysis owner
  -> PositionChainGateway candidate acquisition
  -> complete Magic TDX GatewayBatch + BR-159 acquisition audit
  -> deterministic PositionChainAssignment (not persisted yet)
  -> synchronous concentration query by assignment.primary.board_name
  -> OpenPositionCmd carries the complete assignment, never a raw chain string
  -> one SQLite IMMEDIATE transaction
       append order_audit + order_audit_chain
       insert stock_position with chain_name initially NULL
       append/idempotently verify position_chain_assignment
       link stock_position.chain_name + chain_assignment_id
  -> success receipt only after the whole transaction commits
```

Normal monitor startup refreshes open positions before long-running consumers
load them. `--backfill-chain-name` runs the same orchestrator and exits after a
bounded complete pass. Before the synchronous tracker handles a code without an
open position, its async owner acquires exactly one candidate assignment.
An unavailable/verified-empty assignment rejects that candidate before sizing;
an existing-position close/update path does not depend on a fresh candidate
assignment. A race that turns an apparent existing position into a new
candidate also fails closed because no assignment was supplied.

## 5. Deterministic assignment rule

All valid memberships from the complete provider batch are retained in the
immutable assignment evidence. The single legacy `chain_name` projection is
selected only after full validation:

1. accept `Industry` and `Concept`; exclude `Region` and unsupported kinds;
2. prefer `Industry` over `Concept`;
3. within the same kind, sort by canonical `board_code`, then `board_name`;
4. select the first row as the primary projection.

Empty complete membership is `VerifiedEmpty` and leaves the chain `NULL`.
Duplicate `(code, board_code)` with identical facts is idempotent; conflicting
facts fail the complete assignment. No display limit or fuzzy name matching is
allowed.

## 6. Persistence and audit

`position_chain_assignment` is append-only and stores:

- deterministic assignment ID and content hash;
- six-digit instrument code;
- primary board code/name/kind;
- the complete canonical membership JSON used for the choice;
- provider, source, optional source time, observed time and source batch ID;
- creation time.

`UPDATE` and `DELETE` triggers reject mutation. Repeating the same assignment
identity and content is idempotent; the same identity with different content is
a conflict. `stock_position.chain_assignment_id` links the current
`chain_name` projection to the immutable row.

During schema initialization, any non-empty `stock_position.chain_name` without
a valid linked assignment is normalized to `NULL`. This deliberately retires
values previously written by the static registry. A provider failure does not
erase a still-valid linked assignment, but an unlinked value can never remain
production evidence.

The Board Gateway continues to own BR-159 acquisition auditing. Existing
position refresh owns the assignment/projection transaction. New-position
execution owns a stronger transaction: order audit plus tamper-resistant audit
hash, position insert, assignment append/idempotency check and projection link.
The position row is inserted with `chain_name = NULL`; only the validated
assignment may populate it. A database, assignment or audit failure rolls back
all four effects and no filled receipt is returned.

`DatabaseManager::save_position` remains available for non-order account
snapshot/test fixtures but must reject any caller-supplied non-null
`chain_name`. This prevents a second unaudited write path from recreating the
retired static registry semantics.

## 7. Failure modes

- invalid/non-production code: reject before provider construction;
- provider transport/protocol/partial batch: explicit `GatewayError`;
- batch identity or returned instrument mismatch: reject;
- empty complete batch: verified empty, position remains `NULL`;
- no Industry/Concept membership: verified empty for chain purposes;
- duplicate conflict or invalid text: reject complete assignment;
- assignment append/projection transaction failure: roll back both;
- candidate code/assignment mismatch: reject before any position or audit
  success row is committed;
- new-position transaction failure: roll back order audit/hash, position,
  assignment and link together; the rejected attempt remains separately
  auditable through the existing failure path;
- startup refresh partial by code: retain successful codes, report each failed
  code and a non-success aggregate; no failed code receives a value;
- pre-trade assignment failure: reject that candidate before sizing/order.

## 8. Old module disposition

| Module | Disposition | Reason |
|---|---|---|
| `data_provider/chain_registry.rs` | delete | hand-maintained production data source |
| `DatabaseManager::backfill_chain_name` | replace | database layer must not own provider selection |
| `portfolio::store` registry fallback | delete | reads consume only linked persisted evidence |
| `position_tracker` registry/cache fallback | replace | concentration requires verified assignment |

No compatibility flag or static fallback remains.

## 9. Verification

- unit tests for primary selection, verified empty, duplicate conflict and
  invalid identity;
- SQLite tests for append-only enforcement, idempotency, conflict rollback,
  linked projection and unlinked legacy cleanup;
- execution tests proving order audit/hash, new position, assignment and link
  either all commit or all roll back;
- repository tests proving raw `NewStockPosition.chain_name` is rejected;
- async-owner tests proving only a new candidate requires acquisition and
  provider failure rejects only that candidate;
- process test for `--backfill-chain-name` isolation and explicit failures;
- source guard proving `chain_registry` and its call sites are deleted;
- bounded real Magic TDX probe for the current position codes;
- `cargo fmt --all -- --check`;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`;
- `cargo test --workspace --all-targets --all-features -- --test-threads=1`;
- `bash tools/compliance/check.sh`.

## 10. Rollback

Rollback disables the refresh orchestration and stops writing new assignments.
The append-only evidence table is retained for audit. The projection may be
cleared to `NULL`; rollback must not restore the static registry or copy
unverified labels back into `stock_position`.
