# G5b completion evidence boundary (2026-10-01)

## Current owners

- `G5B_LAST_RUN` in `src/bin/monitor/main.rs` is an in-process date gate for a
  selected monitor batch. Once that batch enters the event loop, the gate is
  set when the loop ends without a journal I/O failure, even if an LLM request
  failed, a Frozen replay only restored JSONL, or counted delivery returned
  Denied or SinkError. `done` counts analyzed and frozen rows after the counted
  call; it is not a delivery count. Restart clears the date gate.
- Frozen result files and exact JSONL archive bytes prove only that the LLM
  result was saved and projected. Historical recovery restores the archive
  without retrying counted delivery.
- The durable SQLite coordinator owns counted decisions. Its G5b day reader
  validates every decision for the business date in one read transaction,
  including decisions outside the saved selection, and distinguishes physical
  Accepted from ManualAccepted, Rejected, Uncertain, and ManualNotDelivered.

## Read-only state machine

The selected-event inspector first verifies the saved selection, attempt and
frozen files, and exact archive bytes. It then reconciles every same-day G5b
SQLite decision with the selection and frozen source/rendered hashes. Orphans,
malformed evidence, and mismatches return an error; they have no verdict.

| Observed evidence | Event state | Day classification |
| --- | --- | --- |
| No selection artifact and no G5b decision | — | `NoSelection` |
| Selected, no attempt | `NotStarted` | `Incomplete` |
| Attempt, no validated frozen result | `CompletionUnproven` | `Incomplete`; never rerun the LLM from this observation |
| Frozen, no counted decision | `NoCountedDecision` | `Incomplete`; JSONL is not delivery proof |
| Counted decision without verified terminal evidence | `Pending` | `Incomplete` |
| Counted uncertain/manual-review evidence | `Uncertain` | `Incomplete`; requires separate resolution |
| Verified physical receipt | `Accepted` | Count physical Accepted separately |
| Verified manual resolution | `ManualAccepted` | Count manual acceptance separately; not physical Accepted |
| Verified rejection or manual non-delivery | `Rejected` / `ManualNotDelivered` | Count negative terminal outcomes separately |
| Any selected event without exact JSONL archive | Existing event state | `Incomplete`, including when counted is Accepted |

When every selected event has an exact archive and a terminal outcome other
than Uncertain, the pure classifier returns `TerminalOutcomesObserved` with
separate outcome counts. It does not return a success or completion boolean.

## Why this does not seal a business day

The filesystem inspection and the SQLite transaction are sequential, not one
atomic snapshot. `NoSelection` also does not prove that the day's alert input
is closed. A concurrent journal change or newly eligible alert can invalidate
the observation. Neither `TerminalOutcomesObserved` nor any count authorizes
writing `G5B_LAST_RUN`, retrying counted, or claiming user delivery.

A future durable day seal needs a defined input cutoff and immutable selected
identity set, a cross-medium fence or equivalent revision check for journal
files and SQLite decisions, an explicit policy for manual/negative outcomes,
and a persisted compare-and-set completion record. The current in-process
date gate remains an attempt scheduler, not that record.
