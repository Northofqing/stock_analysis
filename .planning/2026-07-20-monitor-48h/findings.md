# Findings & Decisions: master release monitor 48-hour observation

## Requirements

- Run the release `monitor` built from current `master` for at least 48 cumulative active hours.
- Observe continuously across restarts; exclude restart downtime from cumulative runtime.
- Collect concrete issues using sanitized counts/statuses only.
- If a runtime blocker appears, implement only the smallest root-cause fix.
- Every fix must follow Gate A–D, PR merge, master rebuild, immediate single-process restart.
- Commit the final sanitized operations report under `docs/` and merge it to `master` via PR.
- Never expose or commit account data, holdings, securities, credentials, notification targets,
  platform/message identifiers, or message bodies.

## Baseline Findings

- Local and remote `master` both resolve to merge commit `f45525423bcb2d73b2dd97f5e25b50abfe47fbd4`.
- The running optimized binary checksum is
  `d108ed6e28e0039c57ccf65fb652824db727c07a46d21d8f972abdafb86b3f2f`.
- First authoritative log record for the current master deployment is
  `2026-07-20T16:01:07+08:00`; this starts active-runtime segment 1.
- Exactly one `monitor` process plus its `caffeinate` wrapper was present at baseline.
- The private raw log is `/private/tmp/stock_analysis_monitor.log`, mode `0600`; it is not a Git
  deliverable and raw lines must not be copied into project documents.
- Initial master-start aggregate evidence: AccountMode and DataMode initialized; one real Feishu
  receipt was validated; one L7 `data_mode/Pushed` record exists; governance-unavailable,
  sink-error, dry-run, and announcement-error counters were zero.
- Missing risk context continues to retry because the real-account batch is not fresh enough for
  the 30-second red line. This is explicit fail-closed degradation, not a process blocker.

## Gate Evidence Carried into Baseline

- PR #6 merged the notification-liveness fix into master with independent Gate A–D approval and
  zero blocking objections.
- Formatting, strict Clippy, full workspace tests, compliance, and release build passed.
- Current coverage report: global 80.52%; registered core 95.37% across 101 files.
- The notification transport now records `Pushed` only after non-empty, non-placeholder channel
  receipt identities are validated; only aggregate status/length evidence is retained.

## Observation Schema (sanitized)

Each checkpoint records:

- checkpoint timestamp and active-runtime duration;
- process count and whether the expected master binary is running;
- private log size/mtime/mode and whether it advanced;
- counts of panic/fatal/exit, governance-unavailable, sink/audit/database errors, validated
  receipts, source failures/successes, and explicit risk-context unavailability;
- L7 grouped template/outcome/sink counts without event IDs, users, bodies, codes, or targets;
- classification: blocking, degraded/non-blocking, or expected idle/weekend behavior.

## Checkpoint Findings

- Continuation branch audit confirms `master`/remote remain equal at `076c116`; no BR-142
  compatibility hotfix branch exists yet. The existing BR-142 tests cover strict authoritative
  schema, one-way v2 migration, and complete legacy envelopes, but no regression test covers an
  immutable legacy delivery row whose two subject-identity fields are both byte-exact empty
  strings. A prior reviewer READY result belongs to the already-merged one-way-schema work and is
  not evidence for this newly isolated compatibility defect.
- Source/history comparison localizes the incompatibility to the shared `PushRecord::try_from`
  identity parser introduced before the one-way-schema hardening: it rejects every trimmed-empty
  legacy `payload.code` before comparing it with `envelope.entity_key`. The immutable-prefix reader
  reuses that shared parser, while the existing BR-142 legacy-extension test only uses a non-empty
  synthetic test identity. Therefore the production prefix shape was never represented by the
  regression suite, and weakening authoritative v2 validation is neither necessary nor acceptable.
- Sanitized production evidence is exact and bounded: the immutable file has 568 legacy rows,
  zero v2 rows, 30 rows with byte-exact `payload.code == ""` and `entity_key == ""`, and zero
  equal whitespace-only pairs. The pre-hardening parser admitted an empty string as `Some("")`;
  the current shared parser rejects it. The safe compatibility contract therefore restores only
  this historical byte-exact pair at the immutable-prefix reader, normalizes it to absent identity
  in memory, and leaves whitespace, one-sided, mismatched, new legacy, and every v2 path strict.

