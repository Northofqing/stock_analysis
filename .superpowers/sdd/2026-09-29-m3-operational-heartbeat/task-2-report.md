# Task 2 report — independent resident heartbeat

## Change

- After acquiring and binding the singleton lease, the bare resident monitor makes one best-effort heartbeat write with that lease's immutable boot ID, then starts a separate 60-second Tokio timer. This happens before other startup work and the awaited startup health check/webhook. The loop only calls the Task 1 typed local writer; it never awaits account/data evaluation, banner storage, providers, or delivery sinks.
- `selection_cli.requires_service_enablement()` gates both the immediate write and timer. It checks zero explicit arguments; leased one-shot invocations, including `--test`, do not start a resident heartbeat. The existing BR-141 selection test verifies the corresponding bare-versus-argument gate.
- Write errors log under a fixed operational-health diagnostic on the first consecutive failure and every tenth consecutive failure. They do not stop the timer or monitor. Successful writes reset the count. The timer does not alter banner evaluation timestamps.
- A startup guard aborts its task if normal control leaves startup before service supervision. Supervision takes the handle into its existing background-task set and aborts/joins it on shutdown and writer/main-loop failure. Existing `std::process::exit` early exits terminate the entire runtime and task together.
- Exposed Task 1's explicit-path typed writer to sibling monitor module tests only (`pub(crate)`), allowing the scheduler harness to use an isolated temporary test path.

## Verification

- `cargo test --locked --offline --bin monitor operational_heartbeat` — 2 passed, 0 failed. Paused scheduler clock and injected UTC clock show file `observed_at` advances after a simulated writer failure while an evaluation callback remains pending and a separate evaluation fails; guard test shows startup drop aborts the timer.
- `cargo test --locked --offline --bin monitor br141_only_bare_monitor_requires_service_enablement` — 1 passed, 0 failed.
- `cargo test --locked --offline --bin monitor br141_supervisor_orders_signal_producer_stop_bus_close_and_writer_drain` — 1 passed, 0 failed.
- `git diff --check` — passed.

The first `cargo fmt --all` touched unrelated existing unformatted news and paper-ledger test lines; those formatting-only edits were restored. No full suite, release build, activation, production root, real provider, or webhook was used.

## Self-review and Task 3 gate

The immediate write and loop share the same immutable lease boot ID. The loop starts after lease binding and before startup awaits, skips missed timer ticks rather than writing a burst, and keeps I/O failure best-effort. The file can remain stale after repeated write failures; Task 3 must report that explicitly. Task 3 still needs the read-only `--health` heartbeat fields, freshness and live-lease boot-ID checks, text/JSON parity, and the additional overall-ok gate. No overall health status was changed here.
