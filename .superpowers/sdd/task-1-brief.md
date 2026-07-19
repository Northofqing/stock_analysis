### Task 1: Lock the canonical DeepSeek provider contract

**Files:**
- Modify: `src/llm/providers.rs:9-31`
- Modify: `src/llm/registry.rs:1-67`
- Modify: `src/llm/ticker_extractor.rs:110-220`
- Test: inline `#[cfg(test)]` modules in the files above

**Interfaces:**
- Consumes: `DEEPSEEK_API_KEY`, optional `DEEPSEEK_BASE_URL`, optional `DEEPSEEK_MODEL`.
- Produces: `DeepSeekProvider::from_env() -> Option<DeepSeekProvider>` and registry selection under provider name `deepseek`.

- [ ] **Step 1: Write a deterministic failing provider-contract test**

Implement this exact private helper and make `from_env()` call it with `|name| std::env::var(name).ok()`:

```rust
fn from_lookup<F>(get: F) -> Option<Self>
where
    F: Fn(&str) -> Option<String>,
{
    let key = get("DEEPSEEK_API_KEY").filter(|value| !value.is_empty())?;
    let base = get("DEEPSEEK_BASE_URL")
        .unwrap_or_else(|| "https://api.deepseek.com/v1".to_string());
    let model = get("DEEPSEEK_MODEL")
        .unwrap_or_else(|| "deepseek-chat".to_string());
    let cfg = OpenAIConfig::new().with_api_key(key).with_api_base(base);
    Some(Self {
        client: Client::with_config(cfg),
        model,
    })
}
```

Add tests with a synthetic lookup map:

```rust
#[test]
fn deepseek_provider_reads_canonical_names_only() {
    let provider = DeepSeekProvider::from_lookup(|name| match name {
        "DEEPSEEK_API_KEY" => Some("test-key".into()),
        "DEEPSEEK_BASE_URL" => Some("https://example.invalid/v1".into()),
        "DEEPSEEK_MODEL" => Some("deepseek-reasoner".into()),
        _ => None,
    })
    .expect("canonical DeepSeek key should create provider");

    assert_eq!(provider.name(), "deepseek");
    assert_eq!(provider.model(), "deepseek-reasoner");
}

#[test]
fn deepseek_provider_does_not_use_legacy_openai_names() {
    assert!(DeepSeekProvider::from_lookup(|name| match name {
        "OPENAI_API_KEY" => Some("stale-key".into()),
        "OPENAI_BASE_URL" => Some("https://api.deepseek.com/v1".into()),
        "OPENAI_MODEL" => Some("deepseek-chat".into()),
        _ => None,
    })
    .is_none());
}
```

- [ ] **Step 2: Run the focused tests and verify the seam is initially absent**

Run:

```sh
cargo test --lib llm::providers::tests::deepseek_provider_reads_canonical_names_only -- --exact
cargo test --lib llm::providers::tests::deepseek_provider_does_not_use_legacy_openai_names -- --exact
```

Expected before implementation: compilation/test failure because `from_lookup` and the new tests do not yet exist.

- [ ] **Step 3: Implement the canonical constructor seam**

Make `from_env()` delegate to `from_lookup(|name| std::env::var(name).ok())`. Keep the existing default base URL/model and `OpenAIConfig` construction. Do not add any `OPENAI_*` fallback.

Delete `OpenAiCompatProvider` from `src/llm/providers.rs`. Keep the shared `openai_compatible_chat_json` helper in `src/llm/mod.rs` because DeepSeek and MiniMax use it.

In `src/llm/registry.rs`:

```rust
use super::providers::{DeepSeekProvider, MiniMaxProvider};
```

Remove the `OpenAiCompatProvider` registration and change the default fallback expression to:

```rust
.unwrap_or_else(|_| "deepseek,minimax".to_string())
```

In `src/llm/ticker_extractor.rs`, change the ignored real-API test to construct `DeepSeekProvider` and gate on `DEEPSEEK_API_KEY`; remove the obsolete `OpenAiCompatProvider` import and the `_check_imports` helper if it is no longer needed after compilation.

- [ ] **Step 4: Run focused tests and verify provider selection**

Run:

```sh
cargo test --lib llm::providers:: -- --nocapture
cargo test --lib llm::registry:: -- --nocapture
cargo test --lib llm::ticker_extractor:: -- --nocapture
```

Expected: all focused tests pass; no test performs network I/O; the ignored live test remains ignored.

- [ ] **Step 5: Check the removal boundary**

Run:

```sh
git grep -n -E 'OPENAI_COMPAT_|OpenAiCompatProvider' -- src/llm .env.example || true
```

Expected: no output. Generic transport identifiers such as `openai_compatible_chat_json` may remain elsewhere and are explicitly allowed by the design.

---

