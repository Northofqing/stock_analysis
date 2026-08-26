# Unified Financial and News Data Final Cutover

**Date:** 2026-07-25
**Status:** Gate B / In Progress
**Rules:** BR-099, BR-148, BR-153, BR-156, BR-158, BR-159, BR-164, BR-166,
BR-168; AGENTS.md 2.1, 2.2, 2.3, 2.4, 2.7, 2.8, 2.9, 2.10

## 1. Objective

Finish the repository-wide migration described by
`2026-07-23-magic-market-data-unified-gateway-design.md`. Every production
financial-market or financial-news request must enter through
`stock_analysis::data_gateway` and consume a released
`magic-market-data-rs` provider batch. After a domain is proven equivalent or
better, its local HTTP client, parser, fallback, dependency and configuration
are deleted.

The final production flow is:

```text
official/public source
  -> magic-market-data-rs Provider
  -> magic-market-core typed DataBatch + SourceEvidence
  -> magic-market-router complete-batch admission
  -> stock_analysis::data_gateway
  -> persistence / review / selection / risk / content governance
```

Account ownership, cash, positions, net asset value and order placement stay
outside this migration. They require real broker evidence and must remain
explicitly unavailable when the broker contract is absent.

## 2. Source ownership

| Domain | Canonical upstream provider | Downstream owner |
| --- | --- | --- |
| Realtime A-share quote | Magic TDX → Tencent → Sina | `MarketDataGateway` |
| Daily A-share bars | Magic TDX → Tencent → Sina → Baidu | `HistoricalBarsGateway` |
| Minute bars, order book and security metadata | Magic TDX → Tencent → Sina | `MarketCapabilitiesGateway` |
| A-share indices | Tencent through Magic | `IndexDataGateway` |
| Financial statements | Sina through Magic | `CompanyDataGateway` |
| Company market statistics | Tencent through Magic | `CompanyDataGateway` |
| Research reports and documents | Eastmoney through Magic | `ResearchDataGateway` |
| Consensus | Eastmoney through Magic | `ConsensusDataGateway` |
| Board directory/membership and board daily flow | TDX / Eastmoney through Magic | `BoardDataGateway` |
| Instrument/post-close flow and northbound statistics | Eastmoney / HKEX through Magic | `CapitalDataGateway` |
| Limit-up review inputs and selection bars | Eastmoney / Tonghuashun / TDX through Magic | `ReviewDataGateway`, `MagicTdxGateway` |
| Dragon tiger | Eastmoney through Magic | `DragonTigerGateway` |
| Announcements | CNInfo through Magic | `EventCalendarGateway` |
| Instrument/global news | Sina / Eastmoney / CLS / Jin10 / The Paper through Magic | `SinaInstrumentNewsGateway`, `GlobalNewsGateway` |
| Published economic releases | Jin10 through Magic | `EconomicCalendarGateway` |
| Futures delivery notices | CFFEX through Magic | `FuturesDeliveryGateway` |
| Checked-in A-share calendar notice authority | SSE / SZSE official notice URLs | `exchange_calendar_authority` |

The pinned TDX quote batch currently lacks a provider timestamp with enough
precision to satisfy the strict five-second admission rule. TDX remains first
in the declared route, but that batch cannot win until it can prove the
required `source_at`; Tencent or Sina may therefore be the admitted realtime
batch. Local observation time must not be substituted.

The following capabilities are not production Gateway facts today: generic
trades, generic normalized `MoneyFlow`, THS/iWencai data, investor questions,
State Council/MIIT policy feeds and non-CFFEX delivery calendars. They remain
explicit `Unsupported`/`Unavailable`; a search result, cache row or related
field must not be promoted to these contracts.

Generic web search, LLM calls, Feishu/WeChat delivery, broker APIs and local
observability HTTP are not financial acquisition providers and are not
rewritten by this cutover. Search results cannot be promoted to financial facts
without a typed upstream contract.

## 3. Gateway shape

The existing focused gateways remain valid. The current composition root
exposes the actual modules below rather than provider constructors:

