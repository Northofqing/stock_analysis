use chrono::{DateTime, NaiveDate};

use crate::calendar::{resolve_verified_replay_range, verified_a_share_calendar_authority_hash};
use crate::monitor::push_job::{
    CalendarId, MachineCatalog, Namespace, RunId, Sha256Digest, UnitId,
};

use super::activation::PromotionAction;
use super::activation_deployment::{
    observe_activation_business_day, revalidate_activation_business_day, ActivationCalendarClaims,
    ActivationCalendarScope, ActivationDeploymentError, ObservedActivationBusinessDay,
};
use super::activation_transaction::UtcMicrosRange;

fn micros(value: &str) -> u64 {
    u64::try_from(
        DateTime::parse_from_rfc3339(value)
            .expect("TEST_CODE timestamp")
            .timestamp_micros(),
    )
    .expect("TEST_CODE positive timestamp")
}

fn digest(value: char) -> Sha256Digest {
    Sha256Digest::parse("TEST_CODE hash", &value.to_string().repeat(64)).expect("TEST_CODE SHA")
}

struct CalendarFixture {
    catalog: MachineCatalog,
    claims: ActivationCalendarClaims,
    namespace: Namespace,
    calendar_id: CalendarId,
    unit: UnitId,
    action: PromotionAction,
    now: u64,
    window: UtcMicrosRange,
}

impl CalendarFixture {
    fn new(timestamp: &str, action: PromotionAction) -> Self {
        let catalog = MachineCatalog::bundled().expect("TEST_CODE catalog");
        let date = NaiveDate::from_ymd_opt(2026, 9, 8).expect("TEST_CODE date");
        // Reference established replay API independently of the new natural-day adapter.
        let calendar =
            resolve_verified_replay_range(date, date).expect("TEST_CODE replay authority");
        let namespace = Namespace::test(
            RunId::try_new("activation-calendar".to_owned()).expect("TEST_CODE run"),
        );
        let calendar_id = CalendarId::try_new("test-approved-sse-calendar".to_owned())
            .expect("TEST_CODE calendar id");
        let unit = catalog.units()[0].id().clone();
        let claims = ActivationCalendarClaims {
            namespace: namespace.clone(),
            catalog_sha256: catalog.catalog_sha256().clone(),
            catalog_units: catalog
                .units()
                .iter()
                .map(|unit| unit.id().clone())
                .collect(),
            calendar_id: calendar_id.clone(),
            authority_sha256: Sha256Digest::parse("TEST_CODE authority", calendar.authority_hash())
                .expect("TEST_CODE hash"),
            utc_offset_seconds: 28_800,
        };
        let now = micros(timestamp);
        Self {
            catalog,
            claims,
            namespace,
            calendar_id,
            unit,
            action,
            now,
            window: UtcMicrosRange {
                start: now - 1_000_000,
                end: now + 172_800_000_000,
            },
        }
    }

    fn scope(&self) -> ActivationCalendarScope<'_> {
        ActivationCalendarScope {
            namespace: &self.namespace,
            calendar_id: &self.calendar_id,
            unit_id: &self.unit,
            action: self.action,
        }
    }

    fn observe(&self) -> Result<ObservedActivationBusinessDay, ActivationDeploymentError> {
        observe_activation_business_day(
            &self.catalog,
            &self.claims,
            &self.scope(),
            self.now,
            self.window,
        )
    }

    fn revalidate(
        &self,
        previous: &ObservedActivationBusinessDay,
    ) -> Result<ObservedActivationBusinessDay, ActivationDeploymentError> {
        revalidate_activation_business_day(
            previous,
            &self.catalog,
            &self.claims,
            &self.scope(),
            self.now,
            self.window,
        )
    }
}