- User-reported end-to-end symptom at the current checkpoint: no notification has been observed by
  the account owner despite the process remaining alive. The earlier validated `data_mode` receipt
  proves only that one startup/governance transport call received a non-placeholder platform
  receipt; it does not prove that a user-visible business notification was generated or delivered.
- This symptom is now treated as a notification-liveness incident requiring boundary-by-boundary
  diagnosis (event generation, governance decision, transport invocation, receipt/audit outcome),
  not as harmless weekend idleness. No root cause or fix is assumed yet.
- At 16:32:58 the same monitor/wrapper pair was alive for 31:52; monitor RSS was 33,428 KiB and
  the private log continued advancing with mode `0600`. The process has both the primary database
  and the separate push-analytics database open, so the next red-capable check can use durable
  outcome aggregates without exposing message bodies or destinations.
- `push_analytics` has sufficient non-sensitive dimensions (`template_id`, timestamp, governance
  decision, pushed flag, and sink name) to assert the exact server-side symptom. Aggregation must
  omit event IDs, user IDs, validation-error payloads, message bodies, and destinations.
- The red-capable live assertion was run twice with the same verdict:
  `RED business_push_receipts=0` for pushed templates other than startup/account mode since the
  master segment began. Current durable aggregate rows are one denied daily report, forty denied
  earnings notifications, and one approved DataMode transport record. This reproduces the user's
  server-side symptom deterministically without inspecting payloads or targets.
- Recent master history is dominated by notification-liveness changes, including governance
  initialization, receipt validation, durable audits, fresh-risk binding, and retrying monitor
  state alerts. The regression/behavior comparison therefore needs to inspect these exact seams,
  not unrelated market scanners. No repository-level `CONTEXT.md` or obvious ADR file was found
  in the relevant path listing; architecture docs under `docs/` remain the design reference.
- The production bridge confirms ordering: current governance context is evaluated before dedup
  and before any sink call. A `data_quality` denial writes `pushed=false/sink=none` and returns;
  therefore the two denied business templates never reached Feishu. This confirms hypothesis 1
  for observed attempts and falsifies dedup or transport as their immediate cause.
- Dedup runs only after governance approval and writes `sink=deduped`; no such durable rows exist
  in the current interval. Hypothesis 5 is therefore rejected for this incident window.
- Default profiles require `DataMode::Degraded` or better for every PushKind except AccountMode;
  DataMode is the sole data-source-down exemption. The current `Down` context therefore blocks
  daily/earnings business messages by design while allowing only the DataMode state alert. This
  explains the durable row pattern but exposes a liveness gap: repeated degraded dependencies do
  not automatically become additional exempt operational alerts.
- Delivery code has two possible paths. The legacy/current `push_wechat` path is used unless
  `STOCK_ANALYSIS_PUSH_V6_ENABLE=1`; the generic L6 HTTP sinks treat any HTTP 2xx as success without
  parsing provider-level response status. We have not yet established which path this process used,
  so this is a boundary risk, not yet the root cause of the missing user-visible DataMode message.
- The current `push_wechat` router chooses direct HTTP only when a Feishu webhook URL is configured;
  otherwise it invokes the MagicLaw CLI. CLI success requires two non-placeholder receipt fields,
  while direct HTTP only verifies Feishu's business success code. Neither proves that the account
  owner is a member of, or is personally represented by, the resolved destination. The already
  observed `receipt=validated` marker makes CLI the likely live route; this will be confirmed by
  aggregate log counts without exposing the receipt or target.
- The first aggregate route count found two validated CLI receipts and zero HTTP, dry-run, sink,
  or target-resolution errors in the append-only file. Because the file contains pre-segment data,
  the count cannot yet be assigned wholly to this master run; repeat with the authoritative 16:01
  boundary before using it as incident evidence.
- The corrected master-segment count confirms exactly one validated CLI receipt and zero direct-HTTP
  success, dry-run, sink-error, or target-resolution-error markers. Thus the DataMode message was
  accepted by the CLI/platform path, while every business message stopped earlier at governance.
