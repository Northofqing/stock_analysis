# Progress — Sina instrument-news downstream Gateway migration

## 2026-07-25

- Read repository engineering rules and brainstorming,
  planning-with-files, writing-plans, implement and TDD skills.
- Emitted the required AGENTS §1.3 pre-flight before editing.
- Created an isolated plan without changing `.planning/.active_plan`.
- Began Gate A read-only caller tracing.
- Confirmed at least one direct `fetch_stock_news_in_range` production call
  exists in `src/bin/monitor/main.rs`.
- Completed multiline scheduling trace: one post-close review caller, called
  by one 30-minute-after-15:30 scheduler, spawned once by the long-running
  monitor branch. No second production stock-news acquisition caller exists.
- Confirmed the sibling module can reuse BR-159 helpers without editing
  `src/data_gateway/review.rs`.
- Audited upstream `SinaClient`, Core news/evidence contracts and
  `InstrumentNewsRouter`.
- Audited the legacy news-items persistence boundary and identified the exact
  lossless UTC/evidence field mapping plus the explicit blank-summary
  compatibility rule.
- Registered BR-163 and wrote the approved design and implementation plan.
- Added the typed sibling `SinaInstrumentNewsGateway`, including real upstream
  `SinaClient: NewsProvider` acquisition, one-source `InstrumentNewsRouter`
  admission, verified-empty preservation, UTC exact filtering, immutable
  evidence projection, explicit error classification, `spawn_blocking`, and
  BR-159 fail-closed acquisition audit.
- Migrated the sole production caller in `post_close_news_review` and added a
  monitor source guard.
- Deleted only the retired local stock-news builder/fetch/range code and
  stock-only tests. General/top Sina financial news remains.
- Isolated-target Gateway tests: 5 passed.
- Monitor BR-163 source-guard test: 1 passed.
- Retained top-news integration test file: 6 passed.
- Provider-local top-news tests: 7 passed; the one loopback transport test was
  not runnable in the filesystem/network sandbox because binding the local
  test listener returned `PermissionDenied`. A requested non-sandbox rerun
  was interrupted before execution and is not counted as passed.
- Upstream real probe supplied by the integration coordinator:
  `cargo run -p magic-sina-rs --example instrument_news_probe --locked`
  returned 3 verified records for 600396.SH and 3 for 000001.SZ; evidence,
  provider time, observation time and canonical URL checks agreed and
  `status=passed`.
- Combined validation currently keeps all five unified dependencies pointed
  at `target/magic_market_unified_work` by coordinator request. Adjacent
  `../magic-market-data-rs` paths must be restored only after the upstream
  worktree is committed/merged.
