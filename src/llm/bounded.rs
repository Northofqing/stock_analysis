//! Single local HTTP POST with a reviewed conservative reservation. Legacy SDK calls are unchanged.
use super::{ModelCallReceipt, ReceiptBearingJson};
use async_openai::config::{Config, OpenAIConfig};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

pub fn hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedPricing {
    pub schema_version: String,
    pub reviewed_by: String,
    pub contract_version: String,
    pub valid_until: DateTime<Utc>,
    pub provider: String,
    pub requested_model: String,
    pub endpoint: String,
    pub upstream_models: Vec<String>,
    pub currency: String,
    pub billing_scope: String,
    pub input_bound_method: String,
    /// Reviewed upper bound on billed framing and non-message prompt tokens.
    pub framing_tokens: u32,
    pub max_output_tokens: u16,
    pub input_micro_cny_per_million: u64,
    pub output_micro_cny_per_million: u64,
    pub fixed_max_micro_cny: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub wall_ms: u64,
    pub max_calls: u8,
    pub max_input_tokens: u32,
    pub max_output_tokens: u16,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub max_content_bytes: usize,
    pub ceiling_micro_cny: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            wall_ms: 20_000,
            max_calls: 2,
            max_input_tokens: 32_000,
            max_output_tokens: 1500,
            max_request_bytes: 64_000,
            max_response_bytes: 32_000,
            max_content_bytes: 16_000,
            ceiling_micro_cny: 0,
        }
    }
}
impl Limits {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(1..=60_000).contains(&self.wall_ms)
            || self.max_calls > 2
            || self.max_input_tokens == 0
            || self.max_input_tokens > 128_000
            || self.max_output_tokens == 0
            || self.max_output_tokens > 8192
            || self.max_request_bytes == 0
            || self.max_request_bytes > 256_000
            || self.max_response_bytes == 0
            || self.max_response_bytes > 256_000
            || self.max_content_bytes == 0
            || self.max_content_bytes > self.max_response_bytes
        {
            return Err("invalid_limits");
        }
        Ok(())
    }
}
pub struct BoundedJsonRequest<'a> {
    pub system: &'a str,
    pub user: &'a str,
    pub limits: &'a Limits,
}
/// Non-cloneable; private constructor binds spend approval, model, endpoint, and whole-run deadline.
pub struct SingleAttemptPermit {
    pricing: ReviewedPricing,
    reservation: Reservation,
    deadline: Instant,
}
#[derive(Debug, Clone, Serialize)]
pub struct Reservation {
    pub attempt: u8,
    pub pricing_sha256: String,
    pub maximum_micro_cny: u64,
    pub input_tokens: u32,
    pub output_tokens: u16,
    pub disposition: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct CacheUsage {
    pub prompt_cache_hit_tokens: u32,
    pub prompt_cache_miss_tokens: u32,
    pub cached_tokens: u32,
    pub reasoning_tokens: Option<u32>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache: Option<CacheUsage>,
}
#[derive(Debug, Serialize)]
pub struct BoundedResponse {
    pub response: ReceiptBearingJson,
    pub usage: Usage,
    pub reservation: Reservation,
}
#[derive(Debug, Serialize)]
pub struct BoundedFailure {
    pub code: &'static str,
    pub started: bool,
    pub receipt: Option<ModelCallReceipt>,
    /// Exact received content is diagnostic only; never retains content above the cap.
    pub raw_content: Option<String>,
    pub raw_content_state: &'static str,
    /// Available only after all supported usage/billing/bound checks pass.
    pub usage: Option<Usage>,
}
impl BoundedFailure {
    pub fn new(code: &'static str, started: bool) -> Self {
        Self {
            code,
            started,
            receipt: None,
            raw_content: None,
            raw_content_state: "unavailable",
            usage: None,
        }
    }
}
pub struct RunBudget {
    limits: Limits,
    deadline: Instant,
    calls: u8,
    reserved: u64,
}
impl RunBudget {
    pub fn new(limits: Limits, deadline: Instant) -> Result<Self, &'static str> {
        limits.validate()?;
        Ok(Self {
            limits,
            deadline,
            calls: 0,
            reserved: 0,
        })
    }
    pub fn summary(&self) -> Value {
        json!({"attempt_slots_issued":self.calls,"retained_maximum_micro_cny":self.reserved,"refunds_micro_cny":0})
    }
    pub fn reserve(
        &mut self,
        pricing: &ReviewedPricing,
        provider: &str,
        model: &str,
        endpoint: &str,
        system: &str,
        user: &str,
    ) -> Result<SingleAttemptPermit, &'static str> {
        let url = url::Url::parse(endpoint).map_err(|_| "pricing_mismatch")?;
        if pricing.schema_version != "assistant-reviewed-pricing-v1"
            || pricing.reviewed_by.trim().is_empty()
            || pricing.contract_version.trim().is_empty()
            || pricing.valid_until <= Utc::now()
            || pricing.provider != provider
            || pricing.requested_model != model
            || pricing.endpoint != endpoint
            || pricing.currency != "CNY"
            || pricing.billing_scope != "prompt_completion_only_no_hidden_tokens"
            || pricing.input_bound_method != "utf8_bytes_plus_reviewed_framing"
            || pricing.upstream_models.is_empty()
            || pricing.upstream_models.iter().any(|s| s.trim().is_empty())
            || pricing.max_output_tokens < self.limits.max_output_tokens
            || url.has_host() == false
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || !(url.scheme() == "https"
                || (cfg!(test) && url.scheme() == "http" && url.host_str() == Some("127.0.0.1")))
        {
            return Err("pricing_mismatch");
        }
        // This bound is usable only when reviewed for the declared provider tokenizer/billing contract.
        let input = system
            .len()
            .checked_add(user.len())
            .and_then(|n| u32::try_from(n).ok())
            .and_then(|n| n.checked_add(pricing.framing_tokens))
            .ok_or("input_limit")?;
        if input > self.limits.max_input_tokens {
            return Err("input_limit");
        }
        let component = |n: u64, price: u64| {
            n.checked_mul(price)
                .and_then(|v| v.checked_add(999_999))
                .map(|v| v / 1_000_000)
        };
        let maximum = component(input as u64, pricing.input_micro_cny_per_million)
            .and_then(|n| {
                component(
                    self.limits.max_output_tokens as u64,
                    pricing.output_micro_cny_per_million,
                )
                .and_then(|o| n.checked_add(o))
            })
            .and_then(|n| n.checked_add(pricing.fixed_max_micro_cny))
            .ok_or("cost_overflow")?;
        let reserved = self.reserved.checked_add(maximum).ok_or("cost_overflow")?;
        if self.calls >= self.limits.max_calls {
            return Err("call_limit");
        }
        if reserved > self.limits.ceiling_micro_cny {
            return Err("monetary_limit");
        }
        if Instant::now() >= self.deadline {
            return Err("deadline");
        }
        self.calls += 1;
        self.reserved = reserved;
        Ok(SingleAttemptPermit {
            pricing: pricing.clone(),
            deadline: self.deadline,
            reservation: Reservation {
                attempt: self.calls,
                pricing_sha256: hash(&serde_json::to_vec(pricing).map_err(|_| "pricing_mismatch")?),
                maximum_micro_cny: maximum,
                input_tokens: input,
                output_tokens: self.limits.max_output_tokens,
                disposition: "retained_maximum_no_refund_not_settled_bill".into(),
            },
        })
    }
}
impl SingleAttemptPermit {
    pub fn reservation(&self) -> &Reservation {
        &self.reservation
    }
}

