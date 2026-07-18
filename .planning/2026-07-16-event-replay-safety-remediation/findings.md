# Findings: Event Replay Safety Remediation

- Existing CLI and replay tests pass but cover only happy paths.
- `parse_args` returns `Ok(None)` immediately on known monitor flags, so event command composition is order-dependent.
- The documented replay-rate syntax uses `=N`; the only test uses a separate value token.
- `limit=0` is defined as explicit unbounded mode in v17.3 but CLI rejects `<= 0`.
- `ReplayRunner::run` names the rate parameter `_rate_ms` and never reads it.
- Replay directly deserializes `EventEnvelope`, bypassing `ReplayablePushEvent::validate`.
- Publish count increments before `EventBus::publish`; failure outcomes are logging-only.
- Fresh IDs use a counter local to one `run`, so identical source files repeat IDs across runs.
- Monitor uses `runner.run(...).await.unwrap_or(0)`, collapsing replay errors into a zero-success exit.
- `HistoryQuery` also rewrote zero to the default 100, so the unbounded contract required both parser and query-layer fixes.
- Existing `generate_trace_id` demonstrates the repository's process-wide `AtomicU64` pattern.
- Replay unit tests reused the same temp path within a process, which hid behavior behind parallel file races once coverage expanded.
- History tests had the same process-wide temp-path collision; its success-rate fixture also used local noon, which is outside a trailing 24-hour window when the test runs before noon.
- The mandatory daily-data backfill needs external network access; sandboxed providers all returned empty while the approved non-sandbox run succeeded for every symbol.
- Changed-file coverage exceeds 80% and replay exceeds 95%, but the repository-wide 51.14% coverage prevents Gate D / Release Ready status under AGENTS.md Part 4.
- A broadcast `Published` outcome only proves a receiver exists; force replay needs an awaited publisher boundary to distinguish actual sink acceptance from queue admission.
- `limit=0` must be checked at parser, query, and presentation layers; a fixed `.take(20)` in the CLI can silently reintroduce an output limit after an unbounded query.

## Repository-wide continuation (2026-07-17)

- The previous seven replay findings are closed in commit `289b7b9`; the continuation scope is the remaining repository-wide historical/spec and gate debt.
- Known baseline blockers to reproduce: all-target `v14_e2e` compilation, strict Clippy diagnostics, repository formatting drift, and repository coverage below 80%.
- A warning count is not a repair plan: each diagnostic must be traced to a root cause and classified as correctness, safety, obsolete code, or style before editing.
- Existing user changes in `.gitignore` and `.superpowers/sdd/progress.md` remain out of scope and must not be staged.
- Documentation inventory currently contains 176 files in `docs/`; 72 files contain completion/PASS-style language and therefore require evidence classification rather than assumed truth.
- `cargo test --all-targets --all-features --no-run` reproduces one hard compiler error: `src/bin/v14_e2e.rs:285` calls `Dispatcher::dispatch` with two arguments after the API gained `dedup_code: Option<&str>` as argument three.
- The same file has working three-argument calls at lines 134 and 144, giving an in-repository reference pattern. The compile failure is API-migration drift in one stale test-binary call site, not a production dispatcher defect.
- The all-target compile also exposes 59 library warnings before strict Clippy; these include a real unreachable `Ok(false)` branch, private-interface mismatches, unused imports/state, deprecated API use, and an unused `MutexGuard` result.
- Adding the missing `None` makes `cargo test --bin v14_e2e --no-run` pass; the fix preserves the existing no-sub-kind dedup behavior.
- The next all-target blocker is `tests/rule_filter_benchmark.rs:616`: its `Option<EventType>` display match predates four enum variants (`Earnings`, `MarketAction`, `AnalystView`, and one additional variant reported by the compiler). This is another exhaustive-match migration gap and must be compared against the canonical `EventType` mapping before editing.
- Canonical `EventType` defines exactly four post-v15.3 variants missing from the benchmark helper: `Earnings`, `MarketAction`, `AnalystView`, and `Announcement`; `EventType::label` confirms their intended identities.
- Explicit arms were added instead of a wildcard so future enum expansion remains compile-detectable. `cargo test --test rule_filter_benchmark --no-run` now passes.
- After the two migration fixes, `cargo test --all-targets --all-features --no-run` exits 0 and builds every library, binary, integration test, and benchmark target. Compilation Gate B is restored; runtime failures remain to be measured separately.
- Full all-target execution reaches the library suite and fails 1/1254: `opportunity::auction_agent::tests::test_run_auction_agent_filters_normal` expected the time gate to skip, but the test ran during the real 09:15–09:25 auction window and processed one abnormal record.
- The failing test was authored with a hard-coded comment assuming execution at 16:45 while production correctly calls `Local::now()` via `is_auction_now()`. Root cause is a clock-dependent test seam, not changed auction classification behavior.
- `calendar::is_auction_now` has no injectable clock. The auction Agent now delegates to a private session-aware core; production passes the real calendar result while the time-gate test injects `false`. The previously failing exact test passes without changing production thresholds or force semantics.
# 2026-07-17 all-target runtime regression