#[test]
fn calendar_quota_is_shanghai_natural_day_not_utc_or_market_session() {
    for now in [
        "2026-09-07T16:00:00Z",
        "2026-09-08T01:15:00Z",
        "2026-09-08T04:00:00Z",
        "2026-09-08T15:59:59.999999Z",
    ] {
        let fixture = CalendarFixture::new(now, PromotionAction::Activate);
        assert_eq!(fixture.claims.catalog_units.len(), 52);
        let observed = fixture.observe().expect("TEST_CODE observed");
        assert_eq!(observed.business_date().as_str(), "2026-09-08");
        assert_eq!(
            observed.interval(),
            UtcMicrosRange {
                start: micros("2026-09-07T16:00:00Z"),
                end: micros("2026-09-08T16:00:00Z")
            }
        );
        assert_eq!(fixture.revalidate(&observed), Ok(observed));
    }
}

#[test]
fn calendar_closures_block_activate_but_keep_exact_rollback_natural_day() {
    for (now, day, start, end) in [
        (
            "2026-09-12T02:00:00Z",
            "2026-09-12",
            "2026-09-11T16:00:00Z",
            "2026-09-12T16:00:00Z",
        ),
        (
            "2026-10-01T02:00:00Z",
            "2026-10-01",
            "2026-09-30T16:00:00Z",
            "2026-10-01T16:00:00Z",
        ),
    ] {
        let mut fixture = CalendarFixture::new(now, PromotionAction::Activate);
        assert_eq!(
            fixture.observe(),
            Err(ActivationDeploymentError::NonTradingDay)
        );
        fixture.action = PromotionAction::Rollback;
        let observation = fixture
            .observe()
            .expect("TEST_CODE raw rollback day, NOT approval");
        assert_eq!(observation.business_date().as_str(), day);
        assert_eq!(
            observation.interval(),
            UtcMicrosRange {
                start: micros(start),
                end: micros(end)
            }
        );
        let date = NaiveDate::parse_from_str(day, "%Y-%m-%d").expect("TEST_CODE day");
        assert_eq!(
            verified_a_share_calendar_authority_hash(date).expect("TEST_CODE covered closure"),
            fixture.claims.authority_sha256.as_str()
        );
        assert!(
            resolve_verified_replay_range(date, date).is_err(),
            "TEST_CODE original replay still rejects empty range"
        );
    }
}

#[test]
fn calendar_coverage_is_fail_closed_for_promotion_and_rollback() {
    for now in ["2024-12-31T02:00:00Z", "2027-01-04T02:00:00Z"] {
        for action in [PromotionAction::Activate, PromotionAction::Rollback] {
            let fixture = CalendarFixture::new(now, action);
            assert_eq!(
                fixture.observe(),
                Err(ActivationDeploymentError::CalendarUnavailable)
            );
        }
    }
    let fixture = CalendarFixture::new("2026-12-31T15:59:59Z", PromotionAction::Rollback);
    assert_eq!(
        fixture
            .observe()
            .expect("TEST_CODE final covered day")
            .interval()
            .end,
        micros("2026-12-31T16:00:00Z")
    );
}

#[test]
fn calendar_exact_join_rejects_each_changed_deployment_claim() {
    for field in 0..6 {
        let mut fixture = CalendarFixture::new("2026-09-08T02:00:00Z", PromotionAction::Activate);
        match field {
            0 => fixture.claims.namespace = Namespace::Production,
            1 => {
                fixture.claims.namespace = Namespace::test(
                    RunId::try_new("another-run".to_owned()).expect("TEST_CODE run"),
                )
            }
            2 => fixture.claims.catalog_sha256 = digest('a'),
            3 => {
                fixture.claims.calendar_id = CalendarId::try_new("different-calendar".to_owned())
                    .expect("TEST_CODE calendar")
            }
            4 => fixture.claims.authority_sha256 = digest('b'),
            5 => fixture.claims.utc_offset_seconds = 0,
            _ => unreachable!("TEST_CODE finite matrix"),
        }
        assert_eq!(
            fixture.observe(),
            Err(ActivationDeploymentError::CalendarBindingMismatch),
            "TEST_CODE field {field}"
        );
    }
}

