# G5b durable day seal: implementation gate (2026-10-01)

Status: design audit only. No runtime gate, sender, database, or production
configuration changes are authorized by this document. The proposed seal means
**the fixed G5b selected batch for one local business date is closed under an
explicit outcome policy**. It does not mean every alert of that date was sent.

## Source facts and current gap

| Boundary | Current behavior | Consequence for a day seal |
| --- | --- | --- |
| Alert source | `src/monitor/alert_log.rs:159-197,231-232` reads a file chosen by `Local::now()`, returns an empty list on missing/read error, and skips malformed lines. Appends at `:78-109` have no shared G5b fence or durable input head. | An empty list is not proof of no eligible alerts or of a stable input prefix. |
| Selection | `src/monitor/attribution_deep.rs:479-568` writes a nonempty v1 selection once, after eligible candidates and a provider exist. Its event list is fixed on later ticks; no input offset, generation, or cutoff is saved. | The selection is a fixed batch, not proof that the entire day's alert stream was closed. v1 cannot be backfilled into an automatic day seal. |
| Journal and archive | `begin_assessment` persists an attempt before LLM; `freeze_result` writes an immutable result; `archive_frozen_result` compares exact rows and atomically replaces JSONL (`attribution_deep.rs:572-754,1051-1143`). Historical recovery restores archives only (`:1148-1275`). The archive lock covers archive writes alone. | Attempt-only remains completion-unknown; frozen or archived rows are not counted delivery. Current journal operations do not share a day fence. |
| Counted owner | `src/durable_delivery/coordinator_g5b.rs:109-136` enumerates all same-date G5b decisions in one SQLite read transaction and revalidates envelope, source, terminal disposition and receipt. `src/durable_delivery/schema.rs:155-184` has a decision-identity primary key, but no unique G5b occurrence owner or day-seal row. | The reader catches an orphan or duplicate at observation time, but its snapshot cannot block a later admission or be atomically joined to the files. |
| Monitor date gate | `src/bin/monitor/main.rs:10695-10912` sets process-local `G5B_LAST_RUN` after a batch without journal I/O failure. LLM failures, Frozen replay, or counted Denied/SinkError can still reach that write; restart clears it. | This is an attempt scheduler, not a durable completion owner. |
| Existing verdict | `attribution_deep.rs:298-388,422-430` classifies reconciled observations. `TerminalOutcomesObserved` is explicitly point-in-time. | No existing value authorizes a day seal. |

The tracked-source writer inventory at this base is concrete: the two monitor
alert producers call `alert_log::append_jsonl` at `main.rs:11832,12337`.
`AlertLog::{append_jsonl,append_md,append_batch}` and their public production
wrappers at `alert_log.rs:89-132,205-216` remain callable even where the
current monitor does not call Markdown/batch methods. The G5b monitor calls
`existing_recovery_dates`/`recover_frozen_archives_for_dates` at
`main.rs:10676,10670`, `load_or_select` at `:10716`, `begin_assessment` at
`:10743`, `freeze_result` at `:10819`, and archive projection at `:10754,10828`;
the journal methods also call one another internally. The counted path is
`main.rs:10856-10871` → `notify::push_counted_with_binding` →
`durable_delivery_runtime.rs:2284` → coordinator `prepare`. A final cutover
audit must repeat this inventory; any additional production writer outside
the fence disables sealing until moved under it.

## Chosen input cutoff contract

1. **One business date, one bounded input prefix.** Use the configured local
   business date, not `Local::now()` inside a reader. The normal selection
   opportunity remains 15:05:00 through 15:20:59 local. A v2 selection is the
   first successfully committed nonempty selection in that window, chosen
   from one strictly parsed, writer-fenced alert prefix and capped by
   `DEEP_ATTRIBUTION_MAX_EVENTS`. Save the input head generation, committed byte
   offset and prefix SHA-256, plus the exact ordered selected records, in the
   immutable v2 selection. Later appends are outside this batch and must be
   observable as late/excluded input; they do not silently expand the batch.
