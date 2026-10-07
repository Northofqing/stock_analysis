//! Ordinary-cfg lower mechanics, using the actual persistent test borrower.
//! Fixtures are owned inputs, not SQL origin, layout/provider or identity proof.
use super::paper_replay_financial_work_v1::{
    self as fw, FinancialFailure, FinancialWork, RawRowFrame,
};
use crate::database::global_schema_v1::replay_work::{self as work, ReplayTerminalFailure};
use chrono::{NaiveDate, TimeZone, Utc};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Case {
    Boundary(Boundary),
    Qualification,
    FloatSink,
    RawErrorShort,
    RawCurrent,
    RawUnused,
    RawExtras,
    RawNumbers,
    RawDepth,
    RawUnicode,
    RawPositions,
    RawRoots,
    RawExact,
    RawShort,
    Cumulative,
    Identity,
    SourceLower,
    SourceErrors,
    SourceBoundaries,
    AuditKnown,
    OriginalReplay,
    Gen1Double,
    Adjudication,
    LegacyFifo,
    LegacyHistory,
    Serialization,
    CloneOrder,
    Chrono,
    HashDuplicates,
    SortBranches,
    QueueGrowth,
}

#[test]
fn history_ordinary_profile_refuses_before_requests() {
    work::history_fixture(Case::Qualification);
}
#[test]
fn history_raw_float_sink_overflow_latches_before_error_output() {
    work::history_fixture(Case::FloatSink);
}
#[test]
fn history_raw_error_output_denial_precedes_owned_destination() {
    work::history_fixture(Case::RawErrorShort);
}
#[test]
fn history_raw_current_owner_comparison_and_hash_match() {
    work::history_fixture(Case::RawCurrent);
}
#[test]
fn history_raw_unused_nested_values_are_fully_validated() {
    work::history_fixture(Case::RawUnused);
}
#[test]
fn history_raw_extra_slots_validate_and_preserve_length() {
    work::history_fixture(Case::RawExtras);
}
#[test]
fn history_raw_default_numbers_match_bits_and_unsigned_kind() {
    work::history_fixture(Case::RawNumbers);
}
#[test]
fn history_raw_depth_127_and_128_match_default_recursion() {
    work::history_fixture(Case::RawDepth);
}
#[test]
fn history_raw_unicode_escapes_and_lone_surrogates_match() {
    work::history_fixture(Case::RawUnicode);
}
#[test]
fn history_raw_error_positions_and_trailing_data_match() {
    work::history_fixture(Case::RawPositions);
}
#[test]
fn history_raw_wrong_root_errors_match_without_object_walk() {
    work::history_fixture(Case::RawRoots);
}
#[test]
fn history_raw_exact_limit_copies_then_next_request_and_retry_refuse() {
    work::history_fixture(Case::RawExact);
}
#[test]
fn history_raw_one_short_refuses_before_destination_entry() {
    work::history_fixture(Case::RawShort);
}
#[test]
fn history_raw_repeated_prepare_render_owned_copies_accumulate() {
    work::history_fixture(Case::Cumulative);
}
#[test]
fn history_source_missing_identity_is_first_sticky_terminal() {
    work::history_fixture(Case::Identity);
}
#[test]
fn history_source_lower_nonempty_terminal_carry_and_double_hash_match() {
    work::history_fixture(Case::SourceLower);
}
#[test]
fn history_source_lower_first_errors_and_fixed_shanghai_dates_match() {
    work::history_fixture(Case::SourceErrors);
}
#[test]
fn history_source_empty_repudiated_duplicate_and_audit_water_boundaries_match() {
    work::history_fixture(Case::SourceBoundaries);
}
#[test]
fn history_audit_known_three_box_error_and_ledger_category_match() {
    work::history_fixture(Case::AuditKnown);
}
#[test]
fn history_original_nonempty_genesis_mark_head_matches_historical() {
    work::history_fixture(Case::OriginalReplay);
}
#[test]
fn history_gen1_repeated_owner_passes_accumulate_without_reset() {
    work::history_fixture(Case::Gen1Double);
}
#[test]
fn history_adjudication_marked_duplicate_keys_and_quarantine_match() {
    work::history_fixture(Case::Adjudication);
}
#[test]
fn history_legacy_fifo_partial_sell_t_plus_one_and_order_match() {
    work::history_fixture(Case::LegacyFifo);
}
#[test]
fn history_legacy_raw_chain_lineage_correction_and_result_match() {
    work::history_fixture(Case::LegacyHistory);
}
#[test]
fn history_fixed_outputs_and_raw_hashes_preserve_original_bytes() {
    work::history_fixture(Case::Serialization);
}
#[test]
fn history_nontrusted_clone_before_backing_and_trusted_backing_first() {
    work::history_fixture(Case::CloneOrder);
}
#[test]
fn history_hash_duplicate_full_insert_and_occupied_terminal_entry_differ() {
    work::history_fixture(Case::HashDuplicates);
}
#[test]
fn history_stable_sort_empty_small_and_heap_envelope_preserve_order() {
    work::history_fixture(Case::SortBranches);
}
#[test]
fn history_fifo_wrap_growth_and_later_pop_retain_cumulative_requests() {
    work::history_fixture(Case::QueueGrowth);
}
#[test]
fn history_chrono_fixed_recipes_match_boundaries_and_fractional_widths() {
    work::history_fixture(Case::Chrono);
}