- The current diagnostic shell has no Feishu target/transport variables, but it is not the already
  running monitor's environment and cannot establish the deployed target. Inspect only the monitor
  process's variable names, presence, and non-secret receive-id type; never print values.
- The approved initial-process environment view also reports target/transport variables unset.
  This does not contradict the successful CLI call because the application may load `.env` after
  process start and `ps eww` is not a reliable view of later in-process environment mutation.
  Therefore target provenance must be traced from dotenv call sites and key names, not inferred as
  missing or printed from configuration values.
- `monitor` calls `dotenvy::dotenv()` at startup, and a local untracked `.env` exists in the repo;
  the CLI then runs from the MagicLaw project root, which has its own `.env`. This confirms runtime
  configuration can differ from `ps`'s initial environment. Only key names/presence may be examined;
  values remain out of scope.
- Sanitized key-presence inspection shows the monitor repo supplies `FEISHU_TO`, while the monitor
  repo does not supply a receive-id type; the downstream MagicLaw dotenv supplies
  `FEISHU_RECEIVE_ID_TYPE` plus application credentials. Consequently destination and identifier
  type come from different configuration files. A type/shape mismatch or simply a valid-but-wrong
  target can explain “platform receipt exists, owner sees nothing”; values will not be printed.
- MagicLaw source shows CLI receive-id type priority is explicit flag, then prefix auto-detection,
  then dotenv/default. Recognized `oc_` and `ou_` targets therefore cannot be mis-typed merely
  because the monitor omitted the flag. A recognized target shape would downgrade the mismatch
  hypothesis and leave “valid but wrong destination” as the stronger explanation.
- The configured monitor target has a recognized chat-ID prefix, while MagicLaw's dotenv default is
  open-ID. Because the monitor passes no explicit receive-id flag and MagicLaw auto-detects the chat
  prefix before the default, it will send as `chat_id`. The type-mismatch hypothesis is rejected.
  The remaining endpoint explanation is a valid Feishu chat destination that the owner is not
  viewing or not a member of; verifying identity would require comparing the configured target with
  the intended destination, which must not be exposed in logs/docs.
- MagicLaw's live CLI path POSTs directly to Feishu OpenAPI, rejects non-2xx or business `code != 0`,
  and exposes the platform-returned message ID only after acceptance. Therefore the single DataMode
  receipt is not a local placeholder or queue-only acknowledgement: Feishu accepted a message into
  the configured chat. The user-visible miss is now localized to destination correctness/membership
  or client visibility, not transport failure.
- DataMode notification state advances immediately after that platform receipt. Once the initial
  Unsafe/Down-equivalent state is recorded as confirmed, later evaluations of the same state are
  exact-state deduplicated; persistent degradation generates no reminder. This explains why the
  monitor can remain unhealthy with retries for hours while producing no further alert after one
  accepted message. It confirms hypothesis 2: sustained failures are not represented as recurring
  operational liveness events.
- User follow-up about “~98% banner unavailable” was checked against the append-only log boundary.
  Full-history banner-unavailable markers total 4,320 across 44,216 structured records, but the
  current master segment has zero across 2,668 records. Its 82 L5 denials are all `data_quality`.
  Banner bootstrap is therefore a repaired historical cause; current silence is BR-113 safety plus
  BR-116 persistent-state suppression, not a regression of banner initialization.
- The existing BR-116 contract intentionally retries only unconfirmed delivery or mode changes; it
  has no periodic re-notification/acknowledgement requirement for a confirmed-but-unseen persistent
  unsafe state. Any remediation changes notification behavior and therefore requires Gate A plus a
  registered business rule before code edits.
- BR-135 is now implemented in three isolated commits: deterministic reminder state, the existing
  governed DataMode delivery route, and confirmation wiring in the recurring hook. Only a durable
  `Pushed` outcome advances the 30-minute clock; denied, deduped, and sink-error outcomes remain due.
- The release candidate's initial real DataMode notification advanced every authoritative boundary
  by exactly one (Feishu L7, event bus, and immutable audit) and emitted the BR-135 confirmation
  marker with no banner, sink, audit, panic, or fatal failure. This proves the branch uses the real
  producer path; the distinct persistent-reminder behavior still requires the 30-minute observation.