2. **Source writer contract.** Capture D once before choosing the lock and
   filenames, including for `append_batch` across midnight. Every production
   `alert_log` JSONL and Markdown append for D takes the same D fence as G5b
   journal writes. Before the first JSONL append, a
   durable D input head exists, including an explicit zero-generation head for
   a new day. Under the fence, append one complete canonical line, sync the
   file, then atomically publish and sync a new head containing generation,
   committed offset and prefix hash. The strict reader verifies the head,
   exact prefix bytes, newline termination, every row's schema and eligibility,
   and any suffix beyond the head. A missing head, missing/replaced file with
   nonzero head, malformed line, partial append, or unexplained suffix is
   `Unknown`, never an empty input. A crash between file sync and head publish
   leaves an uncommitted suffix that blocks sealing until explicit recovery.
3. **Empty day.** 15:21:00 local is only the earliest window-close check. If
   no v2 selection exists, acquire the D fence, strictly verify the durable
   input head and all committed rows through that instant, and persist an
   empty-input cutoff only if the eligible set is empty. Eligible rows with no
   selection (including provider outage) remain unresolved. A later append
   after the cutoff is recorded as excluded-late input. Never derive an empty
   seal from `read_today_records()` or from file absence alone.
4. **Historical boundary.** A v1 selection, pre-head alert file, legacy JSONL,
   or absent source provenance remains `Unknown` for automatic sealing. Keep
   the existing per-event observer and archive recovery available, but do not
   synthesize a v2 cutoff or a historical empty day from surviving bytes.

The input prefix defines the scope of this *selected-batch* seal. Product
policy must continue to surface alerts appended after that prefix; the seal
must not be presented as delivery of those excluded alerts.

## Cross-file and SQLite fence

The implementation must have **one per-date filesystem fence** honored by
every production `alert_log` append, selection creation, attempt marker,
frozen result, archive projection/recovery, and G5b counted admission. A writer
that bypasses it makes day sealing unsafe. Never hold this fence across an LLM
call or physical sink call. All operations needing both resources take the
fence before the durable SQLite connection/transaction; no reverse lock order
is permitted. The existing archive lock remains subordinate to the day fence.
Fence, head, selection and revision paths must be pinned as regular files and
reject symlinks or pathname replacement while held.

- A versioned day-journal revision records `Dirty(generation)` durably before
  any file mutation and `Clean(generation, digest)` only after the affected
  file and parent directory are synced. The digest covers the exact selection,
  ordered attempt/frozen identities and hashes, and full archive bytes. Crash
  with Dirty or a digest mismatch blocks sealing. Recovery may validate and
  republish Clean under the fence, but never reruns LLM or counted.
- A seal attempt holds the D fence while it strictly rechecks the input prefix
  and clean journal revision. It then starts **one SQLite `BEGIN IMMEDIATE`**
  transaction, reuses the G5b source/terminal validators *inside that same
  transaction*, enumerates every G5b decision for D (including orphan rows),
  reconciles exact occurrence/source/rendered hashes with every frozen selected
  event, and applies the outcome policy. The existing day reader must be
  factored to accept the caller's transaction; calling its separate read
  transaction cannot establish this boundary.
- In that transaction, insert an immutable day-seal row with the input head
  generation/offset/hash, selection digest, clean journal generation/digest,
  canonical counted-fact-set digest (including decision/disposition/receipt
  identities), policy version, outcome and audit identity. Check that the
  insert changed exactly one row. On a conflicting existing row, compare the
  exact stored evidence and return `AlreadySealed` only for equality;
  otherwise return `Conflict`. Commit while still holding the D fence.
