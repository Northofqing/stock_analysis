# Task Plan: 2026-08-18 Opening Data Recovery

## Goal
Before the 2026-08-18 A-share open, make the production monitor consume the
real `client-bundle` gRPC contract with complete provider/source/batch evidence,
while keeping missing, stale, unsupported, and partial data fail-closed.

## Current Phase
Phase 4

## Phases

### Phase 1: Requirements & Discovery (Gate A input)
- [x] Reproduce bare-monitor production failures from the existing single instance
- [x] Inventory the `client-bundle` contract without exposing credentials
- [x] Probe remote health/capabilities and compare with local 34-op bridge
- [x] Map the observed opening-critical consumers to required RPC/evidence fields
- [x] Select the consolidated design under the user's standing project authorization
- **Status:** completed

### Phase 2: Architecture & Implementation Plan (Gate A)
- [x] Write reviewable design with data flow, failure modes, old modules, and rollback
- [x] Register/confirm applicable business rules before logic changes
- [x] Self-review the design against code evidence and repository rules
- [x] Write tiny-step implementation plan with regression seams
- **Status:** completed

### Phase 3: Implementation (Gate B)
- [x] Add production-safe client-bundle transport/config loading (mTLS + Bearer, no secret logs)
- [x] Close the SecurityIdentity and blocking-board no-feature call-path gaps
- [x] Fix server/bridge classification and source/batch propagation defects in scope
- [x] Add regression tests for remote transport, contract compatibility, and consumers
- [x] Finish identity request/freshness admission and integrated monitor build
- **Status:** completed

### Phase 4: Compliance & Verification (Gate C/D)
- [x] Run targeted tests and authenticated ExternalV1 live read-only probes
- [ ] Run fmt, strict clippy, full tests, compliance, coverage, and release build
- [ ] Verify no mock/fallback, fresh evidence, audit trace, and secrets absent from output
- [ ] Independently review changes and record PR evidence fields
- **Status:** in_progress

### Phase 5: Production Cutover & Opening Readiness
- [ ] Preserve old release and prove single-instance deployment procedure
- [ ] Stop old processes gracefully, start compatible data service/monitor, verify health
- [ ] Run real read-only canary for quotes, metadata, news, boards, flows, and order books
- [ ] Confirm process survives and alert/push governance remains audited
- [ ] Hand off exact readiness status, residual disabled capabilities, and rollback
- **Status:** pending

## Decisions Made
| Decision | Rationale |
|----------|-----------|
| Keep the current release monitor running until Gate C passes | Avoid a blind outage or duplicate production instances. |
| Treat `client-bundle` as private deployment input, never a tracked source asset | It contains a client private key and Bearer token. |
| Do not turn stale/partial data into opening decisions | AGENTS 2.2/2.3/2.4 require explicit fail-closed behavior. |
| Use the remote bundle as the authenticated upstream and keep normalization/evidence enforcement at the local boundary | This avoids changing every monitor consumer and preserves a single fail-closed contract. |

## Errors Encountered
| Error | Resolution |
|-------|------------|
| `.codex` planning skill link lacked helper scripts | Switched to the complete `.agents/skills/planning-with-files` installation. |
| `ps`/`pgrep` blocked by sandbox | Used approved escalated read-only process inspection. |
