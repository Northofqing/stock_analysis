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

    fn recorded_requests(&self) -> Vec<Vec<u8>> {
        self.requests.lock().expect("fixture requests").clone()
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
    spawn_webhook_fixture_with_lifetime(responses, FIXTURE_LIFETIME)
}

fn spawn_webhook_fixture_with_lifetime(
    responses: Vec<ScriptedResponse>,
    lifetime: Duration,
) -> WebhookFixture {
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
        let deadline = Instant::now() + lifetime;
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
            // The listener is nonblocking so accept can be stopped, but the complete HTTP
            // reader needs a blocking connection bounded by its deadline.
            stream
                .set_nonblocking(false)
                .map_err(|error| format!("set fixture stream blocking: {error}"))?;
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

fn feishu_payload(request: &[u8]) -> serde_json::Value {
    let body_start = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
        .expect("captured request must contain an HTTP body");
    serde_json::from_slice(&request[body_start..]).expect("Feishu request body must be JSON")
}

fn feishu_payload_content(request: &[u8]) -> String {
    let payload = feishu_payload(request);
    let content = match payload.get("msg_type").and_then(serde_json::Value::as_str) {
        Some("interactive") => payload.pointer("/card/elements/0/text/content"),
        Some("text") => payload.pointer("/content/text"),
        other => panic!("unexpected Feishu payload type: {other:?}"),
    };
    content
        .and_then(serde_json::Value::as_str)
        .expect("Feishu payload must contain message text")
        .to_owned()
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
async fn feishu_regular_long_report_is_lossless_through_observed_public_entry() {
    const FEISHU_MAX_BYTES: usize = 384;
    const MARKDOWN: &str = "# TEST_CODE 普通长报告\n> 风险提示🙂\n---\n- 第一项\n- 第二项\n\n连续长段开始：甲乙丙丁戊己庚辛壬癸，行情快照保持原序；研究摘要跨越边界仍需连续；emoji🚀与中文测试🙂不能拆坏；资金流、公告、财务指标、风险提示依次完整保留；这是没有旧标题或分隔符可借用的普通正文，分片不能以截断换取成功；再补一段连续字符ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789abcdefghijklmnopqrstuvwxyz，确保正文确定超过预算；末端附近还有中文字符壹贰叁肆伍陆柒捌玖拾与emoji🧭。\n\n中间空行必须保留。\n\nTEST_CODE_TAIL_UNIQUE";
    const FORMATTED: &str = "**TEST_CODE 普通长报告**\n💬 风险提示🙂\n────────\n• 第一项\n• 第二项\n\n连续长段开始：甲乙丙丁戊己庚辛壬癸，行情快照保持原序；研究摘要跨越边界仍需连续；emoji🚀与中文测试🙂不能拆坏；资金流、公告、财务指标、风险提示依次完整保留；这是没有旧标题或分隔符可借用的普通正文，分片不能以截断换取成功；再补一段连续字符ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789abcdefghijklmnopqrstuvwxyz，确保正文确定超过预算；末端附近还有中文字符壹贰叁肆伍陆柒捌玖拾与emoji🧭。\n\n中间空行必须保留。\n\nTEST_CODE_TAIL_UNIQUE";

    assert!(FORMATTED.len() > FEISHU_MAX_BYTES);
    let fixture = spawn_webhook_fixture(
        (0..8)
            .map(|_| ScriptedResponse::Http(r#"{"code":0}"#))
            .collect(),
    );
    let service = test_service(
        NotificationConfig {
            feishu_webhook_url: Some(fixture.url()),
            feishu_max_bytes: FEISHU_MAX_BYTES,
            ..NotificationConfig::default()
        },
        vec![NotificationChannel::Feishu],
    );

    let report = service.send_report(MARKDOWN).await;

    assert_attempts(
        &report,
        &[(NotificationChannel::Feishu, 0, WeakOutcomeKind::Accepted)],
    );
    let requests = fixture.recorded_requests();
    drop(fixture);
    assert!(
        requests.len() > 1,
        "an over-budget ordinary report must use more than one request"
    );

    let total = requests.len();
    let mut reconstructed = String::new();
    for (index, request) in requests.iter().enumerate() {
        let body = feishu_payload_content(request);
        assert!(
            body.len() <= FEISHU_MAX_BYTES,
            "Feishu body {} is {} bytes, over configured {}-byte budget",
            index + 1,
            body.len(),
            FEISHU_MAX_BYTES
        );
        let marker = format!("\n\n📄 ({}/{})", index + 1, total);
        let content = body
            .strip_suffix(&marker)
            .unwrap_or_else(|| panic!("Feishu body {} has wrong page marker", index + 1));
        reconstructed.push_str(content);
    }

    assert_eq!(reconstructed, FORMATTED);
    assert!(reconstructed.ends_with("TEST_CODE_TAIL_UNIQUE"));
}

#[tokio::test]
async fn feishu_invalid_budgets_fail_before_first_http_request() {
    let cases = [
        (0, "A".to_owned(), "必须大于 0"),
        (12, "A".repeat(13), "不足以容纳分页标记和消息内容"),
        (
            14,
            format!("{}🧭", "A".repeat(11)),
            "无法容纳 4 字节的 Unicode 字符",
        ),
    ];

    for (max_bytes, content, expected_error) in cases {
        let fixture = spawn_webhook_fixture(vec![ScriptedResponse::Http(r#"{"code":0}"#)]);
        let service = test_service(
            NotificationConfig {
                feishu_webhook_url: Some(fixture.url()),
                feishu_max_bytes: max_bytes,
                ..NotificationConfig::default()
            },
            vec![NotificationChannel::Feishu],
        );

        let error = service
            .send_to_feishu(&content)
            .await
            .expect_err("invalid Feishu budget must fail locally");
        assert!(
            error.to_string().contains(expected_error),
            "budget {max_bytes} returned unexpected error: {error}"
        );

        let report = service.send_report(&content).await;
        assert_attempts(
            &report,
            &[(NotificationChannel::Feishu, 0, WeakOutcomeKind::Unknown)],
        );
        assert!(!report.has_success());
        assert!(
            fixture.recorded_requests().is_empty(),
            "budget {max_bytes} must be rejected before the first HTTP request"
        );
        drop(fixture);
    }
}

#[tokio::test]
async fn feishu_exact_short_body_is_one_unmarked_request() {
    const CONTENT: &str = "测试🙂";
    const FEISHU_MAX_BYTES: usize = 10;
    assert_eq!(CONTENT.len(), FEISHU_MAX_BYTES);

    let fixture = spawn_webhook_fixture(vec![ScriptedResponse::Http(r#"{"code":0}"#)]);
    let service = test_service(
        NotificationConfig {
            feishu_webhook_url: Some(fixture.url()),
            feishu_max_bytes: FEISHU_MAX_BYTES,
            ..NotificationConfig::default()
        },
        vec![NotificationChannel::Feishu],
    );

    let report = service.send_report(CONTENT).await;

    assert_attempts(
        &report,
        &[(NotificationChannel::Feishu, 0, WeakOutcomeKind::Accepted)],
    );
    let requests = fixture.finish();
    assert_eq!(requests.len(), 1);
    let payload = feishu_payload(&requests[0]);
    assert_eq!(
        payload.get("msg_type").and_then(serde_json::Value::as_str),
        Some("interactive")
    );
    let body = payload
        .pointer("/card/elements/0/text/content")
        .and_then(serde_json::Value::as_str)
        .expect("interactive Feishu card must contain message text");
    assert_eq!(body.as_bytes(), CONTENT.as_bytes());
    assert!(!body.contains("\n\n📄 ("));
}

#[tokio::test]
async fn feishu_two_digit_pages_stay_bounded_and_lossless() {
    const CONTENT: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const FEISHU_MAX_BYTES: usize = 16;
    const MAX_RESPONSES: usize = 40;
    const LONG_FIXTURE_LIFETIME: Duration = Duration::from_secs(50);
    const OUTER_TIMEOUT: Duration = Duration::from_secs(55);
    assert_eq!(CONTENT.len(), 40);

    let fixture = spawn_webhook_fixture_with_lifetime(
        (0..MAX_RESPONSES)
            .map(|_| ScriptedResponse::Http(r#"{"code":0}"#))
            .collect(),
        LONG_FIXTURE_LIFETIME,
    );
    let service = test_service(
        NotificationConfig {
            feishu_webhook_url: Some(fixture.url()),
            feishu_max_bytes: FEISHU_MAX_BYTES,
            ..NotificationConfig::default()
        },
        vec![NotificationChannel::Feishu],
    );

    let (report, requests) = tokio::time::timeout(OUTER_TIMEOUT, async move {
        let report = service.send_report(CONTENT).await;
        let requests = fixture.recorded_requests();
        drop(fixture);
        (report, requests)
    })
    .await
    .expect("two-digit Feishu paging must finish within 55 seconds");

    assert_attempts(
        &report,
        &[(NotificationChannel::Feishu, 0, WeakOutcomeKind::Accepted)],
    );
    let total = requests.len();
    assert!(
        total >= 10,
        "fixture must exercise two-digit page-number markers, got {total} pages"
    );

    let mut reconstructed = String::new();
    for (index, request) in requests.iter().enumerate() {
        let body = feishu_payload_content(request);
        assert!(
            body.len() <= FEISHU_MAX_BYTES,
            "Feishu body {} is {} bytes, over configured {}-byte budget",
            index + 1,
            body.len(),
            FEISHU_MAX_BYTES
        );
        let marker = format!("\n\n📄 ({}/{})", index + 1, total);
        let content = body
            .strip_suffix(&marker)
            .unwrap_or_else(|| panic!("Feishu body {} has wrong page marker", index + 1));
        assert!(
            !content.is_empty(),
            "Feishu page {} has no report body",
            index + 1
        );
        reconstructed.push_str(content);
    }

    assert_eq!(reconstructed.as_bytes(), CONTENT.as_bytes());
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