- G5b counted **admission** must take the D fence *before* its SQLite prepare
  transaction, check the sealed-day row, and atomically claim a unique
  `(business_date, occurrence_identity)` owner. It releases the fence before
  sink I/O. A DB trigger or equivalent in-transaction guard also rejects a
  late G5b decision or terminal-evidence replacement after a seal. Journal
  writers check the sealed-day row under the D fence and refuse mutation.
  Pending and Uncertain decisions prevent sealing, so in-flight sink work
  cannot be mistaken for a terminal outcome.
- The current `CoordinatorConfig` (`src/durable_delivery/model.rs:80-101`)
  contains only the DB location, not a G5b fence capability. Introduce a
  typed, isolated-testable day-fence dependency or a private admission permit:
  generic `prepare` must reject G5b without that permit, and the G5b path
  acquires it before entering the coordinator's DB operation lease. Guarding
  only the monitor caller is insufficient because direct coordinator prepare
  remains callable. Never acquire the FS fence from inside a held DB lease.

This lock plus `BEGIN IMMEDIATE` gives a linearization point at the day-seal
commit: file writers and counted admission are fenced, and DB terminal writers
are serialized. The persisted digests allow later tamper detection; they do
not make arbitrary external file edits safe. Every production writer must be
identified and moved under the fence before this protocol is enabled.

## Outcome policy and durable schema

| Reconciled selected batch | Automatic action | Persisted label |
| --- | --- | --- |
| Every selected event exactly archived and has validated physical `Accepted` receipt | Eligible for automatic seal after input/fence checks | `PhysicalAccepted` |
| Verified zero eligible input at the post-window cutoff | Eligible for empty seal | `EmptyInput` |
| Any `NotStarted`, `CompletionUnproven`, missing archive/decision, `Pending`, or `Uncertain` | Keep open; no automatic LLM/counted retry | No seal row |
| Any `ManualAccepted` | Never label as physical Accepted | Only `OperatorReviewedManual` after a separate explicit, audited operator closure |
| Any `Rejected` or `ManualNotDelivered` | Never label as delivered | Only `OperatorReviewedNonDelivery` after a separate explicit, audited operator closure |

For mixed manual/negative outcomes, the operator closure retains each event's
validated terminal category and disposition identity. `Uncertain` must first
be resolved to a different validated terminal; an operator acknowledgement of
uncertainty alone cannot seal. Until the operator-closure contract is approved
and implemented, only `PhysicalAccepted` and `EmptyInput` are eligible.

Proposed durable schema migration: version 11 to 12 in
`src/durable_delivery/schema.rs` after the P-05 occurrence-owner v11 change,
with `g5b_day_seals` keyed by business date,
versioned evidence hashes/revisions, typed outcome, optional operator approval
identity, immutable audit reference, and seal timestamp. Add
`g5b_occurrence_owners` keyed by `(business_date, occurrence_identity)` and
linked to one decision. Activate owner claims only from a recorded future
business date, before that day's first G5b admission; a mid-day cutover with
existing decisions is refused. Historical G5b decisions remain readable but
are not automatically backfilled as owners or seals. Duplicates or malformed
historical rows remain Unknown and require controlled audit, never
first-row-wins. Old v1 days are not backfilled into `g5b_day_seals`.

Suggested interfaces: `capture_g5b_input_cutoff(D) -> InputCutoffV2`,
`with_g5b_day_fence(D, operation)`, transaction-scoped
`validate_g5b_day_facts(tx, D)`, `try_seal_g5b_day(D, expected_input_head,
expected_journal_revision, policy_request) -> New | AlreadySealed | Conflict |
Incomplete`, and `read_g5b_day_seal(D) -> Option<G5bDaySealV1>`. The
`policy_request` is physical-only by default; operator-reviewed requests
carry a separately persisted approval and exact terminal fact set. None of
these interfaces returns a generic delivery-success boolean.

## Ordered implementation and verification

1. **Input provenance.** Add explicit-date strict alert read and a per-day
   append head/fence, then v2 selection and empty cutoff records. Keep v1
   readable as Unknown. Test malformed/truncated lines, missing/replaced file,
   append-before-head crash, post-cutoff append, provider outage, and concurrent
   selection. Result: a verified fixed input prefix or a typed Unknown.
