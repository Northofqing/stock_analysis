# Chain Intelligence Gateway / A-10 — Slice 2 Design

**Status:** Gate B implemented; Gate D live/release evidence remains open
**Parent design:** `2026-07-23-magic-market-data-unified-gateway-design.md`
**Data red lines:** 2.1, 2.2, 2.3, 2.4, 2.7, 2.8, 2.10
**Business rule:** BR-160

## 1. Outcome

A-10 must consume a same-trading-date, immutable and traceable chain-intelligence
batch produced through the unified Gateway. It must not read an old
`chain_daily` row whose producer happened to run in a different command or
process.

This slice migrates the complete production path:

```text
full-market upper-limit batch
  + typed board/concept membership
  + provider-backed security identity
  + independently sourced news/catalyst evidence
  -> deterministic chain clustering
  -> immutable batch persistence with provider evidence
  -> A-10 review rendering
  -> downstream AI evidence inventory
```

It does not treat an LLM label as market data, infer concept membership from a
headline, or reuse a cache read time as provider time.

## 2. Reproduced current failure

The current producer is only invoked from the live summary pipeline:

```text
AnalysisPipeline::send_live_summary
  -> legacy MarketAnalyzer::get_limit_up_stocks
  -> pipeline::chain_analysis::run_chain_analysis
  -> DatabaseManager::save_chain_clusters(chain_daily)
```

`monitor --review` only reads `chain_daily`. It does not own a same-date refresh.
Therefore an otherwise healthy review can fail with:

```text
A-10 chain_daily as_of=<old date> 与复盘日期 <review date> 不一致
```

The producer also still calls local acquisition code for:

- full-market limit-up stocks;
- per-stock concepts;
- board discovery and constituents;
- LHB facts;
- research/search/news context;
- laggard quotes.

Replacing only `DataFetcherManager::get_stock_name` would leave the stale-input
and old-provider architecture unchanged.

## 3. Required upstream contracts

Already available in the release line:

- complete exact-date full-market upper-limit pool routed as Eastmoney P1 then Tonghuashun P2;
- TDX normalized security metadata and bars;
- research metadata;
- per-instrument announcements;
- global news providers;
- full-market Dragon-Tiger discovery after the R-04 slice is released.

Core and Router already expose
`BoardMembershipProvider<[InstrumentId] -> DataBatch<BoardMembership>>`.
Magic TDX already parses real industry/concept/index block files, but no
production implementation currently connects those raw records to that Core
contract.

Required before the factual chain batch can enter Gate B:

1. a production Magic TDX `BoardMembershipProvider`;
2. request-bound instrument news with provider publication time when A-10
   requires catalyst text.

Required before the optional non-limit-up/laggard candidate section can enter
Gate B:

3. a typed `BoardConstituentRequest -> DataBatch<BoardConstituent>` discovery
   contract and production Provider/Router source.

The factual A-10 chain batch must not be blocked merely because optional
laggard discovery is unavailable; that section remains explicitly unavailable
until its independent complete batch exists.

This is the BR-160 component split that supersedes BR-114's old report-wide
coupling. BR-114 still governs completeness inside each component: a failed
board directory or constituent response is `Unavailable`, never an empty
component. Once the BR-160 core chain batch is complete, optional Dragon-Tiger,
news and laggard-candidate components may independently remain `Unavailable`
or `NotRequested`; their status and any already-admitted board evidence must be
retained in the report, and they must not affect core membership.

Each board record must preserve provider, source time when provided, observed
time and batch ID. Board name/code aliases may be normalized only from
source-backed fields; fuzzy substring matching must not create membership.

## 4. Gateway interface

The production interface remains async:

```text
ChainIntelligenceGateway::build_for_date(trading_date)
  -> GatewayBatch<ChainIntelligenceBatch>
```

`ChainIntelligenceBatch` contains:

- trading date;
- stable batch identity and calculation version;
- every input batch evidence record;
- accepted chains;
- isolated records with structured reason codes;
- completeness state.

Each chain contains:

- canonical board/concept identity;
- source-backed member securities;
- same-date upper-limit members;
- first-board/continuous-board facts from the limit-pool batch;
- optional same-identity Dragon-Tiger evidence;
- optional independently sourced catalyst/news references;
- deterministic lifecycle observations derived from prior committed batches.

