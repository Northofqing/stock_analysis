# BR-208 Realtime Evidence Timestamp Contract

## Status

Gate C passed for the focused realtime-quote timestamp compatibility repair.
Gate D remains pending on repository-wide coverage and independent live
market-session evidence; the bounded closed-session production run is green.

## Problem

`magic-tencent-rs` emits observation evidence as unsigned Unix seconds with an
optional one-to-nine digit fractional nanosecond component. The pinned
`magic-market-core::EvidenceTimestamp::parse_instant` contract admits that
unambiguous representation, and `magic-market-router` therefore accepts the
batch. `stock_analysis::data_gateway::market_data` currently reparses the same
provenance as RFC3339 only. A valid batch can consequently cross the upstream
router and then fail locally as `invalid_evidence`, leaving paper valuation
without a quote.

The production example that exposed the mismatch was
`1785792189.398743000`. It is observation evidence, not provider source time;
the latter remains separately required and subject to the five-second gate.

## Contract

The realtime market-data boundary accepts exactly the unambiguous instant
encodings already admitted by Magic Core:

1. RFC3339 with `Z` or an explicit numeric offset;
2. unsigned Unix seconds;
3. unsigned Unix seconds plus one-to-nine fractional digits;
4. `unix-ms:` followed by unsigned decimal milliseconds.

All accepted forms are converted to `chrono::DateTime<Utc>` without using a
floating-point intermediate. Empty, signed, over-precision, non-decimal,
date-only, offset-free wall-clock and out-of-range values remain explicit
`invalid_evidence` failures. No missing timestamp is filled and no source time
is substituted with observation time.

The existing source/batch/record equality checks and the exact realtime
freshness interval `0..=5_000ms` remain unchanged.

## Failure modes

- malformed or ambiguous evidence: fail closed as `invalid_evidence`;
- integer/nanosecond overflow or unsupported chrono range: fail closed as
  `invalid_evidence`;
- valid timestamp older than five seconds or in the future: retain the existing
  explicit freshness rejection;
- record/provenance mismatch: retain the existing explicit evidence rejection.

## Validation

- a regression using the observed production epoch value must fail before the
  repair and pass afterward;
- RFC3339 behavior and malformed-value rejection remain covered;
- focused gateway tests, format, strict Clippy, full workspace tests and the
  repository compliance suite must pass;
- a bounded normal monitor run must no longer report this valid epoch as an
  invalid `observed_at` timestamp.

## Rollback

Revert the BR-208 parser branch, its tests, this design and the BR row together.
No upstream dependency, provider order, freshness threshold, holding, order or
notification state is changed.
