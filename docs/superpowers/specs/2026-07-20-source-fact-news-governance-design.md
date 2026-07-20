# Source-fact news governance design

## 1. Problem

The live monitor classifies real earnings and other source events, but the generic v14 adapter gives
nearly every PushKind a minimum DataMode of `Degraded` and represents every non-DataMode event as an
empty `HoldingHealth` payload. When quote/kline/money-flow capabilities make the global DataMode
`Down`, structurally complete news facts are denied as `data_quality` even though they do not consume
those capabilities. Production evidence showed repeated classified `EarningsMiss` decisions and no
delivery.

Account/banner liveness is a separate historical fault and is currently healthy. This design does
not relax strict review or any trading path.

## 2. Dependency boundary

Introduce one validated `SourceFactEvidence` value owned by the v14 adapter. It binds:

- the exact allowed PushKind;
- a governance identity used by existing dedup, separate from any security code;
- an optional real security code, distinct from a flash event identity;
- non-empty headline and source provenance;
- the real adapter-observation timestamp, provider publication date, and explicit upstream stale
  state. A provider publication date earlier than the current local date is stale; a missing date
  cannot qualify for the source-fact path.

Provider time semantics are part of the boundary, not a downstream guess. Flash providers that
receive epoch/full timestamps must retain a full local timestamp in `SearchResult`; truncating to
`HH:MM` is rejected. Financial facts must preserve the provider `NOTICE_DATE` independently from
the accounting `REPORT_DATE`; an accounting period is never publication evidence.

Only these source-self-contained kinds are allowed:

- `Announcement`
- `PolicyHit`
- `EarningsBeat`
- `EarningsMiss`
- `AnalystUpgrade`
- `NewsFlashCritical`

`MarketActionAlert` depends on trading state and remains generic. `NewsCatalyst`, `NewsToIdea`,
`NewsRanked`, and review/holding/trading kinds mix other data and remain generic. Aggregated flash is
not included because its current buffer drops per-item provenance; it stays fail closed until a
separate design preserves all contributing sources.

## 3. Data flow

```text
real provider/classifier
  -> NormalizedSourceEvent or critical MarketEvent
  -> validate source identity, kind, code rules, bounds, observation time, provider date, and stale flag
  -> SourceFactEvidence
  -> push_source_fact_v3
  -> independent capability-health context (does not read account/banner state)
  -> source-fact v14 gate (DataMode minimum Down)
  -> unchanged LaunchGate / quiet hours / dedup / daily count
  -> unchanged sink / L7 / delivery event / immutable audit
```

The production producer boundary is explicit. The existing real announcement poll classifies each
complete provider announcement and gives every successfully classified item exclusively to the
normalized source-fact path; its exact outcome remains visible and no non-Pushed outcome may fall
through to the legacy sender. Only a classification failure remains eligible for the existing legacy
path. Government policy is polled directly from `GovPolicyProvider`, classified from the original
`SearchResult`, and then delivered by the same normalized path. It is removed from the generic
NewsAggregator so a pre-delivery `seen_simhash` cannot consume the only retry.

The prepared SignalEvent uses `SignalPayload::NewsCatalyst` with headline, source, and explicit
optional security code. The provider identity is hashed into `SignalEvent.event_id` and is the L4
dedup identity; it is never written to `SignalEvent.code` or delivery-audit `code/entity_key`.
It never uses an empty `HoldingHealth` payload.

## 4. Validation and failure modes

- Empty/mismatched kind, identity, headline, or source is an explicit denial.
- Announcement, earnings, and analyst facts require a non-empty code accepted by the environment
  guard; Policy may be global. Critical flash requires an event identity but does not put it in the
  security-code field.
- Strength/certainty outside `0..=100`, an upstream `stale=true`, a future observation/publication
  time, a missing provider publication date, or a provider publication date before the current
  local date is rejected. `observed_at` records the real adapter observation; it must not replace
  the provider date. No value is clamped or filled at the gate.
