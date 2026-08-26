# BR-207 Review Quiet-Hour Retry Design

**Status:** Gate A design; implementation is limited to review outcome
classification.

## Problem

The notification governor correctly denies non-emergency messages from 02:00
through 05:59. A strict review dispatcher currently converts every governance
denial into a non-retryable `Failed` result. The review scheduler consequently
marks a source-complete task terminal even when the sole reason is
`quiet_hour`. This preserves notification silence but loses the task's delivery
eligibility.

## Decision

`PushOutcome::Denied("quiet_hour")` remains a denied delivery and must perform
no sink call. At the review outcome boundary it is classified as a retryable
existing-source failure. All other governance denials retain their existing
non-retryable classification, and sink errors retain their existing retryable
classification.

The change does not bypass L5, alter the 02:00–06:00 window, infer data, change
source freshness, or claim a delivery. It only prevents a transient time gate
from becoming a terminal review-task state. A later attempt must pass the same
governor and all ordinary delivery/audit gates.

## Data flow

1. A review dispatcher acquires and validates its source batch.
2. L5 returns `Denied("quiet_hour")` before the sink.
3. The shared review outcome converter returns retryable `Failed`.
4. BR-140 schedules the next bounded retry and appends the ordinary task
   transition audit.
5. A later attempt reacquires/revalidates according to the task's existing
   contract and may deliver only after L5 approval.

## Failure modes

- Unknown or policy denial: terminal, unchanged.
- Quiet-hour denial: retryable, no delivery claimed.
- Sink failure: retryable, unchanged.
- Deduplication: terminal, unchanged.
- Audit persistence failure: propagated by the existing review runner.

## Old modules

| module | decision | reason |
| --- | --- | --- |
| L5 quiet-hour governor | adopt unchanged | It is the authority that prevents disturbing delivery. |
| `ReviewTaskOutcome::from_push_outcome` | amend | It owns the shared review classification seam. |
| Per-task dispatcher special cases | reject | Duplicated classification could drift across R/A tasks. |

## Validation and rollback

Add a unit regression proving quiet-hour denial is retryable while another
denial remains terminal. Run formatting, clippy, workspace tests, compliance,
then the strict review and isolated template-test commands. Rollback is the
single BR-207 converter branch, its regression, this design, and the BR row;
the underlying quiet-hour governor is never changed.
