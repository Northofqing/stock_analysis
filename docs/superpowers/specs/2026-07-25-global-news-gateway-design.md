# Unified Global Financial News Gateway

**Gate:** A
**Rule:** BR-166
**Data red lines:** 2.1, 2.2, 2.3, 2.4, 2.7, 2.8, 2.10

## Outcome

Production global financial-news acquisition is reduced to the four released
and typed upstream providers:

| Feed | Upstream client | Expected source |
| --- | --- | --- |
| Eastmoney finance roll | `EastmoneyClient` | `eastmoney-web` |
| CLS telegraph | `ClsClient` | `cls-v1` |
| Jin10 flash | `Jin10Client` | `jin10-flash-v1` |
| The Paper finance | `ThePaperClient` | `thepaper-finance-v1` |

Every client is constructed, called and dropped in a blocking Gateway worker.
No consumer imports a provider crate or retains a provider URL/parser.

## Data flow

```text
NewsAggregator registered feed
  -> GlobalNewsGateway::fetch(provider, limit)
  -> released magic-* client / NewsProvider::global_news
  -> complete DataBatch<NewsItem>
  -> provider-specific admission
  -> BR-159 immutable acquisition audit
  -> GatewayBatch<GlobalNewsRecord>
  -> MarketEvent conversion
  -> BR-137 freshness gate + BR-155 event inbox
```

The four feeds are independent. A provider error remains one failed
`FeedAttempt`; the other complete providers continue. Cross-provider field
filling and fallback are forbidden.

The source proves that an item was published, but does not prove its market
direction or impact. The initial `MarketEvent` projection therefore uses
`Neutral`, impact strength `0`, and publication certainty `100`; later
classification may change impact fields only from separately audited evidence.

## Admission

- common limit is `1..=20`;
- provenance must be complete and match the provider/source contract;
- batch ID, observation time and provider source time must be present;
- records must have matching provider/batch evidence;
- identity and canonical URL must be unique;
- title, publisher, publication time and official HTTPS URL are mandatory;
- publication time must parse, must not be after observation time, and records
  must remain in provider newest-first order;
- missing optional summary/content/instrument/topic data stays absent.

Verified empty is preserved only when the complete upstream batch itself proves
it and does not claim a false source time. Protocol, evidence, order and
identity contradictions reject the whole batch.

## Old modules

The following direct financial-news sources are deleted after callers migrate:

- local Jin10, CLS and Eastmoney provider implementations;
- WallStreetCN, Weibo, Gelonghui, KcbDaily and SinaFlash production feeds;
- financial provider registrations in `SearchService`;
- provider host/signature/retry configuration made unreachable by the cutover.

Generic user-authorized web search may remain for non-authoritative research,
but its results cannot enter the governed financial-news event path as source
facts.

## Validation

- deterministic admission/error tests for every source;
- aggregator tests prove independent failed/successful attempts;
- architecture test proves no financial host/provider import outside Gateway;
- real provider probes retain identity, source time, observation time and batch
  ID;
- fmt, strict Clippy, full tests, compliance and coverage gates.

## Rollback

Revert the Gateway, feed-registry and deletion commit together. Rollback must
not restore a direct provider fallback or delete persisted news/acquisition
audit records.
