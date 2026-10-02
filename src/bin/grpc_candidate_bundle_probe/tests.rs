use super::*;

fn offline_args(output: &Path) -> Args {
    Args::try_parse_from([
        std::ffi::OsString::from("candidate-probe"),
        std::ffi::OsString::from("--profile"),
        std::ffi::OsString::from(PROFILE),
        std::ffi::OsString::from("--offline-plan"),
        std::ffi::OsString::from("--output"),
        output.as_os_str().to_owned(),
    ])
    .unwrap()
}

#[test]
fn candidate_b7_probe_cli_requires_fixed_profile_and_exactly_one_mode() {
    let valid = [
        "candidate-probe",
        "--profile",
        PROFILE,
        "--offline-plan",
        "--output",
        "/isolated/new-plan",
    ];
    assert!(Args::try_parse_from(valid).is_ok());
    for args in [
        vec!["candidate-probe"],
        vec!["candidate-probe", "--offline-plan", "--output", "new-plan"],
        vec!["candidate-probe", "--profile", PROFILE],
        vec!["candidate-probe", "--profile", PROFILE, "--offline-plan"],
        vec![
            "candidate-probe",
            "--profile",
            "other-profile",
            "--offline-plan",
            "--output",
            "new-plan",
        ],
        vec![
            "candidate-probe",
            "--profile",
            PROFILE,
            "--execute-plan",
            "plan",
            "--output",
            "new-receipt",
        ],
        vec![
            "candidate-probe",
            "--profile",
            PROFILE,
            "--verify-receipt",
            "receipt",
        ],
    ] {
        assert!(Args::try_parse_from(args).is_err());
    }
    for extra in [
        vec!["--execute-plan", "plan", "--bundle", "bundle"],
        vec!["--verify-receipt", "receipt"],
        vec!["--bundle", "bundle"],
        vec!["--receipt-sha256", "0"],
    ] {
        let mut args = valid.to_vec();
        args.extend(extra);
        assert!(Args::try_parse_from(args).is_err());
    }
    assert!(Args::try_parse_from([
        "candidate-probe",
        "--profile",
        PROFILE,
        "--execute-plan",
        "plan",
        "--bundle",
        "bundle",
        "--output",
        "new-receipt",
    ])
    .is_ok());
}

#[test]
fn candidate_b7_probe_cli_rejects_identity_endpoint_and_credential_overrides() {
    for option in [
        "--expected-identity",
        "--service-version",
        "--source-revision",
        "--contract-sha256",
        "--binary-sha256",
        "--endpoint",
        "--tls-server-name",
        "--bearer",
        "--authorization",
        "--allow-unadmitted",
        "--database",
        "--notify",
    ] {
        assert!(Args::try_parse_from([
            "candidate-probe",
            "--profile",
            PROFILE,
            "--offline-plan",
            "--output",
            "new-plan",
            option,
            "not-an-authority",
        ])
        .is_err());
    }
}

