use stock_analysis::selection::activation_runtime::{
    OutcomeDisabledReason, SelectionDisabledReason,
};
use stock_analysis::selection::trading_calendar_v2::{
    classify_calendar_authority_presence, parse_notice_manifest_canonical,
    parse_parser_equality_canonical, parse_raw_notice_set_canonical,
    parse_release_prerequisite_canonical, CalendarAuthorityClassification,
    CalendarAuthorityPresence, ReviewedCalendarPrerequisiteReason, CALENDAR_MANIFEST_RELATIVE_PATH,
    NOTICE_MANIFEST_RELATIVE_PATH, RAW_NOTICE_ROOT_RELATIVE_PATH,
    RELEASE_PREREQUISITE_RELATIVE_PATH,
};

#[test]
fn br193_calendar_paths_manifest_and_raw_root_are_distinct() {
    assert_eq!(
        CALENDAR_MANIFEST_RELATIVE_PATH,
        "config/selection/a_share_trading_calendar.v1.json"
    );
    assert_eq!(
        NOTICE_MANIFEST_RELATIVE_PATH,
        "config/selection/a_share_trading_calendar_notices.v1.json"
    );
    assert_eq!(
        RAW_NOTICE_ROOT_RELATIVE_PATH,
        "config/selection/a_share_trading_calendar_notices.v1"
    );
    assert_ne!(
        NOTICE_MANIFEST_RELATIVE_PATH, RAW_NOTICE_ROOT_RELATIVE_PATH,
        "the notice manifest file must never double as the raw-notice directory"
    );
    assert!(
        NOTICE_MANIFEST_RELATIVE_PATH.ends_with(".json")
            && !RAW_NOTICE_ROOT_RELATIVE_PATH.ends_with(".json")
    );
}

#[test]
fn br193_calendar_notice_manifest_rfc8785_golden() {
    const GOLDEN: &str = "{\"domain\":\"stock_analysis.a_share_calendar_notice_manifest.v1\",\"schema_version\":1,\"entries\":[{\"provider\":\"sse\",\"published_at\":\"2025-12-22T09:30:00.000000000+08:00\",\"notice_id\":\"sse-2026-calendar\",\"notice_id_sha256\":\"794561dace4e1359f6338b56dcf7161af06637012441b77a7d14ddd87a6ca9b3\",\"canonical_url\":\"https://www.sse.com.cn/lawandrules/sselawsrules/notice/sse-2026-calendar.html\",\"raw_artifact_path\":\"config/selection/a_share_trading_calendar_notices.v1/sse/794561dace4e1359f6338b56dcf7161af06637012441b77a7d14ddd87a6ca9b3.raw\",\"raw_content_sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"parser_id\":\"sse-calendar-notice\",\"parser_version\":\"1\"},{\"provider\":\"szse\",\"published_at\":\"2025-12-22T09:31:00.000000000+08:00\",\"notice_id\":\"szse-2026-calendar\",\"notice_id_sha256\":\"906282cbf2c78188bbd11fdbb09a936e68cbeeb2517dbdf7d4ad3b96212b4747\",\"canonical_url\":\"https://www.szse.cn/lawrules/rule/notice/szse-2026-calendar.html\",\"raw_artifact_path\":\"config/selection/a_share_trading_calendar_notices.v1/szse/906282cbf2c78188bbd11fdbb09a936e68cbeeb2517dbdf7d4ad3b96212b4747.raw\",\"raw_content_sha256\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\"parser_id\":\"szse-calendar-notice\",\"parser_version\":\"1\"}]}";

    let verified =
        parse_notice_manifest_canonical(GOLDEN.as_bytes()).expect("golden manifest must verify");
    assert_eq!(verified.entry_count(), 2);
    assert_eq!(
        verified.content_hash(),
        "4a2a9ecf592526813aa69dff82730db18db41f01af935ca1a254957845a0778d"
    );

    let noncanonical = GOLDEN.replacen(
        "{\"domain\":\"stock_analysis.a_share_calendar_notice_manifest.v1\",\"schema_version\":1",
        "{\"schema_version\":1,\"domain\":\"stock_analysis.a_share_calendar_notice_manifest.v1\"",
        1,
    );
    let error = parse_notice_manifest_canonical(noncanonical.as_bytes())
        .expect_err("semantic JSON with reordered object keys must reject");
    assert_eq!(error.code(), "calendar_notice_manifest_noncanonical");
}

