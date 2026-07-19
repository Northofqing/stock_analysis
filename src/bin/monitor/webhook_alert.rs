//! Optional operations webhook with explicit disabled and failure outcomes.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookDelivery {
    Disabled,
    Delivered,
}

fn configured_webhook_url() -> Option<String> {
    normalize_webhook_url(std::env::var("ALERT_WEBHOOK_URL").ok())
}

fn normalize_webhook_url(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub async fn send_webhook_alert(event: &str, message: &str) -> Result<WebhookDelivery, String> {
    let Some(url) = configured_webhook_url() else {
        log::warn!("[webhook] ALERT_WEBHOOK_URL 未配置, 告警渠道已禁用: {event}");
        return Ok(WebhookDelivery::Disabled);
    };
    let body = serde_json::json!({
        "event": event,
        "message": message,
        "ts": chrono::Utc::now().to_rfc3339(),
        "source": "stock_analysis_monitor",
    });
    let response = stock_analysis::http_client::SHARED_FAST_HTTP_CLIENT
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("webhook POST {url}: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("webhook POST {url}: HTTP {}", response.status()));
    }
    Ok(WebhookDelivery::Delivered)
}

pub async fn on_health_fail(
    status: &crate::health::HealthStatus,
) -> Result<WebhookDelivery, String> {
    let failed = [
        ("db_writable", status.db_writable),
        ("bus_alive", status.bus_alive),
        ("strategy_registered", status.strategy_registered),
        ("perf_recent", status.perf_recent),
        ("quote_provider", status.quote_provider),
    ]
    .into_iter()
    .filter_map(|(name, ok)| (!ok).then_some(name))
    .collect::<Vec<_>>();
    send_webhook_alert("health_check_fail", &format!("健康检查 fail:{failed:?}")).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_webhook_setting_is_disabled() {
        assert_eq!(normalize_webhook_url(None), None);
        assert_eq!(normalize_webhook_url(Some("   ".to_string())), None);
        assert_eq!(
            normalize_webhook_url(Some(" https://example.invalid/hook ".to_string())),
            Some("https://example.invalid/hook".to_string())
        );
    }
}
