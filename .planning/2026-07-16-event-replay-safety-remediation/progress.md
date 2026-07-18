# Progress: Event Replay Safety Remediation

## 2026-07-16

- Read mandatory rules, v17.3 spec/plan, current CLI/replay code, monitor integration, and relevant skills.
- Reproduced the baseline fact that focused CLI and replay tests pass despite the reviewed defects.
- Chose structured replay summary design and preserved current module boundaries.
- Registered BR-043 and wrote design/implementation plan.
- Next: start CLI vertical TDD slices.
- CLI composition test failed under the early-return implementation, then passed after known monitor flags became skippable.
- Documented replay-rate equals form failed as unrecognized, then passed after adding the value-bearing prefix parser.
- Explicit `limit=0` failed as `InvalidLimit(0)`, then passed after rejecting only negative values.
- Full CLI module result: 15/15 PASS.
- Invalid/non-string replay text test failed by publishing one envelope, then passed after explicit text validation; blank text is also rejected.
- Rate test failed with zero elapsed delay, then passed after sleeping only between force publish attempts.
- Repeated replay ID test reproduced the duplicate ID, then passed with process/time/atomic-sequence IDs.
- No-subscriber and shutdown cases now increment `failed`; `ReplaySummary` separates scan and dispatch outcomes.
- Monitor replay handling no longer uses `unwrap_or(0)` and exits nonzero for replay errors or failed rows/publishes.
- A downstream test showed `HistoryQuery` still truncated explicit `limit=0` at 100; zero now skips truncation and returns all 101 test rows.
- Replay tests initially interfered through a shared temp directory; unique test directory suffixes removed the race.
- Full event test run exposed the same collision in history helpers plus a fixed-noon timestamp that can fall in the future; both test-only hazards were removed.
- `cargo build --bin monitor`: PASS (124 existing warnings).
- `cargo test --lib`: PASS, 1253 passed / 7 ignored.
- Mandatory freshness backfill initially returned 33/33 empty providers in the network sandbox; the approved non-sandbox retry succeeded 33/33 and advanced `stock_daily` to 2026-07-16 (3008 rows).
- `bash tools/compliance/check.sh`: ALL CHECKS PASSED after backfill.
- Coverage: `event/cli.rs` 91.50%, `event/history.rs` 88.61%, `event/replay.rs` 97.83%; repository total 51.11%, so Gate D remains blocked globally.
- Global `cargo fmt --check` reports about 17,510 pre-existing diff lines; strict clippy reports 342 diagnostics; all-target test compilation fails at `src/bin/v14_e2e.rs:285` because an old dispatcher call omits argument 3.
- First two-axis review found force mode still had no real subscriber, unbounded history still printed only 20 rows, zero-result summary was partial, pacing did not prove the first attempt was immediate, and ID generation silently defaulted a failed clock read.
- Added awaited `ReplayPublisher`; monitor force mode now calls the real notification sink and counts success only after sink acceptance. EventBus remains a publisher adapter for isolated bus failure tests.
- History formatting moved to `format_history_lines`, and the 101-row test proves all unbounded results reach terminal formatting.
- Replay IDs now use pid + process-wide atomic sequence without a fallible clock; pacing test observes the first event before the 200ms interval and blocks the second until the interval elapses.
- Post-review verification: replay 11/11, event 74/74, lib 1254/1254, monitor build PASS, compliance PASS.
- Refreshed coverage: CLI 91.50%, history 88.80%, replay 98.00%, repository 51.14%.
- Final Standards review found that a corrupt existing replay-audit tail could silently restart at `GENESIS`; the file sink now validates every existing record hash and parent link and rejects corruption before notification.
- Added corrupt-audit regression coverage; production publisher suite is 5/5 PASS. Final event suite is 74/74, library suite 1254/1254 with 7 ignored, monitor build PASS, and compliance PASS.
- Final Spec and Standards re-reviews both returned PASS; only scoped Git tracking/commit remains.
- Scoped remediation code and evidence were committed without staging the user's unrelated dirty files; release readiness remains blocked only by recorded repository-wide gates.

## 2026-07-17 — Repository-wide continuation

