# BR-183 Business-First Selection Capability Gate

**Status:** Gate A approved

**Date:** 2026-07-29

**Business rule:** BR-183

**Data red lines:** 2.1, 2.2, 2.4, 2.5, 2.7, 2.8, 2.10

## 1. Problem

The monitor currently treats an unavailable selection-v2 activation as a
process-wide startup failure. The checked-in selection proposal, verified
provider-board artifact, activation receipt and amended schema are not yet a
complete production release. Consequently `cargo run --bin monitor -- --test`,
`cargo run --bin monitor -- --review`, and an enabled normal monitor all exit
before the already implemented core business services can run.

That coupling does not make the incomplete selection capability safer. It
only makes unrelated monitoring, review, news, portfolio and notification
capabilities unavailable. The selection capability must remain fail-closed,
but its unavailable state must not be promoted to a failure of the whole
process.

## 2. Decision

Use a capability-scoped state:

```text
SelectionCapability
├── Active(opaque verified binding)
└── Disabled(stable reason code)
```

The zero-argument BR-179 facade remains the only owner of real argv parsing.
It classifies help, version, invalid input, service-disabled, test, review and
normal operation exactly once. It then returns an opaque proof that contains
only the minimum public decisions needed by the binary:

- whether the process is terminal or operational;
- whether selection-v2 is active;
- a stable non-sensitive disabled reason for one startup summary.

Until the full BR-179 schema/config/provider/sink proof is implemented and
released, every operational mode returns an explicit selection-v2
`Disabled(selection_v2_activation_not_released)` state. This is a compile-time
truthful release state, not an environment override and not a substitute data
source.

The core monitor is allowed to continue from the same verified operational
CLI classification. It must not infer mode from an environment variable,
CWD, filename, database contents or a caller-supplied argument.

### 2.1 Core database continuity while selection-v2 is disabled

BR-183 separates the unreleased selection-v2 capability from the existing
core-business database. Disabling selection-v2 must not remove the
`DatabaseManager` installation required by account governance, review,
backfill and the isolated E2E command.

Until the BR-179 `STSA/1` cutover is released, the core database keeps the
existing migration owner and uses only mode-owned identities:

- production is fixed to
  `${CARGO_MANIFEST_DIR}/data/stock_analysis.db`;
- exact test mode allocates an invocation-unique directory named
  `TEST_CODE_monitor_<pid>_<nonce>` under the operating-system temporary
  directory and uses its `stock_analysis.db`;
- caller `DATABASE_PATH`, `.env` database defaults, `MAGICLAW_DB_PATH`, CWD
  and filename inputs do not choose either identity;
- the selected identity is installed into `DATABASE_PATH` only for existing
  core consumers after mode classification, then `DatabaseManager::init`
  must complete before any health check, repository, review or backfill;
- test mode also binds `MAGICLAW_DB_PATH` to the isolated database so no
  production sink/database route can leak through a caller environment.

This is not a selection fallback: the resulting `DatabaseManager` has no
selection schema authority, and every selection-v2 call remains guarded by
the disabled capability. The later BR-179 cutover replaces this temporary
core continuity seam only after the exact global schema capability is
available.

## 3. Considered Options

### A. Keep the process-wide fatal gate

This is the current behavior. It preserves selection-v2 safety but makes all
business functions unavailable whenever one unreleased capability is
unavailable. Rejected because it violates the business-availability priority
without adding a stronger selection proof.

### B. Capability-scoped fail-closed

Selected. Core business starts; selection-v2 remains explicitly disabled.
No selection-v2 provider, repository, audit writer, sink or scheduler exists
in the disabled branch.

### C. Environment-variable bypass

Rejected. A bypass would allow runtime input to overrule a safety decision,
would be difficult to audit, and could silently re-enable legacy/fake paths.

## 4. Startup Data Flow

```text
read real argv once
→ strict parse/classification
→ help/version/invalid/service-disabled terminal handling
→ operational core proof
→ selection release-state decision
   ├── Active: consume BR-179 opaque verified capability
   └── Disabled: record stable reason; construct no selection resources
→ install the mode-owned core database
→ initialize remaining core business resources
→ start command-specific core business flow
→ start selection scheduler only when Active
```

