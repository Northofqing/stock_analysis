# Gate C 分层门禁与覆盖率棘轮实施计划

> **2026-08-27 当前实施权威：** 本节取代下方历史任务清单。用户已经确认
> `docs/superpowers/specs/2026-08-02-gate-d-coverage-closure-design.md` §10 的修订方案：
> 核心改动覆盖率至少 90%，其他生产代码改动覆盖率至少 85%，全仓 global/core 只升不降；
> Gate D 仍保留 global 80%、core 95%、真实 freshness 与 live/audit 证据。

**目标：** 让普通 PR 用当前改动能够提供的证据完成 Gate C 合并，同时保持数据、资金安全红线
和 Gate D 发布标准不降级。

**实现边界：** 只深化现有 `check_thresholds.py`、`check.sh` 与两个 workflow；测试继续写入
已有且受跟踪的 `tests/test_coverage_thresholds.rs` 和 `tests/test_data_freshness_check.rs`。覆盖率
基线写入已有 `config/design_contracts.toml [coverage]`；不创建或强制跟踪 `.gitignore` 排除文件。

### Task 40：固化 Gate A 规则、阈值证据与回滚边界

**文件：** `AGENTS.md`、`docs/ENGINEERING_RULES_V2.md`、`.github/copilot-instructions.md`、
`CLAUDE.md`、`docs/superpowers/specs/2026-08-02-gate-d-coverage-closure-design.md`、
`docs/business_rules.md`、`config/design_contracts.toml`

- [x] 将 Gate C 明确定义为 PR 合并门禁，Gate D 定义为发布/部署门禁；任何数据红线仍然阻断。
- [x] 登记 BR-252 与 `[coverage]`：经两次新鲜完整报告复现，global `201256/258810`、
  core `157635/202935`、218 个核心
  文件、Rust 1.95.0/LLVM 22.1.2/cargo-llvm-cov 0.8.7，以及 PR 90%/85%、Release 80%/95%。
- [x] 记录阈值证明：当前分支核心改动 `13707/14923=91.85%`、其他生产改动
  `8368/9824=85.18%`；不得把统一 95% 继续作为当前 PR 的历史债代理。
- [x] 运行 `git diff --check` 与 `bash tools/compliance/lib/check_design_contradiction.sh`；结果 PASS。
- [x] 独立提交规则与计划（`ee8c8d1`），提交信息引用 BR-252、spec §10 与 `[coverage]`；回滚只 revert 该提交，
  不删除报告、审计或真实持仓数据。

### Task 41：测试先行实现覆盖率双策略

**文件：** `tests/test_coverage_thresholds.rs`、`tools/coverage/check_thresholds.py`

- [x] 先扩展真实 CLI 进程测试，建立临时 Git checkout、JSON/LCOV 报告和 `[coverage]` 配置；
  验证 release/default 仍按 80%/95%，缺核心行与坏报告 exit 2。
- [x] 新增失败测试：核心恰好 90% 通过、低一行失败；非核心恰好 85% 通过、低一行失败；某桶无
  可执行改动输出 N/A；删除-only 不伪报 100%；缺 base/LCOV/配置、路径逃逸与坏 diff exit 2。
- [x] 新增棘轮测试：当前比例等于/高于 `[coverage]` 通过；global 或 core 任一低于 baseline
  失败；base 已有 baseline 时 candidate 比例下降失败；base 无 baseline 且未传
  `--bootstrap-baseline` 失败。
- [x] 实现接口：旧位置参数保持 release 兼容；新接口为
  `--policy pr|release --report <json> [--lcov <lcov> --base-ref <ref> --bootstrap-baseline]`。
  JSON 负责全仓/核心统计，LCOV `DA` 负责差分可执行行，Git `--unified=0` 负责新增/修改行；
  两个 patch 桶独立按整数交叉相乘判定。退出码固定 0=通过、1=政策未达标、2=不可验证。
- [x] 运行 `cargo test --test test_coverage_thresholds -- --test-threads=1`（14/14 PASS），随后
  `python3 tools/coverage/check_thresholds.py target/coverage/coverage.json` 仍应以 release 语义
  报告当前 77.77%/77.69% 并 exit 1。
- [x] 独立提交 coverage policy 与测试（`577366b`）；回滚不触碰 `[coverage]` 历史证据。

### Task 42：测试先行实现合规双策略

**文件：** `tests/test_data_freshness_check.rs`、`tools/compliance/check.sh`

- [x] 先新增真实进程测试：`--policy pr` 必须运行仓库全部离线检查且不调用 freshness；
  `--policy release` 与无参数默认都必须调用 freshness；未知参数必须非零且不运行任何子脚本；
  缺失数据库造成的 freshness 失败必须向上传播。禁止测试目录注入，避免形成可替换检查脚本的旁路。
