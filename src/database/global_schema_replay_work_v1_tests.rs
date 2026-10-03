use super::super::super::global_schema_catalog_v1::GlobalSchemaCatalogError;
use super::*;

fn old_detail(error: GlobalSchemaCatalogError) -> String {
    match error {
        GlobalSchemaCatalogError::CatalogMismatch { detail } => detail,
        other => panic!("unexpected catalog error: {other:?}"),
    }
}

#[test]
fn debit_preserves_zero_exact_limit_and_over_limit_attempt() {
    let mut primitive = RowsSpecWork::new(8, 1, 1);
    let mut historical = RowsSpecWork::new(8, 1, 1);
    for amount in [0, 3, 5] {
        assert_eq!(primitive.try_charge(amount), Ok(()));
        historical.charge(amount).unwrap();
        assert_eq!(primitive.used(), historical.used());
    }
    for (amount, expected_used) in [(1, 9), (0, 9), (2, 11)] {
        assert_eq!(
            primitive.try_charge(amount),
            Err(RowsSpecDebitFailure::Exceeded)
        );
        assert_eq!(
            old_detail(historical.charge(amount).unwrap_err()),
            "whole rows: metadata work exceeded"
        );
        assert_eq!(primitive.used(), expected_used);
        assert_eq!(historical.used(), expected_used);
    }
}

#[test]
fn debit_overflow_retains_prior_total_and_historical_error() {
    let mut primitive = RowsSpecWork::new(u64::MAX, 1, 1);
    let mut historical = RowsSpecWork::new(u64::MAX, 1, 1);
    primitive.try_charge(u64::MAX).unwrap();
    historical.charge(u64::MAX).unwrap();
    assert_eq!(primitive.try_charge(1), Err(RowsSpecDebitFailure::Overflow));
    assert_eq!(
        old_detail(historical.charge(1).unwrap_err()),
        "whole rows: metadata work overflow"
    );
    assert_eq!(primitive.used(), u64::MAX);
    assert_eq!(historical.used(), u64::MAX);
    // Historical behavior is deliberately NOT made sticky by the new primitive.
    assert_eq!(primitive.try_charge(0), Ok(()));
    historical.charge(0).unwrap();
}

#[test]
fn phases_reborrow_one_meter_without_reset_or_refund() {
    let mut metadata = RowsSpecWork::new(20, 1, 1);
    metadata.charge(2).unwrap(); // preexisting catalog work is retained
    {
        let mut work = BorrowedReplayWork::borrow(&mut metadata);
        {
            let prepare = &mut work;
            assert_eq!(
                prepare.reserve(ReplaySite::StateCopy, 5).unwrap().consume(),
                5
            );
            drop(prepare.reserve(ReplaySite::CodecScratch, 4).unwrap());
        }
        {
            let render = &mut work;
            assert_eq!(render.used(), 11);
            assert_eq!(
                render.reserve(ReplaySite::StateCopy, 9).unwrap().consume(),
                9
            );
        }
        work.finish().unwrap();
        assert_eq!(work.used(), 20);
    }
    assert_eq!(metadata.used(), 20);
}

#[test]
fn first_debit_failure_stays_sticky_across_phase_reborrows() {
    let mut metadata = RowsSpecWork::new(3, 1, 1);
    let mut work = BorrowedReplayWork::borrow(&mut metadata);
    let first = work.reserve(ReplaySite::Collection, 4).err().unwrap();
    assert_eq!(
        first,
        ReplayResourceFailure {
            site: ReplaySite::Collection,
            cause: ResourceCause::Debit(RowsSpecDebitFailure::Exceeded),
            used: 4,
        }
    );
    {
        let render = &mut work;
        assert_eq!(
            render.reserve(ReplaySite::ErrorStorage, 0).err(),
            Some(first)
        );
        assert_eq!(
            render.reserve(ReplaySite::Formatting, u64::MAX).err(),
            Some(first)
        );
    }
    assert_eq!(work.finish(), Err(first));
    assert_eq!(work.used(), 4);
}

#[test]
fn layout_failure_prevents_later_success_even_without_a_debit() {
    let mut metadata = RowsSpecWork::new(u64::MAX, 1, 1);
    let mut work = BorrowedReplayWork::borrow(&mut metadata);
    let mut reached_owned_operation = false;
    let result = work.reserve_array::<u64>(ReplaySite::SqlRows, u64::MAX);
    if result.is_ok() {
        reached_owned_operation = true;
    }
    let first = result.err().unwrap();
    assert!(!reached_owned_operation);
    assert!(matches!(first.cause, ResourceCause::Layout(_)));
    assert_eq!(work.used(), 0);
    assert_eq!(work.reserve(ReplaySite::SqlRows, 1).err(), Some(first));
    assert_eq!(work.finish(), Err(first));
}