#[test]
fn calendar_requires_complete_unique_catalog_units_not_one_selected_unit() {
    for field in 0..5 {
        let mut fixture = CalendarFixture::new("2026-09-08T02:00:00Z", PromotionAction::Activate);
        let unknown = UnitId::try_new("MU-unknown".to_owned()).expect("TEST_CODE Unit");
        match field {
            0 => {
                fixture.claims.catalog_units.pop();
            }
            1 => {
                fixture.claims.catalog_units[1] = fixture.claims.catalog_units[0].clone();
            }
            2 => {
                fixture.claims.catalog_units.push(unknown);
            }
            3 => {
                fixture.claims.catalog_units[1] = unknown;
            }
            4 => fixture.unit = unknown,
            _ => unreachable!("TEST_CODE finite matrix"),
        }
        assert_eq!(
            fixture.observe(),
            Err(ActivationDeploymentError::UnitCoverageMismatch)
        );
    }
    let mut fixture = CalendarFixture::new("2026-09-08T02:00:00Z", PromotionAction::Activate);
    let observed = fixture.observe().expect("TEST_CODE full coverage");
    fixture.claims.catalog_units.reverse();
    assert_eq!(
        fixture.revalidate(&observed),
        Ok(observed),
        "TEST_CODE set ordering is not identity"
    );
}

#[test]
fn calendar_recheck_rejects_changed_scope_even_when_new_claims_are_self_consistent() {
    for field in 0..5 {
        let mut fixture = CalendarFixture::new("2026-09-08T02:00:00Z", PromotionAction::Activate);
        let previous = fixture.observe().expect("TEST_CODE first observation");
        match field {
            0 => {
                fixture.namespace = Namespace::Production;
                fixture.claims.namespace = fixture.namespace.clone();
            }
            1 => {
                fixture.calendar_id = CalendarId::try_new("another-approved-claim".to_owned())
                    .expect("TEST_CODE calendar");
                fixture.claims.calendar_id = fixture.calendar_id.clone();
            }
            2 => fixture.unit = fixture.catalog.units()[1].id().clone(),
            3 => fixture.action = PromotionAction::Rollback,
            4 => fixture.window.end += 1,
            _ => unreachable!("TEST_CODE finite matrix"),
        }
        assert!(
            fixture.observe().is_ok(),
            "TEST_CODE new raw claim still self-consistent"
        );
        assert_eq!(
            fixture.revalidate(&previous),
            Err(ActivationDeploymentError::CalendarBindingMismatch)
        );
    }
}

#[test]
fn calendar_approval_windows_are_half_open_and_i64_bounded() {
    let mut fixture = CalendarFixture::new("2026-09-08T02:00:00Z", PromotionAction::Activate);
    fixture.window = UtcMicrosRange {
        start: fixture.now,
        end: fixture.now + 2,
    };
    let observed = fixture.observe().expect("TEST_CODE inclusive start");
    fixture.now += 1;
    assert!(fixture.revalidate(&observed).is_ok());
    fixture.now += 1;
    assert_eq!(
        fixture.revalidate(&observed),
        Err(ActivationDeploymentError::InvalidTime)
    );
    for window in [
        UtcMicrosRange {
            start: fixture.now + 1,
            end: fixture.now + 2,
        },
        UtcMicrosRange {
            start: fixture.now,
            end: fixture.now,
        },
        UtcMicrosRange {
            start: fixture.now + 1,
            end: fixture.now,
        },
        UtcMicrosRange {
            start: 0,
            end: u64::MAX,
        },
    ] {
        fixture.window = window;
        assert_eq!(
            fixture.observe(),
            Err(ActivationDeploymentError::InvalidTime)
        );
    }
    fixture.now = u64::MAX;
    assert_eq!(
        fixture.observe(),
        Err(ActivationDeploymentError::InvalidTime)
    );
}

