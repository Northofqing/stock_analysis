//! BR-196 opt-in, non-production-only Feishu acceptance transport.
//!
//! This module intentionally does not reuse the generic notification target
//! resolver.  The generic resolver permits aliases/defaults, while BR-196
//! requires an exact, release-pinned tenant/app/conversation identity and a
//! short-lived, one-shot batch permit.

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::collections::HashSet;
use std::io::Write;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;

const ALLOWLIST_VERSION: &str = "BR196_FEISHU_TARGETS_V1";
const PINNED_ALLOWLIST_SHA256: &str =
    "e351650d70e0716eae3895a8092908c8b6facaea1a9d405da514cbeadacd16ba";
const ALLOWLIST_BYTES: &[u8] =
    include_bytes!("../../../config/br196_non_production_feishu_targets.toml");
const PERMIT_LIFETIME_SECONDS: i64 = 300;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetHashGroup {
    target_sha256: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetAllowlistManifest {
    version: String,
    non_production_acceptance: TargetHashGroup,
    production_deny: TargetHashGroup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedFeishuTargetIdentity {
    tenant_id: String,
    app_id: String,
    conversation_id: String,
}

impl ResolvedFeishuTargetIdentity {
    fn from_release_pinned_configuration() -> Result<Self, String> {
        Ok(Self {
            tenant_id: exact_env("BR196_FEISHU_TENANT_ID")?,
            app_id: exact_env("BR196_FEISHU_APP_ID")?,
            conversation_id: exact_env("BR196_FEISHU_CONVERSATION_ID")?,
        })
    }

    fn canonical_bytes(&self) -> Vec<u8> {
        format!(
            "tenant_id={}\napp_id={}\nconversation_id={}\n",
            self.tenant_id, self.app_id, self.conversation_id
        )
        .into_bytes()
    }

    fn identity_sha256(&self) -> String {
        sha256_domain(
            "stock_analysis.br196.feishu_target_identity.v1",
            &self.canonical_bytes(),
        )
    }
}

fn exact_env(name: &str) -> Result<String, String> {
    let value =
        std::env::var(name).map_err(|_| format!("BR-196 target identity missing field {name}"))?;
    exact_value(name, Some(&value))
}

fn exact_value(name: &str, value: Option<&String>) -> Result<String, String> {
    let value = value
        .ok_or_else(|| format!("BR-196 target identity missing field {name}"))?
        .to_owned();
    if value.is_empty()
        || value.trim() != value
        || value.chars().any(char::is_whitespace)
        || value.contains('/')
        || value.contains('\\')
    {
        return Err(format!("BR-196 target identity malformed field {name}"));
    }
    Ok(value)
}

fn read_dotenv(path: &Path) -> Result<std::collections::HashMap<String, String>, String> {
    let mut values = std::collections::HashMap::new();
    let iterator = dotenvy::from_path_iter(path)
        .map_err(|error| format!("BR-196 pinned target config open failed: {error}"))?;
    for item in iterator {
        let (key, value) =
            item.map_err(|error| format!("BR-196 pinned target config parse failed: {error}"))?;
        values.insert(key, value);
    }
    Ok(values)
}

fn validate_magiclaw_target_binding(
    magiclaw_home: &Path,
    target: &ResolvedFeishuTargetIdentity,
) -> Result<(), String> {
    let magiclaw_env = read_dotenv(&magiclaw_home.join(".env"))?;
    let configured_tenant =
        exact_value("FEISHU_ACCOUNT_ID", magiclaw_env.get("FEISHU_ACCOUNT_ID"))?;
    let configured_app = exact_value("FEISHU_APP_ID", magiclaw_env.get("FEISHU_APP_ID"))?;
    if configured_tenant != target.tenant_id || configured_app != target.app_id {
        return Err("BR-196 MagicLaw target binding mismatch".to_string());
    }
    Ok(())
}

#[derive(Debug)]
struct NonProductionFeishuTargetAuthorityV1 {
    target: ResolvedFeishuTargetIdentity,
    target_identity_sha256: String,
    allowlist_version: String,
    allowlist_sha256: String,
}

#[derive(Debug)]
pub(super) struct BR196LiveFeishuAcceptancePermit {
    test_code: String,
    authority: NonProductionFeishuTargetAuthorityV1,
    issued_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    _not_send_or_sync: PhantomData<Rc<()>>,
}

#[derive(Debug)]
struct BatchCheckGuard<'permit> {
    root: &'permit BR196LiveFeishuAcceptancePermit,
    checked_at: DateTime<Utc>,
}

#[derive(Debug)]
struct BR196BatchSendPermit<'check> {
    check: &'check BatchCheckGuard<'check>,
    consumed: Cell<bool>,
    _not_send_or_sync: PhantomData<Rc<()>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BR196TransportBatch<'a> {
    pub ordinal: usize,
    pub template_ids: &'a [&'static str],
    pub text: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BR196LiveDeliveryReport {
    pub target_identity_sha256: String,
    pub target_allowlist_sha256: String,
    pub external_process_attempted: usize,
    pub batches_attempted: usize,
    pub batches_pushed: usize,
    pub families_pushed: usize,
    pub receipt_audit_appended: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliDeliveryReceipt {
    message_id: String,
    platform_msg_id: String,
}

trait ProcessClock {
    fn now(&self) -> Result<DateTime<Utc>, String>;
}

struct SystemProcessClock;

impl ProcessClock for SystemProcessClock {
    fn now(&self) -> Result<DateTime<Utc>, String> {
        Ok(Utc::now())
    }
}

struct ReceiptAuditWriter {
    root: PathBuf,
}

impl ReceiptAuditWriter {
    fn production(test_code: &str) -> Result<Self, String> {
        validate_test_code(test_code)?;
        Ok(Self {
            root: Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("data/test")
                .join(test_code)
                .join("template_delivery_audit"),
        })
    }

    fn append(
        &self,
        test_code: &str,
        batch: &BR196TransportBatch<'_>,
        receipt: &CliDeliveryReceipt,
        observed_at: DateTime<Utc>,
    ) -> Result<String, String> {
        validate_test_code(test_code)?;
        let receipt_preimage = serde_json::to_vec(&serde_json::json!({
            "schema": "stock_analysis.br196.test_template_receipt.v2",
            "test_code": test_code,
            "batch_ordinal": batch.ordinal,
            "template_ids": batch.template_ids,
            "message_id": receipt.message_id,
            "platform_msg_id": receipt.platform_msg_id,
        }))
        .map_err(|error| format!("BR-196 receipt serialization failed: {error}"))?;
        let receipt_sha256 = sha256_domain(
            "stock_analysis.br196.test_template_receipt.v2",
            &receipt_preimage,
        );
        let record = serde_json::to_vec(&serde_json::json!({
            "schema": "stock_analysis.br196.test_template_delivery_audit.v2",
            "test_code": test_code,
            "batch_ordinal": batch.ordinal,
            "template_ids": batch.template_ids,
            "transport": "feishu-cli",
            "receipt_sha256": receipt_sha256,
            "observed_at": observed_at.to_rfc3339_opts(SecondsFormat::Nanos, true),
        }))
        .map_err(|error| format!("BR-196 audit serialization failed: {error}"))?;

        std::fs::create_dir_all(&self.root)
            .map_err(|error| format!("BR-196 audit directory create failed: {error}"))?;
        let path = self
            .root
            .join(format!("{}.jsonl", observed_at.format("%Y-%m-%d")));
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|error| format!("BR-196 audit open failed: {error}"))?;
        file.write_all(&record)
            .and_then(|()| file.write_all(b"\n"))
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("BR-196 audit append failed: {error}"))?;
        Ok(receipt_sha256)
    }
}

