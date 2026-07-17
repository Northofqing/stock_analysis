# Findings: Event Replay Safety Remediation

- Existing CLI and replay tests pass but cover only happy paths.
- `parse_args` returns `Ok(None)` immediately on known monitor flags, so event command composition is order-dependent.
- The documented replay-rate syntax uses `=N`; the only test uses a separate value token.
- `limit=0` is defined as explicit unbounded mode in v17.3 but CLI rejects `<= 0`.
- `ReplayRunner::run` names the rate parameter `_rate_ms` and never reads it.
- Replay directly deserializes `EventEnvelope`, bypassing `ReplayablePushEvent::validate`.
- Publish count increments before `EventBus::publish`; failure outcomes are logging-only.
- Fresh IDs use a counter local to one `run`, so identical source files repeat IDs across runs.
- Monitor uses `runner.run(...).await.unwrap_or(0)`, collapsing replay errors into a zero-success exit.
- `HistoryQuery` also rewrote zero to the default 100, so the unbounded contract required both parser and query-layer fixes.
- Existing `generate_trace_id` demonstrates the repository's process-wide `AtomicU64` pattern.
- Replay unit tests reused the same temp path within a process, which hid behavior behind parallel file races once coverage expanded.
- History tests had the same process-wide temp-path collision; its success-rate fixture also used local noon, which is outside a trailing 24-hour window when the test runs before noon.
- The mandatory daily-data backfill needs external network access; sandboxed providers all returned empty while the approved non-sandbox run succeeded for every symbol.
- Changed-file coverage exceeds 80% and replay exceeds 95%, but the repository-wide 51.14% coverage prevents Gate D / Release Ready status under AGENTS.md Part 4.
- A broadcast `Published` outcome only proves a receiver exists; force replay needs an awaited publisher boundary to distinguish actual sink acceptance from queue admission.
- `limit=0` must be checked at parser, query, and presentation layers; a fixed `.take(20)` in the CLI can silently reintroduce an output limit after an unbounded query.