2. **Journal revision.** Route every selection/attempt/freeze/archive/recovery
   writer through the D fence and Dirty/Clean revision. Preserve exact JSONL
   bytes and attempt-first semantics. Test each crash point, concurrent
   recovery versus selection, lock-order behavior, and no LLM/counted replay.
   Result: a clean exact file digest, or a fail-closed dirty state.
3. **Durable owner and migration.** Add schema v12 day-seal and occurrence-owner
   tables, immutable guards, an activation-business-date gate, and G5b prepare
   admission check. Test duplicate historical rows stay Unknown without
   blocking unrelated kinds, mid-day activation refusal, same-occurrence
   concurrency, old-version startup policy, post-seal admission rejection,
   and migration rollback. Result: one counted owner per active-date
   occurrence and no post-seal new decision.
4. **Atomic seal API.** Refactor day validation to accept one caller-owned
   SQLite transaction; implement FS-fenced `BEGIN IMMEDIATE` reconciliation,
   physical/empty policy, exact one-row CAS and readback. Test writer-versus-
   seal interleavings, orphan/missing facts, Accepted versus ManualAccepted,
   damaged source/receipt, ambiguous commit and exact retry. Result: a
   persistent, evidence-bound seal or an explicit incomplete/conflict result.
5. **Scheduler integration after review.** Run costly inspections in a bounded
   blocking worker. Read the durable seal on restart; keep `G5B_LAST_RUN` as
   an in-process attempt throttle until its separate replacement is designed.
   Do not let a weak `PushOutcome`, JSONL existence, or the current observational
   verdict write the seal. Add operator-reviewed closure only after its policy
   and audit command have separate acceptance tests. Result: restart-safe day
   closure without automatic counted resends.

Use the smallest relevant targets per stage (`cargo test --lib g5b_ --
--test-threads=2`, the specific schema/migration test filter, and
`cargo test --bin monitor g5b_counted_binding_is_per_event_per_day` when the
producer changes), plus `rustfmt` and `git diff --check`. Expand to release
validation only during an authorized activation task.

## Crash, rollback and cutover gates

| Last durable boundary | Required recovery behavior |
| --- | --- |
| Alert line synced, head not published | Source suffix/head mismatch is Unknown; repair under D fence before cutoff. |
| Day revision Dirty, file mutation incomplete or complete | Revalidate files under D fence; no seal, Fresh LLM, or counted replay from ambiguity. |
| Attempt marker committed, no frozen result | `CompletionUnproven`; do not call LLM again. |
| Frozen result committed, archive missing | Archive recovery may project exact bytes; it cannot infer or invoke counted. |
| Archive committed, counted decision absent | `NoCountedDecision`; no automatic send based on JSONL. |
| Counted admitted or sink attempted, terminal receipt absent | Pending/Uncertain; leave day open for durable coordinator/manual reconciliation. |
| DB seal transaction outcome unknown | Read the exact CAS key and evidence hashes; identical row is idempotent, missing/conflicting row is not success. |
| DB seal committed, process crashes before updating memory | Restart reads the immutable seal row; `G5B_LAST_RUN` is irrelevant to durable closure. |
| Seal committed, later file/DB evidence differs | Raise integrity conflict; never silently rewrite the seal or claim physical delivery. |

Do not activate seal writes while any alert, journal, or counted-admission
writer can bypass the D fence. Deploy reader compatibility before the v12
migration: the current development source supports v11 and rejects a newer
schema. The rollback path after migration is to disable new seal writes while
preserving v12 rows and running a compatible binary; restoring an older binary
needs a controlled DB restore and cannot reinterpret sealed days. Keep legacy
v1 days and existing archive recovery intact throughout cutover. This document
performs no deployment, migration, or production read/write.