#[test]
fn br193_calendar_auxiliary_payloads_reject_mutation_and_reorder() {
    const RAW_SET: &str = "{\"domain\":\"stock_analysis.a_share_calendar_raw_notice_set.v1\",\"schema_version\":1,\"entries\":[{\"provider\":\"sse\",\"published_at\":\"2025-12-22T09:30:00.000000000+08:00\",\"notice_id\":\"sse-2026-calendar\",\"raw_artifact_path\":\"config/selection/a_share_trading_calendar_notices.v1/sse/794561dace4e1359f6338b56dcf7161af06637012441b77a7d14ddd87a6ca9b3.raw\",\"raw_content_sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"},{\"provider\":\"szse\",\"published_at\":\"2025-12-22T09:31:00.000000000+08:00\",\"notice_id\":\"szse-2026-calendar\",\"raw_artifact_path\":\"config/selection/a_share_trading_calendar_notices.v1/szse/906282cbf2c78188bbd11fdbb09a936e68cbeeb2517dbdf7d4ad3b96212b4747.raw\",\"raw_content_sha256\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"}]}";
    const PARSER_EQUALITY: &str = "{\"domain\":\"stock_analysis.a_share_calendar_parser_equality.v1\",\"schema_version\":1,\"coverage_start\":\"2026-01-01\",\"coverage_end\":\"2026-01-31\",\"parser_descriptors\":[{\"provider\":\"sse\",\"notice_id\":\"sse-2026-calendar\",\"parser_id\":\"sse-calendar-notice\",\"parser_version\":\"1\",\"executable_revision\":\"cccccccccccccccccccccccccccccccccccccccc\",\"raw_content_sha256\":\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"},{\"provider\":\"szse\",\"notice_id\":\"szse-2026-calendar\",\"parser_id\":\"szse-calendar-notice\",\"parser_version\":\"1\",\"executable_revision\":\"cccccccccccccccccccccccccccccccccccccccc\",\"raw_content_sha256\":\"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"}],\"session_dates\":[\"2026-01-05\",\"2026-01-06\",\"2026-01-07\",\"2026-01-08\",\"2026-01-09\",\"2026-01-12\"],\"t0_d5_vectors\":[{\"t0\":\"2026-01-05\",\"d1\":\"2026-01-06\",\"d2\":\"2026-01-07\",\"d3\":\"2026-01-08\",\"d4\":\"2026-01-09\",\"d5\":\"2026-01-12\"}]}";

    assert_eq!(
        parse_raw_notice_set_canonical(RAW_SET.as_bytes())
            .expect("canonical raw set")
            .entry_count(),
        2
    );
    assert_eq!(
        parse_parser_equality_canonical(PARSER_EQUALITY.as_bytes())
            .expect("canonical parser equality")
            .session_count(),
        6
    );

    let reordered_root = RAW_SET.replacen(
        "{\"domain\":\"stock_analysis.a_share_calendar_raw_notice_set.v1\",\"schema_version\":1",
        "{\"schema_version\":1,\"domain\":\"stock_analysis.a_share_calendar_raw_notice_set.v1\"",
        1,
    );
    assert_eq!(
        parse_raw_notice_set_canonical(reordered_root.as_bytes())
            .expect_err("root-key reorder must reject")
            .code(),
        "calendar_auxiliary_payload_noncanonical"
    );

    let mutated_raw_hash = RAW_SET.replacen(
        "\"raw_content_sha256\":\"aaaaaaaa",
        "\"raw_content_sha256\":\"Aaaaaaaa",
        1,
    );
    assert_eq!(
        parse_raw_notice_set_canonical(mutated_raw_hash.as_bytes())
            .expect_err("one-field raw hash mutation must reject")
            .code(),
        "calendar_auxiliary_payload_invalid"
    );

    let reordered_sessions = PARSER_EQUALITY.replacen(
        "[\"2026-01-05\",\"2026-01-06\"",
        "[\"2026-01-06\",\"2026-01-05\"",
        1,
    );
    assert_eq!(
        parse_parser_equality_canonical(reordered_sessions.as_bytes())
            .expect_err("session array reorder must reject")
            .code(),
        "calendar_auxiliary_payload_invalid"
    );

    let mutated_vector =
        PARSER_EQUALITY.replacen("\"d5\":\"2026-01-12\"", "\"d5\":\"2026-01-13\"", 1);
    assert_eq!(
        parse_parser_equality_canonical(mutated_vector.as_bytes())
            .expect_err("T0..D5 mutation must reject")
            .code(),
        "calendar_auxiliary_payload_invalid"
    );

    let unknown_field = PARSER_EQUALITY.replacen(
        "\"schema_version\":1",
        "\"schema_version\":1,\"caller_override\":true",
        1,
    );
    assert_eq!(
        parse_parser_equality_canonical(unknown_field.as_bytes())
            .expect_err("unknown field must reject")
            .code(),
        "calendar_auxiliary_payload_invalid"
    );
}