fn paid_row<'loan, 'pool>(
    raw: &str,
    work: FinancialWork<'loan, 'pool>,
) -> RawRowFrame<'loan, 'pool> {
    match RawRowFrame::fixture_copy(raw, work) {
        Ok(frame) => frame,
        Err((error, _)) => panic!("unexpected paid raw request refusal: {error:?}"),
    }
}
fn ledger_error(error: FinancialFailure) -> String {
    match error {
        FinancialFailure::Financial(super::paper_ledger::LedgerError::IntegrityFailure(text)) => {
            text
        }
        other => panic!("expected exact raw syntax error, got {other:?}"),
    }
}
fn raw_oracle<'loan, 'pool>(
    raw: &str,
    work: FinancialWork<'loan, 'pool>,
) -> FinancialWork<'loan, 'pool> {
    let original = serde_json::from_str::<Vec<serde_json::Value>>(raw);
    let mut frame = paid_row(raw, work);
    {
        let mut scan = frame.scan().unwrap();
        match original {
            Err(error) => assert_eq!(
                ledger_error(scan.require_decoded().unwrap_err()),
                error.to_string(),
                "{raw:?}"
            ),
            Ok(values) => {
                assert_eq!(scan.len().unwrap(), values.len());
                for (index, value) in values.iter().take(15).enumerate() {
                    assert_eq!(scan.is_null(index).unwrap(), value.is_null());
                    assert_eq!(scan.text(index).unwrap().as_deref(), value.as_str());
                    assert_eq!(
                        scan.number(index).unwrap().map(|n| n.as_f64().to_bits()),
                        value.as_f64().map(f64::to_bits)
                    );
                    assert_eq!(
                        scan.number(index).unwrap().and_then(|n| n.as_u64()),
                        value.as_u64()
                    );
                }
            }
        }
    }
    frame.finish()
}
fn resource(error: FinancialFailure) -> ReplayTerminalFailure {
    match error {
        FinancialFailure::Terminal(terminal @ ReplayTerminalFailure::Resource(_)) => terminal,
        other => panic!("expected resource failure: {other:?}"),
    }
}
fn retry_copy<'loan, 'pool>(
    work: FinancialWork<'loan, 'pool>,
    raw: &str,
) -> FinancialWork<'loan, 'pool> {
    let (first, work) = match RawRowFrame::fixture_copy(raw, work) {
        Err(pair) => pair,
        Ok(_) => panic!("expected pre-owned refusal"),
    };
    let first = resource(first);
    let used = work.history_used();
    let entries = work.history_entries();
    assert!(matches!(work.finish(), Err(FinancialFailure::Terminal(e)) if e == first));
    let (again, work) = match RawRowFrame::fixture_copy(raw, work) {
        Err(pair) => pair,
        Ok(_) => panic!("sticky raw retry reached owned copy"),
    };
    assert_eq!(resource(again), first);
    assert_eq!(work.history_used(), used);
    assert_eq!(work.history_entries(), entries);
    work
}

