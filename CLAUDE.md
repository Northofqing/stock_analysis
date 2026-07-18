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

Every new capability, refactor, or module addition MUST satisfy ALL FOUR conditions before any assistant says "✅ done":

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

### 4. Production evidence (NEW — this is what was missing in v17.7 / 集合竞价)

The first three layers prove the code works in isolation. Layer 4 proves the **production code path actually carries real data**, not just that the binary compiles and a `--test` mock fires. This catches the failure modes where:
- A PushKind is wired but no producer routes real data through it (v17.7: 5 of 6 declared PushKinds had zero production pushes on 2026-07-17).
- A scanner exists in library code but `main.rs` never calls it (集合竞价: `run_auction_agent()` has been orphan code since v13.10.1).
- A test passes locally but production logs show the path is dead (PushRecord rejecting `push.delivery.audit` made `--history --success-rate` always empty).

Verification (paste results in summary):

```bash
# 4a. Today's production push log has ≥ 1 file with this PushKind
DATE=$(date +%Y-%m-%d)
grep -lE '^\[<PushKind>\]' data/push_log/${DATE}/ 2>&1 | head -3 | wc -l   # must be ≥ 1
# If 0: the new path is wired but no real producer feeds it → BLOCKED, do not declare done.

# 4b. JSONL audit trail records the actual delivery outcome
grep -c '"event_type":"<event_type>' data/event_bus/${DATE}.jsonl 2>&1
# If 0: either the audit pipeline is dead OR the producer is dead. Identify which before claiming done.

# 4c. Cross-check the producer→adapter→push→audit chain end-to-end
# Trace from the upstream producer (Eastmoney / SearchService / monitor event bus) →
# through classify_* / classify_announcement / handle_monitor_event →
# through v17_sources::push_normalized_event →
# to push_governor_v3 → publish_delivery → JSONL audit.
# Each link MUST have ≥ 1 occurrence in production logs of the last 7 days.

# 4d. Spec-only PushKinds (designed but no real producer) MUST print a
#     "disabled=no_producer" banner at startup — silence is a defect.
```

### Anti-patterns (auto-reject)
- "✅ W{N} done" when `src/bin/monitor/` has 0 references to the new module.
- "✅ integration complete" based only on `v14_e2e.rs` (an isolated test binary, not the production monitor).
- Reporting `cargo test --lib` as proof of completion without checking integration.
- "The release binary pushed successfully" without confirming the new module was on the path of that push.
- **"✅ done" without grep `data/push_log/<today>/` showing ≥ 1 real push of this kind** (NEW).
- **"✅ 收官 / 100% / complete" claims in progress.md without production log evidence pasted** (NEW).

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

