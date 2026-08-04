# Findings

## R-02 market review

- The template requires a single review-date market overview containing three
  indices, total turnover, and full-market breadth/limit-up/limit-down facts.
- The current `MarketAnalyzer` explicitly rejects market statistics because
  the unified Gateway does not expose a complete source-versioned A-share
  universe plus complete quotes and both limit pools in one settled batch.
- The partial index/technical snapshot cannot prove the missing breadth and
  turnover fields and therefore cannot restore R-02 under BR-093/140/164.

## R-05 signal review

- `execution_tracking` has no authoritative signal-delivery-execution-
  settlement lineage, and current production rows do not provide settled
  outcomes.
- Generic `paper_trades` rows cannot prove which confirmed push produced an
  execution or an effective T+N outcome.
- The required source is an append-only typed outcome dataset joining signal
  identity, confirmed delivery receipt, execution/trade identity, and settled
  result.

## R-06 failure attribution

- The existing failure-attribution module is a renderer over caller-created
  classifications, not an authoritative classifier/repository.
- Order rejection reasons and generic trade rows cannot be relabelled as
  strategy failures such as late entry, sector fade, or index drag.
- The required source is evidence-bound classified outcome data tied to the
  original signal, delivery, execution/non-execution, settled result, and
  market/sector context with classifier version.

## Decision

All three tasks must remain `Disabled`. The safe implementation improvement is
to remove the unnecessary partial R-02 acquisition and make every disabled
reason name its exact missing source contract.