pub(crate) fn run<'loan, 'pool>(
    case: Case,
    mut work: FinancialWork<'loan, 'pool>,
) -> FinancialWork<'loan, 'pool> {
    match case {
        Case::Boundary(_) | Case::Qualification | Case::FloatSink => unreachable!(),
        Case::RawErrorShort => {
            let raw = format!("\"{}\"", "a".repeat(8 * 1024 * 1024));
            let expected = serde_json::from_str::<Vec<serde_json::Value>>(&raw)
                .unwrap_err()
                .to_string();
            let expected_used = (raw.len() + expected.len()) as u64;
            assert!(expected_used > 16 * 1024 * 1024);
            let mut frame = paid_row(&raw, work);
            let first = resource(frame.scan().unwrap().require_decoded().unwrap_err());
            let again = match frame.scan() {
                Err(error) => resource(error),
                Ok(_) => panic!("latched frame scan cannot succeed"),
            };
            assert_eq!(first, again);
            work = frame.finish();
            assert_eq!(work.history_used(), expected_used);
            assert_eq!(work.history_entries()[0], 1);
            assert_eq!(work.history_entries()[4], 0);
            assert!(
                matches!(work.finish(), Err(FinancialFailure::Terminal(error)) if error == first)
            );
        }
        Case::RawCurrent => work = super::paper_ledger::history_raw_fixture(work),
        Case::RawUnused => {
            for raw in [
                r#"[0,{"a":[true,null,{"x":false}],"a":4},[],"unused"]"#,
                r#"[0,{"a":[true,]},1]"#,
                r#"[0,{"a":1 "b":2}]"#,
            ] {
                work = raw_oracle(raw, work);
            }
        }
        Case::RawExtras => {
            for raw in [
                "[]",
                "[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,{\"x\":[]}]",
                "[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,{\"x\":] ]",
            ] {
                work = raw_oracle(raw, work);
            }
        }
        Case::RawNumbers => {
            for token in [
                "0",
                "-0",
                "-0.0",
                "1.234567890123456789",
                "18446744073709551615",
                "18446744073709551616",
                "-9223372036854775808",
                "-9223372036854775809",
                "1e308",
                "1e309",
                "0e99999999999999",
                "1e-99999999999999",
                "1.",
                "01",
                "1e+",
                "--1",
            ] {
                work = raw_oracle(&format!("[{token}]"), work);
            }
        }
        Case::RawDepth => {
            for depth in [126, 127, 128, 129] {
                work = raw_oracle(
                    &format!("{}0{}", "[".repeat(depth), "]".repeat(depth)),
                    work,
                );
            }
        }
        Case::RawUnicode => {
            for raw in [
                r#"["中文","\u4e2d\u6587","\ud83d\ude03","\"\\\/\b\f\n\r\t"]"#,
                r#"["\ud800"]"#,
                r#"["\udc00"]"#,
                r#"["\ud800\u0041"]"#,
                r#"["\u00xx"]"#,
                r#"["\x"]"#,
                "[\"a\u{0001}b\"]",
            ] {
                work = raw_oracle(raw, work);
            }
        }
        Case::RawPositions => {
            for raw in [
                "",
                " ",
                "[",
                "[\n1,\n]",
                "[true false]",
                "[\"x\"",
                "[]\nfalse",
                "[truX]",
                "[nul]",
                "[1e9999,0]",
                "[\n{\"x\":1,}\n]",
            ] {
                work = raw_oracle(raw, work);
            }
        }
        Case::RawRoots => {
            for raw in [
                "null",
                "true",
                "4",
                "-2.5",
                "-0.0",
                "1e308",
                "1e-300",
                "5e-324",
                "1.7976931348623157e308",
                "1e-6",
                "1e16",
                r#""中文\n""#,
                r#""a'\u0000\u0301\u2028\u200d\ud83d\ude03""#,
                "{\"bad\": [",
                "{}",
                "false trailing",
            ] {
                work = raw_oracle(raw, work);
            }
        }
        Case::RawExact => {
            let source = "x".repeat(16 * 1024 * 1024);
            let frame = paid_row(&source, work);
            assert_eq!(frame.copied_bytes(), source);
            work = frame.finish();
            assert_eq!(work.history_used(), 16 * 1024 * 1024);
            assert_eq!(work.history_entries()[0], 1);
            work = retry_copy(work, "x");
            assert_eq!(work.history_used(), 16 * 1024 * 1024 + 1);
            assert_eq!(work.history_entries()[0], 1);
        }
        Case::RawShort => {
            let source = "x".repeat(16 * 1024 * 1024 + 1);
            work = retry_copy(work, &source);
            assert_eq!(work.history_used(), source.len() as u64);
            assert_eq!(work.history_entries()[0], 0);
        }
        Case::Cumulative => {
            let source = "r".repeat(2 * 1024 * 1024);
            for pass in 0..8 {
                work = paid_row(&source, work).finish();
                assert_eq!(work.history_used(), (pass + 1) * source.len() as u64);
            }
            work = retry_copy(work, "r");
            assert_eq!(work.history_entries()[0], 8);
        }
        Case::Identity
        | Case::SourceLower
        | Case::SourceErrors
        | Case::SourceBoundaries
        | Case::CloneOrder
        | Case::HashDuplicates
        | Case::SortBranches => {
            crate::database::attribution_epochs::history_source_fixture(case, &mut work)
        }
        Case::AuditKnown => crate::database::order_audit::history_audit_fixture(&mut work),
        Case::OriginalReplay | Case::Gen1Double | Case::Adjudication | Case::Serialization => {
            super::paper_ledger::history_owner_fixture(case, &mut work)
        }
        Case::LegacyFifo => fifo_fixture(&mut work),
        Case::LegacyHistory => work = super::paper_ledger::history_legacy_fixture(work),
        Case::QueueGrowth => queue_fixture(&mut work),
        Case::Chrono => chrono_fixture(&mut work),
    }
    work
}

