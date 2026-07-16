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
- [x] v17.3 Tasks 1-2 (push records + JSONL writer)
- [x] v17.3 Task 3 (history + success rate)
- [ ] v17.3 Task 4 (replay + CLI parser)
- [ ] v17.3 Task 5 (Gate C verification)
- [ ] v17.7 (after Gate C)

## Commits Ledger

### v17.3 Task 3
- affe306 — feat(v17.3): add event history and delivery rates (reviewer approved)
  - 2/2 tests pass; typed `HistoryError`; per-sink/per-kind rates correct; `Denied`/`Deduped` excluded from rate denominator
  - Minor (logged, not fixed): silent `Err(_) => continue` in `push_success_rate` file-open path (history.rs:298-300) — should log warn per global constraint

