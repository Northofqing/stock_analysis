### Task 2: Migrate the legacy `GeminiAnalyzer` to DeepSeek fields

**Files:**
- Modify: `src/analyzer/types.rs:148-231`
- Modify: `src/analyzer/mod.rs:1-15,73-87,245-390`
- Modify: `src/analyzer/client.rs:1-205,352-459`
- Modify: `src/agent/multi_agent/mod.rs:33-59`
- Test: inline analyzer tests in `src/analyzer/mod.rs` or `src/analyzer/client.rs`

**Interfaces:**
- Consumes: canonical `DEEPSEEK_*`, `DEEPSEEK_QUICK_MODEL`, `DEEPSEEK_DEEP_MODEL`.
- Produces: `GeminiAnalyzer::use_deepseek`, `deepseek_model_for(AgentMode)`, and a compatible DeepSeek request path returning `Result<String>`.

- [ ] **Step 1: Add a failing no-network route-selection test**

Add a unit test using `GeminiConfig::default()` and synthetic values:

```rust
#[test]
fn deepseek_config_selects_deepseek_without_openai_fields() {
    let mut config = GeminiConfig::default();
    config.deepseek_api_key = Some("test-key".into());
    config.deepseek_base_url = Some("https://example.invalid/v1".into());
    config.deepseek_model = "deepseek-chat".into();
    config.deepseek_quick_model = Some("deepseek-chat".into());
    config.deepseek_deep_model = Some("deepseek-reasoner".into());

    let analyzer = GeminiAnalyzer::new(config);

    assert!(analyzer.use_deepseek);
    assert_eq!(analyzer.deepseek_model_for(AgentMode::Quick), "deepseek-chat");
    assert_eq!(analyzer.deepseek_model_for(AgentMode::Deep), "deepseek-reasoner");
}
```

Expected before implementation: compilation failure because the DeepSeek fields, flag, and selector do not exist.

- [ ] **Step 2: Rename the configuration fields and defaults**

In `GeminiConfig`, replace:

```rust
pub openai_api_key: Option<String>,
pub openai_base_url: Option<String>,
pub openai_model: String,
pub openai_quick_model: Option<String>,
pub openai_deep_model: Option<String>,
```

with the equivalent `deepseek_*` fields. Set the default model to `deepseek-chat`, while keeping the key/base defaults optional as before.

- [ ] **Step 3: Replace environment reads and active provider flags**

In `GeminiAnalyzer::from_env()`, replace the `OPENAI_*` reads with:

```rust
let deepseek_api_key = std::env::var("DEEPSEEK_API_KEY").ok();
let deepseek_base_url = std::env::var("DEEPSEEK_BASE_URL").ok();
let deepseek_model =
    std::env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| "deepseek-chat".to_string());
let deepseek_quick_model = std::env::var("DEEPSEEK_QUICK_MODEL").ok();
let deepseek_deep_model = std::env::var("DEEPSEEK_DEEP_MODEL").ok();
```

In `GeminiAnalyzer::new()`:

```rust
let use_doubao = config.doubao_api_key.is_some();
let use_deepseek = !use_doubao && config.deepseek_api_key.is_some();
```

Update `is_available()` to include `deepseek_api_key` instead of `openai_api_key`.

- [ ] **Step 4: Rename the compatible request path to DeepSeek**

In `src/analyzer/client.rs`:

- Rename `openai_model_for` to `deepseek_model_for` and read the new fields.
- Rename `call_openai_api` to `call_deepseek_api`.
- Read `self.config.deepseek_api_key`, `deepseek_base_url`, and use the default `https://api.deepseek.com/v1`.
- Preserve the existing request body and response parsing, including `reasoning_content` fallback, because DeepSeek is still using the same compatible wire contract.
- Change user-facing errors/log labels from `OpenAI` to `DeepSeek`.
- In `call_api_internal`, select DeepSeek after Doubao and before Gemini.
- Remove the post-Gemini OpenAI fallback block instead of creating a new fallback policy.

- [ ] **Step 5: Update multi-agent route reporting**

Replace the `else if self.use_openai` branch in `src/agent/multi_agent/mod.rs` with `self.use_deepseek`, use `deepseek_model_for(AgentMode::Quick/Deep)`, and report `"DeepSeek"`.

- [ ] **Step 6: Run the analyzer route test and compile the library**

Run:

```sh
cargo test --lib analyzer:: -- --nocapture
cargo build --lib
```

Expected: analyzer tests pass; library build exits 0; no `OPENAI_*` compile errors remain.

- [ ] **Step 7: Verify no direct OpenAI analyzer configuration remains**

Run:

```sh
git grep -n -E 'openai_api_key|openai_base_url|openai_model|OPENAI_API_KEY|OPENAI_BASE_URL|OPENAI_MODEL|OPENAI_QUICK_MODEL|OPENAI_DEEP_MODEL|call_openai_api|use_openai|OpenAI兼容' -- src/analyzer src/agent/multi_agent || true
```

Expected: no output. Protocol-level `async_openai` identifiers outside these direct service/config paths remain allowed.

---

