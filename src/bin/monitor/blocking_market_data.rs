pub async fn run_blocking_market_data<T, F>(label: &'static str, operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    match tokio::task::spawn_blocking(operation).await {
        Ok(result) => result,
        Err(error) => Err(format!("{label} blocking task failed: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::run_blocking_market_data;

    #[tokio::test(flavor = "current_thread")]
    async fn blocking_market_data_owns_reqwest_blocking_client_off_async_worker() {
        run_blocking_market_data("TEST_CODE reqwest lifecycle", || {
            let client = reqwest::blocking::Client::builder()
                .no_proxy()
                .build()
                .map_err(|error| error.to_string())?;
            drop(client);
            Ok(())
        })
        .await
        .expect("blocking client lifecycle must remain outside async worker");
    }

    #[tokio::test]
    async fn blocking_market_data_preserves_business_error() {
        let error = run_blocking_market_data("TEST_CODE source", || {
            Err::<(), _>("source batch rejected".to_string())
        })
        .await
        .expect_err("business rejection must remain visible");

        assert_eq!(error, "source batch rejected");
    }

    #[tokio::test]
    async fn blocking_market_data_converts_worker_panic_to_labeled_error() {
        let error = run_blocking_market_data("TEST_CODE panic", || -> Result<(), String> {
            panic!("forced blocking worker panic")
        })
        .await
        .expect_err("worker panic must become an explicit error");

        assert!(
            error.contains("TEST_CODE panic blocking task failed"),
            "{error}"
        );
        assert!(error.contains("panicked"), "{error}");
    }
}