pub(super) fn live_acceptance_opted_in() -> bool {
    std::env::var("BR196_LIVE_FEISHU_ACCEPTANCE")
        .ok()
        .as_deref()
        == Some("1")
}

pub(super) fn deliver_live_batches(
    test_code: &str,
    batches: &[BR196TransportBatch<'_>],
    frozen_news: &crate::br196_test_delivery::NewsFlashProcessCapabilitySnapshot,
) -> Result<BR196LiveDeliveryReport, String> {
    if !live_acceptance_opted_in() {
        return Err("live_acceptance_not_opted_in".to_string());
    }
    if stock_analysis::risk::env_guard::current_env()
        != stock_analysis::risk::env_guard::TradingEnv::Test
    {
        return Err("BR-196 live acceptance requires Test environment".to_string());
    }
    validate_test_code(test_code)?;
    if batches.is_empty() {
        return Err("BR-196 live acceptance has no batches".to_string());
    }

    let authority = resolve_live_target_authority()?;
    let clock = SystemProcessClock;
    let issued_at = clock.now()?;
    let permit = BR196LiveFeishuAcceptancePermit {
        test_code: test_code.to_owned(),
        authority,
        issued_at,
        expires_at: issued_at + Duration::seconds(PERMIT_LIFETIME_SECONDS),
        _not_send_or_sync: PhantomData,
    };
    let audit = ReceiptAuditWriter::production(test_code)?;
    deliver_batches_with(
        &permit,
        batches,
        frozen_news,
        &clock,
        &audit,
        &resolve_live_target_authority,
    )
}

fn deliver_batches_with(
    permit: &BR196LiveFeishuAcceptancePermit,
    batches: &[BR196TransportBatch<'_>],
    frozen_news: &crate::br196_test_delivery::NewsFlashProcessCapabilitySnapshot,
    clock: &dyn ProcessClock,
    audit: &ReceiptAuditWriter,
    target_resolver: &dyn Fn() -> Result<NonProductionFeishuTargetAuthorityV1, String>,
) -> Result<BR196LiveDeliveryReport, String> {
    let mut report = BR196LiveDeliveryReport {
        target_identity_sha256: permit.authority.target_identity_sha256.clone(),
        target_allowlist_sha256: permit.authority.allowlist_sha256.clone(),
        external_process_attempted: 0,
        batches_attempted: 0,
        batches_pushed: 0,
        families_pushed: 0,
        receipt_audit_appended: 0,
    };
    let mut seen_receipts = HashSet::new();

    for batch in batches {
        frozen_news.require_unchanged(
            crate::br196_test_delivery::current_news_process_capability().as_ref(),
        )?;
        let current_authority = target_resolver()?;
        if current_authority.target_identity_sha256 != permit.authority.target_identity_sha256
            || current_authority.allowlist_sha256 != permit.authority.allowlist_sha256
            || current_authority.allowlist_version != permit.authority.allowlist_version
        {
            return Err("BR-196 target or allowlist changed during invocation".to_string());
        }
        let checked_at = clock.now()?;
        permit.validate_time(checked_at)?;
        let guard = BatchCheckGuard {
            root: permit,
            checked_at,
        };
        let batch_permit = guard.mint_batch_permit();
        report.batches_attempted += 1;
        let attempt = spawn_br196_batch(
            batch_permit,
            batch,
            &permit.authority.target.conversation_id,
            clock,
        );
        match attempt {
            Ok(receipt) => {
                report.external_process_attempted += 1;
                if !seen_receipts
                    .insert((receipt.message_id.clone(), receipt.platform_msg_id.clone()))
                {
                    return Err("BR-196 duplicate MagicLaw delivery receipt".to_string());
                }
                audit.append(&permit.test_code, batch, &receipt, clock.now()?)?;
                report.receipt_audit_appended += 1;
                report.batches_pushed += 1;
                report.families_pushed += batch.template_ids.len();
            }
            Err(failure) => {
                report.external_process_attempted += usize::from(failure.process_attempted);
                return Err(failure.reason);
            }
        }
    }
    Ok(report)
}

impl BR196LiveFeishuAcceptancePermit {
    fn validate_time(&self, now: DateTime<Utc>) -> Result<(), String> {
        if now < self.issued_at {
            return Err("BR-196 permit clock rollback".to_string());
        }
        if now >= self.expires_at {
            return Err("BR-196 permit expired".to_string());
        }
        Ok(())
    }
}

impl<'permit> BatchCheckGuard<'permit> {
    fn mint_batch_permit<'check>(&'check self) -> BR196BatchSendPermit<'check> {
        BR196BatchSendPermit {
            check: self,
            consumed: Cell::new(false),
            _not_send_or_sync: PhantomData,
        }
    }
}

