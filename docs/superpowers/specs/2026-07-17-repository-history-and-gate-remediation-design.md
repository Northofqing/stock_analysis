# Repository History and Gate Remediation Design

## Status and references

- Status: Gate A design for autonomous continuation authorized on 2026-07-17.
- Fixed point: `289b7b9` (completed v17.3 replay remediation).
- Data red lines: 2.1–2.10, with 2.3/2.4/2.5/2.6/2.7 treated as highest-risk review areas.
- Planning record: `.planning/2026-07-16-event-replay-safety-remediation/` Phases 6–10.

## Objective

Reconcile active historical documents with the code that actually runs, eliminate current repository compilation/test/lint/format blockers by root cause, and raise coverage using behavior-focused tests. A document claim is not considered implemented until module, production integration, and executable evidence agree.

## Data flow and execution order

`active docs/specs` → claim inventory → code/call-chain evidence → executable reproduction → risk classification → one vertical RED/GREEN repair → scoped formatting → module/integration validation → full gates → coverage → Standards/Spec review → Git evidence.

The order is deliberate: compilation and correctness failures are repaired before style debt; critical trading/data paths receive coverage before utilities; stale documentation is corrected only after code facts are recomputed at HEAD.

## Repair batches

1. Baseline: capture exact active specs, current failures, diagnostic counts, coverage, and dirty-worktree ownership.
2. Compilation/regression: repair all-target failures one root cause at a time with focused tests where behavior changes.
3. Safety/compliance: inspect diagnostics and documented claims touching real data, order guards, test/live isolation, audit, and silent fallback.
4. Lint/format: group diagnostics by repeated root cause; make scoped mechanical edits only after correctness batches are green.
5. Coverage/spec closure: add tests through public seams, prioritize core paths, and replace unsupported completion claims with exact evidence.
6. Release evidence: release build, safe monitor dry-run, compliance/freshness, coverage, and parallel two-axis review.

## Failure modes

| Failure | Required behavior |
|---|---|
| A spec claim has no reproducible code evidence | Mark unverified/stale; do not implement or delete code from the claim alone. |
| Single-line grep reports zero callers | Trace multiline call chains and production entry points before deciding. |
| A diagnostic is in a trading/data path | Investigate semantics and add a behavior test; do not apply blind auto-fix. |
| Formatting touches unrelated historical code | Revert the formatting-only scope and format only explicitly changed files. |
| Full tests need live credentials/network | Run safe local layers; report the exact external blocker and never fabricate success. |
| Freshness fails | Run the mandated backfill and recheck before proceeding. |
| A fix reveals a data red-line conflict | Return to Gate A/B and update failure-mode analysis before continuing. |

## Old module relations

| Area | Decision | Reason |
|---|---|---|
| Existing production monitor paths | Adopt and trace | They are the required integration truth; isolated replacement modules are insufficient. |
| Existing historical specs under `docs/v*.x/` | Audit, not trust blindly | Claims may be stale or spec-on-spec derived; executable evidence decides status. |
| Existing tests | Adopt and deepen | Passing tests are retained; missing behaviors get focused public-interface tests. |
| Compliance scripts | Adopt unchanged initially | They encode repository red lines and remain mandatory gates. |
| Global rustfmt/clippy auto-fix | Reject as a batch strategy | It obscures semantic changes and can rewrite live-trading code without diagnosis. |

## Validation

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets --all-features`
- `cargo test --lib`
- `cargo build --release --bin monitor`
- safe monitor dry-run with external delivery disabled
- `bash tools/compliance/check.sh`
- `cargo llvm-cov --all-features --workspace --summary-only` or the narrowest supported equivalent

## Rollback

Each repair batch is committed independently. Revert the affected batch with `git revert <sha>` and rerun the gate that motivated it. No database migration, live order, or real notification is authorized by this plan.

# Gate D coverage-closure addendum (2026-07-18)

## Status and authorization

- The user authorized autonomous continuation until all mandatory gates pass and the PR is merged.
- Fixed-tree baseline at `9198f82`: global line coverage 43,129/84,231 = 51.20%; registered core coverage 11,834/21,342 = 55.45% across 94 files.
- The minimum no-growth gaps are 24,256 global lines and 8,441 core lines. Gate D remains blocking until the measured report reaches global >=80% and core >=95%.

## Considered approaches

1. **Behavior slices through existing interfaces (selected).** Cover deterministic pure modules first, then introduce only internal seams where an orchestration module currently creates its dependencies. Tests use worked literals, strict parser samples, isolated SQLite, and `TEST_CODE_` identities. This gives diagnostic failures and preserves production interfaces.
2. **Whole-process integration only (rejected as the primary strategy).** It can cover broad happy paths but is slow, clock/network sensitive, and cannot reliably exercise bad-data and failure branches. Process tests remain a final integration layer.
3. **Coverage exclusions, threshold reductions, assertion-free execution, or denominator deletion (rejected).** These would create a green metric without release confidence and violate AGENTS Gate D. Truly unreachable production code may be decommissioned only with independent call-chain evidence and a business reason, never to improve the percentage.

## Module and seam design

- Pure scoring, veto, formatting, parsing, and state-transition modules keep their current interfaces. Unit tests live beside the implementation so private parsing branches can be exercised without widening the production interface.
- Orchestration modules keep the external interface used by `AnalysisPipeline`. If they instantiate clocks, transports, or stores internally, the implementation gains the smallest internal seam that accepts the dependency and returns an owned result. Production and tests cross the same behavioral interface; test adapters never enter production registration.
- Network providers separate request/response transport from strict response parsing. Parser tests use deterministic protocol payloads labelled with `TEST_CODE_`; they are protocol fixtures, not fabricated production market/account data. Production fetch failures remain explicit and no fake fallback is introduced.
- Database tests use unique `TEST_CODE_` rows and isolated temporary databases when the public singleton is not involved. They validate through repository interfaces and audit records, not by weakening schema constraints.
- Process tests remove notification credentials, use test environment guards, and must not place real orders or send real messages.

## Coverage batch order

1. Pure zero-coverage core modules: score breakdown, veto, report/technical renderers, result types, market regime, price statistics, trade type, multi-timeframe, summary helpers.
2. Strict provider parsing and missing/bad-data paths: money flow, financials, valuation, consensus, industry, chip distribution, GTIMG/Eastmoney/Baostock/Sina/RustDX adapters.
3. Pipeline orchestration: position tracker, backtest runner, chain analysis/fetchers, main analysis pipeline, reporting.
4. Remaining registered core deficits in database, trading, decision, risk, and event modules.
5. Repository-wide deficits, prioritizing pure notification/report/search/strategy code before decomposing the monitor process entry point.
6. Recompute coverage after every batch; the JSON report, not test count, decides the next target.

## Failure modes and data red lines

| Failure | Required behavior |
|---|---|
| Fixture represents a live order/account | Reject; use `TEST_CODE_` and test mode or locally attested private evidence outside Git. |
| Missing provider/account field | Assert `None`, warning, or explicit error; never use zero/default to make a test pass (2.2). |
| Bad price/time/continuity payload | Assert whole-batch rejection before computation (2.3/2.4). |
| Test would need a production webhook/broker credential | Remove the credential and exercise a dry-run/test adapter or explicit unavailable error (2.5). |
| Refactor changes order or data semantics | Return to Gate A, register/update the relevant BR, and add a RED public behavior test before implementation. |
| Coverage rises but assertions do not prove outcomes | Reject the test as evidence and replace it with independent expected literals. |
| Threshold still fails | Record exact numerator/denominator and continue from the largest safe uncovered behavior cluster. |

## Same-day real-account evidence boundary

- The local 2026-07-18 attested snapshot proves seven positions, cash, market value, total assets, and holding P&L at its capture time. It does not provide daily P&L; the source displayed that field as unavailable.
- Same-day account persistence must therefore represent daily P&L as nullable and store source, source timestamp, observed-at timestamp, and account-mode provenance. It must not insert zero into the current non-null ledger field.
- A schema/data-flow change requires an idempotent migration, compatibility read path, BR-103 documentation, isolated database tests, and a backup/rollback procedure before the private local database is migrated.

## Release and rollback

- Every batch is a small commit with its focused test and refreshed coverage evidence. A failing batch is reverted with `git revert <batch-sha>` and the relevant gate is rerun.
- Schema migration rollback restores the pre-migration local database backup and reverts the migration commit; private evidence and account values remain ignored and are never uploaded.
- PR #2 stays Draft until all Gate B/C checks, both coverage thresholds, live-data validation, audit trace validation, and independent Standards/Spec/Audit review pass. Only then may it become Ready and merge into `master`.

# External integration test isolation addendum

- Default `cargo test --all-targets --all-features` must not require a live quote server, API key, system proxy, or production account.
- Live RustDX connection/K-line/realtime probes remain executable as explicitly ignored integration tests; deterministic code normalization and market routing stay in the default unit suite.
- LLM environment-name tests validate parsed provider settings without constructing an HTTP client. Production construction still uses the same settings to build the real client.
- External failures remain explicit in production paths; this test isolation must not add mock or fallback data to production (Data Redlines 2.1/2.2).

# Process-global database test isolation addendum

- Tests using the production `DatabaseManager` singleton share one fixed test database for the entire test process; later `init(path)` calls must not be treated as reconfiguration.
- No test may unlink the database file while the global pool is alive. Tests isolate rows with `TEST_CODE` identifiers and unique dates, and may delete only their own rows.
- Library unit-test builds force all singleton initialization calls onto one process-unique temporary database, so legacy hard-coded cleanup paths cannot unlink the active pool. Non-test/production builds continue honoring the supplied path.
- This repairs test lifecycle only; production singleton, migrations, WAL, and explicit database errors remain unchanged.

# Notification test safety addendum

- A `cfg(test)` monitor binary must never enter a real notification transport, regardless of process-environment races. Its transport result is deterministic dry-run success while governance/rendering/audit paths remain exercised.
- Non-test builds retain the existing `V10_DRY_RUN_PUSH` opt-in semantics and real sink failure reporting.
- `NotificationChannel::Pushover` must use Pushover's versioned Message API (`POST https://api.pushover.net/1/messages.json`, form fields `token`, `user`, `message`) and count success only when HTTP is successful and response JSON has `status=1`; it must never reuse another channel's webhook.

# Immediate attribution and alert-audit addendum

- Canonical remediation rule: BR-045. The v10 registry's BR-019 is retained as a historical alias because `docs/business_rules.md` already used BR-019 for an unrelated zero-signal rule; new code and evidence must cite BR-045.
- `AlertEvent` carries source news importance as `Option<u8>`. Only a provider/classifier value actually received by the event builder may populate it; missing importance stays `None` and must not be inferred from alert level or free text.
- `AttributionRequested` is a synchronous domain message at the monitor boundary. Its handler may use only source importance and the deterministic `chain_mapper` rule path. It must not call LLM, T+1 data, live HTTP, or fabricate fund-flow evidence.
- Before delivery, the handler writes the conclusion into `AlertDetail.ai_decision`. No-catalyst and missing-evidence outcomes are explicit. The synchronous rule path has a 2-second budget and returns its measured elapsed time for tests/audit.
- An accepted alert is archived once, after enrichment and before notification delivery. JSONL includes the attribution decision and relevant news evidence. Serialization/open/write failures return errors and are logged explicitly; they are never silently counted as successful audit writes.
- Detection-only test paths may archive raw events separately, but production loops must not double-write the same accepted event through both the caller and `push()`.