The disabled path must produce exactly one summary equivalent to:

```text
[selection-v2][BR-183] capability=disabled
reason_code=selection_v2_activation_not_released
providers=0 database_operations=0 sinks=0 schedulers=0
```

It must not claim a verified schema/config/provider or a successful empty
selection result.

## 5. Mode Matrix

| Invocation | Core business | selection-v2 | Storage before classification |
| --- | --- | --- | --- |
| `--help`, `-h`, `--version`, `-V` | terminal success | not constructed | zero |
| invalid CLI | terminal error | not constructed | zero |
| bare argv with service disabled | terminal success | not constructed | zero |
| exact `--test` | runs in existing TEST_CODE isolation | Disabled until real test activation exists | zero |
| `--test --push-dry-run` | runs the complete BR-196 monitor-card catalog dry-run in TEST_CODE isolation, then exits | Disabled until real test activation exists | zero |
| exact `--review` | runs production review | Disabled until real production activation exists | zero |
| bare argv with service enabled | runs normal monitor | Disabled until real production activation exists | zero |

BR-183 does not relax test/live order isolation. `--test` must continue to
reject real-symbol orders and production must continue to reject TEST_CODE
orders. Explicit terminal actions take precedence over the generic `--test`
review fallback: the test flag selects isolation only and must not replace the
requested terminal action.

## 6. Failure Modes

- CLI ambiguity remains a process error before storage or provider creation.
- A recognized terminal action shadowed by the generic test fallback is a
  process-routing failure and a release blocker.
- Core configuration, database, audit or provider failures remain their
  existing explicit failures. BR-183 does not turn them into success.
- A missing or failed mode-owned core database installation is a terminal
  startup error; no health check or database consumer may run first.
- Caller database environment values are ignored for identity selection.
- Selection-v2 disabled is an explicit capability state, not an error retry
  loop and not `VerifiedEmpty`.
- An attempted selection-v2 repository/provider/sink/scheduler construction
  in the disabled branch is a test and release blocker.
- A future Active state without the complete opaque BR-179 proof is a test
  and release blocker.
- No old selection source, embedded sample, default value, mock or cross-source
  field splice may replace the disabled capability.

## 7. Existing Modules

| Module | Decision | Reason |
| --- | --- | --- |
| `selection::process_bootstrap` | adopt and narrow | remains the sole argv owner; gains capability state |
| `bin/monitor/main.rs` | adopt | core startup continues after disabled summary and installs a mode-owned core database before consumers |
| `database::DatabaseManager::init` | temporarily retain for core continuity | production remains on the existing legacy `0/0` business schema until the offline BR-179 `STSA/1` cutover is complete |
| selection-v2 outcome/generation schedulers | guard | start only for Active |
| legacy selection shadow/provider paths | reject as fallback | disabled must remain zero-call |
| `config_activation_v2` and global schema owner | retain | required for a later real Active proof |

## 8. Acceptance

Focused static and unit checks:

```bash
cargo fmt --all -- --check
cargo test --test selection_process_bootstrap_isolation -- --test-threads=1
cargo test --test monitor_help_isolation -- --test-threads=1
cargo check --bin monitor
bash tools/compliance/lib/check_business_rules.sh
```

Bounded command validation:

```bash
cargo run --bin monitor -- --test
cargo run --bin monitor -- --review
cargo run --bin monitor
```

Evidence must show that all three operational invocations pass the old
selection process-wide rejection point, print the BR-183 disabled summary
once and reach their command-specific core business startup. Test validation
must use existing TEST_CODE/no-production-sink controls. Review and normal
validation may report genuine external-data failures, but must not fail merely
because selection-v2 is disabled.

Before release the repository-wide gates remain mandatory:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features -- --test-threads=1
bash tools/compliance/check.sh
```

Coverage and live validation remain Gate D evidence; passing the focused
startup checks alone does not make the unified migration complete.

## 9. Rollback

Before release, revert only the BR-183 implementation commit and rerun the
focused bootstrap tests. This intentionally restores the former process-wide
fatal gate without changing databases, activation artifacts, provider data or
schema. After release, rollback must use `git revert <exact-commit-sha>` and
must not delete or rewrite production data.