pub(crate) fn date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 24).unwrap()
}
pub(crate) fn at() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 24, 2, 0, 0).unwrap()
}
pub(crate) fn seed() -> super::paper_ledger::SeedManifest {
    use super::paper_ledger::*;
    SeedManifest {
        account_id: "TEST_CODE_HISTORY_ACCOUNT".into(),
        epoch_id: "TEST_CODE_HISTORY_EPOCH".into(),
        command_id: "TEST_CODE_HISTORY_SEED".into(),
        cutover_at: at(),
        account_effective_at: at(),
        positions_effective_at: at(),
        source_reference: "history lower fixture".into(),
        source_hash: "a".repeat(64),
        approved_by: "fixture".into(),
        cash: Money::from_micros(10_000_000_000),
        original_total: Money::from_micros(11_000_000_000),
        excluded_residual: None,
        lots: vec![SeedLot {
            code: "600001".into(),
            name: "原始持仓".into(),
            quantity: 100,
            reported_cost: None,
            sellable_from: Some(date()),
            sellability_evidence: Some("old lot".into()),
        }],
        marks: vec![Mark {
            code: "600001".into(),
            price: Money::from_micros(10_000_000),
            observed_at: at(),
            source: "original mark".into(),
        }],
        policy: RiskPolicyV1::default(),
    }
}
fn fifo_fixture(work: &mut FinancialWork<'_, '_>) {
    use crate::performance::economic_position::EconomicFillRow;
    let rows: Vec<EconomicFillRow> = [
        (1, "buy", 200, "2026-09-23 10:00:00"),
        (2, "buy", 100, "2026-09-23 11:00:00"),
        (3, "sell", 100, "2026-09-24 10:00:00"),
    ]
    .into_iter()
    .map(|(id, direction, quantity, time)| EconomicFillRow {
        id,
        plan_id: format!("plan-{id}"),
        code: "600001".into(),
        name: "名字".into(),
        direction: direction.into(),
        fill_price: Some(10.0),
        quantity,
        occurred_at: time.into(),
        virtual_reason: "decision".into(),
    })
    .collect();
    let expected =
        crate::performance::attribution_epoch::build_legacy_carry(&rows, date()).unwrap();
    let actual =
        crate::performance::attribution_epoch::build_legacy_carry_body(&rows, date(), work)
            .unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual[0].quantity, 200);
    let mut fills = Vec::new();
    for row in &rows {
        fills.push(super::paper_lot_ledger::PaperFill {
            id: row.id,
            code: row.code.clone(),
            name: row.name.clone(),
            direction: row.direction.clone(),
            fill_price: row.fill_price,
            quantity: row.quantity,
            occurred_at: super::paper_lot_ledger::parse_paper_fill_timestamp(
                row.id,
                &row.occurred_at,
            )
            .unwrap(),
        });
    }
    let original = super::paper_lot_ledger::rebuild_paper_positions(&fills, date()).unwrap();
    let actual =
        super::paper_lot_ledger::rebuild_paper_positions_body(&fills, date(), work).unwrap();
    assert_eq!(actual, original);
    let mut invalid = fills.clone();
    invalid[2].occurred_at = invalid[1].occurred_at + chrono::Duration::seconds(1);
    let expected = super::paper_lot_ledger::rebuild_paper_positions(&invalid, date()).unwrap_err();
    assert!(expected.contains("T+1"));
    match super::paper_lot_ledger::rebuild_paper_positions_body(&invalid, date(), work).unwrap_err()
    {
        FinancialFailure::History(actual) => assert_eq!(actual, expected),
        other => panic!("{other:?}"),
    }
}
fn chrono_fixture(work: &mut FinancialWork<'_, '_>) {
    use fw::HistoryChrono as C;
    for raw in [
        "2026-09-24 10:00:00",
        "2026-09-24 10:00:00.123456789",
        "1900-01-01 00:00:00",
        "1991-09-15 01:59:59",
    ] {
        let value = chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f").unwrap();
        for request in [
            C::Whole(value),
            C::NaiveNanos(value),
            C::Date(value.date()),
            C::UtcMillis(value.and_utc()),
        ] {
            assert_eq!(work.history_time(request).unwrap(), request.historical());
        }
        let fixed = chrono::FixedOffset::east_opt(28_800)
            .unwrap()
            .from_utc_datetime(&value);
        assert_eq!(
            work.history_time(C::FixedNanos(fixed)).unwrap(),
            C::FixedNanos(fixed).historical()
        );
    }
    let raw = "escaped\\raw 中文\n";
    assert_eq!(
        work.raw_hash(raw.as_bytes()).unwrap(),
        hex::encode(Sha256::digest(raw.as_bytes()))
    );
}

