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
- a governance identity used by existing dedup;
- an optional real security code, distinct from a flash event identity;
- non-empty headline and source provenance;
- observed timestamp and explicit upstream stale state.

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
  -> validate source identity, kind, code rules, bounds, and stale flag
  -> SourceFactEvidence
  -> push_source_fact_v3
  -> independent capability-health context (does not read account/banner state)
  -> source-fact v14 gate (DataMode minimum Down)
  -> unchanged LaunchGate / quiet hours / dedup / daily count
  -> unchanged sink / L7 / delivery event / immutable audit
```

The prepared SignalEvent uses `SignalPayload::NewsCatalyst` with headline, source, and explicit
optional security code. It never uses an empty `HoldingHealth` payload.

## 4. Validation and failure modes

- Empty/mismatched kind, identity, headline, or source is an explicit denial.
- Announcement, earnings, and analyst facts require a non-empty code accepted by the environment
  guard; Policy may be global. Critical flash requires an event identity but does not put it in the
  security-code field.
- Strength/certainty outside `0..=100`, an upstream `stale=true`, or a future observed timestamp is
  rejected. No value is clamped or filled at the gate.
- The source-fact governance and L7 paths read the real process-local capability tracker directly;
  a missing account/banner snapshot cannot block them. Capability-tracker, analytics, lock, sink,
  dedup, or immutable-audit failure keeps its existing explicit failure result. A sink result is
  never inferred from logging.
- Source-fact approval at DataMode Down is not an `always_send_on_data_source_down` bypass; normal
  quiet-hour and launch policy still apply.

## 5. Interface placement

`v14_adapter` owns evidence validation and prepared-event governance, hiding profile construction
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
| blanket PushKind profile downgrade | reject | Would allow mixed-data advice when dependencies are down. |
| banner defaults/placeholders | reject | Would fabricate account or data-health facts. |

## 7. Acceptance

Focused tests prove a complete source fact is approved when current DataMode is Down, its payload
retains provenance, generic news/advice remains denied, invalid facts are rejected, MarketAction is
not whitelisted, and critical flash keeps event identity separate from security code. Existing
dedup, delivery, audit, source batching, and strict-review tests must remain green.

Production acceptance uses only aggregate L7/event/audit counts and governance reasons. It must not
print message bodies, security identities, account values, notification targets, or receipts.

## 8. Rollback

Revert the merge commit, rebuild release, stop only the verified monitor PID, and restart one master
process. Do not rewrite the source database, account/holding tables, push analytics, event bus, or
immutable audit chain.