#[test]
fn borrower_overflow_records_prior_used_and_cannot_be_cleared() {
    let mut metadata = RowsSpecWork::new(u64::MAX, 1, 1);
    metadata.try_charge(u64::MAX).unwrap();
    let mut work = BorrowedReplayWork::borrow(&mut metadata);
    let failure = work.reserve(ReplaySite::Collection, 1).err().unwrap();
    assert_eq!(
        failure.cause,
        ResourceCause::Debit(RowsSpecDebitFailure::Overflow)
    );
    assert_eq!(failure.used, u64::MAX);
    assert_eq!(work.reserve(ReplaySite::Collection, 0).err(), Some(failure));
}

#[test]
fn checked_array_and_alignment_refuse_unrepresentable_layouts() {
    assert_eq!(exact_array_bytes::<u64>(3), Ok(24));
    assert_eq!(exact_array_bytes::<()>(usize::MAX as u64), Ok(0));
    assert!(exact_array_bytes::<u64>(u64::MAX).is_err());
    assert_eq!(
        exact_array_bytes::<u8>(isize::MAX as u64 + 1),
        Err(LayoutFailure::AddressSpace)
    );
    assert_eq!(round_up(13, 8), Ok(16));
    assert_eq!(round_up(1, 0), Err(LayoutFailure::InvalidAlignment));
    assert_eq!(round_up(1, 3), Err(LayoutFailure::InvalidAlignment));
    assert_eq!(round_up(u64::MAX, 8), Err(LayoutFailure::Overflow));
    assert_eq!(
        record_upper(&[FieldLayout { size: 1, align: 0 }]),
        Err(LayoutFailure::InvalidAlignment)
    );
    assert!(record_upper(
        &[FieldLayout {
            size: isize::MAX as u64,
            align: 1
        }; 3]
    )
    .is_err());
}

#[test]
fn record_envelope_covers_every_five_field_permutation() {
    // This tests the arithmetic proof for actual public field types. It does
    // not measure a synthetic private-node mirror or certify this toolchain.
    let mut fields = [
        FieldLayout::of::<Option<NonNull<()>>>(),
        FieldLayout::of::<u16>(),
        FieldLayout::of::<u16>(),
        FieldLayout::of::<[MaybeUninit<String>; 11]>(),
        FieldLayout::of::<[MaybeUninit<u8>; 11]>(),
    ];
    let bound = record_upper(&fields).unwrap();
    fn visit(fields: &mut [FieldLayout], start: usize, bound: FieldLayout, seen: &mut usize) {
        if start == fields.len() {
            let mut offset = 0;
            for f in fields.iter() {
                offset = round_up(offset, f.align).unwrap() + f.size;
            }
            let exact_for_order = round_up(offset, bound.align).unwrap();
            assert!(exact_for_order <= bound.bytes());
            assert_eq!(record_upper(fields).unwrap(), bound);
            *seen += 1;
        } else {
            for index in start..fields.len() {
                fields.swap(start, index);
                visit(fields, start + 1, bound, seen);
                fields.swap(start, index);
            }
        }
    }
    let mut seen = 0;
    visit(&mut fields, 0, bound, &mut seen);
    assert_eq!(seen, 120);
}

#[test]
fn source_formulas_do_not_grant_a_runtime_pin() {
    let (leaf, internal) = btree_node_bounds::<String, u64>().unwrap();
    assert!(internal.bytes() >= leaf.bytes() + 12 * size_of::<NonNull<()>>() as u64);
    assert!(
        serde_error_box_bound().unwrap().bytes()
            >= size_of::<Box<str>>() as u64 + 2 * size_of::<usize>() as u64
    );
    assert_eq!(
        require_reviewed_layout_pin(),
        Err(LayoutPinRefusal::BuildVerificationNotInstalled)
    );
}

#[test]
fn vector_and_sort_bounds_cover_growth_and_reject_overflow() {
    assert_eq!(amortized_vector_bytes::<u8>(0, 1), Ok(8));
    assert_eq!(amortized_vector_bytes::<u64>(0, 1), Ok(32));
    assert_eq!(amortized_vector_bytes::<u8>(8, 9), Ok(16));
    assert_eq!(amortized_vector_bytes::<u64>(8, 8), Ok(0));
    assert!(amortized_vector_bytes::<u64>(u64::MAX / 2 + 1, u64::MAX).is_err());
    assert_eq!(stable_sort_scratch_bytes::<u8>(100), Ok(100));
    assert_eq!(stable_sort_scratch_bytes::<u8>(20_000_001), Ok(10_000_001));
    assert_eq!(stable_sort_scratch_bytes::<()>(100), Ok(0));
    assert!(stable_sort_scratch_bytes::<u64>(u64::MAX).is_err());
}

#[test]
fn hash_envelope_covers_both_small_table_groups_and_growth() {
    assert_eq!(hash_table_bound::<u8>(0), Ok(0));
    assert_eq!(hash_table_bound::<u8>(1), Ok(48)); // 16 data + 16 controls + 16 tail
    assert_eq!(hash_table_bound::<u16>(1), Ok(40)); // 8*2 data + 8 controls + 16 tail
    assert_eq!(hash_table_bound::<u64>(1), Ok(52)); // 4*8 data + 4 controls + 16 tail
    assert_eq!(hash_table_bound::<u8>(15), Ok(80)); // 32 buckets + controls + tail
    assert!(hash_table_bound::<u64>(u64::MAX).is_err());
}

