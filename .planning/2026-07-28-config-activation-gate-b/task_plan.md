# Task Plan: Config Activation Gate B

## Goal
Implement only the first real, fail-closed BR-179 Gate B vertical slice: the unique public
zero-argument process bootstrap, opaque non-forgeable parsed-CLI proof, real `args_os` ownership,
strict terminal/rejected/operational state machine, single `OnceLock` installation, and
production/test symbol isolation. Until the private global database factory exists, operational
startup must return a typed `Unavailable` error and must not install an incomplete success state.

## Current Phase
Phase 3

## Phases

### Phase 1: Requirements & Discovery
- [x] Read implement skill and mandatory repository rules
- [x] Read the complete approved Gate A design
- [x] Inventory existing bootstrap, database, activation, persistence and provider seams
- [x] Document exact reusable code and gaps in findings.md
- [x] Reconcile the pre-existing partial implementation against the independently approved Gate A
- **Status:** complete

### Phase 2: Planning & Structure
- [x] Restrict edits to process bootstrap/re-export/process tests
- [x] Define public-interface TDD behaviors in execution order
- [x] Identify the missing private global DB factory as an explicit typed startup blocker
- [x] Wait for the shared Cargo artifact lock owner before observing RED/GREEN
- **Status:** complete

### Phase 3: Implementation
- [x] RED→GREEN staged: unique public zero-argument facade owns the sole real `args_os` read
- [x] RED→GREEN staged: returned proof is opaque, non-Clone and cannot expose argv/mode/path/root/DB
- [x] RED→GREEN staged: exact help/version install storage-free terminal states
- [x] RED→GREEN staged: invalid argv installs storage-free rejected state and returns typed error
- [x] RED→GREEN staged: operational argv returns typed `Unavailable` without resource creation
- [x] RED→GREEN staged: every second bootstrap call is rejected by one closed `OnceLock`
- [x] RED→GREEN staged: production/test symbol contracts are bound to the private parsed mode
- **Status:** in progress; Cargo execution intentionally waiting for the shared artifact lock

### Phase 4: Testing & Verification
- [x] Targeted unit tests (9/9)
- [x] Targeted child-process tests (4/4)
- [x] Targeted rustfmt and scoped diff hygiene
- [ ] Targeted strict Clippy/check (root owns next Cargo slot/full integration)
- [x] Static scan: exactly one `args_os`, no caller-controlled bootstrap authority
- [x] Scoped diff review and parent handoff
- **Status:** scoped tests complete; repository-level strict gate remains with root

### Phase 5: Delivery
- [x] Perform scoped independent code review
- [x] Address two Important review findings in the shared integration
- [x] Report implemented slice and explicit remaining blockers to parent
- **Status:** complete for assigned slice; full Gate B remains in progress

## Decisions Made
| Decision | Rationale |
|----------|-----------|
| Do not run Cargo until the artifact lock is released | Parent explicitly reserved the shared target lock for another parallel lane |
| Never treat the existing direct DB init as bootstrap success | It does not implement the Gate A global lease/catalog/receipt factory and would violate fail-closed startup |
| Store terminal/rejected attempts in the same `OnceLock` | Gate A requires one atomically installed closed process state and fatal repeat calls |
| Keep any argv-slice parser helper module-private | Gate A permits it only for pure unit tests; executable behavior uses real child argv |

## Errors Encountered
| Error | Resolution |
|-------|------------|
| Planning-file context match failed once | Re-read the isolated files and retried with narrower context |
| Pre-existing Gate B plan overstated implementation completion | Reconciled it to the approved Gate A and the parent's narrower first-slice assignment |
| Removing the invalid path-bearing binding leaves `database/mod.rs::init_selection_bound` referring to the removed type | Parent retained ownership of this integration deletion; report as an explicit compile blocker until root removes it |