fn queue_fixture(work: &mut FinancialWork<'_, '_>) {
    use super::paper_lot_ledger::{
        rebuild_paper_positions, rebuild_paper_positions_body, PaperFill,
    };
    let mut fills = Vec::new();
    for index in 0..48 {
        let (date, direction) = if index < 12 {
            (23, "buy")
        } else if index < 20 {
            (24, "sell")
        } else if index < 40 {
            (24, "buy")
        } else {
            (25, "sell")
        };
        fills.push(PaperFill {
            id: index + 1,
            code: "600001".into(),
            name: if index < 20 {
                "a".into()
            } else {
                "grown name containing 中文".into()
            },
            direction: direction.into(),
            fill_price: Some(10.0),
            quantity: 100,
            occurred_at: NaiveDate::from_ymd_opt(2026, 9, date)
                .unwrap()
                .and_hms_opt(10, 0, index as u32)
                .unwrap(),
        });
    }
    let as_of = NaiveDate::from_ymd_opt(2026, 9, 25).unwrap();
    let expected = rebuild_paper_positions(&fills, as_of).unwrap();
    let before = work.history_used();
    let actual = rebuild_paper_positions_body(&fills, as_of, work).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual[0].total_quantity, 1600);
    let first = work.history_used() - before;
    assert!(first > 0);
    assert_eq!(
        rebuild_paper_positions_body(&fills, as_of, work).unwrap(),
        expected
    );
    assert_eq!(work.history_used() - before, first * 2);
}

