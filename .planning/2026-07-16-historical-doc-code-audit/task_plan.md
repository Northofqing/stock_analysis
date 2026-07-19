# Task Plan: Historical documentation-to-code audit

## Goal
Determine, with reproducible evidence, whether every feature and bug described in repository documentation from the first commit through current HEAD is fully resolved in production code, tests, integration, and compliance gates.

## Current Phase
Phase 11

## Phases

### Phase 1: Scope and evidence inventory
- [x] Pin Git range and record dirty-worktree exclusions
- [x] Inventory current and historical/deleted documentation
- [x] Classify sources as requirements, bug reports, plans, completion claims, or non-actionable references
- [x] Establish an auditable requirement/bug ledger structure
- **Status:** complete

### Phase 2: Documentation claim extraction
- [x] Extract high-risk actionable feature and bug claims with source locations
- [x] Deduplicate superseded/repeated claims without losing provenance
- [x] Identify acceptance criteria and claimed completion status
- **Status:** complete

### Phase 3: Code and integration trace
- [x] Map high-risk claims to implementation and production call sites
- [x] Map claims to tests, failure-path coverage, migration/config evidence, and old-path removal/bridging
- [x] Check data red lines 2.1-2.10 and CLAUDE.md three-layer completion rule
- **Status:** complete

### Phase 4: Independent two-axis review
- [x] Run Standards audit against repository mandatory rules
- [x] Run Spec audit against the extracted claim ledger
- [x] Reconcile only factual conflicts while keeping both axes separate
- **Status:** complete

### Phase 5: Validation gates
- [x] Run safe static/build/test/compliance commands
- [x] Verify coverage and live-data evidence; do not fabricate or trigger unsafe live actions
- [x] Record blockers and distinguish pass, partial, unresolved, contradicted, and unverifiable
- **Status:** complete

### Phase 6: Report
- [x] Produce summary counts and high-risk findings
- [x] Provide a traceable matrix/report artifact
- [x] State whether "all completely resolved" is supportable
- **Status:** complete

### Phase 7: Exhaustive corpus manifest
- [x] Pin the new audit snapshot and preserve concurrent worktree changes
- [x] Inventory every current documentation and implementation artifact with hashes
- [x] Inventory deleted/renamed documentation reachable in Git history
- **Status:** complete

### Phase 8: Exhaustive actionable-claim ledger
- [x] Extract every feature, bug, acceptance criterion, TODO/deferred item, and completion claim as a high-recall candidate set
- [x] Assign stable claim IDs and retain source path/line/commit provenance
- [x] Normalize duplicates and supersession without dropping any source claim
- **Status:** complete

### Phase 9: Full implementation trace
- [x] Index all Rust, tests, scripts, configs, migrations, and CI files
- [x] Map every normalized claim to implementation, production caller, tests, and failure paths
- [x] Record negative searches and unverifiable mappings explicitly
- **Status:** complete

### Phase 10: Independent exhaustive review
- [x] Run Standards review across the full implementation corpus
- [x] Run early-document Spec review with ledger rows rather than sampled findings
- [x] Run late-document Spec review with ledger rows rather than sampled findings
- [x] Reconcile reviewer rows and audit coverage gaps
- **Status:** complete

### Phase 11: Gate validation and final exhaustive report
- [x] Run safe Gate B/C/D checks in an isolated snapshot
- [x] Delta-check repository changes after the pinned snapshot
- [x] Publish corpus coverage, claim status counts, complete ledger, and residual evidence limits
- **Status:** complete

## Key Questions
1. What counts as "all historical documents"? Working assumption: all documentation-like files reachable in Git history from the root commit through HEAD, including deleted/renamed files, plus actionable references in commit messages; external issue contents are included only if a configured tracker makes them retrievable.
2. What counts as "fully resolved"? Implementation exists, production integration exists, obsolete paths are removed/bridged, relevant tests and failure paths pass, mandatory compliance evidence exists, and no newer document reopens or contradicts the claim.
3. How are untestable historical assertions handled? Mark unverifiable, never pass by absence of evidence.

## Decisions Made
| Decision | Rationale |
|----------|-----------|
| Use root commit `3c7fad274a462a972bc5f6ef183d2119ef30a708` through HEAD `0d85dc5c74eb3655c825a35eaaa91825b6ca4725` | User requested all past documentation, not a single branch diff. |
| Extend the audit through pinned HEAD `c1e53321b2f4fb5d1f21cc0baf7ff4ade1ffcb7b` | The user requested a literal full-code, claim-by-claim audit after the initial high-risk/counterexample pass. |
| Preserve dirty files and exclude them from attributed HEAD evidence unless separately labelled | Existing changes belong to the user and are not committed historical proof. |
| Do not run the live monitor/test mode without proof it cannot send real notifications/orders | Fund/data safety outranks completeness convenience. |
| Skip issue-tracker setup mutation | The request is an evaluation; configuring external project infrastructure would expand scope. Missing tracker remains an evidence limitation. |

## Errors Encountered
| Error | Attempt | Resolution |
|-------|---------|------------|
| `.github/copilot-instructions.md` missing | 1 | Confirmed via `rg --files`; record as mandatory pre-flight evidence gap. |
| `docs/agents/issue-tracker.md` missing | 1 | Continue with repository Git/docs evidence; report external issue coverage as unverifiable. |
| `cargo fmt --check` failed on pinned HEAD | 1 | Record Gate B failure; do not autoformat because this is a read-only evaluation. |
| Compliance script returned green with 16 PENDING BRs and 155 zero fallbacks | 1 | Treat script exit 0 as script behavior only, not semantic red-line proof. |

## Constraints
- AGENTS.md rules 2.1-2.10 are blocking DoD criteria.
- No production-code changes are authorized by this evaluation request.
- Existing dirty files at the exhaustive-pass baseline: modified `src/event/mod.rs` and untracked `src/event/jsonl_writer.rs`; do not modify them.
- Repository HEAD advanced concurrently after the audit was pinned; all line-level findings remain tied to `0d85dc5c74eb3655c825a35eaaa91825b6ca4725`. Newer commits will receive a delta check before delivery.
