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
