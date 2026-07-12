# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```
cargo build                             # compilation
cargo test --lib                        # all unit tests (~289)
cargo run --bin monitor                 # live monitoring
cargo run --bin monitor -- --test       # full pipeline smoke test
cargo run --bin monitor -- --review     # manual post-market review
```

## Architecture (v3–v6)

The system is an **event-driven live trading monitor** for A-share (Chinese stocks), not a batch quant strategy.

**7 Contexts** (DDD bounded contexts, no Clean Architecture layers):

| Context | Directory | Job |
|---------|-----------|-----|
| Portfolio | `portfolio/` | Single source of truth for positions, trades, ledger |
| Market | `monitor/` + `data_provider/` + `market_analyzer/` | Quotes, announcements, detection |
| Signal | `signal/` | Unified Signal/SignalSet data structures |
| Opportunity | `opportunity/` | News → industry chain → candidate discovery |
| Review | `review/` | Daily/weekly post-trade review & falsification |
| Decision | `decision/` | Exclusion, sector tiering, capital verification, rotation |
| Risk | `risk/` | Hard position/sector/cash limits (parallel to monitor/risk.rs) |
| Breakout | `breakout/` | Multi-dimensional volume breakout analysis (v6) |

**Data sources** (multi-host fallback): Eastmoney push2 (3 hosts) → Sina → Yahoo. Flash news from Jin10 + WallStreetCN.

## Critical Rules (from AGENTS.md, MUST priority)

**Data**: All data must be real. No mock data in production paths. Missing data → log warning, don't silently fill. Failed data source → explicit error.

**Development flow**: `/architecture-patterns` → 4-angle challenge → `/project-planner` → code → `/review` (must check old modules!) → fix → test.

**When new capability is added**: Check whether existing old modules should upgrade to use it. Document the decision for each.

## Completion Rule (Hard Constraint) — Prevents "Code Without Integration" Hallucination

**Module code + unit tests passing ≠ completion.** This rule exists because an assistant can write 2700 lines of new modules with 1058 passing tests while the production binary (`src/bin/monitor/`) still uses the old path entirely. Both the module-level work and the integration into the live binary must be verified before claiming completion.

Every new capability, refactor, or module addition MUST satisfy ALL THREE conditions before any assistant says "✅ done":

### 1. Module layer
- Module code exists in the expected path (e.g. `src/push_l4/dispatcher.rs`).
- Unit tests in that module pass (`cargo test --lib <module>::`).
- `cargo build --lib` exits 0.

### 2. Integration into `src/bin/monitor/`
- The module is actually used by the production binary. Verify with:

  ```bash
  grep -RInE 'use stock_analysis::push_l[1-7]::|<module>::' src/bin/monitor/ \
      --include="*.rs"
  ```

  Result MUST show ≥ 1 import + ≥ 1 call site in `main.rs` / `push_templates.rs` / `notify.rs`. Zero hits = integration 0% = NOT complete.

- The old call path (`notify::push_governor(&text, PushKind::X)` etc.) that the new module replaces must be either removed or explicitly bridged. Stale dual paths are NOT acceptable.

### 3. Live-binary verification
- `cargo build --release --bin monitor` exits 0.
- `cargo run --release --bin monitor -- --test` runs end-to-end and the resulting push goes through the new module's path (visible in logs: `dispatcher.dispatch`, `governance.check`, `sink.route`, `analytics.record`, etc.).
- If the new module is supposed to gate or transform a push, the live test MUST show it did so (e.g. governance Deny actually blocks a push, dispatcher dedup actually skips a duplicate, analytics row actually written).

### Anti-patterns (auto-reject)
- "✅ W{N} done" when `src/bin/monitor/` has 0 references to the new module.
- "✅ integration complete" based only on `v14_e2e.rs` (an isolated test binary, not the production monitor).
- Reporting `cargo test --lib` as proof of completion without checking integration.
- "The release binary pushed successfully" without confirming the new module was on the path of that push.

### How to verify before claiming completion
Run this checklist and paste results in the summary:

```bash
# 1. Module layer
cargo test --lib <module_path>::                  # must pass
cargo build --lib                                  # must exit 0

# 2. Integration grep (zero hits = NOT complete)
grep -RInE 'use stock_analysis::<module>|<module>::' src/bin/monitor/ \
    --include="*.rs" | wc -l

# 3. Live binary
cargo build --release --bin monitor                # must exit 0
./target/release/monitor --test 2>&1 | grep -E \
    '<new_module_keywords>' | head -5               # must show ≥ 1 hit
```

If any of the three checks fails, the work is **not complete**, regardless of how many unit tests passed.

## Configuration

- `.env`: `STOCK_LIST` (watchlist codes), `WECHAT_SEND_SCRIPT`, `DATABASE_PATH`
- `config/*.toml`: chain rules, exclusion boards, announcement keywords, monitor timers — SIGHUP hot-reloadable
- All config files have code-level `const` fallbacks if toml is missing

# Tool Calling Rules
When calling tools, follow these rules strictly. They override any conflicting habits from chat training.
## Argument formatting
1. **Omit optional fields you don't need.** Do not send `null`, `""`, `{}`, or `[]` as a placeholder. If a field is optional and you have no value, leave it out of the JSON entirely.
2. **Match the container type exactly.**- Array fields take JSON arrays: `["a", "b"]`, never `"[\"a\",\"b\"]"` (string), never `{}` (object), never `"foo"` (bare string).- Single-element arrays still need brackets: `["foo"]`, not `"foo"`.- Object fields take JSON objects, not arrays or strings.
3. **Strings are raw strings.** Do not wrap values in extra quotes, code fences, or markdown.
4. **Numbers and booleans are unquoted.** `30`, not `"30"`. `true`, not `"true"`.
## Paths and identifiers
5. **File paths, URLs, IDs, and similar fields go to system functions, not chat output.** Never format them as markdown links, never wrap them in backticks, never add explanatory parentheses.
Correct: `"/Users/me/notes.md"`Wrong: `"[notes.md](notes.md)"`Wrong: `` "`/Users/me/notes.md`" ``Wrong: `"/Users/me/notes.md (the notes file)"`
6. **If a tool description says "path", treat it as input to a filesystem call.** No formatting, no decoration.
## Related parameters
7. **When a tool has paired parameters (e.g., offset + limit, start + end, from + to), provide both or neither.** Read the description — if two fields work together, half the pair often produces an error.
## Recovery
8. **If a tool returns a validation error, read the error message carefully and fix only what it complains about.** Do not rewrite the whole call. Do not retry the same arguments.
9. **If a tool returns a "Note:" with a defaulted value, that's informational, not an error.** Continue the task. If the default is wrong, retry with the correct explicit value.
## Tool selection
10. **Use the tool whose description matches your intent most specifically.** Don't reach for `shellCommand` if a dedicated tool exists. Don't reach for `execute_code` for things a single tool call can handle.