An AI summary is a derived artifact, never a source field. It may annotate a
chain only after the deterministic batch commits.

The committed `ChainIntelligenceBatch` and its immutable input evidence are
the A-10 report-level data gate. Global intraday `DataMode` health describes
unrelated quote/K-line/money-flow/order-book capabilities and must not reject
the independently admitted after-close chain batch a second time. The
`CatalystReview` template therefore accepts global mode `Down`, while all
BR-160 input validation, persistence visibility, acquisition audit,
launch/dedup/cooldown governance, sink receipt and delivery audit remain
mandatory.

Every A-10 dispatcher attempt must also preserve its terminal delivery
category. A pushed outcome may record an empty error; deduplication, governance
denial and sink failure must record a non-empty category-specific reason in the
dispatcher audit. This is an observability requirement only: it must not turn a
non-push outcome into a successful delivery or bypass existing governance.

The authoritative delivery audit is also part of the BR-160 evidence chain.
Generic schema-v2 delivery rows are insufficient because their subject identity
does not prove which immutable chain batch was rendered. A-10 therefore uses a
closed source-batch delivery schema that persists the exact chain batch ID,
batch content SHA-256, source business date and the latest real `observed_at`
from the committed batch inputs. The delivery subject hash is domain-separated
and binds all four values before the ordinary kind/channel identity is derived.
The audit carries `BR-160`; missing/invalid observation time, a date mismatch,
or any tampered lineage field rejects the audit before append. The trading date
remains a date-valued field and must not be converted into a fabricated exact
provider timestamp. Existing generic and counted delivery schemas remain
readable and cannot contain the source-batch-only fields.

All members of a committed chain are same-date upper-limit members. A-10 may
display the first three deterministic members as the front group and the
remaining members as the rest of the same admitted group; it must not relabel
the latter as “pending” or “not started.” The report displays the source-backed
member and continuous-board counts. A score or next-day watch point is shown
only when a separately admitted evidence batch proves it; otherwise the
missing field is explicit and no generic advice is synthesized.

## 5. Deterministic clustering

BR-160 registers these rules before code:

1. validate complete input batches before filtering;
2. join by canonical instrument and board identities only;
3. reject duplicate identity with conflicting content;
4. collapse byte-equivalent/source-equivalent duplicates only;
5. remove generic trading-style labels using a versioned exclusion taxonomy;
6. require at least the configured minimum number of same-date limit-up members;
7. order chains by:
   - accepted limit-up member count descending;
   - continuous-board member count descending;
   - canonical board ID ascending;
8. order members by:
   - board streak descending;
   - source event identity ascending;
   - instrument ID ascending;
9. apply any display limit only after the complete accepted batch is committed.

The production contract is `config/chain.toml [chain_intelligence]`:

- `min_members = 3`;
- `calculation_version = "chain-intelligence-v2"`;
- `taxonomy_version = "tdx-board-exclusions-v1"`;
- `excluded_board_names` is an exact-name list.

Threshold-Proof: three is the smallest cluster that establishes multi-security
co-movement rather than a pairwise coincidence, and it matches the append-only
schema guard (`upper_limit_count >= 3`). Raising the configured value only
changes admission for a new calculation version; it never rewrites historical
batches. Values below three or above 100 fail configuration validation. No
environment-only threshold may silently change historical batch semantics.

`chain-intelligence-v2` is a persistence-identity migration, not a threshold
change. Compared with v1, all input-evidence, chain-row and rejection row IDs
are scoped by the derived batch identity, and member row IDs are additionally
scoped by the parent chain-row identity. This prevents a stable source fact
from colliding when it is legitimately referenced by multiple immutable
derived batches. The v1 rows remain append-only evidence; v2 never rewrites or
falls back to them.

## 6. Persistence

The current `chain_daily(date, concept, stocks, continuation_count)` schema
cannot prove source batches or immutable derivation. Do not extend it into a
second compatibility truth.

Add append-only tables:

```text
chain_intelligence_batch
chain_intelligence_input_evidence
chain_intelligence_chain
chain_intelligence_member
chain_intelligence_rejection
chain_intelligence_visibility_receipt
```

The batch identity includes:

```text
(trading_date, calculation_version, ordered_input_batch_ids, taxonomy_version)
```

Rules:

