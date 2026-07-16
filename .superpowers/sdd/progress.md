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
- [x] v17.3 Task 1 (push records + latency thread-through) — F1 ✅
- [x] v17.3 Task 2 (JSONL writer)
- [ ] v17.3 Task 3 (history filtering + success rate)
- [ ] v17.3 Task 4 (replay + CLI parser)
- [ ] v17.3 Task 5 (Gate C verification)
- [ ] v17.7 (after Gate C)

## Commits Ledger

### v17.3 Task 2
- 92a08f4 — feat(v17.3): persist event envelopes as daily JSONL (reviewer approved)
  - 3/3 tests pass against real filesystem; cleanup never touches today; replay filtering correct
  - Out-of-scope fix: `bus.rs:138` `shutdown()` now drops the Sender so `Closed` reaches receivers (correct, minimal, required for graceful-shutdown test path; pre-existing bug)
  - Minor (logged): `create_dir_all` on every write; `chrono::Duration::days` deprecation