Failure handling:

| Failure | Behavior |
|---|---|
| Source importance absent | Preserve `None`; chain rules may still establish a catalyst, otherwise write the no-catalyst label. |
| Chain rules unavailable or over budget | Record the failure/missing item; do not call AI fallback in G5a. |
| Audit serialization/open/write fails | Log an error with event code and path context, continue risk notification delivery, and expose failure through the audit API/tests. |
| Notification fails after audit | Existing notification governor reports delivery failure; the attributed audit record remains available for reconciliation. |

# Broker, quote, and health fail-closed addendum

- The monitor must register only a quote provider that performs a real data-source request. A missing, unsupported, or unknown `BROKER_SOURCE` selection is an explicit startup/configuration error; it must not install a mock/no-op provider or claim that an unimplemented SDK is active (Data Redlines 2.1/2.8).
- `QuoteProvider` returns `Result` and validates every price as finite and strictly positive before it reaches decision or paper-trading computation (2.2/2.3). Provider absence, transport failure, an empty response, or an invalid price stays an error.
- A stored push price is a historical observation, not a realtime quote. Decision and paper sell paths must not silently substitute push price or average cost when their contract requires current price. They skip/error without emitting a trade when current quote evidence is unavailable.
- The former broker-to-application `BrokerPush` implementations are removed because they had no production callers and only logged. Existing realtime data providers remain the adopted integration seam; future QMT/Magiclaw adapters must implement a real provider before their source can be selected.
- Quote health is blocking and requires a registered real provider. Performance health queries the persisted `paper_performance_snapshot.created_at` age rather than returning a constant. Bus health reports only observable bus state supported by the bus API; constructing singleton handles alone is not evidence of live consumers.
- Webhook delivery performs the configured HTTP request and reports failures explicitly. An absent webhook URL disables that optional sink with an explicit result; it must not default to a fake hostname or log-only success (2.8).

Failure handling:

| Failure | Behavior |
|---|---|
| Public quote provider construction fails | Startup returns an error and health remains failed. |
| Realtime quote request/parse returns no valid price | The affected decision/sell operation returns or records an explicit error; no order/simulation is emitted. |
| Unsupported broker source selected | Reject configuration and name the unsupported source; do not downgrade silently. |
| Performance snapshot absent or older than 24 hours | `perf_recent=false`; health alert names the component. |
| Alert webhook unset | Return `Disabled` and log the disabled state; never manufacture a URL. |
| Alert webhook HTTP/non-2xx failure | Return an error with endpoint/status context so callers can audit the failed delivery. |

# Compliance-gate and business-rule registry addendum

- Compliance prerequisites are fail-closed. A missing production database, missing `sqlite3`, unreadable schema, or unparseable date is a gate failure, never a successful skip (2.4).
- Daily freshness is measured in A-share trading days from a checked-in calendar for the current year. Weekends and declared exchange holidays are excluded; absence of current-year calendar coverage is itself a failure so a stale calendar cannot silently approve data.
- Fake-implementation checking covers every production Rust function whose name contains `verify`, `save`, `notify`, `push`, `sync`, `update_result`, or `reconcile`. Logging-only/no-op bodies and explicit mock/stub/default-success markers are blocking unless the function is under `#[cfg(test)]` or a clearly named test helper (2.1/2.8).
- Design-contradiction checking reads explicit configuration fields and matching implementation bounds from a machine-readable contract; missing sides fail. It must not infer a bound by taking arbitrary maximum numbers from nearby source text (2.9).
- `docs/business_rules.md` is the current canonical registry required by AGENTS 2.10. The historical Chinese registry remains source evidence during migration, but duplicate IDs with different meanings are forbidden. Later colliding rules receive new stable IDs and active code/spec references are updated; the compliance gate checks uniqueness and referenced-path existence.
- The top-level compliance runner must propagate every blocking subcheck. No `|| true` is permitted on a check advertised by the runner as part of compliance.

Failure handling:

| Failure | Behavior |
|---|---|
| DB/tool/calendar prerequisite missing | Fail with the exact missing prerequisite and remediation command. |
| Current-year trading calendar absent | Fail; add an exchange-sourced calendar before release. |
| Business-rule ID has multiple meanings | Fail and list both registry rows; renumber the later rule and update active references. |
| Rule points to a missing code path | Fail for both new and historical active rules; spec-only rows must be explicitly marked. |
| Threshold contract side missing/unparseable | Fail rather than skip. |

# Paper and simulated order-safety addendum

- All simulated/paper orders use one validation contract before persistence: finite price > 0; quantity > 0 and divisible by 100; buy notional <= available cash and <= RMB 1,000,000; order price inside the instrument's current daily limit range; duplicate business ID rejected for 60 seconds; and notional >= RMB 500,000 requires an explicit secondary-confirmation bit (Data Redlines 2.3/2.6).
- Missing quote, daily-limit bounds, account cash/net value, or confirmation evidence is a rejection. Auction/no-trade periods do not turn `quote_price=0` into a fill.
- Paper account state comes from the persisted ledger and positions, with `created_at` no older than 30 seconds and ledger date equal to the current trading day. Fabricated initial capital/cash percentages and `(0,0,0)` error fallbacks are removed (2.1/2.4).
- The realtime provider exposes execution evidence (price, upper/lower limit, observation time). Parsers reject malformed required price/bound fields; unavailable optional fundamentals remain `None` rather than numeric zero (2.2/2.3).
- Database table definitions and migrations enforce positive price, positive lot quantity, and valid filled-price constraints so direct SQL cannot bypass the application gate.
- Existing call sites that cannot supply all evidence skip/error without writing an order. This intentionally makes an incomplete historical feature unavailable instead of apparently filled.

Failure handling:

| Failure | Behavior |
|---|---|
| Account snapshot absent/stale/invalid | Reject before evaluate or persistence; identify the missing/stale field. |
| Quote or daily-limit bound absent/invalid/stale | Reject; no signal-price/cost-price substitution. |
| Order >= RMB 500,000 without confirmation | Reject with `secondary confirmation required`. |
| Persistence fails after dedup reservation | Roll back the in-memory reservation so an identical safe retry can proceed. |
| Existing DB violates new constraints during migration | Migration fails and reports offending rows; do not coerce historical values. |

## Order concentration and immutable audit extension

- Position sizing never invents account capital. Non-dynamic sizing uses a fresh persisted cash snapshot; dynamic sizing requires a finite positive volatility input. Neither path forces a minimum lot when its budget is insufficient.
- A new position requires a concrete chain classification from a fresh concept cache or the checked-in chain registry. Existing and same-day frozen exposure for that chain is read from `stock_position`; missing classification/query evidence blocks the order rather than becoming zero (BR-085).
- Every order attempt carries a business order ID and decision basis into an append-only `order_audit` table. Accepted position mutation and its `Filled` audit row commit in one SQLite transaction. A rejected attempt is recorded before returning whenever the database is available; inability to persist required audit evidence blocks the action (2.7, BR-086).
- `order_audit` rejects application-level UPDATE and DELETE operations. Every row is also committed to an immutable SHA-256 hash chain in the same database transaction. Runtime bootstrap validates the complete chain before accepting new audit records; a missing, partial, or mismatched chain blocks order-audit initialization and subsequent order activity. A legacy database may be backfilled only when the chain table is entirely empty, preserving the existing audit row order. No cleanup path shorter than five years is provided; future archival must preserve both records and chain evidence and be documented as a migration.
- A versioned `v17-order-audit` migration creates the same table, semantic insert trigger, index and immutable triggers used by runtime bootstrap. Its down migration intentionally preserves the table and triggers because code rollback does not authorize destruction of retained audit evidence.

| Failure | Behavior |
|---|---|
| Chain classification or exposure unavailable | Reject the opening order and name BR-085; do not assume an `其他` bucket or zero exposure. |
| Volatility or real cash unavailable | Reject sizing; do not substitute 3% volatility or RMB 100,000 capital. |
| Audit insert fails | Roll back a prospective fill or reject the attempt; never report an unaudited fill. |
| Audit hash chain is missing, partial, or mismatched | Fail closed during bootstrap or before append; never repair a partial chain implicitly. |
| Position write fails inside audited transaction | Roll back both position and filled audit; write a separate rejected audit if possible. |

## Trade-event source extension

- T-14/T-15 may consume only a registered real `TradeEventSource`. Registration is process-wide and single-assignment; an absent source is an explicit unavailable error, not an empty successful event batch (BR-087, redlines 2.1/2.8).
- Source transport errors and malformed order/fill events are recorded separately. Dispatchers filter by event type only after a successful fetch and never claim a source is healthy merely because its adapter object exists.
## Addendum: BR-078 新闻源失败语义与生产注册表（2026-07-17）

- 数据流：生产注册表只装载具有真实抓取实现的轮询源；抓取成功的空列表表示“本轮无事件”，抓取错误保留源名并上抛给聚合器记录失败。
- 失败模式：未实现、主动触发或被动注入型 feed 不得伪装成轮询成功；直接误调用返回 `unavailable` 错误。单源失败不伪造事件，也不阻断其他真实源，但本轮失败必须可观测。
- 旧模块关系：保留现有 provider 和 BR-082 NewsFlashGate；拒绝并停止注册六个 skeleton feed。公告仍由既有 `NewsMonitor` 主路径消费，避免双轨重复。
- 回滚：恢复 `news_aggregator_init.rs` 的旧 feed vector 与 `feed.rs` 空列表实现；该回滚会重新引入红线 2.1/2.8 风险，只允许通过受控例外路径执行。
## Addendum: BR-005 机会推送日限额（2026-07-17）

- 数据流：`CandidateBoard` 进入 L5 前，从持久化 `push_analytics` 查询本地当日、默认用户、同模板且 `pushed=1` 的真实成功数；小于 5 才允许继续。
- 失败模式：计数数据库不可用或查询失败时 fail closed 为 `daily_limit_count_unavailable`；不得回退为 0。治理拒绝、去重和发送失败记录不计入成功配额。
- 旧模块关系：采用现有 L5 `max_per_user_per_day` 与 L7 SqliteStore，删除适配器硬编码 0；不另建第二套内存计数器。
- 回滚：撤销 CandidateBoard profile 上限及 L7 模板计数查询；会恢复 BR-005 绕过，仅允许受控例外。
## Addendum: BR-089 LaunchGate Calmar 真值口径（2026-07-17）

- 数据流：`metrics_from_db` 同一连接读取 prediction_tracker 的运行期/胜率与按日期升序的真实 `ledger`，Calmar 使用净值首尾收益、245 日年化和全序列最大回撤。
- 失败模式：缺样本、坏净值、无序/重复日期、交易日断档、无法定义的零回撤均返回错误，调用方不得把错误转换为 0 后继续评估升级。
- 旧模块关系：复用 `ledger` 作为账户净值单一事实源；不采用按单笔 paper trade 计算的金额回撤，因为其缺少资金基数，无法形成可比比率。
- 回滚：恢复固定 `calmar_ratio=0.0`；会使 Shadow→Gray 永远不可达并重新引入 STD-024。
## Addendum: BR-051 monitor --test 物理隔离（2026-07-17）

