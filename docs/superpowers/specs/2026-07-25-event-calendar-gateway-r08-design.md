# Event Calendar Gateway / R-08 — Slice 3 Design

**Status:** Gate A approved; implementation is split by independently complete component
**Parent design:** `2026-07-23-magic-market-data-unified-gateway-design.md`
**Data red lines:** 2.1, 2.2, 2.3, 2.4, 2.7, 2.8, 2.10
**Business rule:** BR-161

## 1. Outcome

R-08 must consume typed, independently evidenced event-calendar components
through one `EventCalendarGateway`. A provider failure is not an empty
component, local database update time is not provider time, and one component
must never fill another component's missing fields.

The report can be explicitly degraded when at least one independent component
is complete. It fails when every component is unavailable.

## 2. Components

```text
EventCalendarGateway::for_date(trading_date)
  ├─ MarketAnnouncements
  ├─ VerifiedBrokerPositions
  ├─ UserConfirmedVirtualObservations
  ├─ GlobalOvernightIndices
  └─ GlobalOvernightFx
```

Every provider-backed component returns
`Available | VerifiedEmpty | Unavailable | Stale | Partial | Conflict |
Unsupported` plus provider, source time when supplied, local observation time,
batch ID and immutable acquisition-audit receipt.

### 2.1 Market announcements

The current R-08 semantics use full-market discovery and then distinguish
monitored/holding relevance. The upstream release currently has typed
per-instrument announcements, not a full-market announcement-discovery
contract. Per-instrument queries may serve a separately labelled monitored
universe section, but must not be called or rendered as a full-market batch.

Required upstream addition:

```text
MarketAnnouncementRequest { trading_date, page_limit, record_limit }
  -> DataBatch<Announcement>
```

The provider must prove pagination termination, canonical announcement identity,
publication date and detail identity. A partial page/detail batch is `Partial`,
not `VerifiedEmpty`.

### 2.2 Verified broker positions

This is an account capability, not a public market-data capability. It requires
a real broker snapshot with provider, immutable batch identity, source time and
30-second freshness. The user-confirmed local snapshot may be shown as a local
valuation/observation fact under its own label, but cannot become a broker
position batch.

### 2.3 User-confirmed virtual observations

The existing physically isolated user-confirmed observation store remains the
source. Directory/database failure is `Unavailable`; a successfully read empty
snapshot is `VerifiedEmpty`. Test mode accepts only `TEST_CODE` identities.

### 2.4 Global overnight market

The released upstream now provides two separate Sina packet contracts:

```text
GlobalIndexRequest { exact index identities }
  -> DataBatch<GlobalIndexQuote>

FxRequest { exact currency-pair identities }
  -> DataBatch<FxQuote>
```

They remain separate downstream components. A complete USD/CNY batch may be
rendered when the US-index batch is unavailable, and vice versa.

The current released Sina global-index packet contains exact identity, positive
value, finite change facts, provider identity and batch evidence, but it does
not contain a provider timestamp. Under this R-08 contract it is rejected
until the upstream record can prove `source_at`; local `observed_at` is not a
substitute. The released Sina FX packet does contain a provider date/time and
may be admitted independently after exact identity, cardinality, freshness and
batch-evidence validation.

## 3. Filtering, ordering and display limits

BR-161 preserves the existing user-facing limits without moving them into
acquisition:

- validate the whole component before relevance filtering;
- stable-deduplicate announcements by canonical external identity;
- monitored/holding-related announcements first, then other market events;
- within a group order by provider publication time descending and identity
  ascending;
- display at most three related and three other announcements only after the
  complete accepted component is audited;
- display at most five position/observation events after their component is
  complete.

Lifecycle-excluded announcements from BR-138 remain auditable source rows but
cannot re-enter R-08 content.

## 4. Failure modes

- full-market discovery unsupported: market-announcement component is
  `Unsupported`; do not call per-instrument data “full market”;
- provider time missing/future or date mismatch: reject/isolate with a
  structured reason, never substitute observed time;
- broker source absent or older than 30 seconds: position component
  `Unavailable`/`Stale`;
- a global component lacking source timestamp or a positive finite value:
  reject the atomic component; do not replace provider time with local
  observation time;
- US-index failure does not erase an independently complete USD/CNY component,
  and FX failure does not erase an independently complete US-index component;
- database/acquisition-audit failure: the affected component fails closed;
- all components unavailable: R-08 is `Failed`, not `NoData`.
- after every provider-backed component has passed its own admission and audit,
  the unrelated global intraday `DataMode` does not reject the report again;
  R-08 delivery is governed by these component states under BR-161.

## 5. Old-module disposition

| Module | Decision |
|---|---|
| `data_provider::announcement` full-market direct HTTP | replace after upstream market-discovery parity, then delete |
| direct `data_provider::yahoo` calls | deleted; the typed Sina global-market Gateway is the only replacement |
| local `stock_position.updated_at` broker fallback | reject and delete from R-08 |
| user-confirmed virtual observation reader | retain behind the Gateway |
| R-08 renderer, PushKind and BR-140 task outcome | retain |
| old inline review path in `main.rs` | delete; governed dispatcher is the only owner |

## 6. Validation

- source guards prove active R-08 has no direct announcement/Yahoo call;
- valid empty, unsupported, stale and partial states remain distinct;
- component failure does not erase another complete component;
- all components unavailable fails;
- provider and batch evidence are present in acquisition audit;
- display limits run only after complete component admission;
- `cargo run --bin monitor -- --review` names the Gateway component states and
  emits no legacy acquisition log.

## 7. Rollback

Before legacy deletion, revert the R-08 slice without changing immutable audit
or user-confirmed observation records. After deletion, deploy the previous
release commit; do not recreate or mutate old evidence as an automatic
fallback.