- [x] 给 `check.sh` 增加严格参数解析；检查目录始终固定为仓库 `tools/compliance/lib`。仅 PR policy
  跳过 `check_data_freshness.sh`，其他检查列表完全一致。
- [x] 运行 `cargo test --test test_data_freshness_check -- --test-threads=1`（7/7 PASS）与
  `bash tools/compliance/check.sh --policy pr`；预期 PASS。不得运行回填或把 PR skip 写成 freshness PASS。
- [x] 独立提交 compliance policy 与测试（`929324f`）。

### Task 43：让 CI 只签发其拥有的 Gate C 证据

**文件：** `.github/workflows/coverage.yml`、`.github/workflows/compliance.yml`、
`tests/test_coverage_thresholds.rs`、`tests/test_data_freshness_check.rs`

- [x] 先加 workflow 静态测试：coverage 必须 `fetch-depth: 0`、固定 Rust `1.95.0`、安装
  `llvm-tools-preview` 与 cargo-llvm-cov `0.8.7`、生成 JSON+LCOV、调用 `--policy pr` 与 base SHA；
  compliance 必须调用 `--policy pr`，不得出现 `backfill_daily`。
- [x] 修改 workflow：PR 使用 base SHA，push 使用 before SHA；首次 base 无 `[coverage]` 时显式传
  `--bootstrap-baseline`。CI 不读取生产库、不自动回填、不输出 Gate D PASS。
- [x] 运行两个测试 binary（15/15、8/8 PASS）与 YAML 解析；`git diff --check` 和完整 PR compliance
  纳入 Task 44 最终验证。
- [x] 独立提交 workflow 与静态测试（`671e441`）。

### Task 44：完成 Gate C 验证、PR 证据与合并

- [x] 闭合独立审查发现的证据旁路：改动源码遗漏/rename/路径逃逸/坏 diff/未知 schema 必须
  exit 2；bootstrap 固定阈值下限、BR-252 批准和 source SHA；release 高覆盖 PASS 必须具备
  完整工具、源码和 LCOV provenance。
- [x] Release compliance 必须显式绑定固定生产 `STOCK_DB` 并拒绝测试日期/日历覆盖；PR 证据
  强制 baseline 与 Gate C/Gate D 分列；compliance workflow 使用 `fetch-depth: 0`。
- [x] 复现并纠正首次 bootstrap 的不可复现计数：两次同源码、同工具、同测试集完整运行均为
  global `201256/258810`、core `157635/202935`；只纠正基线事实，不改变任何门禁阈值。
- [x] 运行 `cargo fmt --all -- --check`。
- [x] 运行 `cargo clippy --workspace --all-targets --all-features -- -D warnings`。
- [x] 运行 `cargo test --workspace --all-targets --all-features -- --test-threads=1`。
- [x] 运行 `bash tools/compliance/check.sh --policy pr`。
- [x] 重新生成 workspace/all-features JSON+LCOV，并运行
  `python3 tools/coverage/check_thresholds.py --policy pr --report target/coverage/coverage.json --lcov target/coverage/lcov.info --base-ref c6024e5 --bootstrap-baseline`；预期核心 patch ≥90%、其他 patch ≥85%、global/core 不低于 baseline。
- [x] 更新 BR-252 为 Gate C PASS，并在 PR 描述完整填写 Refs、Data-Redlines、OldModules、
  Threshold-Proof、Business-Rules、Rollback、Gate-Policy 与 Bootstrap-Baseline。
- [ ] 只在上述 Gate C 证据和独立审查均无 Critical/Important 后提交、推送并合并。Gate D 若仍未
  达到 global80/core95 或未获真实 freshness/live 授权，明确标记 `Release Blocked`，不得部署。

---

> **Historical evidence only (superseded 2026-08-02):** Do not execute the
> remaining checklist against the current tree. It references retired
> `src/data_provider/**`, RustDX, and PR #2. Current coverage ownership, scope,
> failure semantics, and closure slices are defined by
> `docs/superpowers/specs/2026-08-02-gate-d-coverage-closure-design.md` and its
> future post-review implementation plan. Completed entries below are retained
> only as audit history.

> **For agentic workers:** REQUIRED SUB-SKILL: Use TDD to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Raise registered core line coverage to at least 95% and repository-wide line coverage to at least 80%, add truthful nullable same-day account evidence, pass Gate A-D, and merge PR #2 into `master`.

**Architecture:** Add vertical behavior tests through existing deep module interfaces, extracting only internal dependency seams where orchestration code is otherwise unreachable. Work core-first, recompute llvm-cov after each batch, then close global gaps; keep live account data local and persist missing daily P&L as nullable with source provenance.

**Tech Stack:** Rust, Tokio, Serde JSON, Chrono, SQLite/rusqlite/r2d2, `cargo llvm-cov`, shell compliance gates, GitHub PR workflow.

---

## File structure

