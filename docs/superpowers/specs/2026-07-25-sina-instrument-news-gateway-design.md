# Sina Instrument-News Gateway Design

**Status:** Gate A approved for the bounded downstream Sina instrument-news migration.

## Scope and evidence

The active legacy production caller is reproducible with:

```bash
rg -n -U 'post_close_news_review[\s\S]{0,2200}(SinaNewsProvider|fetch_stock_news_in_range)' \
  src/bin/monitor/main.rs
```

Observed output before implementation includes:

```text
8900: use stock_analysis::data_provider::sina_news_provider::SinaNewsProvider;
8908: let provider = SinaNewsProvider::new();
8935: match provider.fetch_stock_news_in_range(code, from, now).await {
```

The scheduling chain is:

```text
long-running monitor
  -> post_close_news_scheduler (one spawned owner)
  -> post_close_news_review (every 30 minutes after 15:30)
  -> legacy SinaNewsProvider::fetch_stock_news_in_range
  -> news_items
```

There is no second production `fetch_stock_news` or
`fetch_stock_news_in_range` caller. General Sina financial/top news is a
separate capability and remains unchanged.

## Architecture

Create `src/data_gateway/sina_instrument_news.rs` as a sibling of
`src/data_gateway/review.rs`. It reuses the existing `GatewayBatch`,
`BatchEvidence`, `GatewayError`, request hashing, immutable BR-159 acquisition
audit, and blocking-join failure helpers. It must not modify the concurrent
review Gateway or database audit implementation.

`SinaInstrumentNewsGateway::instrument_news_in_range` accepts a six-digit
A-share code plus exact UTC start/end instants. The blocking upstream
`SinaClient` is created, called and dropped wholly inside `spawn_blocking`.
The worker:

1. Builds a typed Core equity `InstrumentId` and inclusive date-range request
   with a fixed 100-row limit.
2. Calls the real upstream `SinaClient: NewsProvider` exactly once.
3. Preserves a provider-proven empty batch as `VerifiedEmpty`.
4. Routes a cloned non-empty real batch through a one-source
   `InstrumentNewsRouter` with complete-quality and source-time acceptance.
5. Validates provider, instrument, batch ID, published time and observation
   evidence, then converts records to the existing persistence `NewsItem`.
6. Applies the exact UTC `[from,to]` filter without sorting. If it removes all
   records, returns `VerifiedEmpty` with the original batch evidence.
7. Persists one aggregate BR-159 audit outcome per code request. Audit failure
   fails closed.

The returned typed record owns both the persistence item and immutable
record-level `SourceEvidence`; the production caller gets read-only accessors.
This prevents the legacy database shape from becoming the evidence contract.

## State and failure contract

- `Available`: one or more accepted records with immutable batch and record
  evidence.
- `VerifiedEmpty`: the provider returned a valid empty batch or exact UTC
  filtering removed every valid source record. This is not an error.
- `Unsupported`: a typed upstream capability/exchange rejection, including
  unadmitted Beijing company-news pages.
- `Unavailable`: transport failure or absence of a verified batch.
- Invalid request, protocol, decode, quality and evidence failures remain
  explicit classified errors; none become an empty collection.

Error retryability follows the upstream category: transport is retryable;
invalid request, unsupported, decode, protocol, Core/evidence and quality
failures are not.

## Field mapping

| Upstream Core field | Existing persistence field | Rule |
| --- | --- | --- |
| `item_id` | `external_id` | preserve provider identity |
| `title` | `title` | non-empty, unchanged |
| no provider summary | `summary` | explicit empty string; never invent |
| `canonical_url` | `url` | HTTPS canonical URL |
| `publisher` | `source_name` | unchanged |
| `published_at` | `published_at` | parse provider `+08:00`, convert to UTC |
| evidence `observed_at` | `fetched_at` | parse immutable provider observation time |
| requested instrument | `code` | exact canonical six-digit code |
| title + blank summary | `content_hash` | existing deterministic hash helper |

Batch and record evidence must each be internally valid. Provider and batch ID
must agree exactly; record source/observation times remain their immutable
page-level values and may differ from the batch's newest-source/final-fetch
times. Database/process time cannot substitute missing evidence.

## Old-module disposition

| Module or surface | Decision | Reason |
| --- | --- | --- |
| Upstream `magic-sina-rs` company news | adopt | official typed provider and evidence contract |
| Upstream `InstrumentNewsRouter` | adopt | canonical provider/evidence admission |
| Local `fetch_stock_news*` and `lid=2516` URL builder | delete | exact retired acquisition surface |
| Local global/top Sina news | retain | active separate capability outside this slice |
| General news aggregator and other providers | retain unchanged | no equivalent caller in this migration |

Stock-only tests and fixtures are deleted after deterministic guards prove
there are no callers. Top-news parser, decoder, transport and tests remain.

## Validation

The first regression signal is:

```bash
if rg -n -U 'post_close_news_review[\s\S]{0,2200}(SinaNewsProvider|fetch_stock_news_in_range)' \
  src/bin/monitor/main.rs; then
  exit 1
fi
```

Additional deterministic guards require:

- the production caller imports and calls `SinaInstrumentNewsGateway`;
- no source contains `lid=2516`, `build_stock_news_url`,
  `fetch_stock_news`, or `fetch_stock_news_in_range`;
- typed tests cover available, verified-empty, exact-time-filtered empty,
  unsupported, unavailable, evidence mismatch and immutable evidence;
- the temporary validation dependency paths point to
  `target/magic_market_unified_work`, then are restored to the adjacent
  `../magic-market-data-rs` project before handoff;
- focused tests, format, strict Clippy, full tests, compliance, coverage,
  release build and bounded real provider evidence are recorded separately.

## Rollback

Revert only this Gateway, its production caller integration, dependency and
BR/document changes. Do not restore the retired `lid=2516` feed as a fallback;
rollback must leave instrument news explicitly unavailable until a verified
provider is restored. Existing database and acquisition audit records are
never removed or rewritten.

## PR evidence

- **Refs:** this design and BR-163.
- **Data-Redlines:** 2.1, 2.2, 2.3, 2.4, 2.7, 2.8, 2.10.
- **OldModules:** disposition table above.
- **Threshold-Proof:** fixed 100-row bound matches the retired five pages of
  20 and is within upstream Sina's 200-row clamp; no config threshold changes.
- **Business-Rules:** BR-159 and BR-163.
- **Rollback:** scoped Git revert; never delete evidence or restore an
  unverified provider fallback.