#[test]
fn candidate_b7_probe_cli_receipt_hash_requires_lowercase_exact_sha256() {
    let valid = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    assert!(Args::try_parse_from([
        "candidate-probe",
        "--profile",
        PROFILE,
        "--verify-receipt",
        "receipt",
        "--receipt-sha256",
        valid,
    ])
    .is_ok());
    for invalid in [
        "",
        "0123456789abcdef",
        "0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef",
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdeg",
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0",
    ] {
        assert!(Args::try_parse_from([
            "candidate-probe",
            "--profile",
            PROFILE,
            "--verify-receipt",
            "receipt",
            "--receipt-sha256",
            invalid,
        ])
        .is_err());
    }
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "linux",
    target_os = "android"
))]
mod unix_tests {
    use super::*;
    use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};

    fn isolated_directory() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        root
    }

    #[tokio::test]
    async fn candidate_b7_probe_cli_offline_plan_writes_checked_plan_without_bundle() {
        let root = isolated_directory();
        let forbidden = root.path().join("forbidden-runtime");
        let output = root.path().join("plan.bin");
        let args = offline_args(&output);
        assert!(args.bundle.is_none());
        assert!(args.execute_plan.is_none());
        let summary = run(args, &forbidden).await.unwrap();
        assert_eq!(summary.outcome, "OfflinePlanCreated");
        assert_eq!(summary.rpc_count, 0);
        assert_eq!(summary.exit_code, 0);
        assert!(serde_json::from_str::<serde_json::Value>(&summary.compiled_inputs).is_ok());
        let bytes = fs::read(&output).unwrap();
        assert!(!bytes.is_empty());
        assert!(bytes.len() <= MAX_CANDIDATE_PLAN_BYTES);
        assert_eq!(sha256(&bytes), summary.artifact_sha256);
        assert_eq!(fs::metadata(&output).unwrap().mode() & 0o777, 0o600);
        assert_eq!(
            CandidateProbePlan::read_checked(&bytes)
                .unwrap()
                .canonical_bytes()
                .unwrap(),
            bytes
        );
        assert!(matches!(
            run(offline_args(&output), &forbidden).await,
            Err(LocalError::OutputRejected)
        ));
        assert_eq!(fs::read(&output).unwrap(), bytes);
    }

    #[tokio::test]
    async fn candidate_b7_probe_cli_execution_preflight_rejects_before_bundle_load() {
        let root = isolated_directory();
        let forbidden = root.path().join("forbidden-runtime");
        let plan_path = root.path().join("plan");
        let bundle = root.path().join("empty-bundle");
        let output_parent = root.path().join("public-output");
        fs::create_dir(&bundle).unwrap();
        fs::create_dir(&output_parent).unwrap();
        fs::set_permissions(&output_parent, fs::Permissions::from_mode(0o755)).unwrap();
        let output = output_parent.join("receipt");
        let args = || {
            Args::try_parse_from([
                std::ffi::OsString::from("candidate-probe"),
                std::ffi::OsString::from("--profile"),
                std::ffi::OsString::from(PROFILE),
                std::ffi::OsString::from("--execute-plan"),
                plan_path.as_os_str().to_owned(),
                std::ffi::OsString::from("--bundle"),
                bundle.as_os_str().to_owned(),
                std::ffi::OsString::from("--output"),
                output.as_os_str().to_owned(),
            ])
            .unwrap()
        };
        fs::write(&plan_path, b"invalid plan").unwrap();
        assert!(matches!(
            run(args(), &forbidden).await,
            Err(LocalError::PlanRejected)
        ));
        fs::write(
            &plan_path,
            CandidateProbePlan::windows_b7()
                .unwrap()
                .canonical_bytes()
                .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            run(args(), &forbidden).await,
            Err(LocalError::OutputRejected)
        ));
        assert_eq!(fs::read_dir(&bundle).unwrap().count(), 0);
        assert!(!output.exists());
    }

    #[test]
    fn candidate_b7_probe_cli_output_preserves_existing_file_and_rejects_symlink_leaf() {
        let root = isolated_directory();
        let forbidden = root.path().join("forbidden");
        let output = root.path().join("receipt.bin");
        let original = b"original receipt";
        fs::write(&output, original).unwrap();
        assert!(matches!(
            PrivateOutput::create(&output, &forbidden),
            Err(LocalError::OutputRejected)
        ));
        assert_eq!(fs::read(&output).unwrap(), original);
        let alias = root.path().join("receipt-alias.bin");
        symlink(&output, &alias).unwrap();
        assert!(matches!(
            PrivateOutput::create(&alias, &forbidden),
            Err(LocalError::OutputRejected)
        ));
        assert_eq!(fs::read(&output).unwrap(), original);
        let new_output = root.path().join("new-receipt.bin");
        let mut file = PrivateOutput::create(&new_output, &forbidden).unwrap();
        file.write_durable(b"bounded receipt", 32).unwrap();
        assert_eq!(fs::read(&new_output).unwrap(), b"bounded receipt");
        assert_eq!(fs::metadata(&new_output).unwrap().nlink(), 1);
    }

    #[test]
    fn candidate_b7_probe_cli_output_rejects_nonprivate_and_forbidden_aliases() {
        let root = isolated_directory();
        let forbidden = root.path().join("forbidden-runtime");
        fs::create_dir(&forbidden).unwrap();
        fs::set_permissions(&forbidden, fs::Permissions::from_mode(0o700)).unwrap();
        let public = root.path().join("public-output");
        fs::create_dir(&public).unwrap();
        fs::set_permissions(&public, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(
            PrivateOutput::create(&public.join("receipt"), &forbidden),
            Err(LocalError::OutputRejected)
        ));
        let alias = root.path().join("runtime-alias");
        symlink(&forbidden, &alias).unwrap();
        for path in [
            forbidden.join("receipt"),
            alias.join("receipt"),
            forbidden
                .join("..")
                .join("forbidden-runtime")
                .join("receipt"),
        ] {
            assert!(matches!(
                PrivateOutput::create(&path, &forbidden),
                Err(LocalError::OutputRejected)
            ));
        }
        assert!(!forbidden.join("receipt").exists());
        assert!(!public.join("receipt").exists());
    }

    #[test]
    fn candidate_b7_probe_cli_output_rejects_retained_parent_replaced_by_alias_or_new_directory() {
        let root = isolated_directory();
        let forbidden = root.path().join("forbidden-runtime");
        fs::create_dir(&forbidden).unwrap();
        fs::set_permissions(&forbidden, fs::Permissions::from_mode(0o700)).unwrap();
        for alias in [true, false] {
            let parent = root
                .path()
                .join(if alias { "alias-parent" } else { "new-parent" });
            fs::create_dir(&parent).unwrap();
            fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
            let output = parent.join("receipt");
            let mut retained = PrivateOutput::create(&output, &forbidden).unwrap();
            let original = root.path().join(if alias {
                "original-alias"
            } else {
                "original-new"
            });
            fs::rename(&parent, &original).unwrap();
            if alias {
                symlink(&forbidden, &parent).unwrap();
                assert!(matches!(
                    PrivateOutput::create(&parent.join("other-receipt"), &forbidden),
                    Err(LocalError::OutputRejected)
                ));
            } else {
                fs::create_dir(&parent).unwrap();
                fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
            }
            assert_eq!(
                retained
                    .write_durable(b"TEST_CODE_MUST_NOT_WRITE", 64)
                    .unwrap_err(),
                LocalError::OutputRejected
            );
            assert_eq!(fs::read(original.join("receipt")).unwrap(), b"");
            assert_eq!(fs::read_dir(&parent).unwrap().count(), 0);
            assert_eq!(fs::read_dir(&forbidden).unwrap().count(), 0);
        }
    }

    #[test]
    fn candidate_b7_probe_cli_input_rejects_symlink_hardlink_oversize_and_nonregular() {
        let root = isolated_directory();
        let forbidden = root.path().join("forbidden-runtime");
        let input = root.path().join("input");
        fs::write(&input, b"1234").unwrap();
        assert_eq!(
            read_bounded_regular(&input, 4, &forbidden).unwrap(),
            b"1234"
        );
        assert_eq!(
            read_bounded_regular(&input, 3, &forbidden).unwrap_err(),
            LocalError::InputRejected
        );
        let symlink_input = root.path().join("symlink-input");
        symlink(&input, &symlink_input).unwrap();
        assert_eq!(
            read_bounded_regular(&symlink_input, 4, &forbidden).unwrap_err(),
            LocalError::InputRejected
        );
        let hardlink_input = root.path().join("hardlink-input");
        fs::hard_link(&input, &hardlink_input).unwrap();
        for linked in [&input, &hardlink_input] {
            assert_eq!(
                read_bounded_regular(linked, 4, &forbidden).unwrap_err(),
                LocalError::InputRejected
            );
        }
        assert_eq!(
            read_bounded_regular(root.path(), 4, &forbidden).unwrap_err(),
            LocalError::InputRejected
        );
    }

    #[test]
    fn candidate_b7_probe_cli_input_rejects_forbidden_canonical_and_lexical_aliases() {
        let root = isolated_directory();
        let forbidden = root.path().join("forbidden-runtime");
        fs::create_dir(&forbidden).unwrap();
        fs::write(forbidden.join("receipt"), b"1234").unwrap();
        let alias = root.path().join("runtime-alias");
        symlink(&forbidden, &alias).unwrap();
        for path in [
            forbidden.join("receipt"),
            alias.join("receipt"),
            forbidden
                .join("..")
                .join("forbidden-runtime")
                .join("receipt"),
        ] {
            assert_eq!(
                read_bounded_regular(&path, 4, &forbidden).unwrap_err(),
                LocalError::InputRejected
            );
        }
    }
}