- User authorized continuing through the remaining historical problems without another confirmation gate.
- Re-read AGENTS.md, ENGINEERING_RULES_V2.md, and CLAUDE.md; `.github/copilot-instructions.md` and `CONTEXT.md` are missing.
- Activated persistent planning, systematic root-cause diagnosis, TDD, implementation, and final two-axis review workflows.
- Added Phases 6–10 for full traceability inventory, compilation/test repair, lint/format cleanup, coverage/spec closure, and final gates.
- Counted 176 documentation files and 72 with completion/PASS-style claims for later evidence classification.
- Reproduced the all-target compile blocker exactly: `v14_e2e.rs:285` omits the third `Dispatcher::dispatch` argument; nearby calls demonstrate the migrated three-argument form.
- Repaired the stale `v14_e2e` call with explicit `None`; its test binary now compiles.
- Re-ran all-target compilation and exposed the next blocker: a non-exhaustive `EventType` match in `tests/rule_filter_benchmark.rs` after recent enum expansion.
- Added explicit benchmark categories for all four canonical new `EventType` variants; the integration-test target now compiles.
- All-target/all-feature compilation now passes across every listed binary, integration test, and benchmark.
- Full all-target runtime found one deterministic-at-current-time failure: the auction-agent test assumes wall-clock 16:45 and fails when the suite runs during the actual auction window.
- Added a session input seam around the auction Agent and made the time-gate test deterministic; the exact failing test now passes.

## Test Log

| Command | Result |
|---|---|
| `cargo test --lib event::cli::tests -- --nocapture` | PASS baseline; missing reviewed combinations |
| `cargo test --lib event::replay::tests -- --nocapture` | 4/4 PASS baseline; missing failure/rate/repeat-run cases |
| `cargo test --lib event::replay::tests -- --nocapture` | 9/9 PASS after replay summary/rate/ID fixes (before blank-text test addition) |
| `cargo test --lib event::history::tests::zero_limit_returns_and_formats_all_matching_history_entries -- --exact --nocapture` | RED 100/101, then PASS 101/101 |
| `cargo test --lib event:: -- --nocapture` | PASS 73/73 after test-isolation repair |
| `cargo build --bin monitor` | PASS |
| `cargo test --lib` | PASS 1253/1253; 7 ignored |
| `bash tools/compliance/check.sh` | PASS after §2.4 backfill |
| `cargo llvm-cov --lib --summary-only` | PASS; total lines 51.11%, replay lines 97.83% |
# 2026-07-17 continuation