#[test]
fn calendar_lock_recheck_rejects_backwards_clock_and_cross_day_without_replanning() {
    let mut fixture = CalendarFixture::new("2026-09-08T15:59:59Z", PromotionAction::Activate);
    let observed = fixture.observe().expect("TEST_CODE before midnight");
    fixture.now -= 1;
    assert_eq!(
        fixture.revalidate(&observed),
        Err(ActivationDeploymentError::TimeContextChanged)
    );
    fixture.now = micros("2026-09-08T16:00:00Z");
    assert!(
        fixture.observe().is_ok(),
        "TEST_CODE next day otherwise valid"
    );
    assert_eq!(
        fixture.revalidate(&observed),
        Err(ActivationDeploymentError::TimeContextChanged)
    );
}

#[cfg(unix)]
mod files {
    use std::fs::{self, File};
    use std::io::{Seek, SeekFrom};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    use super::super::activation_deployment::{
        DeploymentMaterialMetadata, OpenedDeploymentMaterial,
    };
    use super::ActivationDeploymentError;

    const ABC_SHA: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn descriptor_reads_actual_bytes_and_metadata_without_moving_shared_offset() {
        let root = tempfile::tempdir().expect("TEST_CODE temp root");
        let path = root.path().join("artifact");
        fs::write(&path, b"abc").expect("TEST_CODE artifact");
        let file = File::open(&path).expect("TEST_CODE file");
        let mut shared = file.try_clone().expect("TEST_CODE clone");
        shared.seek(SeekFrom::Start(2)).expect("TEST_CODE cursor");
        let expected = file.metadata().expect("TEST_CODE metadata");
        let observed = OpenedDeploymentMaterial::observe(file, 3).expect("TEST_CODE observation");
        let metadata: &DeploymentMaterialMetadata = observed.metadata();
        assert_eq!(
            (metadata.device, metadata.inode),
            (expected.dev(), expected.ino())
        );
        assert_eq!(
            (metadata.uid, metadata.gid, metadata.mode),
            (expected.uid(), expected.gid(), expected.mode())
        );
        assert_eq!(metadata.size, 3);
        assert_eq!(observed.sha256().as_str(), ABC_SHA);
        assert_eq!(shared.stream_position().expect("TEST_CODE position"), 2);
        assert_eq!(observed.revalidate(), Ok(()));
        assert!(!format!("{observed:?}").contains("artifact"));
    }