- 数据流：CLI 模式判定后、DB 和 broker 初始化前设置 `STOCK_ENV_MODE`；test 默认数据库固定在 `data/test/monitor_test.db`，生产默认仍为 `data/stock_analysis.db`。
- 失败模式：测试 E2E 不允许硬编码生产 DB 路径或非 `TEST_CODE` 标的；目录/数据库初始化失败必须出声并停止相关路径。
- 旧模块关系：保留 DatabaseManager 与现有 E2E dispatcher，只替换路径解析与 seed 代码；外部显式 `DATABASE_PATH` 仍可为 CI 提供一次性隔离库。
- 回滚：恢复统一生产 DB 默认值和 `999999` seed；违反红线 2.5，禁止常规回滚。
## Addendum: BR-091 推送投递不可变审计（2026-07-17）

- 数据流：EventBus 的 `push.delivery.audit` 精确路由到 AuditDispatcher；dispatcher 在锁内校验/续接年度 hash chain、append、flush+sync 后才返回 Handled。
- 失败模式：既有链任一行 JSON/parent/hash 不一致、路径不可写或 fsync 失败均返回 Failed；主消费者打印 error，不把失败当成功。计数仅代表已持久化记录。
- append、flush 或 sync 任一步失败后，内存链状态必须永久标记 poisoned；同一进程后续写入直接拒绝，禁止以旧 `last_hash` 续写可能已经部分落盘的 sibling 记录。只有重启并重新验证完整磁盘链后才允许继续。
- 旧模块关系：保留 JsonlWriter 作为通用事件回放源，但它不再充当不可变审计证明；其保留期和 dispatcher log 同步提高到至少 1827 天。
- 回滚：恢复 stdout-only AuditDispatcher 或 7 天清理；违反红线 2.7，禁止常规回滚。
## Addendum: BR-092 日 K 坏数据边界（2026-07-17）

- 数据流：provider CSV 或 stock_daily nullable row 先转为已验证 KlineData，再交给策略；批量完成后按交易日历检查日期连续性和相邻收盘变化。
- 失败模式：列缺失、字段解析失败、非有限/负值、OHLC 关系错误、重复/缺失交易日、相邻变化绝对值大于 20% 均返回带行号/日期的错误，不跳过、不填零。
- 旧模块关系：保留 KlineData 领域结构；把 repository 转换改为可失败收集，Baostock parser 不再“容错”伪造。
- 回滚：恢复 `unwrap_or(0.0)`/today fallback；违反红线 2.2/2.3，禁止常规回滚。
## Addendum: BR-093 R-02 同源快照与缺失字段（2026-07-17）

- 数据流：单次 MarketAnalyzer overview 同时提供三大指数、两市成交额、涨停/跌停家数；R-02 只在必填快照完整时评估并推送。
- 失败模式：JoinError、provider error、指数缺失/异常、成交额非法或计数为负均显式拒绝；尚无真实源的辅助字段保持 None 并渲染“暂无”。
- 旧模块关系：替换 `fetch_index_changes` + `fetch_market_amount_yi` + 第二次涨停请求的拼接快照，保留 MarketStage evaluator，但只喂真实可用维度。
- 回滚：恢复 `(0,0,0)` 和 0 金额 fallback；违反红线 2.1/2.2，禁止常规回滚。

## Addendum: Yahoo 行情缺失语义与公共日 K 质量门（2026-07-17）

- 数据流：Yahoo 请求只给六位 A 股代码添加交易所后缀，海外指数/外汇符号原样发送；响应字段以 `Option` 保留缺失，使用方按自身必填字段校验。公共日 K 在任何策略消费前逐行验证，再校验重复日期、交易日连续性和相邻有效值变化。
- 失败模式：客户端构造、HTTP、状态码、JSON、必需符号或必需涨跌幅缺失均返回错误；不得变成空数组、0 或“持平”。日 K 任一坏行、重复日期或未确认的绝对跳变大于 20% 使整批失败，不得 skip/dedup 后继续计算。
- 旧模块关系：采用 Yahoo 现有 provider 和 `validate_daily_kline_quality` 公共入口；删除零填充、错误符号映射和按板块放宽至 25%/40% 的规则。已显式登记的除权/IPO 元数据视为人工确认入口，但仍保留审计日志。
- 回滚：恢复 `unwrap_or(0.0)`、网络错误空数组或坏行过滤会重新违反红线 2.2/2.3，禁止常规回滚。

## Addendum: BR-094 Agent 工具事实完整性（2026-07-17）

- 数据流：每个 Agent tool 只在真实源成功并形成有效结果时返回 `Ok(data)`；Toolbelt 的错误作为不可用 observation 写审计，但不进入 `ContextManager.facts`。后续成功可写入 fact，后续失败必须移除同名旧 fact，防止使用过期事实。
- 失败模式：HTTP、JSON、空结果、字段缺失和计算失败返回 `Err`；不得包装成错误 JSON 的成功字符串。Validator 对尚未调用的工具不作结论，但已存在的财务/研报 fact 缺少必填结构时返回 `MissingField`。no-op validator 从生产 API 删除。
- 旧模块关系：采用现有 `ToolResult` 数据可用性清单和 AgentLogDao；不再依赖扫描字符串中的 `"error"` 作为正常失败协议，但保留兼容检测以防第三方工具违反契约。
- 回滚：恢复 `Ok({"error":...})` 或把错误字符串存入 facts 会重新污染 LLM 决策依据，违反红线 2.1/2.7/2.8，禁止常规回滚。
- 决策日志 `agent_scratchpad` 是最终 Agent 决策依据的一部分，必须 append-only：数据库初始化必须创建拒绝 UPDATE/DELETE 的触发器，触发器创建失败阻断初始化；既有记录不做自动清理，保留期不得短于五年。写失败继续由 `AgentLogDao` 向决策循环传播。

## Addendum: BR-095 竞价影子预测安全收口（2026-07-17）

- 数据流：`run_auction_agent` 只消费调用方传入的真实 `AuctionResult`；先验证配置和每行字段，再分类异常，拒绝疑似虚假申报，最后才写 prediction_tracker。目标日期由交易日历计算。
- 失败模式：字段缺失/非有限/越界、虚假申报、DB 未初始化、写入失败和样本统计查询失败均进入 report.errors 并留下具体原因；不得用恒真 veto 或统计 0 继续。当前仓库没有能提供匹配量与撤单证据的真实 source，因此不得增加伪造生产调度或宣称该扫描功能已上线。
- 旧模块关系：删除与订单安全无关的 `veto_check_auction_anomaly` stub；保留纯处理入口供未来真实竞价源调用。ActionGate/订单 2.6 不适用于只写影子预测，未来若升级为订单必须另走统一订单安全门 BR-084。
- 回滚：恢复恒真 veto、自然日 `+1` 或统计错误按 0 处理会违反 2.3/2.8，禁止常规回滚。

## Addendum: BR-096 机会评分阈值单一事实源与 Gate C（2026-07-17）

- 数据流：`config/strategy.toml` 根级 `opportunity_push_threshold` 是生产推送门的唯一配置事实源，当前值 75；`evaluate_hit_for_push` 将该值同时传入评分函数和最终比较，禁止评分层再硬编码 60。总分值域为 0..=100，数据不足或 winrate 缺失/非正时的专用封顶恒为 `threshold - 1`。
- 边界证明：设推送门为 `T`，配置校验要求 `1 <= T <= 100`；总分封顶 `M=100`，因此 AGENTS 2.9 的 `T > M` 会被 CI 拒绝；数据不足封顶 `C=T-1`，因此 `score <= C < T`，不可能进入实时推送。
- 机器合同：`config/design_contracts.toml` 登记阈值字段、总分边界与数据不足关系；`check_design_contradiction.sh` 精确读取合同、配置和 Rust 常量，任一侧缺失/无法解析都 fail closed，不再从邻近注释或任意数字猜测。
- 旧模块关系：保留 `compute_dual_score` 两参兼容入口，其使用登记默认阈值 75；生产推送路径使用显式阈值入口。删除未被 `MonitorConfig` 反序列化的 `[push].event_risk_score_threshold=60` 伪配置和默认开启的灰度旁路。
- CI 门禁：主 CI 必须运行 fmt、strict clippy 和全目标/全特性测试；coverage 不得用 60% 代替 AGENTS 的全局 80% / 核心 95%；合规主入口不得用 `|| true` 软化已列出的子检查。
- 回滚：恢复三套阈值、默认灰度封顶 70 或任意 grep 的合规脚本会重新引入 BR-014/BR-096 矛盾，禁止常规回滚。

## Addendum: BR-097 实时个股行情完整性与缺失语义（2026-07-18）

- 数据流：Eastmoney/Sina 的实时个股响应先形成统一 `TopStock`。代码、名称、现价、涨跌幅和来源时间作为整批必填字段校验；成交量、量比和主力净流作为可选辅助字段保留 `None`，由具体消费者声明是否需要。
- 失败模式：传输、协议、必填字段、非有限值、非正价格、绝对涨跌幅超过 20% 或过期使整批失败。需要量比/主力流的排名、突破检测和做 T 扫描只消费两个字段均存在的行；通知展示缺失为“暂无”。持仓净值快照要求全部持仓取得有效实时价，任一缺失即拒绝整次快照，禁止成本价或 0 回退。
- 旧模块关系：采用现有 Eastmoney→Sina 的真实源回退顺序和 `Detector`，但回退仅发生在上游显式失败后；删除解析阶段的当前时间伪造、辅助字段补 0、空映射继续打分和持仓成本价替代实时价。未接入辅助字段的 Sina 行仍可用于只依赖价格的展示，但不得进入依赖资金面的计算。
- 排序与限制：涉及 `volume_ratio`、`main_net_yi` 的 filter/sort/top-N 只能在字段存在的子集执行，并记录被排除数量；最大候选数和最终 Top-N 沿用既有配置/常量，不改变阈值。
- 回滚：恢复 `unwrap_or(0.0)`、空报价表继续筛选、辅助字段补 0 或成本价快照会违反红线 2.2/2.3/2.4/2.10，禁止常规回滚。

## Addendum: BR-098 pushed_stocks 真实策略评分与时效门（2026-07-18）

- 数据流：`pushed_stocks` 行解析真实推送时间、价格和指标 JSON，按 `push_kind` 选择唯一对应的 v16.4 `Strategy`，把策略输出交给统一的 6 分决策门；盘后只复用 Momentum 策略且要求策略输出至少 8 分。
- 失败模式：时间无法解析/本地时区歧义、未来时间、价格非正/非有限、JSON 非对象、策略必需指标缺失/非有限均返回明确错误或拒绝该行。盘中时效使用本轮真实 `now`，禁止把候选自身时间同时当成当前时间。数据失败不写 `consumed_at`，不得制造固定评分。
- 旧模块关系：采用 `strategy::v16_4` 的 8 个实现作为唯一评分实现；`decision::layers` 保留纯计算接口，但生产不得继续使用 v16.3 的 `match label -> 固定分数`。各 push recorder 必须写入对应策略实际需要的指标。
- 边界：盘中 `age <= 1h` 且 `score >= 6` 才允许进入统一订单安全门；盘后 Momentum 不采用盘中一小时时效，但仍要求当日查询、完整指标、`score >= 8`。所有阈值沿用现有登记值，本批不修改数值。
- 回滚：恢复固定评分、坏指标补零或 `now=push_time` 会重新绕过数据与时效门，违反红线 2.3/2.4/2.10，禁止常规回滚。

## Addendum: BR-099 候选多源证据与热度排序真实性（2026-07-18）