- 修复 `src/bin/v14_e2e.rs` 过期的三参数 `Dispatcher::dispatch` 调用。
- 修复 `tests/rule_filter_benchmark.rs` 对新增 `EventType` 变体的非穷尽匹配。
- 修复 `src/opportunity/auction_agent.rs` 单测依赖真实 09:15–09:25 时钟导致的随机失败；精确回归已通过。
- `cargo test --all-targets --all-features` 正在继续，已通过 lib 的 1261 项及 monitor 的 263 项测试，等待最终退出状态。
- 全量测试最终退出 101：`launch_gate_tests` 9/10，通过根因定位确认 Gray 低胜率回退分支误返回当前状态。
- 在编码前已将灰度回退语义登记为 `docs/business_rules.md` BR-044。
- 修复 LaunchGate 低胜率分支并增加 50% 边界测试；目标测试 11/11 PASS。
- 首次全目标复跑在 lib 1252/1261 后发现 2 个内部旧断言仍期望错误的 `Gray`；已按 BR-044 修正。
- 一次无上下文替换误改了同一 E2E 测试的 Shadow→Gray 正向断言；通过行号复现后已精确恢复正向断言，并只修改低胜率 Gray→Shadow 断言。
- `cargo test --lib opportunity::launch_gate::tests --quiet` 7/7 PASS；第三次全目标回归正在执行，stderr 告警暂存 `/tmp/stock_analysis_alltargets.err` 供后续 Clippy 分类。
- 第三次全目标回归：1250 passed / 4 failed / 7 ignored；失败均为默认单测依赖外部 RustDX 或 macOS HTTP client 系统配置，已完成隔离设计，准备实施。
- 外部测试隔离已实施并定向通过：RustDX 1/1 deterministic PASS、3 live probes ignored；DeepSeek 2/2 PASS。
- 第四次 `cargo test --all-targets --all-features --quiet` 正在执行；告警仍单独收集，等待所有目标最终退出状态。
- 第四次回归已通过 lib 1252 项和 monitor 263 项，随后在 Baostock timeout 测试因 loopback bind `EPERM` 失败；已改为相同读取逻辑上的内存流超时测试。
- Baostock timeout 定向测试 PASS（约 20ms），确认无端口权限依赖且错误路径仍显式返回 timeout。
- 第五次全目标回归正在运行；编译阶段无新增阻塞，等待所有测试目标结束。
- 第五次回归在 lib 1245 passed 后出现 7 个共享 SQLite 生命周期失败；已定位为 OnceCell 首次路径与测试删库竞态，进入统一隔离修复。
- 已为 `cfg(test)` 库单测增加进程唯一临时 DB 路径；非测试构建仍严格使用调用方路径，未改变生产数据库选择。
- 数据库隔离后的完整 lib 并行测试 PASS：1252 passed / 0 failed / 10 ignored。
- 第六次全目标回归正在运行；重点验证独立集成测试进程仍按其显式数据库路径工作。
- 第六次回归 lib 全绿、monitor 262/263；唯一失败定位为 `V10_DRY_RUN_PUSH` 并行环境竞态，进入测试构建 fail-safe 修复。
- 已实现 monitor `cfg(test)` 传输层强制 dry-run，生产 env 语义不变；等待 monitor 全套并行复验。
- `cargo test --bin monitor --quiet` 263/263 PASS；确认环境竞态不再导致真实 sink 路径。
- 第七次全目标回归已通过 lib/monitor/Baostock 本地测试，后在默认联网的盘后 5-provider 集成测试失败；已统一改为显式 ignored live tests。
- `cargo test --test fallback_post_close_test --quiet` PASS：0 failed / 2 live integrations ignored。
- 第八次回归定位到 Sina/fallback 两个默认联网用例；已标为显式 live integrations，默认门禁不再依赖 DNS。
- 已复查其余明确网络集成测试的 ignore 分类，准备再次跑全目标最终状态。
- 全目标全特性回归最终 PASS（exit 0）；Phase 7 编译/运行回归阻塞清零，转入严格 Clippy/format 阶段。
- Strict Clippy 基线已采集：lib 292 / lib-test 341 errors；Phase 8 从 correctness diagnostics 开始。
- Correctness triage found one false-success notification branch, one invalid lazy regex, one poisoned-lock handling lint, redundant breakout branches, and a separate production `news_ai` mock gap requiring design-level remediation.
- Verified official Pushover message contract and added it to Gate A design before replacing the fake Slack reuse.
- Correctness batch 1 complete and lib tests PASS (1254/1254 active); next is production attribution mock remediation.
### 2026-07-17 — Attribution historical-gap diagnosis

