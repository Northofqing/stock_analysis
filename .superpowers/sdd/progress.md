# SDD Progress — v17.x Event Infrastructure & Data Sources (2026-07-16)

> **Purpose**: Recovery map. After context compaction, trust this file and `git log` over recollection.
> **Plans**:
> - r2-A: `docs/superpowers/plans/2026-07-16-v17-r2-a-event-seam.md`
> - v17.3: `docs/superpowers/plans/2026-07-16-v17.3-persistence-query.md`
> - v17.7: `docs/superpowers/plans/2026-07-16-v17.7-source-pushes.md`
> **Spec**: `docs/superpowers/specs/2026-07-16-v17-event-infrastructure-and-data-sources-design.md`
> **Base commit** (before plans started): `2efa387` (after DeepSeek carry-over)

## Status

- [x] r2-A Task 1 (envelope contract)
- [x] r2-A Task 2 (EventBus)
- [ ] r2-A Task 3 (DispatcherRegistry)
- [ ] r2-A Task 4 (production bridge)
- [ ] r2-A Task 5 (Gate B verification)
- [ ] v17.3 (after Gate B)
- [ ] v17.7 (after Gate C)

## Commits Ledger

### r2-A Task 1
- d3fd0df — feat(v17.1-r2): add event envelope contract (reviewer approved, 0 Critical/Important, 1 Minor error-label drift)

### r2-A Task 2
- c3ee6fd — feat(v17.1-r2): add bounded event bus (reviewer approved, 0 Critical/Important, 2 Minor — dead-code `record_lagged`, best-effort serialization scope)
