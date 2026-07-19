### Task 3: Migrate legacy ReAct, test, and startup validation paths

**Files:**
- Modify: `src/deep_analyzer.rs:201-267`
- Modify: `src/bin/agent_test.rs:20-61`
- Modify: `src/app/bootstrap.rs:18-36`
- Test: existing compile/test coverage plus a pure model-order helper test if needed

**Interfaces:**
- Consumes: `DOUBAO_*`, `DEEPSEEK_*`, and `GEMINI_*`.
- Produces: model configuration order Doubao → DeepSeek → Gemini and startup validation that recognizes `DEEPSEEK_API_KEY`.

- [ ] **Step 1: Add a failing pure model-order test**

Extract `collect_model_configs` behind this lookup function so it can be tested without mutating global environment:

```rust
fn collect_model_configs_from<F>(get: F) -> Vec<ModelConfig>
where
    F: Fn(&str) -> Option<String>,
{
    let mut configs = Vec::new();

    if let Some(key) = get("DOUBAO_API_KEY").filter(|value| !value.is_empty()) {
        configs.push(ModelConfig {
            api_key: key,
            api_base: get("DOUBAO_BASE_URL")
                .unwrap_or_else(|| "https://ark.cn-beijing.volces.com/api/v3".to_string()),
            model: get("DOUBAO_MODEL")
                .unwrap_or_else(|| "doubao-seed-2-0-pro-260215".to_string()),
        });
    }
    if let Some(key) = get("DEEPSEEK_API_KEY").filter(|value| !value.is_empty()) {
        configs.push(ModelConfig {
            api_key: key,
            api_base: get("DEEPSEEK_BASE_URL")
                .unwrap_or_else(|| "https://api.deepseek.com/v1".to_string()),
            model: get("DEEPSEEK_MODEL")
                .unwrap_or_else(|| "deepseek-chat".to_string()),
        });
    }
    if let Some(key) = get("GEMINI_API_KEY").filter(|value| !value.is_empty()) {
        configs.push(ModelConfig {
            api_key: key,
            api_base: get("GEMINI_BASE_URL").unwrap_or_else(|| {
                "https://generativelanguage.googleapis.com/v1beta/openai/".to_string()
            }),
            model: get("GEMINI_MODEL")
                .unwrap_or_else(|| "gemini-2.5-flash".to_string()),
        });
    }

    configs
}

fn collect_model_configs() -> Vec<ModelConfig> {
    collect_model_configs_from(|name| env::var(name).ok())
}
```

Add a unit test with synthetic values that asserts:

```rust
let values = std::collections::HashMap::from([
    ("DOUBAO_API_KEY", "doubao-key"),
    ("DOUBAO_MODEL", "doubao-test"),
    ("DEEPSEEK_API_KEY", "deepseek-key"),
    ("DEEPSEEK_BASE_URL", "https://api.deepseek.example/v1"),
    ("DEEPSEEK_MODEL", "deepseek-test"),
    ("GEMINI_API_KEY", "gemini-key"),
    ("GEMINI_MODEL", "gemini-test"),
]);
let configs = collect_model_configs_from(|name| {
    values.get(name).map(|value| (*value).to_string())
});
assert_eq!(configs[0].model, "doubao-test");
assert_eq!(configs[1].model, "deepseek-test");
assert_eq!(configs[2].model, "gemini-test");
assert_eq!(configs[1].api_base, "https://api.deepseek.example/v1");

let stale_only = std::collections::HashMap::from([
    ("OPENAI_API_KEY", "stale-key"),
    ("OPENAI_BASE_URL", "https://api.deepseek.com/v1"),
    ("OPENAI_MODEL", "deepseek-chat"),
]);
assert!(collect_model_configs_from(|name| {
    stale_only.get(name).map(|value| (*value).to_string())
}).is_empty());
```

Expected before implementation: compilation failure because the helper and DeepSeek branch do not exist.

- [ ] **Step 2: Migrate `src/deep_analyzer.rs`**

Replace the active `OPENAI_*` branch with:

```rust
if let Some(key) = env::var("DEEPSEEK_API_KEY").ok().filter(|k| !k.is_empty()) {
    configs.push(ModelConfig {
        api_key: key,
        api_base: env::var("DEEPSEEK_BASE_URL")
            .unwrap_or_else(|_| "https://api.deepseek.com/v1".to_string()),
        model: env::var("DEEPSEEK_MODEL")
            .unwrap_or_else(|_| "deepseek-chat".to_string()),
    });
}
```

Keep the existing model collection order and change the missing-config message to list `DEEPSEEK_API_KEY`.

- [ ] **Step 3: Migrate `src/bin/agent_test.rs`**

Replace its `OPENAI_*` branch with the same canonical DeepSeek branch and update the error message. Retain `async_openai::Client` because it is the protocol client.

- [ ] **Step 4: Update startup validation**

In `src/app/bootstrap.rs`, change:

```rust
let has_any_ai = ["GEMINI_API_KEY", "OPENAI_API_KEY", "DOUBAO_API_KEY"]
```

to:

```rust
let has_any_ai = ["GEMINI_API_KEY", "DEEPSEEK_API_KEY", "DOUBAO_API_KEY"]
```

Update the accompanying user-facing message to name `DEEPSEEK_API_KEY`.

- [ ] **Step 5: Run focused tests/builds**

Run:

```sh
cargo test --lib deep_analyzer -- --nocapture
cargo build --bin agent_test
cargo build --lib
```

Expected: all selected tests pass and both builds exit 0.

- [ ] **Step 6: Verify direct config removal across legacy paths**

Run:

```sh
git grep -n -E 'OPENAI_API_KEY|OPENAI_BASE_URL|OPENAI_MODEL|OPENAI_QUICK_MODEL|OPENAI_DEEP_MODEL' -- src/deep_analyzer.rs src/bin/agent_test.rs src/app/bootstrap.rs || true
```

Expected: no output.

---