- Modify core behavior/tests beside implementation under `src/{pipeline,data_provider,database,trading,decision,risk,event}/`.
- Add integration/process coverage under `tests/` only when behavior crosses a public process or crate interface.
- Modify owning `src/database/` migration/model/repository files for nullable same-day account evidence; private values never enter source or tests.
- Update `docs/business_rules.md`, the Gate D design, handoff, and `.planning/2026-07-16-event-replay-safety-remediation/` evidence after every batch.
- Regenerate `target/coverage/coverage.json`; it remains local build evidence unless repository policy explicitly tracks it.

### Task 1: Cover score breakdown and veto behavior

**Files:**
- Modify: `src/pipeline/score_breakdown.rs`
- Modify: `src/pipeline/veto_rules.rs`

- [x] **Step 1: Add one score tracer bullet**

Add a `#[cfg(test)]` module with a complete deterministic `KlineData` builder and assert missing optional evidence remains neutral while sentiment is clamped:

```rust
let score = compute(
    &ScoreInputs {
        sentiment_score: 120,
        money_flow: None,
        money_flow_section: None,
        volume_ratio_5d: None,
    },
    &kline(10.0),
);
assert_eq!(score.technical, 100);
assert_eq!(score.fundamental_quality, 50);
assert_eq!(score.valuation_safety, 50);
assert_eq!(score.capital_flow, 50);
assert_eq!(score.growth_sustainability, 50);
```

- [x] **Step 2: Run the tracer bullet**

Run: `cargo test --lib pipeline::score_breakdown::tests::missing_evidence_is_neutral_and_sentiment_is_clamped -- --exact`

Expected: PASS. This is a characterization slice and should require no production behavior change.

- [x] **Step 3: Add score behaviors vertically**

Cover independently worked outcomes for factor actions/clamps, valuation percentile and target-price bands, raw money-flow bands, one-day bounce cap, legacy section parsing, volume-ratio adjustment, financial quality, growth bands/ROE trend, Markdown traffic-light rendering, and default equal-weight ranking. Run the named test after each addition.

- [x] **Step 4: Add veto behaviors vertically**

Assert exact outcomes for no signal, three negative revenue periods, low CFO/NI divergence, target-price overvaluation, P99/P99 and P95/P95 and P80/P90 tiers, one-day bounce, strictest cap precedence, downgrade precedence, and rendered advice/cap/flag text. Use a five-day flow whose sum is `-40e8` and latest inflow is `2e8` for the documented bounce condition.

- [x] **Step 5: Validate and commit the pure-core batch**

```bash
cargo fmt --all -- --check
cargo test --lib pipeline::score_breakdown::tests
cargo test --lib pipeline::veto_rules::tests
cargo clippy --lib --all-features -- -D warnings
```

Expected: all PASS. Commit only the two modules plus design/plan/progress evidence.

### Task 2: Cover remaining pure pipeline modules

**Files:**
- Modify: `src/pipeline/{market_regime,price_stats,trade_type,multi_timeframe,data,result_types,extra_context,summary_notify,technical_report,reporting}.rs`
- Modify: `src/pipeline/section_utils.rs`

- [ ] Add one characterization test per existing interface, then boundary cases for every match arm and optional field. Expected literals include exact regime labels, price statistics, trade-type labels, timeframe agreement, missing-section omission, and rendered headings.
- [ ] If a module only exposes an `AnalysisPipeline` method, instantiate the existing pipeline with test-safe dependencies or place tests beside private methods; do not make helpers public solely for coverage.
- [ ] Run each named module test immediately, then `cargo test --lib pipeline::` and strict library Clippy.
- [ ] Regenerate coverage and record per-file percentages. Continue until the pure batch is at least 95% per file or any remaining platform-only defensive branch is explicitly documented.

### Task 3: Cover strict data-provider contracts

**Files:**
- Modify: `src/data_provider/{money_flow,financials,valuation_history,consensus,industry,chip_distribution,gtimg_provider,eastmoney_provider,baostock_provider,announcement,rustdx_provider,intraday_kline,service,mod}.rs`
- Test: existing provider tests under `tests/`

- [ ] For each parser, add a valid `TEST_CODE_` protocol fixture with independent expected domain values.
- [ ] Add one malformed-present-field, missing-required-field, non-finite/non-positive-price, and relevant stale/time-continuity case. Assert explicit `Err`, never empty or zero-filled success.
- [ ] Cover request code/market normalization without network access.
- [ ] Where transport creation blocks parser tests, extract a private `parse_*(&str, observed_at) -> Result<OwnedDomainValue>` seam; public async fetchers remain responsible for status/body/freshness checks.
- [ ] Run focused provider tests after each slice, then all data-provider tests and strict Clippy before committing.

### Task 4: Cover pipeline orchestration

