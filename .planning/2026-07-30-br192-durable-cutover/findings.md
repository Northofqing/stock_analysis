# Findings

- The old generic counted path derived occurrence and provider evidence from local dispatch
  time plus an empty payload. It was unstable across retry/restart and is now fail-closed.
- `ReviewLhb` already owns a complete Eastmoney GatewayBatch and can bind its real BR-140
  R-04 task identity and source batch.
- `EventCalendar` owns four independent complete Gateway batches and can bind their ordered
  identities to R-08.
- The old `CandidateTriggered` path has quote/statistics evidence but no durable candidate
  lifecycle transition or formal selection-decision batch. It cannot be migrated honestly
  until that seam exists.
- Schedule state is in memory. Startup reconciliation must continue exposing both Pending and
  Applied durable hydrations so a restart can reconstruct terminal BR-140 task state without
  emitting a second legacy transition.
- Monitor binary tests are large: a source-changing rebuild has taken 1–9 minutes locally,
  which materially affects the Gate D estimate.
- `HoldingPlan` cannot yet bind honestly: the quote Gateway evidence is reduced to a price
  map, the local position projection intentionally has no source time, hard-stop evidence is
  absent and no plan decision is persisted.
- `HoldingEvent` cannot yet bind honestly: current position acquisition fails before quotes
  because the position projection lacks source evidence, required volume-ratio/money-flow
  fields are absent, and both alert state plus health-summary identity are process-local.
- `T0Advice` is recoverable without inventing a new provider. Its real user snapshot and full
  Magic TDX inputs exist, but the current batch hash is only code/source-time/price and later
  projection discards the evidence. The migration must hash the complete validated evidence
  and carry it through a persisted canonical decision binding.
- The T0 migration is now complete: the immutable batch is frozen after actual provider
  completion and binds requested/source/observed times, typed instruments, every normalized
  record and explicit rejection. Its five focused suites pass 33/33.
- Twelve production-semantic counted callers remain on the generic fail-closed governor.
  PaperTrade and the real-position summary have enough durable identity to migrate after
  retaining their receipts. The other callers must remain capability-unavailable until their
  immutable business artifact exists.
- The legacy v12 "20 real templates" test still contains fabricated R-02/R-08 facts and many
  generic counted calls. It is not valid release evidence and must be retired, while pure
  renderer tests and the focused BR-192 GatewayBatch tests remain.
- PaperEngine's after-close fallback currently wraps a historical daily close as
  `ExecutionQuote { observed_at: Utc::now() }`, which falsely presents daily evidence as a
  fresh realtime quote. The safe short-term resolution is to enforce <=5s realtime
  freshness before any reservation/audit and isolate the daily fallback until a typed
  settled-daily receipt is persisted.
- The PaperTrade freshness defect is closed: realtime execution rejects future or
  older-than-five-second observations before side effects, and settled-daily execution is
  explicitly unavailable until a typed daily receipt is designed and persisted.
- BR-164's transport cutover is structurally clean: production code outside
  `src/data_gateway/**` has no public financial/news endpoint literals, direct public-data
  `reqwest` acquisition, direct Magic provider constructors, or RustDX/QMT/Mootdx paths.
- A remaining P0 evidence defect prevents final cutover: admitted financial and consensus
  batches lose provider, batch ID and acquisition timestamps in `company_financials`,
  `data_provider::service` and `v17_sources`; normalized events then synthesize local
  observation time and a hard-coded source. The evidence-bearing projection must be carried
  end-to-end before Gate D.
- Several upstream capability gaps correctly fail closed but remain feature-incomplete:
  full-market breadth/turnover/limit identity, post-close overview, enriched sector/limit-up
  projections, complete market rankings, chain laggards, benchmark index bars, richer
  financial metrics and research summary/rating changes.
- Pinned-upstream audit at `d7dfa3140919525f3280bed87136602a78fa17ad` found none of
  those nine gaps is a downstream-only wiring omission. Seven are partial but contract-
  insufficient, post-close overview is absent (adjacent flow is unsupported), and complete
  market rankings are live-admission blocked. The nearest next upstream slices are normalized
  historical index bars and a first-class optional financial-metrics snapshot; neither may be
  bypassed with raw TDX zero filling or downstream composition.
- Gate B review found a release-blocking test/live isolation bug: a normal production process
  can honor `V10_DRY_RUN_PUSH=1`, synthesize a TEST_CODE accepted sink receipt and commit it to
  the production durable-delivery store. Dry-run authority must be physically test-scoped and
  rejected before production reservations or sink classification.
- Financial and consensus Gateway evidence now survives normalization end-to-end:
  provider/source, optional provider time, observed time, batch ID and normalized content hash
  are retained; earnings binds both batches, analyst events bind consensus, and missing,
  mismatched or tampered evidence fails closed without hard-coded source or local-now
  substitution.
- The independent review's tentative manual-resolution finding was withdrawn: BR-192
  intentionally requires immutable authorization append before the SQLite CAS so a later
  database failure can be retried with identical bytes. No code change was made.
- Gate B review also found production schedule hydration is cleared in memory without a
  durable/audited Applied acknowledgement, so restart can reapply the same transition.
- A retry-authorized `RejectedDurable` decision is also stranded after the runtime reaches
  startup-ready: the coordinator supports the transition back to `Reserved`, but the live
  runtime only resumes `Reserved` and maps the rejected state to non-retryable Denied. R-04
  and R-08 can therefore remain stuck until process restart.
