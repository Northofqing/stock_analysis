# BR-181 Configuration and README Truthfulness Cleanup

**Status:** Gate A revision 4 approved — independent review 2026-07-29:
0 Critical / 0 Important

**Date:** 2026-07-29

**Release-baseline amendment (2026-08-01):** the original revision-4 review
counted thirteen Magic crates at revision
`660902ff93a07f18367dc16879cf67732accd25a`. The released unified baseline now
contains fourteen crates at
`5f1ce93656a55854c844065390520cd4aecd9a14`; this amendment supersedes only the
dependency-count/revision evidence below and does not rewrite the original
review result.

**Business rule:** BR-181

**Data red lines:** 2.1, 2.2, 2.9, 2.10

**Dead-field amendment (2026-08-01):** repository-wide call-graph evidence
also proves that `screener_interval_min`, `opportunity_min_confidence`, and
`opportunity_require_cross_source` have no production consumer. Their TOML
keys, `MonitorConfig` fields, serde defaults, `Default` projection values and
tests are therefore removed as one unit under BR-181. This does not change a
runtime threshold or schedule because no runtime path read any of the three
values. `opportunity_push_threshold`, `opportunity_use_dual_score`, the public
`opportunity::score` API and the BR-096 machine contract are not part of this
amendment: retiring that public API and compliance contract requires a
separate breaking-change decision and must not begin with an orphaned config
deletion.

## 1. Decision

The TOML inputs owned by `src/config.rs::load_all()` are exactly
`config/strategy.toml` and `config/chain.toml`.
`config/design_contracts.toml` is a compliance input, not a runtime input.
Environment variables and `.env` are separate runtime inputs and are not
reclassified or deleted by this cleanup. Documentation, comments, diagnostics
and examples must describe those boundaries and their implemented behavior.
Deleted files such as `monitor.toml`, `opportunity.toml`,
`chain_rules.toml`, `announce_keywords.toml` and `exclusion.toml` must not be
presented as editable runtime inputs in active source or user documentation.

This cleanup does not add SIGHUP handling. The monitor invokes
`config::load_all()` once during startup. The remaining first-consumption
`chain_mapper` disk read and compile-time TOML fallback are deleted so that
chain rules have the same startup-owned availability as the other three
`chain.toml` projections. Any later hot-reload feature requires a separate
design, signal lifecycle, atomic activation evidence and tests before it may
be documented.

`LiveVetoConfig::default()` and a missing `live_veto.mode` key fail safe to
`dry_run`. The checked-in `config/strategy.toml` explicitly overrides that
fallback with the current active value `live`. This cleanup changes neither
value and must document both without calling either one the other.

`MonitorConfig::opportunity_scan_interval_min`, its default function and the
matching TOML key are removed because the reproducible call-graph evidence in
§5 proves there is no repository consumer. Opportunity times remain owned by
`OpportunitySchedule::default()`; stale comments/logs that claim those times
come from TOML are corrected, without changing the schedule.

The same complete-removal rule applies to `screener_interval_min`,
`opportunity_min_confidence`, and `opportunity_require_cross_source`: their
only repository references are the checked-in TOML key, config projection,
default value and projection test. None reaches a scheduler, filter, scorer or
delivery gate.

`load_risk_config()` has no repository caller but is an exported public API.
It is retained as a deprecated compatibility wrapper in this release rather
than treating repository-local zero calls as proof of zero external users.

## 2. Failure Semantics

`strategy.toml` is currently parsed independently as `RiskConfig` and
`MonitorConfig`; it is not an atomic combined snapshot. Each successful
projection replaces only its corresponding cache, and each failed projection
retains only that projection's previous/default value. The cleanup adds
explicit projection-specific warnings but does not silently claim validation
or change this partial-update behavior.

On `chain.toml` read, parse, or BR-160 validation failure:

- `ANNOUNCE_KEYWORDS` becomes unavailable;
- `CHAIN_INTELLIGENCE` becomes unavailable;
- `CHAIN_RULES` retains its previous value, which is unavailable on a first
  failed startup;
- `EXCLUSION_BOARDS` retains its previous value, which is unavailable on a
  first failed startup.

