use super::{NotificationChannel, NotificationConfig, NotificationSendReport, NotificationService};
use crate::monitor::push_job::WeakOutcomeKind;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};

enum ScriptedResponse {
    Http(&'static str),
    Disconnect,
}

const FIXTURE_LIFETIME: Duration = Duration::from_secs(8);
const MAX_REQUEST_BYTES: usize = 1024 * 1024;

struct WebhookFixture {
    url: String,
    requests: Arc<Mutex<Vec<Vec<u8>>>>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<Result<(), String>>>,
}

impl WebhookFixture {
    fn url(&self) -> String {
        self.url.clone()
    }

    fn finish(mut self) -> Vec<Vec<u8>> {
        let result = self
            .handle
            .take()
            .expect("fixture thread handle")
            .join()
            .expect("fixture thread should not panic");
        result.expect("fixture should serve every scripted response");
        let requests = self.requests.lock().expect("fixture requests").clone();
        requests
    }
}

impl Drop for WebhookFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            handle.thread().unpark();
            let _ = handle.join();
        }
    }
}

fn spawn_webhook_fixture(responses: Vec<ScriptedResponse>) -> WebhookFixture {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind random loopback port");
    listener
        .set_nonblocking(true)
        .expect("set fixture listener nonblocking");
    let address = listener.local_addr().expect("fixture address");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let thread_requests = Arc::clone(&requests);
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);

    let handle = thread::spawn(move || {
        let deadline = Instant::now() + FIXTURE_LIFETIME;
        for response in responses {
            let (mut stream, _) = loop {
                if thread_stop.load(Ordering::Acquire) {
                    return Err("fixture stopped before all requests arrived".to_string());
                }
                if Instant::now() >= deadline {
                    return Err("fixture timed out waiting for request".to_string());
                }
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        thread::park_timeout(Duration::from_millis(5));
                    }
                    Err(error) => return Err(format!("fixture accept failed: {error}")),
                }
            };
            let request = read_complete_http_request(&mut stream, deadline)?;
            thread_requests
                .lock()
                .map_err(|_| "fixture request mutex poisoned".to_string())?
                .push(request);

            match response {
                ScriptedResponse::Http(body) => {
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let remaining = remaining_until(deadline, "write response")?;
                    stream
                        .set_write_timeout(Some(remaining))
                        .map_err(|error| format!("set write timeout: {error}"))?;
                    stream
                        .write_all(response.as_bytes())
                        .map_err(|error| format!("write fixture response: {error}"))?;
                    remaining_until(deadline, "flush response")?;
                    stream
                        .flush()
                        .map_err(|error| format!("flush fixture response: {error}"))?;
                }
                ScriptedResponse::Disconnect => {}
            }
        }
        Ok(())
    });

    WebhookFixture {
        url: format!("http://{address}/TEST_CODE_fixture"),
        requests,
        stop,
        handle: Some(handle),
    }
}

fn remaining_until(deadline: Instant, operation: &str) -> Result<Duration, String> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| format!("fixture lifetime expired before {operation}"))
}

fn read_complete_http_request(
    stream: &mut TcpStream,
    deadline: Instant,
) -> Result<Vec<u8>, String> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let remaining = remaining_until(deadline, "read request")?;
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|error| format!("set read timeout: {error}"))?;
        let read = stream
            .read(&mut buffer)
            .map_err(|error| format!("read fixture request: {error}"))?;
        if read == 0 {
            return Err("request closed before declared body completed".to_string());
        }
        request.extend_from_slice(&buffer[..read]);
        if request.len() > MAX_REQUEST_BYTES {
            return Err("fixture request exceeded size limit".to_string());
        }

        if let Some(header_end) = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|position| position + 4)
        {
            let content_length = parse_content_length(&request[..header_end])?;
            let request_length = header_end
                .checked_add(content_length)
                .filter(|length| *length <= MAX_REQUEST_BYTES)
                .ok_or_else(|| "declared Content-Length exceeds fixture limit".to_string())?;
            if request.len() >= request_length {
                request.truncate(request_length);
                return Ok(request);
            }
        }
    }
}