- `cargo test --all-targets --all-features` 已越过此前三个阻塞点：过期 dispatcher 调用、`EventType` 非穷尽匹配、依赖真实竞价时段的随机测试。
- `opportunity::auction_agent` 保持生产路径调用真实 `is_auction_now()`；仅把核心过滤函数增加显式 `auction_now` 输入，使测试固定为非竞价时段。
- 当前完整测试进程仍在执行剩余目标；不得在进程最终退出前声明 Gate B 全量回归通过。
- 完整进程最终在 `tests/launch_gate_tests.rs::test_gray_back_to_shadow_underperformance` 失败：实现的 `< 50%` 分支返回 `Some(Gray)`，而模块级契约、函数注释和测试均要求 `Some(Shadow)`；这是实现笔误，不是测试阈值漂移。
- 该状态筛选/回退语义已先登记为 BR-044（AGENTS 2.10），并明确 50%/55% 边界，随后才能修改实现。
- 修复为 `< 50% -> Shadow` 后，`cargo test --test launch_gate_tests` 11/11 PASS；新增 `50%` 边界用例验证不误回退。
- 复跑全目标时发现模块内另有两处旧断言将低胜率回退错误锁定为 `Gray`，甚至其中注释明确写“回退 Shadow”；这些属于与契约矛盾的历史测试，统一修正为 BR-044 的 `Shadow`。
- 下一轮全目标回归发现 4 个环境耦合测试：3 个 RustDX 用例直接依赖公网 TCP 行情服务器，1 个 DeepSeek 环境键测试在构造 HTTP client 时进入 macOS system-configuration 并因 NULL 对象 panic。
- 修复设计：真实 RustDX 探针改为显式 ignored integration tests，同时补本地解析单测；DeepSeek 拆出纯设置解析，配置命名测试不再构造网络 client。生产外部失败仍显式报错，不引入 mock/fallback（2.1/2.2）。
- 定向验证：RustDX `1 passed / 3 ignored`，DeepSeek `2 passed`；生产构造仍通过 `Client::with_config`，只把纯配置测试与系统代理副作用解耦。
- 后续全目标回归在 Baostock 超时测试因沙箱禁止绑定 loopback 端口而失败。生产读取逻辑只依赖 `AsyncRead`，故将 `read_tcp_response` 泛化到 `AsyncRead + Unpin`，测试改用保持不写的 Tokio duplex；真实 `TcpStream` 生产调用不变。
- 下一轮出现 7 个随机 SQLite 失败。根因不是 DAO 各自失效：`DatabaseManager` 是进程级 `OnceCell`，仅首次 `init(path)` 生效；多个测试却传不同路径并在结束时 `remove_file`，可能删除全局连接池正在使用的文件，导致缺表、事务消失和 disk I/O error。
- 数据库测试修复契约：全局单例测试统一固定 `./test_data/test.db`，运行中不得删整库，只用 `TEST_CODE*`/唯一日期隔离并清理自身行（AGENTS 2.5）。
- 实施时采用更强隔离：`cfg(test)` 库构建把所有 singleton init 指向进程唯一临时库，使旧硬编码清理路径无法删除活动 pool；`cargo test --lib --quiet` 结果 1252 passed / 0 failed / 10 ignored。
- 全目标随后仅在 monitor `policy_hit_with_no_code_is_pushed_as_policy` 随机失败。根因是多个 monitor 测试并行 set/remove `V10_DRY_RUN_PUSH`，甚至有测试明确 remove 后探测真实通道；失败测试因此偶发走到真实 `push_wechat`。
- 安全契约：`cfg(test)` 的 monitor 通知传输必须无条件 dry-run，不能由共享 env 竞态触发真实外发；生产构建保留原 env/真实 sink 语义。
- 通知层新增单一 `dry_run_push_active()` 判定，`push_wechat` 与 L7 channel 共同使用；测试构建恒 true，非测试构建仍仅认 `V10_DRY_RUN_PUSH=1`。
- 全目标下一阻塞为 `tests/fallback_post_close_test.rs` 默认访问 5 个公网行情源且全部不可用；同文件已把 Baostock 直连标为 ignored，却遗漏主 fallback 集成测试。两者统一标为显式 live integration tests，生产失败仍返回“所有数据源均返回空”。
- 随后 `tests/fallback_sina_test.rs` 两个用例同样因 DNS/公网依赖失败；按同一契约保留为显式 ignored live integrations。
- 全库关键词复查未发现其他未标记的明确 live market-data round-trip tests；`tests/v11_three_sources.rs` 及 provider 内真实网络测试已有 ignored 标记。
- 最终 `cargo test --all-targets --all-features --quiet` exit 0：lib 1252 passed/10 ignored，monitor 263 passed，其余 bin/integration targets 全部 0 failed；新增 ignored 仅限明确 live external probes。
- Strict Clippy baseline (`--all-targets --all-features -- -D warnings`) exit 101：292 lib errors，341 lib-test errors（后者包含前者及 test-only）。优先人工审计：notification unreachable result branch、unsupported regex backreference、known-None unwrap_or、unused MutexGuard；其余按 unused/dead_code/deprecation/style 分批。
- Correctness audit details: Pushover branch uses `Ok(_)` before `Ok(false)` and therefore counts a false send as success; research HTML regex uses unsupported backreference and can panic on first lazy initialization; news sink discards a poisoned MutexGuard result; breakout classification has redundant identical branches.
- Critical historical gap: `monitor/attribution.rs` production path explicitly sets `news_ai_importance: Option<u8> = None` with “mock / Phase 6 TODO”. This is not a style lint; it is an unimplemented production data source and must be wired to real existing analysis or made explicitly unavailable with docs/evidence (2.1/2.8).
- Pushover official contract verified: HTTPS POST `/1/messages.json` with form `token/user/message`; success requires HTTP OK plus JSON `status=1`. Existing Slack reuse is a fake target implementation, not an acceptable simplification.
- First correctness batch implemented: real Pushover request/response contract with offline request test; Pushover false no longer counted as success; valid script/style regex plus regression test; explicit poisoned guard drop; equivalent breakout branch simplification. Full lib regression: 1254 passed / 0 failed / 10 ignored.
## 2026-07-17 — BR-019 attribution gap