After deletion of the `chain_mapper` disk/embedded fallback, unavailable
`CHAIN_RULES` yields a typed configuration-unavailable error. It is distinct
from an available rule set that produces zero deterministic matches.
`map_news_to_chains_ai()` must not call Gemini/another LLM when configuration
is unavailable. The existing synchronous mapper, generic-rule lookup, monitor
attribution, impact assessment and asynchronous mapper boundaries must expose
the unavailable reason rather than report “zero matches”. BR-174 already
deleted the former `run_opportunity_scan` and `run_post_close_candidates`
production consumers; this cleanup neither treats those missing functions as
acceptance targets nor restores the legacy candidate path. AI fallback remains
eligible only after a successfully activated rule snapshot was evaluated and
produced zero matches (or the existing generic-rule condition). No old file,
embedded text, AI result or invented rule may convert a configuration failure
into production chain evidence. This is an intentional fail-closed
source-ownership correction, not a threshold change.

The typed unavailable result must also propagate through the synchronous
`map_news_to_chains()` and `is_generic_rule_hit()` APIs. The live monitor
attribution path must return its existing typed failure instead of converting
configuration unavailability to `Ok(Vec::new())`; opportunity impact
assessment must preserve an explicit unavailable disposition instead of
collapsing it to `None`. Unit and `chain_exclusive` integration tests that
exercise the available path must install an explicit in-memory rule snapshot
rather than relying on disk or compile-time fallback.

Missing optional LLM/search credentials remain missing and fail through their
existing typed boundaries. `.env.example` may document
`LLM_DEFAULT_FALLBACK`, but no secret or fake default is added.

## 3. README Data-Source Contract

The top-level README must:

- distinguish A-share identity resolution (Tencent then Sina) from
  Magic-TDX-backed listing-date/company-action lifecycle evidence;
- state that complete security metadata remains explicitly unsupported where
  the Gateway reports it;
- list the already implemented `GlobalMarketGateway`,
  `GeneralWebResearchGateway`, and `ReviewDataGateway`;
- scope `DATABASE_PATH` to monitor/runtime and `STOCK_DB` to the tools that
  actually consume it;
- describe unified-data cutover as Gate B / in progress until release gates
  pass;
- keep QMT only as a broker boundary concept, not as a parser dependency.

The README must not claim that the migration, production selection-v2
activation, live-data validation, coverage or merge is complete before their
evidence exists.

## 4. Preserved Inputs

The following are explicitly not cleanup targets:

- every field in `config/design_contracts.toml`;
- `rules`, `chain_intelligence`, `announce_keywords` and `boards` in
  `config/chain.toml`;
- `LLM_ROLE_NEWS_AI`, the exact mixed-case MiniMax variables and all supported
  search-provider keys in `.env.example`;
- all fourteen Magic crates pinned to released immutable revision
  `5f1ce93656a55854c844065390520cd4aecd9a14`;
- direct Polars 0.54 and the test-only `reqwest_011` dependency.

## 5. Reproducible Evidence, Validation and Rollback

Pre-implementation repository call-graph evidence:

```bash
rg -n --glob '!target/**' --glob '!docs/_archive/**' \
  'opportunity_scan_interval_min' src config tests
# expected: src/config.rs:194 field, src/config.rs:482 initializer and
# config/strategy.toml:29 only

rg -n --glob '!target/**' --glob '!docs/_archive/**' \
  'default_opp_interval' src tests
# expected: src/config.rs:193 serde reference and src/config.rs:271 definition only

rg -n --glob '!target/**' --glob '!docs/_archive/**' \
  'load_risk_config\(' src tests
# expected: one public definition in src/config.rs and no caller

rg -n --glob '!target/**' --glob '!docs/_archive/**' \
  'OpportunitySchedule::default\(' src tests
# expected: active monitor scheduler ownership remains visible

rg -n --glob '!target/**' --glob '!docs/_archive/**' \
  'screener_interval_min|opportunity_min_confidence|opportunity_require_cross_source' \
  src config tests
# expected before implementation: config/strategy.toml plus MonitorConfig
# projection/default/test references only; expected after implementation: no matches
```

Focused acceptance:

```bash
cargo fmt --all -- --check
cargo test --lib config -- --test-threads=1
cargo test --lib opportunity::chain_mapper -- --test-threads=1
cargo test --lib chain_rules_unavailable -- --test-threads=1
# expected exact tests cover: synchronous mapper and generic-rule lookup return
# typed unavailable; monitor attribution and impact assessment preserve it;
# async mapper performs zero AI calls. BR-174-removed opportunity consumers are
# absent and must not be restored as acceptance fixtures.
cargo test --test chain_exclusive -- --test-threads=1
cargo test --test unified_data_architecture -- --test-threads=1
cargo clippy --all-targets --all-features -- -D warnings
bash tools/compliance/lib/check_design_contradiction.sh
bash tools/compliance/lib/check_business_rules.sh
```

