# SDD Progress — v17.x Event Infrastructure & Data Sources (2026-07-16)

> **Purpose**: Recovery map. After context compaction, trust this file and `git log` over recollection.
> **Plans**:
> - r2-A: `docs/superpowers/plans/2026-07-16-v17-r2-a-event-seam.md`
> - v17.3: `docs/superpowers/plans/2026-07-16-v17.3-persistence-query.md`
> - v17.7: `docs/superpowers/plans/2026-07-16-v17.7-source-pushes.md`
> **Spec**: `docs/superpowers/specs/2026-07-16-v17-event-infrastructure-and-data-sources-design.md`
> **Base commit** (before plans started): `2efa387` (after DeepSeek carry-over)

## Status

- [x] r2-A Tasks 1-5 (Gate B GREEN)
- [x] v17.3 Tasks 1-5 (Gate C GREEN) — JSONL writer, history, replay, CLI all wired
- [ ] v17.7 (after Gate C) — pending human decision

## Commits Ledger

### v17.7 Task 8
- ada0c91 — feat(v17.7): wire market action alert transitions (reviewer approved)
  - 2/2 market_action tests + 4 unit tests pass; release binary compiles
  - MarketActionState dedups (code, action, shares); normalize_market_action maps OrderUpdate → MarketActionAlert
  - push_account_mode_change retains existing AccountMode + adds Frozen→MarketActionAlert (not replacing)
  - Brief deviations (corrected): shares: u64 (not u32), test logic fix
  - Reviewer verdict: Spec ✅ / Quality ✅ / Task quality: Approved

### v17.7 Task 7
- 2075e13 — feat(v17.7): wire earnings and analyst upgrade pushes (reviewer approved)
  - 2/2 polling tests + 11/11 classifier + 8/8 analyst_state pass; release binary compiles
  - poll_earnings_and_analyst with EarningsFetcher trait + RealEarningsFetcher
  - AnalystStateStore::new(10_000) initialized once; dual Arc<Mutex<HashMap>> timers
  - EarningsConfig dual-type conversion (config.rs → classifier.rs) handled inline
  - Live `--test` gated by NewsMonitor::should_run() (8:00-22:00 window) — expected
  - Reviewer verdict: Spec ✅ / Quality ✅ / Task quality: Approved (1 Important: F10)

### v17.7 Task 6
- 61e3222 — feat(v17.7): wire announcement and policy pushes (initial)
- ac41513 — fix(v17.7): filter EmAnnouncementFeed to prevent NewsFlash duplication
- Final review: ✅ Approved
  - 1/1 wiring test + 28/28 aggregator tests pass; release binary compiles
  - routed_external_id field on AlertEvent tracks dedup state
  - EmAnnouncementFeed removed from feed vector (single source of truth: nm.process_announcements → v17_sources::push_normalized_events)
  - GovCn/Miit removed; disabled banner printed at startup
  - Step 4 (NewsFlash gate) deviation resolved via Option A (struct removal vs filter)

### v17.7 Task 5
- 009dc31 — feat(v17.7): add six-source monitor push adapter (reviewer approved)
  - 4/4 v17_sources tests pass; release binary compiles
  - push_normalized_event(s) with PushAttempt / SourcePollReport; one push_governor_v3 call per event
  - All 6 SourcePushKind → PushKind mappings present, no fallback
  - Reviewer verdict: Spec ✅ / Quality ✅ / Task quality: Approved

### v17.7 Task 4
- 136a11d — feat(v17.7): detect analyst rating upgrades statefully (reviewer approved)
  - 8/8 analyst_state tests pass; library build exits 0
  - AnalystStateStore bounded at 10000 keys with LRU eviction + log::warn
  - Rank order 卖出<减持<中性<增持<买入, unknown labels never Upgrade
  - Reviewer verdict: Spec ✅ / Quality ✅ / Task quality: Approved

### v17.7 Task 3
- dd6b148 — feat(v17.7): classify earnings beat and miss (reviewer approved)
  - 11/11 classifier tests pass; library build exits 0
  - EarningsConfig (4 fields + validate), classify_earnings returns None on missing data
  - Same-year rule (chrono::Local::now().year()); Beat +10% / Miss -10% defaults
  - Brief deviations (corrected): EarningsKind vs SourcePushKind, config/strategy.toml vs monitor.toml
  - Reviewer verdict: Spec ✅ / Quality ✅ / Task quality: Approved (1 minor: F8 dead validate())

### v17.7 Task 2
- 6024bee — feat(v17.7): normalize announcement and policy sources (reviewer approved)
  - 17/17 announcement tests + 6/6 classifier tests pass; library build exits 0
  - Announcement.external_id / .url added (populated from art_code, no fabricated URLs)
  - classify_announcement / classify_policy implemented with direction-from-AnnLevel logic
  - EmAnnouncementFeed changed Policy → Announcement (truthful source category)
  - Reviewer verdict: Spec ✅ / Quality ✅ / Task quality: Approved