#[derive(Debug)]
struct SpawnFailure {
    reason: String,
    process_attempted: bool,
}

fn spawn_br196_batch(
    permit: BR196BatchSendPermit<'_>,
    batch: &BR196TransportBatch<'_>,
    conversation_id: &str,
    clock: &dyn ProcessClock,
) -> Result<CliDeliveryReceipt, SpawnFailure> {
    if permit.consumed.replace(true) {
        return Err(SpawnFailure {
            reason: "BR-196 batch permit already consumed".to_string(),
            process_attempted: false,
        });
    }
    validate_batch(batch).map_err(|reason| SpawnFailure {
        reason,
        process_attempted: false,
    })?;
    if permit.check.checked_at < permit.check.root.issued_at {
        return Err(SpawnFailure {
            reason: "BR-196 batch guard predates root permit".to_string(),
            process_attempted: false,
        });
    }
    let configuration_checked_at = clock.now().map_err(|reason| SpawnFailure {
        reason,
        process_attempted: false,
    })?;
    permit
        .check
        .root
        .validate_time(configuration_checked_at)
        .map_err(|reason| SpawnFailure {
            reason,
            process_attempted: false,
        })?;
    let magiclaw_bin = crate::notify::resolve_magiclaw_bin();
    let magiclaw_home =
        crate::notify::resolve_magiclaw_home(&magiclaw_bin).ok_or_else(|| SpawnFailure {
            reason: "BR-196 MagicLaw configuration root unavailable".to_string(),
            process_attempted: false,
        })?;
    validate_magiclaw_target_binding(&magiclaw_home, &permit.check.root.authority.target).map_err(
        |reason| SpawnFailure {
            reason,
            process_attempted: false,
        },
    )?;
    let db_path = std::env::var("MAGICLAW_DB_PATH").ok();
    let receive_id_type = std::env::var("FEISHU_RECEIVE_ID_TYPE").ok();

    // BR-196 authority use-site: this is the second fresh clock read.  After
    // validation, only infallible Command assembly occurs before spawn.
    let spawn_checked_at = clock.now().map_err(|reason| SpawnFailure {
        reason,
        process_attempted: false,
    })?;
    permit
        .check
        .root
        .validate_time(spawn_checked_at)
        .map_err(|reason| SpawnFailure {
            reason,
            process_attempted: false,
        })?;
    let mut command = Command::new(&magiclaw_bin);
    command
        .arg("send")
        .arg("--channel")
        .arg("feishu")
        .arg("--message")
        .arg(batch.text)
        .arg("--to")
        .arg(conversation_id)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.current_dir(magiclaw_home);
    if let Some(db_path) = db_path.filter(|value| !value.trim().is_empty()) {
        command.env("MAGICLAW_DB_PATH", db_path.trim());
    }
    if let Some(receive_id_type) = receive_id_type.filter(|value| !value.trim().is_empty()) {
        command.arg("--receive-id-type").arg(receive_id_type.trim());
    }
    let child = command.spawn().map_err(|error| SpawnFailure {
        reason: format!("BR-196 MagicLaw spawn failed: {error}"),
        process_attempted: false,
    })?;
    let output = child.wait_with_output().map_err(|error| SpawnFailure {
        reason: format!("BR-196 MagicLaw wait failed: {error}"),
        process_attempted: true,
    })?;
    if !output.status.success() {
        return Err(SpawnFailure {
            reason: format!("BR-196 MagicLaw nonzero exit={}", output.status),
            process_attempted: true,
        });
    }
    parse_receipt(&String::from_utf8_lossy(&output.stdout)).map_err(|reason| SpawnFailure {
        reason,
        process_attempted: true,
    })
}