- On final HEAD `d918240`, the first 324 structured records from `19:13 +08:00` contain zero
  `banner unavailable` markers. Historical banner failures are therefore attributable to the
  repaired bootstrap cycle, while current normal-message denials are explicit `data_quality`
  governance outcomes under real Unsafe capability health.
- The banner bootstrap cycle was: incomplete account evidence caused AccountMode assembly to
  return before `LATEST_BANNER` was stored; every v14 push then required that missing banner, and
  the DataMode hook required it too. Repeated scheduled producers amplified one shared missing
  prerequisite into a high apparent push-failure percentage; it was not a Feishu transport rate.
- The live `stock_position` projection can preserve quantity, average cost, and source time, but it
  cannot preserve current price, market value, holding PnL, or broker-reported available quantity.
  Those facts remain in the ignored source record. Treating an available/total difference as a
  completed trade, same-day rebuy, or pending-order freeze would fabricate a cause, so no such
  adjustment was written.
- Independent review found that reminder correctness depends on two different moments: decision
  eligibility is sampled before dispatch, but the next interval must be anchored after authoritative
  delivery. It also established that a real recovery is a health fact, not a notification fact, so
  it must break the continuous-Unsafe interval even if the recovery message cannot be delivered.
- Brainstorming selected a fixed 30-minute persistent-Unsafe reminder as the smallest bounded
  behavior: use only the real DataMode health input and existing DataSourceDown exemption; advance
  the reminder clock only after confirmed sink plus audit success; clear on recovery. Per-source
  alerts were rejected as a retry-amplification risk, and client acknowledgements were rejected as
  a separate integration project.
- The deep-module seam will be a pure reminder-decision state in `monitor::data_mode`: callers supply
  current mode/time and commit only a confirmed result. This keeps clock/rate-limit logic out of the
  huge binary loop and makes the public behavior testable without notification mocks. The existing
  DataMode adapter remains the sole real delivery path.
- The existing notification-liveness design already establishes the compatible constraints: first
  Unsafe is audible, mode state commits only after `Pushed`, DataMode has no L4 time cooldown, and
  all state alerts use the real L4/L5/L6/L7 path. The new design extends, rather than replaces,
  BR-116 with a separate confirmed-reminder timestamp; it must not weaken exact transition retry.
- `monitor::data_mode` is currently a pure evaluator plus process-local capability tracker and is
  the appropriate deep module for a small reminder-state interface. It has no notification sink or
  clock dependency today; the new state should accept caller-supplied monotonic elapsed time and
  return a decision, preserving deterministic tests and keeping real delivery in the binary adapter.
- The existing template orchestrator already isolates `DataModeNotificationPlan` and returns a
  typed `ModeDispatchResult`; it can be extended with a `PersistentReminder` trigger without a new
  sink or bypass. A reminder can reuse `PushKind::DataMode`, so existing DataSourceDown governance,
  receipt validation, L7 analytics, dedup commit, and immutable delivery audit remain authoritative.
- There is exactly one production caller and a focused monitor test cluster for this orchestrator.
  The implementation can therefore add one `reminder_due` input and one dispatch-reason enum while
  preserving all existing call paths. The first RED slice will remain at the pure reminder-state
  interface; the second RED slice will prove the same Unsafe plan dispatches only when due.
- Feishu CLI destination precedence is explicit: `FEISHU_TO`, `MAGICLAW_FEISHU_TO`, chat ID,
  open ID, user ID, then email. Receipt validation does not record which configuration key won or
  whether its receive-id type matches the identifier; this is a concrete observability gap.

- Checkpoint 1 at 16:13:20 proves 12 minutes 13 seconds of active segment-1 runtime.
- The process and private log are progressing; no panic/fatal, governance, sink, audit,
  database-lock, or announcement blocker was detected.
- The window contains 297 warnings and 27 errors, so totals alone are not sufficient evidence of
  health. The next action is sanitized category attribution by stable module/error class.
- Risk-context unavailability (50) and explicit retries (25) are visible rather than silent.
- One receipt-backed DataMode delivery is durable in L7. Twenty non-pushed `earnings_miss` rows
  require classification; no event IDs, users, securities, or bodies will be inspected.