- Flash feeds derive freshness from their real provider timestamp/date. Missing publication time is
  marked stale and excluded from both critical and aggregate flash decisions. The generic feed
  adapter preserves `SearchResult.source`; it must not replace real provenance with a category
  label. Before either flash path accepts an event, identity, headline, provider provenance, score
  bounds, and time must all validate.
- Date-only parsing matches the entire provider string. Prefix parsing such as accepting
  `2026-07-21garbage` is forbidden. The flash gate independently compares the event publication date
  to the current local date even when an upstream caller supplied `stale=false`.
- Announcement production uses the real Eastmoney external identity and validated publication date.
  Policy production uses the real government provider source/date and never first converts the item
  into a generic flash. Fetch, classification, governance, and sink failures remain distinct.
- `SourceFactEvidence` revalidates the provider publication date in addition to observation time and
  stale state; a caller cannot obtain the relaxed profile by supplying only `now` and `false`.
- Earnings uses the real financial provider and `NOTICE_DATE`. Analyst upgrades use the real report
  provider and its `publishDate`. Synthetic labels such as `earnings_classifier` and
  `analyst_tracker`, and use of `FinancialPeriod.report_date` as freshness, are rejected.
- The source-fact governance and L7 paths read the real process-local capability tracker directly;
  a missing account/banner snapshot cannot block them. Capability-tracker, analytics, lock, sink,
  dedup, or immutable-audit failure keeps its existing explicit failure result. A sink result is
  never inferred from logging.
- Source-fact approval at DataMode Down is not an `always_send_on_data_source_down` bypass; normal
  quiet-hour and launch policy still apply.

## 5. Interface placement

`v14_adapter` owns evidence validation, source-identity hashing, and prepared-event governance, hiding profile construction
from callers. It also owns the independent capability-health context used before and after source
fact delivery; account state is deliberately absent because it is not a dependency of source facts.
`notify` owns the sole source-fact delivery entry and reuses the same common tail as the generic
governor. Source adapters can request the narrow capability but cannot edit profile thresholds or
construct an approved SignalEvent themselves.

## 6. Old modules

| Module | Decision | Reason |
| --- | --- | --- |
| `NormalizedSourceEvent` | adopt and revalidate | Existing real-source contract has public fields. |
| `push_governor_v3` | adopt unchanged for generic paths | Prevents a global news/data-mode exemption. |
| `deliver_and_record` | adopt | Preserves real sink and all authoritative audit layers. |
| `NewsFlashGate` | adopt for critical only | It retains the original MarketEvent before rendering. |
| announcement poll | adopt with normalized ownership | Successfully classified items use one governed source-fact path; legacy remains only for classification failures. |
| `GovPolicyProvider` | adopt as direct producer | Preserve original `SearchResult` and retry through delivery governance instead of aggregator pre-dedup. |
| generic `GovPolicyFeed` registration | reject | It erases the production `PolicyHit` route and commits seen state before delivery. |
| blanket PushKind profile downgrade | reject | Would allow mixed-data advice when dependencies are down. |
| banner defaults/placeholders | reject | Would fabricate account or data-health facts. |

## 7. Acceptance

Focused tests prove a complete source fact is approved when current DataMode is Down, its payload
retains provenance, generic news/advice remains denied, invalid/stale/missing-date facts are rejected,
MarketAction is not whitelisted, two provider events for one security retain independent L4 identities,
critical flash keeps event identity separate from security code and delivery audit, each registered
flash provider retains a full timestamp, financial `NOTICE_DATE` is distinct from `REPORT_DATE`, and
malformed flash events cannot enter either buffer. Producer tests must prove one real announcement
batch and one real policy result reach their respective normalized PushKind, while malformed
timestamps and old events with `stale=false` are
rejected before buffering. Existing dedup, delivery, audit, source batching, and strict-review tests
must remain green.

Production acceptance uses only aggregate L7/event/audit counts and governance reasons. It must not
print message bodies, security identities, account values, notification targets, or receipts.

## 8. Rollback

Revert the merge commit, rebuild release, stop only the verified monitor PID, and restart one master
process. Do not rewrite the source database, account/holding tables, push analytics, event bus, or
immutable audit chain.
