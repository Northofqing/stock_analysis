//! Keep immutable NewsFlash audit recovery off the async runtime workers.

use stock_analysis::event::{NewsFlashAuthoritySnapshot, NewsFlashReconcileError};

pub(super) async fn reconcile_news_flash_business_date(
    business_date: chrono::NaiveDate,
) -> Result<NewsFlashAuthoritySnapshot, NewsFlashReconcileError> {
    on_blocking_worker(move || {
        stock_analysis::event::reconcile_news_flash_business_date(business_date)
    })
    .await
}

async fn on_blocking_worker<T, F>(reconcile: F) -> Result<T, NewsFlashReconcileError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, NewsFlashReconcileError> + Send + 'static,
{
    tokio::task::spawn_blocking(reconcile).await.map_err(|_| {
        NewsFlashReconcileError::AuthorityUnavailable(
            "news_flash_authority_worker_join_failed".to_owned(),
        )
    })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[tokio::test(flavor = "current_thread")]
    async fn news_authority_recovery_keeps_single_runtime_worker_responsive() {
        let runtime_progress = Arc::new(AtomicBool::new(false));
        let worker_progress = runtime_progress.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let recovery = tokio::spawn(on_blocking_worker(move || {
            started_tx.send(()).unwrap();
            // A regression to an inline call must terminate with a failed
            // assertion rather than deadlock this single-thread runtime.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while !worker_progress.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
                std::thread::yield_now();
            }
            Ok(worker_progress.load(Ordering::Acquire))
        }));
        started_rx.await.unwrap();
        runtime_progress.store(true, Ordering::Release);
        assert!(recovery.await.unwrap().unwrap());
    }

    #[tokio::test]
    async fn news_authority_recovery_preserves_typed_authority_errors() {
        let expected = NewsFlashReconcileError::InvalidChain("TEST_CODE_bad_chain".into());
        let original = expected.clone();
        let result = on_blocking_worker(move || Err::<(), _>(original)).await;
        assert_eq!(result.unwrap_err(), expected);
    }

    #[tokio::test]
    async fn news_authority_recovery_worker_panic_cannot_authorize_reservation() {
        let result = on_blocking_worker(|| -> Result<(), NewsFlashReconcileError> {
            panic!("TEST_CODE_recovery_worker_failed");
        })
        .await;
        assert_eq!(
            result.unwrap_err(),
            NewsFlashReconcileError::AuthorityUnavailable(
                "news_flash_authority_worker_join_failed".into()
            )
        );
    }
}