fn parse_content_length(headers: &[u8]) -> Result<usize, String> {
    let headers =
        std::str::from_utf8(headers).map_err(|error| format!("request headers UTF-8: {error}"))?;
    for line in headers.split("\r\n") {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                return value
                    .trim()
                    .parse()
                    .map_err(|error| format!("invalid Content-Length: {error}"));
            }
        }
    }
    Ok(0)
}

fn test_service(
    config: NotificationConfig,
    available_channels: Vec<NotificationChannel>,
) -> NotificationService {
    NotificationService {
        config,
        client: reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .expect("build no-proxy test client"),
        available_channels,
    }
}

fn assert_attempts(
    report: &NotificationSendReport,
    expected: &[(NotificationChannel, usize, WeakOutcomeKind)],
) {
    assert_eq!(report.attempts().len(), expected.len());
    for (attempt, (channel, target_index, outcome)) in report.attempts().iter().zip(expected.iter())
    {
        assert_eq!(attempt.channel(), *channel);
        assert_eq!(attempt.target_index(), *target_index);
        assert_eq!(attempt.outcome(), *outcome);
    }
}

#[tokio::test]
async fn no_channels_report_is_empty_and_legacy_send_is_false() {
    let service = test_service(NotificationConfig::default(), Vec::new());

    let report = service.send_report("TEST_CODE no channels").await;

    assert!(report.attempts().is_empty());
    assert!(!report.has_success());
    assert!(!service.send("TEST_CODE no channels").await.unwrap());
}

