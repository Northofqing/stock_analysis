# Economic release Gateway design

Business rule: BR-167.

## Scope and intent

This slice moves the remaining Jin10 macroeconomic-release acquisition out of
`SearchService` and behind `src/data_gateway`. The released upstream contract
provides the latest public economic releases. It does not prove a future
calendar window, so the downstream report must stop labelling the data as
“future 48h events”.

## Data flow

1. A consumer requests `1..=20` latest releases and an optional exact country.
2. `EconomicCalendarGateway` creates and uses `Jin10Client` inside one
   `spawn_blocking` worker.
3. The upstream typed request and provider perform network and schema
   validation.
4. The Gateway admits only one complete `ProviderId::Jin10` /
   `jin10-flash-v1` batch, validates batch and record evidence, time ordering,
   identity uniqueness and importance bounds, and writes the BR-159
   acquisition audit.
5. `SearchService::search_macro_news` renders admitted releases as latest
   releases. Its `importance >= 2` and display limit 15 are applied only after
   complete-batch admission.

## Failure modes

- Invalid limit/country: typed non-retryable invalid request.
- Transport failure: retryable unavailable.
- Decode, protocol, incomplete quality, missing or conflicting evidence,
  future release time, duplicate identity, invalid importance or source-order
  drift: explicit non-retryable invalid evidence.
- Blocking worker or acquisition-audit failure: explicit unavailable; no
  consumer-visible batch.
- A complete admitted batch whose consumer importance filter matches nothing:
  verified no matching releases, not source failure.

Missing optional values remain absent. No current time, zero, empty string,
cross-source field, legacy provider or fixture may fill them.

## Old-module disposition

| Module | Decision | Reason |
| --- | --- | --- |
| `search_service/providers/jin10.rs` calendar path | reject and delete after all callers migrate | Duplicates the released typed provider and collapses failures to empty output. |
| `Jin10CalendarEvent` | reject | Loses immutable provider/batch evidence and implies an unsupported future-window contract. |
| `GlobalNewsGateway` | adopt | Macro news must reuse already-released typed financial-news batches. |
| generic paid web-search providers | retain for user-authorized search only | They are not authoritative economic-release evidence and are not a fallback. |

## Validation

- Deterministic Gateway request and admission tests.
- Rejection tests for invalid evidence, future/order/duplicate/importance
  failures, and exact missing-value preservation.
- SearchService macro rendering test using admitted records without network.
- Targeted format, strict Clippy and tests, then repository-wide gates.

## Rollback

Revert this slice. Do not restore legacy Jin10 production fallback after the
final cutover; if the upstream provider is unavailable, the safe state is an
explicit unavailable macro-release component.