- Confirmed `attribute_event` is currently dead outside unit tests.
- Confirmed existing NewsAI public paths are LLM-backed and cannot satisfy BR-019's no-LLM/latency gate as-is.
- Next: trace the production `AlertEvent` creation/dispatch path, document a Gate-A addendum, then implement the smallest auditable rule-only vertical slice test-first.
- Production trace found: news loop -> state machine -> `push(&AlertEvent)`; current loop and `push()` both archive, and attribution is absent from both points.
- Found conflicting BR-019 definitions across the two registries; remediation will allocate a new canonical rule ID and cross-reference the legacy v10 label.
- BR-045 design and canonical registration completed before implementation.
- Attribution/audit focused assertions pass, but the focused monitor suite revealed 9 pre-existing DB-initialization-order panics in `EntityLinker::new`; returned to Gate B root-cause repair before production wiring.
- Portfolio DB access now fails explicitly instead of panicking; entity-linker focused suite is green (6/6).
- Production `push(AlertEvent)` now applies deterministic attribution, persists structured evidence with explicit audit-write errors, then delivers. The news loop no longer writes the accepted event twice.
- All targets/features compile (`cargo test --all-targets --all-features --no-run`: exit 0).
- Focused monitor suite is green after production wiring and DB fail-closed repair: 211 passed, 0 failed.
- Phase 8 continues: recompute strict Clippy from the new baseline, triage semantic diagnostics before scoped mechanical cleanup.
- Strict Clippy remains red: library compilation stops after 286 errors. Correctness diagnostics already triaged/fixed; next pass uses Cargo's machine-applicable fixes only, followed by full diff review and focused tests before handling non-auto diagnostics manually.
- Machine-applicable library cleanup completed after one safe type-inference prerequisite: 137 library diagnostics remain. Running whitespace and full library regression before manual interface/dead-code work.
- Removed three whitespace defects introduced by machine fixes and relocated three test-only imports after the initial lib-test compile failure. Re-running the library suite from the corrected tree.
- Post-fix validation is green: `git diff --check` exit 0; `cargo test --lib --quiet` 1259 passed, 0 failed, 10 ignored. Manual Clippy triage continues from 137 remaining library diagnostics.
- Captured all 137 remaining library diagnostics in `/tmp/stock_analysis_clippy_lib.json`; applying localized correctness/hygiene fixes before narrow rationale-based lint boundaries for legacy interfaces.
- Removed the first localized warning batch: redundant market branches, unused counters/imports, deprecated helper self-use, and overwritten initial assignments. Caller search confirmed three news-outcome helpers are module-private and several “dead” helpers are test-only or intentionally dormant, so visibility/cfg decisions will reflect actual use.
- Tightened visibility/cfg for module-private and test-only helpers, exposed two implemented calculation APIs that were accidentally hidden, removed no-op emoji replacements, combined identical source branches, and converted SimHash bit mutation to an indexed-safe iterator.
- Replaced 19 comparator closures with equivalent key-based ordering while preserving ascending/descending semantics explicitly through `Reverse`; no unstable sort substitution was used.
- Corrected one same-name-variable patch scope in `breakout::engine`; strict library Clippy now compiles and is down to 79 diagnostics: 24 dead-code, 27 broad-interface/type-complexity, and 28 localized documentation/control-flow items.
- Exported the exact 28 localized diagnostics and split them into semantics-free fixes versus two interface-sensitive cases (`FromStr` compatibility and non-`Send` analyzer ownership); semantics-free items are being resolved first, with focused regression before ownership changes.
- Localized batch completed: iterator/control-flow/clamp findings fixed, parsing moved to infallible standard `FromStr` while preserving safe defaults, and Pipeline now owns its non-thread-safe analyzer instead of wrapping it in a misleading `Arc`. Strict library diagnostics fell from 79 to 63.
- Rewrote the remaining rustdoc list/continuation warnings without changing runtime behavior. Next batch is the 14 repeated complex types, then explicit review of 13 stable wide interfaces and 24 deserialization/legacy dead fields.
- Extracted named aliases for all repeated complex types and applied only function-local, rationale-bearing allowances to 13 stable schema/protocol/audit boundaries; no crate-wide lint suppression was introduced.
- Dead-code audit converted three hidden gaps into behavior fixes: JSONL startup retention cleanup, configured Sina lid fan-out, and BR-046 quoted stock-code batch filtering with a leading-zero regression test. Redundant response fields, duplicate caches, and an obsolete outcome helper were removed.
- Strict library Clippy is down to one intentional legacy compatibility stack (5 diagnostics); each exact legacy JSON-dashboard entry is now annotated with a narrow compatibility rationale pending final recomputation.
- Strict library Clippy now passes with `-D warnings` (exit 0). Full all-target lint inventory is isolated to test/bin code: mainly dead/deprecated test helpers, doc spacing, unused locals, and mechanical sort/guard simplifications; no new library production diagnostic appeared.
- Proceeding with machine-applicable all-target fixes only (no broken-code mode), followed by diff review and full regression before manual test/bin cleanup.
- All-target machine pass completed after correcting one stale test fixture; it applied safe suggestions in 20+ test-focused files and left only non-machine diagnostics. `git diff --check` found seven whitespace-only artifacts to clean.
- Global `cargo fmt --all -- --check` remains red on a large repository-wide pre-existing formatting baseline (17k diff lines, including untouched benches/modules). This is now an explicit Gate B debt to resolve after strict lint semantics are clean, not evidence of the focused fixes passing fmt.
- Applied full repository rustfmt to satisfy the mandatory global gate; semantic validation is required because this intentionally touched the historical formatting baseline across the tree.
- Added BR-047 and changed the two-argument push entry to reject PerTicket kinds without a real code. Migrated E2E ticket templates plus production screener and holding-T flows to `push_governor_v3` with explicit codes; legacy L4 E2E now uses reserve+commit.
- Strict all-target compilation now reaches the monitor binary. Remaining diagnostics are concentrated in detached rustdoc blocks, test-only/dead template helpers, and a smaller mechanical batch; the previous cross-ticket cooldown bug is no longer left as a deprecated call-site warning.

## 2026-07-18 — BR-098/BR-099 real-data closure