**Files:**
- Modify: `src/pipeline/{position_tracker,backtest_runner,analyze,mod}.rs`
- Modify: `src/pipeline/chain_analysis/{mod,fetchers}.rs`
- Test: `tests/position_tracker_tests.rs`

- [ ] Expand position tests for open/closed/new states, quantity/cost/return calculations, test/live rejection, missing quote, bad price, and persistence failure with isolated `TEST_CODE_` records.
- [ ] Cover backtest no-data, insufficient history, valid signal, sell/close, costs, as-of factor snapshots, and persistence errors through the existing `AnalysisPipeline` interface.
- [ ] Extract internal adapters for chain transport/clock only when behavior genuinely varies; validate strict parsing, registered dedup/sort/limit semantics, missing source, malformed source, and incomplete quote batches.
- [ ] Drive `analyze` through the pipeline interface with validated K-lines and explicit unavailable optional sources. Assert result fields, veto/ranking application, and failure propagation; never register a production mock source.
- [ ] Run `cargo test --lib pipeline::`, affected integration tests, and strict Clippy after every vertical slice.

### Task 3A: Execute public provider protocol tracer bullets

**Files:**
- Modify: `src/data_provider/baostock_provider.rs`
- Modify: `src/data_provider/sina_provider.rs`
- Modify: `src/data_provider/sina_news_provider.rs`

- [x] Add BaoStock tests beside the public functions. Build non-compressed and zlib frames with the documented 21-byte header, then assert version/type/body/CRC fields. Exercise `read_tcp_response` through `tokio::io::duplex` for chunked marker completion, EOF, timeout and the 1 MiB limit.
- [x] Add a valid two-day CSV/CDATA batch whose independent expected values include `close=10.1`, `amount=101000.0`, `pct_chg=1.0`, `AdjustType::Qfq`; then reject missing headers/fields, bad date/number/OHLC, duplicate/gapped dates and an adjacent close jump above 20%.
- [x] Add Sina hq tests using exactly 32 locally constructed fields and assert the UTC conversion of `2026-07-16 15:00:00` from Shanghai time plus price/volume/amount values. Reject missing quotes, short rows, bad/negative/zero prices and invalid source time. Characterize empty K-line arrays as empty and every present row as unavailable because the source protocol lacks real `amount`.
- [x] Add Sina news tests with fixed epoch `1700000000`, covering financial/stock source labels, media/oid/docid/default source names, optional intro, stable content hash, UTF-8 and GBK decode. Reject invalid JSON/data/category/code, missing URL/title/integer time, pre-2000 and more-than-five-minute future times.
- [x] Run `cargo test --lib data_provider::baostock_provider::inline_tests`, `cargo test --lib data_provider::sina_provider`, and `cargo test --lib data_provider::sina_news_provider`; each must pass without network access. Then regenerate focused coverage for the three files and run strict library Clippy.
- [x] Commit the tests and exact coverage evidence independently before extracting any Tencent/Eastmoney private parser seam.

### Task 5: Close remaining registered core deficits

**Files:**
- Modify tests beside uncovered code under `src/{database,trading,decision,risk,event}/`

- [ ] Regenerate coverage and sort registered core files by missed lines using `tools/coverage/check_thresholds.py` prefixes.
- [ ] From the current baseline, prioritize `trading/mod.rs`, `database/concepts.rs`, `decision/intraday_monitor.rs`, `database/mod.rs`, and `database/kline.rs`, then continue largest-first.
- [ ] Database tests use isolated files or unique `TEST_CODE_` rows and validate audit/source/time fields through repository interfaces.
- [ ] Trading/risk tests cover cash, lot size, daily limits, idempotency, secondary confirmation, environment isolation, and fail-closed missing quote/account paths.
- [ ] Event tests preserve immutable audit/hash-chain behavior and explicit sink outcomes.
- [ ] Repeat until `python3 tools/coverage/check_thresholds.py target/coverage/coverage.json --global-min 0 --core-min 95` exits 0.

### Task 3B: Close Tencent/Eastmoney K-line batch validation

**Files:**
- Modify: `src/data_provider/gtimg_provider.rs`
- Modify: `src/data_provider/eastmoney_provider.rs`
- Modify: `docs/business_rules.md`

- [x] Add RED local JSON tests proving both parsers reject empty batches, invalid OHLC/volume/amount/pct, duplicate/gapped dates and adjacent close changes above 20%; Tencent also covers qfqday/day and every typed field error.
- [x] Route each completed parsed vector through `validate_kline_series_strict`; Tencent computes pct from ascending real closes before the shared call. Do not add network fixtures or fallback values.
- [x] Assert complete two-day batches return newest-first, QFQ, exact amount and independently expected pct values.
- [x] Run both provider test modules, full data-provider/library tests, focused coverage, fmt, strict all-target Clippy and compliance; commit independently.

### Task 4A: Restore v16.x pushed_stocks and cover local core execution

