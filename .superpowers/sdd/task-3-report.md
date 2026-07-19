# Task 3 Report: Migrate legacy ReAct, test, and startup validation paths

## Status: DONE

## Changed Files

1. `src/deep_analyzer.rs`
2. `src/bin/agent_test.rs`
3. `src/app/bootstrap.rs`

## Implementation Details

### Step 1 - `collect_model_configs_from` helper + unit tests (`deep_analyzer.rs:208-293`)

Extracted `collect_model_configs()` into a pure `collect_model_configs_from<F>` that takes a getter closure, enabling testability without env mutation. Added `#[cfg(test)]` module with two tests:

- `test_collect_model_configs_order` - verifies Doubao -> DeepSeek -> Gemini ordering and custom `api_base` override for DeepSeek
- `test_collect_model_configs_stale_openai_ignored` - verifies that `OPENAI_API_KEY` (stale key name) produces an empty config vector

### Step 2 - `deep_analyzer.rs` OPENAI_* -> DEEPSEEK_*

Replaced `OPENAI_API_KEY` / `OPENAI_BASE_URL` / `OPENAI_MODEL` branch with `DEEPSEEK_API_KEY` / `DEEPSEEK_BASE_URL` / `DEEPSEEK_MODEL`. Updated bail message to name `DEEPSEEK_API_KEY`. Model order: Doubao -> DeepSeek -> Gemini.

### Step 3 - `agent_test.rs` OPENAI_* -> DEEPSEEK_*

Replaced `OPENAI_API_KEY` branch with `DEEPSEEK_API_KEY`. Updated panic message to name `DEEPSEEK_API_KEY`. Retained `async_openai::Client<OpenAIConfig>` as protocol client.

### Step 4 - `bootstrap.rs` startup validation

Changed `has_any_ai` from `["GEMINI_API_KEY", "OPENAI_API_KEY", "DOUBAO_API_KEY"]` to `["GEMINI_API_KEY", "DEEPSEEK_API_KEY", "DOUBAO_API_KEY"]`. Updated error message.

## Build/Test Outcomes

```
cargo test --lib deep_analyzer::tests -- --nocapture
  running 15 tests
  deep_analyzer::tests::test_collect_model_configs_stale_openai_ignored ... ok
  deep_analyzer::tests::test_collect_model_configs_order ... ok
  ... (13 pre-existing tests)
  test result: ok. 15 passed; 0 failed

cargo build --bin agent_test   # exit 0
cargo build --lib               # exit 0 (53 warnings, pre-existing)
```

## Step 6 Verification

```sh
grep -n -E 'OPENAI_API_KEY|OPENAI_BASE_URL|OPENAI_MODEL' \
  src/deep_analyzer.rs src/bin/agent_test.rs src/app/bootstrap.rs
```

Only remaining `OPENAI_*` reference: `deep_analyzer.rs:276` - intentional stale-only test data in `test_collect_model_configs_stale_openai_ignored`, confirming the test correctly ignores stale keys. Zero active-code `OPENAI_*` references remain.

## Concerns

- Worktree has its own `Cargo.toml`; cargo commands run from worktree cwd, not main repo.
- `OPENAI_API_KEY` remains only in test data (`stale_only` HashMap) — intended per brief.
- Pre-existing warnings in `cargo build --lib` are unrelated to this migration.