#[test]
fn br193_outcome_disabled_reason_enum_and_token_are_closed() {
    let encoded = serde_json::to_string(&OutcomeDisabledReason::OutcomeActivationNotReleased)
        .expect("the one released outcome-disabled reason must serialize");
    assert_eq!(encoded, "\"outcome_activation_not_released\"");

    let decoded: OutcomeDisabledReason =
        serde_json::from_str("\"outcome_activation_not_released\"")
            .expect("the canonical token must decode");
    assert_eq!(decoded, OutcomeDisabledReason::OutcomeActivationNotReleased);

    for rejected in [
        "\"OutcomeActivationNotReleased\"",
        "\"OUTCOME_ACTIVATION_NOT_RELEASED\"",
        "\"outcome_activation_not_released_by_caller\"",
        "\"\"",
        "null",
        "{}",
        "[]",
    ] {
        assert!(
            serde_json::from_str::<OutcomeDisabledReason>(rejected).is_err(),
            "noncanonical token unexpectedly decoded: {rejected}"
        );
    }
}

#[test]
fn br193_selection_disabled_reason_tokens_are_closed() {
    let fixtures = [
        (
            SelectionDisabledReason::SchemaNotAmended,
            "schema_not_amended",
        ),
        (SelectionDisabledReason::ProposalMissing, "proposal_missing"),
        (
            SelectionDisabledReason::BoardArtifactUnverified,
            "board_artifact_unverified",
        ),
        (
            SelectionDisabledReason::BoardArtifactExpired,
            "board_artifact_expired",
        ),
        (
            SelectionDisabledReason::ActivationMissing,
            "activation_missing",
        ),
        (
            SelectionDisabledReason::ActivationNotEffective,
            "activation_not_effective",
        ),
        (
            SelectionDisabledReason::ActivationExpired,
            "activation_expired",
        ),
        (
            SelectionDisabledReason::ActivationUnreceipted,
            "activation_unreceipted",
        ),
        (
            SelectionDisabledReason::ActivationRevoked,
            "activation_revoked",
        ),
        (
            SelectionDisabledReason::TradingCalendarMissing,
            "trading_calendar_missing",
        ),
        (
            SelectionDisabledReason::TradingCalendarUnverified,
            "trading_calendar_unverified",
        ),
        (
            SelectionDisabledReason::TradingCalendarCoverageIncomplete,
            "trading_calendar_coverage_incomplete",
        ),
        (
            SelectionDisabledReason::IngressContractUnavailable,
            "ingress_contract_unavailable",
        ),
    ];

    for (reason, token) in fixtures {
        assert_eq!(reason.as_str(), token);
        assert_eq!(
            serde_json::to_string(&reason).expect("closed reason must serialize"),
            format!("\"{token}\"")
        );
        assert_eq!(
            serde_json::from_str::<SelectionDisabledReason>(&format!("\"{token}\""))
                .expect("canonical token must decode"),
            reason
        );
    }

    for rejected in [
        "\"selection_v2_activation_not_released\"",
        "\"trading_calendar_unknown\"",
        "\"TradingCalendarMissing\"",
        "\"\"",
        "null",
    ] {
        assert!(
            serde_json::from_str::<SelectionDisabledReason>(rejected).is_err(),
            "noncanonical disabled reason unexpectedly decoded: {rejected}"
        );
    }
}