Preservation/deletion assertions:

```bash
rg -n 'opportunity_scan_interval_min|default_opp_interval' src config tests
# expected: no matches

rg -n 'screener_interval_min|default_screener_interval|opportunity_min_confidence|default_opportunity_min_confidence|opportunity_require_cross_source' \
  src config tests
# expected: no matches

rg -n --glob '!docs/_archive/**' \
  '(config/)?(monitor|opportunity|chain_rules|announce_keywords|exclusion)\.toml|SIGHUP|热更新' \
  README.md docs/README.md src config
# expected: no active claim that those files or SIGHUP are runtime inputs

test "$(rg -c 'git = "https://github.com/Northofqing/magic-market-data-rs\.git".*rev = "5f1ce93656a55854c844065390520cd4aecd9a14"' Cargo.toml)" -eq 14
# expected: exit 0

rg -q 'design_contracts' tools/compliance/lib/check_design_contradiction.sh
rg -q '^\[chain_intelligence\]' config/chain.toml
rg -q '^\[announce_keywords\]' config/chain.toml
rg -q '^\[\[boards\]\]' config/chain.toml
rg -q '^# LLM_ROLE_NEWS_AI=' .env.example
test "$(rg -c '^# MiniMax_(API_KEY|BASE_URL|MODEL)=' .env.example)" -eq 3
test "$(rg -c '^# (SERPAPI_KEYS|TAVILY_API_KEYS|BOCHA_API_KEYS)=' .env.example)" -eq 3
rg -q '^polars = \{ version = "0\.54",' Cargo.toml
rg -q '^reqwest_011 = \{ package = "reqwest", version = "0\.11\.27",' Cargo.toml
# expected: every command exits 0 independently

rg -q '^\| 全球市场 \| `GlobalMarketGateway` \|' README.md
rg -q '^\| 通用 Web 研究 \| `GeneralWebResearchGateway` \|' README.md
rg -q '^\| 盘后复盘 \| `ReviewDataGateway` \|' README.md
rg -q '^\| A 股证券身份 \| `MarketCapabilitiesGateway` \| Magic Tencent → Magic Sina \|' README.md
rg -q '^\| 上市日与公司行动 \| `SecurityLifecycleGateway` \| Magic TDX \|' README.md
rg -Fxq -- '- `DATABASE_PATH`：monitor、通知与运行时主业务库路径；' README.md
rg -Fxq -- '- `STOCK_DB`：仅供显式读取它的回填、模拟器和合规脚本等离线工具使用；monitor 不读取此变量；' README.md
rg -Fxq 'QMT 仅作为尚未接入的券商执行边界保留，不包含 `qmt-parser` 数据解析依赖。' README.md
rg -Fxq '当前统一数据迁移仍处于 **Gate B / In Progress**。模块存在或能够编译不代表发布完成；在全量测试、合规、覆盖率和真实数据门禁通过前，不宣称 Gate D 就绪。' README.md
# expected: every README contract assertion exits 0 independently
```

Gate D additionally requires:

```bash
cargo test --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
cargo llvm-cov --workspace --all-features --json \
  --output-path target/coverage/coverage.json -- --test-threads=1
python3 tools/coverage/check_thresholds.py target/coverage/coverage.json
cargo build --release --bin monitor
```

The implementation is split into two isolated commits:

1. active-source/config truthfulness plus tests;
2. README/docs truthfulness.

Before release activation either commit may be reverted with
`git revert <exact-commit-sha>` and the focused acceptance commands rerun.
Reverting the active-source/config commit intentionally restores exactly its
prior behavior; it must not touch already deleted provider implementations or
any unrelated migration work. After release evidence, rollback uses a new
reviewed roll-forward change rather than an undocumented partial revert.

## 6. Diagnostic Ordering

The startup banner that prints `screener_min_score` moves after
`config::load_all()` so it reports the activated monitor projection rather
than a pre-load default. No threshold changes.

Active stale comments/logs in `src/config.rs`,
`src/opportunity/chain_mapper.rs`, `src/monitor/news_monitor.rs`,
`src/risk/veto_chain.rs`, `src/bin/winrate_simulator.rs` and
`src/bin/monitor/main.rs`, `src/monitor/attribution.rs`,
`src/opportunity/impact.rs`,
`src/opportunity/candidate_state.rs` and
`src/opportunity/news_outcome.rs` are in scope. Archived design/history
documents are not rewritten and are excluded from zero-match assertions.