**Files:**
- Modify: `src/database/mod.rs`
- Modify: `src/signal/push_recorder.rs`
- Modify tests: `src/decision/intraday_monitor.rs`, `src/database/{concepts,kline}.rs`, `src/trading/paper_engine.rs`
- Modify: `docs/business_rules.md`

- [x] Add a RED fresh-database contract proving the v16.x `pushed_stocks` table and its three registered indexes are absent, then add the exact idempotent DDL from v16.3 under BR-126.
- [x] Exercise `push_recorder` → intraday/evening consumption with unique `TEST_CODE_` rows, test-only fresh quote/account evidence, explicit bad-candidate non-consumption, successful audit fields and same-day re-entry prevention.
- [x] Cover concept/chain/event/board repository validation and lifecycle through real isolated SQLite; cover K-line upsert/range/context/result lifecycle without touching the ignored live account database.
- [x] Cover paper-engine open-position/advice decisions and failure boundaries; preserve real quote/account requirements and do not add production mocks.
- [x] Run focused tests, full instrumented library suite, fmt, strict all-target Clippy and compliance; commit independently.

### Task 4B: Cover backtest execution and chain-analysis prompt/protocol boundaries

- [x] Extract deterministic Bollinger/RSI execution after real benchmark acquisition; cover validated `TEST_CODE_` histories, OOS, walk-forward, report generation and insufficient batches.
- [x] Make required backtest trade/NAV audit writes fail closed and cover successful isolated files plus an unwritable target.
- [x] Execute deep/simple/overview chain prompts with present and missing optional evidence against an explicitly unavailable analyzer; assert no generated fallback.
- [x] Split push2 status/body validation from transport and cover status, JSON, missing-data and complete-response contracts without sockets or external network.
- [x] Run focused tests and full instrumented library coverage; record 1,507 pass / 10 ignored / 0 failed, global 67.04% and registered core 83.06%.

### Task 4C: Validate RustDX, announcements, and deterministic decision boundaries

- [x] Route converted RustDX daily bars through BR-092 complete-batch validation and cover empty, field, continuity, jump, ordering, percentage, adjustment and pre-transport failure paths.
- [x] Split announcement response validation, high-risk detail selection and result assembly from real HTTP transport; cover complete local provider protocol rows and every missing/bad-field class without fallback data.
- [x] Cover exclusion, leader, rotation, sector score and capital verification public states; reject zero-window and invalid RS endpoints without changing registered thresholds.
- [x] Run focused suites and full instrumented library coverage; record 1,521 pass / 10 ignored / 0 failed, global 67.77% and registered core 84.64%.

### Task 4D: Close auxiliary database audit and LHB evidence gaps

- [x] Register BR-127 before changing account-mode or LHB behavior; require exact affected-row audit marking and complete LHB API/cache batches.
- [x] Replace LHB missing-field/zero fallback parsing with strict local protocol parsing, trading-day/domain validation, batch duplicate rejection and propagated cache read/write failures.
- [x] Cover account-mode audit, LHB persistence/query/cleanup, position available-share composition, Stock/Trade repositories and DataFetchService cache-hit/expiry paths with isolated `TEST_CODE_` facts.
- [x] Run focused tests, strict all-target Clippy, full library regression and instrumented coverage; record 1,529 pass / 10 ignored / 0 failed, global 68.62% and registered core 85.86%.

### Task 6: Raise repository-wide coverage to 80%

**Files:**
- Modify tests beside the largest remaining non-core gaps, initially `src/bin/monitor/{push_templates,main,notify}.rs`, `src/search_service/service.rs`, `src/opportunity/mod.rs`, `src/strategy/rsi/standard.rs`, `src/analyzer/analyze.rs`, `src/agent/multi_agent/slices.rs`, and `src/notification/report.rs`.

- [ ] Cover pure renderers and scoring/state transitions first with exact output/result assertions.
- [ ] Move monitor orchestration bodies behind small internal functions accepting parsed configuration, time, repository, and notification dependencies; `main` remains environment/argument parsing plus exit-code mapping.
- [ ] Add process tests for exit-code/failure boundaries with webhook/broker credentials removed.
- [ ] Recompute global coverage after every commit and continue by largest safe missed-line cluster until `python3 tools/coverage/check_thresholds.py target/coverage/coverage.json --core-min 95 --global-min 80` exits 0.

### Task 7: Persist truthful nullable same-day account evidence

**Files:**
- Modify: owning migration/model/repository files under `src/database/`
- Modify: `docs/business_rules.md` (BR-103)
- Test: owning database module tests

- [x] Add a RED repository test proving a same-day account snapshot stores `daily_pnl=None` with total assets, market value, cash, source, source timestamp, observed-at timestamp, and nullable account mode.
- [x] Add RED validation tests rejecting non-finite/impossible negative values, inconsistent totals beyond documented tolerance, stale source timestamps, and missing provenance.
- [x] Implement the smallest idempotent migration and repository interface; keep legacy ledger reads compatible and never coerce `None` to zero.
- [x] Back up the ignored local database, migrate locally, persist only user-attested fields, and revalidate integrity/totals. Private values/evidence remain outside Git.
- [x] Update BR-103, design, handoff, and rollback commands; run focused tests, database tests, and compliance.