fn event(cells: [ScalarCellExtent; 6]) -> ScalarExtentRow {
    ScalarExtentRow {
        kind: SqlExtentKind::V1Event,
        arity: 6,
        cells,
    }
}
fn head() -> ScalarExtentRow {
    use ScalarCellExtent::{Null, TextBytes as T};
    ScalarExtentRow {
        kind: SqlExtentKind::V1Head,
        arity: 3,
        cells: [T(3), T(10), T(4), Null, Null, Null],
    }
}

#[test]
fn scalar_event_and_head_fold_exact_owned_text_extents() {
    use ScalarCellExtent::{Null, TextBytes as T};
    let mut events = ScalarExtentAccumulator::new(SqlExtentKind::V1Event);
    events
        .observe(event([T(1), T(2), T(3), T(8), Null, Null]))
        .unwrap();
    events
        .observe(event([T(0), T(2), T(3), T(9), T(4), T(5)]))
        .unwrap();
    assert_eq!(
        events.finish(),
        Ok(ScalarExtentSummary {
            rows: 2,
            text_bytes: 37,
            max_cell_bytes: 9
        })
    );
    let mut heads = ScalarExtentAccumulator::new(SqlExtentKind::V1Head);
    heads.observe(head()).unwrap();
    assert_eq!(
        heads.finish(),
        Ok(ScalarExtentSummary {
            rows: 1,
            text_bytes: 17,
            max_cell_bytes: 10
        })
    );
}

#[test]
fn scalar_wrong_kind_arity_and_nonempty_unused_slots_refuse() {
    let mut acc = ScalarExtentAccumulator::new(SqlExtentKind::V1Event);
    assert_eq!(acc.observe(head()), Err(ScalarExtentFailure::WrongKind));
    assert_eq!(acc.finish(), Err(ScalarExtentFailure::WrongKind));
    for row in [
        ScalarExtentRow { arity: 4, ..head() },
        ScalarExtentRow {
            cells: [ScalarCellExtent::TextBytes(1); 6],
            ..head()
        },
    ] {
        let mut acc = ScalarExtentAccumulator::new(SqlExtentKind::V1Head);
        assert_eq!(acc.observe(row), Err(ScalarExtentFailure::WrongArity));
        assert_eq!(acc.finish(), Err(ScalarExtentFailure::WrongArity));
    }
}

#[test]
fn scalar_type_null_and_negative_length_errors_cannot_be_hidden() {
    use ScalarCellExtent::{InvalidStorage, Null, TextBytes as T};
    for (bad, expected) in [
        (InvalidStorage, ScalarExtentFailure::InvalidStorage),
        (Null, ScalarExtentFailure::RequiredNull),
        (T(-1), ScalarExtentFailure::NegativeLength),
    ] {
        let mut acc = ScalarExtentAccumulator::new(SqlExtentKind::V1Head);
        let mut row = head();
        row.cells[1] = bad;
        assert_eq!(acc.observe(row), Err(expected));
        assert_eq!(acc.observe(head()), Err(expected));
        assert_eq!(acc.finish(), Err(expected));
    }
    // Nullable means SQL NULL is allowed, never a BLOB/non-text value.
    let mut acc = ScalarExtentAccumulator::new(SqlExtentKind::V1Event);
    assert_eq!(
        acc.observe(event([T(0), T(0), T(0), T(0), InvalidStorage, Null])),
        Err(ScalarExtentFailure::InvalidStorage)
    );
}

#[test]
fn scalar_byte_and_row_overflow_leave_no_successful_summary() {
    use ScalarCellExtent::{Null, TextBytes as T};
    let mut huge_row = ScalarExtentAccumulator::new(SqlExtentKind::V1Event);
    assert_eq!(
        huge_row.observe(event([T(i64::MAX), T(i64::MAX), T(2), T(0), Null, Null])),
        Err(ScalarExtentFailure::Overflow)
    );
    assert_eq!(huge_row.summary.rows, 0); // failing row commits none of its extents
    assert_eq!(huge_row.summary.text_bytes, 0);
    assert_eq!(huge_row.finish(), Err(ScalarExtentFailure::Overflow));
    for (rows, bytes) in [(u64::MAX, 0), (0, u64::MAX - 1)] {
        let mut acc = ScalarExtentAccumulator::new(SqlExtentKind::V1Head);
        acc.summary.rows = rows;
        acc.summary.text_bytes = bytes;
        assert_eq!(acc.observe(head()), Err(ScalarExtentFailure::Overflow));
        assert_eq!(acc.summary.rows, rows);
        assert_eq!(acc.summary.text_bytes, bytes);
        assert_eq!(acc.finish(), Err(ScalarExtentFailure::Overflow));
    }
}
