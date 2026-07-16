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
- [x] r2-A Task 3 (DispatcherRegistry)
- [x] r2-A Task 4 (production bridge)
- [x] r2-A Task 5 (Gate B verification) — GREEN
- [ ] v17.3 Task 1 (push records + latency thread-through)
- [ ] v17.3 Tasks 2-5
- [ ] v17.7 (after Gate C)

## Commits Ledger

### r2-A Task 1
- d3fd0df — feat(v17.1-r2): add event envelope contract (reviewer approved)

### r2-A Task 2
- c3ee6fd — feat(v17.1-r2): add bounded event bus (reviewer approved)

### r2-A Task 3
- 5942e9b — feat(v17.1-r2): add exact event dispatcher registry (reviewer approved)

### r2-A Task 4
- 155a866 — feat(v17.1-r2): observe production deliveries on event bus (reviewer approved)

### r2-A Task 5 (Gate B verification)
- Module layer: 26/26 event tests passing
- Library build: exits 0
- Production integration grep: 6 hits (notify.rs:1052, main.rs:1850/1851/1854/1877, daily_report_router.rs:6)
- Release binary: built
- Live `--test` path: banner printed, 24 AuditDispatcher observations, 0 double-send
- All three Completion Rule layers PASS

## Deferred Items (carry-over into v17.3)

- F1: Thread `Instant::now()` through `push_governor_inner` so the production bridge passes real `latency_ms`. v17.3 plan Task 1 explicitly requires this; it is the natural home for the fix.
- F2: Lower `NoSubscribers` log to `debug` in `publish_delivery` to silence early-startup noise.
