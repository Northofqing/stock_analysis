//! Paired, opt-in CLI evidence for one newly spawned Feishu invocation.
//! The reviewed producer is the trust boundary; a nonce binds its private stdout
//! pipe to this invocation, rather than authenticating an arbitrary executable.
//! This contract must never classify retained historical delivery records.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::Read;

const SCHEMA: &str = "magiclaw.feishu_delivery.v1";
const MAX_RESULT_BYTES: usize = 4096;

pub(super) struct Invocation {
    pub(super) invocation_id: String,
    receive_id_type: String,
    account_id: String,
    app_id_sha256: String,
    target_sha256: String,
    content_sha256: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum DeliveryOutcome {
    Accepted {
        message_id: String,
        platform_message_id: String,
    },
    RejectedBeforeMessage,
    Uncertain {
        reason_code: &'static str,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeliveryResult {
    schema: String,
    invocation_id: String,
    channel: String,
    account_id: String,
    app_id_sha256: String,
    receive_id_type: String,
    target_sha256: String,
    content_sha256: String,
    kind: String,
    phase: String,
    message_request_started: bool,
    reason_code: String,
    // Absent is permitted; explicit null is not part of the v1 wire contract.
    #[serde(default, deserialize_with = "present_receipt")]
    receipt: Option<Receipt>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    message_id: String,
    platform_message_id: String,
}

fn present_receipt<'de, D>(deserializer: D) -> Result<Option<Receipt>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Receipt::deserialize(deserializer).map(Some)
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn uuid_v4(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
        && value.as_bytes()[14] == b'4'
        && matches!(value.as_bytes()[19], b'8' | b'9' | b'a' | b'b')
}

fn remote_id(value: &str) -> bool {
    value.strip_prefix("om_").is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix.len() <= 128
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    })
}

fn fresh_invocation_id() -> Result<String, &'static str> {
    let mut bytes = [0_u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|_| "magiclaw_feishu_invocation_id_unavailable")?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let encoded = hex::encode(bytes);
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &encoded[..8],
        &encoded[8..12],
        &encoded[12..16],
        &encoded[16..20],
        &encoded[20..]
    ))
}

fn receive_id_type(target: &str) -> Result<String, &'static str> {
    let configured = match std::env::var("FEISHU_RECEIVE_ID_TYPE") {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => String::new(),
        Err(_) => return Err("magiclaw_feishu_receive_id_type_invalid"),
    };
    let configured = configured.trim();
    if !configured.is_empty() {
        if !["open_id", "user_id", "chat_id", "union_id", "email"].contains(&configured) {
            return Err("magiclaw_feishu_receive_id_type_invalid");
        }
        return Ok(configured.to_owned());
    }
    Ok(if target.starts_with("oc_") {
        "chat_id"
    } else if target.starts_with("ou_") {
        "open_id"
    } else if target.starts_with("on_") {
        "union_id"
    } else if target.contains('@') {
        "email"
    } else {
        "open_id"
    }
    .to_owned())
}

