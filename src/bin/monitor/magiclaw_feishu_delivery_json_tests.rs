use super::*;
use std::os::unix::fs::PermissionsExt;

const TARGET: &str = "oc_TEST_CODE_phase";
const BODY: &str = "TEST_CODE_PHASE_BODY";

struct CliFixture {
    _root: tempfile::TempDir,
    calls: std::path::PathBuf,
    executable: std::path::PathBuf,
    _env: crate::TestEnvGuard,
}

impl CliFixture {
    fn new(kind: &str, phase: &str, started: bool, reason: &str, exit: i32) -> Self {
        let root = tempfile::tempdir().unwrap();
        let calls = root.path().join("calls.txt");
        let executable = root.path().join("magiclaw-stub");
        let env = crate::TestEnvGuard::capture(&[
            "MAGICLAW_BIN",
            "MAGICLAW_HOME",
            "MAGICLAW_DB_PATH",
            "FEISHU_TO",
            "FEISHU_RECEIVE_ID_TYPE",
            "MAGICLAW_FEISHU_DELIVERY_JSON_V1",
            "MAGICLAW_FEISHU_EXPECTED_APP_ID_SHA256",
            "MAGICLAW_FEISHU_EXPECTED_ACCOUNT_ID",
        ]);
        std::env::set_var("MAGICLAW_BIN", &executable);
        std::env::set_var("MAGICLAW_HOME", root.path());
        std::env::remove_var("MAGICLAW_DB_PATH");
        std::env::set_var("FEISHU_TO", TARGET);
        std::env::remove_var("FEISHU_RECEIVE_ID_TYPE");
        std::env::set_var("MAGICLAW_FEISHU_DELIVERY_JSON_V1", "1");
        std::env::set_var("MAGICLAW_FEISHU_EXPECTED_APP_ID_SHA256", "a".repeat(64));
        std::env::set_var("MAGICLAW_FEISHU_EXPECTED_ACCOUNT_ID", "TEST_CODE_account");
        let fixture = Self {
            _root: root,
            calls,
            executable,
            _env: env,
        };
        let mut value = serde_json::json!({
            "schema": "magiclaw.feishu_delivery.v1", "invocation_id": "TEST_INVOCATION_MARKER",
            "channel": "feishu", "account_id": "TEST_CODE_account", "app_id_sha256": "a".repeat(64),
            "receive_id_type": "chat_id",
            "target_sha256": "8abf4cf6ef74d2c7b73296df7874d3ee0d43b81981d2d31f78514f2d9de783c5",
            "content_sha256": "b340b09baf53bfad643288dd34034affbfdab978cf3050e2e728ec7da18b5bb0",
            "kind": kind, "phase": phase, "message_request_started": started, "reason_code": reason,
        });
        if kind == "Accepted" {
            value["receipt"] = serde_json::json!({
                "message_id": "fedcba98-7654-4321-8abc-def012345678",
                "platform_message_id": "om_x100b6343eab8f0a4c4556e4b1737efa",
            });
        }
        fixture.set_output(&value.to_string(), &format!("exit {exit}"));
        fixture
    }