- 数据流：产业链与四个 P5 文件分别贡献自己的 `CandidateSource`；合并后按代码与来源去重，再用同一批完整实时行情填入价格、涨跌幅和主力净流，计算可选 heat score。证据档位、真实来源数、heat score、代码依次决定排序。
- 失败模式：P5 文件不存在表示该源本轮无数据并记录提示；文件存在但读取、逐行 JSON、六位代码或非空名称不合法时整源失败。实时行情批次不完整时 D-01/P-03 拒绝本轮。禁止把一个产业链候选复制成四种来源，也禁止用涨幅或 0 冒充主力热度。
- 缺失语义：`current_price`、`change_pct`、`heat_score` 使用 `Option`；展示缺失为“暂无”，硬门、价格区间、虚拟单与候选触发只消费具备所需真实值的候选。
- 排序：档位优先，其次真实来源数，再次 `Some(heat_score)` 降序，最后代码升序；热度不改变“是否推送”门，只影响已有合格候选顺序。
- P-03 量能分档仅使用实时量比：`<1.0=Weak`、`1.0..<3.0=Mid`、`>=3.0=Strong`；量比缺失则不生成触发。未独立取得的新闻、K 线和盘口证据统一标记 `Missing`，不从候选来源名称推断证据强度。
- 回滚：恢复伪多源复制、`change_pct` 冒充热度或零值渲染会违反红线 2.1/2.2/2.10，禁止常规回滚。

## Addendum: BR-100 P-04 真实回报与候选转正门（2026-07-18）

- P-04 数据流：从 `paper_trades` 查询当地日记录，按 ID 升序逐行校验并组装回报；无记录是正常 no-data，数据库错误或坏行是整批失败。删除 `count=3` 与固定“虚拟仓/NotFilled”生成。
- 渲染边界：`Filled` 的成交价、数量和主理由均为必填；`NotFilled` 的未成交原因为必填。通用渲染器即使被其他调用方误用，也只能显示“— 缺失”，不得渲染成 0 股或空原因。
- 候选转正：环境变量只表示人工开关，不是性能证明。生产 P-03 必须取得可审计 `PromotionEvidence`并通过既有的 30 样本/分层胜率门；未接入证据存储时显式返回 unavailable，继续 Shadow。
- 回滚：恢复占位推送或仅开关转正会重新违反 2.1/2.2/2.8/2.10，禁止常规回滚。

## Addendum: BR-101 主线/板块库严格读与 P-01 真实快照（2026-07-18）

- 数据流：`chain_daily` 最新簇提供最多 3 个主题和每主题头股，`board_rotation_daily` 按已登记强度顺序提供最多 3 条真实新闻，其 stocks JSON 提供同次归因已持久化的真实股名。P-01 不在 09:00 依赖未开盘实时价，但要求主线头股能在 rotation 证据中找到名称。
- 失败模式：DB 连接/查询错误、stocks JSON 损坏、代码/概念/新闻缺失或行情不完整均返回 `Err`；表为空返回空快照并不推送。不使用 `catch_unwind`、持仓名字表、代码或空数组伪装成完整快照。
- 旧模块关系：保留 `PreopenNewsHotParams` 渲染协议；新增严格 DAO 方法并迁移生产调用，旧容错 `Vec` 方法仅留给历史兼容边界且必须记录错误。
- 回滚：恢复空 news、code-as-name 或 DB 错误当 no-data 会重新违反 2.1/2.2/2.8/2.10，禁止常规回滚。

## Addendum: BR-102 A-10 催化复盘真实快照（2026-07-18）

- 数据流：最新主线簇的 concept/stocks/continuation_count 与同期板块联动 code-name 证据合并为一个拥有型 `CatalystReviewSnapshot`；前 3 只为已启动，随后 3 只为待启动。
- 失败模式：主线或 rotation 查询错误、JSON 损坏、concept 为空、任一入选 code 缺真名均返回 `Err`。表真空时返回空快照不推送。
- 缺失语义：当前没有可证的统一催化强度分，因此 `score=None`；不从簇数量反推伪分数。theme 直接使用 concept，不再写固定“主线题材”。
- 回滚：恢复簇数伪分、固定 theme 或 code-as-name 会违反 2.1/2.2/2.10，禁止常规回滚。

## Addendum: BR-103 纸面绩效比率缺失语义（2026-07-18）

- 数据流：读取截至结算日的全部 Filled 纸面成交，按 `(ts,id)` 升序、代码分组做 FIFO 配对，仅把结算日完成卖出的已实现 PnL 放入当日序列。Sharpe 使用全样本标准差，Sortino 使用下行偏差，胜率和最大回撤只在有完整交易时计算。Information Ratio 等待真实基准序列。
- 缺失语义：无样本、零方差、无下行样本或无基准都是 `None/NULL`，不是 0 或“满分”。表结构与 Rust 模型统一使用 nullable 字段。
- 旧模块关系：保留既有 PnL 口径与日结算调度；删除 Sortino=10 和 IR=Sharpe 两个伪实现。旧表需通过幂等重建/迁移获得 nullable 约束。
- 回滚：恢复伪满分、复制 IR 或缺失补 0 会违反 2.2/2.8，禁止常规回滚。

## Addendum: BR-104 虚拟观察快照与次日复盘失败边界（2026-07-18）

- 数据流：监控产生的虚拟观察记录先通过字段和环境隔离校验，再与当地日快照按 code 合并。日快照与 latest 都采用同目录临时文件写入、刷盘和原子替换。次日复盘只读取最近一个严格早于今天的日期快照，并按该快照日期逐票获取真实 T+1 收盘价。
- 失败模式：既有日快照或选中的历史快照损坏、目录遍历、读写、序列化、抓取器、K 线请求或 `spawn_blocking` 失败均向调用方返回 `Err` 并记录；不存在历史快照和真实 K 线尚未覆盖 T+1 分别表示 no-data/unavailable，不转换为零或空批成功。
- 幂等与顺序：同一 code 同一 entry_date 最多一条观察，输入后出现的同 code 记录覆盖前者；日文件是历史事实，latest 是可重建索引。文件失败后的安全重试只更新同 code 观察，不制造重复记录。
- 旧模块关系：保留现有 JSON 格式和 T+1 文本渲染；虚拟观察没有订单提交、成交回报或资金冻结证据，因此删除向 `trades` 写入 buy fill 的旧旁路。真正的手工/策略虚拟成交仍必须走统一订单安全和审计门，不能由观察快照代替。
- 回滚：恢复 JSON 损坏当空集合、忽略 DB/任务错误或直接覆盖文件会违反 2.1/2.2/2.5/2.7/2.10，禁止常规回滚。

## Addendum: BR-105 监控标的池与复盘批次完整性（2026-07-18）

- 数据流：portfolio 严格读取持仓和显式配置的自选，Scanner 在一个结果中构建 L1/L2 目标并按 code 去重；主监控和 NewsMonitor 只在该批次成功后开始消费。盘后 R-03 对入选标的逐票取得质检后的 K 线，R-08 使用一次成功的公告批次和同批完整持仓实时价。
- 失败模式：DB/自选解析/名称证据、fetcher、K 线、公告 HTTP、HTTP client 或后台任务任一失败均记录具体错误并拒绝对应批次。长期监控入口退避后重试，手工 review 返回非零错误；不得将失败改写成空持仓、空公告、空排名或“无事件”。
- 缺失语义：成功返回的空持仓、自选或公告才表示真实 no-data。持仓没有公告时可以展示持有状态，但只有存在有效实时价和正成本时才计算浮盈；否则该持仓事件标记“实时价不可用”或拒绝，不用成本价制造 0% 收益。
- 排序与限制：Scanner 保留 L1 持仓优先、L2 自选随后并按首次出现去重；R-03 沿用最多 20 只，R-08 沿用公告 Top3/持仓 Top3，未改变数值阈值。
- 旧模块关系：采用 portfolio、DataFetcherManager、announcement provider 与 NewsRanker；删除 Scanner 内的第二套容错 DAO、`股票{code}` 名称和 main 中的 `unwrap_or_default` 旁路。
- 回滚：恢复数据源错误当空集合会违反 2.1/2.2/2.4/2.10，禁止常规回滚。

## Addendum: BR-106 板块行情、换手率与连板证据（2026-07-18）

- 数据流：东方财富板块榜和板块成份响应先通过严格解析，所有请求字段完整后才形成 `ConceptBoard`/`BoardStock`。换手率榜从已验证成份股去重后按 `turnover` 降序取 Top10；连板批查使用公共日 K 质检后的“最新在前”序列，跳过今日并检查昨日、前日是否连续涨停。
- 失败模式：缺 `data.diff`、坏代码/名称、字段缺失、非有限/越界、HTTP/fetcher/K 线/时效/任务错误均拒绝整批。源成功且 `diff=[]` 才是 no-data。不得把失败改写为 0、空榜卡片或默认首板。
- 真实性边界：板块涨幅、资金流不能推导个股涨停家数或连板高度，因此删除盘中 synthetic R-03；真实 R-03 继续由涨停池和历史 K 线入口提供。成份接口没有个股主力净流字段，换手率模板将该字段保留为 `None/暂无`。
- 排序与限制：板块榜沿用调用方 top_n；换手候选取前 10 个板块、每板块最多 30 个成份股，按 code 去重后 `turnover desc, code asc`，最终 Top10；独立计时器 600 秒。连板批查沿用最多 40 只。
- 旧模块关系：采用现有 sector_monitor、公共 K 线 fallback/QC 和 TurnoverTop 模板；删除 `limit_up_estimate` 与复用 FundInflow 计时器的死路径。
- 回滚：恢复字段补 0、假涨停数或资金流冒充换手率会违反 2.1/2.2/2.3/2.10，禁止常规回滚。

## Addendum: BR-107 手工虚拟成交旁路关闭（2026-07-18）

- 现状判定：旧 `--buy/--sell` 只解析用户提供的价格/数量并直接插入通用 `trades`，把命令输入当作成交事实；名称失败还回退成 code。它没有取得统一订单门要求的行情、账户、限价、模式、可卖量和确认，也没有写 BR-086 订单审计。
- 安全决策：在具备完整的手工 paper order command（显式证据字段和审计）前，CLI 对这两个参数 fail closed 非零退出。不得部分复用旧写函数，也不得把 research observation 迁回成交表。
- 旧模块关系：保留 `paper_trade::simulate` 作为唯一虚拟成交持久化入口；保留 `trades` 供已有真实/历史交易复盘读取，但删除 portfolio 中无安全上下文的新增写入口。
- 失败模式：任何 `--buy` 或 `--sell` 形式都显示迁移说明并退出 2；不解析默认数量、不查询名称、不触发报价或数据库成交写入。
- 回滚：恢复直接写 `trades` 会绕过 2.5/2.6/2.7/2.8，禁止常规回滚。

## Addendum: BR-108 真实 DataMode 与 banner 可用性（2026-07-18）

- 数据流：进程内 capability tracker 只接受真实数据边界在成功完成协议和质量校验后写入的 `Instant`；DataMode 每轮从 Quote/Kline/MoneyFlow/News/OrderBook 的年龄快照评估。AccountMode 使用真实 portfolio metrics 和已持久化前态计算；首次评估把 `prev=new` 作为初始化审计写入。两者合并成功后才发布 banner。
- 失败模式：tracker/banner 锁、账户 metrics、前态 DB、模式标签或推送/审计失败均返回明确错误；banner 保持 unavailable 或上一份已验证值，不生成 Normal/Full/0 值。调用方拿不到 banner 时跳过需要治理语境的 dispatcher。
- 时效边界：沿用 DataMode 已登记阈值，关键能力 120 秒，OrderBook 600 秒；Quote 缺失/过期为 Unsafe，其余关键能力缺失/过期为 Degraded，OrderBook 缺失只做缺失标记。实时订单行情仍使用更严格 5 秒门，本规则不放宽 2.4。
- 旧模块关系：采用现有 `monitor::data_mode::evaluate`、account mode evaluator 与 `LATEST_BANNER`；删除三处 `Capability::ALL -> fresh(30)` 和 `current_banner` 的默认对象。
- 回滚：恢复固定 fresh 或 Normal/Full 默认值会违反 2.1/2.2/2.4/2.8，禁止常规回滚。