impl Invocation {
    /// Reads only the inherited process environment, never a dotenv file.
    /// Missing paired identity or invalid configuration rejects before spawn.
    pub(super) fn prepare(target: &str, content: &str) -> Result<Option<Self>, &'static str> {
        match std::env::var("MAGICLAW_FEISHU_DELIVERY_JSON_V1") {
            Err(std::env::VarError::NotPresent) => return Ok(None),
            Ok(value) if value.trim().is_empty() || value.trim() == "0" => return Ok(None),
            Ok(value) if value.trim() == "1" => {}
            _ => return Err("magiclaw_feishu_delivery_json_v1_config_invalid"),
        }
        let app_id_sha256 = std::env::var("MAGICLAW_FEISHU_EXPECTED_APP_ID_SHA256")
            .map_err(|_| "magiclaw_feishu_expected_identity_missing")?;
        let account_id = std::env::var("MAGICLAW_FEISHU_EXPECTED_ACCOUNT_ID")
            .map_err(|_| "magiclaw_feishu_expected_identity_missing")?;
        let app_id_sha256 = app_id_sha256.trim();
        let account_id = account_id.trim();
        if !lower_hex(app_id_sha256, 64)
            || account_id.is_empty()
            || account_id.len() > 256
            || account_id.chars().any(char::is_control)
        {
            return Err("magiclaw_feishu_expected_identity_invalid");
        }
        let target = target.trim();
        let receive_id_type = receive_id_type(target)?;
        Ok(Some(Self {
            invocation_id: fresh_invocation_id()?,
            receive_id_type,
            account_id: account_id.to_owned(),
            app_id_sha256: app_id_sha256.to_owned(),
            target_sha256: sha256(target.as_bytes()),
            content_sha256: sha256(content.as_bytes()),
        }))
    }

    pub(super) fn append_args(&self, command: &mut std::process::Command) {
        command
            .arg("--delivery-result-json-v1")
            .arg("--invocation-id")
            .arg(&self.invocation_id)
            .arg("--receive-id-type")
            .arg(&self.receive_id_type);
    }

    pub(super) fn classify(
        &self,
        stdout: &[u8],
        exit_code: Option<i32>,
    ) -> Result<DeliveryOutcome, &'static str> {
        if stdout.is_empty() || stdout.len() > MAX_RESULT_BYTES {
            return Err("magiclaw_feishu_delivery_json_v1_invalid");
        }
        // Deserialize directly to a strict struct: Value would lose duplicate
        // keys. from_slice also rejects a second object or trailing log text.
        let result: DeliveryResult = serde_json::from_slice(stdout)
            .map_err(|_| "magiclaw_feishu_delivery_json_v1_invalid")?;
        if result.schema != SCHEMA
            || result.channel != "feishu"
            || result.invocation_id != self.invocation_id
            || result.account_id != self.account_id
            || result.app_id_sha256 != self.app_id_sha256
            || result.receive_id_type != self.receive_id_type
            || result.target_sha256 != self.target_sha256
            || result.content_sha256 != self.content_sha256
        {
            return Err("magiclaw_feishu_delivery_json_v1_binding_mismatch");
        }
        match (
            result.kind.as_str(),
            result.phase.as_str(),
            result.message_request_started,
            result.reason_code.as_str(),
            exit_code,
            result.receipt,
        ) {
            ("Accepted", "message", true, "accepted", Some(0), Some(receipt))
                if uuid_v4(&receipt.message_id) && remote_id(&receipt.platform_message_id) =>
            {
                Ok(DeliveryOutcome::Accepted {
                    message_id: receipt.message_id,
                    platform_message_id: receipt.platform_message_id,
                })
            }
            (
                "RejectedBeforeMessage",
                "auth",
                false,
                "feishu_auth_failed_before_message",
                Some(1),
                None,
            ) => Ok(DeliveryOutcome::RejectedBeforeMessage),
            ("Uncertain", "message", true, "feishu_message_result_unconfirmed", Some(1), None) => {
                Ok(DeliveryOutcome::Uncertain {
                    reason_code: "feishu_message_result_unconfirmed",
                })
            }
            ("Uncertain", "preflight", false, "feishu_preflight_unconfirmed", Some(1), None) => {
                Ok(DeliveryOutcome::Uncertain {
                    reason_code: "feishu_preflight_unconfirmed",
                })
            }
            _ => Err("magiclaw_feishu_delivery_json_v1_invalid"),
        }
    }

    /// Unvalidated CLI streams may contain credentials or message text. Retain
    /// only invocation binding and stream fingerprints for investigation.
    pub(super) fn invalid_output_evidence(&self, output: &std::process::Output) -> Vec<u8> {
        format!(
            "invocation_id={} exit={} stdout_bytes={} stdout_sha256={} stderr_bytes={} stderr_sha256={}",
            self.invocation_id, output.status, output.stdout.len(), sha256(&output.stdout),
            output.stderr.len(), sha256(&output.stderr)
        ).into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn invocation() -> Invocation {
        Invocation {
            invocation_id: "01234567-89ab-4cde-8f01-23456789abcd".to_owned(),
            receive_id_type: "chat_id".to_owned(),
            account_id: "TEST_CODE_account".to_owned(),
            app_id_sha256: "a".repeat(64),
            target_sha256: "b".repeat(64),
            content_sha256: "c".repeat(64),
        }
    }

    fn wire(kind: &str, phase: &str, started: bool, reason: &str) -> Value {
        json!({
            "schema": SCHEMA, "invocation_id": "01234567-89ab-4cde-8f01-23456789abcd",
            "channel": "feishu", "account_id": "TEST_CODE_account",
            "app_id_sha256": "a".repeat(64), "receive_id_type": "chat_id",
            "target_sha256": "b".repeat(64), "content_sha256": "c".repeat(64),
            "kind": kind, "phase": phase, "message_request_started": started,
            "reason_code": reason,
        })
    }

    fn rejected() -> Value {
        wire(
            "RejectedBeforeMessage",
            "auth",
            false,
            "feishu_auth_failed_before_message",
        )
    }

    fn accepted() -> Value {
        let mut value = wire("Accepted", "message", true, "accepted");
        value["receipt"] = json!({
            "message_id": "fedcba98-7654-4321-8abc-def012345678",
            "platform_message_id": "om_x100b6343eab8f0a4c4556e4b1737efa",
        });
        value
    }

    fn classify(value: &Value, exit: Option<i32>) -> Result<DeliveryOutcome, &'static str> {
        invocation().classify(&serde_json::to_vec(value).unwrap(), exit)
    }

    #[test]
    fn feishu_json_v1_accepts_only_closed_contract_variants() {
        assert!(matches!(
            classify(&accepted(), Some(0)),
            Ok(DeliveryOutcome::Accepted { .. })
        ));
        assert_eq!(
            classify(&rejected(), Some(1)),
            Ok(DeliveryOutcome::RejectedBeforeMessage)
        );
        for (phase, started, reason) in [
            ("message", true, "feishu_message_result_unconfirmed"),
            ("preflight", false, "feishu_preflight_unconfirmed"),
        ] {
            assert_eq!(
                classify(&wire("Uncertain", phase, started, reason), Some(1)),
                Ok(DeliveryOutcome::Uncertain {
                    reason_code: reason
                })
            );
        }
    }

    #[test]
    fn feishu_json_v1_rejects_every_binding_mismatch_and_replay() {
        for field in [
            "schema",
            "invocation_id",
            "channel",
            "account_id",
            "app_id_sha256",
            "receive_id_type",
            "target_sha256",
            "content_sha256",
        ] {
            let mut value = rejected();
            value[field] = json!("TEST_CODE_OTHER_INVOCATION");
            assert!(classify(&value, Some(1)).is_err(), "binding field {field}");
        }
        let mut second = invocation();
        second.invocation_id = "01234567-89ab-4cde-9f01-23456789abcd".to_owned();
        assert!(second
            .classify(&serde_json::to_vec(&rejected()).unwrap(), Some(1))
            .is_err());
    }

    #[test]
    fn feishu_json_v1_never_infers_rejection_from_inconsistent_phase_or_exit() {
        for (field, replacement) in [
            ("kind", json!("Rejected")),
            ("phase", json!("message")),
            ("message_request_started", json!(true)),
            ("reason_code", json!("auth_timeout")),
            ("receipt", json!(null)),
            ("receipt", accepted()["receipt"].clone()),
        ] {
            let mut value = rejected();
            value[field] = replacement;
            assert!(classify(&value, Some(1)).is_err(), "inconsistent {field}");
        }
        for exit in [None, Some(0), Some(2), Some(137)] {
            assert!(classify(&rejected(), exit).is_err());
        }
        assert!(classify(&accepted(), Some(1)).is_err());
        let mut value = accepted();
        value.as_object_mut().unwrap().remove("receipt");
        assert!(classify(&value, Some(0)).is_err());
        for value in [
            wire(
                "Uncertain",
                "auth",
                false,
                "feishu_auth_failed_before_message",
            ),
            wire(
                "Uncertain",
                "message",
                false,
                "feishu_message_result_unconfirmed",
            ),
            wire(
                "Uncertain",
                "preflight",
                true,
                "feishu_preflight_unconfirmed",
            ),
        ] {
            assert!(classify(&value, Some(1)).is_err());
        }
    }

    #[test]
    fn feishu_json_v1_rejects_duplicate_extra_missing_and_polluted_json() {
        let valid = serde_json::to_string(&rejected()).unwrap();
        for (field, value) in rejected().as_object().unwrap() {
            let duplicate = format!("{{\"{field}\":{value},{}", &valid[1..]);
            assert!(
                invocation()
                    .classify(duplicate.as_bytes(), Some(1))
                    .is_err(),
                "duplicate {field}"
            );
            let mut missing = rejected();
            missing.as_object_mut().unwrap().remove(field);
            assert!(classify(&missing, Some(1)).is_err(), "missing {field}");
        }
        let mut extra = rejected();
        extra["TEST_CODE_extra"] = json!(false);
        assert!(classify(&extra, Some(1)).is_err());
        let mut extra_receipt = accepted();
        extra_receipt["receipt"]["TEST_CODE_extra"] = json!(false);
        assert!(classify(&extra_receipt, Some(0)).is_err());
        let receipt_duplicate = serde_json::to_string(&accepted()).unwrap().replace(
            "\"receipt\":{",
            "\"receipt\":{\"message_id\":\"fedcba98-7654-4321-8abc-def012345678\",",
        );
        assert!(invocation()
            .classify(receipt_duplicate.as_bytes(), Some(0))
            .is_err());
        for bytes in [
            Vec::new(),
            b"send ok (feishu): message_id=legacy".to_vec(),
            format!("{valid}\n{valid}").into_bytes(),
            format!("log\n{valid}").into_bytes(),
            format!("{valid}\nlog").into_bytes(),
            b"[]".to_vec(),
            b"null".to_vec(),
            b"{\"schema\":".to_vec(),
            vec![0xff],
            vec![b' '; MAX_RESULT_BYTES + 1],
        ] {
            assert!(invocation().classify(&bytes, Some(1)).is_err());
        }
    }

    #[test]
    fn feishu_json_v1_validates_receipt_ids_without_hex_or_fixed_length_guess() {
        for id in [
            "om_x",
            "om_x100b00000000000000000000000000",
            &format!("om_{}", "a".repeat(128)),
        ] {
            let mut value = accepted();
            value["receipt"]["platform_message_id"] = json!(id);
            assert!(matches!(
                classify(&value, Some(0)),
                Ok(DeliveryOutcome::Accepted { .. })
            ));
        }
        for id in [
            "",
            "<missing>",
            "om_",
            "om_X",
            "om_a b",
            "om_a\n",
            &format!("om_{}", "a".repeat(129)),
        ] {
            let mut value = accepted();
            value["receipt"]["platform_message_id"] = json!(id);
            assert!(classify(&value, Some(0)).is_err());
        }
        for id in [
            "",
            "<missing>",
            "fedcba98-7654-1321-8abc-def012345678",
            "fedcba98-7654-4321-0abc-def012345678",
            "FEDCBA98-7654-4321-8abc-def012345678",
        ] {
            let mut value = accepted();
            value["receipt"]["message_id"] = json!(id);
            assert!(classify(&value, Some(0)).is_err());
        }
    }

    #[test]
    fn feishu_json_v1_generates_fresh_canonical_rfc4122_uuid_v4() {
        let first = fresh_invocation_id().unwrap();
        let second = fresh_invocation_id().unwrap();
        assert!(uuid_v4(&first) && uuid_v4(&second));
        assert_ne!(first, second);
    }
}
