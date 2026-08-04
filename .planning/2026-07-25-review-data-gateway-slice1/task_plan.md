# A-01 / R-03 Review Data Gateway Slice 1

## Goal

Wire the production `monitor --review` A-01 and R-03 paths through a small
`stock_analysis::data_gateway` seam backed by the adjacent
`magic-market-data-rs` 0.2.0 contracts. Preserve provider/source timestamps and
batch identity, distinguish verified-empty from unavailable, and keep all
review task IDs, PushKinds, audit event types, and public payloads unchanged.

## Scope constraints

- Do not touch NewsAI or test-isolation files.
- Do not delete old acquisition code until multiline-aware caller evidence is
  zero and real-date parity is proven.
- Do not use production mocks, defaults, zeros, or fabricated timestamps.
- Do not commit.

## Phases

1. **Gate A evidence and focused design** — complete
   - Recompute A-01/R-03 callers and upstream API facts.
   - Add/reconcile BR-158 and BR-159 before implementation.
   - Write the focused Slice 1 design and rollback/failure behavior.
2. **Tracer 1: gateway result/evidence contract** — in progress
   - RED test for verified batch vs verified-empty vs unavailable.
   - Minimal public Gateway types and validation.
3. **Tracer 2: A-01 daily bars** — pending
   - RED behavior test.
   - Magic Bars provider/router adapter and production A-01 call.
4. **Tracer 3: R-03 limit pool** — pending
   - RED behavior test.
   - Magic LimitPool provider/router adapter and monitored-universe filter.
5. **Integration and old-path disposition** — pending
   - Enumerate every changed caller.
   - Keep legacy acquisition code unless zero-caller + parity conditions hold.
6. **Gate B/C validation and production evidence** — pending
   - Targeted tests, build, fmt, strict Clippy, diff check.
   - Release `monitor --test` / `monitor --review` Gateway log evidence when
     real data is available; otherwise record explicit blocked evidence.

## Errors encountered

| Error | Attempt | Resolution |
| --- | --- | --- |
| None | 0 | — |
