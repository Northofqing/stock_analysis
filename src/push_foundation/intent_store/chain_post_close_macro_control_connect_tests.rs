use super::*;

#[tokio::test]
async fn single_user_external_macro_health_connect_unavailable_reopens_without_reconnect() {
    control_tests::run_control_rejection(control_tests::RejectionCase::HealthConnectUnavailable)
        .await;
}