- First category attribution: all 27 error-level records are BR-134 account-freshness/risk-context
  failures. They are explicit fail-closed paper-risk rejections, not process exits.
- Warning rules in the same window: BR-115 external-source protocol failures 133, BR-134 risk
  context 27, BR-112 incomplete opportunity context 9, and BR-108 account evidence 5.
- Safe tags show a small number of NewsAggregator/NewsMonitor/announcement/health warnings or
  errors plus repeated portfolio/paper-engine degradation. Source-code call sites must be mapped
  before assigning a concrete root cause; no raw runtime line will be copied.
- Stable call-site mapping now shows:
  - `health` error means the startup composite health check was not all-ok; its paired warning can
    mean the optional health webhook is not configured. This does not stop main startup.
  - `announcement` warning can mean the keyword configuration is missing and the documented
    compile-time keyword policy is used with an explicit warning.
  - `portfolio` warnings mean a real position lacks industry-chain metadata; the field remains
    empty rather than being fabricated.
  - `NewsAggregator` warnings are per-feed real HTTP failures; the aggregator continues other
    independent feeds. The eight registered real feeds are Jin10, WallStreetCN, CLS, Sina,
    Weibo, Gelonghui, KcbDaily, and GovPolicy.
  - `NewsMonitor` errors have explicit retry/continue branches for target-pool or announcement
    acquisition failure; exact stable-message counts are required before classification.

### Checkpoint 1 exact stable-message attribution

- Startup composite health failed once; the optional legacy health webhook was unconfigured once.
  The process continued and the governed Feishu DataMode path still produced a validated receipt.
- Announcement acquisition failed once after master start. This is an explicit retryable source
  failure; recovery must be proven by later complete-batch evidence before calling it transient.
- Announcement keyword configuration fell back once with an explicit warning to the registered
  compile-time policy. This is configuration debt, not fabricated market/account data.
- Jin10 was the only registered NewsAggregator feed with a stable per-feed failure signature in
  this interval (7 failures); independent feeds continued.
- v17 earnings enrichment rejected 133 incomplete financial/consensus batches. The records were
  not pushed (`earnings_miss` L7 outcomes remain `pushed=0/sink=none`), which is fail-closed but a
  high-volume degraded source condition.
- Real positions were missing industry-chain metadata 24 times and the field stayed empty.
- AccountMode incomplete evidence was reported 5 times; paper risk context rejection was reported
  repeatedly. Neither path fabricated a complete account state.
- One NewsAI warning was observed; exact cause remains to be mapped from stable source call sites.
- Announcement recovery has authoritative safe markers: a complete list fetch logs one list-success
  marker, complete assembly logs one filtered-success marker, and only then is News capability
  marked successful. The next checkpoint will count those markers and compare their timestamps to
  the one failure without reading any announcement content.
- v17 earnings enrichment intentionally does not advance `last_poll_earnings` when either the
  financial or consensus source fails. This preserves immediate retry eligibility and explains the
  133 warnings, but also creates retry/log amplification while a source is persistently down.
  It is non-blocking in the current window and should be reported as operational debt unless it
  begins to starve required loops or exhaust resources.
- Announcement failure was transient and self-recovered: one failure occurred at 16:05:37, while
  complete list+filtered successes continued through 16:18:14. Across the window there were nine
  list successes, eight complete filtered successes, and nine non-empty aggregator ticks.
- Process resource baseline at about 16:17:51: monitor RSS 29,232 KiB, CPU 0.0%; wrapper RSS
  2,324 KiB, CPU 0.0%. Both share the authoritative start time 16:01:06 and remained healthy.
- Checkpoint 3 confirms continued progress through 16:21:09 with no new blocker signatures.
  The short delta reproduced the same non-blocking pattern (earnings-source rejection, stale risk
  context, one Jin10 failure) and included another complete announcement success.
- Monitor RSS rose from 29,232 to 30,440 KiB between two early samples. This single 1,208 KiB
  change is not evidence of a leak; retain it as a baseline for later trend comparison.
- Checkpoint 5 extends active evidence to 28 minutes 36 seconds. The same degradation mix repeats
  without any new error class, while three complete announcement batches prove forward progress.