- `src/monitor/attribution.rs::attribute_event` has no production caller; current coverage only exercises the module's own tests, so the documented G5a attribution path is not wired into live alert processing.
- The module still hard-codes `news_ai_importance = None`, emits a `[未接 fund_flow]` placeholder, and does not persist the required `alert_log.jsonl` audit record.
- Existing `NewsAIAnalyzer` entry points call an LLM with multi-second timeouts. Wiring them directly would contradict BR-019's deterministic, no-LLM, P95 <= 2s immediate-attribution contract. The remediation therefore needs a rule-only classifier and a real production/audit seam, not a cosmetic replacement of the `None`.
- `docs/业务规则清单-registry.md` still marks BR-019 as pending, consistent with the code-level gap.
- The real news loop writes each event before state-machine acceptance and `push()` writes accepted events again, so accepted legacy events can be archived twice while neither copy contains attribution.
- `alert_log::append_jsonl` and `append_md` swallow create/write/serialization failures; this cannot provide the explicit, testable audit-failure handling required by 2.7.
- Business-rule traceability is itself contradictory: `docs/business_rules.md` assigns BR-019 to zero-signal suppression, while `docs/业务规则清单-registry.md` assigns BR-019 to immediate attribution. New remediation semantics must use a fresh canonical ID and retain the old ID only as a legacy alias.
- `news_monitor::process_flash` receives a real source importance but drops it when constructing `AlertEvent`; reconstructing that exact source field from free text would be fabrication. The first safe vertical slice must therefore distinguish source-provided importance from deterministic title-rule evidence, and leave unavailable evidence explicit.
- Focused `cargo test --lib monitor::` exposed an additional order dependency hidden by the full suite: `EntityLinker::new` panics through `DatabaseManager::get()` unless an unrelated earlier test initialized the singleton. Nine entity-linker/news-monitor tests fail when selected alone. Constructors must tolerate an uninitialized optional cache/database rather than depending on test order.
- Root cause was the portfolio storage layer: five public `Result`-returning functions called panic-only `DatabaseManager::get()`. They now use `try_get()` and return context-specific explicit errors. Independently selected entity-linker tests pass 6/6.
- `cargo test --all-targets --all-features --no-run` succeeds after the structured `news_importance` field and production attribution wiring, proving every constructor explicitly declares the evidence as `Some(real_value)` or `None`.
- Strict Clippy recomputation now reports 286 library errors (down from the earlier 292 baseline before the correctness batches), plus later test/binary diagnostics. The majority are machine-applicable unused imports, iterator/style simplifications, deprecated test-only calls, and dead-code/interface hygiene; non-mechanical groups include overly broad interfaces and dormant historical modules.
- After adding explicit `f64` types at four score accumulators (to keep Clippy's cast-removal fix type-safe), `cargo clippy --fix --lib --all-features` applied machine-applicable changes across the library and reduced remaining library diagnostics from 286 to 137. No `--broken-code` mode was used.
- The first post-fix lib-test compile exposed three imports that were production-unused but test-required (`TemplateCategory`, `Path`, `serde_json::Value`). They were restored inside their respective `#[cfg(test)]` modules, keeping normal builds clean without breaking tests.
- Remaining 137 strict-library diagnostics were exported as structured JSON. Main groups: 15 unused/deprecated/private-interface diagnostics, 34 intentional/dormant dead-code diagnostics, 18 broad-interface/type-complexity diagnostics, and 70 localized control-flow/iterator/documentation diagnostics.
- The current 79-diagnostic remainder contains 28 localized findings. Most are semantics-preserving iterator/doc/control-flow cleanups; `ReportType::from_str`, `VetoMode::from_str`, and `Arc<GeminiAnalyzer>` require caller/ownership analysis because a cosmetic lint fix could change public parsing defaults or async sendability.
- Caller analysis showed both parsing helpers intentionally accept unknown strings with a safe default; implementing infallible `FromStr` preserves that contract. `AnalysisPipeline` is not cloned/shared across threads, so owning `GeminiAnalyzer` directly matches its `RefCell`-based single-task state and removes the false thread-sharing signal from `Arc`.
- The 63-item strict-library remainder is now cleanly bounded: 24 dead-code/deserialization compatibility findings, 14 type-complexity findings, 13 wide stable interfaces, and 12 rustdoc findings already patched pending recomputation.
- Dead-field review found real behavior gaps: `JsonlWriter.retention_days` was stored but never invoked; `SinaFlashProvider::with_lids` was ignored by a hard-coded four-way `join!`; and `paper_engine` escaped codes but built an unquoted SQL `IN` list, risking loss of leading-zero semantics. These were fixed as behavior, not silenced.
- The separate `SearchService.xueqiu` field was a duplicate unused provider; Xueqiu already participates through the main `providers` vector, so only the duplicate instance was removed. NewsAI's unused linker similarly duplicated the production linker owned by `NewsMonitor` and caused needless DB/cache loading.
- After semantic cleanup, only the analyzer's superseded JSON dashboard parser/request stack remains dead. It is retained solely for archived public-type compatibility while production uses `TEXT_SYSTEM_PROMPT`; narrow item/module annotations document that boundary.
- Library strict lint gate is green. All-target strict lint still reports test/bin-only debt led by 58 dead-code and 41 deprecated diagnostics plus mechanical documentation/iterator warnings; this is a separate cleanup surface and does not invalidate the library gate evidence.
- The all-target fix pass exposed one stale `Candidate.source` test initializer after the production query stopped selecting that unused column; the fixture was corrected and the machine pass completed without broken-code mode.
- `cargo fmt --all -- --check` reveals repository-wide baseline drift far beyond changed files (including benches and untouched strategy/trading modules). AGENTS Gate B still requires a green global check, so final completion must either format the full tree and review/test it or remain explicitly In Progress; it cannot be reported green from scoped formatting alone.
- Deprecated monitor push calls were not harmless style debt: the two-argument shim assigned every PerTicket event the same `_per_ticket_unbound` key. BR-047 now makes that misuse fail closed, while actual ticket flows carry real codes through structured screener/T0 outputs and E2E parameters.
- Once the library/test blockers were removed, strict all-target lint exposed a second monitor-only layer: many push-template symbols compile in the production binary but are referenced only by test/E2E modules, while a smaller set of dispatch/load helpers is genuinely uncalled. These must be classified as test-only, intentionally retained compatibility, or documented-but-unwired before the monitor gate can be called green.

## 2026-07-18 — Candidate truthfulness findings

- `load_news_to_idea_snapshot_real` treated an absent P5 file as permission to copy one industry-chain code into `StockPick`, `OptimalClose`, `VolumeWatchlist`, and `VolumeRealTrade`. This manufactured four-source agreement and changed ranking/stage semantics.
- Candidate entries used `0.0` for missing price/change and reused change percent as a heat proxy even though the documented heat formula requires main net inflow.
- P-03 constructed an empty candidate list, fell back to one chain row with zero price, then either stayed permanently silent or would have emitted fixed `vol_ratio=1.0` and three fabricated `Mid` evidence grades after a future price patch.
- `load_p5_source_items` silently returned empty for unknown source names, all IO errors, malformed JSON rows, invalid codes, and blank names, making source corruption indistinguishable from legitimate no-data.
- The safe seam is a shared `RealCandidateBatch`: exact source parsing, one complete real-time quote batch, explicit optional auxiliary values, hard-gate filtering, and deterministic sorting. D-01/P-03 then consume only the fields each template actually requires.

## 2026-07-18 — Final review-remediation findings

- SQLite UPDATE/DELETE triggers alone were insufficient for BR-086. The implemented order audit now has same-transaction SHA-256 chain evidence, full startup/pre-append validation, immutable chain triggers and fail-closed handling for partial or mismatched chains.
- Production periodic notification paths had advanced timers before data/push outcomes and the holding-health comparison mutated state before delivery. BR-116 separates check/commit, advances only for real-empty, deduped or confirmed outcomes, and gives T0 scanning an independent timer.
- Sector and money-flow batches require per-code unique consecutive A-share trading dates. A calendar-day test fixture had incorrectly included a weekend; production correctly rejected it and the fixture now uses 2026-07-15/16/17.
- HeatStage Fade/Cold/Divergence and day-extreme Climax have complete single-day evidence and do not consume sector history. Start/Ferment/cumulative Climax require validated continuous history; failure there remains Unknown rather than zero fallback (BR-117).
- Refreshed Gate D remains materially below threshold: global 42325/83443 = 50.72%, core 11540/21081 = 54.74% over 94 files. This cannot be truthfully closed by a localized safety patch.
- Real-account current-day cash/position/net-value evidence is unavailable. The correct release behavior is fail-closed exit 2, so Release Ready and Done remain prohibited by AGENTS Part 4.

## 2026-07-18 — Recovered-task closure findings

- BR-116 delivery confirmation is now complete for periodic holding/review/turnover and T-14/T-15 batches: only real-empty, fully deduped, or fully delivered batches advance state; any denied/sink-failed member leaves the batch due for retry.
- BR-087 validation must occur before an event-type filter. Otherwise an unknown type can disappear as an apparently legitimate empty batch. T-14/T-15 now validate every fetched event first and reject incomplete identity, unknown type, invalid price/lot, blank order ID or missing status as applicable.
- Test fixture conversion exposed legitimate format-protocol boundaries. Parser/provider/CLI/code-mapping tests retain native six-digit values with explicit exception comments; ordinary decision, risk, push, audit, opportunity, diagnostic and in-memory E2E fixtures use `TEST_CODE_`.
- `news::sink` owns a process-global sender. Its two tests could call `install()` concurrently, allowing the sender whose receiver had already dropped to overwrite the still-live pair. A test-only mutex now covers installation, receiver lifetime and assertion; production sender semantics are unchanged.
- Current exact Gate D deficit is global 42895/84187 = 50.95% versus 80% and core 11802/21298 = 55.41% versus 95% over 94 files. Gate D cannot be closed by relabeling ignored live integrations or narrowing the denominator.
- The release smoke demonstrates the required failure boundary, not release readiness: a test-isolated review run exits 2 when the 2026-07-18 real ledger NAV is absent. Real account cash/position/NAV evidence and an auditor sign-off are still external blockers.

## 2026-07-18 — Final gate findings

- Gate B is green after the BR-091..BR-115 remediation: global formatting, all-target/all-feature compilation, strict Clippy with `-D warnings`, and the complete all-target test process all exit 0. The final library result is 1311 passed / 10 ignored; monitor is 284 passed; every integration target has zero failures.
- BR-115 closes the remaining audited supplement-data gap: financials, money flow, intraday shape and minute K-line APIs preserve transport/protocol/bad-row errors; only verified values enter caches. The incomplete Sina money-flow fallback was removed because it fabricated unavailable big-order and percentage fields as zero.
- Feishu and daemon notification response-body read failures now remain delivery errors. The v17 earnings poll requires complete financial plus consensus evidence and no longer substitutes default consensus or the current date for a bad report date.
- `bash tools/compliance/check.sh` exits 0. Freshness passes with `stock_daily MAX(date)=2026-07-16`, one trading day behind on 2026-07-18; fake implementation, design contradiction, business-rule and silent-push-fallback checks pass.
- The release monitor builds successfully. The isolated `--test` smoke opens only `data/test/release_smoke.db`, forces dry-run delivery, and exits 2 before review because the test database has no same-day real ledger NAV. This is the intended BR-108 fail-closed behavior; it does not prove live-account validation.
- Refreshed coverage is an exact Gate D blocker: global 41131/82555 = 49.82% versus 80%; core 11008/20615 = 53.40% across 94 files versus 95%. Closing the gap requires roughly 24,913 additional global covered lines and 8,574 core covered lines (assuming the denominator does not grow), not a localized patch.
- Because coverage and real-account same-day evidence are unavailable, AGENTS Part 4 requires status `In Progress / Blocked`; Gate A/B/C evidence is valid but Release Ready and auditor sign-off must not be claimed.
