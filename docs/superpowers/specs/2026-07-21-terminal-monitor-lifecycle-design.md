# Terminal Monitor Lifecycle Design

**Date:** 2026-07-21

**Rule:** BR-141

**Data red lines:** 2.1, 2.2, 2.5, 2.7, 2.8, 2.10

## Problem and evidence

`monitor --test --review` currently returns status 0 when `MONITOR_ENABLED` is
unset. It never initializes the database or enters strict review. The process has
already spawned the event JSONL writer, so Tokio runtime shutdown can cancel the
writer's blocking filesystem initialization and emit
`I/O error: background task failed` after the command has falsely succeeded.

The same binary with `MONITOR_ENABLED=true` enters strict review and returns 2
when the isolated database lacks real account evidence. This controlled
comparison proves that the global enable gate, not review data transport, is the
short circuit. Git history also shows the JSONL writer was placed before the
global enable check and wrapped in an unnecessary second `tokio::spawn`.

## Decision

`MONITOR_ENABLED` is a service-lifecycle switch, not a CLI authorization switch.
It applies only when the process has no user arguments and would enter the
long-running loops. Every explicit argument must reach normal parsing and either
execute its terminal command or return an explicit CLI error.

The enable check remains early, but moves before event-bus subscription and
JSONL writer startup. A disabled bare monitor logs one aggregate lifecycle line,
returns without runtime data creation, and does not initialize a database or
notification sink.

`JsonlWriter::spawn` becomes an async ready boundary. It creates the base
directory and performs retention cleanup before spawning the receive loop. A
directory or cleanup failure returns `JsonlError` to the caller. The receive
task returns `Result<(), JsonlError>`: an envelope write failure or receiver lag
is terminal evidence loss, not a warning that permits false success.

The monitor owns the single `JoinHandle<Result<(), JsonlError>>`. Every explicit
terminal path that starts runtime persistence closes the global event bus and
awaits this handle before selecting the process status. A writer or join failure
overrides the requested status with exit 2. `run_review_only` returns a typed
error to this owner rather than calling `process::exit` below the lifecycle seam.
The await is bounded to ten seconds; timeout aborts the writer task and exits 2
so an external supervisor can recover rather than hanging forever. Long-running
shutdown first cancels and awaits every owned producer task, then applies the
same bus-close/writer-drain step. This makes the module deep:
callers learn one ready start contract and one awaited completion result, while
filesystem and broadcast failure details remain local to the writer.

The long-running service also selects on the writer handle alongside its market
loops and SIGINT. A writer error, join error, or unexpected clean writer stop
before bus shutdown is a terminal audit-health failure and exits 2 immediately;
the service must not continue trading while event evidence is no longer being
persisted.

Because shutdown now runs in production on a four-worker Tokio runtime, the
event bus sender slot must be synchronized. `UnsafeCell<Option<Sender>>` with a
manual `Send + Sync` assertion is rejected: publish can pass the atomic flag and
race a concurrent `take()`, which is undefined behavior and can also panic.
The sender slot uses `std::sync::RwLock<Option<Sender>>`; publish/subscribe/count
hold a read guard and shutdown takes the write guard. A publisher that reaches
the slot after shutdown receives `Rejected(ShuttingDown)` without dereferencing
an empty option.

No explicit CLI argument may fall through to the long-running service. Existing
one-shot flags (`--push-dry-run` and the registered backfill flags) remain known
to the event parser and reach their handlers. Test-only diagnostics require
`--test`; `--v13-diag` without it exits 2 before runtime initialization. A final
defensive guard rejects any otherwise-unhandled explicit argument instead of
entering market loops without `MONITOR_ENABLED`.

History is a read-only terminal command but still follows truthful status:
corrupt JSONL or statistics input exits 1. Invalid explicit arguments likewise
reach the parser instead of the service-enable gate.

## Alternatives rejected

- Setting `MONITOR_ENABLED=true` internally for test/review hides the misplaced
  gate and mutates operator configuration semantics.
- Special-casing only `--test --review` leaves production `--review`, history,
  replay, and invalid arguments vulnerable to the same silent exit.
- Ignoring the writer shutdown error preserves a false-success command and loses
  an explicit initialization failure path.

## Data and failure flow

```text
argv
  -> no user args + MONITOR_ENABLED != true -> disabled bare exit (no writer/DB)
  -> explicit args -> normal CLI/test/review path
       -> JsonlWriter::spawn().await
          -> create directory
          -> retention cleanup
          -> return consumer JoinHandle
       -> initialization error -> log + exit 2
       -> command outcome controls the truthful terminal status
```

No production data source, account field, threshold, order path, notification
policy, retention duration, or JSONL record format changes.

The event JSONL file is explicitly a non-authoritative observation/replay
projection. It does not claim to satisfy red line 2.7 by itself. Every production
delivery envelope reaches it only after `AuditDispatcher` has appended and
`sync_data`-committed the existing SHA-256 chained authoritative delivery audit;
orders and other critical audit owners remain unchanged. Writer failure is still
terminal because continuing without the declared replay projection is an
operational integrity failure, but a successful writer result is not presented
as WORM/tamper-resistance evidence.

## Tests and acceptance

- A process integration test removes `MONITOR_ENABLED`, runs isolated
  `--test --review`, and requires status 2, a strict-review marker, and no JSONL
  fatal/background-task marker.
- Process tests remove `EVENT_AUDIT_DIR` and `PUSH_LOG_DIR` (or point them under
  the isolated root), so inherited operator configuration cannot write test
  evidence into production paths.
- A disabled bare-monitor process returns 0 without creating the event-bus data
  directory.
- Writer unit tests await ready initialization, retain append/replay-filter and
  retention behavior, and cover directory initialization failure, retention
  cleanup failure, envelope write failure, and deterministic receiver lag.
- A corrupt isolated history JSONL file returns exit 1; a writer initialization
  failure at monitor startup returns exit 2.
- A monitor lifecycle unit test proves a completed writer error closes the bus
  and is returned to the process owner; the long-running select treats that
  completion as exit 2 instead of waiting until a later shutdown.
- A concurrent publish/shutdown stress test proves that every publish is either
  delivered/no-subscriber before the synchronized close or explicitly rejected
  afterwards, with no panic or unsafe sender access.
- Process tests require test-only diagnostics without `--test` to exit 2 before
  creating runtime data, and require an unhandled explicit mode never to enter
  long-running service loops.
- Terminal writer drain timeout and the writer completion state machine cover
  clean unexpected stop, writer error, join error, and timeout. Owned background
  tasks are aborted and joined before bus close; direct nested producers are
  removed so no producer handle is detached across shutdown.
- Focused tests, fmt, clippy, full workspace tests, compliance, coverage and
  release build must pass.
- A release canary repeats `monitor --test --review` without
  `MONITOR_ENABLED`; it must enter review, fail closed for absent real evidence,
  and never contact a real notification sink because test mode forces dry-run.

## Rollback

Revert the BR-141 merge commit and rebuild `monitor`. No data migration, account
mutation, audit deletion, process restart, or threshold rollback is required.