- Monitor RSS is 32,544 KiB at checkpoint 5, up 3,312 KiB from the first resource sample. This is
  still an early cache/warm-up range, not a leak conclusion; later hourly samples must determine
  whether RSS stabilizes or grows monotonically.

## Technical Decisions

| Decision | Rationale |
|---|---|
| Use fixed-pattern aggregate extraction rather than copying log lines | Prevents accidental leakage while preserving actionable frequency and trend evidence. |
| Verify process identity with `pgrep` plus binary/master checksum | The restricted sandbox may reject targeted `ps`; these checks still prove one master release instance. |
| Keep stale account evidence unavailable | Database holdings without a fresh same-batch broker snapshot cannot satisfy red line 2.4. |
| Require log progress only according to configured retry/idle cadence | A weekend market loop may be idle by design; absence of market ticks is not itself a stall. |

## Issues Encountered

| Issue | Resolution / Classification |
|---|---|
| Targeted `ps -p` is sandbox-denied | Use approved process-list forms; tooling issue, not monitor defect. |
| Risk context is unavailable because real-account evidence is stale | Explicit retry + conservative restriction; degraded/non-blocking unless it stops required loops. |
| Normalized announcement delivery still has pre-existing legacy routing debt | Non-blocking; current announcement acquisition protocol succeeds and remains observable. |

## Sensitive-data Exclusion

Do not inspect, transcribe, store, or commit raw message text, holdings, account values, securities,
credentials, recipients, notification destinations, message IDs, platform IDs, or user IDs. A final
report may include only aggregate counts, timestamps, classifications, commit/checksum provenance,
and non-sensitive module/error categories.
# 2026-07-20 closed-session scheduler finding

- The persistent-reminder state and delivery path were correct at `d918240`, but their recurring
  caller was not live on closed sessions. The hook sat after branches that exit the iteration on a
  weekend, so only startup evaluation occurred.
- A scheduler for governance health is not a market-data scan. Its ownership must be outside
  market-session control flow, otherwise a market calendar decision can silently disable outage
  reporting precisely when stale capabilities persist longest.
- Final-head production evidence now demonstrates the intended separation: startup one-shot,
  exactly one scheduler start, then a 60-second evaluation during the same closed weekend session.

## 2026-07-22 continuation gap

- The previous observation did not remain continuously supervised: no monitor-related process or
  launchd service exists at recovery time, and the private log stopped the prior evening.
- Therefore elapsed wall-clock time cannot prove the 48-hour requirement. Only reconstructed
  closed segments through the last private-log boundary plus a newly observed open segment may be
  counted; the intervening gap is excluded in full.
- Sanitized timestamp-sequence reconstruction proves seven closed segments totaling `27:43:02`.
  The final historical restart gap was 308 seconds, from `08:03:43` to `08:08:51` on July 21; the
  last valid segment then continued through `19:52:21`. No raw log line or payload was copied.
- Current local and remote master are aligned at `3565bf7`, so the next valid segment must be built
  from that commit. A non-release user build currently holds Cargo resources and must not be killed
  or mistaken for the production monitor.
- Segment 8's first structured record is at physical append-only log line 189,974. BR-135 exposes
  stable, payload-free markers for independent scheduler start, persistent-reminder due,
  confirmation commit, and unconfirmed retry; these can be counted without reading message bodies.
- Segment 8 is making forward progress and already has validated delivery receipts, but early
  warning/error volume remains high. Coarse text matching is insufficient for blocker decisions;
  only stable module/reason categories plus process/log forward progress may support classification.
- A real delivery-audit issue is present in segment 8: 25 error-level records carry the dedicated
  `push.delivery.audit` tag. This is narrower than a process blocker but can block specific
  governed deliveries. Source contract and sanitized error-class evidence must determine whether
  the newly merged v2 audit rejects a historical chain or a current payload.
- The dispatcher poisons its in-memory chain state after the first persistence failure, so every
  later governed delivery fails its audit deterministically until process restart. The sink is
  invoked before audit persistence, which explains simultaneous validated transport receipts and
  application-level `SinkError` outcomes. Because the durable yearly file did not advance, this is
  a red-line 2.7 blocker for the governed delivery path even though the monitor process itself lives.