### Task 8: Full release validation, review, and merge

**Files:**
- Verify all changed source/tests/docs and PR #2 evidence.

- [ ] Run `cargo fmt --all -- --check` and `git diff --check`.
- [ ] Run `cargo check --all-targets --all-features`.
- [ ] Run `cargo clippy --all-targets --all-features -- -D warnings`.
- [ ] Run `cargo test --all-targets --all-features`.
- [ ] Run `bash tools/compliance/check.sh`; on freshness failure run the mandated backfill and recheck.
- [ ] Run `cargo build --release --bin monitor` plus a test-mode dry-run with delivery credentials removed.
- [ ] Generate llvm-cov JSON and require global >=80% plus registered core >=95%.
- [ ] Validate the local real-account/account-audit trail without printing or uploading private values.
- [ ] Run independent Standards, Spec, and Audit reviews against a fixed SHA; resolve every Critical/Important finding and repeat.
- [ ] Complete the PR checklist, mark PR #2 Ready, merge through GitHub into `master`, update local `master`, and verify the merge commit plus clean worktree.

### Task 19: Cover deterministic data, timeframe, quote, and report boundaries

- [x] Route the real post-fetch pipeline batch through one strict persistence/freshness boundary and cover empty, stale, and valid isolated batches.
- [x] Separate real 60/15-minute acquisition from deterministic multi-timeframe resolution and exercise insufficient and complete evidence.
- [x] Route Tencent name/realtime HTTP bodies and Eastmoney minute bodies through private strict parsers; cover protocol, field, price, time, continuity, and runtime failures with `TEST_CODE_` fixtures only.
- [x] Exercise daily/WeChat report construction with complete nullable domain evidence and exact output assertions; no network or production fallback is introduced.
- [x] Run focused tests, formatter, strict library Clippy, and full instrumented library coverage: 1,561 passed / 10 ignored / 0 failed; library global 51,018/72,142 = 70.72%, registered core 26,010/29,655 = 87.71%.

### Task 20: Cover deterministic multi-factor backtest execution

- [x] Keep production ranking and real daily-history acquisition in the existing wrapper; pass only the complete history batch and nullable real benchmark into an internal resolved executor.
- [x] Exercise explicit insufficient-stock/short-history failures plus a complete 120-day, three-stock base/OOS/walk-forward report with `TEST_CODE_` identities.
- [x] Run the exact focused test, formatter, strict library Clippy, and focused instrumented coverage; `backtest_runner.rs` reaches 915/1,111 = 82.36%.

### Task 21: Cover monitor protocol, rendering, and local-delivery boundaries

- [x] Register BR-131 and correct the North Exchange `92` prefix to the documented 30% price-limit profile while preserving ST precedence.
- [x] Exercise monitor delivery type/target resolution, token parsing and caches, file permissions, local peer/log parsing, and missing-recipient failure behavior without sending a message.
- [x] Exercise ranking, fund-flow, turnover, announcement-summary, source-wrapper, and UTF-8 truncation renderers with complete and explicitly missing facts.
- [x] Run all 301 monitor tests, strict monitor Clippy, formatter, and focused coverage; `notify.rs` reaches 985/1,811 = 54.39%, `push_templates.rs` 4,643/8,660 = 53.61%, and `market_data.rs` 85/582 = 14.60%.

### Task 22: Exercise core real-source failures and parsed pipeline boundaries

- [x] Execute existing provider request construction against a test-only unreachable proxy with strict short timeouts; require explicit all-source errors and no empty/default downgrade.
- [x] Complete deterministic parser/selector/error tests for the largest remaining `data_provider/`, `pipeline/`, `decision/`, `risk/`, and database gaps using only `TEST_CODE_` facts.
- [ ] Run focused suites, full strict Clippy/compliance, then regenerate CI-equivalent workspace coverage and continue until registered core reaches 95%.

### Task 23: Execute chain analysis from already validated facts

- [x] Keep every real acquisition call in `run_chain_analysis`, then move cluster persistence, board resolution and final report orchestration behind private resolved helpers.
- [x] Cover successful `TEST_CODE_` cluster lifecycle, board matching/candidate attachment and unavailable-model report output with an isolated database only.
- [x] Re-run chain tests and coverage; reject any implementation that creates a source fake, fallback fact, external request, notification or order.

### Task 24: Exercise real-provider HTTP outcome decisions

