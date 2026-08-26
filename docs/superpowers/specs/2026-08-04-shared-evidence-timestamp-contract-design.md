# BR-210 Shared Evidence Timestamp Contract

## Status

Gate A approved by the user's instruction to restore `monitor`, `--review`, and
`--test` as quickly as possible. Gate B/C/D remain pending.

## Problem

Magic providers preserve observation evidence in more than one unambiguous
instant encoding. The realtime quote gateway already accepts the exact Magic
Core contract, but monitor-side quote projection, announcement routing,
financial projection and consensus event construction reparse `BatchEvidence`
with RFC3339-only parsers. Real TDX integer seconds, Tencent/Sina fractional
seconds, Eastmoney `unix-ms:` and a CNInfo batch with
`observed_at=1785799979.851045000` are therefore rejected after source admission.

This is consumer parser drift. It is not missing provider data and must not be
repaired by replacing the provider observation time with `now`.

## Considered designs

1. **Shared exact parser (selected).** Move the BR-208 conversion into one
   data-gateway helper and use it from realtime quotes and announcements. This
   keeps one validation authority and preserves the raw evidence bytes.
2. Normalize all upstream provenance to RFC3339. Rejected because rewriting an
   immutable provider evidence value weakens the audit trail.
3. Add a second parser inside `v17_sources`. Rejected because duplicated parsing
   is the root cause and would allow the two consumers to drift again.

## Contract

`data_gateway::parse_evidence_instant` accepts exactly the instant encodings
admitted by the pinned `magic_market_core::EvidenceTimestamp` contract:

1. RFC3339 with `Z` or an explicit numeric offset;
2. unsigned Unix seconds;
3. unsigned Unix seconds with one-to-nine fractional digits;
4. `unix-ms:` followed by unsigned decimal milliseconds.

The helper validates with Magic Core first and then converts to
`chrono::DateTime<Utc>` without floating point. It receives the capability,
provider and field role so failures remain attributable. Empty, signed,
over-precision, ambiguous and out-of-range values fail closed as
`invalid_evidence`.

Consumers may convert the returned UTC instant to local time only after exact
parsing. The original `BatchEvidence.observed_at` remains unchanged for audit;
`source_at` and `observed_at` remain separate roles. No fallback, `now`, or
synthetic timestamp is permitted.

## Scope

This slice changes only:

- the shared evidence-time parser;
- the existing realtime quote parser call sites and regressions;
- monitor-side realtime quote projection;
- financial projection and its retained evidence checks;
- consensus source-event construction and latest-observation comparison;
- `route_announcement_batch` and its focused regression;
- BR-210 registration and validation evidence.

Review R-04/R-08/R-09 parsers were audited against their provider contracts and
are intentionally unchanged.

## Failure modes

- malformed/ambiguous/out-of-range evidence: explicit non-retryable
  `invalid_evidence`;
- valid announcement observation evidence: route every input to exactly one
  typed disposition;
- malformed empty batch evidence: record one batch failure while retaining zero
  input dispositions (no fabricated row);
- classification, audience, governance or sink failure after parsing: retain
  the existing BR-137/BR-138 disposition semantics;
- no provider data: remain unavailable; never manufacture an empty batch.

## Validation

- parser regression for `1785799979.851045000` and all BR-208 encodings;
- malformed/signed/over-precision/offset-free values remain rejected;
- announcement batch regression proves the production encoding is parsed and
  does not become a batch-wide timestamp failure;
- focused tests, `cargo fmt --check`, strict Clippy, workspace tests,
  `bash tools/compliance/check.sh`, and bounded real `monitor --review` run.

## Rollback

Revert the BR-210 helper, call-site changes, tests, design, plan and BR row as a
single slice. Provider order, freshness limits, source evidence, holdings,
orders and notification state are unchanged.