- The tentative pre-sink retry-projection Critical was retracted after exact code review:
  `freeze_disposition` already updates the decision row's retry flag atomically. No change was
  made. The valid retry finding is orchestration: authorized rejection can be stranded live,
  while startup must treat legitimate budget/cooldown/head deferral as non-fatal and zero-sink.
- While persisting hydration Applied, the implementation exposed a cross-date loss bug:
  schedule state ignores foreign business dates, but the caller previously acknowledged every
  queued transition identity. Only identities actually applied for the schedule's business
  date may be durably acknowledged; foreign-date transitions must remain Pending.
- Hydration application is now two-phase at the caller: apply into a cloned schedule, durably
  acknowledge only the exact current-date transition identities, then commit the clone. An
  acknowledgement failure leaves both durable hydration and local schedule retryable.
- Runtime namespace must be immutable after resolution. A cached production coordinator must
  never be reused after a request resolves to a TEST_CODE namespace, even if in-process code
  mutates environment variables; the cache now compares requested and bound namespaces before
  returning the stored runtime.
- Exact lexical test paths are insufficient by themselves. The coordinator must also reject
  a TEST_CODE parent symlinked to production data and a test DB hard-linked to the production
  DB; canonical parent and Unix device/inode checks now run immediately before SQLite open.
- A narrow system-level TOCTOU remains between the final physical-path check and SQLite's own
  open. Fully eliminating it would require an fd-relative `openat`/`O_NOFOLLOW` design rather
  than path-based `rusqlite::Connection::open`; the independent review must classify whether
  this is release-blocking for the current threat model.
- Fresh file-sink review proved the incremental cursor violates the repository's stronger
  append contract: changing an older line in place while preserving inode and length can be
  followed by a successful append until the 1,024-record/24-hour checkpoint. Every append
  must therefore stream and validate the complete existing chain; the cursor/cache and its
  unbounded identity map must be removed.
- Counted BR-192 acceptance currently writes the internal immutable acceptance audit but
  omits the existing durable `push.delivery.audit` event used by JSONL/history evidence. The
  final accepted boundary must join exact decision, attempt, artifact, sink-result and receipt
  hashes to that existing event. A validated remote receipt followed by audit failure is
  `Uncertain`, not Accepted or retryable Rejected.
- Fresh SQLite review found four outbox/acknowledgement UPDATEs still auto-committed through
  `with_connection`; post-validation could return failure after state was already committed.
  All writes require `BEGIN IMMEDIATE`, pre-commit validation, exact affected-row CAS,
  explicit rollback, and post-commit/post-rollback validation.
- The runtime's TEST_CODE builder currently passes a manifest-absolute database path while
  `CoordinatorConfig::test` admits only the exact relative namespace. This is an actual
  `monitor --test` startup failure, not merely a missing test.
- Safe Rust makes same-inode raw-fd ABA unreachable while the coordinator's private
  `Arc<Mutex<Connection>>` remains live and no raw handle/close/journal-mode API escapes.
  macOS and Linux also expose OFD locks; a shared marker plus exclusive probe can add
  defense-in-depth, but it must coexist across coordinators, avoid SQLite lock ranges, retain
  the private-ownership invariant and not claim defense against compromised in-process code.
- Provider-free retry needs two distinct sets in reconciliation: immediately deliverable
  `Reserved` decisions and authorized `RejectedDurable` candidates awaiting typed admission.
  Budget, business-date claim, rolling head, uncertainty and cooldown are typed deferrals,
  not a single `false`; one cycle may attempt an identity at most once and must perform zero
  provider calls. `Uncertain` is never auto-retried.
- The second file-sink review found that strengthening the append primitive alone is
  insufficient: the counted acceptance boundary still invokes the older pathname-based
  `AuditDispatcher`, which can re-open a replaced audit root after preflight. The dispatcher
  itself must retain and revalidate the entire root/year/lock/JSONL identity chain before and
  after durability, or fail closed before a `Committed` marker can be written.
- A remote receipt followed by an audit or commit-marker failure must remain authoritative
  `Uncertain` and must be represented as non-auto-retryable everywhere, including the event
  envelope. Mapping it to a generic retryable `Failed` outcome reintroduces duplicate-send
  risk even if the coordinator state is correct.
- Release evidence requires one checked-in verifier that joins the exact pending artifact,
  schema-v3 `push.delivery.audit` record and matching `Committed` marker. Independent
  validation of the three stores is not enough because it cannot prove they describe the
  same counted attempt.
- SQLite physical isolation cannot be established after `initialize_schema`: enabling WAL,
  committing DDL/policy rows and only then binding WAL/SHM allows an isolation error to be
  returned after real mutation. Bootstrap must perform only the minimum sidecar
  materialization under the retained authority, attest main/WAL/SHM immediately, and execute
  schema/policy transactions only inside the fully validated operation boundary.
- Platform, descriptor-enumeration, OFD and pre-existing-sidecar capability checks must
  precede `O_CREAT` of the main ledger. An unsupported or injected failure that leaves an
  empty production/test ledger behind is still a physical-isolation violation.
- Test “zero touch” evidence must not hash or read production SQLite/WAL/SHM bytes. If any
  production artifact exists, the isolation suite must refuse before test mutation; in an
  empty production namespace it can prove the exact artifact set remains absent while
  retaining/statting the manifest-to-data ancestor authority.