## Addendum: BR-109 持仓缺失值与 T-16 真值链（2026-07-18）

- `Position.hard_stop` 改为可空语义；当前 `stock_position` 没有该字段时保持 `None`，展示层写“未设”，需要止损价才能形成建议的路径拒绝该票。
- `find_position`、ST 批查、交易和净值反序列化返回 `Result`，数据库或坏行不能被降级为空/固定日期。
- T-16 从严格 ST 持仓批次出发，逐票取 `broker::execution_quote` 的 5 秒内真实价；5%→10%重算是显式纯函数并验证有限正值。任一环节失败只留下错误日志，不产生推送。
- 失败模式：无 broker、陈旧/非法行情、DB 未初始化、无 banner、数量溢出或规则参数非法均 fail closed；不回退成本价、0 或估算价。
- 回滚：反向恢复 BR-109 涉及的组合/展示/T-16 文件；不更改数据库 schema 或交易阈值配置。

## Addendum: BR-110 盘后 R 系列来源完整性（2026-07-18）

- 数据流：R-03 复用 `load_review_limit_chain_stocks`，从持仓/自选的真实日 K 经质量门得到逐股连板事实，再交给 `limit_chain_review::aggregate`；R-08 只有在公告批次和持仓批次均成功后才组装事件日历。盘后批次汇总每个子报告的真实结果。
- 失败模式：数据库、公告、持仓、日 K、后台任务或字段校验失败均拒绝对应报告并写明来源。龙虎榜、信号复盘和失败归因当前缺少可证的生产数据合同，启动/调用时显式 `disabled=no_producer` 或 `unavailable`，不得用空列表、通用交易金额或固定说明伪装已完成。
- 排序与限制：R-03 继续使用已登记的产业链分组、龙头优先级与 Top20 输入限制；本批不改变数值阈值。R-08 沿用公告/持仓 Top3/Top5 展示限制，但仅在完整源批次上执行。
- 旧模块关系：采用现有公共日 K 质量门、`load_review_limit_chain_stocks`、`limit_chain_review::aggregate`、公告 provider 与严格 portfolio API；拒绝 `chain_daily.continuation_count` 多字段推断、`fetch_recent_lhb -> Vec::new()` 和 R-05/R-06 通用交易伪统计。
- 回滚：可反向恢复调用装配，但不得恢复空集合成功、推断涨停数或虚构绩效；若真实 producer 尚未接入，对应报告必须保持显式禁用。

## Addendum: BR-092 日 K provider 严格批次（2026-07-18）

- 受影响数据流：Eastmoney/Tencent/Sina/RustDX 原始响应 → provider 行解析/分页 → 公共日 K 质量门 → pipeline、机会扫描和盘后复盘。
- 失败模式：任一行字段不足或解析失败、任一分页请求失败/线程 panic、成交额由成交量和收盘价估算、来源不提供必填成交额、日期重复/断档、数值非有限或相邻涨幅不一致，均拒绝该来源整批并尝试下一独立真实源；所有源失败则调用批次失败。
- 来源选择：Eastmoney、Baostock、RustDX 只在取得真实必填字段并通过公共质量门后可成功；Tencent/Sina 日 K 接口不提供真实成交额时保持显式 unavailable，但它们的独立实时行情接口不受影响。
- 旧模块关系：复用 `fallback::fetch_kline_with_fallback` 和 `validate_daily_kline_quality`，不新增第二套质量门；删除坏行 skip、部分页成功和估算成交额语义。
- 回滚：可反向恢复解析实现；若某提供方协议发生兼容问题，只允许将该提供方标记 unavailable，禁止恢复估算/补零。

## Addendum: BR-111 外部通知成功确认（2026-07-18）

- 数据流：渠道 HTTP 响应 → 严格协议字段解析 → 投递 outcome → 权威 delivery audit → dispatcher 返回值。
- 失败模式：非 2xx、响应非 JSON、成功字段缺失/类型错误/非成功码、审计落盘失败均返回失败，不增加成功计数。
- Slack Incoming Webhook 例外使用其官方纯文本合同：只有 HTTP 2xx 且去除首尾空白后的响应体精确等于 `ok` 才算成功；空正文或其他文本均失败。
- Custom Webhook 使用本仓库的显式确认合同：HTTP 2xx 后响应必须是 JSON object，且布尔字段 `ok` 精确为 `true`；缺失、字符串值、`false` 或不可解析正文均失败。所有 Custom URL 独立计数，不得由另一个 URL 的成功掩盖失败。
- 旧模块关系：保留现有 NotificationService 和 event dispatcher；只收紧成功确认，不新增备用“成功”路径。
- 回滚：不得恢复 `unwrap_or(0)`、缺确认字段即成功或审计失败后继续报告 Pushed。

## Addendum: BR-112 机会扫描生产真值链（2026-07-18）

- 数据流：新闻、板块榜、成份股、持仓与市场上下文必须分别通过严格解析和时效门，再以一个可失败批次进入产业链机会、盘后候选和公告排序；只有整批成功才允许推送或生成研究观察。
- 失败模式：HTTP/DB、分页、坏行、后台任务或必填上下文缺失均拒绝整批。不得用空集合、固定 0、默认 `MarketContext` 或已消费全部公告 ID 伪装成功。
- 过渡状态：现有机会研究实现仍包含兼容回退，不能作为生产 producer。在其接口整体迁移为严格 `Result` 前，主监控只记录 `disabled=incomplete_source_contract`，不调用、不推送，也不从其文本生成虚拟观察；公告继续走已经完成字段校验的旧事件路径。
- 旧模块关系：保留 `opportunity`、`chain_mapper` 和 `news_ranker` 供测试与后续逐层迁移；关闭 `main.rs` 中四个生产调度入口和缺完整市场上下文的公告二次排名，不删除研究算法。
- 回滚：只有在所有源、上下文、任务和投递结果均能传播失败且具有测试证据后才能重新启用；不得恢复 failure-to-empty 或缺失补 0。

## Addendum: BR-113 推送治理与 L7 持久审计（2026-07-18）

- 数据流：已验证的共享 banner 进入 L5 治理；治理、去重和实际 sink outcome 形成 L7 记录并写入隔离 SQLite；成功记录和 BR-091 hash-chain 都完成后，调用方才可看到 `Pushed`。
- 失败模式：banner 未初始化/锁中毒、SQLite 打开或写入失败、枚举/时间/JSON 坏行和统计查询失败均显式传播。不得以 `BannerCtx::default()`、内存库、no-op、当前时间、默认枚举、0 或空集合代替失败。
- dispatcher `RwLock` 或 store `Mutex` 一旦 poisoned，不得调用 `into_inner()` 恢复潜在不一致数据；治理、commit/rollback 和 L7 记录全部返回错误，最终 outcome 不得为 `Pushed`。
- 环境隔离：测试构建或 `STOCK_ENV_MODE=test` 使用 `data/test/push_analytics.db`；生产只使用 `data/push_analytics.db`。两者都必须可持久化，测试可通过显式内存构造器单测存储实现，但运行时不得自动回退。
- 顺序：sink 失败仍写真实失败 outcome；sink 成功后若权威审计失败，保留去重防止重复外发，但返回 `SinkError` 并出声。治理上下文不可用时在调用 sink 前拒绝。
- 回滚：不得恢复内存/no-op fallback、日志后成功或缺失 banner 默认 Normal/Full。

## Addendum: BR-114 产业链分析来源完整批次（2026-07-18）

- 数据流：涨停池代码先读取严格概念缓存，缺项逐股调用板块工具并全部成功落库；完整概念图聚类后，主线批次事务写入，再读取完整板块代码/成份与龙虎榜证据形成报告。
- 失败模式：数据库、连接、工具、HTTP client、所有 push2 主机、分页、JSON、字段、非有限数值或事务失败均返回错误；不得把部分页、坏行或失败源当空集合继续。
- 排序与限制：只在完整成份批次上排除已涨停、ST、北交所及涨幅不在 `-3%..=7%` 的股票，按涨幅降序/代码升序稳定取 Top8；本批不改阈值。
- 持久化：`save_stock_concepts`、`save_chain_clusters`、`save_event_seen`、`save_board_rotations` 和清理接口都返回结果，禁止日志后宣称已保存。主线同日同概念仍按既有唯一键覆盖，但一个批次使用事务。
- 回滚：不得恢复 error-to-empty、逐行 skip 或保存失败后继续生成成功报告。

## Addendum: BR-115 补充行情与通知协议失败传播（2026-07-18）

- 数据流：财务、资金流与分时响应经真实 provider 的协议解析和字段校验后，以 `Result` 进入进程缓存；只有成功值可以写缓存。Agent 工具和快速分析流水线必须显式处理每一路结果。外部通知只有在响应体读取、JSON 解析和渠道成功字段全部确认后才可报告成功。
- 失败模式：全部财务源失败、后台线程 panic、资金流或分时传输/解析失败、通知响应体不可读、成功字段缺失或类型错误均返回错误。禁止缓存 `Financials::default()`、空资金流、空分时，禁止将线程 panic 改写为默认值，也禁止仅凭 HTTP 2xx 宣称投递成功。
- 可选证据：补充源不影响已通过 BR-092 质量门的日 K 主序列；调用方可以在展示中明确标注某个补充证据 unavailable，但不得把 unavailable 数值用于评分、筛选或交易。
- 旧模块关系：保留 `DataFetchService` 的 TTL/单飞缓存、现有多财务源顺序和通知协议；只把错误合同贯穿到消费者，不新增 mock 或伪备用源。
- 回滚：不得恢复 `unwrap_or_default`、空对象缓存、线程 panic 默认化或通知响应体缺失即成功。

## Addendum: 独立复核阻塞项闭环（BR-051/084/086/097/108/111/112/115，2026-07-18）