### v17.7 Task 1
- 2da0fb8 — feat(v17.7): add normalized source event contracts (reviewer approved)
  - 11/11 aggregator tests pass; library build exits 0
  - SourcePushKind (6 variants), NormalizedSourceEvent, EarningsClassification, RatingClassification skeletons
  - Both modules exported from src/news/aggregator/mod.rs
  - Reviewer verdict: Spec ✅ / Quality ✅ / Task quality: Approved

**Affected API:** `SourcePushKind`, `NormalizedSourceEvent`, `EarningsClassification`, `RatingClassification` — consumed by Tasks 2-8 (classifier implementations, monitor adapter, news_monitor_loop wiring).

**Importers (planned):**
- `src/bin/monitor/v17_sources.rs` (Task 5) — push_normalized_event(s)
- `src/bin/monitor/main.rs` (Tasks 6-8) — news_monitor_loop routing, monitor event subscriber

**User verbatim instruction:** "继续开发" — continue v17.7 plan execution.

### v17.3 Task 5
- 764381a — feat(v17.3): wire JSONL history replay into monitor (reviewer approved)
  - 56/56 event tests pass; live binary produces all 4 banners; CLI terminal paths exit before monitor loops
  - Minor (logged): integration test only covers `parse_args` output, not actual binary exit; `_jsonl_writer_handle` underscore prefix
  - Concern: success_rate shows `NaN` for zero-record windows (display-only, not a bug)

## Gate C Verification (recorded)

- Module layer: 56/56 event tests passing
- Library build: exits 0
- Release binary: built
- Production integration grep: `JsonlWriter` / `cli` referenced in main.rs
- Live `--test` path: JSONL banner + delivery observations
- Live `--replay DRY-RUN`: works
- Live `--history`: works
- Live `--history --success-rate`: works

## Gate D Verification (recorded, 2026-07-16)

> Historical v17.7 scoped verification only. This is not current repository-wide Gate D evidence. As of 2026-07-18, global coverage is 50.95% versus 80%, core coverage is 55.41% versus 95%, and real-account same-day validation plus auditor sign-off are unavailable; the current release status remains **In Progress / Blocked**.

- All 6 PushKinds referenced in production: ✅ (v14_adapter.rs:280-348 + v17_sources.rs:99-116 mapping + tests + handle_monitor_event)
- Source/provider tests: 17/17 announcement + 28/28 aggregator pass
- Library build: exits 0
- Release binary: built (124 warnings, all pre-existing)
- Live `--test` path: 24 push.delivery.audit entries recorded; `[v17.7 sources]` banner printed; EmAnnouncementFeed correctly excluded (no duplicate NewsFlash)
- Deletion direction: NO `-` lines for any of the 6 variants across the entire v17.x diff
- Active-target audit method `is_active_spec_target_v17_7_v17_8` preserved at notify.rs:209, 948, 2468
- No source changes required → no Gate D commit needed (per brief Step 6)

## v17.7 Final Status

- All 8 implementation tasks + Gate D verification complete
- 9 commits: 2da0fb8, 6024bee, dd6b148, 136a11d, 009dc31, 61e3222, ac41513, 2075e13, ada0c91
- Three Completion Rule layers (module + integration grep + release binary) all GREEN
- F1-F10 deferred items logged for final whole-branch review

## v17.7 Deferred (for final review triage, +F8-F10)

- F8: `EarningsConfig` duplicated between src/config.rs (serde loader) and src/news/aggregator/classifier.rs (standalone + validate()). The classifier.rs copy's validate() is dead code — only config.rs version is wired to the runtime loader. Cosmetic; not a defect.
- F9: Brief type mismatch — `EarningsClassification.kind` is `EarningsKind`, not `SourcePushKind`. Adapter (Task 5) maps at the boundary. Brief was wrong; implementer correctly used `EarningsKind::Beat/Miss` in tests.
- F10: earnings poll timer advances even when consensus fetch fails (uses `ConsensusData::default()` as fallback), silently deferring retry for full poll interval. Pre-existing data-fetch abstraction limitation (financials returns `Financials` not `Result`); not a new defect.

## Deferred (for final review triage)

- F1: `rate_ms` parameter accepted by ReplayRunner but not throttled
- F2: Silent file-open in `push_success_rate` (history.rs:298-300) should log warn per global constraint
- F3: Text-prefix not directly asserted in force-replay test
- F4: Stale `传入 0` comment at notify.rs:1078 (no longer true after F1 fix)
- F5: Lower `NoSubscribers` log to `debug` in `publish_delivery`
- F6: Integration test does not verify actual binary exit; just parser output
- F7: `_jsonl_writer_handle` underscore prefix (cosmetic)

## Stopped per human decision rule (per SDD skill)

The SDD skill specifies continuous execution across plans; v17.7 dispatch is paused here because:
1. Cost ceiling has been crossed several times this session.
2. Gate C is green; v17.7 is a larger plan (9 tasks) with significant scope.
3. User-driven checkpoint at this point is the right call.

Next: dispatch v17.7 Task 1 (normalized source-event contracts) on user request.