    #[test]
    fn descriptor_rejects_oversized_and_non_regular_sources() {
        let root = tempfile::tempdir().expect("TEST_CODE temp root");
        let path = root.path().join("artifact");
        fs::write(&path, b"abc").expect("TEST_CODE file");
        assert_eq!(
            OpenedDeploymentMaterial::observe(File::open(&path).expect("TEST_CODE open"), 2)
                .unwrap_err(),
            ActivationDeploymentError::MaterialRejected
        );
        assert_eq!(
            OpenedDeploymentMaterial::observe(
                File::open(root.path()).expect("TEST_CODE directory"),
                1024
            )
            .unwrap_err(),
            ActivationDeploymentError::MaterialRejected
        );
        let write_only = fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("TEST_CODE write-only fd");
        assert_eq!(
            OpenedDeploymentMaterial::observe(write_only, 3).unwrap_err(),
            ActivationDeploymentError::MaterialUnreadable
        );
        fs::write(&path, b"").expect("TEST_CODE empty file");
        assert_eq!(
            OpenedDeploymentMaterial::observe(File::open(&path).expect("TEST_CODE empty open"), 0)
                .expect("TEST_CODE raw empty bytes")
                .sha256()
                .as_str(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn descriptor_revalidation_rejects_same_inode_bytes_or_permission_change() {
        for change in ["content", "permissions"] {
            let root = tempfile::tempdir().expect("TEST_CODE temp root");
            let path = root.path().join("artifact");
            fs::write(&path, b"abc").expect("TEST_CODE bytes");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("TEST_CODE mode");
            let observed =
                OpenedDeploymentMaterial::observe(File::open(&path).expect("TEST_CODE open"), 1024)
                    .expect("TEST_CODE observe");
            if change == "content" {
                fs::write(&path, b"xyz").expect("TEST_CODE new same length bytes");
            } else {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o666))
                    .expect("TEST_CODE new mode");
            }
            assert_eq!(
                fs::metadata(&path).expect("TEST_CODE metadata").ino(),
                observed.metadata().inode
            );
            assert_eq!(
                observed.revalidate(),
                Err(ActivationDeploymentError::MaterialChanged),
                "TEST_CODE {change}"
            );
        }
    }

    #[test]
    fn descriptor_observation_never_reopens_replaced_path() {
        let root = tempfile::tempdir().expect("TEST_CODE temp root");
        let path = root.path().join("artifact");
        fs::write(&path, b"abc").expect("TEST_CODE original");
        let file = File::open(&path).expect("TEST_CODE held descriptor");
        let original_inode = file.metadata().expect("TEST_CODE original metadata").ino();
        fs::rename(&path, root.path().join("held-artifact")).expect("TEST_CODE move original");
        fs::write(&path, b"replacement").expect("TEST_CODE replace pathname");
        let observed =
            OpenedDeploymentMaterial::observe(file, 3).expect("TEST_CODE reads held original");
        assert_eq!(observed.metadata().inode, original_inode);
        assert_ne!(
            fs::metadata(&path)
                .expect("TEST_CODE replacement metadata")
                .ino(),
            original_inode
        );
        assert_eq!(observed.sha256().as_str(), ABC_SHA);
        assert_eq!(observed.revalidate(), Ok(()));
    }

    #[test]
    fn descriptor_checks_detect_real_file_mutation_between_metadata_and_read() {
        for replacement in [b"".as_slice(), b"longer".as_slice(), b"xyz".as_slice()] {
            let root = tempfile::tempdir().expect("TEST_CODE temp root");
            let path = root.path().join("artifact");
            fs::write(&path, b"abc").expect("TEST_CODE original");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .expect("TEST_CODE original mode");
            let file = File::open(&path).expect("TEST_CODE held fd");
            let mut hook_called = false;
            let mut mutate = || {
                hook_called = true;
                fs::write(&path, replacement).expect("TEST_CODE mutate actual file");
                // Make the same-length case deterministic even on coarse-mtime filesystems.
                fs::set_permissions(&path, fs::Permissions::from_mode(0o640))
                    .expect("TEST_CODE mutate actual mode");
            };
            let result = OpenedDeploymentMaterial::observe_with_read_hook(file, 1024, &mut mutate);
            assert!(hook_called);
            assert_eq!(
                result.unwrap_err(),
                ActivationDeploymentError::MaterialChanged
            );
        }
    }

    #[test]
    fn identical_artifact_hashes_do_not_identify_a_deployment_or_source_instance() {
        let root = tempfile::tempdir().expect("TEST_CODE temp root");
        let first = root.path().join("first");
        let clone = root.path().join("clone");
        fs::write(&first, b"abc").expect("TEST_CODE first");
        fs::copy(&first, &clone).expect("TEST_CODE independent same bytes");
        let first =
            OpenedDeploymentMaterial::observe(File::open(first).expect("TEST_CODE first fd"), 3)
                .expect("TEST_CODE raw first");
        let clone =
            OpenedDeploymentMaterial::observe(File::open(clone).expect("TEST_CODE clone fd"), 3)
                .expect("TEST_CODE raw clone");
        assert_eq!(first.sha256(), clone.sha256());
        assert_ne!(first.metadata().inode, clone.metadata().inode);
        // Neither value has an authentication/owner-permit conversion.
    }
}