- 行情时间：实时行情模型必须携带 provider 响应中的来源时间；无法从协议取得来源时间的 provider 不得供 5 秒订单门使用。订单报价和完整行情批次都以来源时间而非本机请求完成时间判定时效。
- 订单安全：委托请求价与成交行情分别校验；涨跌停门使用请求价。60 秒 business order ID 幂等必须由数据库唯一事实支持，不能只依赖进程内 mutex。买卖持仓证据必须携带并验证 30 秒内更新时间。
- 账户模式：任何环境变量只能请求通知展示，不得把 Frozen/ReduceOnly 改写为 Normal，也不得影响交易授权矩阵。
- CLI 与生产 producer：`--review` 是独立终止命令；只调用已验证 banner 与严格 dispatcher，不能落入常驻监控或旧内联样本。BR-112 禁用的候选/NewsAI producer 在完整 `Result` 真值链完成前不得继续调用、推送或推进去重状态。
- `--review` 的可达执行路由固定为 `StrictDispatchers`：使用共享真实 banner，并调用逐项记录 outcome 的 `dispatch_post_session_review`。旧 `run_review_only_inner` 只允许 TEST_CODE E2E 夹具入口调用，生产 `--review` 和普通常驻入口都不得可达。
- 通知协议：Server酱、Telegram、飞书卡片/文本与其他渠道一致，只有 HTTP 成功、响应体可读、JSON 可解析且类型正确的显式成功字段成立时才返回成功。
- 财务与一致预期：报告日期、必填身份和所有存在的数值字段必须严格解析、有限且在领域范围；坏字段使整源失败，禁止部分行/部分字段成功进入缓存或更新时间。
- 测试隔离：`cfg(test)` 和 `STOCK_ENV_MODE=test` 的推送日志、delivery audit、analytics 与事件 JSONL 全部写 `data/test/**`；测试事件代码使用 `TEST_CODE*`。生产目录不得由测试创建或追加。
- 订单审计链：`order_audit`、SHA-256 链证据与接受的持仓/paper 变更同事务；启动与每次追加前验证完整链，只允许对“审计存在且链完全为空”的旧库执行一次性回填，部分链、坏链接或坏 hash 一律拒绝启动/写入。失败路径测试必须覆盖回填、部分链、坏 hash，以及链写失败时审计与持仓/paper 全回滚。
- 审计链回滚约束：down migration 保留审计表、链表和不可变触发器。不得在订单或 paper writer 仍启用时回退到不识别 `order_audit_chain` 的旧应用；回退前必须冻结全部交易/paper 写入，或先部署能够同步追加并验证同一链协议的兼容 writer。否则旧 writer 会制造部分链并导致再次升级时失败关闭。
- 失败与回滚：任一上述证据不可用均返回错误/拒绝，不推进成功计时器、去重或审计成功状态。不得恢复本地当前时间、内存幂等、默认 Normal、零值上下文、字符串 contains 成功判断或生产目录测试写入。

## Addendum: BR-103 当日净值时效与真实账户边界（2026-07-18）

- 数据流：复盘日期 → 真实账户现金与持仓市值快照 → 会计恒等式校验 → 当日 ledger → 连续历史净值 → 指标与报告。读取历史曲线时必须验证最后一条 ledger 等于要求的报告交易日；跨午夜或缺当日行均为 stale。
- 过渡状态：当前 `snapshot_portfolio_value` 没有真实账户现金来源并显式 unavailable，因此生产 `--review` 与收盘复盘不得读取旧 ledger 后继续生成数值净值报告；整个相关报告 fail-closed，直到真实账户快照完成并持久化当日事实。
- 失败模式：快照 unavailable/失败、当日 ledger 缺失、日期断档、会计不平、坏数值均传播并阻止推送；不得 `let _ =`、只记日志后继续，或把历史最后一条当作当日事实。
- 旧模块关系：保留 `portfolio::load_ledger` 与 nullable `EquityStats`；在公共读取边界增加 required-as-of 校验，不复制第二套指标。首日 `daily_pnl` 的 nullable 迁移在真实快照接入时实施，本批不伪造首日数值。
- 回滚：不得恢复陈旧 ledger 推送、现金补零或快照失败后继续报告。

### 真实账户快照的可空持久化（Gate D Task 7）

- 所有者：新增 `database::account_snapshot` 深模块及 `real_account_snapshot` 表。旧 `ledger` 继续表示已具备完整逐日盈亏口径的绩效曲线；人工截图或未来券商回报先写账户事实表，缺失的当日盈亏不得为了兼容旧 ledger 而填 0。
- 数据合同：总资产、证券市值、可用现金、来源提供方、账户类型、币种、所有权声明、账户引用状态、当日盈亏状态、来源采集时间、人工/适配器观察时间与 SHA-256 证据引用必填；账号本身不要求提供，可取现金、持仓盈亏、当日盈亏、仓位比例、账户安全模式和脱敏账户引用均可空。金额必须有限，总资产/市值/现金非负且 `总资产 = 证券市值 + 可用现金` 的绝对误差不超过 0.01 元；可取现金不得超过可用现金；仓位比例存在时必须位于 0..=100 且与资产比例误差不超过 0.1 个百分点。
- 时间与行动边界：保存历史、用户确认的真实快照时保留原始采集/观察时间，即使已超过 30 秒也允许作为审计事实入库；只有 `is_fresh_for_action(now)` 可授权实时消费者，年龄必须位于 0..=30 秒。未来时间、不可解析时间和观察早于来源采集均拒绝。
- 幂等与审计：证据哈希唯一；重复导入返回原 ID，不修改已存事实。表由 `BEFORE UPDATE/DELETE` 触发器保护，来源/时间/证据引用随行保留。数据库迁移只增表与索引；回滚应用代码时保留历史表和触发器，禁止删除真实账户事实。
- 本地导入：通用 one-shot 命令只读取用户指定的忽略 JSON 与数据库路径，调用同一验证/存储接口；不得复制原图、打印账户值或把证据文件加入 Git。导入前保存本地数据库备份，导入后仅输出记录 ID、是否幂等及校验布尔值。

## Addendum: BR-116 周期任务确认后提交（2026-07-18）

- 数据流：到期计时器触发真实数据读取，批次校验通过后进入推送治理；只有真实空批次、治理明确去重或 sink 已确认投递才提交本轮计时器。获取、后台任务、治理拒绝或 sink 失败均保留到期状态，并在下一监控轮重试。
- 状态提交：持仓健康哈希采用检查/提交两阶段，只有确认投递后写入新哈希；相同哈希属于明确去重并推进五分钟计时器。做 T 扫描使用独立三十秒计时器，避免与健康汇总互相重置。
- 失败模式：任何失败必须记录具体阶段和返回结果，不得丢弃 `PushOutcome` 或在异步工作开始前推进 timer。真实空结果不是失败，可以推进 timer，防止无数据时忙循环。
- 回滚：不得恢复提前推进 timer、检查时覆盖健康哈希或健康/做 T 共用计时器。

## Addendum: BR-117 新闻阶段证据最小集（2026-07-18）

- 判定顺序保持业务优先级：当日下跌且主力流出为 Fade；当日极端上涨且涨停数达门槛为 Climax；当日上涨但资金加速度显著转弱为 Divergence；当日非正、零涨停且主力不流入为 Cold。上述结论不消费三日历史。
- 只有累计高潮、Ferment 和 Start 分支读取三日板块历史；历史必须通过 BR-092 的日期唯一、连续和字段质量门。历史损坏或不可读时这些分支返回 Unknown 并记录错误，禁止用 0 代替。
- 该拆分避免一个与当前分支无关的历史文件故障覆盖已具备完整当日证据的结论，同时不放宽任何需要历史的判定。

## Addendum: SQLite 启动 WAL 顺序与锁错误闭环（2026-07-18）

- 数据流：`DatabaseManager::init(path)` 先用单一 bootstrap 连接请求数据库级 journal mode 切换为 WAL，并读取 SQLite 返回的实际模式；只有返回值严格为 `wal` 才关闭 bootstrap 连接并创建 10 连接 r2d2 pool。初始化代码同时持有并逐条配置全部 10 个连接，依次设置 `busy_timeout`、connection-local `synchronous` 与 `wal_autocheckpoint`；全部成功后才由其中一条连接执行既有 migrations/schema 初始化。运行期 `get_conn` 在返回连接前重复这一幂等配置并直接传播错误。
- 失败模式：bootstrap 建连、WAL 请求、返回模式非 WAL、10 条初始池连接中的任一 PRAGMA、迁移或 schema 初始化任一步失败都必须使 `init` 返回错误；运行期连接配置失败必须使 `get_conn` 返回错误。禁止把 PRAGMA 放在 r2d2 `CustomizeConnection::on_acquire` 中，因为 r2d2 会指数退避重试并可能掩盖首次失败。monitor 的目录创建和数据库初始化入口都记录具体错误并以 2 退出；不得因 PRAGMA SQL 执行成功就假定 WAL 已生效。
- 测试 seam：通过编译后的 monitor 进程对全新隔离 SQLite 文件执行 `--test --review`。缺少当日真实 ledger 仍应以 2 fail-closed，但完整输出不得包含 `database is locked`。另以 SQLite `:memory:` 作为公开进程级非 WAL 反例，必须显示实际返回模式并以 2 退出。测试不调用私有 helper，也不暴露新的调用接口。
- 旧模块关系：保留 `DatabaseManager::init`、`get_conn`、现有 migrations、10 连接 pool 与每连接 timeout 设置；数据库级 WAL 切换位于串行 bootstrap，连接级 PRAGMA 位于数据库管理模块自身，不依赖 r2d2 隐式重试定制器。被删除的孤立 `src/broker/ib.rs` 与 `src/app/context.rs` 不参与数据库初始化。
- 回滚：`git revert` 本批并重跑 fresh-DB 进程测试；不执行 schema down migration、不删除数据库，也不得用忽略日志或字符串过滤掩盖锁错误。

## Addendum: BR-118 旧资金流文本数值边界（2026-07-18）

- 数据流：原始 `MoneyFlowSummary` 保持首选；只有旧调用方仅提供 `format_for_prompt` Markdown 时，评分兼容路径才读取“近5日”字段。解析从 ASCII/中文冒号之后开始，到首个“亿”之前结束，整段必须能严格解析为有限 `f64`。
- 失败模式：标签中的数字 5、其他段落数字、缺单位、空值、NaN/Inf、尾随说明或非法字符都不得混入数值。解析失败保持资金面中性分，并由上游缺失语义处理，不抽取任意数字、不补 0。
- 测试 seam：通过公开 `compute(ScoreInputs, KlineData)` 输入生产格式 `主力资金近5日: +2.50亿`，独立断言资金流档位 90 加量比 5 后等于 95；同时覆盖中文冒号和非法输入。
- 回滚：只可整体回退 BR-118 代码/测试；不得恢复把标签数字送入浮点解析的旧逻辑。

## Addendum: BR-119 估值与一致预期完整批次（2026-07-18）

- 数据流：HTTP 响应完成 JSON 解码后进入不含网络副作用的批次解析器；解析器先验证数组、日期顺序与每个存在字段，再一次性计算估值分位或研报聚合。只有完整成功对象可返回给缓存和评分。
- 估值边界：交易日必须严格降序、唯一且连续。正且有限的 PE/PB 才进入统计；负值代表亏损或负净资产，只排除该值，不补 0。少于 30 个有效样本时对应分位保持 `None`。
- 一致预期边界：报告日期必须位于本次 180 日窗口并严格非升序；机构、标题、评级均非空，目标价必须为正且下限不高于上限。整批至少有一个真实 EPS 预测。
- 失败模式：缺数组、空数组、非法日期、重复/缺口、乱序、非有限/非法数值或矛盾目标价都返回错误，不跳行、不保留部分聚合、不以默认对象继续。
- 测试 seam：本地 `serde_json::Value` 协议夹具直接调用同一批次解析函数，覆盖完整成功批次及各失败边界；不访问真实外网、不提供生产账户或 API 凭据。
- 回滚：整体回退 BR-119 解析器与测试；不得恢复坏字段转缺失、日期错误继续统计或 EPS 空批次成功。

## Addendum: BR-120 行业对标完整批次（2026-07-18）