- [x] Keep Eastmoney/Tencent real hosts, request construction and retry ordering unchanged; route completed status/body facts through private deterministic outcome helpers.
- [x] Cover 2xx complete data, empty/bad bodies, 4xx terminal failures, 5xx retry failures and strict parser rejection without sockets or external requests.
- [x] Run both provider suites, formatter, strict Clippy and instrumented core coverage; reject any fake success payload in a production transport path.

### Task 25: Exercise RustDX post-fetch assembly

- [x] Keep real TCP pagination and BR-092 parsing unchanged; move only already validated K-line decoration and nullable real Tencent enrichment to a private resolved helper.
- [x] Cover complete quote, absent quote and explicit quote-source failure with local `TEST_CODE_` histories; settled close must never be overwritten.
- [x] Run RustDX tests, formatter, strict Clippy and core coverage.

### Task 26: Exercise resolved chain-search evidence

- [x] Keep real search/LHB acquisition unchanged; extract only generated-query protocol parsing, result dedup/truncation/rendering and strict LHB mapping.
- [x] Cover query limits, sentence rejection, duplicate titles, absent dates, result caps and invalid/duplicate LHB records with local facts.
- [x] Run chain fetcher tests, formatter, strict Clippy and core coverage.

### Task 27: Exercise money-flow and intraday HTTP responses

- [x] Keep the three real push2his hosts and request order unchanged; route completed status/body facts through strict private parsers.
- [x] Cover non-2xx, read failure, empty/HTML, bad JSON, missing arrays, complete flow rows and complete intraday shape locally.
- [x] Run money-flow tests, formatter, strict Clippy and core coverage.

### Task 28: Exercise announcement list and detail HTTP responses

- [x] Keep the real Eastmoney announcement endpoints, request headers and high-risk detail selection unchanged; route completed status/body facts through strict private parsers.
- [x] Cover non-2xx, read failure, empty/HTML/bad JSON, missing list/content and complete list/detail responses locally without sockets.
- [x] Run announcement tests, formatter, strict Clippy and core coverage; reject any title/summary fallback for required detail content.

### Task 29: Exercise industry HTTP, pagination, and resolved benchmark decisions

- [x] Keep the three real Eastmoney hosts and endpoint fields unchanged; require 2xx plus a readable complete JSON body before parsing.
- [x] Cover map-page termination/conflict, missing board mapping, complete constituent resolution and all response failures with local facts.
- [x] Run industry tests, formatter, strict Clippy and core coverage; never cache a partial map or infer a default industry.

### Task 30: Exercise small core state and cache commit boundaries

- [x] Cover same-day exclusion cache reuse, resolved component mapping, empty scans, leader prompt/result application and unavailable analysis without external calls.
- [x] Route successful provider results through validation-before-cache helpers; cover complete/empty/error money-flow and intraday states plus unreachable financial transport.
- [x] Exercise present/absent mainline parsing and rendering from local `TEST_CODE_` protocol rows; run focused tests, formatter, strict Clippy and core coverage.

### Task 31: Exercise successful HTTP transport over a test-only loopback

- [x] Add a `cfg(test)` loopback responder that serves exact status/body sequences on `127.0.0.1`, times out safely, and is absent from production builds.
- [x] Route existing private host/base seams for chain pages, announcement list/detail and industry three-step acquisition to the loopback while preserving every production endpoint.
- [x] Assert complete pagination/batch results and request completion with `TEST_CODE_` identities; run focused suites, formatter, strict Clippy and core coverage.

### Task 32: Exercise Eastmoney, Tencent, and Sina transport over loopback

- [x] Route the existing Eastmoney host retry loop through a private host/base seam while preserving production hosts, attempt order, headers, delay and terminal/retry decisions.
- [x] Route Tencent K-line/name/realtime and Sina K-line/HQ requests through private URL/base seams while preserving strict parsing and nullable enrichment behavior.
- [x] Cover complete, retry and terminal transport outcomes with the `cfg(test)` loopback responder and `TEST_CODE_` identities; 1,647 library tests pass with 10 explicit live integrations ignored, global coverage is 59,661/75,771 = 78.74%, and registered core is 29,225/31,793 = 91.92%.

### Task 33: Exercise BaoStock TCP sessions and repair async test isolation

- [x] Replace the synchronous pipeline test mutex held across `await` with the repository's async-compatible serialized test domain and preserve environment restoration.
- [x] Execute BaoStock login, session reuse, K-line request and strict response assembly against a test-only loopback TCP listener without changing its production endpoint or protocol.
- [x] Cover complete and explicit protocol failures, then run focused tests, formatter, all-target strict Clippy and instrumented library coverage; 1,649 tests pass with 10 live integrations ignored, global coverage is 59,788/75,852 = 78.82%, and registered core is 29,352/31,874 = 92.09%.

### Task 34: Exercise auxiliary HTTP providers over loopback

