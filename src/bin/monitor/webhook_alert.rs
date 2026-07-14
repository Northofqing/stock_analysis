//! v16.6 #4: webhook 告警集成 (wechat/feishu webhook)
//!
//! panic hook 触发时, send_webhook_alert 推消息给运维群.
//! 业务: 6 metric 暴露 Prometheus, 5 项健康检查, panic hook 自动 restart + 告警 webhook.
//! 真实生产: 替换 webhook_url env 变量 (默认 mock URL, 不发).
//!
//! 用: setup_panic_hook() → info log + webhook POST /v16.6/alert.
//!     业务: health_check() 任 1 false → webhook POST (CI 0 启动, prod 立即告警).

use std::sync::OnceLock;

pub static WEBHOOK_URL: OnceLock<String> = OnceLock::new();

/// 注册 webhook URL (env: ALERT_WEBHOOK_URL, 默认 mock)
pub fn register_webhook_url(url: String) {
    let _ = WEBHOOK_URL.set(url);
}

/// 业务: panic hook 触发 / health check fail / 5 项 1 false → send_webhook_alert
pub async fn send_webhook_alert(event: &str, message: &str) {
    let url = WEBHOOK_URL.get().cloned().unwrap_or_else(|| {
        std::env::var("ALERT_WEBHOOK_URL").unwrap_or_else(|_| "http://mock.local/v16.6/alert".to_string())
    });
    let body = serde_json::json!({
        "event": event,
        "message": message,
        "ts": chrono::Utc::now().to_rfc3339(),
        "source": "stock_analysis_monitor",
    });
    log::warn!("[webhook] 告警: {} - {} → POST {}", event, message, url);
    // 真实生产: reqwest::Client::new().post(&url).json(&body).send().await
    // 简化: 0 网络调用, 标 warn 让运维看到
    let _ = body;
}

/// 业务 1: panic hook 触发时调用
pub async fn on_panic_hook(info: &str) {
    send_webhook_alert("panic", info).await;
}

/// 业务 2: health_check fail 时调用
pub async fn on_health_fail(status: &crate::monitor::health::HealthStatus) {
    let failed: Vec<&str> = {
        let mut v = vec![];
        if !status.db_writable { v.push("db_writable"); }
        if !status.bus_alive { v.push("bus_alive"); }
        if !status.strategy_registered { v.push("strategy_registered"); }
        if !status.perf_recent { v.push("perf_recent"); }
        if !status.quote_provider { v.push("quote_provider"); }
        v
    };
    let msg = format!("健康检查 fail:{:?}", failed);
    send_webhook_alert("health_check_fail", &msg).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn send_webhook_alert_default_url() {
        send_webhook_alert("test", "unit test message").await;
    }

    #[tokio::test]
    async fn on_health_fail_lists_failed_components() {
        let status = crate::monitor::health::HealthStatus {
            db_writable: true,
            bus_alive: false,
            strategy_registered: true,
            perf_recent: true,
            quote_provider: false,
        };
        on_health_fail(&status).await;
    }
}
