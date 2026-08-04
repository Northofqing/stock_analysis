# Findings

- `8740b8a665e2fb68894ad82cb99228de5151dc33` is the historical design blob
  used when this isolated implementation plan started. Current HEAD is blob
  `31e4e3fa4b5ab261f40f43de50e8861d5fd6e77c`, SHA-256
  `e203a98a012bb86efb51bba184300426de4128a7fdfdfe04412ed07fae4c22b4`.
- The current corrective bytes add normative section 13 and explicitly state
  that fresh independent review is required before Gate B. This is a process
  blocker, not an implementation failure.
- Starting checkpoint reported by the parent: activation `7/22`, scheduler
  `9/19`; library clippy green.
- Remaining frozen slots: activation 15, scheduler 10, migration 26,
  projection 11.
- This slice is limited to durable cadence/acquisition artifacts and their
  single scheduler-owner seam.
- Production databases, audit streams, push sinks, orders, and paper positions
  are out of scope and must not be touched.
- `selection::acquisition_v2` already contains the closed feed outcome matrix,
  stopped-prefix validation, serial intent/provider/seal choreography, and
  strict uncertainty carrier.
- Before this slice there was no SQLite table/repository/read model for a
  cadence receipt, intent, response seal, aggregate seal, terminal receipt, or
  uncertainty artifact.
- The first durable dependency is the restart-aware cadence receipt. Its
  append must be split into immutable SQLite row -> synced audit append ->
  immutable audit-closure row so every crash boundary is recoverable without
  overwriting evidence.
- Current partial tests expose 7 activation contracts and 10 scheduler
  contracts. The scheduler suite and the three journal unit tests are green.
- The durable repository currently persists only cadence receipts and their
  audit closures. It does not yet own plan/feed intents, response seals,
  uncertainty rows, aggregate seals or cycle-terminal receipts, and the
  frozen `selection_v2_generation_scheduler_loop` production owner is absent.
- Once Gate A is independently green, the next smallest vertical slice is one
  immutable plan-intent row plus exact audit closure/readback and restart/
  transaction-failure tests. Feed/provider/seal orchestration remains a later
  slice.