```text
src/data_gateway/
  market_data.rs
  historical_bars.rs
  market_capabilities.rs
  index.rs
  company.rs
  research.rs
  consensus.rs
  board.rs
  capital.rs
  review.rs
  dragon_tiger.rs
  event_calendar.rs
  global_news.rs
  sina_instrument_news.rs
  economic_calendar.rs
  futures_delivery.rs
  exchange_calendar_authority.rs
  chain_intelligence.rs
  magic_tdx.rs
  audit.rs
  error.rs
```

Only `src/data_gateway/**` may import `magic-*-rs` provider clients. Business
modules receive typed records and batch evidence. Architecture regressions fail
when a production source file outside the gateway:

1. constructs a Magic provider;
2. contains an allowlisted financial-source URL;
3. creates a financial `reqwest` transport;
4. implements a financial-source response parser or fallback.

The check excludes tests, documentation, generic search, notification,
broker/account and local observability URLs.

## 4. Admission and status

Every request has exactly one complete provider batch. Cross-provider field
splicing is forbidden. Gateways expose:

- `Available`;
- `VerifiedEmpty`;
- `Unavailable`;
- `Stale`;
- `Partial`;
- `Conflict`;
- `Unsupported`.

`source_at` and `observed_at` are never interchangeable. A cache hit retains the
original provider age. Missing optional values remain absent. Missing required
values, bad identities, invalid prices, duplicated or discontinuous time
series, unproved emptiness and partial pagination reject the batch.

For blocking provider routes, the freshness-validation clock is captured only
after the provider returns. Capturing the comparison clock before network
acquisition would make the provider's later, truthful local `observed_at`
appear to be in the future and would incorrectly classify a newly acquired
batch as stale. Provider `observed_at` remains the acquisition evidence; the
post-route clock is used only to calculate its age.

Realtime quotes must satisfy the five-second rule. Position/cash freshness is
not inferred from market data. Daily/historical batches must satisfy the
one-trading-day rule and the repository freshness gate.

Each request writes one BR-159 immutable acquisition-audit result. Audit failure
fails the gateway closed. Per-record rejection logs are grouped by batch and
reason with bounded samples.

### 4.1 Configuration Threshold-Proof

These are the only `config/strategy.toml` changes authorized by this cutover:

| Config field/section | Action | Proof and linked rule |
| --- | --- | --- |
| `dq_quote_stale_sec` | Set to `5` seconds | Equal to AGENTS.md 2.4 and the existing Rust default; BR-148/BR-153/BR-156 require strict provider-time freshness. It does not exceed or conflict with a clamp. |
| `[schedule]` | Delete the empty table | It contains no deserialized value and changes no runtime threshold or schedule behavior. |
| `[st_price_limit].st_daily_limit` | Delete the unmapped table/field | `MonitorConfig`/`RiskConfig` do not deserialize it. Removal changes no runtime price-limit threshold; real order price limits remain governed by the order-safety contract. |
| `[gem_market_maker].liquidity_threshold` | Delete the unmapped table/field | `MonitorConfig`/`RiskConfig` do not deserialize it. Removal changes no runtime liquidity threshold and does not authorize a new clamp/default. |
| `[v17_7_sources.earnings]` → `[v17_7_earnings]` | Correct the table name | `MonitorConfig` owns the direct `v17_7_earnings` field. The four configured values and `EarningsConfig::validate` sign constraints do not change; a non-default-value parse test prevents silent fallback. |
| `[trading]`, `[slippage]`, `[performance]`, `[regime]`, `[exposure]`, `[alert]` | Delete the tables and their dead `RiskConfig` fields/types | They were deserialized but had no production consumer. Removing inactive knobs changes no production threshold; the executable owners of fees, analytics, market state, exposure and alert behavior remain unchanged. |
| `[account_mode]` | Retain unchanged | It is the only `RiskConfig` section read by production monitor call sites and remains governed by BR-021. |

The consumer audit was run against the repository before deletion:

```text
$ rg -n -U "get_risk_config\\([\\s\\S]{0,160}" src --glob '*.rs'
src/config.rs:718:pub fn get_risk_config() -> RiskConfig {
src/bin/monitor/main.rs:1890:    let thresholds = stock_analysis::config::get_risk_config()
src/bin/monitor/main.rs:1891:        .account_mode
src/bin/monitor/main.rs:1994:    let thresholds = stock_analysis::config::get_risk_config()
src/bin/monitor/main.rs:1995:        .account_mode
src/bin/monitor/push_templates.rs:13408:            &stock_analysis::config::get_risk_config()
src/bin/monitor/push_templates.rs:13409:                .account_mode
```

No fee, slippage, performance, regime, exposure or alert call site consumed
those six sections. All other live or uncertain thresholds, including
`[account_mode]`, remain unchanged until a separate design proves their owner,
bounds and rollback.

## 5. Domain cutover sequence

The sequence prevents a production capability gap:

1. pin all Magic crates to one immutable upstream Git revision;
2. market/calendar gateways;
3. company/research gateways;
4. signal/capital gateways;
5. news/announcement gateways; policy remains explicitly unsupported until a
   separate typed provider contract is linked;
6. migrate all callers and add source-level architecture tests;
7. delete replaced local acquisition modules and configuration;
8. remove unused direct HTTP dependencies;
9. rewrite README and run release evidence.

Deletion happens only after the relevant gateway tests and callers compile.
There is no permanent compatibility switch and no hidden legacy fallback.

### 5.1 Final consumer projections

The final cutover may retain downstream analysis models, but their acquisition
must be removed. A projection is admitted only when the Magic field has the
same semantics as the downstream field:

- the legacy financial view currently maps only Sina income-statement
  `basiceps`/“基本每股收益”; every old F10 ratio or growth field remains absent
  until an equivalent typed upstream field is available;
- Eastmoney research projections retain the source organization ID and the
  source-proven `indvAimPriceT` upper bound / `indvAimPriceL` lower bound.
  `ConsensusDataGateway` computes each side's average only from present values
  in the same admitted research batch. A report whose lower bound exceeds its
  upper bound rejects the batch; a missing side remains absent and is never
  copied from the other side or joined from another request;
- 15/60-minute entry bars are built only from one admitted minute batch, in
  separate Shanghai morning and afternoon sessions; negative cumulative-volume
  deltas, lunch-spanning buckets and incomplete current buckets are rejected or
  omitted as specified by BR-168;
- the monitor announcement view maps only CNInfo identity, security code,
  title, provider publication time and canonical URL. Missing company name,
  summary and body remain blank; post-admission title classification is
  downstream analysis, not a second acquisition path.

These adapters retain the original batch evidence and may never call a deleted
`data_provider` transport as a fallback.

### 5.2 BR-099 / BR-159 candidate orchestration

The production D-01/P-03 candidate path uses one explicit, evidence-preserving
assembly:

```text
strict chain/P5 candidate identities
  -> merge and canonical ordered code set
  -> complete realtime quote Gateway batch
  +  complete company market-statistics Gateway batch for the same code set
  -> pure identity/cardinality/evidence validation and projection
  -> BR-099 hard gates using confirmed position codes only
  -> deterministic heat sort
  -> D-01/P-03 consumers
```

Watch-list membership is not a held-position fact and must not trigger the
BR-099 held-position exclusion. Only `portfolio::get_positions()` codes are
passed to that gate. Quote and statistics rows must each match the requested
ordered universe exactly and retain their own BR-159 `BatchEvidence`; the two
batches are never treated as one provider batch or used to fill unrelated
fields. `volume_ratio` is projected only from the admitted Tencent market
statistics row. A provider-missing value remains `None`.

The pinned MoneyFlows capability does not provide the candidate
`main_net_yi` contract. It therefore stays `None`, so candidate `heat_score`
also stays `None`. Source labels alone do not fabricate strong evidence.
P-03 rejects a missing/invalid volume ratio explicitly and maps only real
values to the existing weak/mid/strong tiers (`<1`, `1..<3`, `>=3`).