// Each fixed case is a separate invocation with its own genuine original
// test_borrow. Within a case, prefix/operation/retry share that one borrower.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Boundary {
    HashExact,
    HashShort,
    TerminalExact,
    TerminalTableShort,
    TerminalVectorShort,
    VectorNewExact,
    VectorNewShort,
    VectorExact,
    VectorShort,
    TreeExact,
    TreeShort,
    MarkedExact,
    MarkedShort,
    SortExact,
    SortShort,
    QueueExact,
    QueueShort,
    NameExact,
    NameShort,
    ChronoExact,
    ChronoOffsetShort,
    ChronoFormatterShort,
}

#[test]
fn history_full_duplicate_hash_independent_h_exact_short_and_sticky() {
    for case in [Boundary::HashExact, Boundary::HashShort] {
        work::history_fixture(Case::Boundary(case));
    }
}
#[test]
fn history_absent_terminal_table_then_nested_vec_independent_boundaries() {
    for case in [
        Boundary::TerminalExact,
        Boundary::TerminalTableShort,
        Boundary::TerminalVectorShort,
    ] {
        work::history_fixture(Case::Boundary(case));
    }
}
#[test]
fn history_vector_full_growth_independent_exact_short_and_sticky() {
    for case in [Boundary::VectorExact, Boundary::VectorShort] {
        work::history_fixture(Case::Boundary(case));
    }
}
#[test]
fn history_btree_vacant_node_independent_exact_short_and_sticky() {
    for case in [Boundary::TreeExact, Boundary::TreeShort] {
        work::history_fixture(Case::Boundary(case));
    }
}
#[test]
fn history_marked_collector_independent_exact_short_and_sticky() {
    for case in [Boundary::MarkedExact, Boundary::MarkedShort] {
        work::history_fixture(Case::Boundary(case));
    }
}
#[test]
fn history_heap_sort_independent_exact_short_and_sticky() {
    for case in [Boundary::SortExact, Boundary::SortShort] {
        work::history_fixture(Case::Boundary(case));
    }
}
#[test]
fn history_wrapped_queue_full_growth_independent_exact_short_and_sticky() {
    for case in [Boundary::QueueExact, Boundary::QueueShort] {
        work::history_fixture(Case::Boundary(case));
    }
}
#[test]
fn history_name_clone_from_full_growth_independent_exact_short_and_sticky() {
    for case in [Boundary::NameExact, Boundary::NameShort] {
        work::history_fixture(Case::Boundary(case));
    }
}
#[test]
fn history_fixed_nanos_offset_and_hidden_formatter_independent_boundaries() {
    for case in [
        Boundary::ChronoExact,
        Boundary::ChronoOffsetShort,
        Boundary::ChronoFormatterShort,
    ] {
        work::history_fixture(Case::Boundary(case));
    }
}

#[test]
fn history_trusted_vector_backing_independent_exact_short_and_sticky() {
    for case in [Boundary::VectorNewExact, Boundary::VectorNewShort] {
        work::history_fixture(Case::Boundary(case));
    }
}