- Replaced production `pushed_stocks` hard-coded scores with the eight registered v16.4 strategy implementations; invalid/future times, malformed metric JSON, missing required fields, and stale rows now fail explicitly. Focused decision/strategy tests pass.
- Replaced I-03 percent-as-price and list-position fabricated board levels with a complete Eastmoney quote batch plus daily-K board-level evidence; incomplete quotes/levels reject the batch.
- Replaced `produce_winrate_samples` empty placeholder output with verified T+1/T+3/T+5 prediction-tracker export and atomic JSONL replacement; empty or invalid samples are errors.
- Registered BR-099 and changed candidate price/change/heat to explicit optional values. Removed the compatibility branch that copied one industry-chain candidate into four fake P5 sources.
- P5 source parsing now distinguishes missing known files from unknown sources and rejects source IO/JSON/code/name corruption as a whole-source error.
- D-01 and P-03 share a real candidate batch: exact source identities, complete same-batch real-time quotes, actual stock names/prices/changes, optional main-flow heat, held/ST/market/limit filters, and deterministic sorting.
- P-03 no longer emits zero price, fixed volume ratio, or fabricated Mid news/K-line evidence. It requires real volume ratio and marks unavailable independent evidence as Missing.
- Verification: `cargo check --all-targets --all-features` PASS; candidate-panel focused tests 7/7 PASS; strict P5 parser regression 1/1 PASS.

## 2026-07-18 — Final integration evidence