#[test]
fn br193_calendar_conflicts_are_fatal_reviewed_absence_only_is_disabled() {
    let reviewed_absences = [
        (
            ReviewedCalendarPrerequisiteReason::TradingCalendarMissing,
            SelectionDisabledReason::TradingCalendarMissing,
        ),
        (
            ReviewedCalendarPrerequisiteReason::TradingCalendarUnverified,
            SelectionDisabledReason::TradingCalendarUnverified,
        ),
        (
            ReviewedCalendarPrerequisiteReason::TradingCalendarCoverageIncomplete,
            SelectionDisabledReason::TradingCalendarCoverageIncomplete,
        ),
    ];
    for (marker, expected) in reviewed_absences {
        assert_eq!(
            classify_calendar_authority_presence(&CalendarAuthorityPresence::reviewed_absence(
                marker
            ))
            .expect("one reviewed wholly-unclaimed absence is disabled"),
            CalendarAuthorityClassification::Disabled(expected)
        );
    }

    for conflict in [
        CalendarAuthorityPresence::unreviewed_absence(),
        CalendarAuthorityPresence::multiple_reviewed_absence_markers(),
        CalendarAuthorityPresence::partial_authority(),
        CalendarAuthorityPresence::absent_with_activation_claim(),
        CalendarAuthorityPresence::complete_authority_with_marker(),
    ] {
        assert_eq!(
            classify_calendar_authority_presence(&conflict)
                .expect_err("conflicting calendar evidence must be fatal")
                .code(),
            "calendar_release_integrity_conflict"
        );
    }

    assert_eq!(
        classify_calendar_authority_presence(&CalendarAuthorityPresence::complete_authority())
            .expect("complete authority proceeds to strict descriptor/payload verification"),
        CalendarAuthorityClassification::Claimed
    );
}

#[test]
fn br193_calendar_release_prerequisite_marker_is_closed() {
    assert_eq!(
        RELEASE_PREREQUISITE_RELATIVE_PATH,
        "config/selection/a_share_trading_calendar_release_prerequisite.v1.json"
    );
    let canonical = "{\"domain\":\"stock_analysis.a_share_trading_calendar_release_prerequisite.v1\",\"schema_version\":1,\"reason_code\":\"trading_calendar_missing\",\"reviewed_at\":\"2026-07-30T12:00:00.000000000+08:00\",\"executable_revision\":\"660902ff93a07f18367dc16879cf67732accd25a\"}";
    assert_eq!(
        parse_release_prerequisite_canonical(canonical.as_bytes())
            .expect("canonical reviewed marker")
            .reason(),
        &ReviewedCalendarPrerequisiteReason::TradingCalendarMissing
    );

    for invalid in [
        canonical.replace(
            "\"trading_calendar_missing\"",
            "\"trading_calendar_unknown\"",
        ),
        canonical.replacen("\"schema_version\":1", "\"schema_version\":2", 1),
        canonical.replacen(
            "\"schema_version\":1",
            "\"schema_version\":1,\"alternate_path\":true",
            1,
        ),
        canonical.replacen(
            "{\"domain\":\"stock_analysis.a_share_trading_calendar_release_prerequisite.v1\",\"schema_version\":1",
            "{\"schema_version\":1,\"domain\":\"stock_analysis.a_share_trading_calendar_release_prerequisite.v1\"",
            1,
        ),
    ] {
        assert!(
            parse_release_prerequisite_canonical(invalid.as_bytes()).is_err(),
            "invalid or noncanonical marker unexpectedly admitted: {invalid}"
        );
    }
}
