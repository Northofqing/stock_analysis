# Sina instrument-news downstream Gateway migration

## Goal

Replace only the active downstream local Sina instrument-news acquisition with
the evidence-preserving upstream `magic-sina-rs` 0.2.0 contract through a
separate typed Gateway, while leaving general news aggregation and unrelated
providers unchanged.

## Scope constraints

- Do not edit `src/data_gateway/review.rs` or database audit code.
- Reuse its `pub(super)` BR-159 helpers from a sibling Gateway module.
- Preserve `Available`, `VerifiedEmpty`, `Unsupported`, and `Unavailable`
  without fabricating or mutating provider evidence.
- Migrate only multiline-traced, behaviorally equivalent production callers.
- Delete only the replaced local Sina instrument-news code/config.
- Validate against `target/magic_market_unified_work`, then restore committed
  Cargo dependency paths.
- Do not commit or push.

## Phases

1. **Gate A caller and contract evidence** — completed
   - Trace every active local Sina instrument-news caller.
   - Audit legacy acquisition/config and upstream/Router contracts.
   - Register the new BR before implementing filtering, deduplication, sorting
     or limiting.
   - Write and self-review a focused design and implementation plan.
2. **Tracer 1: typed Gateway state/evidence** — completed
   - RED public-behavior test.
   - Minimal sibling Gateway module using BR-159 helpers.
3. **Tracer 2: exact production migration** — completed
   - RED source/caller guard.
   - Migrate only exact equivalent production callers.
4. **Legacy removal** — completed
   - Recompute callers.
   - Delete only replaced local stock-news acquisition/config/tests.
5. **Gate B/C/D evidence** — in progress
   - Focused tests and upstream worktree compile.
   - Restore committed Cargo paths.
   - Fmt, strict Clippy, relevant/full tests, compliance, release build,
     coverage and bounded live/prod evidence as available.

## Decisions

- The parent brief is the approved scope boundary, not evidence that a
  production caller exists.
- No integration code is allowed if multiline caller tracing finds no exact
  local Sina instrument-news consumer.
- Existing shared worktree changes belong to other tasks and must be preserved.

## Errors encountered

| Error | Attempt | Resolution |
| --- | --- | --- |
| Initial TDD skill lookup used the user skill root | 1 | Read the repository-local `.agents/skills/tdd/SKILL.md` path from the available-skill catalog. |
