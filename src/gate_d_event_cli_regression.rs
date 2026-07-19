use super::*;

#[test]
fn malformed_replay_forms_are_rejected_at_their_exact_boundary() {
    assert!(matches!(
        parse_args(&["monitor", "--replay", "--replay-rate-ms"]),
        Err(CliError::InvalidInteger(message)) if message.contains("missing value")
    ));
    assert!(matches!(
        parse_args(&["monitor", "--replay-rate-ms=10"]),
        Err(CliError::ReplayRateWithoutReplay)
    ));
    assert!(matches!(
        parse_args(&["monitor", "--replay"]),
        Err(CliError::MalformedDate(message)) if message.contains("missing date")
    ));
}

#[test]
fn history_only_filters_are_rejected_without_history_mode() {
    for flag in [
        "--code=TEST_CODE_000001",
        "--kind=signal",
        "--limit=2",
        "--sink=test",
    ] {
        assert!(matches!(
            parse_args(&["monitor", flag]),
            Err(CliError::UnrecognizedFlag(actual)) if actual == flag
        ));
    }
}

#[test]
fn both_supported_program_path_forms_are_ignored_but_other_positionals_fail() {
    assert_eq!(
        parse_args(&["/tmp/monitor"]).expect("unix program path"),
        None
    );
    assert_eq!(
        parse_args(&[r"C:\tmp\monitor"]).expect("windows program path"),
        None
    );
    assert!(matches!(
        parse_args(&["worker"]),
        Err(CliError::UnrecognizedArg(0, value)) if value == "worker"
    ));
}