- 数据流：行业列表每一页先通过严格本地解析器形成 name-code 映射；目标板块的完整成份响应再解析为显式可空的 PE/PB/ROE/增速记录；最后一个纯聚合器计算目标股字段、中位数与百分位。
- 完整性：协议缺 `data.diff`、空或类型错误的 code/name、冲突映射、重复成份代码、存在但非法/非有限的数值或空成份批次都返回错误。缺失与空字符串表示 `None`，不得以 `NaN`、0 或跳行代表缺失。
- 统计边界：正 PE/PB 才进入估值分布；有限 ROE/净利增速可保留负值。目标股缺对应指标时该百分位保持 `None`，不借用同业值填充。
- 测试 seam：本地 JSON 页与成份夹具调用生产同一严格解析和聚合函数；覆盖完整成功、缺数组、坏行、冲突/重复及目标字段缺失，不访问外网。
- 回滚：整体回退 BR-120；不得恢复 `unwrap_or_default`、`NaN` 哨兵、逐行 skip 或失败后空基准。

## Addendum: BR-121 单股评分与基本面章节边界（2026-07-18）

- 数据流：单股分析只消费已经由 BR-092、BR-115、BR-119、BR-120 校验成功的真实 K 线和补充批次。布林/MACD、财务质量、估值分位、卖方预期与行业对标通过无 IO 的确定性函数调整趋势评分；展示函数只格式化同一批证据，不再次抓取或填充数据。
- 评分边界：布林/MACD 动作保持 `UptrendStart +12`、`BottomBuy +10`、`PreReversal +3`、`TopSell -15`；基本面各项保持既有阈值，合计限幅 `-25..=25`，最终评分限幅 `0..=100`。TopSell 和高财务风险继续执行既有买入信号降级。样本门或可选字段不满足时跳过该项，不以 0 代表缺失。
- 展示边界：行业同业少于 3 家、估值少于 30 日、财务少于 2 期或一致预期无研报时对应章节保持 `None`；其余缺字段显示 `-`。最近研报沿来源顺序最多展示 3 条，标题按字符最多保留 28 字后加省略号。
- 测试 seam：把既有评分修正和 Markdown 渲染抽为无网络副作用的模块私有函数，使用 `TEST_CODE_` 本地结构体夹具覆盖所有档位、限幅、缺失和降级路径；生产入口调用完全相同的函数。
- 回滚：整体回退 BR-121 抽取与测试；不得借回滚改变既有评分阈值、放宽样本门或恢复缺失字段补 0。

## Addendum: BR-122 大盘门控与结果投影缺失语义（2026-07-18）

- 数据流：生产适配器取得沪深300最新日线后，先验证指数涨跌；随后纯门控函数验证所有存在的个股涨跌、检查至少一半标的有证据、计算上涨/下跌广度并执行既有三态分类。指数适配器失败/空批次为 `Err`，已知个股不足为明确的 `Ok(None)`，完整批次才渲染章节并可能调整建议。
- 阈值与排序：不修改任何配置或业务阈值；普跌仍为指数 `<=-1%` 且下跌占比 `>=70%`，普涨为指数 `>=1%` 且上涨占比 `>=70%`。普跌豁免只作用于“建议减仓”，条件仍为个股收红或相对指数跑赢至少 2 个百分点；调整清单保持输入顺序。
- 缺失与坏值：指数或已存在个股涨跌非有限、绝对值超过 20% 时整批报错；缺失个股不进入分母也不补 0。`result_types` 的窄投影把没有来源证据的现价、涨跌幅、K线条数、均线排列、否决证据和评分保留为 `None`；空通知章节被过滤，但不得生成占位章节。
- 测试 seam：传入确定性指数涨跌闭包与本地 `AnalysisResult`，覆盖来源错误/空批、缺失门、坏值、三态、每个豁免分支和渲染顺序；窄投影通过 Serde 构造的 `TEST_CODE_` 结果覆盖完整与缺失事实，不调用外网。
- 回滚：整体回退 BR-122 代码与测试；不得恢复指数失败静默跳过或任何缺失字段补 0/空串/空数组。

## Addendum: BR-123 持仓产业链缺失与 upsert 语义（2026-07-18）

- 根因：`stock_position.chain_name` 的旧 SQLite 列定义带 `DEFAULT '其他'`；Diesel `Insertable` 默认把 `Option::None` 解释为使用列默认值。因此 upsert 的 `excluded.chain_name` 变成“其他”，现有 `COALESCE` 无法识别缺失并覆盖了此前已确认的产业链。
- 数据合同：`NewStockPosition.chain_name=None` 必须显式绑定 SQL `NULL`。首次保存缺失产业链时数据库保留 `NULL`；同键 upsert 缺失时保留旧明确值，只有非空且不等于“其他”的新证据才更新。历史空串/“其他”仅表示旧缺失哨兵，初始化时幂等归一为 `NULL`；静态 registry 未命中不得制造分类。
- 失败模式：测试环境继续拒绝真实代码；产业链缺失阻断 BR-085 建仓，不得按“其他”或零集中度继续。数据库写入、归一或回填失败均传播，禁止仅记录日志后宣称完成。
- 测试 seam：使用唯一 `TEST_CODE_` 与真实测试 SQLite 执行首次保存、缺失 upsert、显式证据保留、历史 ST/产业链回填和 close round trip；不调用行情网络、不写真实账户数据库。
- 旧模块与回滚：保留 `DatabaseManager`、`NewStockPosition`、静态 registry 和现有表；不新增第二套仓储。整体 `git revert` 本批并恢复数据库备份；不得只恢复“其他”默认填充而继续声称满足 2.2。

## Addendum: BR-124 持仓跟踪与分析保存失败传播（2026-07-18）

- 数据流：已经过日 K 质量门的单股结果进入持仓跟踪；跟踪读取真实持仓、产业链、30 秒账户和 5 秒行情，完成无动作/拒绝/成交状态后，再把同一票的最新有效 K 线与分析结果写数据库。两个动作函数都返回 `Result`，调用方遇错停止该票，不进入后续保存或通知。
- 完整性：持仓买入价必须正且有限，买入日期必须严格解析；空 K 线、参数代码与结果代码不一致、最新收盘非正/非有限或涨跌幅非有限均拒绝保存。T+1 锁仓、无技术买点和市场禁止开仓是已验证的无动作成功，不得与数据源/数据库/订单失败混为一类。
- 失败模式：DB 未初始化/查询/更新/写入失败、产业链或账户不可用、动态波动率非法、仓位不足一手、报价/订单/审计失败均返回错误。禁止 `warn` 后返回成功、坏日期按持有 0 天、空数组索引 panic 或保存失败后继续通知。
- 测试 seam：唯一 `TEST_CODE_` 与测试 SQLite 覆盖无效输入、无动作、持仓更新、产业链暴露和分析结果真实写入；订单成交使用 Task-11 已验证的测试报价/账户隔离边界，不访问外网或真实账户。
- 旧模块与回滚：保留 `AnalysisPipeline`、`SimulatedExecutionGateway`、`DatabaseManager` 和现有表，只收紧内部返回合同。整体回退代码/测试；不得恢复静默成功或坏日期默认化。

## Addendum: Gate D Task 13A 数据适配器公开协议闭环（2026-07-18）

- 范围：本切片只覆盖 BaoStock TCP/CSV、Sina hq/K 线和 Sina 新闻已经存在的公开构造、解码、解析与流式读取接口；不新增生产数据源、不改变 provider 选择顺序、不调用真实网络。
- 数据流：本地完整协议字节/文本 → 生产同一公开解析入口 → 已有价格、日期、连续性、来源时间和新闻身份校验 → 明确领域对象或整批 `Err`。测试用原生六位代码只作为 provider 协议解析参数，不进入订单路径；所有持久化/交易身份仍使用 `TEST_CODE_`。
- 完整性：BaoStock 同时覆盖非压缩登录响应、zlib K 线响应、分块读取/EOF/timeout/上限、CDATA 和 BR-092 K 线批次；Sina hq 覆盖 32 字段与来源时间，K 线继续因缺真实 amount 而拒绝；Sina 新闻覆盖 URL、UTF-8/GBK、来源名分支、未来/过早时间、坏身份和完整批次拒绝。
- 失败模式：短帧、坏 header/body length/zlib、缺列/字段、坏日期/数值/连续性、坏 JSON/时间/代码均必须保持显式错误；禁止为了覆盖率增加默认行情、金额、新闻字段或网络 fallback。
- 回滚：整体回退 Task 13A 测试与文档即可；生产代码预期不变。若测试暴露现有红线缺陷，先登记新 BR 和设计修正，再进入独立实现提交。

## Addendum: BR-125 Tencent/Eastmoney 日 K 完整批次校验（2026-07-18）

- 根因：两个真实 HTTP provider 的解析器逐字段转换并排序后直接返回，没有进入 `validate_kline_series_strict`。因此 JSON 结构完整不等于业务批次有效，空数组、坏价格关系、非有限值、交易日缺口/重复或相邻跳变可能绕过 2.3。
- 数据流：完整 JSON → provider 字段解析 → Tencent 按升序真实收盘计算 pct_chg / Eastmoney 保留来源 pct_chg → 统一 BR-092 批次校验 → 最新日期在前返回。校验失败整批拒绝，不改变 HTTP host/retry、复权口径或 provider 顺序。
- 测试 seam：模块内本地 JSON 同时覆盖 qfqday/day、成功排序/涨跌幅、空/缺数组、非数组行、短行、坏日期/类型/数值、OHLC、量额、涨跌幅、日期缺口/重复和 >20% 跳变。原生六位代码只作为协议路由参数，不进入订单。
- 回滚：整体回退 BR-125 代码和测试；不得只移除严格校验而保留“已满足 2.3”的声明。

## Addendum: BR-126 v16.x pushed_stocks 初始化与消费合同（2026-07-18）

- 根因：v16.1/v16.3 设计、`push_recorder` 和 `intraday_monitor` 都依赖 `pushed_stocks`，但当前 `DatabaseManager::init` 没有创建该表或三个查询索引。新数据库上的首条推送和每次盘中/盘后扫描都会显式失败，R3→R4→R5 数据流实际上不可用。
- 数据流：数据库初始化原子创建 12 字段表和三个登记索引 → `push_recorder` 拒绝空身份/来源、非有限正价格或非对象 JSON → 真实推送先入池 → 盘中按一小时窗口/50 条上限或盘后按当日 15:30/100 条上限稳定读取 → BR-098 严格解析评分 → 报价/账户/风控/成交均成功后更新消费审计字段。任何前置失败保留未消费事实并返回/记录显式错误。
- 测试隔离：RED 数据库合同先证明全新初始化缺表；GREEN 后用 SQLite 元数据验证精确字段与索引，再用唯一 `TEST_CODE_` 行验证坏数据不消费、成功消费审计和盘后防重入。测试报价与账户只存在于 `cfg(test)`，不进入生产 fallback。
- 旧模块关系：采用 v16.x 文档中的表结构、`signal::push_recorder` 写路径和 `decision::intraday_monitor` 消费路径；拒绝新增第二张队列表或内存 fallback，以免绕过审计。
- 回滚：整体回退 BR-126 DDL、测试和文档。若生产数据库已创建空表，`CREATE TABLE/INDEX IF NOT EXISTS` 的回退不删除用户数据；禁止用 `DROP TABLE` 回滚。

## Addendum: BR-098 AuctionAnomaly 可达性修正（2026-07-18）

- 根因：`Candidate::push_kind_label` 已把显式 `AuctionAnomaly` 路由到同名策略，但 `AuctionAnomalyStrategy::score` 只接受 `P-02`，导致已登记类型必然返回 `None`。这是路由/策略协议不一致，不是低分或缺数据。
- 修正：策略接受 v16.x 登记的 `AuctionAnomaly` 显式类型，同时保留原有 `P-02` 兼容入口；盘中路由继续保持 `P-02 → VolumeSurge`，不根据猜测重写已有来源类型。评分公式、5 倍量比门和所有其他策略不变。
- 验证与回滚：八种登记 push kind 均必须到达各自真实策略输出，缺字段/LLM 越界继续显式失败；整体回退这一协议兼容改动即可。

