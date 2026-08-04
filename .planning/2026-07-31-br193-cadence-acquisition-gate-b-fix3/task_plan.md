# BR-193 Gate B — Cadence/Acquisition Durable Slice

## Goal

Implement and verify the next BR-193 Gate B vertical slice without touching
BR-194, delivery, order, paper, or production data paths.

## Design pin

- Historical Gate A design blob: `8740b8a665e2fb68894ad82cb99228de5151dc33`
- Current corrective design blob: `31e4e3fa4b5ab261f40f43de50e8861d5fd6e77c`
- Current corrective design SHA-256:
  `e203a98a012bb86efb51bba184300426de4128a7fdfdfe04412ed07fae4c22b4`
- Design: `docs/superpowers/specs/2026-07-30-br193-selection-v2-activation-design.md`
- Gate status: **Blocked before Gate B** until a fresh independent review of
  the current corrective bytes returns zero Critical and Important findings.

## Scope

1. Inventory the current BR-193 cadence, intent, seal, uncertainty, terminal,
   repository, and scheduler-owner implementation.
2. Add RED tests for missing frozen scheduler contracts.
3. Implement SQLite/audit-backed exact serialization, hash/readback/order, and
   prior-boot/transaction-failure recovery.
4. Connect the production scheduler owner through the opaque
   namespace/lease capability only after durable intent ownership is proven.
5. Run targeted suites, rustfmt, library clippy, and the BR-193 verifier.

## Constraints

- No mock or in-memory-only production path.
- No production selection database, audit, or push sink access.
- Use `TEST_CODE` and temporary databases only.
- Do not edit BR-194/durable-delivery/monitor files.
- Do not remove `raw_v2::join_all` until the durable owner is connected and
  proven.
- Do not add compatibility aliases or a second scheduler owner.
- Preserve all unrelated dirty/untracked files.

## Phases

- [x] Phase 1 — inventory current implementation and exact frozen-test gaps
- [ ] Phase 2 — RED tests for cadence/intent/seal/uncertainty/terminal contracts
  (blocked on corrective Gate A re-review)
- [ ] Phase 3 — durable repository and recovery implementation
- [ ] Phase 4 — opaque owner/lease scheduler connection
- [ ] Phase 5 — validation, self-review, and handoff evidence
