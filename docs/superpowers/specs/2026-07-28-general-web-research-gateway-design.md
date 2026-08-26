# General Web Research Gateway Design

**Date:** 2026-07-28
**Status:** Gate A approved under the unified Gateway cutover
**Business rules:** BR-175, BR-242
**Related:** BR-137, BR-159, BR-164, BR-166, BR-239

## 1. Intent

Bocha, Tavily, and SerpAPI are generic web-discovery services. They are not
authoritative financial-news providers. Their only production role is
explicitly labelled research context for an LLM or a human.

This slice moves their protocol ownership into `data_gateway`, preserves typed
batch provenance and failures, and closes the two known paths that could turn a
generic search result into a governed `MarketEvent` or `PolicyHit`.

## 2. Ownership boundary

```text
search_service
  -> GeneralWebSearchProvider (thin, no network)
  -> GeneralWebResearchGateway
       -> Bocha / Tavily / SerpAPI HTTP and wire parsing
       -> validated ResearchOnly batch
  -> SearchResult with ResearchOnly evidence
       -> LLM/human discovery context: allowed
       -> MarketEvent / PolicyHit / candidate fact: rejected
```

The three hosts, clients, credential environment-variable names and parsing,
authentication formats, payload structs, status-code mapping, key rotation,
and result admission exist only in
`src/data_gateway/general_web_research.rs`. Financial/news fact acquisition
continues to use pinned Magic providers through the existing domain Gateways.
There is no fallback from a Magic failure to a generic web search.

## 3. Contract

Every accepted batch carries:

- exact generic-search provider;
- original query;
- `ResearchOnly` use scope;
- UTC observation time;
- non-empty batch ID derived locally from provider, query, observation, and
  admitted record identities;
- a complete or verified-empty outcome.

Every record carries:

- non-empty title;
- HTTPS URL;
- publisher exactly as returned or the URL host when the provider omits it;
- optional provider publication text and an explicit date-quality state;
- the same provider, batch ID, and observation time as its batch.

Missing publication time remains missing. A publication string that cannot be
consumed completely is rejected, as is a publication time later than the
observation time. URLs are not rewritten and snippets are not fabricated.

## 4. Filtering, ordering, deduplication, and limits

The request limit is `1..=50`. Provider results retain provider order and are
truncated only after a complete response has been parsed and validated.
Duplicate canonical URLs, or conflicting records for the same URL, reject the
whole batch. Cross-provider fields are never merged.

For LocalBridge transport, BR-242 binds an exact request provider (`Bocha`,
`Tavily`, or `SerpApi`), non-empty query, and limit. The server invokes only the
requested provider. The response provider/source, every record's provider and
batch evidence, and `records.len() <= requested_limit` are revalidated at the
client boundary. Unknown/extra request keys, provider substitution, implicit
fallback, mixed evidence, or an over-limit response fail closed.

The existing search service may rerank already admitted `ResearchOnly` records
for display/context. Every rendered/LLM context must visibly state
`ResearchOnly` and that it is not a financial fact or trading/selection basis.
That local ranking is not source evidence and cannot change the use scope.

## 5. Failure modes

| Failure | Typed result | Retry |
|---|---|---|
| Empty/missing credentials | `missing_credentials` | no |
| Invalid query/limit | `invalid_request` | no |
| Timeout/transport | `transport` | yes |
| 401/403 | `authentication` | no |
| 429 | `rate_limited` | yes |
| Other non-success status | `provider_rejected` | status dependent |
| JSON/schema mismatch | `protocol` | no |
| Bad URL/title/date/evidence/dedup | `invalid_evidence` | no |
| Valid response with zero records | `VerifiedEmpty` | n/a |

No failure becomes a successful empty financial/news batch.

## 6. Fact-upgrade barriers

`SearchResultAdapter::to_raw` and `classify_policy` must fail closed when a
result is `ResearchOnly` or has no governed source-fact evidence. This prevents
generic discovery from becoming a `MarketEvent`, policy catalyst, selection
candidate, or backtest fact.

The older `SearchResult` values used by explicitly governed source producers
must carry source-fact scope and evidence from their source adapter. Absence of
scope/evidence is unverified, not implicitly trusted.

## 7. Old modules

| Module | Decision | Reason |
|---|---|---|
| `search_service/providers/bocha.rs` | delete | owns HTTP and wire parser outside Gateway |
| `search_service/providers/tavily.rs` | delete | owns HTTP and wire parser outside Gateway |
| `search_service/providers/serpapi.rs` | delete | owns HTTP and wire parser outside Gateway |
| `search_service/types::ApiKeyManager` | migrate/delete | provider protocol concern |
| existing Magic financial/news Gateways | adopt unchanged | authoritative fact sources |

## 8. Validation and rollback

Validation:

```bash
cargo test --test unified_data_architecture
cargo test general_web_research
cargo test event_extractor
cargo test classifier
cargo fmt --check
cargo check --all-targets --all-features
```

Rollback is a file-scoped revert of BR-175, this design, the new Gateway, thin
adapter/types, upgrade barriers, and architecture tests. No database migration
or destructive data rollback is involved.