## Addendum: BR-092 数据库 K 线写前完整批次校验（2026-07-18）

- 根因：`StockRepository::find_kline` 在读取时执行严格校验，但其最终写入口 `DatabaseManager::save_kline_data` 直接 UPSERT。任何绕过具体 provider 的调用都能把负/非有限价格、坏 OHLC、量额、涨跌幅、缺口/重复或 >20% 跳变写入 `stock_daily`。
- 修正：非空 provider 批次先检查非空代码/来源，再克隆并调用统一 `validate_kline_series_strict`；只有完整批次通过才获取连接并进入事务。校验器的排序只作用于克隆，数据库按日期键 UPSERT 的结果不变；空批次继续返回 0，表达调用方确实没有提交任何行。
- 失败模式：任一坏行使写入前返回错误，旧数据不变；事务/连接错误保持显式传播。旧的可空 `save_daily_record/save_daily_batch` 是分阶段字段存储接口，不被伪装成已完成 provider 批次，严格计算读取仍由 repository 拒绝缺必填字段。
- 回滚：整体回退写前校验和测试；不得只删除校验却保留 BR-092 数据库写边界声明。

## Addendum: BR-005 RFC3339 本地日配额边界（2026-07-19）

- 根因：analytics 以 `DateTime<Local>::to_rfc3339()` 持久化真实时区，但两个日计数 SQL 使用 `date(ts)`。SQLite 会把 `+08:00` 转成 UTC 后再取日期，因此上海 00:00–07:59 的成功投递被归到前一天，日配额被低估。
- 修正：记录格式不变，日计数比较 RFC3339 的前 10 个字符（来源本地民用 `YYYY-MM-DD`）与调用方明确传入的本地日期；用户/模板/`pushed=1` 条件不变。此查询此前已经使用 SQL 函数，改用 `substr` 不新增索引退化。
- 验证：固定存入 `2026-07-19T00:30:00+08:00`，7 月 19 日必须计 1、7 月 18 日必须计 0；治理拒绝和其他模板仍不计。告警统计测试同时移除对并行测试先写文件的顺序依赖。
- 回滚：整体回退 SQL 和测试；不重写历史 analytics 行，不删除真实推送或告警记录。

## Addendum: Gate D Task 15 回测审计与产业链可执行边界（2026-07-18）

- 回测数据流：生产入口仍通过 `DataFetcherManager` 取得真实沪深300基准；已验证的股票历史和可空真实基准进入无网络副作用的布林/RSI 执行函数，完成组合回测、市场状态、样本外切分、walk-forward 与报告生成。测试仅传入 `TEST_CODE_` 本地历史，不注册生产 provider 或基准 fallback。
- 审计失败：交易明细和每日净值属于 2.7 必需审计。目录创建、非 UTF-8 路径或任一 CSV 写入失败现在返回 `Err` 并阻断回测成功，禁止只告警后返回汇总；生产路径和隔离临时目录共用同一写入函数。
- 产业链数据流：真实 push2 多主机请求先取得 HTTP 状态码和原始正文，再进入严格协议解析器；非 2xx、非法 JSON 或缺少 `data` 全部记入主机失败并继续真实主机回退，全部失败时返回聚合错误。测试直接执行该解析器及空 transport 失败，不监听端口、不访问外网。
- LLM 失败：深度、简化和全景提示词均使用完整本地领域证据执行到现有 `GeminiAnalyzer` 边界；没有真实 API 配置时保持明确错误/`None`，不得生成替代分析。候选、龙虎榜、宏观、定向新闻和持仓缺失继续以明确缺失文本进入提示词，不补数值。
- 旧模块与回滚：保留 `AnalysisPipeline`、三种策略引擎、`NotificationService`、真实 provider 顺序与产业链报告格式，只抽取内部 resolved seam 并收紧审计错误传播。整体 `git revert` 本批；不得只恢复审计失败静默成功或 push2 不完整响应成功。

## Addendum: Gate D Task 16 RustDX、公告与决策证据边界（2026-07-18）

- RustDX 数据流：真实 RustDX 返回条目先转换为模块内协议记录，再统一进入 BR-092 严格批次校验；空批次、坏日期/OHLCV/金额、重复或缺失交易日、非有限值及相邻有效收盘跳变超过 20% 都整批报错，成功批次保持最新日期在前。测试只使用本地 provider 协议样本，不建立生产行情 fallback。
- 公告数据流：真实东方财富列表仍由现有 HTTP 客户端取得；响应先完整校验 `data/list`、公告身份、日期和关联股票，再仅为 Emergency/Important 拉取正文，最后通过同一组装函数形成告警。高危正文缺失必须报错；Info 正文保持空白，Skip 不进入结果，缺字段不补默认值。
- 决策边界：RS 计算拒绝零窗口、非有限或非正端点价格；排除、轮动、龙头、板块评分和资金验证只增加确定性输入/渲染测试，不改变既有阈值、排序或筛选语义。缺少 K 线的持仓继续明确跳过，不生成伪造评分。
- 旧模块与回滚：复用 `validate_kline_series_strict`、BR-059 公告分类、既有决策接口和真实 provider；不新增数据源或生产测试开关。整体 `git revert` 本批即可；不得只撤销严格校验而保留已满足 2.3 的声明。

## Addendum: BR-127 辅助审计与龙虎榜缓存完整性（2026-07-18）

- 根因：账户模式推送标记忽略 SQLite 实际更新行数，未知 ID 也会返回成功；龙虎榜 API 结果直接写缓存且写入错误被 `if let Ok` 丢弃，因此上层可能在没有审计缓存的情况下报告完整成功。龙虎榜写入口也未验证坏字段或批内重复。
- 数据流：HTTP 成功响应必须含 `result.data` 数组，每行身份和数值字段严格解析，缺失/坏字段不得跳过或补 0；完整 API 批次再按当前测试/生产环境校验 code 身份、日期、价格、涨跌、买卖/净/总金额及比例。只有整批通过才在一个事务内执行幂等插入。读取、计数、过期清理和去重继续使用现有 SQLite 表；缓存读取失败不得伪装未命中，API 取得真实批次但缓存失败时整个请求返回错误。
- 审计边界：账户模式标记执行后检查 affected rows，必须恰好等于 1。零行代表目标审计事实不存在，多行代表数据库约束异常，两者都不得伪装已推送。
- 测试与回滚：唯一 `TEST_CODE_` 行和进程级隔离测试 SQLite 覆盖合法批次、批内重复、坏字段、事务无部分写入、幂等、查询/计数/清理以及未知账户日志 ID；不调用龙虎榜外网。整体回退 BR-127 代码/测试；不得只恢复吞错或零行成功行为。

## Addendum: BR-128 日 K 多源竞速结果完整性（2026-07-18）

- 根因：现有竞速以“是否看见非空批次”推断全空；当一个真实源传输失败、其余源明确返回空时，最终错误仍写成“所有数据源均返回空”，丢失了真实来源故障事实。
- 数据流：四个既有真实 provider 仍并发执行，首个非空且通过统一日 K 质量门的完整批次胜出；解析、OHLC/日期连续性、涨跌幅与跳变阈值均不改。只有胜出后才更新 Kline capability。没有胜出时按“存在质检拒绝 → 存在传输/任务失败 → 全部明确为空”分类，并保留各来源原因；不引入新来源、默认行情或坏行跳过。
- 测试隔离：本地 future 只承载 `TEST_CODE_` 协议批次和显式错误，用于直接覆盖竞速收敛函数，不注册生产 provider、不访问公网、不读取真实账户或行情数据库。
- 旧模块与回滚：采用现有 `FuturesUnordered`、BR-092 质检、ban 诊断与 BR-108 capability；只抽取无 transport 副作用的收敛 seam。整体回退 BR-128 代码、测试和文档；不得只恢复把源失败误报成空结果的分支。

## Addendum: BR-129 新闻、预测与主题证据持久化（2026-07-18）

- 根因：数据库根模块的低覆盖路径仍以格式化字符串执行预测更新/查询，允许引号改变 SQL 语义且可写入非有限实际涨跌；新闻详存把缺失代码改写为空串并未核验内容哈希；主题签名批次会跳过空行后提交其余记录。这些行为会使审计事实与来源输入不一致。
- 数据流：新闻条目先校验必填身份、时间顺序、当前环境代码和由标题+摘要重算的 SHA-256，再以 nullable bind 写入既有 `(source,external_id)` 幂等表。预测保存、计数、查找、待结算和更新统一使用 Diesel 参数绑定，日期与有限数值在获取连接前校验；原有 reason、日期窗口、hit 和最新 ID 语义不变。主题签名整批预检后在既有事务内 UPSERT，保留 `created_at` 倒序读取和至少 50 条的既有保留口径。
- 失败模式与测试：坏日期、NaN/Inf、空身份、哈希不一致、环境混用、空签名或 SQL 引号都必须显式失败或按字面值处理，失败批次不得留下部分记录。测试只使用唯一 `TEST_CODE_` 新闻/预测和共享测试 SQLite，不读取真实账户、不访问公网。
- 旧模块与回滚：采用现有 `NewsItem`、`prediction_tracker`、`topic_novelty_history` 表及 BR-016/017/066 语义，不创建替代表或内存 fallback。整体回退 BR-129 代码、测试和文档；不得只恢复 SQL 拼接、NULL 空串替换或坏行跳过。

## Addendum: BR-130 真实投递历史完整性（2026-07-18）

- 根因：生产 `PushDeliveryEvent` 持久化 `push.delivery.audit`，但 `PushRecord` 和成功率测试只接受手造的 `push.delivery`，导致真实投递全部被静默排除；未知 outcome 被改写成 Failed，历史读取还会跳过坏 JSON/坏记录，统计可以在证据损坏时仍报告成功。
- 数据流：生产 envelope 类型统一为 `push.delivery.audit`；事件构造先校验非空 kind/channel 和登记 outcome，`PushRecord` 对相同字段、类型及 latency 做严格读取。模板层用于全局冷却的空字符串键在进入事件链前规范化为 `None`，不伪造证券身份。历史查询对 JSONL 每一非缺失分片执行完整解析，非投递事件只在成功解析且明确类型不匹配时排除；投递审计字段错误必须传播。成功率继续使用既有时间窗、sink/kind 筛选和 `Pushed/(Pushed+Failed)` 公式。
- 失败模式与测试：不存在的日期文件表达零记录；目录权限、文件读取、空行、坏 JSON、缺 kind、未知/缺失/错类型 outcome/channel/latency 都不得跳过。测试写入唯一临时 JSONL，覆盖生产事件 round-trip、全局事件空键规范化、筛选、零分母和损坏文件，不调用外部 sink、不写真实审计目录。所有修改推送环境变量的测试共用同一串行域，并在 Drop 时恢复原值，防止静默时段判定串扰。
- 旧模块与回滚：采用现有 `PushDeliveryEvent`、`PushRecord`、`HistoryQuery`、BR-043 排序/限制和 BR-091 hash-chain，不引入兼容别名或第二套统计。整体回退 BR-130 代码、测试和文档；不得只恢复测试专用事件类型或损坏行跳过。