# 4. Production evidence (NEW)
DATE=$(date +%Y-%m-%d)
grep -lE '^\[<PushKind>\]' data/push_log/${DATE}/ 2>&1 | head -3 | wc -l
grep -c '"event_type":"<event_type>' data/event_bus/${DATE}.jsonl 2>&1
```

If any of the four checks fails, the work is **not complete**, regardless of how many unit tests passed.

## Spec Evidence Rule (Hard Constraint) — Prevents "Spec Divorced From Code" Hallucination

**This rule exists because of the v17.x post-mortem (2026-07-16)**: five specs (v17.4-v17.8) were written in bulk on one day, each citing the previous spec's unverified claims instead of code. Single-line grep produced false "0-caller" evidence for 7+ PushKind variants that were live production paths (multiline `dispatch(\n  PushKind::X` calls are invisible to single-line grep); v17.8 declared "收官 100% 覆盖" while the promised enum deletions were 0% executed; spec code sketches called APIs that did not exist (`push_application_service`, `history_query::winrate`). Executing those specs as written would have severed live trading pushes.

Every spec/design doc MUST satisfy ALL of the following before it is citable or implementable:

### 1. Code-fact claims need reproducible evidence
Any claim about the codebase (0-caller, line numbers, variant counts, "API X exists", "module Y is unused") MUST be accompanied by the exact command used and its output, pasted into the spec. Single-line grep is **not acceptable evidence for call-site claims** — trace call chains from `dispatch`/`push_governor` arguments backwards to `main.rs` (multiline-aware).

**Multiline grep is mandatory** for any claim about whether a function/variant/PushKind is "called" or "live". Use:

```bash
# WRONG: single-line grep misses multi-line call sites
grep -RIn 'push_governor_v3' src/

# RIGHT: multiline-aware — matches calls where argument is on a later line
grep -RInA3 'push_governor_v3(' src/
# or:
pcre2grep -RInM 'push_governor_v3\s*\(\s*&?\w+\s*,\s*\w+\s*::\w+' src/
```

This is what caught the v17.7 PushRecord event_type mismatch and the v17.8 "0-caller" hallucination: single-line grep reported zero callers for variants that were live production paths invoked via `dispatch(\n  PushKind::X\n)` (3-line call). For PushKind variant claims, also grep `stable_template_id` and `label` matches — production callers typically construct via the template ID, not the enum literal.

### 2. No spec-on-spec chaining past an unverified gate
A new batch spec MUST NOT be written while the previous batch's Acceptance Criteria are unverified (Gate C not passed = no new Gate A). Derived numbers (e.g. "variant count = 36 after deletion") MUST be recomputed against HEAD, not against another spec's promises.

### 3. AC must be machine-checkable and physically possible
Every AC needs a concrete command + expected output. Reject ACs that are impossible as stated (e.g. "env var restores deleted enum variants" — compile-time constructs cannot be restored at runtime; rollback for code deletion is `git revert`).

### 4. Completion claims follow the Completion Rule
"收官 / 100% / complete" statements in docs are held to the same three-layer standard as code: paste the verification commands and outputs. A spec claiming completion without evidence is treated as **In Progress**.

### 5. No speculative infrastructure without a consumer
Do not design infrastructure layers (buses, registries, replay, ACK protocols) in implementation-level detail until at least one real consumer is being built against them. Prefer the smallest change that solves the user's stated problem (v17.4-D solved the actual pain in ~200 lines; the planned 6-layer event stack was ~6400 lines with zero consumers).

### Anti-patterns (auto-reject)
- "0-caller, 可直接删除" backed only by single-line grep.
- AC numbers computed from another spec instead of from HEAD.
- Writing spec N+1 while spec N's ACs are unverified.
- Code sketches in specs calling functions that `grep` cannot find in the repo, without an explicit "TO BE BUILT" marker.

## Configuration

- `.env`: `STOCK_LIST` (watchlist codes), `WECHAT_SEND_SCRIPT`, `DATABASE_PATH`
- `config/*.toml`: chain rules, exclusion boards, announcement keywords, monitor timers — SIGHUP hot-reloadable
- All config files have code-level `const` fallbacks if toml is missing

## v15.x Lessons Learned (post-mortem)

**核心规则**: 默认值必须是"出声"的状态（推全量、记详细日志、显示警告）。任何"静默"默认值必须用 env var 显式声明才能生效。

详细 post-mortem 见 `docs/v15.x/post-mortem-v15.1.1.md` (P0 推送静默事故)。

**5 条硬规则**:

1. **默认值原则** — 所有"行为开关"默认值 = 出声状态（推全量 / 详细日志 / 显示警告）。静默默认值需 env var 显式声明才能生效。
   - **Self-check on every commit** (NEW): grep the diff for `unwrap_or_default\(|let _ = .*\.await\|Err\(.+\) => \{\}|if .* \{$` against the changed files. Every silent path MUST have a comment explaining why silence is required, OR be converted to an env-var-gated default.
2. **⚠️ BREAKING 标注** — 修改默认值时 commit msg 必须标 `BREAKING` + 写明回滚方法。
3. **测试覆盖默认值** — 不允许默认值无测试断言。
4. **静默路径可见** — 跳过推送/告警/任务的路径，启动时打印一次 mode + 每次跳过时 warn。
5. **测试字符串不进生产** (NEW) — 单元测试里的字符串字面量（"first"/"second"/"test kept"/"mock"/"stub"/etc.）绝不能进生产 push 路径。约束：
   - 单元测试 push 必须用 `test_*` 前缀的 PushKind 或 mock 函数，禁止直接调用 `push_governor_v3("test kept ...", PushKind::X, ...)` 然后让生产路径拿这个字符串去推。
   - CI grep：`grep -RInE '"(first|second|mock|stub|test kept|placeholder|fake|sample)"' data/push_log/$(date +%Y-%m-%d)/ 2>&1 | head -5` — any hit is a P0 defect (test text leaked to production).
   - 触发场景：dry-run / `--test` 模式没被 env var 正确隔离，或单元测试在 production binary 入口被错误执行。

## SDD Protocol (Hard Constraint) — Prevents "Subagent-Without-Context" Hallucination

This rule exists because of the v17.x post-mortem (2026-07-16/17): fresh subagents receive narrow briefs and have no cross-task or cross-version history. Three concrete failures this prevents:
- v17.7 6 declared PushKinds → only 1 had a real producer. Subagents implementing Tasks 6/7/8 did not know about upstream dead links (集合竞价 v13.10.1 deletion never propagated, news_aggregator_init EmAnnouncementFeed removal not yet visible to downstream tasks).
- r2-A Task 1 renamed `push.delivery` → `push.delivery.audit`. v17.3 history query implementer did not know → `PushRecord::try_from` kept requiring old name → `--history --success-rate` returns empty set in production.
- 集合竞价 `run_auction_agent()` library function has zero `main.rs` callers since v13.10.1. No implementer was tasked with wiring it.

### 1. Brief template (mandatory sections)

Every implementer / reviewer / Gate-verifier brief MUST include these three sections before any other content:

```markdown
**Upstream debt** (from progress.md or git log):
  - v_X.Y deletion/refactor that left orphan code or dead callers
  - List the SPECIFIC caller path that was severed
  - Example: "v13.10.1 deleted `notify::push_governor(&text, PushKind::AuctionRepush)` at main.rs:9037.
            No replacement wired. Library function `run_auction_agent()` at src/opportunity/auction_agent.rs:1
            is orphan code."

**Rename impact** (if this task renames/refactors any identifier):
  - List ALL downstream tasks/callers/snapshots that reference the old name
  - Example: "r2-A Task 1 renamed `push.delivery` → `push.delivery.audit`.
            PushRecord::try_from (event/push_record.rs:78) still requires old name → will silently reject all real deliveries."

**Production evidence** (DONE criteria):
  - The exact `data/push_log/<DATE>/` file pattern showing ≥ 1 real push
  - The exact JSONL audit event_type expected
  - The exact startup banner string expected if producer is missing
```

A brief without these sections is auto-rejected by the dispatcher.

### 2. Verifier independence (mandatory)

Every task-reviewer brief MUST start with:

```markdown
**DO NOT trust the implementer's report as ground truth.** Independently:
  - Re-run any command the implementer claimed to have run (cargo test, grep, build).
  - grep `data/push_log/$(date +%Y-%m-%d)/` for evidence the new module is on a real push path.
  - grep `data/event_bus/$(date +%Y-%m-%d).jsonl` for the audit event_type.
  - If the implementer reported "wired but no real data yet", verify the disabled banner exists at startup.
A verifier that returns "Approved" without independent production-log evidence is auto-rejected.
```

### 3. Gate verification (5-step protocol)

Every Gate (B / C / D) MUST be verified by a fresh subagent (not the implementer, not the dispatcher) using:

```bash
# Step 1: Module layer
cargo test --lib <module>::
cargo build --lib                                       # exit 0

# Step 2: Integration grep (multiline-aware — see Spec Evidence Rule §1)
grep -RInA3 'use.*<module>|<module>::' src/bin/monitor --include='*.rs'

# Step 3: Release build + --test smoke
cargo build --release --bin monitor
V10_DRY_RUN_PUSH=1 ./target/release/monitor --test 2>&1 | grep '<module_keyword>'

# Step 4: Production evidence (NEW — see Completion Rule §4)
DATE=$(date +%Y-%m-%d)
grep -lE '^\[<PushKind>\]' data/push_log/${DATE}/ 2>&1 | head -3 | wc -l
grep -c '"event_type":"<event_type>' data/event_bus/${DATE}.jsonl 2>&1

# Step 5: Cross-version debt check (NEW)
# For any PushKind touched by this Gate, grep for `is_active_spec_target_*` and `is_legacy_v17_*`
# annotations in notify.rs. Active variants MUST have a real caller; legacy variants MUST
# have an explicit deletion plan or be flagged as orphan in progress.md.
```

A Gate verifier report that omits step 4 or step 5 is auto-rejected.

### 4. Anti-patterns (auto-reject)

- Dispatching an implementer with a brief that lacks `Upstream debt` / `Rename impact` / `Production evidence` sections.
- Dispatching a reviewer with a brief that lacks the `DO NOT trust implementer` paragraph.
- A Gate verifier reporting "GREEN" without step 4 (production evidence) and step 5 (cross-version debt) output pasted.
- An implementer declaring DONE while their own diff contains silent paths (per v15.x rule 1 self-check) without comments explaining each.

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