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
- [x] v17.3 Tasks 1-4 (push records + JSONL writer + history + replay/CLI)
- [ ] v17.3 Task 5 (Gate C verification)
- [ ] v17.7 (after Gate C)

## Commits Ledger

### v17.3 Task 4
- ff96eed — feat(v17.3): add safe replay and history CLI (reviewer approved)
  - 16/16 tests pass; typed errors; dry-run never publishes; force-replay only `push.source`
  - Minor (logged): `rate_ms` parameter accepted but not throttled; `count` overcounts when all publishes rejected; text-prefix not directly asserted in test