fn validate_batch(batch: &BR196TransportBatch<'_>) -> Result<(), String> {
    if batch.ordinal == 0
        || batch.template_ids.is_empty()
        || batch.template_ids.iter().any(|id| id.trim().is_empty())
        || batch.text.trim().is_empty()
        || !batch.text.contains("[TEST_CODE 模板验收]")
    {
        return Err("BR-196 batch input invalid".to_string());
    }
    Ok(())
}

fn resolve_live_target_authority() -> Result<NonProductionFeishuTargetAuthorityV1, String> {
    let manifest = load_pinned_allowlist()?;
    let target = ResolvedFeishuTargetIdentity::from_release_pinned_configuration()?;
    authorize_target(target, &manifest, PINNED_ALLOWLIST_SHA256)
}

fn authorize_target(
    target: ResolvedFeishuTargetIdentity,
    manifest: &TargetAllowlistManifest,
    allowlist_sha256: &str,
) -> Result<NonProductionFeishuTargetAuthorityV1, String> {
    let target_identity_sha256 = target.identity_sha256();
    let allowed = manifest
        .non_production_acceptance
        .target_sha256
        .iter()
        .any(|hash| hash == &target_identity_sha256);
    let denied = manifest
        .production_deny
        .target_sha256
        .iter()
        .any(|hash| hash == &target_identity_sha256);
    match (allowed, denied) {
        (true, false) => Ok(NonProductionFeishuTargetAuthorityV1 {
            target,
            target_identity_sha256,
            allowlist_version: manifest.version.clone(),
            allowlist_sha256: allowlist_sha256.to_owned(),
        }),
        (false, true) => Err("production_feishu_target_rejected".to_string()),
        (true, true) => Err("conflicting_feishu_target_classification".to_string()),
        (false, false) => Err("unknown_feishu_target_rejected".to_string()),
    }
}

