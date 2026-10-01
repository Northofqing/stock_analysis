# M1 position concept source audit

Scope: the scheduled chain preparation's `position_concepts` input. This is a
read-only code audit; it does not admit this source for full shadow comparison.

## Existing operation

- `prepare_chain_analysis_with_io` skips the call when the positions query returns
  no rows. Otherwise it calls `position_concepts` once with position codes in
  position order. The trait default delegates to `concepts`; production
  `concepts` calls `fetch_concepts_cached`. The earlier limit-up concept stage
  invokes that fetcher separately.
- Each fetcher invocation calls `get_cached_concepts(7)` once. That DAO computes
  a local seven-day cutoff and queries **all** fresh `stock_concepts` rows, not
  just the requested codes. For each missing entry in the requested code list,
  the fetcher makes one `FetchSectorTool` call (up to six concurrently), then
  writes each successful result to the cache once. Duplicate position codes can
  therefore trigger duplicate calls and writes. A successful return covers every
  requested code with a nonempty concept list, but the returned map also holds
  unrelated fresh cache rows.
- `FetchSectorTool` receives a `GatewayBatch` and renders its `BatchEvidence`
  into JSON. `parse_tool_boards` extracts `all_boards` and discards that evidence.
  A `VerifiedEmpty` provider batch fails the tool call; it does not certify an
  empty concept result.

## Missing evidence and status boundary

- `get_cached_concepts` returns only `code -> concepts`. The query's exact
  cutoff, per-row `updated_at`, and read observation time do not reach the
  fetcher. The cache can contain values written by the earlier concept stage
  of the same preparation.
- The parsed provider result has no retained provider, batch ID, source time,
  observed time, or raw response digest. The final `HashMap` cannot distinguish
  cache hits from provider results. Hashing that map later would bind content
  only; it would not reconstruct those source identities.
- On a later provider or cache-write failure, earlier per-code writes may have
  occurred before the fetcher returns `Err`. No complete position-concept map
  is returned. This is a possible partial effect, not a verified empty batch.
- Empty positions mean **NotRequested** for position concepts. With nonempty
  positions, a successful result is neither an empty result nor proof of one
  upstream batch; failure must not be promoted to Available. The current
  `position_concept_source` remains Unknown after success and NotRequested
  before return. Those states do not establish M1 source admission.

## Narrow seam for a later implementation

Capture the existing cache query's cutoff, observation time, and exact rows
including `updated_at` in its one read. Retain each existing tool call's raw
result and validated `BatchEvidence` alongside parsed boards, with no second
provider call. Return the requested-code projection and its deterministic
content digest together with per-code cache/provider evidence and any partial
write outcome. The existing scalar `SourceObservation` cannot express this
mixed per-code provenance by itself; keep any incomplete outcome explicit and
preserve the legacy report and stop behavior.

Code inspected: `src/pipeline/chain_analysis/preparation.rs`,
`src/pipeline/chain_analysis/fetchers.rs`, `src/database/concepts.rs`, and
`src/agent/tools_sector.rs`.
