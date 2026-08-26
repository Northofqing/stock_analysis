# BR-191 Retire sector-history JSONL and shadow NewsRanker

Status: Gate A approved by parent task delegation; Gate B implementation in
progress.
Business rules: BR-061, BR-117, BR-155, BR-169, BR-191.

## 1. Decision

Delete the dormant `market_analyzer::sector_history`,
`opportunity::news_ranker`, and `opportunity::news_audit` modules. They are not
adapted to the unified Gateway because the released board contract does not
provide the complete historical market-shape evidence their scoring semantics
require.

The review-owned `HeatStage` vocabulary remains useful for rendering and
aggregation. It moves to `review::market_stage`; no stage may be inferred from
the retired JSONL or a default market context.

The authoritative event-selection path remains selection-v2. This change does
not restore a legacy candidate producer or change any selection-v2 schema,
receipt, ordering, filtering, or delivery rule.

## 2. Evidence and failure analysis

### 2.1 sector-history JSONL

`append_today*` has no production caller. `BoardDay` lacks provider,
`source_at`, `observed_at`, immutable batch identity, and content hash. The
loader validates every board code as a continuous daily series even though a
top-N board universe naturally changes between dates. The writer rewrites the
whole local file and does not implement the durable append-only audit contract
required by data-redline 2.7.

Consequently the file cannot be treated as a production source, a freshness
proof, or a replay authority.

### 2.2 shadow NewsRanker

`shadow_rank_hits` has no production caller. Its opt-in path constructs
`MarketContext::default()`, turns missing limit-up counts into zero, awards a
freshness score when publication time is missing, and stamps locally generated
observations as if they were source facts. The remaining monitor code only
logs that the path is disabled for precisely this reason.

`news_audit` is another opt-in local JSONL writer with warn-only failure
handling and no production caller. It is not a tamper-resistant delivery or
selection audit.

### 2.3 Unified Gateway boundary

The released board Gateway can return typed board identity/membership and a
limited current board-flow sample. It does not expose one complete,
provider-owned historical batch containing the old ranker's turnover, volume
ratio, today/5-day flow ratios, exact requested trading date, and stable
all-universe coverage. Filling those fields from defaults or unrelated
providers would violate 2.1, 2.2, 2.4, and 2.7.

## 3. Data flow after retirement

```text
provider-owned news batch
  -> source-fact admission and immutable selection-v2 ingress
  -> versioned relation / market feature capture
  -> receipted selection-v2 candidate and outcome paths

review renderers
  -> explicit HeatStage supplied by their own complete review input
  -> presentation only
```

There is no fallback from selection-v2 to local JSONL ranking.

## 4. Historical data disposition

Existing `data/sector_history.jsonl` and `data/news_rank_audit*.jsonl` are left
untouched. They are isolated historical artifacts:

- not authoritative;
- not read by production code;
- not eligible to seed or repair selection-v2;
- not evidence of provider freshness, membership, market stage, or outcome.

Operators may archive them externally after independent review. This change
does not delete or rewrite user data.

## 5. Old-module decisions

| module | decision | reason |
| --- | --- | --- |
| `market_analyzer::sector_history` | delete | no producer, incomplete provenance, invalid universe-continuity assumption |
| `opportunity::news_ranker` | delete | no production caller; default/missing values violate source contract |
| `opportunity::news_audit` | delete | no production caller; local warn-only JSONL is not authoritative audit |
| `review::market_stage::HeatStage` | adopt | still-used review vocabulary, independent of legacy acquisition |
| selection-v2 | retain unchanged | formal receipted event-scoped candidate chain |

## 6. Failure modes

- Any future feature needing historical board stage must stay unavailable until
  a complete provider-owned Gateway batch and freshness contract are released.
- Historical JSONL discovery must not auto-import or enable a fallback.
- Missing review stage remains `Unknown`; it must not be converted to a
  fabricated score or candidate.
- A residual active import, environment switch, or monitor log is a static
  validation failure.

## 7. Validation

Worker-owned static checks:

```bash
rustfmt --edition 2021 --check \
  src/review/market_stage.rs \
  src/review/performance_feedback.rs \
  src/review/limit_chain_review.rs \
  src/opportunity/mod.rs \
  src/market_analyzer/mod.rs \
  src/bin/monitor/main.rs
git diff --check
rg -n \
  'sector_history|SECTOR_HISTORY_PATH|NEWS_RANKER_SHADOW|NEWS_RANK_AUDIT|shadow_rank_hits|opportunity::news_ranker|pub mod news_ranker|pub mod news_audit' \
  src README.md docs/README.md config
```

Root-owned full gates:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
bash tools/compliance/check.sh
```

## 8. Rollback

Revert only the BR-191 commit/PR. Do not restore or transform any historical
`data/` file. If a removed consumer is later required, design a new Gateway
contract and selection-v2 integration at Gate A rather than re-enabling these
modules.