- The failure occurs before the first v2 append: all 568 existing rows are pre-BR-142 legacy rows,
  and validation rejects a legacy row. The repair must preserve and validate that immutable prefix;
  deleting, rewriting, or silently accepting malformed history is prohibited.

## 2026-07-22 user-requested stop

- Monitoring ended at the final structured timestamp 10:04:41 +08:00. The LaunchAgent and both
  monitor-related processes were absent after bootout.
- Segment 8 added 18 minutes 8 seconds, bringing the defensible cumulative active runtime to
  28 hours 1 minute 10 seconds. The original 48-hour acceptance criterion remains unfulfilled by
  explicit cancellation, not by inferred completion.
- By the end of segment 8, the stable delivery-audit error count had reached 90 while the yearly
  authority file remained fixed at 568 rows. The process stayed alive and no panic/fatal,
  database-lock, or banner-unavailable marker appeared.
- Repository state at stop: local `master` and tracked remote `master` both point to `3565bf7`.
  PRs #2 through #12 are merged; PR #11 contains the terminal/audit lifecycle implementation and
  PR #12 records its merge evidence. No open PR was found in the latest repository query.
- The README architecture rewrite is already on `master`; the handoff should reference it rather
  than restating the full architecture.

## 2026-07-22 goal continuation

- PR #13 merged the sanitized handoff into `master` as `076c116`; local and tracked remote master
  match that commit.
- No release monitor or wrapper survived the earlier stop. The next valid segment cannot begin
  until a new exact process and first structured timestamp are observed.
- The BR-142 blocker remains unchanged: 568 legacy authority rows, no successful v2 append during
  segment 8, and the private raw log remains excluded from Git.
- Current source reconfirms the exact failure seam: `validate_existing_chain` verifies outer
  schema, parent and record hash, then routes every pre-domain row through the general
  `PushRecord::try_from`; that parser rejects any `code` where `trim().is_empty()` before comparing
  it to `entity_key`.
- The minimal safe boundary remains persisted-legacy parsing only. Current authoritative dispatch
  already calls `try_from_authoritative` before persistence, so new v2 records must continue to
  reject any `code/entity_key` field and keep domain-separated hashes.
## Virtual portfolio incident (2026-07-21)

- The persisted paper ledger is present and reconstructs non-empty open positions; no reset,
  deletion, or synthetic replacement is authorized.
- `monitor_loop` currently derives every intraday/paper-engine `PaperRiskContext` from the latest
  complete real-account banner. When real account metrics are incomplete, both virtual decisions
  and virtual exits are skipped before their paper ledger can run.
- `paper_trade::portfolio_state` compounds the coupling by reading the same-day real account
  `ledger` and real `stock_position` rows for cash, total value, and per-symbol exposure.
- Therefore the project has durable paper-trade history but not an independently operable paper
  portfolio boundary. Correct repair requires a paper-account projection reconstructed from
  immutable Filled paper trades plus an explicit, real configured starting-capital fact; it must
  never borrow or fabricate live-account cash.
- DataMode operational alerts are independently over-coupled: their hook reads/stores an account
  banner and returns before delivery when that banner is unavailable. Operational health alerts
  must carry capability state directly and remain independent of account metrics, while live trade
  authorization stays fail-closed.
- Tight regression targets: an unavailable real-account banner must not prevent a DataMode alert or
  a paper-ledger-only position status read, but it must still reject every real-order path.

## PR #10 final-review defects (2026-07-21)

- `stock_position` is explicitly a simulated/local projection, and ordinary return refreshes update
  its `updated_at`. That timestamp cannot prove broker-source position freshness. The announcement
  audience must therefore exclude this table until a per-position broker snapshot with an immutable
  source timestamp/evidence identity exists; registered watch codes remain independent.
- The normalized announcement report currently exposes only a set of handled external IDs. Because
  the outer loop appends every handled alert to its downstream `pushed` vector, lifecycle-only and
  off-universe announcements can still trigger D-01/I-02 even though direct delivery was skipped.
- The route contract needs a typed per-ID disposition. Only `Pushed` may enter downstream
  notification-trigger analysis; filtered and delivery-failed identities remain handled solely to
  block legacy fallback.
- The first production canary was not started: the destructive process switch was denied, and the
  independent reviewer found the two code blockers before the planned isolated live-source dry run.