- same identity + same content is idempotent;
- same identity + different content is `Conflict`;
- every input-evidence, chain-row and rejection row identity is scoped by the
  parent derived batch identity, and every member row identity is scoped by
  its parent chain-row identity; stable evidence or byte-identical facts may
  therefore be referenced by later derived batches without overwriting or
  colliding with earlier rows;
- production rows are never updated or deleted;
- readers only see batches joined to an authoritative visibility receipt;
- the batch/children transaction commits first, then an authoritative
  hash-chain audit commits, then a receipt binds the batch content hash to that
  audit record hash;
- audit and data use the existing production/test physical isolation.

After all consumers migrate and parity evidence is accepted, delete
`chain_daily`, its compatibility fallback and its producer code in the same
release sequence. Historical rows may be retained as read-only migration
evidence but cannot remain a production fallback.

## 7. Scheduler ownership

BR-139 remains the only post-session scheduler. It performs:

```text
latest completed trading date
  -> get or build committed chain batch for that exact date
  -> A-10 render
  -> other consumers / AI
```

Expected states:

- date not settled: `ExpectedWait`;
- all complete inputs prove no chains: `VerifiedEmpty`;
- provider unavailable: explicit retryable failure;
- stale/mixed-date/partial/conflicting input: explicit non-delivery;
- committed same-date batch: render and route through existing PushKind/audit.

No second timer, background refresher or implicit `send_live_summary` side
effect may own the batch.

## 8. A-10 and AI consumption

A-10 preserves its existing task ID, PushKind, template identity and delivery
audit semantics. It renders only facts present in the committed batch.

The AI evidence inventory receives typed references:

```text
capability
provider
source_at
observed_at
batch_id
record identities
quality/completeness
```

It may combine independently complete data families for analysis, but it may
not flatten away provenance or use one Provider to fill missing fields in
another Provider's record.

## 9. Failure modes

- missing/blank board membership: isolate the instrument;
- incomplete board batch: reject the whole membership input;
- limit-pool date mismatch: reject the whole run;
- non-positive quote or invalid percentage: reject before ranking;
- publication time missing/future/stale: exclude that news item and record why;
- no instrument-news Provider: chain facts may still commit, but
  catalyst/news-dependent AI fields remain explicitly unavailable;
- LLM unavailable: deterministic chain batch and A-10 factual rendering remain
  available; no generated score is substituted;
- database or audit failure: no visible committed batch.

## 10. Old-module disposition

| Module | Decision |
|---|---|
| `MarketAnalyzer::get_limit_up_stocks` in chain producer | delete after Gateway cutover |
| `chain_analysis/fetchers.rs` direct board/constituent HTTP | replace with typed Gateway, then delete |
| `FetchSectorTool` as production membership source | remove from production; tool-only compatibility is not retained |
| local LHB map acquisition | replace with released R-04 Gateway |
| local news/search acquisition | replace with evidence-bearing content Gateway |
| `chain_daily` producer/consumer fallback | migrate, prove parity, then delete |
| existing clustering/scoring pure logic | adopt only after inputs and thresholds are made explicit |
| A-10 PushKind/template/audit identity | retain unchanged |

## 11. Validation

Gate B focused tests:

- complete same-date inputs produce a deterministic identity;
- duplicate conflicting membership fails;
- generic-board filtering is versioned and deterministic;
- provider unavailability is not `VerifiedEmpty`;
- stale/mixed-date inputs cannot commit;
- repeated identical build is idempotent;
- AI receives evidence references, not source-less strings;
- A-10 has no `chain_daily` or `DataFetcherManager` acquisition path.

Required commands:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
bash tools/compliance/check.sh
cargo llvm-cov --workspace --all-features --summary-only
cargo run --bin monitor -- --test
cargo run --bin monitor -- --review
```

Live evidence must show one bounded same-date batch summary with input Provider
IDs, source/observed times, batch IDs, accepted/rejected counts and committed
batch identity. Raw titles, account details and unrestricted payloads must not
be copied into audit logs.

## 12. Rollback

Before old-module deletion, rollback is a scoped `git revert` of the slice and
restores the previous reader without modifying committed evidence.

After deletion, rollback deploys the previous release commit; it must not
rewrite, truncate or delete chain batches, account data, delivery audit or
selection outcome history. Restoring the old direct HTTP producer requires a
new reviewed change and is not an automatic fallback.
