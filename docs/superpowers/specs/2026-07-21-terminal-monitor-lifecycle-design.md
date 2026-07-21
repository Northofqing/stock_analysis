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
directory or cleanup failure returns `JsonlError` to the caller. The monitor logs
the error and exits 2. The receive loop owns only event consumption and per-event
append errors; there is no nested task whose `JoinError` can be mistaken for an
I/O failure.

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

## Tests and acceptance

- A process integration test removes `MONITOR_ENABLED`, runs isolated
  `--test --review`, and requires status 2, a strict-review marker, and no JSONL
  fatal/background-task marker.
- A disabled bare-monitor process returns 0 without creating the event-bus data
  directory.
- Writer unit tests await ready initialization and retain append/replay-filter and
  retention behavior.
- Focused tests, fmt, clippy, full workspace tests, compliance, coverage and
  release build must pass.
- A release canary repeats `monitor --test --review` without
  `MONITOR_ENABLED`; it must enter review, fail closed for absent real evidence,
  and never contact a real notification sink because test mode forces dry-run.

## Rollback

Revert the BR-141 merge commit and rebuild `monitor`. No data migration, account
mutation, audit deletion, process restart, or threshold rollback is required.