- Registered BR-115 and propagated real-source failures through financials, cached money flow/intraday data, Agent tools, analysis context, v17 earnings polling and external notification response handling.
- Added RED/GREEN regressions proving all failed/empty financial sources and malformed money-flow, intraday and minute-K rows reject the entire batch.
- Removed the incomplete Sina money-flow fallback instead of filling unavailable `xl_net`/`big_net`/`main_pct` with zero; tail-30-minute change is now optional before 14:30.
- `cargo fmt --all -- --check`: PASS.
- `cargo check --all-targets --all-features`: PASS.
- `cargo clippy --all-targets --all-features -- -D warnings`: PASS.
- Independent fixed-SHA verdict: no Critical, Changes Requested. Remediation scope is WAL actual-mode validation, no retry-to-health for pooled PRAGMA failures, outbound-webhook isolation in process tests, exact BR-108 ledger evidence, narrow process-artifact ignores, and current PR evidence.
- Re-read `DatabaseManager::init`, monitor startup, process tests, design addendum, and process artifact inventory. Next action is RED process coverage before changing implementation.
- `cargo test --all-targets --all-features`: PASS, exit 0. Library ran 1,346 tests (1,336 passed / 10 ignored); monitor 293/293 passed; every remaining binary, integration, and benchmark target had zero failures.
- `bash tools/compliance/check.sh`: PASS. Freshness remains `stock_daily MAX(date)=2026-07-16`, one A-share trading day behind; fake implementation, contradiction, business-rule, and silent-fallback checks passed (documented nonblocking warnings unchanged).
- `cargo build --release --bin monitor`: PASS.
- Isolated dry-run smoke exited 2 on missing 2026-07-18 ledger NAV as designed, but also logged repeated `database is locked` errors during first-time DB initialization. Paused Gate D evidence collection to investigate systematically before proposing any fix.
- Reproduced the same startup lock errors on a second unique database path; the symptom is deterministic for clean DB initialization. Began tracing `DatabaseManager::init`, pool customization, and migration ordering.
- Traced the failure boundary to concurrent r2d2 connection customization: every newly opened connection attempts the database-wide WAL transition before migrations, so some acquisitions log `database is locked` and retry. No fix applied yet; proceeding to compare working initialization patterns and test the hypothesis minimally.
- TDD/codebase-design decision: preserve `DatabaseManager::init` as the sole interface and test through the real monitor process. No new public seam; move only database-wide bootstrap work ahead of concurrent pool creation if the RED test confirms the hypothesis.
- Located the existing process-level monitor test seam and the design section that must be amended before implementation. Next: write Gate-A addendum, then one RED integration test.
- Gate-A addendum written for serial WAL bootstrap, explicit lock failure, process-level test seam, old-module relation, and rollback. `CONTEXT.md` is absent, consistent with the existing recorded repository gap.
- TDD RED: added one process-level fresh-DB test; exact run failed as expected with nine `database is locked` messages and otherwise reached the existing exit-2 ledger boundary.
- GREEN attempt 1 remained RED: serial WAL bootstrap reduced lock errors 9→1. Returned to diagnostic Phase 1; next run will label which per-connection PRAGMA still races.
- Diagnostic run labeled the remaining failure as `synchronous=NORMAL`, executed before busy timeout. Proceeding with one-variable statement reordering as GREEN attempt 2.
- TDD GREEN: `fresh_test_database_starts_without_lock_errors` PASS after serial WAL bootstrap plus busy-timeout-first pooled connection setup. The explicit PRAGMA-stage error labels remain as useful failure evidence.
- Refactor verification: complete `monitor_help_isolation` suite 3/3 PASS; the fresh-DB test then passed three additional consecutive runs.
- Post-fix diff review found the change remains localized to WAL bootstrap/customizer ordering plus the public process test and Gate-A addendum. `git diff --check` and `cargo fmt --all -- --check` PASS.
- Post-fix `cargo check --all-targets --all-features`: PASS.
- Post-fix `cargo clippy --all-targets --all-features -- -D warnings`: PASS.
- Post-fix `cargo test --all-targets --all-features`: PASS, exit 0. Library 1,336 passed / 10 ignored; monitor 293/293; all remaining targets zero failures, including fresh-DB isolation 3/3.
- Post-fix `bash tools/compliance/check.sh`: PASS with freshness at 2026-07-16 (one trading day behind) and the existing documented nonblocking warnings.
- Post-fix `cargo build --release --bin monitor`: PASS.
- Post-fix isolated release smoke on a unique fresh DB: no `database is locked`; expected exit 2 occurs only at missing same-day ledger NAV, with test environment and dry-run delivery.
- `cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json`: tests PASS and report written.
- `python3 tools/coverage/check_thresholds.py target/coverage/coverage.json`: FAIL; global 51.18% < 80%, core 55.38% < 95% across 94 files. Gate D and merge remain blocked together with missing live-account evidence and auditor sign-off.
- Final pre-stage inventory: ten tracked paths, 253 insertions / 142 deletions; includes the two authorized orphan deletions, WAL startup fix/test/design, historical progress evidence, and Phase-11 planning. `git diff --check` PASS.
- Committed all authorized tracked changes as `36f93b8` (`fix: close remaining workspace and database startup gaps`) and pushed to `stock_analysis/codex/repository-safety-closure-20260718`; upstream was updated successfully.
- Dispatched independent fixed-SHA review for `4cea222..36f93b8` under `requesting-code-review`; awaiting findings before any further code change or merge decision.
- PR-body update attempt failed due shell command substitution of Markdown backticks. Immediately interrupted it, then verified HEAD/log/upstream/worktree and remote PR body: no revert, commit, index, or PR mutation occurred. Future edit will use a temporary body file, never an interpolated shell argument.
- Safely updated PR #2 via `/private/tmp` body file and `gh pr edit --body-file`; remote body now records commit `36f93b8`, SQLite fix/test, exact 51.18%/55.38% coverage blocker, mandatory fields, and complete newest-to-oldest rollback. PR remains OPEN Draft with CLEAN/MERGEABLE metadata.
- Independent review standards axis returned FAIL with Important findings for unvalidated WAL return mode and broad evidence-directory ignores; the reported rollback omission was based on the old body and is already corrected remotely. Entering another TDD slice before asking for re-review.
- `cargo test --all-targets --all-features`: PASS; lib 1311 passed / 10 ignored, monitor 284 passed, all integration targets zero failures.
- `bash tools/compliance/check.sh`: PASS; freshness latest 2026-07-16, one trading day behind.
- `cargo build --release --bin monitor`: PASS.
- Safe release smoke used `STOCK_ENV_MODE=test`, isolated `data/test/release_smoke.db` and dry-run notification; exited 2 on missing same-day ledger NAV as required by BR-108, with no real order or real push.
- `cargo llvm-cov --workspace --all-features --json --output-path target/coverage/coverage.json`: tests PASS and report written.
- `python3 tools/coverage/check_thresholds.py target/coverage/coverage.json`: FAIL; global 49.82% < 80%, core 53.40% < 95%.
- Parallel final Standards/Spec reviews started; final status remains In Progress / Blocked until their findings are resolved and Gate D/live-account evidence exists.

## 2026-07-18 — Independent-review remediation and final gate refresh

