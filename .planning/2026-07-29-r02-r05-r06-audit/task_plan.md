# R-02 / R-05 / R-06 production capability audit

## Goal

Audit the three production-review tasks currently disabled by BR-140, restore
only capabilities that can be backed by complete real-source evidence, and
leave exact upstream blockers for every capability that cannot be restored
without fabrication or semantic relabelling.

## Scope

- `src/bin/monitor/review_batch.rs`
- `src/bin/monitor/push_templates.rs`
- related unified Gateway consumers and focused tests
- review capability design evidence

Out of scope: CFFEX, global schema, legacy provider restoration, mock/default
data, and shared Cargo validation owned by the root session.

## Phases

1. [completed] Read repository rules and existing BR-093/110/140/164 design.
2. [completed] Trace each disabled task to its required evidence and available
   unified Gateway or persisted outcome source.
3. [completed] Record the capability decision and align runtime disabled
   reasons with the exact missing contracts.
4. [completed] Run static diff checks and hand validation commands plus blockers
   to the root agent.

## Definition of Done

- No disabled task is enabled from partial, inferred, stale, mock, or
  semantically unrelated data.
- Disabled reasons identify the missing authoritative batch/dataset.
- R-02 does not perform a partial live fetch after BR-140 has already
  determined that the full review contract is unavailable.
- Focused validation commands are handed to the root session without running
  Cargo in this parallel worker.

## Errors encountered

| Error | Attempt | Resolution |
| --- | --- | --- |
| Scoped `git diff` output was truncated by extensive pre-existing concurrent changes | 1 | Re-read exact line ranges and use focused `rg` plus `git diff --check`; no broad overwrite was performed. |
