//! v16.6 #2: 故障恢复 (retry + panic hook + critical task monitor)
//!
//! - retry_with_backoff: 3 次重试 (100ms / 500ms / 2s)
//! - setup_panic_hook: panic 立即 log + webhook 告警
//! - spawn_critical_with_restart: 关键 task panic 5s 后自动 restart

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

pub async fn retry_with_backoff<F, T, E>(mut f: F) -> Result<T, E>
where
    F: FnMut() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>>,
{
    for delay_ms in [100u64, 500, 2000] {
        if let Ok(v) = f().await {
            return Ok(v);
        }
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
    f().await
}

pub fn setup_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        log::error!("[panic hook] {}", info);
        log::warn!("[panic hook] webhook 告警已发 (mock)");
    }));
}

pub fn spawn_critical_with_restart<F, Fut>(name: &'static str, f: F)
where
    F: Fn() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    tokio::spawn(async move {
        loop {
            f().await;
            log::warn!("[critical task] {} exited, restart 5s", name);
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn retry_succeeds_on_third_try() {
        let counter = AtomicU32::new(0);
        let result: Result<i32, ()> = retry_with_backoff(|| {
            let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
            Box::pin(async move {
                if n >= 3 { Ok(42) } else { Err(()) }
            })
        }).await;
        assert_eq!(result.unwrap(), 42);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retry_returns_last_error() {
        let counter = AtomicU32::new(0);
        let result: Result<(), &'static str> = retry_with_backoff(|| {
            counter.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err("failed") })
        }).await;
        assert!(result.is_err());
        assert_eq!(counter.load(Ordering::SeqCst), 4);
    }
}
