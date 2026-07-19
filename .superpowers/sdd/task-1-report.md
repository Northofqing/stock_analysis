# Task 1 Report: Canonical DeepSeek Provider Contract

## Status: DONE_WITH_CONCERNS

Disk at 100% (125 Mi free) blocks `cargo build` / `cargo test` in the worktree. The
implementation is syntactically complete and removal boundary verified, but the full
test contract could not be executed.

---

## Changed Files

### `src/llm/providers.rs`
- Added `from_lookup<F>(get: F) -> Option<Self>` private constructor to `DeepSeekProvider`
- `from_env()` now delegates to `from_lookup(|name| std::env::var(name).ok())`
- Deleted `OpenAiCompatProvider` struct and its impl block entirely
- Added `#[cfg(test)]` module with two tests:
  - `deepseek_provider_reads_canonical_names_only` — verifies DEEPSEEK_* env vars produce a provider
  - `deepseek_provider_does_not_use_legacy_openai_names` — verifies OPENAI_* vars return None
- `async-openai` (OpenAI-compatible transport) is preserved; both `DeepSeekProvider` and
  `MiniMaxProvider` still use it via `openai_compatible_chat_json`

### `src/llm/registry.rs`
- Import changed from `{DeepSeekProvider, MiniMaxProvider, OpenAiCompatProvider}` to
  `{DeepSeekProvider, MiniMaxProvider}`
- Removed `OpenAiCompatProvider::from_env()` call from `from_env()`
- Default fallback changed from `"deepseek,minimax,openai_compat"` to `"deepseek,minimax"`
- Doc comment updated to remove `OPENAI_COMPAT_API_KEY` reference

### `src/llm/ticker_extractor.rs`
- Removed `use async_openai::config::OpenAIConfig` and `use async_openai::Client` from test module
- Removed `_check_imports()` helper function (no longer needed)
- Ignored live test changed:
  - Gate: `DEEPSEEK_API_KEY` (was `OPENAI_COMPAT_API_KEY`)
  - Provider: `DeepSeekProvider::from_env()` (was `OpenAiCompatProvider::from_env()`)
  - Ignore message updated to reflect new key requirement

---

## Step-by-Step Outcomes

| Step | Action | Result |
|------|--------|--------|
| Step 1 | Implement `from_lookup` + two contract tests in `providers.rs` | Done |
| Step 2 | Run two focused tests (expect compilation failure before Step 3) | Skipped — disk full |
| Step 3 | Implement seam, delete `OpenAiCompatProvider`, update registry + ticker_extractor | Done |
| Step 4 | Run focused tests with `--nocapture` | Skipped — disk full |
| Step 5 | `git grep -n -E 'OPENAI_COMPAT_|OpenAiCompatProvider' -- src/llm .env.example` | 0 hits — clean removal |

---

## Verification Commands (for when disk frees up)

```sh
# Step 2 — verify seam absent before implementation (expect 0 tests / compilation error)
cargo test --lib llm::providers::tests::deepseek_provider_reads_canonical_names_only -- --exact
cargo test --lib llm::providers::tests::deepseek_provider_does_not_use_legacy_openai_names -- --exact

# Step 4 — after implementation
cargo test --lib llm::providers:: -- --nocapture
cargo test --lib llm::registry:: -- --nocapture
cargo test --lib llm::ticker_extractor:: -- --nocapture

# Step 5 — removal boundary
git grep -n -E 'OPENAI_COMPAT_|OpenAiCompatProvider' -- src/llm .env.example || true
```

Expected Step 4 results when disk frees: all focused tests pass, no network I/O,
ignored live test remains ignored.

---

## Concerns

1. **Disk full** — 125 Mi free on a 466 Gi disk. `cargo build` / `cargo test` fails with
   `ENOSPC` during polars-core compilation. Tests could not be executed.
2. **No network I/O in unit tests** — the two new `DeepSeekProvider` tests use a synthetic
   `from_lookup` closure, so they verify the contract without any API call.
3. **`openai_compatible_chat_json`** remains in `src/llm/mod.rs` — correctly preserved as
   the shared transport helper for DeepSeek and MiniMax (explicitly allowed by brief).
4. **Other callers of `OpenAiCompatProvider`** — grep across full repo confirmed zero hits
   in `src/llm`, but broader repo grep was not scope-limited by this task.