- Closed the first re-review findings: production `--review` now routes only to strict dispatchers; Slack/custom notification success requires protocol confirmation; coverage paths normalize GitHub runner paths; poisoned L7 locks reject; BR-091 persistence failures poison the chain writer; agent audit is append-only; stale/missing current-day ledger blocks reports; rejected paper attempts reserve business IDs; integration tests use physical audit isolation.
- Extended BR-086 order evidence with an immutable SHA-256 chain. Audit row, chain row and accepted position mutation commit atomically; startup validates the complete chain, backfills only a wholly empty legacy chain, and rejects partial/mismatched evidence.
- Added sector-history and money-flow duplicate/trading-day-gap validation, BR-116 confirmation-after-delivery timer semantics, independent T0 timing, and representative TEST_CODE fixture corrections.
- Full regression initially found two deterministic `news_ranker` failures caused by reading an unrelated corrupt historical file before evaluating complete single-day Fade/Cold evidence. BR-117 now limits historical reads to stages that need three-day evidence; exact tests and the complete regression pass.
- `cargo fmt --all -- --check`: PASS.
- `cargo check --all-targets --all-features`: PASS.
- `cargo clippy --all-targets --all-features -- -D warnings`: PASS.
- `cargo test --all-targets --all-features`: PASS; lib 1330 passed / 10 ignored, monitor 290/290 passed, all bin/integration/bench targets have zero failures.
- `bash tools/compliance/check.sh`: PASS; `stock_daily MAX(date)=2026-07-16`, one A-share trading day behind on 2026-07-18.
- Workflow YAML parse and `git diff --check`: PASS.
- `cargo build --release --bin monitor`: PASS.
- Isolated release smoke used `DATABASE_PATH=/private/tmp/stock_analysis_release_smoke_20260718_1446.db`, `STOCK_ENV_MODE=test`, `STOCK_LIST=TEST_CODE_000001` and dry-run notification. It exited 2 because the required 2026-07-18 ledger was absent; strict review did not dispatch and no real order/push occurred.
- Refreshed coverage: global 42325/83443 = 50.72% < 80%; core 11540/21081 = 54.74% < 95% across 94 files. Gate D remains blocked, together with unavailable real-account same-day validation and auditor sign-off.
- Final status: **In Progress / Blocked**. Gate B/C are green; final independent re-review and draft PR remain before handoff.

## 2026-07-18 — Recovered-task final verification

- Closed the final Spec review finding: BR-087 now rejects invalid/blank trade-event identity and unknown event types before T-14/T-15 type filtering; T-14 also rejects blank order IDs and missing status. Independent Spec re-review: PASS.
- Completed repository test/live fixture isolation: ordinary fixtures use `TEST_CODE_`; native six-digit values remain only in documented provider/parser/code-mapping/market-board/environment rejection/live-validation boundaries. Independent Standards re-review: PASS.
- Added serialized isolation for the two `news::sink` global-sender tests after a full-suite-only race allowed one test to replace the sender and immediately drop its receiver. The focused suite passed 10 consecutive parallel runs.
- `cargo fmt --all -- --check`: PASS.
- `cargo check --all-targets --all-features`: PASS.
- `cargo clippy --all-targets --all-features -- -D warnings`: PASS.
- `cargo test`: PASS. Library: 1336 passed / 10 ignored; monitor: 293 passed; all binary, integration and doctest targets have zero failures.
- `bash tools/compliance/check.sh`: PASS; `stock_daily MAX(date)=2026-07-16`, one A-share trading day behind on 2026-07-18.
- Workflow YAML parse and `git diff --check`: PASS.
- `cargo build --release --bin monitor`: PASS.
- Isolated smoke used `DATABASE_PATH=/private/tmp/stock_analysis_release_smoke_20260718_final.db`, `STOCK_ENV_MODE=test`, `STOCK_LIST=TEST_CODE_000001` and `V10_DRY_RUN_PUSH=1`. It exited 2 on missing required 2026-07-18 ledger NAV before strict review dispatch; no real order or real push occurred.
- Refreshed coverage report: global 42895/84187 = 50.95% < 80%; core 11802/21298 = 55.41% < 95% across 94 files.
- Gate D remains blocked by the measured coverage deficit, unavailable real-account current-day cash/position/NAV validation, and missing auditor sign-off. Final status remains **In Progress / Blocked**; the deliverable must stay a draft PR.
- Scoped implementation commit `e7db307` was pushed on `codex/repository-safety-closure-20260718`; Draft PR #2: https://github.com/Northofqing/stock_analysis/pull/2. User-owned changes in `.gitignore`, `.superpowers/sdd/progress.md`, `src/app/context.rs`, and `src/broker/ib.rs` remain unstaged and outside the PR.