Failure modes are fail-closed: either batch unavailable/empty, duplicate or
extra/missing identity, provider/batch evidence contradiction, illegal quote,
or invalid statistics value rejects the whole candidate assembly. Blocking
quote clients are created and dropped inside the existing blocking worker;
the async statistics Gateway is awaited directly, so no Magic runtime is
dropped inside Tokio.

The former synchronous `main.rs` candidate-panel test wrapper is obsolete and
is not a production fallback. D-01 dry-run, diagnostics and P-03 await the
same async loader. Rollback is `git revert <candidate-orchestration-sha>`;
there is no configuration switch or legacy acquisition path to re-enable.

## 6. Old module disposition

| Existing path | Final disposition |
| --- | --- |
| Former local market acquisition | deleted/replaced by `MarketDataGateway`, `HistoricalBarsGateway`, `MarketCapabilitiesGateway`, `IndexDataGateway` |
| Former local company/research/consensus acquisition | deleted/replaced by `CompanyDataGateway`, `ResearchDataGateway`, `ConsensusDataGateway` |
| Former local board/capital acquisition | deleted/replaced by `BoardDataGateway`, `CapitalDataGateway` |
| Former local announcement/news acquisition | deleted/replaced by `EventCalendarGateway`, `SinaInstrumentNewsGateway`, `GlobalNewsGateway`, `EconomicCalendarGateway` |
| Former local fallback manager | deleted; provider routing belongs upstream |
| `market_analyzer/lhb_review.rs` source client | delete after R-04 gateway cutover |
| Generic `search_service/providers/**` | retain only as discovery/LLM context; never promote its results to typed financial facts |
| THS/iWencai/government-policy/investor-question plans | no linked production Gateway; retain explicit unsupported state |
| source host lists, signing keys and request tuning | delete when no caller remains |
| broker/account/order modules | retain; outside scope |

If a file mixes acquisition with downstream analysis, only its acquisition
transport/parser is removed; analysis is retained behind a typed gateway input.

## 7. Failure modes

- Upstream transport/authentication/rate limit: typed unavailable; no empty
  vector and no old-source retry.
- Unsupported upstream capability: visible disabled/unsupported state; no local
  implementation resurrected.
- Stale provider batch: reject before persistence or analysis.
- Partial pagination or missing detail rows: reject the atomic batch.
- Provider conflict: preserve evidence and fail closed.
- Gateway audit/database failure: no consumer-visible success.
- Broker/account evidence missing: account-dependent consumers remain frozen;
  public market data cannot authorize account actions.
- CFFEX unreachable from one network: retain typed transport failure and retry
  eligibility; never downgrade to HTTP or calculate a synthetic “official”
  delivery event.

## 8. Runtime and concurrency

Async callers await async gateway operations. Blocking Magic clients are
created, used and dropped wholly inside `spawn_blocking`; they are never
dropped inside Tokio async context. Bounded concurrent requests retain upstream
rate limits. A request cancellation cannot turn a partial result into
`VerifiedEmpty`.

## 9. Validation

Required Gate B/C/D evidence:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features -- --test-threads=1
bash tools/compliance/check.sh
git diff --check
cargo llvm-cov --workspace --all-features --json --output-path target/coverage.json
python3 tools/coverage/check_thresholds.py target/coverage.json
cargo run --bin monitor -- --review
cargo run --bin monitor -- --test
cargo run --bin monitor
```

The normal monitor run is bounded and stopped gracefully after startup,
gateway acquisitions and at least one scheduler cycle. Live evidence must show
provider/source/provider time where available/local observation/batch ID or an
explicit typed failure. Overall coverage is at least 80%; critical trading and
data paths are at least 95%.

## 10. Rollback

Each domain is committed separately. Rollback is `git revert <domain-sha>`.
Audit rows, user position snapshots, account evidence, trade/order records and
delivery history are never deleted or rewritten. A rollback restores the
previous code revision only; it cannot silently reactivate a removed source
without reverting the corresponding reviewed commit.