pub(super) async fn call(
    config: &OpenAIConfig,
    provider: &str,
    model: &str,
    request: BoundedJsonRequest<'_>,
    permit: SingleAttemptPermit,
) -> Result<BoundedResponse, BoundedFailure> {
    let fail = |code, started| BoundedFailure::new(code, started);
    if !(provider == "deepseek" && matches!(model, "deepseek-flash" | "deepseek-v4-pro"))
        && !(cfg!(test) && provider == "fake" && model == "requested")
    {
        return Err(fail("bounded_model_unavailable", false));
    }
    request.limits.validate().map_err(|c| fail(c, false))?;
    let endpoint = config.url("/chat/completions");
    let input = request
        .system
        .len()
        .checked_add(request.user.len())
        .and_then(|n| u32::try_from(n).ok())
        .and_then(|n| n.checked_add(permit.pricing.framing_tokens));
    if provider != permit.pricing.provider
        || model != permit.pricing.requested_model
        || endpoint != permit.pricing.endpoint
        || input != Some(permit.reservation.input_tokens)
        || request.limits.max_output_tokens != permit.reservation.output_tokens
    {
        return Err(fail("permit_mismatch", false));
    }
    let mut wire = json!({"model": model, "messages": [
        {"role":"system","content":request.system},{"role":"user","content":request.user}],
        "max_tokens":request.limits.max_output_tokens,"n":1,"stream":false,"temperature":0.1,
        "response_format":{"type":"json_object"}});
    if provider == "deepseek" {
        wire["thinking"] = json!({"type":"disabled"});
    }
    let bytes = serde_json::to_vec(&wire).map_err(|_| fail("request_schema", false))?;
    if bytes.len() > request.limits.max_request_bytes {
        return Err(fail("request_bytes", false));
    }
    let remaining = permit.deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(fail("deadline", false));
    }
    // Fresh HTTP/1 client, no pooled connection, proxy, redirects, SDK, or retry loop.
    let client = reqwest_011::Client::builder()
        .http1_only()
        .no_proxy()
        .pool_max_idle_per_host(0)
        .redirect(reqwest_011::redirect::Policy::none())
        .connect_timeout(remaining.min(Duration::from_secs(3)))
        .timeout(remaining)
        .build()
        .map_err(|_| fail("transport_setup", false))?;
    let future = async {
        // Client construction and scheduling may consume the original preflight allowance.
        // Recheck at the dispatch boundary; an expired permit must not start a POST.
        if Instant::now() >= permit.deadline {
            return Err(fail("deadline", false));
        }
        if Utc::now() >= permit.pricing.valid_until {
            return Err(fail("pricing_expired", false));
        }
        let started_at = Utc::now();
        let mut response = client
            .post(endpoint)
            .headers(config.headers())
            .query(&config.query())
            .header("Content-Type", "application/json")
            .body(bytes)
            .send()
            .await
            .map_err(|_| fail("transport", true))?;
        if response
            .content_length()
            .is_some_and(|n| n > request.limits.max_response_bytes as u64)
        {
            return Err(fail("body_limit", true));
        }
        let status = response.status();
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| fail("transport", true))?
        {
            if body
                .len()
                .checked_add(chunk.len())
                .is_none_or(|n| n > request.limits.max_response_bytes)
            {
                return Err(fail("body_limit", true));
            }
            body.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            return Err(fail("http_rejected", true));
        }
        let raw: Value = serde_json::from_slice(&body).map_err(|_| fail("protocol", true))?;
        let nonempty = |v: &Value| {
            v.as_str()
                .filter(|s| !s.trim().is_empty())
                .map(str::to_owned)
        };
        let actual = nonempty(&raw["model"]);
        let id = nonempty(&raw["id"]);
        // Billing eligibility examines every supplied choice before failure selection.
        // Ambiguous choices must not hide unsupported thinking in an unselected message.
        let unsupported_thinking = raw["choices"].as_array().is_some_and(|choices| {
            choices.iter().any(|choice| {
                choice["message"]
                    .get("reasoning_content")
                    .is_some_and(|v| !v.is_null() && v.as_str() != Some(""))
            })
        });
        let choices = raw["choices"].as_array().filter(|v| v.len() == 1);
        let absent = Value::Null;
        let choice = choices.and_then(|c| c.first()).unwrap_or(&absent);
        let received_content = choice["message"]["content"].as_str();
        let content = received_content.unwrap_or_default().to_owned();
        let receipt = received_content
            .and_then(|_| actual.as_ref().zip(id))
            .map(|(actual, id)| ModelCallReceipt {
                provider: provider.into(),
                model: actual.clone(),
                requested_model: Some(model.into()),
                upstream_request_id: None,
                upstream_response_id: Some(id),
                system_sha256: hash(request.system.as_bytes()),
                user_sha256: hash(request.user.as_bytes()),
                response_sha256: hash(content.as_bytes()),
                started_at,
                completed_at: Utc::now(),
            });
        let with_receipt = |code| BoundedFailure {
            code,
            started: true,
            receipt: receipt.clone(),
            raw_content: (received_content.is_some()
                && content.len() <= request.limits.max_content_bytes)
                .then(|| content.clone()),
            raw_content_state: if received_content.is_none() {
                "unavailable"
            } else if content.len() <= request.limits.max_content_bytes {
                "retained_exact_untrusted"
            } else {
                "omitted_content_limit_full_hash_only"
            },
            usage: None,
        };
        // Known cache partitions are non-additional prompt tokens; no discount is assumed.

        let validated_usage = (|| -> Result<Usage, BoundedFailure> {
            if unsupported_thinking {
                return Err(with_receipt("unexpected_thinking"));
            }
            let usage = raw["usage"]
                .as_object()
                .ok_or_else(|| with_receipt("usage_missing"))?;
            let allowed: &[&str] = if provider == "deepseek" {
                &[
                    "prompt_tokens",
                    "completion_tokens",
                    "total_tokens",
                    "prompt_tokens_details",
                    "prompt_cache_hit_tokens",
                    "prompt_cache_miss_tokens",
                    "completion_tokens_details",
                ]
            } else {
                &["prompt_tokens", "completion_tokens", "total_tokens"]
            };
            if usage.keys().any(|k| !allowed.contains(&k.as_str())) {
                return Err(with_receipt("usage_scope"));
            }
            let token = |key: &str| {
                usage
                    .get(key)
                    .and_then(Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok())
                    .ok_or_else(|| with_receipt("usage_invalid"))
            };
            let cache = if provider == "deepseek" {
                let hit = token("prompt_cache_hit_tokens")?;
                let miss = token("prompt_cache_miss_tokens")?;
                let details = usage
                    .get("prompt_tokens_details")
                    .and_then(Value::as_object)
                    .ok_or_else(|| with_receipt("cache_usage_missing"))?;
                if details.keys().any(|k| k != "cached_tokens") {
                    return Err(with_receipt("usage_scope"));
                }
                let cached = details
                    .get("cached_tokens")
                    .and_then(Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok())
                    .ok_or_else(|| with_receipt("cache_usage_missing"))?;
                if hit.checked_add(miss) != Some(token("prompt_tokens")?) || cached != hit {
                    return Err(with_receipt("cache_usage_invalid"));
                }
                let reasoning = if let Some(v) = usage.get("completion_tokens_details") {
                    let obj = v.as_object().ok_or_else(|| with_receipt("usage_scope"))?;
                    if obj.keys().any(|k| k != "reasoning_tokens") {
                        return Err(with_receipt("usage_scope"));
                    }
                    let n = obj
                        .get("reasoning_tokens")
                        .and_then(Value::as_u64)
                        .and_then(|v| u32::try_from(v).ok())
                        .ok_or_else(|| with_receipt("usage_invalid"))?;
                    if n != 0 {
                        return Err(with_receipt("unexpected_thinking"));
                    }
                    Some(n)
                } else {
                    None
                };
                Some(CacheUsage {
                    prompt_cache_hit_tokens: hit,
                    prompt_cache_miss_tokens: miss,
                    cached_tokens: cached,
                    reasoning_tokens: reasoning,
                })
            } else {
                None
            };
            let usage = Usage {
                prompt_tokens: token("prompt_tokens")?,
                completion_tokens: token("completion_tokens")?,
                total_tokens: token("total_tokens")?,
                cache,
            };
            if usage.prompt_tokens.checked_add(usage.completion_tokens) != Some(usage.total_tokens)
                || usage.prompt_tokens > permit.reservation.input_tokens
                || usage.completion_tokens > permit.reservation.output_tokens as u32
            {
                return Err(with_receipt("usage_limit"));
            }
            Ok(usage)
        })();
        let completed_failure = |code| {
            let mut error = with_receipt(code);
            error.usage = validated_usage.as_ref().ok().cloned();
            error
        };
        if choices.is_none() {
            return Err(completed_failure("choices"));
        }
        if received_content.is_none() {
            return Err(completed_failure("empty_response"));
        }
        if content.trim().is_empty() {
            return Err(completed_failure("empty_response"));
        }
        if content.len() > request.limits.max_content_bytes {
            return Err(completed_failure("content_limit"));
        }
        if receipt.is_none() {
            return Err(completed_failure("receipt_missing"));
        }
        if !actual
            .as_ref()
            .is_some_and(|m| permit.pricing.upstream_models.contains(m))
            || choice["index"] != 0
            || choice["finish_reason"] != "stop"
            || choice["message"]
                .get("tool_calls")
                .is_some_and(|v| !v.is_null())
            || choice["message"]
                .get("function_call")
                .is_some_and(|v| !v.is_null())
        {
            return Err(completed_failure("protocol"));
        }
        if unsupported_thinking {
            return Err(completed_failure("unexpected_thinking"));
        }
        let parsed = serde_json::from_str(&content);
        let usage = validated_usage?;
        let value = parsed.map_err(|_| {
            let mut error = with_receipt("json_schema");
            error.usage = Some(usage.clone());
            error
        })?;
        Ok(BoundedResponse {
            response: ReceiptBearingJson {
                value,
                raw_content: content,
                receipt: receipt.expect("identity checked"),
            },
            usage,
            reservation: permit.reservation,
        })
    };
    tokio::time::timeout_at(tokio::time::Instant::from_std(permit.deadline), future)
        .await
        .map_err(|_| fail("deadline", true))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    async fn server(
        response: String,
        delay: Duration,
    ) -> (OpenAIConfig, tokio::task::JoinHandle<Value>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
                .await
                .expect("bounded helper accept deadline")
                .unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0; 4096];
            let body_start = loop {
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(i) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                    break i + 4;
                }
            };
            let headers = String::from_utf8_lossy(&bytes[..body_start]);
            assert!(headers.starts_with("POST /chat/completions HTTP/1.1"));
            let len = headers
                .lines()
                .find_map(|s| {
                    s.to_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|s| s.parse::<usize>().ok())
                })
                .unwrap();
            while bytes.len() < body_start + len {
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
            }
            let request: Value =
                serde_json::from_slice(&bytes[body_start..body_start + len]).unwrap();
            if response.contains('\0') {
                for segment in response.split('\0') {
                    if socket.write_all(segment.as_bytes()).await.is_err() {
                        break;
                    }
                    tokio::time::sleep(delay).await;
                }
            } else {
                tokio::time::sleep(delay).await;
                let _ = socket.write_all(response.as_bytes()).await;
            }
            drop(socket);
            assert!(
                tokio::time::timeout(Duration::from_millis(80), listener.accept())
                    .await
                    .is_err(),
                "unexpected retry or redirect"
            );
            request
        });
        (
            OpenAIConfig::new()
                .with_api_key("loopback-only")
                .with_api_base(format!("http://{addr}")),
            task,
        )
    }
    fn pricing(config: &OpenAIConfig) -> ReviewedPricing {
        ReviewedPricing {
            schema_version: "assistant-reviewed-pricing-v1".into(),
            reviewed_by: "test".into(),
            contract_version: "test-v1".into(),
            valid_until: Utc::now() + chrono::Duration::days(1),
            provider: "fake".into(),
            requested_model: "requested".into(),
            endpoint: config.url("/chat/completions"),
            upstream_models: vec!["actual".into()],
            currency: "CNY".into(),
            billing_scope: "prompt_completion_only_no_hidden_tokens".into(),
            input_bound_method: "utf8_bytes_plus_reviewed_framing".into(),
            framing_tokens: 100,
            max_output_tokens: 8192,
            input_micro_cny_per_million: 100,
            output_micro_cny_per_million: 100,
            fixed_max_micro_cny: 1,
        }
    }
    fn body() -> Value {
        json!({"id":"response-real","model":"actual","choices":[{"index":0,"finish_reason":"stop","message":{"role":"assistant","content":"{\"claims\":[]}"}}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}})
    }
    fn http(body: &Value) -> String {
        let b = body.to_string();
        format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",b.len(),b)
    }
    async fn attempt(
        response: String,
        delay: Duration,
        limits: Limits,
    ) -> Result<BoundedResponse, BoundedFailure> {
        let (config, task) = server(response, delay).await;
        let deadline = Instant::now() + Duration::from_millis(limits.wall_ms);
        let mut budget = RunBudget::new(limits.clone(), deadline).unwrap();
        let permit = budget
            .reserve(
                &pricing(&config),
                "fake",
                "requested",
                &config.url("/chat/completions"),
                "system",
                "user",
            )
            .unwrap();
        let result = call(
            &config,
            "fake",
            "requested",
            BoundedJsonRequest {
                system: "system",
                user: "user",
                limits: &limits,
            },
            permit,
        )
        .await;
        let request = task.await.unwrap();
        assert_eq!(request["max_tokens"], limits.max_output_tokens);
        assert_eq!(request["n"], 1);
        assert_eq!(request["stream"], false);
        assert!(request.get("tools").is_none());
        result
    }
    fn limits() -> Limits {
        Limits {
            ceiling_micro_cny: 100,
            wall_ms: 2000,
            ..Limits::default()
        }
    }
    #[tokio::test]
    async fn genuine_receipt_token_controls_and_single_http1_post() {
        let r = attempt(http(&body()), Duration::ZERO, limits())
            .await
            .unwrap();
        assert_eq!(r.response.receipt().model(), "actual");
        assert_eq!(r.response.receipt().requested_model(), Some("requested"));
        assert_eq!(r.usage.total_tokens, 15);
        assert_eq!(
            r.response.receipt().response_sha256(),
            hash(r.response.raw_content().as_bytes())
        );
    }
    #[tokio::test]
    async fn rate_limit_redirect_body_and_whole_deadline_have_one_request() {
        for (status, extra) in [
            ("429 Too Many Requests", ""),
            ("307 Temporary Redirect", "Location: /redirect\r\n"),
            ("308 Permanent Redirect", "Location: /redirect\r\n"),
        ] {
            let e = attempt(
                format!(
                    "HTTP/1.1 {status}\r\n{extra}Content-Length: 0\r\nConnection: close\r\n\r\n"
                ),
                Duration::ZERO,
                limits(),
            )
            .await
            .unwrap_err();
            assert_eq!(e.code, "http_rejected");
            assert!(e.started);
        }
        let e = attempt(
            "HTTP/1.1 200 OK\r\nContent-Length: 999999\r\nConnection: close\r\n\r\n".into(),
            Duration::ZERO,
            limits(),
        )
        .await
        .unwrap_err();
        assert_eq!(e.code, "body_limit");
        let l = Limits {
            max_response_bytes: 32,
            max_content_bytes: 32,
            ..limits()
        };
        let b = "x".repeat(40);
        let e=attempt(format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n28\r\n{b}\r\n0\r\n\r\n"),Duration::ZERO,l).await.unwrap_err();
        assert_eq!(e.code, "body_limit");
        let e = attempt(
            http(&body()),
            Duration::from_millis(900),
            Limits {
                wall_ms: 500,
                ..limits()
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(e.code, "deadline" | "transport"));
    }
    #[tokio::test]
    async fn trickle_body_and_decoded_content_are_capped() {
        let e=attempt("HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\n\0x\0x\0x\0x\0x\0x\0x\0x".into(),Duration::from_millis(150),Limits{wall_ms:500,..limits()}).await.unwrap_err();
        assert!(matches!(e.code, "deadline" | "transport"));
        let e = attempt(
            http(&body()),
            Duration::ZERO,
            Limits {
                max_content_bytes: 5,
                ..limits()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(e.code, "content_limit");
        assert!(e.raw_content.is_none());
        assert_eq!(e.raw_content_state, "omitted_content_limit_full_hash_only");
        assert_eq!(
            e.receipt.unwrap().response_sha256(),
            hash(
                body()["choices"][0]["message"]["content"]
                    .as_str()
                    .unwrap()
                    .as_bytes()
            )
        );
        assert!(e.usage.is_some());
    }
    #[tokio::test]
    async fn request_byte_limit_is_enforced_before_send() {
        let cfg = OpenAIConfig::new().with_api_base("http://127.0.0.1:9");
        let l = Limits {
            max_request_bytes: 1,
            ..limits()
        };
        let mut budget =
            RunBudget::new(l.clone(), Instant::now() + Duration::from_secs(1)).unwrap();
        let permit = budget
            .reserve(
                &pricing(&cfg),
                "fake",
                "requested",
                &cfg.url("/chat/completions"),
                "system",
                "user",
            )
            .unwrap();
        let e = call(
            &cfg,
            "fake",
            "requested",
            BoundedJsonRequest {
                system: "system",
                user: "user",
                limits: &l,
            },
            permit,
        )
        .await
        .unwrap_err();
        assert_eq!(e.code, "request_bytes");
        assert!(!e.started);
    }
    #[tokio::test]
    async fn missing_invalid_excess_usage_choices_finish_and_identity_fail_closed() {
        for kind in [
            "missing", "negative", "string", "overflow", "sum", "output", "prompt", "detail",
            "empty", "multiple", "length", "identity", "json",
        ] {
            let mut b = body();
            match kind {
                "missing" => b["usage"] = Value::Null,
                "negative" => b["usage"]["prompt_tokens"] = json!(-1),
                "string" => b["usage"]["prompt_tokens"] = json!("10"),
                "overflow" => b["usage"]["prompt_tokens"] = json!(u64::MAX),
                "sum" => b["usage"]["total_tokens"] = json!(100),
                "output" => {
                    b["usage"] =
                        json!({"prompt_tokens":10,"completion_tokens":9999,"total_tokens":10009})
                }
                "prompt" => {
                    b["usage"] =
                        json!({"prompt_tokens":9999,"completion_tokens":5,"total_tokens":10004})
                }
                "detail" => b["usage"]["hidden_tokens"] = json!(1),
                "empty" => b["choices"] = json!([]),
                "multiple" => {
                    b["choices"] = json!([b["choices"][0].clone(), b["choices"][0].clone()])
                }
                "length" => b["choices"][0]["finish_reason"] = json!("length"),
                "identity" => b["model"] = json!("unexpected"),
                "json" => b["choices"][0]["message"]["content"] = json!("invalid"),
                _ => unreachable!(),
            }
            let e = attempt(http(&b), Duration::ZERO, limits())
                .await
                .unwrap_err();
            assert!(e.started, "{kind}");
            if kind == "json" {
                assert_eq!(e.raw_content.as_deref(), Some("invalid"));
                assert_eq!(e.raw_content_state, "retained_exact_untrusted");
                assert_eq!(e.usage.as_ref().unwrap().total_tokens, 15);
            }
            if kind == "detail" {
                assert!(e.usage.is_none());
            }
            if kind == "empty" || kind == "multiple" {
                assert_eq!(e.raw_content_state, "unavailable");
                assert!(e.raw_content.is_none());
                assert_eq!(e.usage.as_ref().unwrap().total_tokens, 15);
            }
        }
    }
    #[tokio::test]
    async fn documented_deepseek_cache_non_thinking_and_unknown_classes() {
        for kind in [
            "valid",
            "hit_sum",
            "cached",
            "reasoning",
            "content_thinking",
            "extra",
            "missing",
            "empty_content",
            "thinking_empty",
            "thinking_over_cap",
            "thinking_protocol",
            "thinking_missing_receipt",
            "thinking_missing_content",
            "thinking_ambiguous",
            "ambiguous_no_thinking",
            "reasoning_tokens_empty",
        ] {
            let mut b = body();
            b["model"] = json!("deepseek-flash");
            b["usage"]["prompt_cache_hit_tokens"] = json!(3);
            b["usage"]["prompt_cache_miss_tokens"] = json!(7);
            b["usage"]["prompt_tokens_details"] = json!({"cached_tokens":3});
            b["usage"]["completion_tokens_details"] = json!({"reasoning_tokens":0});
            match kind {
                "hit_sum" => b["usage"]["prompt_cache_miss_tokens"] = json!(8),
                "cached" => b["usage"]["prompt_tokens_details"]["cached_tokens"] = json!(4),
                "reasoning" => {
                    b["usage"]["completion_tokens_details"]["reasoning_tokens"] = json!(1)
                }
                "content_thinking" => {
                    b["choices"][0]["message"]["reasoning_content"] = json!("unexpected")
                }
                "extra" => {
                    b["usage"]["completion_tokens_details"]["unknown_billable_tokens"] = json!(1)
                }
                "missing" => {
                    b["usage"]
                        .as_object_mut()
                        .unwrap()
                        .remove("prompt_cache_miss_tokens");
                }
                "empty_content" => b["choices"][0]["message"]["content"] = json!(""),
                "thinking_empty" => b["choices"][0]["message"]["content"] = json!(""),
                "thinking_over_cap" => {
                    b["choices"][0]["message"]["content"] =
                        json!("x".repeat(limits().max_content_bytes + 1))
                }
                "thinking_protocol" => b["choices"][0]["finish_reason"] = json!("length"),
                "thinking_missing_receipt" => b["id"] = Value::Null,
                "thinking_missing_content" => b["choices"][0]["message"]["content"] = Value::Null,
                "thinking_ambiguous" | "ambiguous_no_thinking" => {
                    let mut second = b["choices"][0].clone();
                    second["index"] = json!(1);
                    second["message"]["reasoning_content"] = if kind == "thinking_ambiguous" {
                        json!("hidden in second choice")
                    } else {
                        json!("")
                    };
                    b["choices"] = json!([b["choices"][0].clone(), second]);
                }
                "reasoning_tokens_empty" => {
                    b["choices"][0]["message"]["content"] = json!("");
                    b["usage"]["completion_tokens_details"]["reasoning_tokens"] = json!(1);
                }
                _ => (),
            }
            if kind.starts_with("thinking_") && kind != "thinking_ambiguous" {
                b["choices"][0]["message"]["reasoning_content"] = json!("unexpected");
            }
            let (cfg, task) = server(http(&b), Duration::ZERO).await;
            let l = limits();
            let mut price = pricing(&cfg);
            price.provider = "deepseek".into();
            price.requested_model = "deepseek-flash".into();
            price.upstream_models = vec!["deepseek-flash".into()];
            let mut budget =
                RunBudget::new(l.clone(), Instant::now() + Duration::from_millis(l.wall_ms))
                    .unwrap();
            let permit = budget
                .reserve(
                    &price,
                    "deepseek",
                    "deepseek-flash",
                    &price.endpoint,
                    "system",
                    "user",
                )
                .unwrap();
            let reserved = permit.reservation().maximum_micro_cny;
            let result = call(
                &cfg,
                "deepseek",
                "deepseek-flash",
                BoundedJsonRequest {
                    system: "system",
                    user: "user",
                    limits: &l,
                },
                permit,
            )
            .await;
            let wire = task.await.unwrap();
            assert_eq!(wire["thinking"]["type"], "disabled");
            assert_eq!(wire["max_tokens"], l.max_output_tokens);
            assert_eq!(budget.summary()["retained_maximum_micro_cny"], reserved);
            assert_eq!(budget.summary()["refunds_micro_cny"], 0);
            assert_eq!(budget.summary()["attempt_slots_issued"], 1);
            if kind == "valid" {
                let r = result.unwrap();
                assert_eq!(r.usage.cache.unwrap().cached_tokens, 3);
                assert_eq!(
                    r.response.receipt().response_sha256(),
                    hash(r.response.raw_content().as_bytes())
                );
            } else {
                let error = result.unwrap_err();
                assert!(error.started, "{kind}");
                if kind == "content_thinking"
                    || kind.starts_with("thinking_")
                    || kind == "reasoning_tokens_empty"
                {
                    assert!(error.usage.is_none(), "{kind}");
                }
                match kind {
                    "thinking_empty" | "reasoning_tokens_empty" => {
                        assert_eq!(error.code, "empty_response");
                        assert_eq!(error.raw_content.as_deref(), Some(""));
                        assert_eq!(error.receipt.as_ref().unwrap().response_sha256(), hash(b""));
                    }
                    "thinking_over_cap" => {
                        assert_eq!(error.code, "content_limit");
                        assert!(error.raw_content.is_none());
                        assert_eq!(
                            error.raw_content_state,
                            "omitted_content_limit_full_hash_only"
                        );
                        assert_eq!(
                            error.receipt.as_ref().unwrap().response_sha256(),
                            hash(
                                b["choices"][0]["message"]["content"]
                                    .as_str()
                                    .unwrap()
                                    .as_bytes()
                            )
                        );
                    }
                    "thinking_protocol" => assert_eq!(error.code, "protocol"),
                    "thinking_missing_receipt" => assert_eq!(error.code, "receipt_missing"),
                    "thinking_missing_content" => assert_eq!(error.code, "empty_response"),
                    "thinking_ambiguous" | "ambiguous_no_thinking" => {
                        assert_eq!(error.code, "choices");
                        assert!(error.raw_content.is_none());
                        assert!(error.receipt.is_none());
                        assert_eq!(error.raw_content_state, "unavailable");
                        if kind == "ambiguous_no_thinking" {
                            assert!(error.usage.is_some());
                        }
                    }
                    _ => (),
                }
                if kind == "empty_content" {
                    assert_eq!(error.code, "empty_response");
                    assert_eq!(error.receipt.unwrap().response_sha256(), hash(b""));
                    assert_eq!(error.raw_content.as_deref(), Some(""));
                    assert!(error.usage.is_some());
                }
            }
        }
    }
    #[tokio::test]
    async fn unsupported_requested_models_refuse_before_send() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let cfg =
            OpenAIConfig::new().with_api_base(format!("http://{}", listener.local_addr().unwrap()));
        let l = limits();
        for (provider, model) in [
            ("deepseek", "deepseek-chat"),
            ("deepseek", "deepseek-reasoner"),
            ("deepseek", "custom"),
            ("minimax", "MiniMax-M2.7"),
        ] {
            let mut budget =
                RunBudget::new(l.clone(), Instant::now() + Duration::from_secs(1)).unwrap();
            let price = pricing(&cfg);
            let permit = budget
                .reserve(
                    &price,
                    "fake",
                    "requested",
                    &price.endpoint,
                    "system",
                    "user",
                )
                .unwrap();
            let e = call(
                &cfg,
                provider,
                model,
                BoundedJsonRequest {
                    system: "system",
                    user: "user",
                    limits: &l,
                },
                permit,
            )
            .await
            .unwrap_err();
            assert_eq!(e.code, "bounded_model_unavailable");
            assert!(!e.started);
            assert!(
                tokio::time::timeout(Duration::from_millis(80), listener.accept())
                    .await
                    .is_err(),
                "unsupported model sent a request"
            );
        }
    }
    #[tokio::test]
    async fn permit_that_ages_after_reservation_does_not_dispatch() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let cfg =
            OpenAIConfig::new().with_api_base(format!("http://{}", listener.local_addr().unwrap()));
        let l = limits();
        let price = pricing(&cfg);
        let mut budget =
            RunBudget::new(l.clone(), Instant::now() + Duration::from_secs(5)).unwrap();
        let mut permit = budget
            .reserve(
                &price,
                "fake",
                "requested",
                &price.endpoint,
                "system",
                "user",
            )
            .unwrap();
        // Model a previously-issued permit whose reviewed pricing has now expired.
        permit.pricing.valid_until = Utc::now() - chrono::Duration::seconds(1);
        let e = call(
            &cfg,
            "fake",
            "requested",
            BoundedJsonRequest {
                system: "system",
                user: "user",
                limits: &l,
            },
            permit,
        )
        .await
        .unwrap_err();
        assert_eq!(e.code, "pricing_expired");
        assert!(!e.started);
        assert!(
            tokio::time::timeout(Duration::from_millis(80), listener.accept())
                .await
                .is_err()
        );
    }
    #[test]
    fn reservation_ceilings_are_checked_before_dispatch_and_never_refunded() {
        let cfg = OpenAIConfig::new().with_api_base("http://127.0.0.1:9");
        let p = pricing(&cfg);
        let l = limits();
        let mut budget =
            RunBudget::new(l.clone(), Instant::now() + Duration::from_secs(1)).unwrap();
        let a = budget
            .reserve(&p, "fake", "requested", &p.endpoint, "system", "user")
            .unwrap();
        assert!(a.reservation().maximum_micro_cny > 0);
        let _ = budget
            .reserve(&p, "fake", "requested", &p.endpoint, "system", "user")
            .unwrap();
        assert!(budget
            .reserve(&p, "fake", "requested", &p.endpoint, "system", "user")
            .is_err());
        let mut b = RunBudget::new(
            Limits {
                ceiling_micro_cny: 0,
                ..l
            },
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
        assert!(b
            .reserve(&p, "fake", "requested", &p.endpoint, "system", "user")
            .is_err());
    }
}