## 2026-07-18 — Phase 11 commit-all / merge evaluation

- User explicitly requested committing all remaining code and merging everything into `master`.
- Activated the existing event-replay safety-remediation plan for continuation because `.planning/.active_plan` is absent.
- Logged one failed multi-file patch caused by stale findings-tail context; re-read exact tails and switched to targeted patches.
- Next: independently inspect all remaining diffs, run full gates, commit/push the authorized changes, request fixed-SHA code review, and merge only if Gate D is fully green.
- Worktree inventory captured: seven changed files total after planning updates; the only Rust changes are deletion of `src/app/context.rs` and `src/broker/ib.rs` (132 lines combined).
- Read both deleted modules and searched their callers: no references found; the IB file is a documented zero-price/empty-sector placeholder. Continue with module-export verification and add a supersession note to the historical progress ledger.
- One inventory command assumed a nonexistent `src/broker/mod.rs`; logged the error and switched to inspecting `src/broker.rs` plus `src/app/mod.rs` directly.
- Verified both deleted files are orphaned and annotated the historical v17.7 Gate D ledger as scoped/superseded.
- Temporarily removing the planning/workspace ignores exposed extensive unrelated generated artifacts. Restored the user's ignore rules; current tracked plan/progress updates remain visible to Git.
- Phase 11 audit complete: the two Rust deletions remove unexported orphan modules; the IB module is an explicit zero/empty fake provider superseded by active fail-closed `src/broker.rs`. Historical progress claims are now scoped, and generated workspaces remain ignored.
- Phase 11 fast validation: `cargo fmt --all -- --check` PASS and `git diff --check` PASS; exact worktree remains the seven authorized tracked changes.
- `cargo check --all-targets --all-features`: PASS.
- `cargo clippy --all-targets --all-features -- -D warnings`: PASS.
## 2026-07-18 follow-up review closure

- Fixed review Important findings in working tree: strict WAL result validation, fail-closed monitor exit, and precise process-artifact ignore rules.
- TDD regression `memory_database_fails_closed_with_explicit_journal_mode_error` passes.
- Full Gate validation remains required before any merge decision; prior Gate D coverage/live-account evidence blockers remain.
- Added explicit webhook-secret isolation to process tests after independent review.
- Closed remaining fixed-SHA Important code findings in the working tree: no r2d2 customizer retry-to-health, explicit directory-failure exit 2, BR-108-specific fresh-DB assertion, and Gate-0 Copilot instructions.
- Focused public process tests: 5/5 PASS. Connection PRAGMA unit regression: PASS.
- Final Gate B/C rerun: fmt, diff-check, all-target check, strict Clippy, all-target tests, compliance, and release build PASS. Library tests are 1337 passed/10 ignored; monitor tests are 293/293; process isolation is 5/5.
- Fresh fixed-tree coverage: global 43129/84231 = 51.20% (<80%); core 11834/21342 = 55.45% (<95%, 94 files). Gate D remains blocked despite the local user-attested live-position snapshot.
- Closed final review Important findings: no production direct pool checkout remains outside `get_conn`, and webhook isolation is explicit in 5/5 process tests. Full fmt/check/clippy/all-target tests/compliance rerun PASS.

## 2026-07-18 Gate D continuation

- User directed autonomous continuation until completion and merge.
- Restored mandatory rules, design, handoff, plan, findings, progress, Git state, and coverage-gate context.
- Confirmed `codex/repository-safety-closure-20260718` is clean and synchronized at `9198f82`, eight commits ahead of local `master`.
- Updated the active persistent plan to Phase 12: core coverage, global coverage, nullable same-day account evidence, full Gate A-D validation, independent audit, and final merge.
- No production code has been changed in this phase yet.
- Added and self-reviewed the Gate D coverage-closure design addendum and executable implementation plan. The selected approach is core-first behavior testing through existing interfaces, with internal seams only where orchestration currently creates dependencies.
- The plan explicitly rejects threshold reduction, coverage exclusion, assertion-free execution, live production test credentials, missing-value fabrication, and denominator deletion for metric gain.