fn load_pinned_allowlist() -> Result<TargetAllowlistManifest, String> {
    let actual = sha256_bytes(ALLOWLIST_BYTES);
    if actual != PINNED_ALLOWLIST_SHA256 {
        return Err("BR-196 target allowlist release hash mismatch".to_string());
    }
    let manifest: TargetAllowlistManifest = toml::from_str(
        std::str::from_utf8(ALLOWLIST_BYTES)
            .map_err(|error| format!("BR-196 target allowlist UTF-8 failed: {error}"))?,
    )
    .map_err(|error| format!("BR-196 target allowlist parse failed: {error}"))?;
    if manifest.version != ALLOWLIST_VERSION {
        return Err("BR-196 target allowlist version mismatch".to_string());
    }
    let mut all = HashSet::new();
    for hash in manifest
        .non_production_acceptance
        .target_sha256
        .iter()
        .chain(manifest.production_deny.target_sha256.iter())
    {
        validate_hash(hash)?;
        if !all.insert(hash) {
            return Err("BR-196 duplicate/conflicting target hash".to_string());
        }
    }
    Ok(manifest)
}

fn parse_receipt(stdout: &str) -> Result<CliDeliveryReceipt, String> {
    const PREFIX: &str = "send ok (feishu):";
    let line = stdout
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with(PREFIX))
        .ok_or_else(|| "BR-196 missing channel-specific success receipt".to_string())?;
    let mut message_id = None;
    let mut platform_msg_id = None;
    for field in line[PREFIX.len()..].split(',').map(str::trim) {
        let Some((key, value)) = field.split_once('=') else {
            continue;
        };
        match key.trim() {
            "message_id" => message_id = Some(valid_receipt_id("message_id", value)?),
            "platform_msg_id" => {
                platform_msg_id = Some(valid_receipt_id("platform_msg_id", value)?)
            }
            _ => {}
        }
    }
    Ok(CliDeliveryReceipt {
        message_id: message_id.ok_or_else(|| "BR-196 missing message_id".to_string())?,
        platform_msg_id: platform_msg_id
            .ok_or_else(|| "BR-196 missing platform_msg_id".to_string())?,
    })
}

fn valid_receipt_id(name: &str, value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.starts_with('<') && value.ends_with('>') {
        return Err(format!("BR-196 invalid or placeholder {name}"));
    }
    Ok(value.to_owned())
}

fn validate_test_code(test_code: &str) -> Result<(), String> {
    if !test_code.starts_with("TEST_CODE_")
        || test_code.trim() != test_code
        || test_code.contains('/')
        || test_code.contains('\\')
        || test_code.chars().any(char::is_whitespace)
    {
        return Err("BR-196 invalid invocation TEST_CODE namespace".to_string());
    }
    Ok(())
}

fn validate_hash(hash: &str) -> Result<(), String> {
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("BR-196 target manifest contains invalid SHA-256".to_string());
    }
    Ok(())
}