#[tokio::test]
async fn custom_success_and_false_are_distinct_weak_observations() {
    let accepted = spawn_webhook_fixture(vec![ScriptedResponse::Http(r#"{"ok":true}"#)]);
    let declined = spawn_webhook_fixture(vec![ScriptedResponse::Http(r#"{"ok":false}"#)]);
    let config = NotificationConfig {
        custom_webhook_urls: vec![accepted.url(), declined.url()],
        ..NotificationConfig::default()
    };
    let service = test_service(config, vec![NotificationChannel::Custom]);

    let report = service.send_report("TEST_CODE mixed custom").await;

    assert_attempts(
        &report,
        &[
            (NotificationChannel::Custom, 0, WeakOutcomeKind::Accepted),
            (NotificationChannel::Custom, 1, WeakOutcomeKind::Unknown),
        ],
    );
    assert!(report.has_success());
    assert_eq!(accepted.finish().len(), 1);
    assert_eq!(declined.finish().len(), 1);
}

#[tokio::test]
async fn custom_unknown_results_do_not_stop_later_targets_or_retry() {
    let declined = spawn_webhook_fixture(vec![ScriptedResponse::Http(r#"{"ok":false}"#)]);
    let malformed =
        spawn_webhook_fixture(vec![ScriptedResponse::Http("TEST_CODE_MALFORMED_RESPONSE")]);
    let disconnected = spawn_webhook_fixture(vec![ScriptedResponse::Disconnect]);
    let config = NotificationConfig {
        custom_webhook_urls: vec![declined.url(), malformed.url(), disconnected.url()],
        ..NotificationConfig::default()
    };
    let service = test_service(config, vec![NotificationChannel::Custom]);

    let report = service.send_report("TEST_CODE all unknown").await;

    assert_attempts(
        &report,
        &[
            (NotificationChannel::Custom, 0, WeakOutcomeKind::Unknown),
            (NotificationChannel::Custom, 1, WeakOutcomeKind::Unknown),
            (NotificationChannel::Custom, 2, WeakOutcomeKind::Unknown),
        ],
    );
    assert!(!report.has_success());
    assert_eq!(declined.finish().len(), 1);
    assert_eq!(malformed.finish().len(), 1);
    assert_eq!(disconnected.finish().len(), 1);
}

#[tokio::test]
async fn mixed_builtin_channels_keep_target_order_and_protocol_success() {
    let wechat = spawn_webhook_fixture(vec![ScriptedResponse::Http(r#"{"errcode":0}"#)]);
    let feishu = spawn_webhook_fixture(vec![ScriptedResponse::Http(r#"{"code":0}"#)]);
    let dingtalk = spawn_webhook_fixture(vec![ScriptedResponse::Http(r#"{"errcode":0}"#)]);
    let slack = spawn_webhook_fixture(vec![ScriptedResponse::Http("ok\n")]);
    let discord = spawn_webhook_fixture(vec![ScriptedResponse::Http("")]);
    let custom = spawn_webhook_fixture(vec![ScriptedResponse::Http(r#"{"ok":true}"#)]);
    let config = NotificationConfig {
        wechat_webhook_url: Some(wechat.url()),
        feishu_webhook_url: Some(feishu.url()),
        dingtalk_webhook_url: Some(dingtalk.url()),
        slack_webhook_url: Some(slack.url()),
        discord_webhook_url: Some(discord.url()),
        custom_webhook_urls: vec![custom.url()],
        wechat_max_bytes: 4_000,
        feishu_max_bytes: 20_000,
        ..NotificationConfig::default()
    };
    let service = test_service(
        config,
        vec![
            NotificationChannel::Wechat,
            NotificationChannel::Feishu,
            NotificationChannel::DingTalk,
            NotificationChannel::Slack,
            NotificationChannel::Discord,
            NotificationChannel::Custom,
        ],
    );

    let report = service.send_report("TEST_CODE builtin success").await;

    assert_attempts(
        &report,
        &[
            (NotificationChannel::Wechat, 0, WeakOutcomeKind::Accepted),
            (NotificationChannel::Feishu, 1, WeakOutcomeKind::Accepted),
            (NotificationChannel::DingTalk, 2, WeakOutcomeKind::Accepted),
            (NotificationChannel::Slack, 3, WeakOutcomeKind::Accepted),
            (NotificationChannel::Discord, 4, WeakOutcomeKind::Accepted),
            (NotificationChannel::Custom, 5, WeakOutcomeKind::Accepted),
        ],
    );
    assert!(report.has_success());
    assert_eq!(wechat.finish().len(), 1);
    assert_eq!(feishu.finish().len(), 1);
    assert_eq!(dingtalk.finish().len(), 1);
    assert_eq!(slack.finish().len(), 1);
    assert_eq!(discord.finish().len(), 1);
    assert_eq!(custom.finish().len(), 1);
}

#[tokio::test]
async fn wechat_partial_chunks_remain_one_unknown_target_for_false_and_error() {
    let content = format!("TEST_CODE {}\n---\n{}", "A".repeat(120), "B".repeat(120));

    let later_false = spawn_webhook_fixture(vec![
        ScriptedResponse::Http(r#"{"errcode":0}"#),
        ScriptedResponse::Http(r#"{"errcode":1}"#),
    ]);
    let false_service = test_service(
        NotificationConfig {
            wechat_webhook_url: Some(later_false.url()),
            wechat_max_bytes: 220,
            ..NotificationConfig::default()
        },
        vec![NotificationChannel::Wechat],
    );
    let false_report = false_service.send_report(&content).await;
    assert_attempts(
        &false_report,
        &[(NotificationChannel::Wechat, 0, WeakOutcomeKind::Unknown)],
    );
    assert_eq!(later_false.finish().len(), 2);

    let later_error = spawn_webhook_fixture(vec![
        ScriptedResponse::Http(r#"{"errcode":0}"#),
        ScriptedResponse::Http("TEST_CODE_INVALID_JSON"),
    ]);
    let error_service = test_service(
        NotificationConfig {
            wechat_webhook_url: Some(later_error.url()),
            wechat_max_bytes: 220,
            ..NotificationConfig::default()
        },
        vec![NotificationChannel::Wechat],
    );
    let error_report = error_service.send_report(&content).await;
    assert_attempts(
        &error_report,
        &[(NotificationChannel::Wechat, 0, WeakOutcomeKind::Unknown)],
    );
    assert_eq!(later_error.finish().len(), 2);
}

#[tokio::test]
async fn feishu_fallback_is_internal_to_one_observed_target() {
    let fallback_false = spawn_webhook_fixture(vec![
        ScriptedResponse::Http(r#"{"code":1}"#),
        ScriptedResponse::Http(r#"{"code":1}"#),
    ]);
    let false_service = test_service(
        NotificationConfig {
            feishu_webhook_url: Some(fallback_false.url()),
            feishu_max_bytes: 20_000,
            ..NotificationConfig::default()
        },
        vec![NotificationChannel::Feishu],
    );
    let false_report = false_service.send_report("TEST_CODE fallback false").await;
    assert_attempts(
        &false_report,
        &[(NotificationChannel::Feishu, 0, WeakOutcomeKind::Unknown)],
    );
    assert_eq!(fallback_false.finish().len(), 2);

    let fallback_success = spawn_webhook_fixture(vec![
        ScriptedResponse::Http(r#"{"code":1}"#),
        ScriptedResponse::Http(r#"{"code":0}"#),
    ]);
    let success_service = test_service(
        NotificationConfig {
            feishu_webhook_url: Some(fallback_success.url()),
            feishu_max_bytes: 20_000,
            ..NotificationConfig::default()
        },
        vec![NotificationChannel::Feishu],
    );
    let success_report = success_service
        .send_report("TEST_CODE fallback success")
        .await;
    assert_attempts(
        &success_report,
        &[(NotificationChannel::Feishu, 0, WeakOutcomeKind::Accepted)],
    );
    assert_eq!(fallback_success.finish().len(), 2);
}

#[tokio::test]
async fn feishu_empty_heading_reaches_two_chunks_but_one_unknown_target() {
    let fixture = spawn_webhook_fixture(vec![
        ScriptedResponse::Http(r#"{"code":0}"#),
        ScriptedResponse::Http(r#"{"code":1}"#),
        ScriptedResponse::Http(r#"{"code":1}"#),
    ]);
    let service = test_service(
        NotificationConfig {
            feishu_webhook_url: Some(fixture.url()),
            feishu_max_bytes: 512,
            ..NotificationConfig::default()
        },
        vec![NotificationChannel::Feishu],
    );
    let content = format!("TEST_CODE {}\n### \n", "A".repeat(600));

    let report = service.send_report(&content).await;

    assert_attempts(
        &report,
        &[(NotificationChannel::Feishu, 0, WeakOutcomeKind::Unknown)],
    );
    assert_eq!(fixture.finish().len(), 3);
}

#[tokio::test]
async fn feishu_regular_long_report_keeps_existing_single_truncated_chunk() {
    let fixture = spawn_webhook_fixture(vec![ScriptedResponse::Http(r#"{"code":0}"#)]);
    let service = test_service(
        NotificationConfig {
            feishu_webhook_url: Some(fixture.url()),
            feishu_max_bytes: 512,
            ..NotificationConfig::default()
        },
        vec![NotificationChannel::Feishu],
    );
    let content = format!(
        "# TEST_CODE heading\n{}\n---\n### TEST_CODE section\nTEST_CODE_TAIL_SECRET",
        "A".repeat(600)
    );

    let report = service.send_report(&content).await;

    assert_attempts(
        &report,
        &[(NotificationChannel::Feishu, 0, WeakOutcomeKind::Accepted)],
    );
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    assert!(!String::from_utf8_lossy(&requests[0]).contains("TEST_CODE_TAIL_SECRET"));
}

#[tokio::test]
async fn legacy_send_projects_the_same_custom_truth_matrix_without_extra_requests() {
    for (bodies, expected) in [
        ([r#"{"ok":true}"#, r#"{"ok":false}"#], true),
        ([r#"{"ok":false}"#, r#"{"ok":false}"#], false),
    ] {
        let report_fixture = spawn_webhook_fixture(vec![
            ScriptedResponse::Http(bodies[0]),
            ScriptedResponse::Http(bodies[1]),
        ]);
        let report_service = test_service(
            NotificationConfig {
                custom_webhook_urls: vec![report_fixture.url(), report_fixture.url()],
                ..NotificationConfig::default()
            },
            vec![NotificationChannel::Custom],
        );
        let report = report_service.send_report("TEST_CODE report matrix").await;
        assert_eq!(report.has_success(), expected);
        assert_eq!(report_fixture.finish().len(), 2);

        let send_fixture = spawn_webhook_fixture(vec![
            ScriptedResponse::Http(bodies[0]),
            ScriptedResponse::Http(bodies[1]),
        ]);
        let send_service = test_service(
            NotificationConfig {
                custom_webhook_urls: vec![send_fixture.url(), send_fixture.url()],
                ..NotificationConfig::default()
            },
            vec![NotificationChannel::Custom],
        );
        assert_eq!(
            send_service.send("TEST_CODE send matrix").await.unwrap(),
            expected
        );
        assert_eq!(send_fixture.finish().len(), 2);
    }
}

#[tokio::test]
async fn duplicate_custom_url_is_attempted_twice_and_empty_custom_has_no_attempt() {
    let duplicate = spawn_webhook_fixture(vec![
        ScriptedResponse::Http(r#"{"ok":true}"#),
        ScriptedResponse::Http(r#"{"ok":true}"#),
    ]);
    let duplicate_service = test_service(
        NotificationConfig {
            custom_webhook_urls: vec![duplicate.url(), duplicate.url()],
            ..NotificationConfig::default()
        },
        vec![NotificationChannel::Custom],
    );

    let duplicate_report = duplicate_service.send_report("TEST_CODE duplicate").await;

    assert_attempts(
        &duplicate_report,
        &[
            (NotificationChannel::Custom, 0, WeakOutcomeKind::Accepted),
            (NotificationChannel::Custom, 1, WeakOutcomeKind::Accepted),
        ],
    );
    assert_eq!(duplicate.finish().len(), 2);

    let empty_service = test_service(
        NotificationConfig::default(),
        vec![NotificationChannel::Custom],
    );
    let empty_report = empty_service.send_report("TEST_CODE empty custom").await;
    assert!(empty_report.attempts().is_empty());
    assert!(!empty_report.has_success());
}

#[tokio::test]
async fn report_debug_omits_content_url_token_response_and_raw_error() {
    let fixture = spawn_webhook_fixture(vec![ScriptedResponse::Http("TEST_CODE_RESPONSE_SECRET")]);
    let secret_url = format!("{}/TEST_CODE_URL_SECRET", fixture.url());
    let service = test_service(
        NotificationConfig {
            custom_webhook_urls: vec![secret_url],
            custom_webhook_bearer_token: Some("TEST_CODE_TOKEN_SECRET".to_string()),
            ..NotificationConfig::default()
        },
        vec![NotificationChannel::Custom],
    );

    let report = service.send_report("TEST_CODE_CONTENT_SECRET").await;

    let debug = format!("{report:?}");
    assert!(debug.contains("Custom"));
    assert!(debug.contains("Unknown"));
    assert!(debug.contains("target_index: 0"));
    for secret in [
        "TEST_CODE_CONTENT_SECRET",
        "TEST_CODE_URL_SECRET",
        "TEST_CODE_TOKEN_SECRET",
        "TEST_CODE_RESPONSE_SECRET",
    ] {
        assert!(!debug.contains(secret));
    }
    assert_eq!(fixture.finish().len(), 1);
}