    fn set_output(&self, stdout: &str, finish: &str) {
        let quoted = stdout
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "\\$")
            .replace('`', "\\`")
            .replace("TEST_INVOCATION_MARKER", "$task_invocation");
        let script = format!(
            "#!/bin/sh\ntask_invocation=missing\ntask_optin=0\ntask_receive=missing\nwhile [ \"$#\" -gt 0 ]; do\n  case \"$1\" in\n    --invocation-id) task_invocation=$2; shift 2;;\n    --delivery-result-json-v1) task_optin=1; shift;;\n    --receive-id-type) task_receive=$2; shift 2;;\n    *) shift;;\n  esac\ndone\nprintf '%s|%s|%s\\n' \"$task_invocation\" \"$task_optin\" \"$task_receive\" >> '{}'\nprintf '%s\\n' \"{quoted}\"\n{finish}\n", self.calls.display());
        std::fs::write(&self.executable, script).unwrap();
        std::fs::set_permissions(&self.executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(&self.calls)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn call_count(&self) -> usize {
        std::fs::read_to_string(&self.calls)
            .map(|text| text.lines().count())
            .unwrap_or(0)
    }
}

#[test]
#[serial_test::serial(notify_env)]
fn feishu_json_v1_cli_auth_failure_is_rejected_after_one_spawn() {
    let fixture = CliFixture::new(
        "RejectedBeforeMessage",
        "auth",
        false,
        "feishu_auth_failed_before_message",
        1,
    );
    let result = push_via_magiclaw_cli_receipt_blocking(MessageSendType::Feishu, BODY);
    assert!(matches!(
        result,
        Err(BlockingCliDeliveryFailure::Rejected { reason_code, .. })
            if reason_code == "feishu_auth_failed_before_message"
    ));
    assert_eq!(fixture.call_count(), 1, "classification must never resend");
    let calls = fixture.calls();
    assert!(
        calls[0].ends_with("|1|chat_id"),
        "opt-in and receive type must be explicit"
    );
}

#[test]
#[serial_test::serial(notify_env)]
fn feishu_json_v1_cli_message_and_preflight_errors_remain_uncertain() {
    for (phase, started, reason) in [
        ("message", true, "feishu_message_result_unconfirmed"),
        ("preflight", false, "feishu_preflight_unconfirmed"),
    ] {
        let fixture = CliFixture::new("Uncertain", phase, started, reason, 1);
        assert!(
            matches!(push_via_magiclaw_cli_receipt_blocking(MessageSendType::Feishu, BODY),
            Err(BlockingCliDeliveryFailure::Uncertain { reason_code, .. }) if reason_code == reason)
        );
        assert_eq!(fixture.call_count(), 1);
    }
}

#[test]
#[serial_test::serial(notify_env)]
fn feishu_json_v1_cli_accepts_real_id_shape_and_uses_fresh_nonce_per_spawn() {
    let fixture = CliFixture::new("Accepted", "message", true, "accepted", 0);
    for _ in 0..2 {
        assert_eq!(
            push_via_magiclaw_cli_receipt_blocking(MessageSendType::Feishu, BODY).ok(),
            Some(CliDeliveryReceipt {
                message_id: "fedcba98-7654-4321-8abc-def012345678".to_owned(),
                platform_msg_id: "om_x100b6343eab8f0a4c4556e4b1737efa".to_owned(),
            })
        );
    }
    let calls = fixture.calls();
    assert_eq!(calls.len(), 2);
    assert_ne!(calls[0], calls[1]);
}

#[test]
#[serial_test::serial(notify_env)]
fn feishu_json_v1_cli_missing_identity_and_invalid_receive_type_do_not_spawn() {
    for key in [
        "MAGICLAW_FEISHU_EXPECTED_ACCOUNT_ID",
        "MAGICLAW_FEISHU_EXPECTED_APP_ID_SHA256",
    ] {
        let fixture = CliFixture::new(
            "RejectedBeforeMessage",
            "auth",
            false,
            "feishu_auth_failed_before_message",
            1,
        );
        std::env::remove_var(key);
        assert!(
            matches!(push_via_magiclaw_cli_receipt_blocking(MessageSendType::Feishu, BODY),
            Err(BlockingCliDeliveryFailure::Rejected { reason_code, .. }) if reason_code == "magiclaw_feishu_expected_identity_missing")
        );
        assert_eq!(fixture.call_count(), 0);
    }
    let fixture = CliFixture::new(
        "RejectedBeforeMessage",
        "auth",
        false,
        "feishu_auth_failed_before_message",
        1,
    );
    std::env::set_var("FEISHU_RECEIVE_ID_TYPE", "TEST_CODE_invalid");
    assert!(
        matches!(push_via_magiclaw_cli_receipt_blocking(MessageSendType::Feishu, BODY),
        Err(BlockingCliDeliveryFailure::Rejected { reason_code, .. }) if reason_code == "magiclaw_feishu_receive_id_type_invalid")
    );
    assert_eq!(fixture.call_count(), 0);
}

#[test]
#[serial_test::serial(notify_env)]
fn feishu_json_v1_cli_invalid_identity_and_optin_config_do_not_spawn() {
    for (key, value) in [
        ("MAGICLAW_FEISHU_EXPECTED_ACCOUNT_ID", ""),
        (
            "MAGICLAW_FEISHU_EXPECTED_APP_ID_SHA256",
            "TEST_CODE_not_a_hash",
        ),
        ("MAGICLAW_FEISHU_DELIVERY_JSON_V1", "TEST_CODE_invalid_mode"),
    ] {
        let fixture = CliFixture::new(
            "RejectedBeforeMessage",
            "auth",
            false,
            "feishu_auth_failed_before_message",
            1,
        );
        std::env::set_var(key, value);
        assert!(matches!(
            push_via_magiclaw_cli_receipt_blocking(MessageSendType::Feishu, BODY),
            Err(BlockingCliDeliveryFailure::Rejected { .. })
        ));
        assert_eq!(fixture.call_count(), 0);
    }
}

#[test]
#[serial_test::serial(notify_env)]
fn feishu_json_v1_cli_receive_type_uses_process_config_or_known_prefix() {
    let fixture = CliFixture::new(
        "RejectedBeforeMessage",
        "auth",
        false,
        "feishu_auth_failed_before_message",
        1,
    );
    for (target, expected) in [
        ("oc_TEST_CODE_target", "chat_id"),
        ("ou_TEST_CODE_target", "open_id"),
        ("on_TEST_CODE_target", "union_id"),
        ("TEST_CODE@example.invalid", "email"),
        ("TEST_CODE_unknown", "open_id"),
    ] {
        std::env::set_var("FEISHU_TO", target);
        let _ = push_via_magiclaw_cli_receipt_blocking(MessageSendType::Feishu, BODY);
        assert!(fixture
            .calls()
            .last()
            .unwrap()
            .ends_with(&format!("|1|{expected}")));
    }
    std::env::set_var("FEISHU_TO", TARGET);
    std::env::set_var("FEISHU_RECEIVE_ID_TYPE", "user_id");
    let _ = push_via_magiclaw_cli_receipt_blocking(MessageSendType::Feishu, BODY);
    assert!(fixture.calls().last().unwrap().ends_with("|1|user_id"));
    assert_eq!(fixture.call_count(), 6);
}

#[test]
#[serial_test::serial(notify_env)]
fn feishu_json_v1_cli_signal_after_valid_auth_json_is_uncertain() {
    let fixture = CliFixture::new(
        "RejectedBeforeMessage",
        "auth",
        false,
        "feishu_auth_failed_before_message",
        1,
    );
    let script = std::fs::read_to_string(&fixture.executable)
        .unwrap()
        .replace("exit 1\n", "kill -TERM $$\n");
    std::fs::write(&fixture.executable, script).unwrap();
    assert!(matches!(
        push_via_magiclaw_cli_receipt_blocking(MessageSendType::Feishu, BODY),
        Err(BlockingCliDeliveryFailure::Uncertain { .. })
    ));
    assert_eq!(fixture.call_count(), 1);
}

#[test]
#[serial_test::serial(notify_env)]
fn feishu_json_v1_cli_legacy_unsupported_and_signal_never_resend_or_leak_raw_streams() {
    for (stdout, finish) in [
        (
            "send ok (feishu): message_id=TEST_CODE_LEGACY, platform_msg_id=om_x",
            "exit 0",
        ),
        ("", "printf '%s\\n' 'TEST_CODE_SECRET_STDERR' >&2; exit 2"),
        ("TEST_CODE_SECRET_STDOUT", "kill -TERM $$"),
    ] {
        let fixture = CliFixture::new(
            "RejectedBeforeMessage",
            "auth",
            false,
            "feishu_auth_failed_before_message",
            1,
        );
        fixture.set_output(stdout, finish);
        let result = push_via_magiclaw_cli_receipt_blocking(MessageSendType::Feishu, BODY);
        let Err(BlockingCliDeliveryFailure::Uncertain { evidence, .. }) = result else {
            panic!("untrusted or interrupted CLI must remain uncertain");
        };
        let evidence = String::from_utf8(evidence).unwrap();
        assert!(evidence.contains("stdout_sha256=") && evidence.contains("stderr_sha256="));
        assert!(!evidence.contains("TEST_CODE_SECRET") && !evidence.contains("TEST_CODE_LEGACY"));
        assert_eq!(
            fixture.call_count(),
            1,
            "unsupported protocol cannot fall back to a second send"
        );
    }
}

#[test]
#[serial_test::serial(notify_env)]
fn feishu_json_v1_cli_exact_body_hash_and_trimmed_target_binding() {
    let fixture = CliFixture::new(
        "RejectedBeforeMessage",
        "auth",
        false,
        "feishu_auth_failed_before_message",
        1,
    );
    std::env::set_var("FEISHU_TO", format!(" {TARGET} \n"));
    assert!(matches!(
        push_via_magiclaw_cli_receipt_blocking(MessageSendType::Feishu, BODY),
        Err(BlockingCliDeliveryFailure::Rejected { .. })
    ));
    assert!(
        matches!(push_via_magiclaw_cli_receipt_blocking(MessageSendType::Feishu, &format!("{BODY}\n")),
        Err(BlockingCliDeliveryFailure::Uncertain { reason_code, .. }) if reason_code == "magiclaw_feishu_delivery_json_v1_binding_mismatch")
    );
    assert_eq!(fixture.call_count(), 2);
}

#[test]
#[serial_test::serial(notify_env)]
fn feishu_json_v1_wechat_and_disabled_feishu_keep_legacy_receipt_path() {
    let fixture = CliFixture::new("Accepted", "message", true, "accepted", 0);
    std::env::remove_var("MAGICLAW_FEISHU_EXPECTED_ACCOUNT_ID");
    fixture.set_output(
        "send ok: message_id=TEST_CODE_LOCAL, platform_msg_id=TEST_CODE_REMOTE",
        "exit 0",
    );
    assert!(push_via_magiclaw_cli_receipt_blocking(MessageSendType::Wechat, BODY).is_ok());
    std::env::set_var("MAGICLAW_FEISHU_DELIVERY_JSON_V1", "0");
    fixture.set_output(
        "send ok (feishu): message_id=TEST_CODE_LOCAL, platform_msg_id=TEST_CODE_REMOTE",
        "exit 0",
    );
    assert!(push_via_magiclaw_cli_receipt_blocking(MessageSendType::Feishu, BODY).is_ok());
    for call in fixture.calls() {
        assert!(
            call.ends_with("|0|missing"),
            "legacy paths must not get protocol flags"
        );
    }
}