fn sha256_domain(domain: &str, payload: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update(payload);
    format!("{:x}", hasher.finalize())
}

fn sha256_bytes(payload: &[u8]) -> String {
    format!("{:x}", Sha256::digest(payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;

    struct FakeClock {
        values: Mutex<VecDeque<Result<DateTime<Utc>, String>>>,
    }

    impl FakeClock {
        fn new(values: Vec<Result<DateTime<Utc>, String>>) -> Self {
            Self {
                values: Mutex::new(values.into()),
            }
        }
    }

    impl ProcessClock for FakeClock {
        fn now(&self) -> Result<DateTime<Utc>, String> {
            self.values
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Err("fake clock exhausted".to_string()))
        }
    }

    fn identity() -> ResolvedFeishuTargetIdentity {
        ResolvedFeishuTargetIdentity {
            tenant_id: "TEST_CODE_TENANT".to_string(),
            app_id: "TEST_CODE_APP".to_string(),
            conversation_id: "TEST_CODE_CHAT".to_string(),
        }
    }

    fn allowlist(allow: bool, deny: bool) -> TargetAllowlistManifest {
        let hash = identity().identity_sha256();
        TargetAllowlistManifest {
            version: ALLOWLIST_VERSION.to_string(),
            non_production_acceptance: TargetHashGroup {
                target_sha256: if allow { vec![hash.clone()] } else { vec![] },
            },
            production_deny: TargetHashGroup {
                target_sha256: if deny { vec![hash] } else { vec![] },
            },
        }
    }

    #[test]
    #[serial_test::serial(br196_target_env)]
    fn br196_target_resolution_requires_all_explicit_fields_without_fallback() {
        let _guard = crate::TestEnvGuard::capture(&[
            "BR196_FEISHU_TENANT_ID",
            "BR196_FEISHU_APP_ID",
            "BR196_FEISHU_CONVERSATION_ID",
        ]);
        for key in [
            "BR196_FEISHU_TENANT_ID",
            "BR196_FEISHU_APP_ID",
            "BR196_FEISHU_CONVERSATION_ID",
        ] {
            std::env::remove_var(key);
        }

        assert_eq!(
            ResolvedFeishuTargetIdentity::from_release_pinned_configuration().unwrap_err(),
            "BR-196 target identity missing field BR196_FEISHU_TENANT_ID"
        );
        std::env::set_var("BR196_FEISHU_TENANT_ID", "TEST_CODE_TENANT");
        assert_eq!(
            ResolvedFeishuTargetIdentity::from_release_pinned_configuration().unwrap_err(),
            "BR-196 target identity missing field BR196_FEISHU_APP_ID"
        );
        std::env::set_var("BR196_FEISHU_APP_ID", "TEST_CODE_APP");
        std::env::set_var("BR196_FEISHU_CONVERSATION_ID", "TEST_CODE_CHAT");
        assert_eq!(
            ResolvedFeishuTargetIdentity::from_release_pinned_configuration().unwrap(),
            identity()
        );
    }

    #[test]
    fn br196_magiclaw_configuration_must_match_permitted_target() {
        let root = std::env::temp_dir().join(format!(
            "TEST_CODE_br196_magiclaw_binding_{}_{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join(".env"),
            "FEISHU_ACCOUNT_ID=TEST_CODE_TENANT\nFEISHU_APP_ID=TEST_CODE_APP\n",
        )
        .unwrap();

        assert!(validate_magiclaw_target_binding(&root, &identity()).is_ok());
        let mut mismatched = identity();
        mismatched.app_id = "TEST_CODE_OTHER_APP".to_string();
        assert_eq!(
            validate_magiclaw_target_binding(&root, &mismatched).unwrap_err(),
            "BR-196 MagicLaw target binding mismatch"
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn br196_pinned_target_manifest_is_valid_and_empty_until_reviewed() {
        let manifest = load_pinned_allowlist().unwrap();
        assert_eq!(manifest.version, ALLOWLIST_VERSION);
        assert!(manifest.non_production_acceptance.target_sha256.is_empty());
        assert_eq!(manifest.production_deny.target_sha256.len(), 1);
    }

    #[test]
    fn br196_target_authority_is_exact_allow_deny_or_unknown() {
        assert!(authorize_target(identity(), &allowlist(true, false), "a").is_ok());
        assert_eq!(
            authorize_target(identity(), &allowlist(false, true), "a").unwrap_err(),
            "production_feishu_target_rejected"
        );
        assert_eq!(
            authorize_target(identity(), &allowlist(false, false), "a").unwrap_err(),
            "unknown_feishu_target_rejected"
        );
        assert_eq!(
            authorize_target(identity(), &allowlist(true, true), "a").unwrap_err(),
            "conflicting_feishu_target_classification"
        );
    }

    #[test]
    fn br196_receipt_parser_rejects_missing_and_placeholder_ids() {
        assert!(parse_receipt("send ok (feishu): message_id=<id>, platform_msg_id=x").is_err());
        assert!(parse_receipt("send ok (feishu): message_id=x").is_err());
        assert!(parse_receipt("send ok: message_id=x, platform_msg_id=y").is_err());
        assert_eq!(
            parse_receipt(
                "send ok (feishu): message_id=TEST_CODE_MESSAGE_1, platform_msg_id=TEST_CODE_PLATFORM_1"
            )
            .unwrap()
            .platform_msg_id,
            "TEST_CODE_PLATFORM_1"
        );
    }

    #[test]
    fn br196_expiry_after_mint_prevents_process_construction() {
        let issued_at = Utc::now();
        let authority =
            authorize_target(identity(), &allowlist(true, false), "allow-hash").unwrap();
        let root = BR196LiveFeishuAcceptancePermit {
            test_code: "TEST_CODE_EXPIRY".to_string(),
            authority,
            issued_at,
            expires_at: issued_at + Duration::seconds(1),
            _not_send_or_sync: PhantomData,
        };
        let guard = BatchCheckGuard {
            root: &root,
            checked_at: issued_at,
        };
        let batch = BR196TransportBatch {
            ordinal: 1,
            template_ids: &["T-01-account-mode"],
            text: "[TEST_CODE 模板验收] test",
        };
        let clock = FakeClock::new(vec![Ok(root.expires_at)]);
        let failure =
            spawn_br196_batch(guard.mint_batch_permit(), &batch, "TEST_CODE_CHAT", &clock)
                .unwrap_err();
        assert!(!failure.process_attempted);
        assert!(failure.reason.contains("expired"));
    }

    #[test]
    #[serial_test::serial]
    fn br196_fake_magiclaw_process_yields_exact_receipt_without_network() {
        let _guard =
            crate::TestEnvGuard::capture(&["MAGICLAW_BIN", "MAGICLAW_HOME", "TEST_CODE_ARGV"]);
        let root = std::env::temp_dir().join(format!(
            "TEST_CODE_BR196_FAKE_{}_{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let executable = root.join("magiclaw-fake");
        std::fs::write(
            &executable,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$TEST_CODE_ARGV\"\nprintf '%s\\n' 'send ok (feishu): message_id=TEST_CODE_MESSAGE_1, platform_msg_id=TEST_CODE_PLATFORM_1'\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&executable, permissions).unwrap();
        let argv = root.join("argv.txt");
        std::fs::write(
            root.join(".env"),
            "FEISHU_ACCOUNT_ID=TEST_CODE_TENANT\nFEISHU_APP_ID=TEST_CODE_APP\n",
        )
        .unwrap();
        std::env::set_var("MAGICLAW_BIN", &executable);
        std::env::set_var("MAGICLAW_HOME", &root);
        std::env::set_var("TEST_CODE_ARGV", &argv);

        let issued_at = Utc::now();
        let authority =
            authorize_target(identity(), &allowlist(true, false), "allow-hash").unwrap();
        let permit = BR196LiveFeishuAcceptancePermit {
            test_code: "TEST_CODE_FAKE_PROCESS".to_string(),
            authority,
            issued_at,
            expires_at: issued_at + Duration::minutes(5),
            _not_send_or_sync: PhantomData,
        };
        let guard = BatchCheckGuard {
            root: &permit,
            checked_at: issued_at,
        };
        let batch = BR196TransportBatch {
            ordinal: 1,
            template_ids: &["T-01-account-mode"],
            text: "[TEST_CODE 模板验收] test",
        };
        let clock = FakeClock::new(vec![Ok(issued_at), Ok(issued_at)]);
        let receipt =
            spawn_br196_batch(guard.mint_batch_permit(), &batch, "TEST_CODE_CHAT", &clock).unwrap();
        assert_eq!(receipt.message_id, "TEST_CODE_MESSAGE_1");
        let captured = std::fs::read_to_string(argv).unwrap();
        assert!(captured.contains("--to\nTEST_CODE_CHAT"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[serial_test::serial]
    fn br196_multiple_batches_count_families_and_audits_exactly_once() {
        let _guard =
            crate::TestEnvGuard::capture(&["MAGICLAW_BIN", "MAGICLAW_HOME", "TEST_CODE_COUNTER"]);
        let temp_root = std::env::temp_dir().join(format!(
            "TEST_CODE_BR196_MULTI_{}_{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir_all(&temp_root).unwrap();
        let executable = temp_root.join("magiclaw-fake");
        let counter = temp_root.join("counter.txt");
        std::fs::write(
            &executable,
            "#!/bin/sh\nprintf 'attempt\\n' >> \"$TEST_CODE_COUNTER\"\nn=$(wc -l < \"$TEST_CODE_COUNTER\" | tr -d ' ')\nprintf 'send ok (feishu): message_id=TEST_CODE_MESSAGE_%s, platform_msg_id=TEST_CODE_PLATFORM_%s\\n' \"$n\" \"$n\"\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&executable, permissions).unwrap();
        std::fs::write(
            temp_root.join(".env"),
            "FEISHU_ACCOUNT_ID=TEST_CODE_TENANT\nFEISHU_APP_ID=TEST_CODE_APP\n",
        )
        .unwrap();
        std::env::set_var("MAGICLAW_BIN", &executable);
        std::env::set_var("MAGICLAW_HOME", &temp_root);
        std::env::set_var("TEST_CODE_COUNTER", &counter);

        let issued_at = Utc::now();
        let authority =
            authorize_target(identity(), &allowlist(true, false), "allow-hash").unwrap();
        let permit = BR196LiveFeishuAcceptancePermit {
            test_code: "TEST_CODE_MULTI_BATCH".to_string(),
            authority,
            issued_at,
            expires_at: issued_at + Duration::minutes(5),
            _not_send_or_sync: PhantomData,
        };
        let first_ids = ["T-01-account-mode", "T-02-data-mode"];
        let second_ids = ["T-03-holding-plan"];
        let batches = [
            BR196TransportBatch {
                ordinal: 1,
                template_ids: &first_ids,
                text: "[TEST_CODE 模板验收] batch one",
            },
            BR196TransportBatch {
                ordinal: 2,
                template_ids: &second_ids,
                text: "[TEST_CODE 模板验收] batch two",
            },
        ];
        let clock = FakeClock::new(vec![
            Ok(issued_at),
            Ok(issued_at),
            Ok(issued_at),
            Ok(issued_at),
            Ok(issued_at),
            Ok(issued_at),
            Ok(issued_at),
            Ok(issued_at),
        ]);
        let audit = ReceiptAuditWriter {
            root: temp_root.join("audit"),
        };
        let frozen =
            crate::br196_test_delivery::capture_news_process_capability(false, 0, "a".repeat(64))
                .unwrap();
        let resolver = || authorize_target(identity(), &allowlist(true, false), "allow-hash");
        let report =
            deliver_batches_with(&permit, &batches, &frozen, &clock, &audit, &resolver).unwrap();
        assert_eq!(report.external_process_attempted, 2);
        assert_eq!(report.batches_attempted, 2);
        assert_eq!(report.batches_pushed, 2);
        assert_eq!(report.families_pushed, 3);
        assert_eq!(report.receipt_audit_appended, 2);
        let audit_path = audit
            .root
            .join(format!("{}.jsonl", issued_at.format("%Y-%m-%d")));
        assert_eq!(
            std::fs::read_to_string(audit_path).unwrap().lines().count(),
            2
        );

        std::fs::remove_dir_all(temp_root).unwrap();
    }
}