- [x] Route Sina news top/stock/range pages, Yahoo blocking quotes and Eastmoney minute K-line through private base seams while preserving production URLs and strict parsing.
- [x] Route North-flow async/blocking, IPO-date and valuation-history requests through private URL/base seams with explicit transport/protocol failures.
- [x] Cover complete and failing real-client transport locally with `TEST_CODE_` identities; focused loopback tests pass 16/16, all-target strict Clippy and compliance pass, and full library coverage reaches 60,103/76,066 = 79.01% globally and 29,667/32,088 = 92.46% across the registered core.

### Task 35: Exercise remaining core orchestration and real-SQL contracts

- [x] Reuse the existing test-only fetched-data slot behind one private backtest adapter and cover benchmark paging, ranked history acquisition, empty/source failures and pre-output wrapper failures without changing production acquisition.
- [x] Exercise chain position diagnosis and position-tracker decision failures with isolated `TEST_CODE_` SQLite evidence; no live quote, external analysis, notification or real-account read is permitted.
- [x] Complete database root query/validation branches with parameter-bound real-SQL tests; formatter, all-target strict Clippy, compliance and 1,660-test instrumented library regression pass, raising global coverage to 60,656/76,393 = 79.40% and registered core to 30,188/32,415 = 93.13%.

### Task 36: Deepen remaining transport, commit, and deterministic state modules

- [x] Separate RustDX's true external page adapter from strict pagination/whole-batch resolution; cover short/empty pages, multiple pages, source error and panic without a live TCP dependency.
- [x] Route the three backtest result types through a private filesystem commit seam and cover report, chart and mandatory audit outcomes in isolated directories.
- [x] Exercise account-snapshot and event-envelope failure matrices plus Tencent/Eastmoney public provider interfaces with `TEST_CODE_`, pure state validation, temporary files and loopback transport.
- [x] Run instrumented library regression: 1,667 pass / 10 explicit live integrations ignored / 0 failed; global coverage reaches 61,494/76,866 = 80.00%, while registered core reaches 30,792/32,888 = 93.63%, so Gate D remains open without threshold rounding or reduction.

### Task 37: Close the remaining core orchestration gap

- [x] Execute `AnalysisPipeline::run` from isolated validated facts through result collection and all three backtest branches, with notification/model/network disabled and reports committed only to a temporary directory.
- [x] Extract only resolved industry-chain search/artifact decisions needed to cover query generation, result dedup, pagination and complete cache resolution; all production adapters remain real and fail explicitly.
- [x] Convert the two ignored Eastmoney live tests to strict loopback protocol tests and cover deterministic summary/mainline states without deleting denominator lines.
- [ ] Task 37b: execute summary filesystem/notification commit through an explicit production `reports/` adapter and isolated test directory.
- [ ] Task 37b: resolve BaoStock post-close success/empty/failure before the unchanged real fallback chain.
- [ ] Task 37b: cover intraday/evening/exclusion state transitions before quote/account/order boundaries.
- [ ] Task 37b: exercise BR-086 audit-chain length/link/hash/success behavior in isolated real SQLite.
- [ ] Task 37b: execute industry-chain deep/simple/overview model commits through a private `Live/Resolved` evidence adapter and local protocol server; production remains permanently wired to `Live`.
- [ ] Task 37c: replace remaining core ignored/environment-sensitive integration tests with protocol-equivalent loopback/resolved inputs and execute the stock-name/news orchestration with search disabled and a `TEST_CODE_` cache hit.
- [ ] Task 37d: mount behavior-focused private regression modules from repository-level test sources; cover only existing parsing, validation, state-transition and explicit-failure branches, with no threshold/exclusion changes and no external network.
- [ ] Regenerate instrumented coverage and continue until `check_thresholds.py --global-min 80 --core-min 95` passes, followed by all Gate B/C checks.

### Task 38: Execute an isolated monitor process tracer bullet

- [x] Make `monitor --test --e2e` return nonzero when seed or review fails and require a final completion marker in the process test.
- [x] Seed only an isolated TEST_CODE ledger for the latest completed trading day; never write `real_account_snapshot` or touch the production database.
- [x] Add an `IsolatedAll` dispatcher scope that records external real-market/announcement/overnight templates as unexercised instead of depending on network freshness.
- [x] Run the exact workspace coverage command: global 74,007/99,288 = 74.54%; core 32,604/34,195 = 95.35%. Core passes and global remains open.

### Task 39: Close the remaining workspace-global gap

- [ ] Exercise existing tool/CLI binaries through isolated process tests, covering argument validation, local success commits and explicit source failures.
- [ ] Cover the largest non-core public domain boundaries in opportunity, search, notification and market/review using compact table-driven tests and loopback protocol fixtures only.
- [ ] If monitor orchestration still dominates, extract only resolved-input seams immediately behind unchanged real adapters and assert delivery/audit results.
- [ ] Regenerate CI-equivalent workspace coverage until global >=80% and core >=95%, then run all Gate B/C/D release checks.
