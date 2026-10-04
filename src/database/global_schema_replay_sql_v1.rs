//! Fixed V1 SQL mechanics below the real retained-reader loan. No connection
//! factory, general SQL input, layout certificate or financial interpretation.
use super::*;
use crate::trading::paper_ledger::{EventRow, HeadRow};
use rusqlite::{types::ValueRef, Row};

type Result<T> = std::result::Result<T, ReplayTerminalFailure>;
const EVENT_SCALARS: &str = "WITH selected AS (SELECT seq AS original_order_seq,CASE WHEN seq IS NULL THEN NULL ELSE CAST(seq AS INTEGER) END AS v0,CASE WHEN command_id IS NULL THEN NULL ELSE CAST(command_id AS TEXT) END AS v1,CASE WHEN previous_hash IS NULL THEN NULL ELSE CAST(previous_hash AS TEXT) END AS v2,CASE WHEN event_hash IS NULL THEN NULL ELSE CAST(event_hash AS TEXT) END AS v3,CASE WHEN payload IS NULL THEN NULL ELSE CAST(payload AS TEXT) END AS v4,CASE WHEN business_plan_id IS NULL THEN NULL ELSE CAST(business_plan_id AS TEXT) END AS v5,CASE WHEN intent_hash IS NULL THEN NULL ELSE CAST(intent_hash AS TEXT) END AS v6,CASE WHEN is_terminal IS NULL THEN NULL ELSE CAST(is_terminal AS INTEGER) END AS v7,CASE WHEN paper_trade_id IS NULL THEN NULL ELSE CAST(paper_trade_id AS INTEGER) END AS v8,CASE WHEN order_audit_id IS NULL THEN NULL ELSE CAST(order_audit_id AS INTEGER) END AS v9 FROM main.paper_ledger_event WHERE account_id=?1) SELECT CASE WHEN v1 IS NULL THEN -1 WHEN typeof(v1)='text' THEN length(CAST(v1 AS BLOB)) ELSE -2 END,CASE WHEN v2 IS NULL THEN -1 WHEN typeof(v2)='text' THEN length(CAST(v2 AS BLOB)) ELSE -2 END,CASE WHEN v3 IS NULL THEN -1 WHEN typeof(v3)='text' THEN length(CAST(v3 AS BLOB)) ELSE -2 END,CASE WHEN v4 IS NULL THEN -1 WHEN typeof(v4)='text' THEN length(CAST(v4 AS BLOB)) ELSE -2 END,CASE WHEN v5 IS NULL THEN -1 WHEN typeof(v5)='text' THEN length(CAST(v5 AS BLOB)) ELSE -2 END,CASE WHEN v6 IS NULL THEN -1 WHEN typeof(v6)='text' THEN length(CAST(v6 AS BLOB)) ELSE -2 END,CASE WHEN v0 IS NULL THEN 0 WHEN typeof(v0)='integer' THEN 1 ELSE -1 END,CASE WHEN v7 IS NULL THEN 0 WHEN typeof(v7)='integer' THEN 1 ELSE -1 END,CASE WHEN v8 IS NULL THEN 0 WHEN typeof(v8)='integer' THEN 1 ELSE -1 END,CASE WHEN v9 IS NULL THEN 0 WHEN typeof(v9)='integer' THEN 1 ELSE -1 END FROM selected ORDER BY original_order_seq";
const EVENT_VALUES: &str = "WITH selected AS (SELECT seq AS original_order_seq,CASE WHEN seq IS NULL THEN NULL ELSE CAST(seq AS INTEGER) END AS v0,CASE WHEN command_id IS NULL THEN NULL ELSE CAST(command_id AS TEXT) END AS v1,CASE WHEN previous_hash IS NULL THEN NULL ELSE CAST(previous_hash AS TEXT) END AS v2,CASE WHEN event_hash IS NULL THEN NULL ELSE CAST(event_hash AS TEXT) END AS v3,CASE WHEN payload IS NULL THEN NULL ELSE CAST(payload AS TEXT) END AS v4,CASE WHEN business_plan_id IS NULL THEN NULL ELSE CAST(business_plan_id AS TEXT) END AS v5,CASE WHEN intent_hash IS NULL THEN NULL ELSE CAST(intent_hash AS TEXT) END AS v6,CASE WHEN is_terminal IS NULL THEN NULL ELSE CAST(is_terminal AS INTEGER) END AS v7,CASE WHEN paper_trade_id IS NULL THEN NULL ELSE CAST(paper_trade_id AS INTEGER) END AS v8,CASE WHEN order_audit_id IS NULL THEN NULL ELSE CAST(order_audit_id AS INTEGER) END AS v9 FROM main.paper_ledger_event WHERE account_id=?1) SELECT v0,v1,v2,v3,v4,v5,v6,v7,v8,v9 FROM selected ORDER BY original_order_seq";
const HEAD_SCALARS: &str = "WITH selected AS (SELECT CASE WHEN version IS NULL THEN NULL ELSE CAST(version AS INTEGER) END AS v0,CASE WHEN event_hash IS NULL THEN NULL ELSE CAST(event_hash AS TEXT) END AS v1,CASE WHEN projection_bytes IS NULL THEN NULL ELSE CAST(projection_bytes AS TEXT) END AS v2,CASE WHEN projection_hash IS NULL THEN NULL ELSE CAST(projection_hash AS TEXT) END AS v3 FROM main.paper_ledger_head WHERE account_id=?1) SELECT CASE WHEN v1 IS NULL THEN -1 WHEN typeof(v1)='text' THEN length(CAST(v1 AS BLOB)) ELSE -2 END,CASE WHEN v2 IS NULL THEN -1 WHEN typeof(v2)='text' THEN length(CAST(v2 AS BLOB)) ELSE -2 END,CASE WHEN v3 IS NULL THEN -1 WHEN typeof(v3)='text' THEN length(CAST(v3 AS BLOB)) ELSE -2 END,CASE WHEN v0 IS NULL THEN 0 WHEN typeof(v0)='integer' THEN 1 ELSE -1 END FROM selected";
const HEAD_VALUES: &str = "WITH selected AS (SELECT CASE WHEN version IS NULL THEN NULL ELSE CAST(version AS INTEGER) END AS v0,CASE WHEN event_hash IS NULL THEN NULL ELSE CAST(event_hash AS TEXT) END AS v1,CASE WHEN projection_bytes IS NULL THEN NULL ELSE CAST(projection_bytes AS TEXT) END AS v2,CASE WHEN projection_hash IS NULL THEN NULL ELSE CAST(projection_hash AS TEXT) END AS v3 FROM main.paper_ledger_head WHERE account_id=?1) SELECT v0,v1,v2,v3 FROM selected";
const ENCODING: &str = "PRAGMA main.encoding";

pub(super) struct SqlCopyPlan<'id> {
    id: &'id str,
    kind: SqlExtentKind,
    summary: ScalarExtentSummary,
    connection: usize,
    meter: usize,
}
fn shape(work: &mut BorrowedReplayWork<'_>) -> ReplayTerminalFailure {
    work.qualify(ReplaySqlQualificationFailure::Shape)
}
fn arithmetic<T>(
    work: &mut BorrowedReplayWork<'_>,
    value: std::result::Result<T, LayoutFailure>,
) -> Result<T> {
    value.map_err(|cause| work.fail(ReplaySite::SqlRows, ResourceCause::Layout(cause)))
}
fn charge_query(work: &mut BorrowedReplayWork<'_>, sql: &'static str, id: &str) -> Result<()> {
    // Literal SQL is borrowed. Its second byte allowance covers the driver's
    // optional fixed-SQL error copy, NOT the unknown native error message.
    let bytes = mul(sql.len() as u64, 2).and_then(|n| add(n, id.len() as u64));
    let bytes = arithmetic(work, bytes)?;
    let fixed = (size_of::<rusqlite::Statement<'_>>()
        + size_of::<rusqlite::Rows<'_>>()
        + size_of::<SqlCopyPlan<'_>>()
        + size_of::<ScalarExtentRow>()
        + size_of::<&str>()) as u64;
    let bytes = arithmetic(work, add(bytes, fixed))?;
    work.reserve(ReplaySite::SqlRows, bytes)?.consume();
    if id.len() > i32::MAX as usize {
        return Err(shape(work));
    }
    Ok(())
}
fn step_work(work: &mut BorrowedReplayWork<'_>) -> Result<()> {
    work.reserve_array::<ScalarExtentRow>(ReplaySite::SqlRows, 1)?
        .consume();
    Ok(())
}
fn check_encoding(loan: &mut V1RowsLoan<'_>) -> Result<()> {
    charge_query(&mut loan.work, ENCODING, "")?;
    let connection = loan.connection;
    let mut statement = loan
        .work
        .sql(connection.prepare(ENCODING), SqlOperation::Prepare)?;
    {
        let mut rows = loan.work.sql(statement.query([]), SqlOperation::Bind)?;
        step_work(&mut loan.work)?;
        let row = loan.work.sql(rows.next(), SqlOperation::Step)?;
        let Some(row) = row else {
            return Err(shape(&mut loan.work));
        };
        let value = loan.work.sql(row.get_ref(0), SqlOperation::Encoding)?;
        if !matches!(value, ValueRef::Text(b) if b == b"UTF-8") {
            return Err(loan.work.qualify(ReplaySqlQualificationFailure::Encoding));
        }
        step_work(&mut loan.work)?;
        if loan.work.sql(rows.next(), SqlOperation::Step)?.is_some() {
            return Err(shape(&mut loan.work));
        }
    }
    loan.work.sql(statement.finalize(), SqlOperation::Finalize)
}
fn integer(row: &Row<'_>, index: usize, work: &mut BorrowedReplayWork<'_>) -> Result<i64> {
    match work.sql(row.get_ref(index), SqlOperation::Step)? {
        ValueRef::Integer(n) => Ok(n),
        _ => Err(shape(work)),
    }
}
pub(super) fn preflight<'id>(
    loan: &mut V1RowsLoan<'_>,
    id: &'id str,
    kind: SqlExtentKind,
) -> Result<SqlCopyPlan<'id>> {
    loan.work.finish()?;
    check_encoding(loan)?;
    let sql = match kind {
        SqlExtentKind::V1Event => EVENT_SCALARS,
        SqlExtentKind::V1Head => HEAD_SCALARS,
    };
    charge_query(&mut loan.work, sql, id)?;
    let connection = loan.connection;
    let mut statement = loan
        .work
        .sql(connection.prepare(sql), SqlOperation::Prepare)?;
    let mut accumulator = ScalarExtentAccumulator::new(kind);
    {
        let mut rows = loan.work.sql(statement.query([id]), SqlOperation::Bind)?;
        loop {
            step_work(&mut loan.work)?;
            let Some(row) = loan.work.sql(rows.next(), SqlOperation::Step)? else {
                break;
            };
            let arity = if kind == SqlExtentKind::V1Event { 6 } else { 3 };
            let mut cells = [ScalarCellExtent::Null; 6];
            for (index, cell) in cells[..arity].iter_mut().enumerate() {
                *cell = match integer(row, index, &mut loan.work)? {
                    -1 => ScalarCellExtent::Null,
                    n if n >= 0 => ScalarCellExtent::TextBytes(n),
                    _ => ScalarCellExtent::InvalidStorage,
                };
            }
            let flags: &[bool] = if kind == SqlExtentKind::V1Event {
                &[true, true, false, false]
            } else {
                &[true]
            };
            for (offset, required) in flags.iter().enumerate() {
                let flag = integer(row, arity + offset, &mut loan.work)?;
                if flag != 1 && !(!*required && flag == 0) {
                    return Err(shape(&mut loan.work));
                }
            }
            accumulator
                .observe(ScalarExtentRow {
                    kind,
                    arity: arity as u8,
                    cells,
                })
                .map_err(|failure| match failure {
                    ScalarExtentFailure::Overflow => loan.work.fail(
                        ReplaySite::SqlRows,
                        ResourceCause::Layout(LayoutFailure::Overflow),
                    ),
                    _ => shape(&mut loan.work),
                })?;
        }
    }
    loan.work
        .sql(statement.finalize(), SqlOperation::Finalize)?;
    let summary = accumulator.finish().map_err(|_| shape(&mut loan.work))?;
    if kind == SqlExtentKind::V1Head && summary.rows > 1 {
        return Err(shape(&mut loan.work));
    }
    loan.work.finish()?;
    Ok(SqlCopyPlan {
        id,
        kind,
        summary,
        connection: connection as *const _ as usize,
        meter: loan.work.metadata as *const _ as usize,
    })
}

struct View<'a> {
    text: [Option<&'a str>; 6],
    integers: [Option<i64>; 4],
    bytes: u64,
    max: u64,
}
fn view<'a>(
    row: &'a Row<'_>,
    kind: SqlExtentKind,
    work: &mut BorrowedReplayWork<'_>,
) -> Result<View<'a>> {
    let mut result = View {
        text: [None; 6],
        integers: [None; 4],
        bytes: 0,
        max: 0,
    };
    let count = if kind == SqlExtentKind::V1Event { 6 } else { 3 };
    for index in 0..count {
        let required = kind == SqlExtentKind::V1Head || index < 4;
        result.text[index] = match work.sql(row.get_ref(index + 1), SqlOperation::Step)? {
            ValueRef::Text(bytes) => Some(
                std::str::from_utf8(bytes)
                    .map_err(|_| work.qualify(ReplaySqlQualificationFailure::Utf8))?,
            ),
            ValueRef::Null if !required => None,
            _ => return Err(shape(work)),
        };
        let bytes = result.text[index].map_or(0, |s| s.len() as u64);
        result.bytes = arithmetic(work, add(result.bytes, bytes))?;
        result.max = result.max.max(bytes);
    }
    let indices: &[usize] = if kind == SqlExtentKind::V1Event {
        &[0, 7, 8, 9]
    } else {
        &[0]
    };
    for (offset, index) in indices.iter().enumerate() {
        result.integers[offset] = match work.sql(row.get_ref(*index), SqlOperation::Step)? {
            ValueRef::Integer(n) => Some(n),
            ValueRef::Null if offset >= 2 => None,
            _ => return Err(shape(work)),
        };
    }
    Ok(result)
}
fn copy_text(value: &str, work: &mut BorrowedReplayWork<'_>) -> Result<String> {
    // Bytes are prepaid by the consumed plan; this is the actual exact request.
    #[cfg(test)]
    {
        work.text_copy_requests += 1;
    }
    let mut text = String::new();
    text.try_reserve_exact(value.len())
        .map_err(|_| work.fail(ReplaySite::SqlRows, ResourceCause::AllocationFailed))?;
    text.push_str(value);
    Ok(text)
}
fn optional_text(value: Option<&str>, work: &mut BorrowedReplayWork<'_>) -> Result<Option<String>> {
    value.map(|value| copy_text(value, work)).transpose()
}
fn reserve_plan<T>(
    loan: &mut V1RowsLoan<'_>,
    plan: &SqlCopyPlan<'_>,
    kind: SqlExtentKind,
) -> Result<usize> {
    loan.work.finish()?;
    if plan.kind != kind
        || plan.connection != loan.connection as *const _ as usize
        || plan.meter != loan.work.metadata as *const _ as usize
    {
        return Err(loan.work.qualify(ReplaySqlQualificationFailure::Extent));
    }
    let count = usize::try_from(plan.summary.rows).map_err(|_| {
        loan.work.fail(
            ReplaySite::SqlRows,
            ResourceCause::Layout(LayoutFailure::AddressSpace),
        )
    })?;
    loan.work
        .reserve_array::<T>(ReplaySite::SqlRows, plan.summary.rows)?
        .consume();
    loan.work
        .reserve(ReplaySite::SqlRows, plan.summary.text_bytes)?
        .consume();
    Ok(count)
}
fn observe_value(
    summary: &mut ScalarExtentSummary,
    value: &View<'_>,
    plan: &SqlCopyPlan<'_>,
    work: &mut BorrowedReplayWork<'_>,
) -> Result<()> {
    let rows = arithmetic(work, add(summary.rows, 1))?;
    let bytes = arithmetic(work, add(summary.text_bytes, value.bytes))?;
    let max = summary.max_cell_bytes.max(value.max);
    // All checks precede any row's String allocation, including unexpected EOF+1.
    if rows > plan.summary.rows
        || bytes > plan.summary.text_bytes
        || max > plan.summary.max_cell_bytes
    {
        return Err(work.qualify(ReplaySqlQualificationFailure::Extent));
    }
    *summary = ScalarExtentSummary {
        rows,
        text_bytes: bytes,
        max_cell_bytes: max,
    };
    Ok(())
}
fn empty_summary() -> ScalarExtentSummary {
    ScalarExtentSummary {
        rows: 0,
        text_bytes: 0,
        max_cell_bytes: 0,
    }
}
fn complete(
    observed: ScalarExtentSummary,
    plan: &SqlCopyPlan<'_>,
    work: &mut BorrowedReplayWork<'_>,
) -> Result<()> {
    if observed != plan.summary {
        return Err(work.qualify(ReplaySqlQualificationFailure::Extent));
    }
    work.finish()
}

pub(super) fn events(loan: &mut V1RowsLoan<'_>, plan: SqlCopyPlan<'_>) -> Result<Vec<EventRow>> {
    let count = reserve_plan::<EventRow>(loan, &plan, SqlExtentKind::V1Event)?;
    #[cfg(test)]
    {
        loan.work.row_buffer_requests += 1;
    }
    let mut result = Vec::new();
    result.try_reserve_exact(count).map_err(|_| {
        loan.work
            .fail(ReplaySite::SqlRows, ResourceCause::AllocationFailed)
    })?;
    charge_query(&mut loan.work, EVENT_VALUES, plan.id)?;
    let connection = loan.connection;
    let mut statement = loan
        .work
        .sql(connection.prepare(EVENT_VALUES), SqlOperation::Prepare)?;
    let mut observed = empty_summary();
    {
        let mut rows = loan
            .work
            .sql(statement.query([plan.id]), SqlOperation::Bind)?;
        loop {
            step_work(&mut loan.work)?;
            let Some(row) = loan.work.sql(rows.next(), SqlOperation::Step)? else {
                break;
            };
            let value = view(row, SqlExtentKind::V1Event, &mut loan.work)?;
            observe_value(&mut observed, &value, &plan, &mut loan.work)?;
            let text = value.text;
            let ints = value.integers;
            let event = EventRow::from_bounded_sql_parts(
                ints[0].expect("required integer checked"),
                copy_text(text[0].expect("required text checked"), &mut loan.work)?,
                copy_text(text[1].expect("required text checked"), &mut loan.work)?,
                copy_text(text[2].expect("required text checked"), &mut loan.work)?,
                copy_text(text[3].expect("required text checked"), &mut loan.work)?,
                optional_text(text[4], &mut loan.work)?,
                optional_text(text[5], &mut loan.work)?,
                ints[1].expect("required integer checked"),
                ints[2],
                ints[3],
            );
            if result.len() >= count {
                return Err(loan.work.qualify(ReplaySqlQualificationFailure::Extent));
            }
            result.push(event);
        }
    }
    loan.work
        .sql(statement.finalize(), SqlOperation::Finalize)?;
    complete(observed, &plan, &mut loan.work)?;
    Ok(result)
}
pub(super) fn head(loan: &mut V1RowsLoan<'_>, plan: SqlCopyPlan<'_>) -> Result<Option<HeadRow>> {
    reserve_plan::<HeadRow>(loan, &plan, SqlExtentKind::V1Head)?;
    charge_query(&mut loan.work, HEAD_VALUES, plan.id)?;
    let connection = loan.connection;
    let mut statement = loan
        .work
        .sql(connection.prepare(HEAD_VALUES), SqlOperation::Prepare)?;
    let mut result = None;
    let mut observed = empty_summary();
    {
        let mut rows = loan
            .work
            .sql(statement.query([plan.id]), SqlOperation::Bind)?;
        loop {
            step_work(&mut loan.work)?;
            let Some(row) = loan.work.sql(rows.next(), SqlOperation::Step)? else {
                break;
            };
            let value = view(row, SqlExtentKind::V1Head, &mut loan.work)?;
            observe_value(&mut observed, &value, &plan, &mut loan.work)?;
            result = Some(HeadRow::from_bounded_sql_parts(
                value.integers[0].expect("required integer checked"),
                copy_text(
                    value.text[0].expect("required text checked"),
                    &mut loan.work,
                )?,
                copy_text(
                    value.text[1].expect("required text checked"),
                    &mut loan.work,
                )?,
                copy_text(
                    value.text[2].expect("required text checked"),
                    &mut loan.work,
                )?,
            ));
        }
    }
    loan.work
        .sql(statement.finalize(), SqlOperation::Finalize)?;
    complete(observed, &plan, &mut loan.work)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::super::super::target::{test_replay_owner, test_with_retained_v1_reader};
    use super::*;
    use diesel::Connection as _;
    use rusqlite::Connection;
    const BUDGET: u64 = 16 * 1024 * 1024;

    fn fixture(sql: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v1.sqlite");
        let db = Connection::open(&path).unwrap();
        // Deliberately flexible affinity exercises the old driver's converted
        // DTO contract. This is NOT a full catalog/financial qualification.
        db.execute_batch("CREATE TABLE paper_ledger_event(account_id TEXT,seq,command_id,previous_hash,event_hash,payload,business_plan_id,intent_hash,is_terminal,paper_trade_id,order_audit_id); CREATE TABLE paper_ledger_head(account_id TEXT,version,event_hash,projection_bytes,projection_hash);").unwrap();
        db.execute_batch(sql).unwrap();
        db.close().unwrap();
        (dir, path)
    }
    const VALID: &str = "INSERT INTO paper_ledger_event VALUES('a',1,'c','p','h','汉字',NULL,'',1,NULL,7); INSERT INTO paper_ledger_head VALUES('a',1,'h','{}','ph');";

    #[test]
    fn real_sql_scalar_extents_include_utf8_nul_null_and_exact_eof() {
        let (_dir, path) = fixture("INSERT INTO paper_ledger_event VALUES('a',1,'c','p','h',CAST(X'E6B18900E5AD97' AS TEXT),NULL,'',1,NULL,7);");
        let mut owner = test_replay_owner(BUDGET);
        test_with_retained_v1_reader(&mut owner, &path, |loan| {
            let plan = preflight(loan, "a", SqlExtentKind::V1Event)?;
            assert_eq!(
                plan.summary,
                ScalarExtentSummary {
                    rows: 1,
                    text_bytes: 10,
                    max_cell_bytes: 7
                }
            );
            assert_eq!(loan.work.text_copy_requests, 0);
            let empty = preflight(loan, "missing", SqlExtentKind::V1Head)?;
            assert_eq!(empty.summary, empty_summary());
            assert!(head(loan, empty)?.is_none()); // private lower path, no fake pin
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn lower_materializer_matches_actual_historical_diesel_coercions() {
        let (_dir, path) = fixture("INSERT INTO paper_ledger_event VALUES('a',1.9,42,3.5,X'68',CAST(X'E6B18900E5AD97' AS TEXT),NULL,'',1.7,'123e5',X'39'); INSERT INTO paper_ledger_event VALUES('a','2x','c','p','h','last',NULL,NULL,0,NULL,NULL); INSERT INTO paper_ledger_head VALUES('a','99x',5.25,X'7B7D',123);");
        let mut historical = diesel::SqliteConnection::establish(path.to_str().unwrap()).unwrap();
        let expected =
            crate::trading::paper_ledger::test_historical_v1_sql_rows(&mut historical, "a");
        let mut owner = test_replay_owner(BUDGET);
        let actual = test_with_retained_v1_reader(&mut owner, &path, |loan| {
            let events_plan = preflight(loan, "a", SqlExtentKind::V1Event)?;
            let events = events(loan, events_plan)?;
            let head_plan = preflight(loan, "a", SqlExtentKind::V1Head)?;
            Ok((events, head(loan, head_plan)?))
        })
        .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(actual.0.len(), 2); // no until filter: both rows are fetched
    }

    #[test]
    fn repeated_actual_read_loans_charge_same_persistent_owner() {
        let (_dir, path) = fixture(VALID);
        let mut owner = test_replay_owner(BUDGET);
        for round in 1..=2 {
            test_with_retained_v1_reader(&mut owner, &path, |loan| {
                let plan = preflight(loan, "a", SqlExtentKind::V1Event)?;
                assert_eq!(events(loan, plan)?.len(), 1);
                Ok(())
            })
            .unwrap();
            if round == 1 {
                assert!(owner.metadata_used() > 0);
            }
        }
        let twice = owner.metadata_used();
        let mut once = test_replay_owner(BUDGET);
        test_with_retained_v1_reader(&mut once, &path, |loan| {
            let plan = preflight(loan, "a", SqlExtentKind::V1Event)?;
            events(loan, plan)?;
            Ok(())
        })
        .unwrap();
        assert_eq!(twice, once.metadata_used() * 2);
    }

    #[test]
    fn production_owned_routes_refuse_missing_pin_without_any_dto_allocation() {
        let (_dir, path) = fixture(VALID);
        for use_head in [false, true] {
            let mut owner = test_replay_owner(BUDGET);
            let failure = test_with_retained_v1_reader(&mut owner, &path, |loan| {
                let failure = if use_head {
                    loan.head("a").err().unwrap()
                } else {
                    loan.events("a").err().unwrap()
                };
                assert_eq!(
                    failure,
                    ReplayTerminalFailure::Qualification(
                        ReplaySqlQualificationFailure::PinUnavailable
                    )
                );
                assert_eq!(
                    (loan.work.row_buffer_requests, loan.work.text_copy_requests),
                    (0, 0)
                );
                Ok(()) // deliberately swallowed: owner still refuses
            })
            .unwrap_err();
            assert_eq!(owner.require_replay_clear(), Err(failure));
        }
    }

    #[test]
    fn budget_boundaries_stop_query_and_owned_requests_before_allocation() {
        let (_dir, path) = fixture(VALID);
        let mut tiny = test_replay_owner(0);
        assert!(test_with_retained_v1_reader(&mut tiny, &path, |loan| {
            preflight(loan, "a", SqlExtentKind::V1Event)?;
            Ok(())
        })
        .is_err());
        let mut measured = test_replay_owner(BUDGET);
        let plan_bytes = test_with_retained_v1_reader(&mut measured, &path, |loan| {
            let plan = preflight(loan, "a", SqlExtentKind::V1Event)?;
            Ok(exact_array_bytes::<EventRow>(plan.summary.rows).unwrap() + plan.summary.text_bytes)
        })
        .unwrap();
        let mut short = test_replay_owner(measured.metadata_used() + plan_bytes - 1);
        assert!(test_with_retained_v1_reader(&mut short, &path, |loan| {
            let plan = preflight(loan, "a", SqlExtentKind::V1Event)?;
            assert!(events(loan, plan).is_err());
            assert_eq!(
                (loan.work.row_buffer_requests, loan.work.text_copy_requests),
                (0, 0)
            );
            Ok(())
        })
        .is_err());
    }

    #[test]
    fn exact_cumulative_budget_succeeds_and_one_less_cannot_return_rows() {
        let (_dir, path) = fixture(VALID);
        let read = |loan: &mut V1RowsLoan<'_>| {
            let plan = preflight(loan, "a", SqlExtentKind::V1Event)?;
            events(loan, plan)
        };
        let mut measured = test_replay_owner(BUDGET);
        assert_eq!(
            test_with_retained_v1_reader(&mut measured, &path, read)
                .unwrap()
                .len(),
            1
        );
        let required = measured.metadata_used();
        let mut exact = test_replay_owner(required);
        assert_eq!(
            test_with_retained_v1_reader(&mut exact, &path, read)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(exact.metadata_used(), required);
        let mut short = test_replay_owner(required - 1);
        let failure = test_with_retained_v1_reader(&mut short, &path, read).unwrap_err();
        assert!(matches!(failure, ReplayTerminalFailure::Resource(_)));
        assert_eq!(short.require_replay_clear(), Err(failure));
    }

    #[test]
    fn plan_identity_is_checked_before_owned_allocation() {
        let (_dir, path) = fixture(VALID);
        for wrong_connection in [true, false] {
            let mut owner = test_replay_owner(BUDGET);
            assert!(test_with_retained_v1_reader(&mut owner, &path, |loan| {
                let mut plan = preflight(loan, "a", SqlExtentKind::V1Event)?;
                if wrong_connection {
                    plan.connection ^= 1;
                } else {
                    plan.meter ^= 1;
                }
                assert!(events(loan, plan).is_err());
                assert_eq!(
                    (loan.work.row_buffer_requests, loan.work.text_copy_requests),
                    (0, 0)
                );
                Ok(())
            })
            .is_err());
        }
    }

    #[test]
    fn mismatched_plan_refuses_extra_row_or_bytes_before_copy() {
        let (_dir, path) = fixture(VALID);
        for row_short in [true, false] {
            let mut owner = test_replay_owner(BUDGET);
            assert!(test_with_retained_v1_reader(&mut owner, &path, |loan| {
                let mut plan = preflight(loan, "a", SqlExtentKind::V1Event)?;
                if row_short {
                    plan.summary.rows = 0;
                } else {
                    plan.summary.text_bytes -= 1;
                }
                assert!(events(loan, plan).is_err());
                assert_eq!(loan.work.text_copy_requests, 0);
                Ok(())
            })
            .is_err());
        }
    }

    #[test]
    fn exact_eof_rejects_short_stream_without_returning_partial_rows() {
        let (_dir, path) = fixture(VALID);
        let mut owner = test_replay_owner(BUDGET);
        assert!(test_with_retained_v1_reader(&mut owner, &path, |loan| {
            let mut plan = preflight(loan, "a", SqlExtentKind::V1Event)?;
            plan.summary.rows += 1;
            assert!(matches!(
                events(loan, plan),
                Err(ReplayTerminalFailure::Qualification(
                    ReplaySqlQualificationFailure::Extent
                ))
            ));
            Ok(())
        })
        .is_err());
    }

    #[test]
    fn null_required_duplicate_head_and_invalid_utf8_refuse() {
        for sql in [
            "INSERT INTO paper_ledger_event VALUES('a',NULL,'c','p','h','p',NULL,NULL,0,NULL,NULL);",
            "INSERT INTO paper_ledger_event VALUES('a',1,NULL,'p','h','p',NULL,NULL,0,NULL,NULL);",
            "INSERT INTO paper_ledger_head VALUES('a',1,'h','{}','p'),('a',2,'h','{}','p');",
            "INSERT INTO paper_ledger_event VALUES('a',1,'c','p','h',X'80',NULL,NULL,0,NULL,NULL);",
        ] {
            let (_dir, path) = fixture(sql);
            let mut owner = test_replay_owner(BUDGET);
            assert!(test_with_retained_v1_reader(&mut owner, &path, |loan| {
                let kind = if sql.contains("head") { SqlExtentKind::V1Head } else { SqlExtentKind::V1Event };
                let plan = preflight(loan, "a", kind)?;
                if kind == SqlExtentKind::V1Event { events(loan, plan)?; } else { head(loan, plan)?; }
                Ok(())
            }).is_err());
        }
    }

    #[test]
    fn driver_prepare_error_is_sticky_after_swallow_move_and_outer_mapping() {
        let (_dir, path) = fixture("DROP TABLE paper_ledger_event;");
        let mut owner = test_replay_owner(BUDGET);
        let first = test_with_retained_v1_reader(&mut owner, &path, |loan| {
            assert!(preflight(loan, "a", SqlExtentKind::V1Event).is_err());
            Ok(())
        })
        .unwrap_err();
        assert!(matches!(
            first,
            ReplayTerminalFailure::Qualification(ReplaySqlQualificationFailure::Sqlite {
                operation: SqlOperation::Prepare,
                ..
            })
        ));
        let mut moved_owner = owner;
        let mut entered = false;
        assert_eq!(
            test_with_retained_v1_reader(&mut moved_owner, &path, |_| {
                entered = true;
                Ok(())
            }),
            Err(first)
        );
        assert!(!entered);
        let error = moved_owner.metadata(0).unwrap_err();
        assert!(
            matches!(&error, super::super::super::GlobalSchemaV1Error::ReplayTerminal(failure) if *failure == first)
        );
        assert_eq!(error.code(), "global_schema_replay_terminal");
    }

    #[test]
    fn explicit_callback_failure_and_finalize_adapter_failure_cannot_be_dropped() {
        let (_dir, path) = fixture(VALID);
        for operation in [
            SqlOperation::Bind,
            SqlOperation::Step,
            SqlOperation::Finalize,
        ] {
            let mut owner = test_replay_owner(BUDGET);
            let first = test_with_retained_v1_reader(&mut owner, &path, |loan| {
                // Controlled error seam; not a claim of a native close failure.
                let _ = loan
                    .work
                    .sql::<()>(Err(rusqlite::Error::InvalidQuery), operation);
                Ok(())
            })
            .unwrap_err();
            assert!(
                matches!(first, ReplayTerminalFailure::Qualification(ReplaySqlQualificationFailure::Sqlite { operation: actual, .. }) if actual == operation)
            );
            assert_eq!(owner.require_replay_clear(), Err(first));
        }
        let mut owner = test_replay_owner(BUDGET);
        let failure = ReplayTerminalFailure::Qualification(ReplaySqlQualificationFailure::Extent);
        assert_eq!(
            test_with_retained_v1_reader(&mut owner, &path, |_| Err::<(), _>(failure)),
            Err(failure)
        );
        assert_eq!(owner.require_replay_clear(), Err(failure));
    }
}


// The new financial-source route has no native entry/issuer in this slice.
// These fixed places/loans make no selected-provider or release-success claim.
#[allow(dead_code)]
pub(super) mod original_native {
    use super::super::super::rows::original_source::{OriginalSourceWork, SourceOperationError, SourceTerminal};
    use rusqlite::ffi::{sqlite3, sqlite3_stmt};
    use std::ffi::CString;
    use std::marker::PhantomData;
    use std::ptr::NonNull;
    use std::rc::Rc;

    #[repr(u8)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Role { Original, Reference, Copied }
    #[repr(u8)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FixedAction {
        OriginalOpen, ExtendedResult, BusyTimeout, MaterializeCount, ProspectiveExtent, Pragmas, Integrity, ForeignKeyCheck,
        SourceId, CompileOptions, Catalog, ForeignKeys, IndexList, IndexXinfo,
        AttachedNames, TableCount, ReferenceDdl, Encoding, TempCheck,
        RowsExtent, TableShape, TableColumns, RowsPreflight, RowsStream,
        CopiedQueryOnly, SelectionReconciliation, Begin, Commit, Rollback,
        ConstructorClose, OriginalClose, ReferenceClose, CopiedClose,
    }
    #[repr(C, u8)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum CodeSlot { NotCalled, Called(i32) }
    #[derive(Clone, Copy)]
    struct ConnectionStatus {
        open: CodeSlot,
        extended_result: CodeSlot,
        busy_timeout: CodeSlot,
        close_first: CodeSlot,
        close_second: CodeSlot,
    }
    #[repr(u8)]
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum ConnectionPhase {
        OpenError, ConfigureExtended, ConfigureBusyTimeout, Configured,
        FirstCopiedCloseFailed,
    }
    struct RawConnection {
        db: NonNull<sqlite3>,
        name: CString,
        phase: ConnectionPhase,
        status: ConnectionStatus,
    }
    #[repr(C, u8)]
    enum OriginalPlace {
        Empty,
        NameReady { name: CString, status: ConnectionStatus },
        NoHandle { name: CString, status: ConnectionStatus },
        Handle(RawConnection),
        Released(ConnectionStatus),
        #[cfg(test)]
        ProtocolNoHandle(ConnectionStatus),
        #[cfg(test)]
        ProtocolHeld { phase: ConnectionPhase, status: ConnectionStatus },
    }
    #[repr(C, u8)]
    enum ConstructorPlace {
        NameReady { name: CString, status: ConnectionStatus },
        NoHandle { name: CString, status: ConnectionStatus },
        Handle(RawConnection),
        Released(ConnectionStatus),
    }
    #[repr(u8)]
    enum ReferenceGeneration { G1, G2, G3, G4, G5, G6, G7, G8 }
    #[repr(u8)]
    enum ReferencePhase { Legacy, Transitional, Final }
    struct ReferenceOccurrence {
        generation: ReferenceGeneration,
        phase: ReferencePhase,
    }
    #[repr(u8)]
    enum CopiedOccurrence {
        InitialPair, FinalPair, IssueFifth, RenderSixth,
        TargetCompareOne, TargetCompareTwo,
    }
    #[repr(C, u8)]
    enum AuxPlace {
        Empty,
        Reference { occurrence: ReferenceOccurrence, place: ConstructorPlace },
        Copied { occurrence: CopiedOccurrence, place: ConstructorPlace },
    }
    #[repr(u8)]
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum CursorPhase { NoCursor, Active, Ended }
    #[derive(Clone, Copy)]
    struct StmtState {
        role: Role,
        action: FixedAction,
        cursor: CursorPhase,
        step: CodeSlot,
        reset: CodeSlot,
        finalize: CodeSlot,
    }
    #[repr(C, u8)]
    enum StmtSlot {
        Vacant,
        Live { stmt: NonNull<sqlite3_stmt>, state: StmtState },
        Finalized(StmtState),
        #[cfg(test)]
        ProtocolHeld(StmtState),
    }
    #[repr(u8)]
    enum StatementPhase { Empty, Single, Integrity, Catalog, Pair }
    #[repr(u8)]
    enum TxPhase { NotCreated, Active, Consuming, Finished }
    #[repr(u8)]
    enum TxExit { NotSelected, EarlyError, CommitConsume, FailedCommit }
    #[repr(C, u8)]
    enum AutocommitObservation { NotObserved, Observed(i32) }
    #[repr(u8)]
    enum RollbackObservation { NotReached, Reached }
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum BatchPhase { Dormant, Prepare, Step, Primary, Finalize, PaidFinalize, AwaitReturn, Finished }
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum BatchReturn { Unobserved, ReturnedOk, ReturnedError }
    struct FixedBatchRecord {
        phase: BatchPhase,
        prepare: CodeSlot,
        returned: BatchReturn,
    }
    impl FixedBatchRecord {
        fn empty() -> Self { Self { phase: BatchPhase::Dormant, prepare: CodeSlot::NotCalled, returned: BatchReturn::Unobserved } }
    }
    struct TxRecord {
        // Saved consumed scalar ledgers, never additional live VM slots.
        a00_consumed: Option<StmtState>,
        begin_consumed: Option<StmtState>,
        begin: FixedBatchRecord,
        rollback_batch: FixedBatchRecord,
        ignored_rollback: Option<SourceOperationError>,
        phase: TxPhase,
        exit: TxExit,
        autocommit: AutocommitObservation,
        rollback: RollbackObservation,
    }
    struct FixedAdverse {
        role: Role,
        action: FixedAction,
        ordinal: usize,
        code: i32,
    }
    pub(in crate::database::global_schema_v1) struct NativeOriginalOwner {
        original: OriginalPlace,
        aux: AuxPlace,
        statements: [StmtSlot; 3],
        statement_phase: StatementPhase,
        a00: A00Record,
        tx: TxRecord,
        initial_read: InitialReadRecord,
        integrity_read: IntegrityReadRecord,
        capture_prefix: CapturePrefixRecord,
        compile_options: CompileOptionsRecord,
        compile_sort: CompileSortRecord,
        secondary: Option<FixedAdverse>,
        _thread: PhantomData<Rc<()>>,
    }
    // No Connection/Statement/Rows/Transaction overlap, raw getter, from_raw,
    // live-handle constructor or default Drop is introduced. Real acquisition
    // and explicit qualified release remain the next behavior slice.
    impl NativeOriginalOwner {
        pub(in crate::database::global_schema_v1) fn from_start_decision(
            _decision: &super::super::super::FinancialRetainedStartDecision,
        ) -> Self {
            Self::empty()
        }
        fn empty() -> Self {
            Self {
                original: OriginalPlace::Empty,
                aux: AuxPlace::Empty,
                statements: [StmtSlot::Vacant, StmtSlot::Vacant, StmtSlot::Vacant],
                statement_phase: StatementPhase::Empty,
                a00: A00Record::empty(),
                tx: TxRecord {
                    a00_consumed: None, begin_consumed: None,
                    begin: FixedBatchRecord::empty(), rollback_batch: FixedBatchRecord::empty(),
                    ignored_rollback: None,
                    phase: TxPhase::NotCreated,
                    exit: TxExit::NotSelected,
                    autocommit: AutocommitObservation::NotObserved,
                    rollback: RollbackObservation::NotReached,
                },
                initial_read: InitialReadRecord::empty(),
                integrity_read: IntegrityReadRecord::empty(),
                capture_prefix: CapturePrefixRecord::empty(),
                compile_options: CompileOptionsRecord::empty(),
                compile_sort: CompileSortRecord::empty(),
                secondary: None,
                _thread: PhantomData,
            }
        }
    }

    impl NativeOriginalOwner {
        // Fixed same-frame cut only; it carries no native/FS rules. In
        // particular a first failed close cannot permit fresh acquisition.
        pub(in crate::database::global_schema_v1) fn audit_acquisition_ready(&self) -> bool {
            self.original.phase() == Some(ConnectionPhase::Configured)
                && self.original.status().is_some_and(|status| matches!(status.close_first, CodeSlot::NotCalled))
                && self.a00.phase == A00Phase::Complete
                && !self.statements.iter().any(|slot| slot.live().is_some())
                && matches!(&self.aux, AuxPlace::Empty)
                && matches!(&self.tx.phase, TxPhase::NotCreated)
        }
    }

    // Opaque declaration only: the absent issuer cannot create a rule value.
    // No operation here interprets this as a qualified provider or a gate.
    struct SelectedOriginalNativeRules { _issuance: std::convert::Infallible }
    struct OriginalSqlLoan<'n, 'w> {
        native: &'n mut NativeOriginalOwner,
        work: OriginalSourceWork<'w>,
        rules: &'n SelectedOriginalNativeRules,
    }
    impl OriginalSqlLoan<'_, '_> {
        fn reborrow(&mut self) -> OriginalSqlLoan<'_, '_> {
            OriginalSqlLoan {
                native: &mut *self.native,
                work: self.work.reborrow(),
                rules: self.rules,
            }
        }
    }

    // This executable protocol core owns no provider authority. Observations
    // are private, and no production method installs a native pointer. A later
    // genuine adapter must install its actual returned resource before calling
    // these status methods; SelectedOriginalNativeRules remains uninhabited.
    const A00_SQL: &str = "SELECT COUNT(*) FROM sqlite_schema";
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum LifecycleAction {
        NeedSelectedName, OpenOriginal, ExtendedResult, BusyTimeout,
        ConstructorComplete, PrepareA00, BeginA00Query, StepA00, ReadA00Integer,
        RetainPaidPrimary, DiscardPaidReset, DiscardPaidFinalize,
        ResetA00, FinalizeA00, A00Complete, ConstructorClose, OriginalClose,
        Quiescent, FatalBorrow, FinishSidecarAcquisition, DrainSidecarLocals, FinishAuditAcquisition, DrainAuditResources,
        PrepareBegin, StepBegin, FinalizeBegin, RetainTransactionPrimary,
        DiscardBatchFinalize, AwaitBatchReturn, CancelUnissuedBegin, ConsumeKnownBatchReturn,
        ConsumeEarlyTransaction, ObserveAutocommit, PrepareRollback, StepRollback,
        FinalizeRollback, RetainIgnoredRollback, DiscardIgnoredRollback,
        AwaitInitialReadContext, ValidateInitialOptions, RetainInitialPrimary, RetainInitialDriverError, WrapInitialDriverError, DiscardInitialDriverError, RequireZeroOwnedWal, BeforeInitialCapture,
        PrepareInitialRead, CheckInitialNoTail, BindInitialEmpty, StepInitialRead, InitialColumnType, InitialInteger, InitialJournalText,
        ResetInitialRead, FinalizeInitialRead, RetainInitialResetError, RetainInitialFinalizeError,
        DiscardInitialOwnedCleanup, AwaitInitialQueryReturn, InitialPrefixReached, StopInitialRead,
        PrepareIntegrityRead, BindIntegrityEmpty, StepIntegrityRead, IntegrityColumnType, IntegrityText,
        AwaitIntegrityRowReturn, RetainIntegrityRaw, ResetIntegrityRead, AwaitIntegrityWholeReturn,
        WrapIntegrityRaw, CheckIntegritySemantic, RetainIntegrityDetail, AwaitIntegrityDetailReturn,
        WrapIntegrityDetail, DiscardIntegrityOwnedDetail, FinalizeIntegrityRead,
        RetainIntegrityOwnedCleanup, DiscardIntegrityOwnedCleanup, DiscardIntegrityRaw,
        AwaitIntegrityPartialReturn, DiscardIntegrityOwnedVector, DiscardIntegrityPartial, StopIntegrityRead, IntegrityPrefixReached, AwaitIntegrityCaptureReturn,
        BeginCapturePragmaScope, PrepareCaptureRead, CheckCaptureNoTail, BindCaptureEmpty, StepCaptureRead,
        CaptureColumnType, CaptureInteger, CaptureText, AwaitCaptureString, RetainCaptureRaw,
        ResetCaptureRead, FinalizeCaptureRead, RetainCaptureCleanup, DiscardCaptureCleanup,
        AwaitCaptureQueryReturn, AwaitCapturePragmaScopeEnd, AwaitCapturePragmaReturn,
        FormatCaptureDetail, AwaitCaptureDetail, BuildCaptureCatalogError,
        AwaitCaptureRuntimeErrorReturn, AwaitCaptureCatalogReturn, WrapCaptureCatalogError,
        DiscardCaptureRaw, DiscardCaptureDetail, DiscardCaptureCatalogError, DiscardCaptureString,
        AdvanceCaptureQuery, CapturePrefixReached, StopCapturePrefix,
        PrepareCompileStatement, AwaitCompilePrepareObservation, AwaitCompilePrepareReturn,
        QueryCompileEmpty, AwaitCompileQueryObservation, AwaitCompileQueryReturn, AwaitCompileStepObservation,
        StepCompileRead, CompileColumnType, CompileText, AwaitCompileMapperReturn, RetainCompileRaw,
        ResetCompileRows, AwaitCompileRowsDrop, AwaitCompileCalleeScope, AwaitCompileCollectReturn,
        AwaitCompileVector, FormatCompileDetail, AwaitCompileDetail, BuildCompileCatalogError,
        FinalizeCompileStatement, AwaitCompileStatementDrop, AwaitCompileRuntimeReturn, AwaitCompileCatalogReturn,
        WrapCompileCatalogError, DiscardCompileRaw, DiscardCompileDetail, DiscardCompileVector,
        DiscardCompileCatalogError, DiscardCompileSourceId, StopCompileOptions, CompileOptionsBeforeSort,
        SortCompileOptions, AwaitCompileSortReturn, AwaitCompileSortLexicalReturn, CheckCompileDuplicates,
        AcquireCompileDuplicateError, AwaitCompileDuplicateOwner, AwaitCompileDuplicateReturn, AwaitCompileDigestSuccessor,
    }
    enum ConstructorObservation { Open(i32), Extended(i32), BusyTimeout(i32), Close(i32) }
    enum A00Observation { Prepare(i32), QueryStarted, Step(i32), Integer(i64), Reset(i32), Finalize(i32) }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ProtocolFault { UnexpectedObservation, RepeatedObservation, ResourceNotInstalled }
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum A00Phase {
        NotStarted, Prepare, Query, Step, Read, PrimaryPrepare, PrimaryStep,
        Reset, PrimaryNoRows, PrimaryReset, PaidReset, Finalize, PaidFinalize, Complete,
    }
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum A00Outcome { Unobserved, FirstRow, NoRow, StepError, ReadError }
    struct A00Record { phase: A00Phase, prepare: CodeSlot, outcome: A00Outcome }
    impl A00Record {
        fn empty() -> Self {
            Self { phase: A00Phase::NotStarted, prepare: CodeSlot::NotCalled, outcome: A00Outcome::Unobserved }
        }
    }
    fn record_once(slot: &mut CodeSlot, code: i32) -> Result<(), ProtocolFault> {
        if !matches!(slot, CodeSlot::NotCalled) { return Err(ProtocolFault::RepeatedObservation); }
        *slot = CodeSlot::Called(code);
        Ok(())
    }
    impl OriginalPlace {
        fn status(&self) -> Option<&ConnectionStatus> {
            match self {
                Self::NameReady { status, .. } | Self::NoHandle { status, .. } | Self::Released(status) => Some(status),
                Self::Handle(raw) => Some(&raw.status),
                #[cfg(test)]
                Self::ProtocolNoHandle(status) | Self::ProtocolHeld { status, .. } => Some(status),
                Self::Empty => None,
            }
        }
        fn status_mut(&mut self) -> Option<&mut ConnectionStatus> {
            match self {
                Self::NameReady { status, .. } | Self::NoHandle { status, .. } | Self::Released(status) => Some(status),
                Self::Handle(raw) => Some(&mut raw.status),
                #[cfg(test)]
                Self::ProtocolNoHandle(status) | Self::ProtocolHeld { status, .. } => Some(status),
                Self::Empty => None,
            }
        }
        fn phase(&self) -> Option<ConnectionPhase> {
            match self {
                Self::Handle(raw) => Some(raw.phase),
                #[cfg(test)]
                Self::ProtocolHeld { phase, .. } => Some(*phase),
                _ => None,
            }
        }
        fn no_handle(&self) -> bool {
            match self {
                Self::NoHandle { .. } => true,
                #[cfg(test)]
                Self::ProtocolNoHandle(_) => true,
                _ => false,
            }
        }
        fn set_phase(&mut self, next: ConnectionPhase) -> Result<(), ProtocolFault> {
            match self {
                Self::Handle(raw) => raw.phase = next,
                #[cfg(test)]
                Self::ProtocolHeld { phase, .. } => *phase = next,
                _ => return Err(ProtocolFault::ResourceNotInstalled),
            }
            Ok(())
        }
    }
    impl StmtSlot {
        fn live(&self) -> Option<&StmtState> {
            match self {
                Self::Live { state, .. } => Some(state),
                #[cfg(test)]
                Self::ProtocolHeld(state) => Some(state),
                _ => None,
            }
        }
        fn live_mut(&mut self) -> Option<&mut StmtState> {
            match self {
                Self::Live { state, .. } => Some(state),
                #[cfg(test)]
                Self::ProtocolHeld(state) => Some(state),
                _ => None,
            }
        }
    }
    // Same genuine fields, with one owner-local saved paid result. This is not
    // the qualified OriginalSqlLoan and never creates/moves a RowsWork.
    struct OriginalAcquisitionLoan<'a> {
        fields: OriginalOwnerFields<'a>,
        primary: Option<SourceOperationError>,
        draining: bool,
    }
    struct OriginalConstructorPort<'s, 'a> { loan: &'s mut OriginalAcquisitionLoan<'a> }
    struct A00Port<'s, 'a> { loan: &'s mut OriginalAcquisitionLoan<'a> }
    enum AcquisitionSettlement<'a> {
        Released {
            fields: OriginalOwnerFields<'a>,
            primary: Option<SourceOperationError>,
            terminal: Option<SourceTerminal>,
        },
        Held(OriginalAcquisitionLoan<'a>),
    }
    impl<'a> OriginalOwnerFields<'a> {
        fn original_acquisition(self) -> OriginalAcquisitionLoan<'a> {
            OriginalAcquisitionLoan { fields: self, primary: None, draining: false }
        }
    }
    impl<'a> OriginalAcquisitionLoan<'a> {
        fn terminal(&self) -> Option<SourceTerminal> { self.fields.work.terminal() }
        // Normal cleanup/payment obligations survive resource consumption.
        // Both ports and settlement consult this same barrier. Terminal drain
        // deliberately bypasses it and never requests an owned error.
        fn pending_normal_action(&self) -> Option<LifecycleAction> {
            if let Some(action) = self.fields.native.transaction_action(&self.fields.work, self.fields.physical) { return Some(action); }
            if self.fields.physical.audit_pending() {
                return Some(if self.fields.physical.primary.is_some() { LifecycleAction::DrainAuditResources }
                    else { LifecycleAction::FinishAuditAcquisition });
            }
            if self.fields.physical.sidecar_locals_pending() {
                return Some(if self.fields.physical.primary.is_some() { LifecycleAction::DrainSidecarLocals }
                    else { LifecycleAction::FinishSidecarAcquisition });
            }
            let action = match self.fields.native.a00.phase {
                A00Phase::Prepare => LifecycleAction::PrepareA00,
                A00Phase::Query => LifecycleAction::BeginA00Query,
                A00Phase::Step => LifecycleAction::StepA00,
                A00Phase::Read => LifecycleAction::ReadA00Integer,
                A00Phase::PrimaryPrepare | A00Phase::PrimaryStep | A00Phase::PrimaryNoRows | A00Phase::PrimaryReset => LifecycleAction::RetainPaidPrimary,
                A00Phase::Reset => LifecycleAction::ResetA00,
                A00Phase::PaidReset => LifecycleAction::DiscardPaidReset,
                A00Phase::Finalize => LifecycleAction::FinalizeA00,
                A00Phase::PaidFinalize => LifecycleAction::DiscardPaidFinalize,
                A00Phase::NotStarted | A00Phase::Complete => {
                    let original = &self.fields.native.original;
                    let constructor_error = (original.no_handle() || original.phase() == Some(ConnectionPhase::OpenError))
                        && original.status().is_some_and(|status| matches!(status.open, CodeSlot::Called(_)));
                    return if self.primary.is_none() && constructor_error {
                        Some(LifecycleAction::RetainPaidPrimary)
                    } else { None };
                }
            };
            Some(action)
        }
        fn adverse(&mut self, action: FixedAction, code: i32) {
            if code == rusqlite::ffi::SQLITE_OK { return; }
            if self.fields.native.secondary.is_none() {
                self.fields.native.secondary = Some(FixedAdverse { role: Role::Original, action, ordinal: 0, code });
            }
            if self.fields.release.first_secondary.is_none() {
                self.fields.release.first_secondary = Some(FixedAdverse { role: Role::Original, action, ordinal: 0, code });
            }
        }
        fn drain_action(&self) -> LifecycleAction {
            if let Some(action) = self.fields.native.transaction_action(&self.fields.work, self.fields.physical) { return action; }
            if self.fields.physical.audit_pending() { return LifecycleAction::DrainAuditResources; }
            if self.fields.physical.sidecar_locals_pending() { return LifecycleAction::DrainSidecarLocals; }
            if let Some(state) = self.fields.native.statements[0].live() {
                return if state.cursor != CursorPhase::NoCursor && matches!(state.reset, CodeSlot::NotCalled) {
                    LifecycleAction::ResetA00
                } else { LifecycleAction::FinalizeA00 };
            }
            if self.fields.native.statements[1..].iter().any(|slot| slot.live().is_some())
                || !matches!(&self.fields.native.aux, AuxPlace::Empty) {
                // Other owner phases have no cleanup operation in this port.
                return LifecycleAction::FatalBorrow;
            }
            if let Some(phase) = self.fields.native.original.phase() {
                let status = self.fields.native.original.status().expect("held place has status");
                if !matches!(status.close_first, CodeSlot::NotCalled) { return LifecycleAction::FatalBorrow; }
                return if phase == ConnectionPhase::OpenError {
                    LifecycleAction::ConstructorClose
                } else { LifecycleAction::OriginalClose };
            }
            LifecycleAction::Quiescent
        }
        fn constructor(&mut self) -> OriginalConstructorPort<'_, 'a> { OriginalConstructorPort { loan: self } }
        fn begin_a00(&mut self) -> Result<A00Port<'_, 'a>, ProtocolFault> {
            if self.terminal().is_some() || self.primary.is_some() || self.fields.physical.post_a00_started()
                || self.fields.native.original.phase() != Some(ConnectionPhase::Configured)
                || self.fields.native.a00.phase != A00Phase::NotStarted
                || self.fields.native.statements.iter().any(|slot| !matches!(slot, StmtSlot::Vacant))
                || !matches!(&self.fields.native.aux, AuxPlace::Empty)
                || !matches!(&self.fields.native.tx.phase, TxPhase::NotCreated) {
                return Err(ProtocolFault::UnexpectedObservation);
            }
            self.fields.native.a00.phase = A00Phase::Prepare;
            Ok(A00Port { loan: self })
        }
        fn a00_epilogue(&mut self) -> A00Port<'_, 'a> { A00Port { loan: self } }
        // A00's once-owned error stays in this loan on every refused handoff.
        // The successful mechanics handoff moves only the same field borrow;
        // it neither closes Original nor grants FS/SQL/provider qualification.
        fn into_sidecar_scope(mut self) -> Result<OriginalOwnerFields<'a>, Self> {
            if self.terminal().is_some() || self.primary.is_some() || self.draining
                || self.fields.native.original.phase() != Some(ConnectionPhase::Configured)
                || self.fields.native.a00.phase != A00Phase::Complete
                || self.fields.native.statements.iter().any(|slot| slot.live().is_some())
                || !matches!(&self.fields.native.aux, AuxPlace::Empty)
                || !matches!(&self.fields.native.tx.phase, TxPhase::NotCreated) {
                return Err(self);
            }
            if self.fields.physical.begin_after_a00().is_err() { return Err(self); }
            let Self { fields, .. } = self;
            Ok(fields)
        }
        fn retain_paid_primary(&mut self, error: SourceOperationError) -> Result<(), SourceOperationError> {
            if self.terminal().is_some() || self.primary.is_some() || self.fields.physical.post_a00_started() { return Err(error); }
            let phase = self.fields.native.a00.phase;
            let required = matches!(phase, A00Phase::PrimaryPrepare | A00Phase::PrimaryStep | A00Phase::PrimaryNoRows | A00Phase::PrimaryReset)
                || self.constructor().next() == LifecycleAction::RetainPaidPrimary;
            if !required { return Err(error); }
            self.primary = Some(error);
            self.fields.native.a00.phase = match phase {
                A00Phase::PrimaryPrepare => A00Phase::Complete,
                A00Phase::PrimaryStep => A00Phase::Reset,
                A00Phase::PrimaryNoRows | A00Phase::PrimaryReset => A00Phase::Finalize,
                other => other,
            };
            Ok(())
        }
        fn discard_paid_cleanup(&mut self, error: SourceOperationError) -> Result<(), SourceOperationError> {
            if self.terminal().is_some() { return Err(error); }
            let next = match self.fields.native.a00.phase {
                A00Phase::PaidReset => A00Phase::Finalize,
                A00Phase::PaidFinalize => A00Phase::Complete,
                _ => return Err(error),
            };
            drop(error);
            self.fields.native.a00.phase = next;
            Ok(())
        }
        fn start_original_drain(&mut self) -> Result<(), ProtocolFault> {
            if self.terminal().is_none() && (self.pending_normal_action().is_some()
                || self.fields.native.a00.phase != A00Phase::Complete) {
                return Err(ProtocolFault::UnexpectedObservation);
            }
            self.draining = true;
            Ok(())
        }
        fn settle(self) -> AcquisitionSettlement<'a> {
            if self.fields.native.transaction_action(&self.fields.work, self.fields.physical).is_some()
                || (self.terminal().is_none() && self.pending_normal_action().is_some())
                || self.fields.physical.blocks_original_close()
                || self.fields.native.original.phase().is_some()
                || !matches!(&self.fields.native.aux, AuxPlace::Empty)
                || self.fields.native.statements.iter().any(|slot| slot.live().is_some()) {
                return AcquisitionSettlement::Held(self);
            }
            let terminal = self.terminal();
            let Self { fields, primary, .. } = self;
            AcquisitionSettlement::Released { fields, primary, terminal }
        }
    }
    impl OriginalConstructorPort<'_, '_> {
        fn next(&self) -> LifecycleAction {
            if self.loan.terminal().is_some() { return self.loan.drain_action(); }
            if let Some(action) = self.loan.pending_normal_action() { return action; }
            if self.loan.primary.is_some() || self.loan.draining { return self.loan.drain_action(); }
            let original = &self.loan.fields.native.original;
            match original {
                OriginalPlace::Empty => LifecycleAction::NeedSelectedName,
                OriginalPlace::NameReady { .. } => LifecycleAction::OpenOriginal,
                OriginalPlace::NoHandle { .. } => if matches!(original.status().expect("no-handle status").open, CodeSlot::NotCalled) { LifecycleAction::OpenOriginal } else { LifecycleAction::RetainPaidPrimary },
                #[cfg(test)]
                OriginalPlace::ProtocolNoHandle(_) => if matches!(original.status().expect("protocol status").open, CodeSlot::NotCalled) { LifecycleAction::OpenOriginal } else { LifecycleAction::RetainPaidPrimary },
                OriginalPlace::Released(_) => LifecycleAction::Quiescent,
                _ if matches!(original.status().expect("held status").open, CodeSlot::NotCalled) => LifecycleAction::OpenOriginal,
                _ => match original.phase().expect("held phase") {
                    ConnectionPhase::OpenError => LifecycleAction::RetainPaidPrimary,
                    ConnectionPhase::ConfigureExtended => LifecycleAction::ExtendedResult,
                    ConnectionPhase::ConfigureBusyTimeout => LifecycleAction::BusyTimeout,
                    ConnectionPhase::Configured => LifecycleAction::ConstructorComplete,
                    ConnectionPhase::FirstCopiedCloseFailed => LifecycleAction::FatalBorrow,
                },
            }
        }
        fn observe(&mut self, event: ConstructorObservation) -> Result<(), ProtocolFault> {
            match event {
                ConstructorObservation::Open(code) => {
                    if self.next() != LifecycleAction::OpenOriginal { return Err(ProtocolFault::UnexpectedObservation); }
                    let original = &mut self.loan.fields.native.original;
                    // The adapter has already retained the real nonnull resource,
                    // or installed its NoHandle observation; this never imports it.
                    let held = original.phase().is_some();
                    if !held && !original.no_handle() {
                        return Err(ProtocolFault::ResourceNotInstalled);
                    }
                    record_once(&mut original.status_mut().ok_or(ProtocolFault::ResourceNotInstalled)?.open, code)?;
                    if held {
                        original.set_phase(if code == rusqlite::ffi::SQLITE_OK {
                            ConnectionPhase::ConfigureExtended
                        } else { ConnectionPhase::OpenError })?;
                    } else if code == rusqlite::ffi::SQLITE_OK { return Err(ProtocolFault::UnexpectedObservation); }
                    self.loan.adverse(FixedAction::OriginalOpen, code);
                }
                ConstructorObservation::Extended(code) => {
                    if self.next() != LifecycleAction::ExtendedResult { return Err(ProtocolFault::UnexpectedObservation); }
                    let original = &mut self.loan.fields.native.original;
                    record_once(&mut original.status_mut().ok_or(ProtocolFault::ResourceNotInstalled)?.extended_result, code)?;
                    original.set_phase(ConnectionPhase::ConfigureBusyTimeout)?;
                    self.loan.adverse(FixedAction::ExtendedResult, code);
                }
                ConstructorObservation::BusyTimeout(code) => {
                    if self.next() != LifecycleAction::BusyTimeout { return Err(ProtocolFault::UnexpectedObservation); }
                    let original = &mut self.loan.fields.native.original;
                    record_once(&mut original.status_mut().ok_or(ProtocolFault::ResourceNotInstalled)?.busy_timeout, code)?;
                    original.set_phase(if code == rusqlite::ffi::SQLITE_OK { ConnectionPhase::Configured } else { ConnectionPhase::OpenError })?;
                    self.loan.adverse(FixedAction::BusyTimeout, code);
                }
                ConstructorObservation::Close(code) => {
                    let action = match self.next() {
                        LifecycleAction::ConstructorClose => FixedAction::ConstructorClose,
                        LifecycleAction::OriginalClose => FixedAction::OriginalClose,
                        _ => return Err(ProtocolFault::UnexpectedObservation),
                    };
                    let original = &mut self.loan.fields.native.original;
                    record_once(&mut original.status_mut().ok_or(ProtocolFault::ResourceNotInstalled)?.close_first, code)?;
                    if code == rusqlite::ffi::SQLITE_OK {
                        let status = *original.status().expect("observed close status");
                        *original = OriginalPlace::Released(status);
                    }
                    self.loan.adverse(action, code);
                }
            }
            Ok(())
        }
    }
    impl A00Port<'_, '_> {
        fn sql(&self) -> &'static str { A00_SQL }
        fn next(&self) -> LifecycleAction {
            if self.loan.terminal().is_some() { return self.loan.drain_action(); }
            if let Some(action) = self.loan.pending_normal_action() { return action; }
            if self.loan.draining { return self.loan.drain_action(); }
            if self.loan.fields.native.a00.phase == A00Phase::Complete {
                LifecycleAction::A00Complete
            } else { LifecycleAction::NeedSelectedName }
        }
        fn observe(&mut self, event: A00Observation) -> Result<(), ProtocolFault> {
            let expected = match &event {
                A00Observation::Prepare(_) => LifecycleAction::PrepareA00,
                A00Observation::QueryStarted => LifecycleAction::BeginA00Query,
                A00Observation::Step(_) => LifecycleAction::StepA00,
                A00Observation::Integer(_) => LifecycleAction::ReadA00Integer,
                A00Observation::Reset(_) => LifecycleAction::ResetA00,
                A00Observation::Finalize(_) => LifecycleAction::FinalizeA00,
            };
            if self.next() != expected { return Err(ProtocolFault::UnexpectedObservation); }
            match event {
                A00Observation::Prepare(code) => {
                    record_once(&mut self.loan.fields.native.a00.prepare, code)?;
                    let state = self.loan.fields.native.statements[0].live();
                    if code == rusqlite::ffi::SQLITE_OK {
                        let state = state.ok_or(ProtocolFault::ResourceNotInstalled)?;
                        if !matches!(state.role, Role::Original) || !matches!(state.action, FixedAction::MaterializeCount)
                            || state.cursor != CursorPhase::NoCursor { return Err(ProtocolFault::UnexpectedObservation); }
                        self.loan.fields.native.statement_phase = StatementPhase::Single;
                        self.loan.fields.native.a00.phase = A00Phase::Query;
                    } else {
                        if state.is_some() { return Err(ProtocolFault::UnexpectedObservation); }
                        self.loan.fields.native.a00.phase = A00Phase::PrimaryPrepare;
                        self.loan.adverse(FixedAction::MaterializeCount, code);
                    }
                }
                A00Observation::QueryStarted => {
                    self.loan.fields.native.statements[0].live_mut().ok_or(ProtocolFault::ResourceNotInstalled)?.cursor = CursorPhase::Active;
                    self.loan.fields.native.a00.phase = A00Phase::Step;
                }
                A00Observation::Step(code) => {
                    let state = self.loan.fields.native.statements[0].live_mut().ok_or(ProtocolFault::ResourceNotInstalled)?;
                    record_once(&mut state.step, code)?;
                    let (outcome, phase) = if code == rusqlite::ffi::SQLITE_ROW { (A00Outcome::FirstRow, A00Phase::Read) }
                        else if code == rusqlite::ffi::SQLITE_DONE {
                            state.cursor = CursorPhase::Ended;
                            (A00Outcome::NoRow, A00Phase::Reset)
                        } else { (A00Outcome::StepError, A00Phase::PrimaryStep) };
                    self.loan.fields.native.a00.outcome = outcome;
                    self.loan.fields.native.a00.phase = phase;
                    if code != rusqlite::ffi::SQLITE_ROW && code != rusqlite::ffi::SQLITE_DONE { self.loan.adverse(FixedAction::MaterializeCount, code); }
                }
                A00Observation::Integer(value) => {
                    let _ = value; // Actual first i64 is discarded, never copied into a DTO.
                    self.loan.fields.native.a00.phase = A00Phase::Reset;
                }
                A00Observation::Reset(code) => {
                    let state = self.loan.fields.native.statements[0].live_mut().ok_or(ProtocolFault::ResourceNotInstalled)?;
                    record_once(&mut state.reset, code)?;
                    state.cursor = CursorPhase::NoCursor;
                    let no_row = self.loan.fields.native.a00.outcome == A00Outcome::NoRow;
                    self.loan.fields.native.a00.phase = if self.loan.terminal().is_some() { A00Phase::Finalize }
                        else if no_row { if code == rusqlite::ffi::SQLITE_OK { A00Phase::PrimaryNoRows } else { A00Phase::PrimaryReset } }
                        else if code == rusqlite::ffi::SQLITE_OK { A00Phase::Finalize } else { A00Phase::PaidReset };
                    self.loan.adverse(FixedAction::MaterializeCount, code);
                }
                A00Observation::Finalize(code) => {
                    let slot = &mut self.loan.fields.native.statements[0];
                    let mut state = *slot.live().ok_or(ProtocolFault::ResourceNotInstalled)?;
                    if state.cursor != CursorPhase::NoCursor { return Err(ProtocolFault::UnexpectedObservation); }
                    record_once(&mut state.finalize, code)?;
                    *slot = StmtSlot::Finalized(state); // Finalize consumes VM even on error.
                    self.loan.fields.native.statement_phase = StatementPhase::Empty;
                    self.loan.fields.native.a00.phase = if code == rusqlite::ffi::SQLITE_OK || self.loan.terminal().is_some() {
                        A00Phase::Complete
                    } else { A00Phase::PaidFinalize };
                    self.loan.adverse(FixedAction::MaterializeCount, code);
                }
            }
            Ok(())
        }
        fn retain_paid_read_error(&mut self, error: SourceOperationError) -> Result<(), SourceOperationError> {
            if self.next() != LifecycleAction::ReadA00Integer || self.loan.primary.is_some()
                || self.loan.fields.physical.post_a00_started() { return Err(error); }
            self.loan.primary = Some(error);
            self.loan.fields.native.a00.outcome = A00Outcome::ReadError;
            self.loan.fields.native.a00.phase = A00Phase::Reset;
            Ok(())
        }
    }


    // These fixed cfg-test carriers are variants of the real owner's fields,
    // but contain no sqlite pointer/name/rules. They prove protocol only.
    #[cfg(test)]
    #[derive(Clone, Copy)]
    pub(in crate::database::global_schema_v1) enum LifecycleConstructorCase {
        NoHandle, OpenErrorWithResource, BusyTimeoutError,
    }
    #[cfg(test)]
    #[derive(Clone, Copy)]
    pub(in crate::database::global_schema_v1) enum LifecycleA00Case {
        FirstRow, NoRow, StepError,
    }
    #[cfg(test)]
    fn protocol_connection_status() -> ConnectionStatus {
        ConnectionStatus {
            open: CodeSlot::NotCalled, extended_result: CodeSlot::NotCalled,
            busy_timeout: CodeSlot::NotCalled, close_first: CodeSlot::NotCalled,
            close_second: CodeSlot::NotCalled,
        }
    }
    #[cfg(test)]
    fn protocol_stmt_state() -> StmtState {
        StmtState {
            role: Role::Original, action: FixedAction::MaterializeCount,
            cursor: CursorPhase::NoCursor, step: CodeSlot::NotCalled,
            reset: CodeSlot::NotCalled, finalize: CodeSlot::NotCalled,
        }
    }
    #[cfg(test)]
    fn fixed_supplied_error() -> (SourceOperationError, usize) {
        // Fixture ownership is supplied before the core. This is not evidence
        // that a real SQL diagnostic request has been funded or constructed.
        let detail = String::from("TEST_CODE supplied owned lifecycle primary");
        let allocation = detail.as_ptr() as usize;
        (SourceOperationError::Global(super::super::super::GlobalSchemaV1Error::SelectionSnapshotChanged { detail }), allocation)
    }
    #[cfg(test)]
    fn assert_same_supplied_primary(error: SourceOperationError, allocation: usize) {
        let SourceOperationError::Global(super::super::super::GlobalSchemaV1Error::SelectionSnapshotChanged { detail }) = error else {
            panic!("supplied owner category must survive");
        };
        assert_eq!(detail.as_ptr() as usize, allocation);
        assert_eq!(detail, "TEST_CODE supplied owned lifecycle primary");
    }
    #[cfg(test)]
    impl<'a> OriginalAcquisitionLoan<'a> {
        fn fixed_configured_carrier(&mut self) {
            assert!(matches!(&self.fields.native.original, OriginalPlace::Empty));
            self.fields.native.original = OriginalPlace::ProtocolHeld {
                phase: ConnectionPhase::OpenError, status: protocol_connection_status(),
            };
            self.constructor().observe(ConstructorObservation::Open(rusqlite::ffi::SQLITE_OK)).unwrap();
            assert_eq!(self.constructor().next(), LifecycleAction::ExtendedResult);
            self.constructor().observe(ConstructorObservation::Extended(rusqlite::ffi::SQLITE_OK)).unwrap();
            assert_eq!(self.constructor().next(), LifecycleAction::BusyTimeout);
            self.constructor().observe(ConstructorObservation::BusyTimeout(rusqlite::ffi::SQLITE_OK)).unwrap();
            assert_eq!(self.constructor().next(), LifecycleAction::ConstructorComplete);
        }
        fn fixed_prepared_a00(&mut self) {
            self.begin_a00().unwrap();
            assert_eq!(self.a00_epilogue().sql(), A00_SQL);
            assert_eq!(self.a00_epilogue().next(), LifecycleAction::PrepareA00);
            self.fields.native.statements[0] = StmtSlot::ProtocolHeld(protocol_stmt_state());
            self.a00_epilogue().observe(A00Observation::Prepare(rusqlite::ffi::SQLITE_OK)).unwrap();
            assert_eq!(self.a00_epilogue().next(), LifecycleAction::BeginA00Query);
            self.a00_epilogue().observe(A00Observation::QueryStarted).unwrap();
            assert_eq!(self.a00_epilogue().next(), LifecycleAction::StepA00);
        }
    }
    #[cfg(test)]
    impl<'a> OriginalOwnerFields<'a> {
        pub(in crate::database::global_schema_v1) fn test_code_constructor_cut(self, case: LifecycleConstructorCase) {
            let before = self.work.test_code_observation();
            let mut loan = self.original_acquisition();
            assert_eq!(loan.constructor().next(), LifecycleAction::NeedSelectedName);
            loan.fields.native.original = match case {
                LifecycleConstructorCase::NoHandle => OriginalPlace::ProtocolNoHandle(protocol_connection_status()),
                _ => OriginalPlace::ProtocolHeld { phase: ConnectionPhase::OpenError, status: protocol_connection_status() },
            };
            let open = if matches!(case, LifecycleConstructorCase::BusyTimeoutError) {
                rusqlite::ffi::SQLITE_OK
            } else { rusqlite::ffi::SQLITE_CANTOPEN };
            loan.constructor().observe(ConstructorObservation::Open(open)).unwrap();
            if matches!(case, LifecycleConstructorCase::BusyTimeoutError) {
                // Original driver ignores this status; it cannot become primary.
                loan.constructor().observe(ConstructorObservation::Extended(rusqlite::ffi::SQLITE_ERROR)).unwrap();
                assert_eq!(loan.constructor().next(), LifecycleAction::BusyTimeout);
                loan.constructor().observe(ConstructorObservation::BusyTimeout(rusqlite::ffi::SQLITE_ERROR)).unwrap();
            }
            assert_eq!(loan.constructor().next(), LifecycleAction::RetainPaidPrimary);
            assert_eq!(loan.constructor().observe(ConstructorObservation::Close(rusqlite::ffi::SQLITE_OK)), Err(ProtocolFault::UnexpectedObservation));
            let AcquisitionSettlement::Held(mut loan) = loan.settle() else {
                panic!("even null-open must retain its pending primary obligation");
            };
            assert!(loan.primary.is_none());
            assert_eq!(loan.constructor().next(), LifecycleAction::RetainPaidPrimary);
            let (error, allocation) = fixed_supplied_error();
            assert!(loan.retain_paid_primary(error).is_ok());
            if matches!(case, LifecycleConstructorCase::NoHandle) {
                assert_eq!(loan.constructor().next(), LifecycleAction::Quiescent);
                assert!(matches!(loan.fields.native.original.status().unwrap().close_first, CodeSlot::NotCalled));
            } else {
                assert_eq!(loan.constructor().next(), LifecycleAction::ConstructorClose);
                loan.constructor().observe(ConstructorObservation::Close(rusqlite::ffi::SQLITE_OK)).unwrap();
                assert!(matches!(loan.fields.native.original.status().unwrap().close_first, CodeSlot::Called(rusqlite::ffi::SQLITE_OK)));
                assert_eq!(loan.constructor().next(), LifecycleAction::Quiescent);
                assert_eq!(loan.constructor().observe(ConstructorObservation::Close(rusqlite::ffi::SQLITE_OK)), Err(ProtocolFault::UnexpectedObservation));
            }
            assert!(matches!(loan.fields.native.original.status().unwrap().close_second, CodeSlot::NotCalled));
            let AcquisitionSettlement::Released { fields, primary: Some(error), terminal } = loan.settle() else {
                panic!("fixed released carrier retains one supplied primary");
            };
            assert!(terminal.is_none());
            assert_same_supplied_primary(error, allocation);
            assert_eq!(fields.work.test_code_observation(), before);
        }
        pub(in crate::database::global_schema_v1) fn test_code_a00_cut(self, case: LifecycleA00Case) {
            let before = self.work.test_code_observation();
            let mut loan = self.original_acquisition();
            loan.fixed_configured_carrier();
            loan.fixed_prepared_a00();
            let step = match case {
                LifecycleA00Case::FirstRow => rusqlite::ffi::SQLITE_ROW,
                LifecycleA00Case::NoRow => rusqlite::ffi::SQLITE_DONE,
                LifecycleA00Case::StepError => rusqlite::ffi::SQLITE_ERROR,
            };
            loan.a00_epilogue().observe(A00Observation::Step(step)).unwrap();
            let next = match case {
                LifecycleA00Case::FirstRow => LifecycleAction::ReadA00Integer,
                LifecycleA00Case::NoRow => LifecycleAction::ResetA00,
                LifecycleA00Case::StepError => LifecycleAction::RetainPaidPrimary,
            };
            assert_eq!(loan.constructor().next(), next);
            assert_eq!(loan.constructor().observe(ConstructorObservation::Close(rusqlite::ffi::SQLITE_OK)), Err(ProtocolFault::UnexpectedObservation));
            let AcquisitionSettlement::Held(mut loan) = loan.settle() else { panic!("A00 continuation cannot settle early"); };
            let allocation = if matches!(case, LifecycleA00Case::StepError) {
                assert_eq!(loan.a00_epilogue().next(), LifecycleAction::RetainPaidPrimary);
                let (error, allocation) = fixed_supplied_error();
                assert!(loan.retain_paid_primary(error).is_ok());
                Some(allocation)
            } else { None };
            if matches!(case, LifecycleA00Case::FirstRow) {
                assert_eq!(loan.a00_epilogue().next(), LifecycleAction::ReadA00Integer);
                loan.a00_epilogue().observe(A00Observation::Integer(7)).unwrap();
                // First-row success selects reset immediately, without extra EOF.
                assert_eq!(loan.a00_epilogue().next(), LifecycleAction::ResetA00);
                assert_eq!(loan.a00_epilogue().observe(A00Observation::Step(rusqlite::ffi::SQLITE_DONE)), Err(ProtocolFault::UnexpectedObservation));
            }
            let reset = if matches!(case, LifecycleA00Case::NoRow) { rusqlite::ffi::SQLITE_OK } else { rusqlite::ffi::SQLITE_ERROR };
            loan.a00_epilogue().observe(A00Observation::Reset(reset)).unwrap();
            let next = if matches!(case, LifecycleA00Case::NoRow) { LifecycleAction::RetainPaidPrimary }
                else { LifecycleAction::DiscardPaidReset };
            assert_eq!(loan.constructor().next(), next);
            assert_eq!(loan.constructor().observe(ConstructorObservation::Close(rusqlite::ffi::SQLITE_OK)), Err(ProtocolFault::UnexpectedObservation));
            assert_eq!(loan.start_original_drain(), Err(ProtocolFault::UnexpectedObservation));
            let AcquisitionSettlement::Held(mut loan) = loan.settle() else { panic!("pending reset diagnostic cannot settle early"); };
            let allocation = if matches!(case, LifecycleA00Case::NoRow) {
                assert_eq!(loan.a00_epilogue().next(), LifecycleAction::RetainPaidPrimary);
                let (error, allocation) = fixed_supplied_error();
                assert!(loan.retain_paid_primary(error).is_ok());
                Some(allocation)
            } else {
                assert_eq!(loan.a00_epilogue().next(), LifecycleAction::DiscardPaidReset);
                assert!(loan.discard_paid_cleanup(fixed_supplied_error().0).is_ok());
                allocation
            };
            assert_eq!(loan.a00_epilogue().next(), LifecycleAction::FinalizeA00);
            loan.a00_epilogue().observe(A00Observation::Finalize(rusqlite::ffi::SQLITE_ERROR)).unwrap();
            assert!(loan.fields.native.statements[0].live().is_none());
            assert_eq!(loan.a00_epilogue().next(), LifecycleAction::DiscardPaidFinalize);
            assert_eq!(loan.constructor().next(), LifecycleAction::DiscardPaidFinalize);
            assert_eq!(loan.constructor().observe(ConstructorObservation::Close(rusqlite::ffi::SQLITE_OK)), Err(ProtocolFault::UnexpectedObservation));
            assert_eq!(loan.start_original_drain(), Err(ProtocolFault::UnexpectedObservation));
            let AcquisitionSettlement::Held(mut loan) = loan.settle() else { panic!("consumed VM still owes its normal finalize diagnostic"); };
            {
                let port = loan.constructor();
                assert_eq!(port.next(), LifecycleAction::DiscardPaidFinalize);
                // Ending this short port borrow cannot drop the saved primary.
            }
            if let Some(allocation) = allocation {
                let Some(SourceOperationError::Global(super::super::super::GlobalSchemaV1Error::SelectionSnapshotChanged { detail })) = &loan.primary else { panic!("normal primary remains in loan"); };
                assert_eq!(detail.as_ptr() as usize, allocation);
            }
            assert!(loan.discard_paid_cleanup(fixed_supplied_error().0).is_ok());
            assert_eq!(loan.a00_epilogue().next(), LifecycleAction::A00Complete);
            let StmtSlot::Finalized(state) = &loan.fields.native.statements[0] else { panic!("consumed slot keeps statuses"); };
            assert_eq!((state.step, state.reset, state.finalize), (CodeSlot::Called(step), CodeSlot::Called(reset), CodeSlot::Called(rusqlite::ffi::SQLITE_ERROR)));
            loan.start_original_drain().unwrap();
            assert_eq!(loan.constructor().next(), LifecycleAction::OriginalClose);
            if matches!(case, LifecycleA00Case::StepError) {
                loan.constructor().observe(ConstructorObservation::Close(rusqlite::ffi::SQLITE_BUSY)).unwrap();
                let AcquisitionSettlement::Held(mut loan) = loan.settle() else { panic!("normal primary and failed-close fields stay together"); };
                {
                    let port = loan.constructor();
                    assert_eq!(port.next(), LifecycleAction::FatalBorrow);
                }
                let Some(SourceOperationError::Global(super::super::super::GlobalSchemaV1Error::SelectionSnapshotChanged { detail })) = &loan.primary else { panic!("failed-close loan still owns primary"); };
                assert_eq!(Some(detail.as_ptr() as usize), allocation);
                assert!(loan.fields.native.original.phase().is_some());
                assert!(matches!(loan.fields.native.original.status().unwrap().close_first, CodeSlot::Called(rusqlite::ffi::SQLITE_BUSY)));
                assert!(matches!(loan.fields.native.original.status().unwrap().close_second, CodeSlot::NotCalled));
                assert_eq!(loan.constructor().observe(ConstructorObservation::Close(rusqlite::ffi::SQLITE_OK)), Err(ProtocolFault::UnexpectedObservation));
                assert_eq!(loan.fields.work.test_code_observation(), before);
                return;
            }
            loan.constructor().observe(ConstructorObservation::Close(rusqlite::ffi::SQLITE_OK)).unwrap();
            let AcquisitionSettlement::Released { fields, primary, terminal } = loan.settle() else { panic!("fixed close completed"); };
            match (primary, allocation) {
                (Some(error), Some(allocation)) => assert_same_supplied_primary(error, allocation),
                (None, None) => (),
                _ => panic!("ignored cleanup cannot replace supplied primary"),
            }
            assert!(terminal.is_none());
            assert_eq!(fields.work.test_code_observation(), before);
        }
        pub(in crate::database::global_schema_v1) fn test_code_seed_live_a00(self) {
            let mut loan = self.original_acquisition();
            loan.fixed_configured_carrier();
            loan.fixed_prepared_a00();
            loan.a00_epilogue().observe(A00Observation::Step(rusqlite::ffi::SQLITE_ROW)).unwrap();
            let AcquisitionSettlement::Held(mut loan) = loan.settle() else { panic!("live protocol fields cannot detach"); };
            assert_eq!(loan.a00_epilogue().next(), LifecycleAction::ReadA00Integer);
            // Dropping only this loan ends the borrow, with no native release.
        }
        pub(in crate::database::global_schema_v1) fn test_code_terminal_drain(self) {
            let before = self.work.test_code_observation();
            let terminal = before.terminal.expect("fixed fixture already latched terminal");
            let mut loan = self.original_acquisition();
            assert_eq!(loan.begin_a00().err(), Some(ProtocolFault::UnexpectedObservation));
            assert_eq!(loan.a00_epilogue().next(), LifecycleAction::ResetA00);
            loan.a00_epilogue().observe(A00Observation::Reset(rusqlite::ffi::SQLITE_ERROR)).unwrap();
            assert_eq!(loan.a00_epilogue().next(), LifecycleAction::FinalizeA00);
            loan.a00_epilogue().observe(A00Observation::Finalize(rusqlite::ffi::SQLITE_ERROR)).unwrap();
            assert_eq!(loan.constructor().next(), LifecycleAction::OriginalClose);
            loan.constructor().observe(ConstructorObservation::Close(rusqlite::ffi::SQLITE_BUSY)).unwrap();
            assert_eq!(loan.constructor().next(), LifecycleAction::FatalBorrow);
            assert_eq!(loan.constructor().observe(ConstructorObservation::Close(rusqlite::ffi::SQLITE_OK)), Err(ProtocolFault::UnexpectedObservation));
            let AcquisitionSettlement::Held(loan) = loan.settle() else { panic!("failed last close must keep entire loan"); };
            assert!(loan.fields.native.original.phase().is_some());
            assert!(matches!(loan.fields.native.original.status().unwrap().close_first, CodeSlot::Called(rusqlite::ffi::SQLITE_BUSY)));
            assert!(matches!(loan.fields.native.original.status().unwrap().close_second, CodeSlot::NotCalled));
            assert_eq!(loan.terminal(), Some(terminal));
            assert_eq!(loan.fields.work.test_code_observation(), before);
        }
        pub(in crate::database::global_schema_v1) fn test_code_unreleased_a00(&self) -> bool {
            self.native.original.phase().is_some()
                && matches!(self.native.original.status().unwrap().close_first, CodeSlot::Called(rusqlite::ffi::SQLITE_BUSY))
                && matches!(&self.native.statements[0], StmtSlot::Finalized(_))
        }
    }

    // Fixed post-A00 protocol scripts only. No native/FS adapter is invoked.
    #[cfg(test)]
    impl<'a> OriginalOwnerFields<'a> {
        pub(in crate::database::global_schema_v1) fn test_code_enter_sidecars(self) {
            let before = self.work.test_code_observation();
            let mut loan = self.original_acquisition();
            loan.fixed_configured_carrier();
            loan.fixed_prepared_a00();
            loan.a00_epilogue().observe(A00Observation::Step(rusqlite::ffi::SQLITE_ROW)).unwrap();
            loan.a00_epilogue().observe(A00Observation::Integer(7)).unwrap();
            loan.a00_epilogue().observe(A00Observation::Reset(rusqlite::ffi::SQLITE_OK)).unwrap();
            loan.a00_epilogue().observe(A00Observation::Finalize(rusqlite::ffi::SQLITE_OK)).unwrap();
            let Ok(fields) = loan.into_sidecar_scope() else { panic!("completed A00 must hand off the same loan"); };
            assert!(fields.physical.blocks_original_close());
            assert_eq!(fields.work.test_code_observation(), before);
        }
        pub(in crate::database::global_schema_v1) fn test_code_a00_primary_refuses_sidecars(self) {
            let before = self.work.test_code_observation();
            let (error, allocation) = fixed_supplied_error();
            let mut loan = self.original_acquisition();
            loan.fixed_configured_carrier();
            loan.fixed_prepared_a00();
            loan.a00_epilogue().observe(A00Observation::Step(rusqlite::ffi::SQLITE_ERROR)).unwrap();
            loan.retain_paid_primary(error).unwrap_or_else(|_| panic!("supplied primary must move once"));
            loan.a00_epilogue().observe(A00Observation::Reset(rusqlite::ffi::SQLITE_OK)).unwrap();
            loan.a00_epilogue().observe(A00Observation::Finalize(rusqlite::ffi::SQLITE_OK)).unwrap();
            let Err(loan) = loan.into_sidecar_scope() else { panic!("failed A00 must not enter E04"); };
            assert!(!loan.fields.physical.post_a00_started());
            let Some(SourceOperationError::Global(super::super::super::GlobalSchemaV1Error::SelectionSnapshotChanged { detail })) = &loan.primary else {
                panic!("blocked handoff must retain the original primary");
            };
            assert_eq!(detail.as_ptr() as usize, allocation);
            assert_eq!(loan.fields.work.test_code_observation(), before);
            assert!(loan.fields.native.original.phase().is_some());
        }
        pub(in crate::database::global_schema_v1) fn test_code_sidecar_blocks_close(self) {
            let before = self.work.test_code_observation();
            let mut loan = self.original_acquisition();
            let expected = if loan.terminal().is_some() || loan.fields.physical.primary.is_some() { LifecycleAction::DrainSidecarLocals }
                else { LifecycleAction::FinishSidecarAcquisition };
            assert_eq!(loan.constructor().next(), expected);
            assert_eq!(loan.a00_epilogue().next(), expected);
            assert_eq!(loan.constructor().observe(ConstructorObservation::Close(rusqlite::ffi::SQLITE_OK)), Err(ProtocolFault::UnexpectedObservation));
            assert_eq!(loan.a00_epilogue().observe(A00Observation::Finalize(rusqlite::ffi::SQLITE_OK)), Err(ProtocolFault::UnexpectedObservation));
            assert!(loan.begin_a00().is_err());
            let AcquisitionSettlement::Held(loan) = loan.settle() else { panic!("local ownership/payment barrier must hold the whole loan"); };
            assert!(loan.primary.is_none()); // E04's primary remains in G, never duplicated in Q.
            assert_eq!(loan.fields.work.test_code_observation(), before);
            assert!(loan.fields.native.original.phase().is_some());
        }
        pub(in crate::database::global_schema_v1) fn test_code_audit_blocks_close(self) {
            let before = self.work.test_code_observation();
            let mut loan = self.original_acquisition();
            let expected = if loan.terminal().is_some() || loan.fields.physical.primary.is_some() { LifecycleAction::DrainAuditResources }
                else { LifecycleAction::FinishAuditAcquisition };
            assert_eq!(loan.constructor().next(), expected);
            assert_eq!(loan.a00_epilogue().next(), expected);
            assert_eq!(loan.constructor().observe(ConstructorObservation::Close(rusqlite::ffi::SQLITE_OK)), Err(ProtocolFault::UnexpectedObservation));
            let AcquisitionSettlement::Held(loan) = loan.settle() else { panic!("audit resources/primary obligation must hold the same loan"); };
            assert!(loan.primary.is_none());
            assert_eq!(loan.fields.work.test_code_observation(), before);
        }
        pub(in crate::database::global_schema_v1) fn test_code_sidecar_close_ok(self) {
            let before = self.work.test_code_observation();
            let mut loan = self.original_acquisition();
            loan.start_original_drain().unwrap();
            assert_eq!(loan.constructor().next(), LifecycleAction::OriginalClose);
            loan.constructor().observe(ConstructorObservation::Close(rusqlite::ffi::SQLITE_OK)).unwrap();
            let AcquisitionSettlement::Released { fields, primary, terminal } = loan.settle() else { panic!("legal OK close must release only the native slot"); };
            assert!(primary.is_none());
            assert_eq!(terminal, before.terminal);
            assert_eq!(fields.work.test_code_observation(), before);
            assert!(fields.physical.post_a00_started());
        }
        pub(in crate::database::global_schema_v1) fn test_code_sidecar_close_busy(self) {
            let before = self.work.test_code_observation();
            let mut loan = self.original_acquisition();
            loan.start_original_drain().unwrap();
            assert_eq!(loan.constructor().next(), LifecycleAction::OriginalClose);
            loan.constructor().observe(ConstructorObservation::Close(rusqlite::ffi::SQLITE_BUSY)).unwrap();
            assert_eq!(loan.constructor().next(), LifecycleAction::FatalBorrow);
            assert_eq!(loan.constructor().observe(ConstructorObservation::Close(rusqlite::ffi::SQLITE_OK)), Err(ProtocolFault::UnexpectedObservation));
            let AcquisitionSettlement::Held(loan) = loan.settle() else { panic!("failed close must hold all siblings"); };
            assert!(loan.primary.is_none());
            assert_eq!(loan.fields.work.test_code_observation(), before);
            assert!(matches!(loan.fields.native.original.status().unwrap().close_second, CodeSlot::NotCalled));
        }
    }

    // Fixed batch lifecycle only. execute_batch has no reset call: its VM
    // goes directly to Statement::Drop/finalize, whose owned Result is ignored.
    // Neither DONE nor finalize/cleanup can construct a Transaction. Only the
    // independent whole-batch return observation can do that.
    const BEGIN_SQL: &str = "BEGIN IMMEDIATE";
    const ROLLBACK_SQL: &str = "ROLLBACK";
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum BatchKind { Begin, Rollback }
    enum BatchObservation { Prepare(i32), Step(i32), Finalize(i32) }
    // Private status, not a caller Result, native-effect flag or qualification.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum BatchReturnedFact { Ok, Error }
    pub(in crate::database::global_schema_v1) struct OriginalTransactionLoan<'a> {
        fields: OriginalOwnerFields<'a>,
    }
    struct OriginalBeginPort<'s, 'a> { loan: &'s mut OriginalTransactionLoan<'a> }
    struct OriginalEarlyTransactionExitPort<'s, 'a> { loan: &'s mut OriginalTransactionLoan<'a> }
    // Future genuine adapters alone possess this unissued rules borrow. Fixed
    // cfg scripts call the shared core explicitly as protocol observations.
    struct OriginalBatchReturnPort<'s, 'a, 'r> {
        loan: &'s mut OriginalTransactionLoan<'a>,
        kind: BatchKind,
        _rules: &'r SelectedOriginalNativeRules,
    }
    impl OriginalBatchReturnPort<'_, '_, '_> {
        fn retain_return(&mut self, fact: BatchReturnedFact) -> Result<(), ProtocolFault> {
            self.loan.retain_batch_return(self.kind, fact)
        }
    }
    impl NativeOriginalOwner {
        fn batch(&self, kind: BatchKind) -> &FixedBatchRecord {
            match kind { BatchKind::Begin => &self.tx.begin, BatchKind::Rollback => &self.tx.rollback_batch }
        }
        fn batch_mut(&mut self, kind: BatchKind) -> &mut FixedBatchRecord {
            match kind { BatchKind::Begin => &mut self.tx.begin, BatchKind::Rollback => &mut self.tx.rollback_batch }
        }
        fn batch_action(&self, kind: BatchKind, terminal: bool) -> Option<LifecycleAction> {
            let batch = self.batch(kind);
            if matches!(batch.phase, BatchPhase::Dormant | BatchPhase::Finished) { return None; }
            if terminal {
                // A returned rollback VM is installed before its prepare
                // observation; that fact must precede the live-VM drain cut.
                if kind == BatchKind::Rollback && batch.phase == BatchPhase::Prepare { return Some(LifecycleAction::PrepareRollback); }
                if kind == BatchKind::Rollback && batch.phase == BatchPhase::Step { return Some(LifecycleAction::StepRollback); }
                if self.statements[0].live().is_some() { return Some(match kind {
                    BatchKind::Begin => LifecycleAction::FinalizeBegin, BatchKind::Rollback => LifecycleAction::FinalizeRollback,
                }); }
                if kind == BatchKind::Begin && batch.phase == BatchPhase::Prepare
                    && matches!(batch.prepare, CodeSlot::NotCalled) { return Some(LifecycleAction::CancelUnissuedBegin); }
                if batch.returned == BatchReturn::ReturnedError && batch.phase == BatchPhase::Primary {
                    return Some(LifecycleAction::ConsumeKnownBatchReturn);
                }
                // Rollback is cleanup of an already-created transaction. It is
                // permitted after terminal; a fresh BEGIN/step never is.
                return Some(LifecycleAction::AwaitBatchReturn);
            }
            Some(match (kind, batch.phase) {
                (BatchKind::Begin, BatchPhase::Prepare) => LifecycleAction::PrepareBegin,
                (BatchKind::Rollback, BatchPhase::Prepare) => LifecycleAction::PrepareRollback,
                (BatchKind::Begin, BatchPhase::Step) => LifecycleAction::StepBegin,
                (BatchKind::Rollback, BatchPhase::Step) => LifecycleAction::StepRollback,
                (BatchKind::Begin, BatchPhase::Primary) => LifecycleAction::RetainTransactionPrimary,
                (BatchKind::Rollback, BatchPhase::Primary) => LifecycleAction::RetainIgnoredRollback,
                (BatchKind::Begin, BatchPhase::Finalize) => LifecycleAction::FinalizeBegin,
                (BatchKind::Rollback, BatchPhase::Finalize) => LifecycleAction::FinalizeRollback,
                (_, BatchPhase::PaidFinalize) => LifecycleAction::DiscardBatchFinalize,
                (_, BatchPhase::AwaitReturn) => LifecycleAction::AwaitBatchReturn,
                (_, BatchPhase::Dormant | BatchPhase::Finished) => return None,
            })
        }
        fn transaction_action(&self, work: &OriginalSourceWork<'_>, physical: &super::super::super::FinancialPhysical) -> Option<LifecycleAction> {
            let terminal = work.terminal().is_some();
            if let Some(action) = self.batch_action(BatchKind::Begin, terminal) { return Some(action); }
            if let Some(action) = self.capture_prefix_action(work, physical) { return Some(action); }
            if let Some(action) = self.integrity_read_action(work, physical) { return Some(action); }
            if self.initial_read.context != InitialContext::Unbound
                && (matches!(self.tx.phase, TxPhase::Active) || matches!(self.initial_read.context, InitialContext::Unknown | InitialContext::Refused)) {
                if let Some(action) = self.initial_read_action(work, physical) { return Some(action); }
            }
            match self.tx.phase {
                TxPhase::NotCreated | TxPhase::Finished => None,
                TxPhase::Active => Some(if terminal || physical.primary.is_some() { LifecycleAction::ConsumeEarlyTransaction }
                    else { LifecycleAction::AwaitInitialReadContext }),
                TxPhase::Consuming => {
                    if matches!(self.tx.autocommit, AutocommitObservation::NotObserved) { return Some(LifecycleAction::ObserveAutocommit); }
                    if let Some(action) = self.batch_action(BatchKind::Rollback, terminal) { return Some(action); }
                    if self.tx.ignored_rollback.is_some() { Some(LifecycleAction::DiscardIgnoredRollback) }
                    else { None }
                }
            }
        }
        pub(in crate::database::global_schema_v1) fn transaction_release_ready(&self, work: &OriginalSourceWork<'_>, physical: &super::super::super::FinancialPhysical) -> bool {
            self.transaction_action(work, physical).is_none()
                && matches!(self.tx.phase, TxPhase::NotCreated | TxPhase::Finished)
                && self.statements.iter().all(|slot| slot.live().is_none())
                && self.initial_read.driver_error.is_none() && self.initial_read.ignored.is_none()
                && self.integrity_read.stopped_clear() && self.capture_prefix.stopped_clear() && self.compile_options.stopped_clear()
                && self.compile_sort.stopped_clear()
        }
    }
    impl<'a> OriginalOwnerFields<'a> {
        pub(in crate::database::global_schema_v1) fn original_transaction(self) -> OriginalTransactionLoan<'a> {
            OriginalTransactionLoan { fields: self }
        }
    }
    impl<'a> OriginalTransactionLoan<'a> {
        pub(in crate::database::global_schema_v1) fn begin(&mut self) -> bool {
            let n = &mut self.fields.native;
            if self.fields.work.terminal().is_some() || self.fields.physical.primary.is_some()
                || !self.fields.physical.parent_sidecars_retained()
                || self.fields.physical.audit_phase != super::super::super::FinancialAuditPhase::Ready
                || matches!(n.initial_read.context, InitialContext::Unknown | InitialContext::Refused)
                || !n.audit_acquisition_ready() || n.tx.begin.phase != BatchPhase::Dormant { return false; }
            let StmtSlot::Finalized(state) = &n.statements[0] else { return false; };
            if state.action != FixedAction::MaterializeCount || state.cursor != CursorPhase::NoCursor
                || !matches!(state.finalize, CodeSlot::Called(_)) { return false; }
            n.tx.a00_consumed = Some(*state);
            n.statements[0] = StmtSlot::Vacant;
            n.tx.begin.phase = BatchPhase::Prepare;
            true
        }
        fn begin_port(&mut self) -> OriginalBeginPort<'_, 'a> { OriginalBeginPort { loan: self } }
        fn early_exit_port(&mut self) -> OriginalEarlyTransactionExitPort<'_, 'a> { OriginalEarlyTransactionExitPort { loan: self } }
        fn batch_return_port<'s, 'r>(&'s mut self, kind: BatchKind, rules: &'r SelectedOriginalNativeRules)
            -> OriginalBatchReturnPort<'s, 'a, 'r> {
            OriginalBatchReturnPort { loan: self, kind, _rules: rules }
        }
        fn next(&self) -> Option<LifecycleAction> { self.fields.native.transaction_action(&self.fields.work, self.fields.physical) }
        fn adverse(&mut self, action: FixedAction, code: i32) {
            if code == rusqlite::ffi::SQLITE_OK { return; }
            if self.fields.native.secondary.is_none() {
                self.fields.native.secondary = Some(FixedAdverse { role: Role::Original, action, ordinal: 0, code });
            }
            if self.fields.release.first_secondary.is_none() {
                self.fields.release.first_secondary = Some(FixedAdverse { role: Role::Original, action, ordinal: 0, code });
            }
        }
        fn observe_batch(&mut self, kind: BatchKind, event: BatchObservation) -> Result<(), ProtocolFault> {
            let action = match kind { BatchKind::Begin => FixedAction::Begin, BatchKind::Rollback => FixedAction::Rollback };
            let expected = match (&event, kind) {
                (BatchObservation::Prepare(_), BatchKind::Begin) => LifecycleAction::PrepareBegin,
                (BatchObservation::Prepare(_), BatchKind::Rollback) => LifecycleAction::PrepareRollback,
                (BatchObservation::Step(_), BatchKind::Begin) => LifecycleAction::StepBegin,
                (BatchObservation::Step(_), BatchKind::Rollback) => LifecycleAction::StepRollback,
                (BatchObservation::Finalize(_), BatchKind::Begin) => LifecycleAction::FinalizeBegin,
                (BatchObservation::Finalize(_), BatchKind::Rollback) => LifecycleAction::FinalizeRollback,
            };
            if self.next() != Some(expected) { return Err(ProtocolFault::UnexpectedObservation); }
            let terminal = self.fields.work.terminal().is_some();
            match event {
                BatchObservation::Prepare(code) => {
                    let state = self.fields.native.statements[0].live();
                    if code == rusqlite::ffi::SQLITE_OK {
                        let state = state.ok_or(ProtocolFault::ResourceNotInstalled)?;
                        if state.action != action || state.role != Role::Original || state.cursor != CursorPhase::NoCursor {
                            return Err(ProtocolFault::UnexpectedObservation);
                        }
                    } else if state.is_some() { return Err(ProtocolFault::UnexpectedObservation); }
                    record_once(&mut self.fields.native.batch_mut(kind).prepare, code)?;
                    if kind == BatchKind::Rollback { self.fields.native.tx.rollback = RollbackObservation::Reached; }
                    if code == rusqlite::ffi::SQLITE_OK { self.fields.native.statement_phase = StatementPhase::Single; }
                    self.fields.native.batch_mut(kind).phase = if code == rusqlite::ffi::SQLITE_OK { BatchPhase::Step }
                        else if terminal { BatchPhase::AwaitReturn } else { BatchPhase::Primary };
                    if code != rusqlite::ffi::SQLITE_OK { self.adverse(action, code); }
                }
                BatchObservation::Step(code) => {
                    let state = self.fields.native.statements[0].live_mut().ok_or(ProtocolFault::ResourceNotInstalled)?;
                    if state.action != action { return Err(ProtocolFault::UnexpectedObservation); }
                    record_once(&mut state.step, code)?;
                    state.cursor = if code == rusqlite::ffi::SQLITE_DONE { CursorPhase::Ended } else { CursorPhase::Active };
                    let failed = code != rusqlite::ffi::SQLITE_DONE && code != rusqlite::ffi::SQLITE_ROW;
                    self.fields.native.batch_mut(kind).phase = if failed && !terminal { BatchPhase::Primary } else { BatchPhase::Finalize };
                    if failed { self.adverse(action, code); }
                }
                BatchObservation::Finalize(code) => {
                    let mut state = *self.fields.native.statements[0].live().ok_or(ProtocolFault::ResourceNotInstalled)?;
                    if state.action != action { return Err(ProtocolFault::UnexpectedObservation); }
                    record_once(&mut state.finalize, code)?;
                    state.cursor = CursorPhase::NoCursor;
                    self.fields.native.statements[0] = StmtSlot::Finalized(state);
                    if kind == BatchKind::Begin { self.fields.native.tx.begin_consumed = Some(state); }
                    self.fields.native.statement_phase = StatementPhase::Empty;
                    self.fields.native.batch_mut(kind).phase = if code != rusqlite::ffi::SQLITE_OK && !terminal { BatchPhase::PaidFinalize }
                        else { BatchPhase::AwaitReturn };
                    self.adverse(action, code);
                }
            }
            Ok(())
        }
        fn retain_paid_primary(&mut self, error: SourceOperationError) -> Result<(), SourceOperationError> {
            if self.fields.work.terminal().is_some() || self.fields.physical.primary.is_some()
                || self.next() != Some(LifecycleAction::RetainTransactionPrimary) { return Err(error); }
            self.fields.physical.primary = Some(error);
            self.fields.physical.audit_phase = super::super::super::FinancialAuditPhase::Failed;
            self.after_paid_primary(BatchKind::Begin);
            Ok(())
        }
        fn retain_ignored_rollback(&mut self, error: SourceOperationError) -> Result<(), SourceOperationError> {
            if self.fields.work.terminal().is_some() || self.fields.native.tx.ignored_rollback.is_some()
                || self.next() != Some(LifecycleAction::RetainIgnoredRollback) { return Err(error); }
            self.fields.native.tx.ignored_rollback = Some(error);
            self.after_paid_primary(BatchKind::Rollback);
            Ok(())
        }
        fn after_paid_primary(&mut self, kind: BatchKind) {
            let phase = if self.fields.native.statements[0].live().is_some() { BatchPhase::Finalize }
                else if self.fields.native.batch(kind).returned == BatchReturn::Unobserved { BatchPhase::AwaitReturn }
                else { BatchPhase::Finished };
            self.fields.native.batch_mut(kind).phase = phase;
        }
        fn discard_paid_finalize(&mut self, error: SourceOperationError) -> Result<(), SourceOperationError> {
            if self.fields.work.terminal().is_some() || self.next() != Some(LifecycleAction::DiscardBatchFinalize) { return Err(error); }
            let kind = if self.fields.native.tx.begin.phase == BatchPhase::PaidFinalize { BatchKind::Begin } else { BatchKind::Rollback };
            drop(error); self.fields.native.batch_mut(kind).phase = BatchPhase::AwaitReturn; Ok(())
        }
        fn retain_batch_return(&mut self, kind: BatchKind, fact: BatchReturnedFact) -> Result<(), ProtocolFault> {
            // This observes the already-completed driver call. No tail query,
            // step, error factory or native action is performed by this cut.
            if self.next() != Some(LifecycleAction::AwaitBatchReturn) || self.fields.native.statements[0].live().is_some()
                || self.fields.native.batch(kind).returned != BatchReturn::Unobserved { return Err(ProtocolFault::UnexpectedObservation); }
            let terminal = self.fields.work.terminal().is_some();
            let n = &mut self.fields.native;
            if (kind == BatchKind::Begin && n.tx.begin.phase == BatchPhase::Finished)
                || (kind == BatchKind::Rollback && !matches!(n.tx.phase, TxPhase::Consuming)) { return Err(ProtocolFault::UnexpectedObservation); }
            if fact == BatchReturnedFact::Ok {
                if !matches!(n.batch(kind).prepare, CodeSlot::Called(rusqlite::ffi::SQLITE_OK)) { return Err(ProtocolFault::UnexpectedObservation); }
                let state = match &n.statements[0] { StmtSlot::Finalized(state) => state, _ => return Err(ProtocolFault::ResourceNotInstalled) };
                let action = if kind == BatchKind::Begin { FixedAction::Begin } else { FixedAction::Rollback };
                if state.action != action || !matches!(state.step, CodeSlot::Called(rusqlite::ffi::SQLITE_DONE | rusqlite::ffi::SQLITE_ROW)) {
                    return Err(ProtocolFault::UnexpectedObservation);
                }
            }
            n.batch_mut(kind).returned = if fact == BatchReturnedFact::Ok { BatchReturn::ReturnedOk } else { BatchReturn::ReturnedError };
            let unpaid = fact == BatchReturnedFact::Error && !terminal && match kind {
                BatchKind::Begin => self.fields.physical.primary.is_none(), BatchKind::Rollback => n.tx.ignored_rollback.is_none(),
            };
            n.batch_mut(kind).phase = if unpaid { BatchPhase::Primary } else { BatchPhase::Finished };
            match kind {
                BatchKind::Begin => n.tx.phase = if fact == BatchReturnedFact::Ok { TxPhase::Active } else { TxPhase::NotCreated },
                BatchKind::Rollback => if n.tx.ignored_rollback.is_none() && !unpaid { n.tx.phase = TxPhase::Finished; },
            }
            Ok(())
        }
        fn consume_known_return(&mut self) -> Result<(), ProtocolFault> {
            // First terminal consumes only an already-retained return fact.
            // No observation is repeated and no unpaid error is constructed.
            if self.next() != Some(LifecycleAction::ConsumeKnownBatchReturn) { return Err(ProtocolFault::UnexpectedObservation); }
            let kind = if self.fields.native.tx.begin.phase == BatchPhase::Primary { BatchKind::Begin } else { BatchKind::Rollback };
            self.fields.native.batch_mut(kind).phase = BatchPhase::Finished;
            if kind == BatchKind::Begin { self.fields.native.tx.phase = TxPhase::NotCreated; }
            else if self.fields.native.tx.ignored_rollback.is_none() { self.fields.native.tx.phase = TxPhase::Finished; }
            Ok(())
        }
        fn cancel_unissued_begin(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::CancelUnissuedBegin) { return Err(ProtocolFault::UnexpectedObservation); }
            self.fields.native.tx.begin.phase = BatchPhase::Finished;
            self.fields.native.tx.phase = TxPhase::NotCreated; Ok(())
        }
        fn start_early_error(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::ConsumeEarlyTransaction)
                || (self.fields.work.terminal().is_none() && self.fields.physical.primary.is_none()) { return Err(ProtocolFault::UnexpectedObservation); }
            self.fields.native.tx.phase = TxPhase::Consuming; self.fields.native.tx.exit = TxExit::EarlyError; Ok(())
        }
        fn retain_early_primary(&mut self, error: SourceOperationError) -> Result<(), SourceOperationError> {
            if self.fields.work.terminal().is_some() || self.fields.physical.primary.is_some()
                || !matches!(self.fields.native.tx.phase, TxPhase::Active)
                || self.fields.native.initial_read.ignored.is_some() || self.fields.native.statements[0].live().is_some()
                || self.fields.native.initial_read.phase == InitialPhase::Primary || self.fields.native.initial_read.driver_error.is_some()
                || self.fields.native.integrity_read.blocks_early_primary()
                || self.fields.native.capture_prefix.blocks_early_primary()
                || self.fields.native.compile_options.blocks_early_primary()
                || self.fields.native.compile_sort.blocks_early_primary() { return Err(error); }
            self.fields.physical.primary = Some(error);
            self.fields.physical.audit_phase = super::super::super::FinancialAuditPhase::Failed;
            Ok(())
        }
        fn observe_autocommit(&mut self, code: i32) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::ObserveAutocommit) { return Err(ProtocolFault::UnexpectedObservation); }
            self.fields.native.tx.autocommit = AutocommitObservation::Observed(code);
            if code != 0 { self.fields.native.tx.phase = TxPhase::Finished; }
            else {
                // The old VM is consumed; preserve its scalar BEGIN ledger.
                self.fields.native.statements[0] = StmtSlot::Vacant;
                self.fields.native.tx.rollback_batch.phase = BatchPhase::Prepare;
            }
            Ok(())
        }
        fn discard_ignored_rollback(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::DiscardIgnoredRollback) { return Err(ProtocolFault::UnexpectedObservation); }
            drop(self.fields.native.tx.ignored_rollback.take());
            self.fields.native.tx.phase = TxPhase::Finished; Ok(())
        }
    }
    impl OriginalBeginPort<'_, '_> {
        fn sql(&self) -> &'static str { BEGIN_SQL }
        fn observe(&mut self, event: BatchObservation) -> Result<(), ProtocolFault> { self.loan.observe_batch(BatchKind::Begin, event) }
    }
    impl OriginalEarlyTransactionExitPort<'_, '_> {
        fn sql(&self) -> &'static str { ROLLBACK_SQL }
        fn observe(&mut self, event: BatchObservation) -> Result<(), ProtocolFault> { self.loan.observe_batch(BatchKind::Rollback, event) }
    }

    // Fixed protocol scripts only: no pointer, FFI, tail execution or issuer.
    // A returned-Ok script is an independent observation of a completed call,
    // never a DONE/finalize-to-Active inference or provider success assertion.
    #[cfg(test)]
    impl OriginalTransactionLoan<'_> {
        fn fixed_install_batch(&mut self, kind: BatchKind) {
            assert!(matches!(&self.fields.native.statements[0], StmtSlot::Vacant));
            self.fields.native.statements[0] = StmtSlot::ProtocolHeld(StmtState {
                action: if kind == BatchKind::Begin { FixedAction::Begin } else { FixedAction::Rollback },
                ..protocol_stmt_state()
            });
            self.observe_batch(kind, BatchObservation::Prepare(rusqlite::ffi::SQLITE_OK)).unwrap();
        }
        pub(in crate::database::global_schema_v1) fn test_code_assert_transaction_barrier(&mut self) {
            assert!(self.next().is_some());
            assert!(!self.fields.native.transaction_release_ready(&self.fields.work, self.fields.physical));
            let before = self.fields.work.test_code_observation();
            let expected = self.next().unwrap();
            let mut old = self.fields.reborrow().original_acquisition();
            assert_eq!(old.constructor().next(), expected); assert_eq!(old.a00_epilogue().next(), expected);
            assert_eq!(old.constructor().observe(ConstructorObservation::Close(rusqlite::ffi::SQLITE_OK)), Err(ProtocolFault::UnexpectedObservation));
            assert_eq!(old.a00_epilogue().observe(A00Observation::Finalize(rusqlite::ffi::SQLITE_OK)), Err(ProtocolFault::UnexpectedObservation));
            let AcquisitionSettlement::Held(old) = old.settle() else { panic!("transaction/VM/payment must hold whole frame"); };
            assert!(old.primary.is_none()); assert_eq!(old.fields.work.test_code_observation(), before);
        }
        pub(in crate::database::global_schema_v1) fn test_code_failure_cut(&mut self, case: super::super::super::FinancialBeginFailureCase) {
            use super::super::super::FinancialBeginFailureCase as Case;
            assert_eq!(self.begin_port().sql(), "BEGIN IMMEDIATE");
            match case {
                Case::Prepare => self.begin_port().observe(BatchObservation::Prepare(rusqlite::ffi::SQLITE_ERROR)).unwrap(),
                Case::Step | Case::Tail => {
                    self.fixed_install_batch(BatchKind::Begin);
                    self.begin_port().observe(BatchObservation::Step(if matches!(case, Case::Step) { rusqlite::ffi::SQLITE_ERROR } else { rusqlite::ffi::SQLITE_DONE })).unwrap();
                    if matches!(case, Case::Tail) {
                        self.begin_port().observe(BatchObservation::Finalize(rusqlite::ffi::SQLITE_OK)).unwrap();
                        self.retain_batch_return(BatchKind::Begin, BatchReturnedFact::Error).unwrap();
                    }
                }
            }
            assert_eq!(self.next(), Some(LifecycleAction::RetainTransactionPrimary));
            self.test_code_assert_transaction_barrier();
        }
        pub(in crate::database::global_schema_v1) fn test_code_retain_failure_primary(&mut self, primary: SourceOperationError, spare: SourceOperationError) {
            let SourceOperationError::Global(super::super::super::GlobalSchemaV1Error::SelectionSnapshotChanged { detail }) = &spare else { panic!("fixed spare"); };
            let spare_allocation = detail.as_ptr() as usize;
            self.retain_paid_primary(primary).unwrap_or_else(|_| panic!("first G primary"));
            let Err(spare) = self.retain_paid_primary(spare) else { panic!("second G primary must be returned unchanged"); };
            let SourceOperationError::Global(super::super::super::GlobalSchemaV1Error::SelectionSnapshotChanged { detail }) = &spare else { panic!("same spare"); };
            assert_eq!(detail.as_ptr() as usize, spare_allocation); drop(spare);
            { let _short = self.begin_port(); }
        }
        pub(in crate::database::global_schema_v1) fn test_code_finish_failed_begin(&mut self, cleanup: SourceOperationError) {
            if self.fields.native.statements[0].live().is_some() {
                self.begin_port().observe(BatchObservation::Finalize(rusqlite::ffi::SQLITE_ERROR)).unwrap();
                assert!(matches!(&self.fields.native.statements[0], StmtSlot::Finalized(_)));
                self.test_code_assert_transaction_barrier();
                assert_eq!(self.begin_port().observe(BatchObservation::Finalize(rusqlite::ffi::SQLITE_OK)), Err(ProtocolFault::UnexpectedObservation));
                self.discard_paid_finalize(cleanup).unwrap_or_else(|_| panic!("paid ignored finalize"));
            } else { drop(cleanup); }
            if self.fields.native.tx.begin.returned == BatchReturn::Unobserved {
                self.test_code_assert_transaction_barrier();
                self.retain_batch_return(BatchKind::Begin, BatchReturnedFact::Error).unwrap();
            }
            assert!(matches!(self.fields.native.tx.phase, TxPhase::NotCreated));
            assert!(matches!(self.fields.native.tx.autocommit, AutocommitObservation::NotObserved));
            assert!(matches!(self.fields.native.tx.rollback, RollbackObservation::NotReached));
            assert!(self.fields.native.transaction_release_ready(&self.fields.work, self.fields.physical));
            self.assert_a00_saved();
        }
        fn assert_a00_saved(&self) {
            let saved = self.fields.native.tx.a00_consumed.as_ref().unwrap();
            assert_eq!(saved.action, FixedAction::MaterializeCount);
            assert!(matches!(saved.step, CodeSlot::Called(rusqlite::ffi::SQLITE_ROW)));
            assert!(matches!(saved.reset, CodeSlot::Called(rusqlite::ffi::SQLITE_OK)));
            assert!(matches!(saved.finalize, CodeSlot::Called(rusqlite::ffi::SQLITE_OK)));
        }
        pub(in crate::database::global_schema_v1) fn test_code_done_batch(&mut self) {
            self.fixed_install_batch(BatchKind::Begin);
            self.begin_port().observe(BatchObservation::Step(rusqlite::ffi::SQLITE_DONE)).unwrap();
            assert!(matches!(self.fields.native.tx.phase, TxPhase::NotCreated));
            self.begin_port().observe(BatchObservation::Finalize(rusqlite::ffi::SQLITE_ERROR)).unwrap();
            assert!(matches!(&self.fields.native.statements[0], StmtSlot::Finalized(_)));
            assert!(matches!(self.fields.native.tx.phase, TxPhase::NotCreated));
            self.test_code_assert_transaction_barrier();
        }
        pub(in crate::database::global_schema_v1) fn test_code_paid_finalize_then_wait(&mut self, cleanup: SourceOperationError) {
            self.discard_paid_finalize(cleanup).unwrap_or_else(|_| panic!("paid before batch return"));
            assert_eq!(self.next(), Some(LifecycleAction::AwaitBatchReturn));
            self.test_code_assert_transaction_barrier();
        }
        pub(in crate::database::global_schema_v1) fn test_code_observe_fixed_return_ok(&mut self) {
            self.retain_batch_return(BatchKind::Begin, BatchReturnedFact::Ok).unwrap();
            assert!(matches!(self.fields.native.tx.phase, TxPhase::Active)); self.assert_a00_saved();
            assert!(self.fields.native.tx.begin_consumed.is_some());
            assert_eq!(self.retain_batch_return(BatchKind::Begin, BatchReturnedFact::Ok), Err(ProtocolFault::UnexpectedObservation));
            self.test_code_assert_transaction_barrier();
        }
        pub(in crate::database::global_schema_v1) fn test_code_normal_early_primary(&mut self, primary: SourceOperationError, spare: SourceOperationError) {
            let SourceOperationError::Global(super::super::super::GlobalSchemaV1Error::SelectionSnapshotChanged { detail }) = &spare else { panic!("fixed spare"); };
            let spare_allocation = detail.as_ptr() as usize;
            self.retain_early_primary(primary).unwrap_or_else(|_| panic!("single early G primary"));
            let Err(spare) = self.retain_early_primary(spare) else { panic!("second G primary must be returned unchanged"); };
            let SourceOperationError::Global(super::super::super::GlobalSchemaV1Error::SelectionSnapshotChanged { detail }) = &spare else { panic!("same spare"); };
            assert_eq!(detail.as_ptr() as usize, spare_allocation); drop(spare);
            self.start_early_error().unwrap();
            assert_eq!(self.start_early_error(), Err(ProtocolFault::UnexpectedObservation));
        }
        pub(in crate::database::global_schema_v1) fn test_code_autocommit_one(&mut self) {
            self.observe_autocommit(1).unwrap();
            assert_eq!(self.observe_autocommit(0), Err(ProtocolFault::UnexpectedObservation));
            assert!(matches!(self.fields.native.tx.phase, TxPhase::Finished));
            assert!(matches!(self.fields.native.tx.rollback, RollbackObservation::NotReached));
            assert!(self.fields.native.transaction_release_ready(&self.fields.work, self.fields.physical));
        }
        pub(in crate::database::global_schema_v1) fn test_code_normal_rollback_error(&mut self, ignored: SourceOperationError, cleanup: SourceOperationError) {
            let SourceOperationError::Global(super::super::super::GlobalSchemaV1Error::SelectionSnapshotChanged { detail }) = &ignored else { panic!("fixed owned ignored result"); };
            let allocation = detail.as_ptr() as usize;
            self.observe_autocommit(0).unwrap(); assert_eq!(self.early_exit_port().sql(), "ROLLBACK");
            self.fixed_install_batch(BatchKind::Rollback);
            self.early_exit_port().observe(BatchObservation::Step(rusqlite::ffi::SQLITE_ERROR)).unwrap();
            self.test_code_assert_transaction_barrier();
            self.retain_ignored_rollback(ignored).unwrap_or_else(|_| panic!("owned ignored result held in same frame"));
            { let _short = self.early_exit_port(); }
            let Some(SourceOperationError::Global(super::super::super::GlobalSchemaV1Error::SelectionSnapshotChanged { detail })) = &self.fields.native.tx.ignored_rollback else { panic!("retained ignored result"); };
            assert_eq!(detail.as_ptr() as usize, allocation);
            self.early_exit_port().observe(BatchObservation::Finalize(rusqlite::ffi::SQLITE_ERROR)).unwrap();
            self.test_code_assert_transaction_barrier();
            self.discard_paid_finalize(cleanup).unwrap_or_else(|_| panic!("ignored finalize paid once"));
            self.retain_batch_return(BatchKind::Rollback, BatchReturnedFact::Error).unwrap();
            self.test_code_assert_transaction_barrier();
            self.discard_ignored_rollback().unwrap();
            assert_eq!(self.discard_ignored_rollback(), Err(ProtocolFault::UnexpectedObservation));
            assert_eq!(self.observe_autocommit(0), Err(ProtocolFault::UnexpectedObservation));
            assert!(self.fields.native.transaction_release_ready(&self.fields.work, self.fields.physical));
        }
        pub(in crate::database::global_schema_v1) fn test_code_rollback_tail_error(&mut self) {
            self.observe_autocommit(0).unwrap(); self.fixed_install_batch(BatchKind::Rollback);
            self.early_exit_port().observe(BatchObservation::Step(rusqlite::ffi::SQLITE_DONE)).unwrap();
            self.early_exit_port().observe(BatchObservation::Finalize(rusqlite::ffi::SQLITE_OK)).unwrap();
            self.retain_batch_return(BatchKind::Rollback, BatchReturnedFact::Error).unwrap();
            assert_eq!(self.next(), Some(LifecycleAction::RetainIgnoredRollback));
            assert!(self.fields.native.tx.ignored_rollback.is_none()); self.test_code_assert_transaction_barrier();
        }
        pub(in crate::database::global_schema_v1) fn test_code_consume_known_error_at_terminal(&mut self) {
            let before = self.fields.work.test_code_observation();
            assert!(before.terminal.is_some()); assert_eq!(self.next(), Some(LifecycleAction::ConsumeKnownBatchReturn));
            self.test_code_assert_transaction_barrier();
            assert_eq!(self.retain_batch_return(BatchKind::Begin, BatchReturnedFact::Error), Err(ProtocolFault::UnexpectedObservation));
            assert_eq!(self.retain_batch_return(BatchKind::Rollback, BatchReturnedFact::Error), Err(ProtocolFault::UnexpectedObservation));
            self.consume_known_return().unwrap();
            assert_eq!(self.consume_known_return(), Err(ProtocolFault::UnexpectedObservation));
            assert!(self.fields.native.transaction_release_ready(&self.fields.work, self.fields.physical));
            assert!(self.fields.native.tx.ignored_rollback.is_none());
            assert_eq!(self.fields.work.test_code_observation(), before);
        }
        pub(in crate::database::global_schema_v1) fn test_code_terminal_exit(&mut self) {
            assert!(self.fields.work.terminal().is_some()); assert!(self.fields.physical.primary.is_none());
            let before = self.fields.work.test_code_observation();
            self.start_early_error().unwrap(); self.observe_autocommit(0).unwrap();
            assert_eq!(self.next(), Some(LifecycleAction::PrepareRollback)); self.test_code_assert_transaction_barrier();
            self.fixed_install_batch(BatchKind::Rollback);
            assert_eq!(self.next(), Some(LifecycleAction::StepRollback)); self.test_code_assert_transaction_barrier();
            assert_eq!(self.early_exit_port().observe(BatchObservation::Prepare(rusqlite::ffi::SQLITE_OK)), Err(ProtocolFault::UnexpectedObservation));
            { let _short = self.early_exit_port(); }
            self.early_exit_port().observe(BatchObservation::Step(rusqlite::ffi::SQLITE_ERROR)).unwrap();
            assert_eq!(self.next(), Some(LifecycleAction::FinalizeRollback)); self.test_code_assert_transaction_barrier();
            assert_eq!(self.early_exit_port().observe(BatchObservation::Step(rusqlite::ffi::SQLITE_DONE)), Err(ProtocolFault::UnexpectedObservation));
            self.early_exit_port().observe(BatchObservation::Finalize(rusqlite::ffi::SQLITE_ERROR)).unwrap();
            assert_eq!(self.early_exit_port().observe(BatchObservation::Finalize(rusqlite::ffi::SQLITE_OK)), Err(ProtocolFault::UnexpectedObservation));
            self.test_code_assert_transaction_barrier();
            self.retain_batch_return(BatchKind::Rollback, BatchReturnedFact::Error).unwrap();
            assert_eq!(self.retain_batch_return(BatchKind::Rollback, BatchReturnedFact::Error), Err(ProtocolFault::UnexpectedObservation));
            assert!(self.fields.native.tx.ignored_rollback.is_none());
            assert!(matches!(self.fields.native.tx.phase, TxPhase::Finished));
            assert_eq!(self.observe_autocommit(0), Err(ProtocolFault::UnexpectedObservation));
            assert_eq!(self.early_exit_port().observe(BatchObservation::Step(rusqlite::ffi::SQLITE_DONE)), Err(ProtocolFault::UnexpectedObservation));
            assert!(self.fields.native.transaction_release_ready(&self.fields.work, self.fields.physical));
            let adverse = self.fields.native.secondary.as_ref().unwrap();
            // First adverse is BEGIN's consumed finalize; rollback cannot
            // overwrite it or the first terminal.
            assert_eq!(adverse.action, FixedAction::Begin); assert_eq!(adverse.code, rusqlite::ffi::SQLITE_ERROR);
            assert_eq!(self.fields.work.test_code_observation(), before);
        }
    }

    #[repr(C, u8)]
    enum BorrowedCell<'v> {
        Null, Integer(i64), RealBits(u64), Text(&'v [u8]), Blob(&'v [u8]),
    }
    struct StatementAccess<'v> {
        connection: &'v mut RawConnection,
        slot: &'v mut StmtSlot,
    }
    // Cells cannot escape a same-connection mutable borrow. No native cell
    // operation is implemented until pointer/status/view rules are selected.

    #[repr(u8)]
    enum FsAction { JournalAbsent, WalIdentity, ShmIdentity, UnlinkWal, UnlinkShm,
        DirectorySync, SuffixAbsent, NamespaceIdentity }
    #[repr(C, u8)]
    enum FsResult { NotCalled, Ok, Errno(i32), IdentityMismatch }
    struct FsObservation { action: FsAction, ordinal: usize, result: FsResult }
    #[repr(u8)]
    enum FileRelease { NotTaken, TakenAndDropped }
    #[repr(u8)]
    enum GuardRelease { NotTaken, TakenAndReleased }
    struct AuditReleaseStatus {
        unlock: CodeSlot,
        file: FileRelease,
        guard: GuardRelease,
    }
    pub(in crate::database::global_schema_v1) struct FixedDrainLedger {
        first_secondary: Option<FixedAdverse>,
        filesystem: FsObservation,
        audit: AuditReleaseStatus,
    }
    impl FixedDrainLedger {
        pub(in crate::database::global_schema_v1) fn from_start_decision(
            _decision: &super::super::super::FinancialRetainedStartDecision,
        ) -> Self {
            Self {
                first_secondary: None,
                filesystem: FsObservation {
                    action: FsAction::NamespaceIdentity,
                    ordinal: 0,
                    result: FsResult::NotCalled,
                },
                audit: AuditReleaseStatus {
                    unlock: CodeSlot::NotCalled,
                    file: FileRelease::NotTaken,
                    guard: GuardRelease::NotTaken,
                },
            }
        }
    }

    // Short sibling-field borrow, deliberately separate from OriginalSqlLoan.
    // Creating or dropping it supplies no selected native rules or cleanup.
    pub(in crate::database::global_schema_v1) struct OriginalOwnerFields<'a> {
        native: &'a mut NativeOriginalOwner,
        work: OriginalSourceWork<'a>,
        release: &'a mut FixedDrainLedger,
        physical: &'a mut super::super::super::FinancialPhysical,
    }
    impl<'a> OriginalOwnerFields<'a> {
        pub(in crate::database::global_schema_v1) fn lend(
            native: &'a mut NativeOriginalOwner,
            work: OriginalSourceWork<'a>,
            release: &'a mut FixedDrainLedger,
            physical: &'a mut super::super::super::FinancialPhysical,
        ) -> Self {
            Self { native, work, release, physical }
        }
        pub(in crate::database::global_schema_v1) fn reborrow(&mut self) -> OriginalOwnerFields<'_> {
            OriginalOwnerFields {
                native: &mut *self.native,
                work: self.work.reborrow(),
                release: &mut *self.release,
                physical: &mut *self.physical,
            }
        }
        pub(in crate::database::global_schema_v1) fn sidecar_loan(&mut self) -> super::super::super::FinancialSidecarLoan<'_> {
            super::super::super::FinancialSidecarLoan {
                physical: &mut *self.physical,
                work: self.work.reborrow(),
            }
        }
        pub(in crate::database::global_schema_v1) fn source_work(&mut self) -> OriginalSourceWork<'_> {
            self.work.reborrow()
        }
        #[cfg(test)]
        pub(in crate::database::global_schema_v1) fn test_code_unreached(&self) -> bool {
            matches!(&self.native.original, OriginalPlace::Empty)
                && matches!(&self.native.aux, AuxPlace::Empty)
                && self.native.statements.iter().all(|slot| matches!(slot, StmtSlot::Vacant))
                && matches!(&self.native.statement_phase, StatementPhase::Empty)
                && matches!(&self.native.tx.phase, TxPhase::NotCreated)
                && matches!(&self.native.tx.exit, TxExit::NotSelected)
                && matches!(&self.native.tx.autocommit, AutocommitObservation::NotObserved)
                && matches!(&self.native.tx.rollback, RollbackObservation::NotReached)
                && self.native.secondary.is_none()
                && self.release.first_secondary.is_none()
                && matches!(&self.release.filesystem.result, FsResult::NotCalled)
                && matches!(&self.release.audit.unlock, CodeSlot::NotCalled)
                && matches!(&self.release.audit.file, FileRelease::NotTaken)
                && matches!(&self.release.audit.guard, GuardRelease::NotTaken)
        }
    }

    // Fixed A01/A02 query_row prefix. No native pointers are installed here.
    // Actual options/FS/hook/driver return facts require the unissued adapter;
    // private cfg scripts below exercise only this shared ownership protocol.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum InitialContext { Unbound, Unknown, Valid, Refused }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum InitialStage { BeforeBegin, ZeroWal, BeforeCapture, Main, Temp, AppId, Version, ForeignKeys, Journal, Sync, Reached, Stopped }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum InitialQuery { Main, Temp, AppId, Version, ForeignKeys, Journal, Sync }
    impl InitialQuery {
        fn action(self) -> FixedAction { match self { Self::Main | Self::Temp => FixedAction::ProspectiveExtent, _ => FixedAction::Pragmas } }
        fn index(self) -> usize { match self { Self::Main => 0, Self::Temp => 1, Self::AppId => 2, Self::Version => 3, Self::ForeignKeys => 4, Self::Journal => 5, Self::Sync => 6 } }
        fn next_stage(self) -> InitialStage { match self { Self::Main => InitialStage::Temp, Self::Temp => InitialStage::AppId,
            Self::AppId => InitialStage::Version, Self::Version => InitialStage::ForeignKeys,
            Self::ForeignKeys => InitialStage::Journal, Self::Journal => InitialStage::Sync, Self::Sync => InitialStage::Reached } }
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum InitialPhase { Dormant, Prepare, Tail, Bind, Step, Type0, Value0, Type1, Value1, Text, Reset, Primary, NeedResetError, NeedFinalizeError, AwaitReturn, Wrap, Finished }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum InitialOutcome { Unknown, FirstRow, NoRow, Error }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum InitialOwnedCleanup { Reset, Finalize }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum InitialReturnedFact { Ok, Error }
    struct InitialReadRecord {
        context: InitialContext,
        stage: InitialStage,
        phase: InitialPhase,
        prepare: CodeSlot,
        outcome: InitialOutcome,
        rows_started: bool,
        returned: BatchReturn,
        ignored: Option<(InitialOwnedCleanup, rusqlite::Error)>,
        driver_error: Option<rusqlite::Error>,
        consumed: [Option<StmtState>; 7],
        extent_fault: Option<super::super::super::FinancialExtentFault>,
    }
    impl InitialReadRecord {
        fn empty() -> Self { Self { context: InitialContext::Unbound, stage: InitialStage::BeforeBegin,
            phase: InitialPhase::Dormant, prepare: CodeSlot::NotCalled, outcome: InitialOutcome::Unknown,
            rows_started: false, returned: BatchReturn::Unobserved, ignored: None, driver_error: None, consumed: [None; 7], extent_fault: None } }
        fn query(&self) -> Option<InitialQuery> { match self.stage {
            InitialStage::Main => Some(InitialQuery::Main), InitialStage::Temp => Some(InitialQuery::Temp),
            InitialStage::AppId => Some(InitialQuery::AppId), InitialStage::Version => Some(InitialQuery::Version),
            InitialStage::ForeignKeys => Some(InitialQuery::ForeignKeys), InitialStage::Journal => Some(InitialQuery::Journal),
            InitialStage::Sync => Some(InitialQuery::Sync), _ => None,
        } }
    }
    impl NativeOriginalOwner {
        fn initial_read_action(&self, work: &OriginalSourceWork<'_>, physical: &super::super::super::FinancialPhysical) -> Option<LifecycleAction> {
            let r = &self.initial_read;
            if r.context == InitialContext::Unbound || (r.stage == InitialStage::Stopped && r.ignored.is_none() && r.driver_error.is_none()) { return None; }
            if r.ignored.is_some() { return Some(LifecycleAction::DiscardInitialOwnedCleanup); }
            let terminal = work.terminal().is_some();
            if terminal {
                if let Some(state) = self.statements[0].live() {
                    // A BEGIN/ROLLBACK VM belongs to its existing batch port.
                    if !matches!(state.action, FixedAction::ProspectiveExtent | FixedAction::Pragmas) { return None; }
                    return Some(if r.rows_started && matches!(state.reset, CodeSlot::NotCalled) {
                        LifecycleAction::ResetInitialRead
                    } else { LifecycleAction::FinalizeInitialRead });
                }
                return Some(if r.driver_error.is_some() { LifecycleAction::DiscardInitialDriverError } else { LifecycleAction::StopInitialRead });
            }
            // First G primary forbids new read work through either producer.
            // Existing raw/ignored Results and a genuinely started call retain
            // their own cleanup/whole-return obligations before fixed TX exit.
            if physical.primary.is_some() {
                if r.phase == InitialPhase::NeedResetError { return Some(LifecycleAction::RetainInitialResetError); }
                if r.phase == InitialPhase::NeedFinalizeError { return Some(LifecycleAction::RetainInitialFinalizeError); }
                if r.phase == InitialPhase::Primary && r.query().is_some() && r.extent_fault.is_none()
                    && r.driver_error.is_none() { return Some(LifecycleAction::RetainInitialDriverError); }
                if let Some(state) = self.statements[0].live() {
                    if !matches!(state.action, FixedAction::ProspectiveExtent | FixedAction::Pragmas) { return None; }
                    return Some(if r.rows_started && matches!(state.reset, CodeSlot::NotCalled) {
                        LifecycleAction::ResetInitialRead
                    } else { LifecycleAction::FinalizeInitialRead });
                }
                if r.phase == InitialPhase::AwaitReturn || (r.driver_error.is_some() && r.returned == BatchReturn::Unobserved) {
                    return Some(LifecycleAction::AwaitInitialQueryReturn);
                }
                return Some(if r.driver_error.is_some() { LifecycleAction::DiscardInitialDriverError } else { LifecycleAction::StopInitialRead });
            }
            if r.context == InitialContext::Unknown { return Some(LifecycleAction::ValidateInitialOptions); }
            if r.context == InitialContext::Refused {
                return Some(if physical.primary.is_none() { LifecycleAction::RetainInitialPrimary } else { LifecycleAction::StopInitialRead });
            }
            if !matches!(self.tx.phase, TxPhase::Active) { return None; }
            if r.phase == InitialPhase::Primary { return Some(if r.query().is_some() && r.extent_fault.is_none() { LifecycleAction::RetainInitialDriverError } else { LifecycleAction::RetainInitialPrimary }); }
            if r.phase == InitialPhase::Wrap { return Some(LifecycleAction::WrapInitialDriverError); }
            if r.phase == InitialPhase::NeedResetError { return Some(LifecycleAction::RetainInitialResetError); }
            if r.phase == InitialPhase::NeedFinalizeError { return Some(LifecycleAction::RetainInitialFinalizeError); }
            if r.phase == InitialPhase::AwaitReturn { return Some(LifecycleAction::AwaitInitialQueryReturn); }
            Some(match r.stage {
                InitialStage::ZeroWal => LifecycleAction::RequireZeroOwnedWal,
                InitialStage::BeforeCapture => LifecycleAction::BeforeInitialCapture,
                InitialStage::Reached => LifecycleAction::InitialPrefixReached,
                _ => match r.phase {
                    InitialPhase::Prepare => LifecycleAction::PrepareInitialRead,
                    InitialPhase::Tail => LifecycleAction::CheckInitialNoTail,
                    InitialPhase::Bind => LifecycleAction::BindInitialEmpty,
                    InitialPhase::Step => LifecycleAction::StepInitialRead,
                    InitialPhase::Type0 | InitialPhase::Type1 => LifecycleAction::InitialColumnType,
                    InitialPhase::Value0 | InitialPhase::Value1 => LifecycleAction::InitialInteger,
                    InitialPhase::Text => LifecycleAction::InitialJournalText,
                    InitialPhase::Reset => LifecycleAction::ResetInitialRead,
                    InitialPhase::Finished if self.statements[0].live().is_some() => LifecycleAction::FinalizeInitialRead,
                    _ => return None,
                },
            })
        }
    }
    pub(in crate::database::global_schema_v1) struct OriginalInitialReadLoan<'a, 'purpose> {
        fields: OriginalOwnerFields<'a>,
        prefix: &'a mut super::super::super::FinancialInitialReadState<'purpose>,
    }
    // Named ports select only fixed statements; this is no caller SQL getter.
    struct OriginalInitialReadPort<'s, 'a, 'purpose, 'r> {
        loan: &'s mut OriginalInitialReadLoan<'a, 'purpose>,
        kind: InitialQuery,
        _rules: &'r SelectedOriginalNativeRules,
    }
    struct OriginalInitialPrerequisitePort<'s, 'a, 'purpose, 'r> {
        loan: &'s mut OriginalInitialReadLoan<'a, 'purpose>,
        _rules: &'r SelectedOriginalNativeRules,
    }
    struct OriginalInitialOuterWrapPort<'s, 'a, 'purpose, 'r> {
        loan: &'s mut OriginalInitialReadLoan<'a, 'purpose>,
        _rules: &'r SelectedOriginalNativeRules,
    }
    impl OriginalInitialOuterWrapPort<'_, '_, '_, '_> {
        fn extent_catalog_diagnostic(&mut self, paid_detail: String) -> Result<(), String> { self.loan.retain_catalog_wrapped_detail(paid_detail) }
        fn pragma_driver_error(&mut self) -> Result<(), ProtocolFault> { self.loan.retain_pragma_wrapped_error() }
    }
    struct OriginalInitialRowLoan<'s, 'a, 'purpose> { loan: &'s mut OriginalInitialReadLoan<'a, 'purpose> }
    struct OriginalJournalTextLoan<'row, 'short, 's, 'a, 'purpose> {
        row: &'short mut OriginalInitialRowLoan<'s, 'a, 'purpose>,
        bytes: &'row [u8],
    }
    impl<'a> OriginalOwnerFields<'a> {
        pub(in crate::database::global_schema_v1) fn bind_initial_read_context(&mut self) -> bool {
            if self.work.terminal().is_some() || self.physical.primary.is_some()
                || self.native.initial_read.context != InitialContext::Unbound
                || self.native.tx.begin.phase != BatchPhase::Dormant { return false; }
            self.native.initial_read.context = InitialContext::Unknown; true
        }
        pub(in crate::database::global_schema_v1) fn initial_read<'purpose>(self, prefix: &'a mut super::super::super::FinancialInitialReadState<'purpose>)
            -> OriginalInitialReadLoan<'a, 'purpose> { OriginalInitialReadLoan { fields: self, prefix } }
    }
    impl<'a, 'purpose> OriginalInitialReadLoan<'a, 'purpose> {
        fn next(&self) -> Option<LifecycleAction> { self.fields.native.initial_read_action(&self.fields.work, self.fields.physical) }
        fn outer_wrap_port<'s, 'r>(&'s mut self, rules: &'r SelectedOriginalNativeRules) -> OriginalInitialOuterWrapPort<'s, 'a, 'purpose, 'r> { OriginalInitialOuterWrapPort { loan: self, _rules: rules } }
        fn prerequisite_port<'s, 'r>(&'s mut self, rules: &'r SelectedOriginalNativeRules) -> OriginalInitialPrerequisitePort<'s, 'a, 'purpose, 'r> {
            OriginalInitialPrerequisitePort { loan: self, _rules: rules }
        }
        fn fixed_port<'s, 'r>(&'s mut self, kind: InitialQuery, rules: &'r SelectedOriginalNativeRules)
            -> Result<OriginalInitialReadPort<'s, 'a, 'purpose, 'r>, ProtocolFault> {
            if self.fields.native.initial_read.query() != Some(kind) { return Err(ProtocolFault::UnexpectedObservation); }
            Ok(OriginalInitialReadPort { loan: self, kind, _rules: rules })
        }
        fn main_extent<'s, 'r>(&'s mut self, rules: &'r SelectedOriginalNativeRules) -> Result<OriginalInitialReadPort<'s, 'a, 'purpose, 'r>, ProtocolFault> { self.fixed_port(InitialQuery::Main, rules) }
        fn temp_extent<'s, 'r>(&'s mut self, rules: &'r SelectedOriginalNativeRules) -> Result<OriginalInitialReadPort<'s, 'a, 'purpose, 'r>, ProtocolFault> { self.fixed_port(InitialQuery::Temp, rules) }
        fn application_id<'s, 'r>(&'s mut self, rules: &'r SelectedOriginalNativeRules) -> Result<OriginalInitialReadPort<'s, 'a, 'purpose, 'r>, ProtocolFault> { self.fixed_port(InitialQuery::AppId, rules) }
        fn user_version<'s, 'r>(&'s mut self, rules: &'r SelectedOriginalNativeRules) -> Result<OriginalInitialReadPort<'s, 'a, 'purpose, 'r>, ProtocolFault> { self.fixed_port(InitialQuery::Version, rules) }
        fn foreign_keys<'s, 'r>(&'s mut self, rules: &'r SelectedOriginalNativeRules) -> Result<OriginalInitialReadPort<'s, 'a, 'purpose, 'r>, ProtocolFault> { self.fixed_port(InitialQuery::ForeignKeys, rules) }
        fn journal_mode<'s, 'r>(&'s mut self, rules: &'r SelectedOriginalNativeRules) -> Result<OriginalInitialReadPort<'s, 'a, 'purpose, 'r>, ProtocolFault> { self.fixed_port(InitialQuery::Journal, rules) }
        fn synchronous<'s, 'r>(&'s mut self, rules: &'r SelectedOriginalNativeRules) -> Result<OriginalInitialReadPort<'s, 'a, 'purpose, 'r>, ProtocolFault> { self.fixed_port(InitialQuery::Sync, rules) }
        fn observe_validation(&mut self, fact: InitialReturnedFact) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::ValidateInitialOptions) { return Err(ProtocolFault::UnexpectedObservation); }
            self.fields.native.initial_read.context = if fact == InitialReturnedFact::Ok { InitialContext::Valid } else { InitialContext::Refused };
            self.fields.native.initial_read.stage = InitialStage::ZeroWal; Ok(())
        }
        fn observe_prerequisite(&mut self, fact: InitialReturnedFact) -> Result<(), ProtocolFault> {
            let next = self.next();
            if !matches!(next, Some(LifecycleAction::RequireZeroOwnedWal | LifecycleAction::BeforeInitialCapture)) { return Err(ProtocolFault::UnexpectedObservation); }
            if fact == InitialReturnedFact::Error { self.fields.native.initial_read.phase = InitialPhase::Primary; return Ok(()); }
            if next == Some(LifecycleAction::RequireZeroOwnedWal) { self.fields.native.initial_read.stage = InitialStage::BeforeCapture; }
            else { self.fields.native.initial_read.stage = InitialStage::Main; self.fields.native.initial_read.phase = InitialPhase::Prepare; }
            Ok(())
        }
        fn begin_query_slot(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::PrepareInitialRead) || self.fields.native.statements[0].live().is_some()
                || !matches!(self.fields.native.initial_read.prepare, CodeSlot::NotCalled) { return Err(ProtocolFault::UnexpectedObservation); }
            // Only a consumed scalar ledger is replaced; A00/BEGIN already
            // retain their ledgers. There is still one actual VM slot.
            self.fields.native.statements[0] = StmtSlot::Vacant; Ok(())
        }
        fn adverse(&mut self, code: i32) {
            if code == rusqlite::ffi::SQLITE_OK { return; }
            let action = self.fields.native.initial_read.query().map_or(FixedAction::Pragmas, InitialQuery::action);
            if self.fields.native.secondary.is_none() { self.fields.native.secondary = Some(FixedAdverse { role: Role::Original, action, ordinal: 0, code }); }
            if self.fields.release.first_secondary.is_none() { self.fields.release.first_secondary = Some(FixedAdverse { role: Role::Original, action, ordinal: 0, code }); }
        }
        fn observe_prepare(&mut self, code: i32) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::PrepareInitialRead) { return Err(ProtocolFault::UnexpectedObservation); }
            let kind = self.fields.native.initial_read.query().ok_or(ProtocolFault::UnexpectedObservation)?;
            if code == rusqlite::ffi::SQLITE_OK {
                let state = self.fields.native.statements[0].live().ok_or(ProtocolFault::ResourceNotInstalled)?;
                if state.action != kind.action() || state.role != Role::Original { return Err(ProtocolFault::UnexpectedObservation); }
            } else if self.fields.native.statements[0].live().is_some() { return Err(ProtocolFault::UnexpectedObservation); }
            record_once(&mut self.fields.native.initial_read.prepare, code)?;
            self.fields.native.initial_read.phase = if code == rusqlite::ffi::SQLITE_OK { InitialPhase::Tail } else { InitialPhase::Primary };
            if code != rusqlite::ffi::SQLITE_OK { self.fields.native.initial_read.outcome = InitialOutcome::Error; self.adverse(code); } Ok(())
        }
        fn observe_tail_or_bind(&mut self, fact: InitialReturnedFact) -> Result<(), ProtocolFault> {
            let next = self.next();
            if !matches!(next, Some(LifecycleAction::CheckInitialNoTail | LifecycleAction::BindInitialEmpty)) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.initial_read;
            if fact == InitialReturnedFact::Error { r.phase = InitialPhase::Primary; r.outcome = InitialOutcome::Error; }
            else if next == Some(LifecycleAction::CheckInitialNoTail) { r.phase = InitialPhase::Bind; }
            else { r.phase = InitialPhase::Step; r.rows_started = true; }
            Ok(())
        }
        fn observe_step(&mut self, code: i32) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::StepInitialRead) { return Err(ProtocolFault::UnexpectedObservation); }
            let state = self.fields.native.statements[0].live_mut().ok_or(ProtocolFault::ResourceNotInstalled)?;
            record_once(&mut state.step, code)?;
            state.cursor = if code == rusqlite::ffi::SQLITE_DONE { CursorPhase::Ended } else { CursorPhase::Active };
            let r = &mut self.fields.native.initial_read;
            match code {
                rusqlite::ffi::SQLITE_ROW => { r.outcome = InitialOutcome::FirstRow; r.phase = InitialPhase::Type0; }
                rusqlite::ffi::SQLITE_DONE => { r.outcome = InitialOutcome::NoRow; r.phase = InitialPhase::Reset; }
                _ => { r.outcome = InitialOutcome::Error; r.phase = InitialPhase::Primary; }
            }
            if code != rusqlite::ffi::SQLITE_ROW && code != rusqlite::ffi::SQLITE_DONE { self.adverse(code); } Ok(())
        }
        fn first_row(&mut self) -> Result<OriginalInitialRowLoan<'_, 'a, 'purpose>, ProtocolFault> {
            if self.fields.work.terminal().is_some() || self.fields.physical.primary.is_some()
                || !matches!(self.next(), Some(LifecycleAction::InitialColumnType | LifecycleAction::InitialInteger | LifecycleAction::InitialJournalText)) {
                return Err(ProtocolFault::UnexpectedObservation);
            }
            Ok(OriginalInitialRowLoan { loan: self })
        }
        fn observe_reset(&mut self, code: i32) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::ResetInitialRead) { return Err(ProtocolFault::UnexpectedObservation); }
            let state = self.fields.native.statements[0].live_mut().ok_or(ProtocolFault::ResourceNotInstalled)?;
            record_once(&mut state.reset, code)?; state.cursor = CursorPhase::NoCursor;
            let r = &mut self.fields.native.initial_read;
            r.phase = if self.fields.work.terminal().is_some() { InitialPhase::Finished }
                else if r.outcome == InitialOutcome::NoRow { InitialPhase::Primary }
                else if code != rusqlite::ffi::SQLITE_OK { InitialPhase::NeedResetError }
                else { InitialPhase::Finished };
            self.adverse(code);
            // DONE propagates reset Err (or then constructs NoRows); ROW and
            // step/conversion Err ignore the reset Result only after payment.
            Ok(())
        }
        fn observe_finalize(&mut self, code: i32) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::FinalizeInitialRead) { return Err(ProtocolFault::UnexpectedObservation); }
            let mut state = *self.fields.native.statements[0].live().ok_or(ProtocolFault::ResourceNotInstalled)?;
            record_once(&mut state.finalize, code)?; state.cursor = CursorPhase::NoCursor;
            self.fields.native.statements[0] = StmtSlot::Finalized(state);
            self.fields.native.initial_read.phase = if code != rusqlite::ffi::SQLITE_OK && self.fields.work.terminal().is_none() {
                InitialPhase::NeedFinalizeError
            } else { InitialPhase::AwaitReturn };
            self.adverse(code); Ok(())
        }
        fn retain_paid_primary(&mut self, error: SourceOperationError) -> Result<(), SourceOperationError> {
            if self.fields.work.terminal().is_some() || self.fields.physical.primary.is_some()
                || self.next() != Some(LifecycleAction::RetainInitialPrimary) { return Err(error); }
            self.fields.physical.primary = Some(error);
            self.fields.physical.audit_phase = super::super::super::FinancialAuditPhase::Failed;
            let r = &mut self.fields.native.initial_read;
            if r.context == InitialContext::Refused || r.query().is_none() || r.extent_fault.is_some() {
                r.stage = InitialStage::Stopped; r.phase = InitialPhase::Finished;
            } else if let Some(state) = self.fields.native.statements[0].live() {
                r.phase = if r.rows_started && matches!(state.reset, CodeSlot::NotCalled) { InitialPhase::Reset } else { InitialPhase::Finished };
            } else if r.returned == BatchReturn::ReturnedError { r.stage = InitialStage::Stopped; r.phase = InitialPhase::Finished; }
            else { r.phase = InitialPhase::AwaitReturn; }
            Ok(())
        }
        fn retain_paid_driver_error(&mut self, error: rusqlite::Error) -> Result<(), rusqlite::Error> {
            if self.fields.work.terminal().is_some() || self.fields.native.initial_read.driver_error.is_some()
                || self.next() != Some(LifecycleAction::RetainInitialDriverError) { return Err(error); }
            let r = &mut self.fields.native.initial_read;
            r.driver_error = Some(error);
            r.phase = if let Some(state) = self.fields.native.statements[0].live() {
                if r.rows_started && matches!(state.reset, CodeSlot::NotCalled) { InitialPhase::Reset } else { InitialPhase::Finished }
            } else if r.returned == BatchReturn::ReturnedError { InitialPhase::Wrap } else { InitialPhase::AwaitReturn };
            Ok(())
        }
        fn wrap_ready(&self) -> bool {
            self.next() == Some(LifecycleAction::WrapInitialDriverError)
                && self.fields.native.initial_read.returned == BatchReturn::ReturnedError
                && self.fields.native.statements[0].live().is_none()
                && self.fields.native.initial_read.ignored.is_none()
                && self.fields.native.initial_read.driver_error.is_some()
                && self.fields.physical.primary.is_none()
        }
        fn retain_catalog_wrapped_detail(&mut self, paid_detail: String) -> Result<(), String> {
            if !self.wrap_ready() || !matches!(self.fields.native.initial_read.query(), Some(InitialQuery::Main | InitialQuery::Temp)) { return Err(paid_detail); }
            // Future C formatter produces this already-paid owned detail while
            // the raw error is still retained. No formatting/copy happens here.
            self.fields.physical.primary = Some(super::super::super::retain_initial_extent_diagnostic(paid_detail));
            // Mapping remains pending throughout raw consumption; no ledger
            // is marked stopped/consumed before the wrapper is parent-retained.
            drop(self.fields.native.initial_read.driver_error.take());
            self.finish_wrapped_primary(); Ok(())
        }
        fn retain_pragma_wrapped_error(&mut self) -> Result<(), ProtocolFault> {
            if !self.wrap_ready() { return Err(ProtocolFault::UnexpectedObservation); }
            let operation = match self.fields.native.initial_read.query() {
                Some(InitialQuery::AppId) => super::super::super::FinancialInitialPragma::ApplicationId,
                Some(InitialQuery::Version) => super::super::super::FinancialInitialPragma::UserVersion,
                Some(InitialQuery::ForeignKeys) => super::super::super::FinancialInitialPragma::ForeignKeys,
                Some(InitialQuery::Journal) => super::super::super::FinancialInitialPragma::JournalMode,
                Some(InitialQuery::Sync) => super::super::super::FinancialInitialPragma::Synchronous, _ => return Err(ProtocolFault::UnexpectedObservation),
            };
            let raw = self.fields.native.initial_read.driver_error.take().ok_or(ProtocolFault::UnexpectedObservation)?;
            self.fields.physical.primary = Some(super::super::super::retain_initial_pragma_driver_error(operation, raw));
            self.finish_wrapped_primary(); Ok(())
        }
        fn finish_wrapped_primary(&mut self) {
            self.fields.physical.audit_phase = super::super::super::FinancialAuditPhase::Failed;
            self.fields.native.initial_read.stage = InitialStage::Stopped;
            self.fields.native.initial_read.phase = InitialPhase::Finished;
        }
        fn discard_owned_driver_error(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::DiscardInitialDriverError) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.initial_read;
            if let Some(kind) = r.query() {
                if matches!(r.prepare, CodeSlot::Called(rusqlite::ffi::SQLITE_OK)) {
                    if let StmtSlot::Finalized(state) = &self.fields.native.statements[0] {
                        if state.action == kind.action() { r.consumed[kind.index()] = Some(*state); }
                    }
                }
            }
            drop(r.driver_error.take());
            // The selector permits normal discard only after the actual whole
            // Error return with a first G primary; terminal uses primitive drain.
            self.fields.native.initial_read.stage = InitialStage::Stopped;
            self.fields.native.initial_read.phase = InitialPhase::Finished; Ok(())
        }
        fn retain_paid_cleanup(&mut self, error: rusqlite::Error) -> Result<(), rusqlite::Error> {
            if self.fields.work.terminal().is_some() || self.fields.native.initial_read.ignored.is_some() { return Err(error); }
            let kind = match self.next() { Some(LifecycleAction::RetainInitialResetError) => InitialOwnedCleanup::Reset,
                Some(LifecycleAction::RetainInitialFinalizeError) => InitialOwnedCleanup::Finalize, _ => return Err(error) };
            self.fields.native.initial_read.ignored = Some((kind, error)); Ok(())
        }
        fn discard_owned_cleanup(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::DiscardInitialOwnedCleanup) { return Err(ProtocolFault::UnexpectedObservation); }
            let (kind, error) = self.fields.native.initial_read.ignored.take().ok_or(ProtocolFault::UnexpectedObservation)?;
            drop(error); self.fields.native.initial_read.phase = if kind == InitialOwnedCleanup::Reset { InitialPhase::Finished } else { InitialPhase::AwaitReturn }; Ok(())
        }
        fn row_values_complete(&self, kind: InitialQuery) -> bool {
            match kind {
                InitialQuery::Main | InitialQuery::Temp => self.prefix.count.is_some() && self.prefix.extent.is_some(),
                InitialQuery::AppId => self.prefix.application_id.is_some(), InitialQuery::Version => self.prefix.user_version.is_some(),
                InitialQuery::ForeignKeys => self.prefix.foreign_keys.is_some(), InitialQuery::Journal => self.prefix.journal_mode.is_some(),
                InitialQuery::Sync => self.prefix.synchronous.is_some(),
            }
        }
        fn retain_query_return(&mut self, fact: InitialReturnedFact) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitInitialQueryReturn) || self.fields.native.initial_read.returned != BatchReturn::Unobserved || self.fields.native.statements[0].live().is_some() { return Err(ProtocolFault::UnexpectedObservation); }
            let kind = self.fields.native.initial_read.query().ok_or(ProtocolFault::UnexpectedObservation)?;
            let values_complete = self.row_values_complete(kind);
            let first_primary = self.fields.physical.primary.is_some();
            let r = &mut self.fields.native.initial_read;
            if fact == InitialReturnedFact::Ok {
                if r.outcome != InitialOutcome::FirstRow || !values_complete || r.driver_error.is_some() { return Err(ProtocolFault::UnexpectedObservation); }
                if let StmtSlot::Finalized(state) = &self.fields.native.statements[0] { r.consumed[kind.index()] = Some(*state); }
                else { return Err(ProtocolFault::ResourceNotInstalled); }
                r.returned = BatchReturn::ReturnedOk;
                if first_primary { r.phase = InitialPhase::Finished; return Ok(()); }
                if matches!(kind, InitialQuery::Main | InitialQuery::Temp) {
                    let schema = if kind == InitialQuery::Main { super::super::super::FinancialExtentSchema::Main } else { super::super::super::FinancialExtentSchema::Temp };
                    if let Err(fault) = self.prefix.finish_extent(schema) { r.extent_fault = Some(fault); r.phase = InitialPhase::Primary; return Ok(()); }
                }
                r.stage = kind.next_stage(); r.phase = if r.stage == InitialStage::Reached { InitialPhase::Finished } else { InitialPhase::Prepare };
                r.prepare = CodeSlot::NotCalled; r.outcome = InitialOutcome::Unknown; r.rows_started = false; r.returned = BatchReturn::Unobserved;
            } else {
                // Interrupted, incomplete read work can observe an independent
                // Error return; a completed typed ROW cannot be relabeled Error.
                if !matches!(r.outcome, InitialOutcome::Error | InitialOutcome::NoRow)
                    && !(first_primary && !values_complete) { return Err(ProtocolFault::UnexpectedObservation); }
                r.returned = BatchReturn::ReturnedError;
                // Do not infer this fact from primary/step/reset/finalize status.
                if r.driver_error.is_some() { r.phase = InitialPhase::Wrap; }
                else { r.phase = InitialPhase::Primary; }
            } Ok(())
        }
        fn stop_terminal_or_failed_prefix(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::StopInitialRead) || self.fields.native.statements[0].live().is_some()
                || self.fields.native.initial_read.ignored.is_some() || self.fields.native.initial_read.driver_error.is_some() { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.initial_read;
            if let Some(kind) = r.query() {
                // The shared slot may still hold consumed BEGIN or the prior
                // query. An unissued successor has no consumed read ledger.
                if matches!(r.prepare, CodeSlot::Called(rusqlite::ffi::SQLITE_OK)) {
                    if let StmtSlot::Finalized(state) = &self.fields.native.statements[0] {
                        if state.action == kind.action() { r.consumed[kind.index()] = Some(*state); }
                    }
                }
            }
            r.stage = InitialStage::Stopped; r.phase = InitialPhase::Finished; Ok(())
        }
    }
    impl OriginalInitialPrerequisitePort<'_, '_, '_, '_> {
        fn rows_backup_options_validation(&mut self, fact: InitialReturnedFact) -> Result<(), ProtocolFault> { self.loan.observe_validation(fact) }
        fn require_zero_owned_wal(&mut self, fact: InitialReturnedFact) -> Result<(), ProtocolFault> {
            if self.loan.next() != Some(LifecycleAction::RequireZeroOwnedWal) { return Err(ProtocolFault::UnexpectedObservation); }
            self.loan.observe_prerequisite(fact)
        }
        fn before_initial_capture(&mut self, fact: InitialReturnedFact) -> Result<(), ProtocolFault> {
            if self.loan.next() != Some(LifecycleAction::BeforeInitialCapture) { return Err(ProtocolFault::UnexpectedObservation); }
            self.loan.observe_prerequisite(fact)
        }
    }
    impl OriginalInitialReadPort<'_, '_, '_, '_> {
        fn prepare_slot(&mut self) -> Result<(), ProtocolFault> { self.loan.begin_query_slot() }
        fn prepared(&mut self, code: i32) -> Result<(), ProtocolFault> { self.loan.observe_prepare(code) }
        fn checked_no_tail(&mut self, fact: InitialReturnedFact) -> Result<(), ProtocolFault> {
            if self.loan.next() != Some(LifecycleAction::CheckInitialNoTail) { return Err(ProtocolFault::UnexpectedObservation); }
            self.loan.observe_tail_or_bind(fact)
        }
        fn bound_empty(&mut self, fact: InitialReturnedFact) -> Result<(), ProtocolFault> {
            if self.loan.next() != Some(LifecycleAction::BindInitialEmpty) { return Err(ProtocolFault::UnexpectedObservation); }
            self.loan.observe_tail_or_bind(fact)
        }
        fn stepped(&mut self, code: i32) -> Result<(), ProtocolFault> { self.loan.observe_step(code) }
        fn reset(&mut self, code: i32) -> Result<(), ProtocolFault> { self.loan.observe_reset(code) }
        fn finalize(&mut self, code: i32) -> Result<(), ProtocolFault> { self.loan.observe_finalize(code) }
        fn retain_paid_driver_result(&mut self, error: rusqlite::Error) -> Result<(), rusqlite::Error> { self.loan.retain_paid_driver_error(error) }
        fn retain_paid_cleanup_result(&mut self, error: rusqlite::Error) -> Result<(), rusqlite::Error> { self.loan.retain_paid_cleanup(error) }
        fn discard_existing_cleanup_result(&mut self) -> Result<(), ProtocolFault> { self.loan.discard_owned_cleanup() }
        fn discard_existing_driver_result(&mut self) -> Result<(), ProtocolFault> { self.loan.discard_owned_driver_error() }
        fn sql(&self) -> &'static str { match self.kind {
            InitialQuery::Main => "SELECT COUNT(*),COALESCE(SUM(length(CAST(name AS BLOB))+length(CAST(tbl_name AS BLOB))+COALESCE(length(CAST(sql AS BLOB)),0)),0) FROM main.sqlite_schema",
            InitialQuery::Temp => "SELECT COUNT(*),COALESCE(SUM(length(CAST(name AS BLOB))+length(CAST(tbl_name AS BLOB))+COALESCE(length(CAST(sql AS BLOB)),0)),0) FROM temp.sqlite_schema",
            InitialQuery::AppId => "PRAGMA application_id", InitialQuery::Version => "PRAGMA user_version",
            InitialQuery::ForeignKeys => "PRAGMA foreign_keys", InitialQuery::Journal => "PRAGMA journal_mode", InitialQuery::Sync => "PRAGMA synchronous",
        } }
        fn completed_query_row(&mut self, fact: InitialReturnedFact) -> Result<(), ProtocolFault> { self.loan.retain_query_return(fact) }
    }
    impl<'s, 'a, 'purpose> OriginalInitialRowLoan<'s, 'a, 'purpose> {
        fn column_type(&mut self, native_type: i32) -> Result<(), ProtocolFault> {
            if self.loan.next() != Some(LifecycleAction::InitialColumnType) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.loan.fields.native.initial_read;
            let expected = if r.query() == Some(InitialQuery::Journal) { rusqlite::ffi::SQLITE_TEXT } else { rusqlite::ffi::SQLITE_INTEGER };
            if native_type != expected { r.outcome = InitialOutcome::Error; r.phase = InitialPhase::Primary; }
            else { r.phase = if expected == rusqlite::ffi::SQLITE_TEXT { InitialPhase::Text }
                else if r.phase == InitialPhase::Type1 { InitialPhase::Value1 } else { InitialPhase::Value0 }; }
            Ok(())
        }
        fn integer(&mut self, value: i64) -> Result<(), ProtocolFault> {
            if self.loan.next() != Some(LifecycleAction::InitialInteger) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.loan.fields.native.initial_read;
            match r.query().ok_or(ProtocolFault::UnexpectedObservation)? {
                InitialQuery::Main | InitialQuery::Temp => if r.phase == InitialPhase::Value0 {
                    self.loan.prefix.count = Some(value); r.phase = InitialPhase::Type1; return Ok(());
                } else { self.loan.prefix.extent = Some(value); },
                InitialQuery::AppId => self.loan.prefix.application_id = Some(value),
                InitialQuery::Version => self.loan.prefix.user_version = Some(value),
                InitialQuery::ForeignKeys => self.loan.prefix.foreign_keys = Some(value),
                InitialQuery::Sync => self.loan.prefix.synchronous = Some(value),
                InitialQuery::Journal => return Err(ProtocolFault::UnexpectedObservation),
            }
            r.phase = InitialPhase::Reset; Ok(())
        }
        fn journal_text<'row, 'short>(&'short mut self, bytes: &'row [u8])
            -> Result<OriginalJournalTextLoan<'row, 'short, 's, 'a, 'purpose>, ProtocolFault> {
            if self.loan.next() != Some(LifecycleAction::InitialJournalText) { return Err(ProtocolFault::UnexpectedObservation); }
            if std::str::from_utf8(bytes).is_err() {
                self.loan.fields.native.initial_read.outcome = InitialOutcome::Error;
                self.loan.fields.native.initial_read.phase = InitialPhase::Primary;
                return Err(ProtocolFault::UnexpectedObservation);
            }
            Ok(OriginalJournalTextLoan { row: self, bytes })
        }
    }
    impl OriginalJournalTextLoan<'_, '_, '_, '_, '_> {
        // String has already been paid/owned by the future genuine adapter.
        // No clone, allocator, arbitrary byte debit or layout authority here.
        fn retain_paid_string(self, paid: String) -> Result<(), String> {
            if self.row.loan.fields.work.terminal().is_some() || self.row.loan.fields.physical.primary.is_some()
                || self.row.loan.next() != Some(LifecycleAction::InitialJournalText) || self.row.loan.prefix.journal_mode.is_some()
                || paid.as_bytes() != self.bytes { return Err(paid); }
            self.row.loan.prefix.journal_mode = Some(paid);
            self.row.loan.fields.native.initial_read.phase = InitialPhase::Reset; Ok(())
        }
    }


    #[cfg(test)]
    impl OriginalInitialReadLoan<'_, '_> {
        pub(in crate::database::global_schema_v1) fn test_code_validation_unknown_blocks_begin(&mut self) {
            assert_eq!(self.next(), Some(LifecycleAction::ValidateInitialOptions));
            assert!(!self.fields.reborrow().original_transaction().begin());
            assert_eq!(self.observe_prerequisite(InitialReturnedFact::Ok), Err(ProtocolFault::UnexpectedObservation));
            assert!(self.fields.native.statements.iter().all(|s| matches!(s, StmtSlot::Vacant)));
        }
        pub(in crate::database::global_schema_v1) fn test_code_validate_genuine_rows_options(&mut self) -> bool {
            let super::super::super::SelectionSnapshotPurpose::RowsBackup(options) = self.prefix.purpose else { panic!("fixed genuine purpose"); };
            assert_eq!(super::super::super::decide_financial_mode_pair(self.prefix.bound, self.prefix.catalog), Ok(super::super::super::FinancialModeDecision::Production));
            // This safe factory executes only in this test protocol; it does
            // not confer the absent source diagnostic/layout/payment rules.
            match options.validate_mode(self.prefix.bound) {
                Ok(()) => { self.observe_validation(InitialReturnedFact::Ok).unwrap(); true }
                Err(error) => {
                    let super::super::super::GlobalSchemaV1Error::SelectionSnapshotChanged { detail } = &error else { panic!("genuine options refusal"); };
                    let allocation = detail.as_ptr() as usize;
                    self.observe_validation(InitialReturnedFact::Error).unwrap();
                    self.retain_paid_primary(SourceOperationError::Global(error)).unwrap_or_else(|_| panic!("same cfg-owned refusal"));
                    let Some(SourceOperationError::Global(super::super::super::GlobalSchemaV1Error::SelectionSnapshotChanged { detail })) = &self.fields.physical.primary else { panic!("retained real refusal"); };
                    assert_eq!(detail.as_ptr() as usize, allocation); assert!(!self.fields.reborrow().original_transaction().begin()); false
                }
            }
        }
        pub(in crate::database::global_schema_v1) fn test_code_fixed_prerequisites(&mut self) {
            assert_eq!(self.next(), Some(LifecycleAction::RequireZeroOwnedWal));
            assert_eq!(self.observe_prepare(rusqlite::ffi::SQLITE_OK), Err(ProtocolFault::UnexpectedObservation));
            // Explicit fake FS observation, never proof that /dev/null pins
            // satisfy actual owned-WAL/provider qualification.
            self.observe_prerequisite(InitialReturnedFact::Ok).unwrap();
            assert_eq!(self.next(), Some(LifecycleAction::BeforeInitialCapture));
            let result = self.prefix.purpose.options().unwrap().phase(super::super::super::prospective::Phase::BeforeInitialCapture);
            assert!(result.is_ok()); self.observe_prerequisite(InitialReturnedFact::Ok).unwrap();
        }
        fn fixed_row(&mut self) {
            self.begin_query_slot().unwrap();
            let kind = self.fields.native.initial_read.query().unwrap();
            self.fields.native.statements[0] = StmtSlot::ProtocolHeld(StmtState { action: kind.action(), ..protocol_stmt_state() });
            self.observe_prepare(rusqlite::ffi::SQLITE_OK).unwrap();
            assert_eq!(self.observe_prepare(rusqlite::ffi::SQLITE_OK), Err(ProtocolFault::UnexpectedObservation));
            self.observe_tail_or_bind(InitialReturnedFact::Ok).unwrap();
            self.observe_tail_or_bind(InitialReturnedFact::Ok).unwrap();
            self.observe_step(rusqlite::ffi::SQLITE_ROW).unwrap();
            assert_eq!(self.observe_step(rusqlite::ffi::SQLITE_DONE), Err(ProtocolFault::UnexpectedObservation));
        }
        fn fixed_integers(&mut self, first: i64, second: Option<i64>) {
            let mut row = self.first_row().unwrap();
            row.column_type(rusqlite::ffi::SQLITE_INTEGER).unwrap(); row.integer(first).unwrap();
            if let Some(second) = second { row.column_type(rusqlite::ffi::SQLITE_INTEGER).unwrap(); row.integer(second).unwrap(); }
        }
        fn fixed_return_ok(&mut self) {
            self.observe_reset(rusqlite::ffi::SQLITE_OK).unwrap();
            self.observe_finalize(rusqlite::ffi::SQLITE_OK).unwrap();
            assert_eq!(self.next(), Some(LifecycleAction::AwaitInitialQueryReturn));
            assert_eq!(self.observe_step(rusqlite::ffi::SQLITE_DONE), Err(ProtocolFault::UnexpectedObservation));
            self.test_code_shared_barrier(); self.retain_query_return(InitialReturnedFact::Ok).unwrap();
        }
        pub(in crate::database::global_schema_v1) fn test_code_primary_before_prepare_cut(&mut self) {
            assert_eq!(self.fields.native.initial_read.query(), Some(InitialQuery::Main));
            assert_eq!(self.next(), Some(LifecycleAction::PrepareInitialRead));
            assert!(self.fields.native.statements[0].live().is_none());
        }
        pub(in crate::database::global_schema_v1) fn test_code_primary_between_queries_cut(&mut self) {
            self.fixed_row(); self.fixed_integers(3, Some(17)); self.fixed_return_ok();
            assert_eq!(self.prefix.main, Some((3, 17)));
            assert_eq!(self.fields.native.initial_read.query(), Some(InitialQuery::Temp));
            assert_eq!(self.next(), Some(LifecycleAction::PrepareInitialRead));
            assert!(self.fields.native.statements[0].live().is_none());
        }
        pub(in crate::database::global_schema_v1) fn test_code_primary_acquired_row_cut(&mut self) {
            self.fixed_row(); self.fixed_integers(3, None);
            assert_eq!(self.prefix.count, Some(3)); assert!(self.prefix.extent.is_none());
            assert_eq!(self.next(), Some(LifecycleAction::InitialColumnType));
        }
        pub(in crate::database::global_schema_v1) fn test_code_retain_before_read_primary(&mut self, primary: SourceOperationError) {
            assert_eq!(self.next(), Some(LifecycleAction::PrepareInitialRead));
            self.fields.reborrow().original_transaction().retain_early_primary(primary)
                .unwrap_or_else(|_| panic!("same first owned early primary"));
            assert_eq!(self.next(), Some(LifecycleAction::StopInitialRead));
        }
        pub(in crate::database::global_schema_v1) fn test_code_primary_drains_existing_read(&mut self, raw: rusqlite::Error, cleanup: rusqlite::Error) {
            let before = self.fields.work.test_code_observation(); assert!(before.terminal.is_none());
            assert!(self.fields.physical.primary.is_some()); assert!(!self.wrap_ready());
            assert_eq!(self.begin_query_slot(), Err(ProtocolFault::UnexpectedObservation));
            assert_eq!(self.observe_prepare(rusqlite::ffi::SQLITE_OK), Err(ProtocolFault::UnexpectedObservation));
            assert_eq!(self.observe_step(rusqlite::ffi::SQLITE_ROW), Err(ProtocolFault::UnexpectedObservation));
            assert!(self.first_row().is_err()); self.test_code_shared_barrier();
            if self.fields.native.statements[0].live().is_some() {
                let rusqlite::Error::InvalidColumnName(raw_name) = &raw else { panic!("fixed raw child"); };
                let raw_allocation = raw_name.as_ptr() as usize;
                let rusqlite::Error::InvalidColumnName(cleanup_name) = &cleanup else { panic!("fixed ignored child"); };
                let cleanup_allocation = cleanup_name.as_ptr() as usize;
                assert_eq!(self.next(), Some(LifecycleAction::ResetInitialRead));
                self.observe_reset(rusqlite::ffi::SQLITE_OK).unwrap();
                self.observe_finalize(rusqlite::ffi::SQLITE_ERROR).unwrap();
                assert_eq!(self.observe_finalize(rusqlite::ffi::SQLITE_OK), Err(ProtocolFault::UnexpectedObservation));
                assert_eq!(self.next(), Some(LifecycleAction::RetainInitialFinalizeError));
                self.retain_paid_cleanup(cleanup).unwrap();
                { let _short = self.fields.reborrow(); }
                let Some((InitialOwnedCleanup::Finalize, rusqlite::Error::InvalidColumnName(name))) = &self.fields.native.initial_read.ignored else { panic!("same owned cleanup retained"); };
                assert_eq!(name.as_ptr() as usize, cleanup_allocation); self.test_code_shared_barrier();
                self.discard_owned_cleanup().unwrap();
                assert_eq!(self.discard_owned_cleanup(), Err(ProtocolFault::UnexpectedObservation));
                assert_eq!(self.next(), Some(LifecycleAction::AwaitInitialQueryReturn));
                assert!(self.fields.native.initial_read.returned == BatchReturn::Unobserved);
                assert_eq!(self.stop_terminal_or_failed_prefix(), Err(ProtocolFault::UnexpectedObservation));
                assert_eq!(self.discard_owned_driver_error(), Err(ProtocolFault::UnexpectedObservation));
                assert_eq!(self.fields.reborrow().original_transaction().start_early_error(), Err(ProtocolFault::UnexpectedObservation));
                self.test_code_shared_barrier();
                // Incomplete typed ROW cannot claim Ok. Error is separately
                // supplied by the fixed completed-call observation, never
                // inferred from primary/reset/finalize or the raw diagnostic.
                assert_eq!(self.retain_query_return(InitialReturnedFact::Ok), Err(ProtocolFault::UnexpectedObservation));
                assert!(self.fields.native.initial_read.returned == BatchReturn::Unobserved);
                self.retain_query_return(InitialReturnedFact::Error).unwrap();
                assert_eq!(self.next(), Some(LifecycleAction::RetainInitialDriverError));
                self.retain_paid_driver_error(raw).unwrap(); { let _short = self.fields.reborrow(); }
                let Some(rusqlite::Error::InvalidColumnName(name)) = &self.fields.native.initial_read.driver_error else { panic!("same raw child retained until once discard"); };
                assert_eq!(name.as_ptr() as usize, raw_allocation); assert!(!self.wrap_ready());
                assert_eq!(self.retain_pragma_wrapped_error(), Err(ProtocolFault::UnexpectedObservation));
                assert_eq!(self.retain_query_return(InitialReturnedFact::Error), Err(ProtocolFault::UnexpectedObservation));
                self.test_code_shared_barrier(); self.discard_owned_driver_error().unwrap();
                assert!(self.fields.native.initial_read.consumed[InitialQuery::Main.index()].is_some());
                assert_eq!(self.discard_owned_driver_error(), Err(ProtocolFault::UnexpectedObservation));
            } else {
                drop(raw); drop(cleanup); // unused test payloads claim no driver failure/payment.
                assert_eq!(self.next(), Some(LifecycleAction::StopInitialRead));
                assert_eq!(self.retain_query_return(InitialReturnedFact::Ok), Err(ProtocolFault::UnexpectedObservation));
                assert_eq!(self.retain_query_return(InitialReturnedFact::Error), Err(ProtocolFault::UnexpectedObservation));
                let kind = self.fields.native.initial_read.query().unwrap();
                assert!(matches!(self.fields.native.initial_read.prepare, CodeSlot::NotCalled));
                assert!(self.fields.native.initial_read.consumed[kind.index()].is_none());
                self.stop_terminal_or_failed_prefix().unwrap();
                assert!(self.fields.native.initial_read.consumed[kind.index()].is_none());
                if kind == InitialQuery::Temp { assert!(self.fields.native.initial_read.consumed[InitialQuery::Main.index()].is_some()); }
            }
            assert_eq!(self.stop_terminal_or_failed_prefix(), Err(ProtocolFault::UnexpectedObservation));
            assert!(self.fields.native.initial_read.ignored.is_none()); assert!(self.fields.native.initial_read.driver_error.is_none());
            assert_eq!(self.fields.native.initial_read.stage, InitialStage::Stopped);
            assert_eq!(self.fields.work.test_code_observation(), before);
        }
        fn fixed_through_foreign_keys(&mut self) {
            for (kind, first, second) in [(InitialQuery::Main, 3, Some(17)), (InitialQuery::Temp, 1, Some(5)),
                (InitialQuery::AppId, 1398035265, None), (InitialQuery::Version, 1, None), (InitialQuery::ForeignKeys, 1, None)] {
                assert_eq!(self.fields.native.initial_read.query(), Some(kind)); self.fixed_row(); self.fixed_integers(first, second); self.fixed_return_ok();
            }
        }
        fn fixed_journal(&mut self, paid: String) {
            self.fixed_row(); let mut row = self.first_row().unwrap(); row.column_type(rusqlite::ffi::SQLITE_TEXT).unwrap();
            { let _short = row.journal_text(b"wal").unwrap(); }
            row.journal_text(b"wal").unwrap().retain_paid_string(paid).unwrap_or_else(|_| panic!("one supplied paid String move"));
        }
        pub(in crate::database::global_schema_v1) fn test_code_exact_prefix(&mut self, journal: String) {
            let options = self.prefix.purpose.options().unwrap();
            let object_cap = options.max_catalog_objects as i64; let byte_cap = options.max_catalog_bytes as i64;
            for (kind, first, second) in [(InitialQuery::Main, object_cap - 1, Some(byte_cap - 1)),
                (InitialQuery::Temp, 1, Some(1)), (InitialQuery::AppId, 1398035265, None),
                (InitialQuery::Version, 1, None), (InitialQuery::ForeignKeys, 1, None)] {
                assert_eq!(self.fields.native.initial_read.query(), Some(kind)); self.fixed_row(); self.fixed_integers(first, second); self.fixed_return_ok();
            }
            self.fixed_journal(journal); self.fixed_return_ok(); self.fixed_row(); self.fixed_integers(2, None); self.fixed_return_ok();
            assert_eq!(self.next(), Some(LifecycleAction::InitialPrefixReached));
            assert_eq!(self.retain_query_return(InitialReturnedFact::Ok), Err(ProtocolFault::UnexpectedObservation));
            assert_eq!(self.observe_step(rusqlite::ffi::SQLITE_DONE), Err(ProtocolFault::UnexpectedObservation));
            let mut tx = self.fields.reborrow().original_transaction();
            assert_eq!(tx.start_early_error(), Err(ProtocolFault::UnexpectedObservation));
            self.test_code_shared_barrier();
        }
        pub(in crate::database::global_schema_v1) fn test_code_fault_cut(&mut self, case: super::super::super::FinancialInitialReadCase, journal: String) {
            use super::super::super::FinancialInitialReadCase as Case;
            match case {
                Case::NegativeCount | Case::NegativeExtent | Case::ExcessCap | Case::MaximumSum => {
                    drop(journal); self.fixed_row();
                    let (count, extent) = match case { Case::NegativeCount => (-1, 0), Case::NegativeExtent => (1, -1),
                        Case::MaximumSum => (i64::MAX, i64::MAX), _ => (4096, 0) };
                    self.fixed_integers(count, Some(extent)); self.fixed_return_ok();
                    if matches!(case, Case::ExcessCap | Case::MaximumSum) {
                        assert_eq!(self.fields.native.initial_read.query(), Some(InitialQuery::Temp)); self.fixed_row();
                        let value = if matches!(case, Case::MaximumSum) { i64::MAX } else { 1 };
                        self.fixed_integers(value, Some(if matches!(case, Case::MaximumSum) { i64::MAX } else { 0 })); self.fixed_return_ok();
                        assert_eq!(self.fields.native.initial_read.extent_fault, Some(super::super::super::FinancialExtentFault::TotalCap));
                        if matches!(case, Case::MaximumSum) { assert_eq!((self.prefix.objects, self.prefix.bytes), (u64::MAX - 1, u64::MAX - 1)); }
                    }
                }
                Case::WrongInteger => {
                    drop(journal); self.fixed_row(); let mut row = self.first_row().unwrap();
                    row.column_type(rusqlite::ffi::SQLITE_TEXT).unwrap();
                    assert_eq!(row.integer(1), Err(ProtocolFault::UnexpectedObservation));
                }
                Case::JournalWrongType | Case::JournalInvalidUtf8 | Case::SynchronousWrongType => {
                    self.fixed_through_foreign_keys();
                    if matches!(case, Case::SynchronousWrongType) {
                        self.fixed_journal(journal); self.fixed_return_ok(); self.fixed_row();
                        self.first_row().unwrap().column_type(rusqlite::ffi::SQLITE_NULL).unwrap();
                    } else {
                        drop(journal); self.fixed_row(); let mut row = self.first_row().unwrap();
                        row.column_type(if matches!(case, Case::JournalWrongType) { rusqlite::ffi::SQLITE_BLOB } else { rusqlite::ffi::SQLITE_TEXT }).unwrap();
                        assert!(row.journal_text(if matches!(case, Case::JournalWrongType) { b"wal" } else { b"\xff" }).is_err());
                    }
                }
                Case::StepError | Case::NoRows | Case::NoRowsResetError => {
                    drop(journal); self.begin_query_slot().unwrap();
                    self.fields.native.statements[0] = StmtSlot::ProtocolHeld(StmtState { action: FixedAction::ProspectiveExtent, ..protocol_stmt_state() });
                    self.observe_prepare(rusqlite::ffi::SQLITE_OK).unwrap(); self.observe_tail_or_bind(InitialReturnedFact::Ok).unwrap(); self.observe_tail_or_bind(InitialReturnedFact::Ok).unwrap();
                    self.observe_step(if matches!(case, Case::StepError) { rusqlite::ffi::SQLITE_ERROR } else { rusqlite::ffi::SQLITE_DONE }).unwrap();
                    if !matches!(case, Case::StepError) { self.observe_reset(if matches!(case, Case::NoRowsResetError) { rusqlite::ffi::SQLITE_ERROR } else { rusqlite::ffi::SQLITE_OK }).unwrap(); }
                }
            }
            assert!(matches!(self.next(), Some(LifecycleAction::RetainInitialPrimary | LifecycleAction::RetainInitialDriverError))); self.test_code_shared_barrier();
        }
        pub(in crate::database::global_schema_v1) fn test_code_retain_first_error(&mut self, primary: SourceOperationError, raw: rusqlite::Error) {
            if self.next() == Some(LifecycleAction::RetainInitialDriverError) {
                drop(primary); let rusqlite::Error::InvalidColumnName(name) = &raw else { panic!("fixed raw child"); }; let allocation = name.as_ptr() as usize;
                self.retain_paid_driver_error(raw).unwrap(); { let _short = self.fields.reborrow(); }
                let Some(rusqlite::Error::InvalidColumnName(name)) = &self.fields.native.initial_read.driver_error else { panic!("same owned raw error"); }; assert_eq!(name.as_ptr() as usize, allocation);
                let (spare, allocation) = fixed_supplied_error();
                let Err(spare) = self.fields.reborrow().original_transaction().retain_early_primary(spare) else { panic!("no early Global wrapper while raw return pending"); }; assert_same_supplied_primary(spare, allocation);
                assert!(!self.wrap_ready());
            } else { drop(raw); self.retain_paid_primary(primary).unwrap_or_else(|_| panic!("first supplied non-driver primary")); }
        }
        pub(in crate::database::global_schema_v1) fn test_code_finish_normal_error(&mut self, cleanup: rusqlite::Error) -> usize {
            let raw_allocation = match self.fields.native.initial_read.driver_error.as_ref() { Some(rusqlite::Error::InvalidColumnName(name)) => Some(name.as_ptr() as usize), None => None, _ => panic!("fixed owned raw test shape") };
            if self.fields.native.statements[0].live().is_some() {
                if self.next() == Some(LifecycleAction::ResetInitialRead) { self.observe_reset(rusqlite::ffi::SQLITE_OK).unwrap(); }
                self.observe_finalize(rusqlite::ffi::SQLITE_ERROR).unwrap();
                assert_eq!(self.observe_finalize(rusqlite::ffi::SQLITE_OK), Err(ProtocolFault::UnexpectedObservation));
                self.retain_paid_cleanup(cleanup).unwrap();
                { let _short = self.fields.reborrow(); } self.test_code_shared_barrier();
                self.discard_owned_cleanup().unwrap(); assert_eq!(self.discard_owned_cleanup(), Err(ProtocolFault::UnexpectedObservation));
            } else { drop(cleanup); }
            if self.next() == Some(LifecycleAction::AwaitInitialQueryReturn) { assert!(!self.wrap_ready()); self.retain_query_return(InitialReturnedFact::Error).unwrap(); }
            let mut detail_allocation = 0;
            if self.next() == Some(LifecycleAction::WrapInitialDriverError) {
                assert_eq!(self.retain_query_return(InitialReturnedFact::Error), Err(ProtocolFault::UnexpectedObservation));
                if matches!(self.fields.native.initial_read.query(), Some(InitialQuery::Main | InitialQuery::Temp)) {
                    let raw = self.fields.native.initial_read.driver_error.as_ref().unwrap();
                    let detail = raw.to_string(); detail_allocation = detail.as_ptr() as usize;
                    assert_eq!(detail, "Invalid column name: TEST_CODE raw driver error");
                    self.retain_catalog_wrapped_detail(detail).unwrap();
                    let Some(SourceOperationError::Global(super::super::super::GlobalSchemaV1Error::SelectionCatalog { source: super::super::super::GlobalSchemaCatalogError::SqliteReferenceBuildFailure { detail, .. } })) = &self.fields.physical.primary else { panic!("same C/G wrapping payload"); };
                    assert_eq!(detail.as_ptr() as usize, detail_allocation);
                } else {
                    self.retain_pragma_wrapped_error().unwrap();
                    let Some(SourceOperationError::Global(super::super::super::GlobalSchemaV1Error::SelectionSqlite { source: rusqlite::Error::InvalidColumnName(name), .. })) = &self.fields.physical.primary else { panic!("raw child moved into G wrapper"); };
                    assert_eq!(Some(name.as_ptr() as usize), raw_allocation);
                }
            }
            assert_eq!(self.fields.native.initial_read.stage, InitialStage::Stopped);
            assert_eq!(self.retain_pragma_wrapped_error(), Err(ProtocolFault::UnexpectedObservation));
            assert!(self.fields.native.initial_read.driver_error.is_none()); detail_allocation
        }
        pub(in crate::database::global_schema_v1) fn test_code_terminal_cut(&mut self, cut: super::super::super::FinancialInitialTerminalCut, journal: String, cleanup: rusqlite::Error) {
            use super::super::super::FinancialInitialTerminalCut as Cut;
            match cut {
                Cut::BeforePrepare => { drop(journal); drop(cleanup); }
                Cut::FirstColumn => { drop(journal); drop(cleanup); self.fixed_row(); let mut row = self.first_row().unwrap(); row.column_type(rusqlite::ffi::SQLITE_INTEGER).unwrap(); row.integer(3).unwrap(); }
                Cut::QueryReturnUnknown => { drop(journal); drop(cleanup); self.fixed_row(); self.fixed_integers(3, Some(17)); self.observe_reset(rusqlite::ffi::SQLITE_OK).unwrap(); self.observe_finalize(rusqlite::ffi::SQLITE_OK).unwrap(); assert_eq!(self.next(), Some(LifecycleAction::AwaitInitialQueryReturn)); }
                Cut::RawDriverBeforeReturn | Cut::RawDriverAfterReturn => {
                    drop(journal); self.begin_query_slot().unwrap();
                    self.fields.native.statements[0] = StmtSlot::ProtocolHeld(StmtState { action: FixedAction::ProspectiveExtent, ..protocol_stmt_state() });
                    self.observe_prepare(rusqlite::ffi::SQLITE_OK).unwrap(); self.observe_tail_or_bind(InitialReturnedFact::Ok).unwrap(); self.observe_tail_or_bind(InitialReturnedFact::Ok).unwrap();
                    self.observe_step(rusqlite::ffi::SQLITE_ERROR).unwrap(); self.retain_paid_driver_error(cleanup).unwrap();
                    if matches!(cut, Cut::RawDriverAfterReturn) { self.observe_reset(rusqlite::ffi::SQLITE_OK).unwrap(); self.observe_finalize(rusqlite::ffi::SQLITE_OK).unwrap(); self.retain_query_return(InitialReturnedFact::Error).unwrap(); assert!(self.wrap_ready()); }
                    else { assert!(!self.wrap_ready()); }
                }
                Cut::JournalBeforeReset | Cut::OwnedIgnoredReset => {
                    self.fixed_through_foreign_keys(); self.fixed_journal(journal);
                    if matches!(cut, Cut::OwnedIgnoredReset) { self.observe_reset(rusqlite::ffi::SQLITE_ERROR).unwrap(); self.retain_paid_cleanup(cleanup).unwrap_or_else(|_| panic!("same ignored reset")); }
                    else { drop(cleanup); }
                }
            }
            self.test_code_shared_barrier();
        }
        pub(in crate::database::global_schema_v1) fn test_code_terminal_drain_prefix(&mut self) {
            let before = self.fields.work.test_code_observation(); assert!(before.terminal.is_some());
            assert!(self.first_row().is_err());
            assert_eq!(self.observe_prerequisite(InitialReturnedFact::Ok), Err(ProtocolFault::UnexpectedObservation));
            if self.next() == Some(LifecycleAction::DiscardInitialOwnedCleanup) {
                self.discard_owned_cleanup().unwrap(); assert_eq!(self.discard_owned_cleanup(), Err(ProtocolFault::UnexpectedObservation));
            }
            if self.next() == Some(LifecycleAction::ResetInitialRead) { self.observe_reset(rusqlite::ffi::SQLITE_ERROR).unwrap(); }
            if self.next() == Some(LifecycleAction::FinalizeInitialRead) { self.observe_finalize(rusqlite::ffi::SQLITE_ERROR).unwrap(); }
            assert_eq!(self.observe_finalize(rusqlite::ffi::SQLITE_OK), Err(ProtocolFault::UnexpectedObservation));
            if self.next() == Some(LifecycleAction::DiscardInitialDriverError) { self.discard_owned_driver_error().unwrap(); }
            else { self.stop_terminal_or_failed_prefix().unwrap(); }
            assert_eq!(self.stop_terminal_or_failed_prefix(), Err(ProtocolFault::UnexpectedObservation));
            assert!(self.fields.native.initial_read.ignored.is_none()); assert_eq!(self.fields.work.test_code_observation(), before);
        }
        pub(in crate::database::global_schema_v1) fn test_code_shared_barrier(&mut self) {
            let before = self.fields.work.test_code_observation(); let expected = self.fields.native.transaction_action(&self.fields.work, self.fields.physical).unwrap();
            assert!(!self.fields.native.transaction_release_ready(&self.fields.work, self.fields.physical));
            let mut acquire = self.fields.reborrow().original_acquisition();
            assert_eq!(acquire.constructor().next(), expected); assert_eq!(acquire.a00_epilogue().next(), expected);
            assert_eq!(acquire.constructor().observe(ConstructorObservation::Close(rusqlite::ffi::SQLITE_OK)), Err(ProtocolFault::UnexpectedObservation));
            assert!(matches!(acquire.settle(), AcquisitionSettlement::Held(_))); assert_eq!(self.fields.work.test_code_observation(), before);
        }
    }
    #[cfg(test)]
    impl OriginalTransactionLoan<'_> {
        pub(in crate::database::global_schema_v1) fn test_code_read_primary_rollback_once(&mut self, ignored: SourceOperationError, cleanup: SourceOperationError) {
            assert!(self.fields.physical.primary.is_some()); self.start_early_error().unwrap();
            self.test_code_normal_rollback_error(ignored, cleanup);
        }
        pub(in crate::database::global_schema_v1) fn test_code_existing_read_primary_exit(&mut self) {
            assert!(self.fields.physical.primary.is_some()); self.start_early_error().unwrap(); self.observe_autocommit(1).unwrap();
            assert!(self.fields.native.transaction_release_ready(&self.fields.work, self.fields.physical));
        }
    }

    #[repr(u8)]
    enum CloseAction { Constructor, Original, Reference, CopiedFirst, CopiedSecond }
    struct FixedCloseViolation {
        role: Role,
        action: CloseAction,
        status: i32,
        first_terminal: Option<SourceTerminal>,
    }
    struct UnreleasedOriginalFrame<'n, 'w> {
        loan: OriginalSqlLoan<'n, 'w>,
        release: &'n mut FixedDrainLedger,
    }
    // A future genuine owning frame lends these sibling fields at its actual
    // last-close cut. Passing this loan moves neither its native owner nor its
    // RowsWork, and introduces no empty publication/restore/second take.
    fn native_original_contract_violation(
        _frame: UnreleasedOriginalFrame<'_, '_>,
        _violation: FixedCloseViolation,
    ) -> ! {
        std::process::abort()
    }
    // Loans have no Drop action. Native/work/ledger remain in their owning
    // fields during the short exclusive borrow; actual abort qualification,
    // cleanup/native status and all selected layout/full-fit gates stay open.
    // Fixed A03/A04 mechanics only. No collect, growth, formatting, native call
    // or selected rule issuer is implemented here. Whole Vec ownership comes
    // from the actual future collect return; row facts do not construct it.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum IntegrityQuery { Check, Foreign }
    impl IntegrityQuery {
        fn index(self) -> usize { match self { Self::Check => 0, Self::Foreign => 1 } }
        fn action(self) -> FixedAction { match self { Self::Check => FixedAction::Integrity, Self::Foreign => FixedAction::ForeignKeyCheck } }
        fn sql(self) -> &'static str { match self { Self::Check => "PRAGMA integrity_check", Self::Foreign => "PRAGMA foreign_key_check" } }
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum IntegrityStage { Dormant, Check, Foreign, Drain, Ready, Stopped }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum IntegrityPhase { Prepare, Bind, Step, Type, Text, RowReturn, NeedRaw, Reset, AwaitWhole, Wrap, Compare, NeedDetail, Drain }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum IntegrityReturn { Unknown, Ok, Error, Interrupted }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum IntegrityCleanup { Reset, Finalize }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum IntegrityErrorCut { Prepare, Query, Read }
    struct IntegrityReadRecord {
        stage: IntegrityStage,
        query: IntegrityQuery,
        phase: IntegrityPhase,
        prepare: [CodeSlot; 2],
        rows_started: [bool; 2],
        eof: [bool; 2],
        rows: [usize; 2],
        first_all_ok: bool,
        pending_text_ok: bool,
        returned: [IntegrityReturn; 2],
        raw_cut: IntegrityErrorCut,
        raw: Option<rusqlite::Error>,
        need_cleanup: Option<(IntegrityQuery, IntegrityCleanup)>,
        ignored: Option<rusqlite::Error>,
        vector_live: bool,
        partial_live: bool,
        partial_returned: bool,
        failed: bool,
        consumed: [Option<StmtState>; 2],
        semantic_detail_closed: bool,
        detail_started: bool,
        detail_returned: bool,
        capture_started: bool,
        capture_returned: IntegrityReturn,
    }
    impl IntegrityReadRecord {
        fn empty() -> Self { Self { stage: IntegrityStage::Dormant, query: IntegrityQuery::Check,
            phase: IntegrityPhase::Prepare, prepare: [CodeSlot::NotCalled; 2], rows_started: [false; 2],
            eof: [false; 2], rows: [0; 2], first_all_ok: true, pending_text_ok: false,
            returned: [IntegrityReturn::Unknown; 2], raw_cut: IntegrityErrorCut::Prepare, raw: None,
            need_cleanup: None, ignored: None, vector_live: false, partial_live: false,
            partial_returned: false, failed: false, consumed: [None; 2], semantic_detail_closed: false, detail_started: false, detail_returned: false, capture_started: false, capture_returned: IntegrityReturn::Unknown } }
        fn blocks_early_primary(&self) -> bool {
            self.raw.is_some() || self.ignored.is_some() || self.need_cleanup.is_some() || self.detail_started
                || (self.stage != IntegrityStage::Dormant && self.phase == IntegrityPhase::NeedRaw)
        }
        fn stopped_clear(&self) -> bool {
            matches!(self.stage, IntegrityStage::Dormant | IntegrityStage::Stopped)
                && self.raw.is_none() && self.ignored.is_none() && self.need_cleanup.is_none()
                && !self.vector_live && !self.partial_live && !self.detail_started
        }
    }
    impl NativeOriginalOwner {
        fn integrity_semantic_failure_cut_absent(&self) -> bool { !self.integrity_read.semantic_detail_closed }
        fn integrity_read_action(&self, work: &OriginalSourceWork<'_>, physical: &super::super::super::FinancialPhysical) -> Option<LifecycleAction> {
            if !matches!(self.capture_prefix.stage, CaptureStage::Dormant | CaptureStage::Stopped) {
                return self.capture_prefix_action(work, physical);
            }
            let r = &self.integrity_read;
            if matches!(r.stage, IntegrityStage::Dormant | IntegrityStage::Stopped) { return None; }
            if r.ignored.is_some() { return Some(LifecycleAction::DiscardIntegrityOwnedCleanup); }
            // A real already-returned owned Result may still be outside this
            // loan. Terminal cannot infer its absence from the raw status.
            if r.need_cleanup.is_some() { return Some(LifecycleAction::RetainIntegrityOwnedCleanup); }
            if r.phase == IntegrityPhase::NeedRaw { return Some(LifecycleAction::RetainIntegrityRaw); }
            if r.detail_started && !r.detail_returned { return Some(LifecycleAction::AwaitIntegrityDetailReturn); }
            if r.partial_live && !r.partial_returned { return Some(LifecycleAction::AwaitIntegrityPartialReturn); }
            let interrupted = work.terminal().is_some() || physical.primary.is_some();
            if r.detail_started && r.detail_returned {
                return Some(if interrupted { LifecycleAction::DiscardIntegrityOwnedDetail } else { LifecycleAction::WrapIntegrityDetail });
            }
            if r.stage == IntegrityStage::Ready && !interrupted { return Some(LifecycleAction::IntegrityPrefixReached); }
            if r.phase == IntegrityPhase::AwaitWhole { return Some(LifecycleAction::AwaitIntegrityWholeReturn); }
            if r.raw.is_some() && r.returned[r.query.index()] != IntegrityReturn::Unknown {
                return Some(if interrupted { LifecycleAction::DiscardIntegrityRaw } else { LifecycleAction::WrapIntegrityRaw });
            }
            if interrupted || r.stage == IntegrityStage::Drain {
                if let Some(state) = self.statements[1].live() {
                    return Some(if state.cursor != CursorPhase::NoCursor && matches!(state.reset, CodeSlot::NotCalled) {
                        LifecycleAction::ResetIntegrityRead
                    } else { LifecycleAction::FinalizeIntegrityRead });
                }
                // The first completed Vec was declared after the first
                // Statement: failed outer scope drops it before that VM.
                if r.vector_live && (interrupted || r.failed) { return Some(LifecycleAction::DiscardIntegrityOwnedVector); }
                if let Some(state) = self.statements[0].live() {
                    return Some(if state.cursor != CursorPhase::NoCursor && matches!(state.reset, CodeSlot::NotCalled) {
                        LifecycleAction::ResetIntegrityRead
                    } else { LifecycleAction::FinalizeIntegrityRead });
                }
                if r.partial_live { return Some(LifecycleAction::DiscardIntegrityPartial); }
                if !matches!(r.prepare[r.query.index()], CodeSlot::NotCalled)
                    && r.returned[r.query.index()] == IntegrityReturn::Unknown
                    && self.integrity_semantic_failure_cut_absent() {
                    return Some(LifecycleAction::AwaitIntegrityWholeReturn);
                }
                if r.capture_started && r.capture_returned == IntegrityReturn::Unknown { return Some(LifecycleAction::AwaitIntegrityCaptureReturn); }
                return Some(LifecycleAction::StopIntegrityRead);
            }
            Some(match r.phase {
                IntegrityPhase::Prepare => LifecycleAction::PrepareIntegrityRead,
                IntegrityPhase::Bind => LifecycleAction::BindIntegrityEmpty,
                IntegrityPhase::Step => LifecycleAction::StepIntegrityRead,
                IntegrityPhase::Type => LifecycleAction::IntegrityColumnType,
                IntegrityPhase::Text => LifecycleAction::IntegrityText,
                IntegrityPhase::RowReturn => LifecycleAction::AwaitIntegrityRowReturn,
                IntegrityPhase::Reset => LifecycleAction::ResetIntegrityRead,
                IntegrityPhase::AwaitWhole => LifecycleAction::AwaitIntegrityWholeReturn,
                IntegrityPhase::Compare => LifecycleAction::CheckIntegritySemantic,
                IntegrityPhase::NeedDetail => LifecycleAction::RetainIntegrityDetail,
                IntegrityPhase::NeedRaw => LifecycleAction::RetainIntegrityRaw,
                IntegrityPhase::Wrap => LifecycleAction::WrapIntegrityRaw,
                IntegrityPhase::Drain => LifecycleAction::StopIntegrityRead,
            })
        }
    }
    pub(in crate::database::global_schema_v1) struct OriginalIntegrityReadLoan<'a> {
        fields: OriginalOwnerFields<'a>,
        prefix: &'a mut super::super::super::FinancialIntegrityReadState,
    }
    struct OriginalIntegrityPort<'s, 'a, 'r> {
        loan: &'s mut OriginalIntegrityReadLoan<'a>,
        query: IntegrityQuery,
        _rules: &'r SelectedOriginalNativeRules,
    }
    struct OriginalIntegrityDetailPort<'short, 'a> { loan: &'short mut OriginalIntegrityReadLoan<'a> }
    struct OriginalIntegrityTextLoan<'bytes, 'short, 'a> {
        loan: &'short mut OriginalIntegrityReadLoan<'a>,
        text: &'bytes str,
    }
    impl OriginalOwnerFields<'_> {
        pub(in crate::database::global_schema_v1) fn begin_integrity_read(&mut self) -> bool {
            let n = &mut self.native;
            if self.work.terminal().is_some() || self.physical.primary.is_some()
                || !matches!(n.tx.phase, TxPhase::Active) || !n.physical_initial_prefix_ready()
                || n.integrity_read.stage != IntegrityStage::Dormant { return false; }
            // Preserve all seven consumed A01/A02 ledgers and their real G
            // fields, but only the newly selected fixed unit owns continuation.
            n.initial_read.stage = InitialStage::Stopped;
            n.integrity_read.stage = IntegrityStage::Check;
            n.statements[0] = StmtSlot::Vacant;
            n.statements[1] = StmtSlot::Vacant;
            true
        }
    }
    impl NativeOriginalOwner {
        fn physical_initial_prefix_ready(&self) -> bool {
            self.initial_read.context == InitialContext::Valid && self.initial_read.stage == InitialStage::Reached
                && self.initial_read.driver_error.is_none() && self.initial_read.ignored.is_none()
                && self.initial_read.consumed.iter().all(Option::is_some)
                && self.statements.iter().all(|s| s.live().is_none())
                && self.original.phase() == Some(ConnectionPhase::Configured)
        }
    }
    impl<'a> OriginalOwnerFields<'a> {
        pub(in crate::database::global_schema_v1) fn integrity_read(self, prefix: &'a mut super::super::super::FinancialIntegrityReadState) -> OriginalIntegrityReadLoan<'a> {
            OriginalIntegrityReadLoan { fields: self, prefix }
        }
    }
    impl<'a> OriginalIntegrityReadLoan<'a> {
        fn next(&self) -> Option<LifecycleAction> { self.fields.native.integrity_read_action(&self.fields.work, self.fields.physical) }
        fn port<'s, 'r>(&'s mut self, query: IntegrityQuery, rules: &'r SelectedOriginalNativeRules) -> Result<OriginalIntegrityPort<'s, 'a, 'r>, ProtocolFault> {
            if self.fields.native.integrity_read.query != query { return Err(ProtocolFault::UnexpectedObservation); }
            Ok(OriginalIntegrityPort { loan: self, query, _rules: rules })
        }
        fn drain_index(&self) -> Result<usize, ProtocolFault> {
            for i in [1, 0] { if self.fields.native.statements[i].live().is_some() { return Ok(i); } }
            Err(ProtocolFault::ResourceNotInstalled)
        }
        fn adverse(&mut self, query: IntegrityQuery, code: i32) {
            if code == rusqlite::ffi::SQLITE_OK { return; }
            let make = || FixedAdverse { role: Role::Original, action: query.action(), ordinal: query.index(), code };
            if self.fields.native.secondary.is_none() { self.fields.native.secondary = Some(make()); }
            if self.fields.release.first_secondary.is_none() { self.fields.release.first_secondary = Some(make()); }
        }
        fn observe_prepare(&mut self, code: i32) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::PrepareIntegrityRead) { return Err(ProtocolFault::UnexpectedObservation); }
            let query = self.fields.native.integrity_read.query;
            let slot = &self.fields.native.statements[query.index()];
            if code == rusqlite::ffi::SQLITE_OK {
                let state = slot.live().ok_or(ProtocolFault::ResourceNotInstalled)?;
                if state.role != Role::Original || state.action != query.action() || state.cursor != CursorPhase::NoCursor { return Err(ProtocolFault::UnexpectedObservation); }
            } else if slot.live().is_some() { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.integrity_read;
            record_once(&mut r.prepare[query.index()], code)?; r.capture_started = true;
            r.phase = if code == rusqlite::ffi::SQLITE_OK { IntegrityPhase::Bind } else { IntegrityPhase::NeedRaw };
            r.raw_cut = IntegrityErrorCut::Prepare;
            if code != rusqlite::ffi::SQLITE_OK { r.failed = true; self.adverse(query, code); }
            Ok(())
        }
        fn observe_empty_query(&mut self, fact: IntegrityReturn) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::BindIntegrityEmpty) || !matches!(fact, IntegrityReturn::Ok | IntegrityReturn::Error) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.integrity_read;
            if fact == IntegrityReturn::Ok {
                r.rows_started[r.query.index()] = true; r.phase = IntegrityPhase::Step;
                self.fields.native.statements[r.query.index()].live_mut().ok_or(ProtocolFault::ResourceNotInstalled)?.cursor = CursorPhase::Active;
            } else { r.failed = true; r.raw_cut = IntegrityErrorCut::Query; r.phase = IntegrityPhase::NeedRaw; }
            Ok(())
        }
        fn observe_step(&mut self, code: i32) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::StepIntegrityRead) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.integrity_read;
            let state = self.fields.native.statements[r.query.index()].live_mut().ok_or(ProtocolFault::ResourceNotInstalled)?;
            // Multiple next calls are phase-bound, not a repeated observation
            // of one call; only the latest raw status remains in StmtState.
            state.step = CodeSlot::Called(code);
            state.cursor = if code == rusqlite::ffi::SQLITE_DONE { CursorPhase::Ended } else { CursorPhase::Active };
            match code {
                rusqlite::ffi::SQLITE_ROW => r.phase = if r.query == IntegrityQuery::Check { IntegrityPhase::Type } else { IntegrityPhase::RowReturn },
                rusqlite::ffi::SQLITE_DONE => { r.eof[r.query.index()] = true; r.phase = IntegrityPhase::Reset; }
                _ => { r.failed = true; r.raw_cut = IntegrityErrorCut::Read; r.phase = IntegrityPhase::NeedRaw; }
            }
            let query = r.query;
            if code != rusqlite::ffi::SQLITE_ROW && code != rusqlite::ffi::SQLITE_DONE { self.adverse(query, code); }
            Ok(())
        }
        fn observe_column_type(&mut self, native_type: i32) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::IntegrityColumnType) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.integrity_read;
            if native_type == rusqlite::ffi::SQLITE_TEXT { r.phase = IntegrityPhase::Text; }
            else { r.failed = true; r.raw_cut = IntegrityErrorCut::Read; r.phase = IntegrityPhase::NeedRaw; }
            Ok(())
        }
        fn text<'bytes, 'short>(&'short mut self, bytes: &'bytes [u8]) -> Result<OriginalIntegrityTextLoan<'bytes, 'short, 'a>, ProtocolFault> {
            if self.next() != Some(LifecycleAction::IntegrityText) { return Err(ProtocolFault::UnexpectedObservation); }
            let text = match std::str::from_utf8(bytes) {
                Ok(text) => text,
                Err(_) => { let r = &mut self.fields.native.integrity_read; r.failed = true; r.raw_cut = IntegrityErrorCut::Read; r.phase = IntegrityPhase::NeedRaw; return Err(ProtocolFault::UnexpectedObservation); }
            };
            Ok(OriginalIntegrityTextLoan { loan: self, text })
        }
        fn observe_row_return(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitIntegrityRowReturn) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.integrity_read;
            let i = r.query.index();
            r.rows[i] = r.rows[i].checked_add(1).ok_or(ProtocolFault::UnexpectedObservation)?;
            if r.query == IntegrityQuery::Check { r.first_all_ok &= r.pending_text_ok; r.phase = IntegrityPhase::Step; }
            else if let Some(count) = self.prefix.foreign_key_violations.checked_add(1) {
                self.prefix.foreign_key_violations = count; r.phase = IntegrityPhase::Step;
            } else { self.prefix.fault = Some(super::super::super::FinancialIntegrityFault::ForeignOverflow); r.phase = IntegrityPhase::NeedDetail; r.failed = true; }
            Ok(())
        }
        fn retain_existing_raw(&mut self, raw: rusqlite::Error) -> Result<(), rusqlite::Error> {
            if self.next() != Some(LifecycleAction::RetainIntegrityRaw) || self.fields.native.integrity_read.raw.is_some() { return Err(raw); }
            let r = &mut self.fields.native.integrity_read;
            r.raw = Some(raw);
            r.phase = if r.rows_started[r.query.index()] && self.fields.native.statements[r.query.index()].live().is_some_and(|s| matches!(s.reset, CodeSlot::NotCalled)) {
                IntegrityPhase::Reset
            } else { IntegrityPhase::AwaitWhole };
            Ok(())
        }
        fn observe_reset(&mut self, code: i32) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::ResetIntegrityRead) { return Err(ProtocolFault::UnexpectedObservation); }
            let interrupted = self.fields.work.terminal().is_some() || self.fields.physical.primary.is_some();
            let i = self.drain_index()?;
            let query = if i == 0 { IntegrityQuery::Check } else { IntegrityQuery::Foreign };
            let state = self.fields.native.statements[i].live_mut().ok_or(ProtocolFault::ResourceNotInstalled)?;
            record_once(&mut state.reset, code)?; state.cursor = CursorPhase::NoCursor;
            let r = &mut self.fields.native.integrity_read;
            if interrupted {
                r.failed = true; r.phase = IntegrityPhase::Drain; r.stage = IntegrityStage::Drain;
                // A normal primary stops work, but does not consume the
                // independent owned ignored Result returned by Rows::reset.
                if code != rusqlite::ffi::SQLITE_OK && self.fields.work.terminal().is_none() {
                    r.need_cleanup = Some((query, IntegrityCleanup::Reset));
                }
            } else if code != rusqlite::ffi::SQLITE_OK && r.eof[i] && r.raw.is_none() {
                r.failed = true; r.raw_cut = IntegrityErrorCut::Read; r.phase = IntegrityPhase::NeedRaw;
            } else {
                r.phase = IntegrityPhase::AwaitWhole;
                if code != rusqlite::ffi::SQLITE_OK { r.need_cleanup = Some((query, IntegrityCleanup::Reset)); }
            }
            self.adverse(query, code); Ok(())
        }
        fn retain_whole_error_return(&mut self, fact: IntegrityReturn) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitIntegrityWholeReturn) || !matches!(fact, IntegrityReturn::Error | IntegrityReturn::Interrupted) { return Err(ProtocolFault::UnexpectedObservation); }
            let interrupted = self.fields.work.terminal().is_some() || self.fields.physical.primary.is_some();
            let r = &mut self.fields.native.integrity_read;
            if r.returned[r.query.index()] != IntegrityReturn::Unknown { return Err(ProtocolFault::RepeatedObservation); }
            if fact == IntegrityReturn::Error && r.raw.is_none() { return Err(ProtocolFault::UnexpectedObservation); }
            if fact == IntegrityReturn::Interrupted && (!interrupted || (r.eof[r.query.index()] && r.raw.is_none())) { return Err(ProtocolFault::UnexpectedObservation); }
            r.returned[r.query.index()] = fact; r.failed = true;
            r.phase = if r.raw.is_some() { IntegrityPhase::Wrap } else { IntegrityPhase::Drain };
            if r.phase == IntegrityPhase::Drain { r.stage = IntegrityStage::Drain; }
            Ok(())
        }
        fn retain_owned_whole_collection(&mut self, rows: Vec<String>) -> Result<(), Vec<String>> {
            if self.next() != Some(LifecycleAction::AwaitIntegrityWholeReturn) { return Err(rows); }
            let r = &mut self.fields.native.integrity_read;
            if r.query != IntegrityQuery::Check || r.returned[0] != IntegrityReturn::Unknown || !r.eof[0]
                || r.raw.is_some() || self.prefix.integrity_rows.is_some()
                || rows.len() != r.rows[0] || rows.iter().all(|s| s == "ok") != r.first_all_ok { return Err(rows); }
            // This moves the real whole collection; it neither reconstructs
            // collect nor infers capacity/payment from scalar ROW facts.
            self.prefix.integrity_rows = Some(rows); r.vector_live = true;
            r.returned[0] = IntegrityReturn::Ok; r.phase = IntegrityPhase::Compare; Ok(())
        }
        fn retain_foreign_eof_return(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitIntegrityWholeReturn) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.integrity_read;
            if r.query != IntegrityQuery::Foreign || r.returned[1] != IntegrityReturn::Unknown || !r.eof[1] || r.raw.is_some() { return Err(ProtocolFault::UnexpectedObservation); }
            r.returned[1] = IntegrityReturn::Ok; r.phase = IntegrityPhase::Compare; Ok(())
        }
        fn check_semantic(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::CheckIntegritySemantic) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.integrity_read;
            if r.query == IntegrityQuery::Check {
                let rows = self.prefix.integrity_rows.as_ref().ok_or(ProtocolFault::ResourceNotInstalled)?;
                if rows.as_slice() != ["ok"] { self.prefix.fault = Some(super::super::super::FinancialIntegrityFault::IntegrityRows); r.failed = true; r.phase = IntegrityPhase::NeedDetail; }
                else { r.query = IntegrityQuery::Foreign; r.stage = IntegrityStage::Foreign; r.phase = IntegrityPhase::Prepare; }
            } else if self.prefix.foreign_key_violations != 0 {
                self.prefix.fault = Some(super::super::super::FinancialIntegrityFault::ForeignNonzero); r.failed = true; r.phase = IntegrityPhase::NeedDetail;
            } else { r.stage = IntegrityStage::Drain; r.phase = IntegrityPhase::Drain; }
            Ok(())
        }
        fn wrap_existing_raw(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::WrapIntegrityRaw) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.integrity_read;
            let raw = r.raw.take().ok_or(ProtocolFault::ResourceNotInstalled)?;
            let query = if r.query == IntegrityQuery::Check { super::super::super::FinancialIntegrityQuery::Check } else { super::super::super::FinancialIntegrityQuery::Foreign };
            let cut = match r.raw_cut { IntegrityErrorCut::Prepare => super::super::super::FinancialIntegrityErrorCut::Prepare,
                IntegrityErrorCut::Query => super::super::super::FinancialIntegrityErrorCut::Query, IntegrityErrorCut::Read => super::super::super::FinancialIntegrityErrorCut::Read };
            self.fields.physical.primary = Some(super::super::super::retain_integrity_driver_error(query, cut, raw));
            self.fields.physical.audit_phase = super::super::super::FinancialAuditPhase::Failed;
            r.stage = IntegrityStage::Drain; r.phase = IntegrityPhase::Drain; Ok(())
        }
        fn detail_port<'short>(&'short mut self) -> Result<OriginalIntegrityDetailPort<'short, 'a>, ProtocolFault> {
            if self.next() != Some(LifecycleAction::RetainIntegrityDetail) || self.prefix.fault.is_none() { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.integrity_read; r.detail_started = true; r.detail_returned = false;
            Ok(OriginalIntegrityDetailPort { loan: self })
        }
        fn retain_existing_detail(&mut self, detail: String) -> Result<(), String> {
            if self.next() != Some(LifecycleAction::AwaitIntegrityDetailReturn) || self.prefix.paid_detail.is_some() { return Err(detail); }
            self.prefix.paid_detail = Some(detail); self.fields.native.integrity_read.detail_returned = true; Ok(())
        }
        fn wrap_paid_detail(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::WrapIntegrityDetail) { return Err(ProtocolFault::UnexpectedObservation); }
            let detail = self.prefix.paid_detail.take().ok_or(ProtocolFault::ResourceNotInstalled)?;
            self.fields.physical.primary = Some(SourceOperationError::Global(super::super::super::GlobalSchemaV1Error::SelectionSnapshotChanged { detail }));
            self.fields.physical.audit_phase = super::super::super::FinancialAuditPhase::Failed;
            let r = &mut self.fields.native.integrity_read; r.failed = true; r.detail_started = false;
            r.semantic_detail_closed = true; r.stage = IntegrityStage::Drain; r.phase = IntegrityPhase::Drain; Ok(())
        }
        fn discard_paid_detail(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::DiscardIntegrityOwnedDetail) { return Err(ProtocolFault::UnexpectedObservation); }
            drop(self.prefix.paid_detail.take()); let r = &mut self.fields.native.integrity_read;
            r.detail_started = false; r.semantic_detail_closed = true; r.failed = true;
            r.stage = IntegrityStage::Drain; r.phase = IntegrityPhase::Drain; Ok(())
        }
        fn observe_finalize(&mut self, code: i32) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::FinalizeIntegrityRead) { return Err(ProtocolFault::UnexpectedObservation); }
            let i = self.drain_index()?;
            let query = if i == 0 { IntegrityQuery::Check } else { IntegrityQuery::Foreign };
            let mut state = *self.fields.native.statements[i].live().ok_or(ProtocolFault::ResourceNotInstalled)?;
            if state.cursor != CursorPhase::NoCursor { return Err(ProtocolFault::UnexpectedObservation); }
            record_once(&mut state.finalize, code)?;
            self.fields.native.statements[i] = StmtSlot::Finalized(state);
            let r = &mut self.fields.native.integrity_read; r.consumed[i] = Some(state);
            if code != rusqlite::ffi::SQLITE_OK && self.fields.work.terminal().is_none() { r.need_cleanup = Some((query, IntegrityCleanup::Finalize)); }
            r.phase = IntegrityPhase::Drain;
            self.adverse(query, code); Ok(())
        }
        fn retain_existing_cleanup(&mut self, error: rusqlite::Error) -> Result<(), rusqlite::Error> {
            if self.next() != Some(LifecycleAction::RetainIntegrityOwnedCleanup) || self.fields.native.integrity_read.ignored.is_some() { return Err(error); }
            self.fields.native.integrity_read.ignored = Some(error); Ok(())
        }
        fn discard_owned_cleanup(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::DiscardIntegrityOwnedCleanup) { return Err(ProtocolFault::UnexpectedObservation); }
            let primary_drain = self.fields.work.terminal().is_none() && self.fields.physical.primary.is_some();
            let r = &mut self.fields.native.integrity_read;
            drop(r.ignored.take());
            let (_, kind) = r.need_cleanup.take().ok_or(ProtocolFault::UnexpectedObservation)?;
            r.phase = if kind == IntegrityCleanup::Reset && !primary_drain { IntegrityPhase::AwaitWhole } else { IntegrityPhase::Drain }; Ok(())
        }
        fn discard_raw(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::DiscardIntegrityRaw) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.integrity_read;
            drop(r.raw.take()); r.stage = IntegrityStage::Drain; r.phase = IntegrityPhase::Drain; Ok(())
        }
        fn retain_partial_owned(&mut self, row: Option<String>, rows: Option<Vec<String>>) -> Result<(), (Option<String>, Option<Vec<String>>)> {
            let r = &mut self.fields.native.integrity_read;
            if matches!(r.stage, IntegrityStage::Dormant | IntegrityStage::Ready | IntegrityStage::Stopped)
                || r.capture_returned != IntegrityReturn::Unknown || r.partial_live || (row.is_none() && rows.is_none())
                || (!r.failed && self.fields.work.terminal().is_none() && self.fields.physical.primary.is_none()) { return Err((row, rows)); }
            self.prefix.partial_row = row; self.prefix.partial_rows = rows; r.partial_live = true; r.partial_returned = false; Ok(())
        }
        fn observe_partial_owner_return(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitIntegrityPartialReturn) { return Err(ProtocolFault::UnexpectedObservation); }
            // Independent Source ownership return, never derived from ROW,
            // reset/finalize, capacity or the outer collect Error marker.
            self.fields.native.integrity_read.partial_returned = true; Ok(())
        }
        fn discard_vector(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::DiscardIntegrityOwnedVector) { return Err(ProtocolFault::UnexpectedObservation); }
            drop(self.prefix.integrity_rows.take()); self.fields.native.integrity_read.vector_live = false; Ok(())
        }
        fn discard_partial(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::DiscardIntegrityPartial) { return Err(ProtocolFault::UnexpectedObservation); }
            drop(self.prefix.partial_row.take()); drop(self.prefix.partial_rows.take());
            self.fields.native.integrity_read.partial_live = false; Ok(())
        }
        fn retain_capture_return(&mut self, fact: IntegrityReturn) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitIntegrityCaptureReturn)
                || !matches!(fact, IntegrityReturn::Ok | IntegrityReturn::Error | IntegrityReturn::Interrupted) { return Err(ProtocolFault::UnexpectedObservation); }
            let interrupted = self.fields.work.terminal().is_some() || self.fields.physical.primary.is_some();
            let r = &mut self.fields.native.integrity_read;
            if r.capture_returned != IntegrityReturn::Unknown { return Err(ProtocolFault::RepeatedObservation); }
            if fact == IntegrityReturn::Ok {
                if r.failed || r.returned != [IntegrityReturn::Ok; 2] || !self.prefix.integrity_rows.as_ref().is_some_and(|rows| rows.as_slice() == ["ok"]) {
                    return Err(ProtocolFault::UnexpectedObservation);
                }
                if self.prefix.foreign_key_violations != 0 { return Err(ProtocolFault::UnexpectedObservation); }
            } else if !r.failed && !interrupted { return Err(ProtocolFault::UnexpectedObservation); }
            r.capture_returned = fact; Ok(())
        }
        fn stop_or_reach(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::StopIntegrityRead) || self.fields.native.statements.iter().any(|s| s.live().is_some()) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.integrity_read;
            r.stage = if r.failed || self.fields.work.terminal().is_some() || self.fields.physical.primary.is_some() { IntegrityStage::Stopped } else { IntegrityStage::Ready };
            Ok(())
        }
    }
    impl OriginalIntegrityDetailPort<'_, '_> {
        fn retain(self, detail: String) {
            // Preflight holds the exclusive Work/physical/frame loan. The
            // acquired owned return moves before any further gate or Drop.
            self.loan.prefix.paid_detail = Some(detail);
            self.loan.fields.native.integrity_read.detail_returned = true;
        }
        // Dropping an unconsumed short port leaves its independent return
        // Unknown; terminal cannot erase this owed owned-result cut.
    }
    impl OriginalIntegrityTextLoan<'_, '_, '_> {
        fn observe_owned_string_return(self) -> Result<(), ProtocolFault> {
            if self.loan.next() != Some(LifecycleAction::IntegrityText) { return Err(ProtocolFault::UnexpectedObservation); }
            // Actual typed String return fact only. The String remains with
            // its actual collect owner until the whole Result returns it.
            let r = &mut self.loan.fields.native.integrity_read;
            r.pending_text_ok = self.text == "ok"; r.phase = IntegrityPhase::RowReturn; Ok(())
        }
    }
    impl<'a> OriginalIntegrityPort<'_, 'a, '_> {
        fn sql(&self) -> &'static str { self.query.sql() }
        fn prepare(&mut self, code: i32) -> Result<(), ProtocolFault> { self.loan.observe_prepare(code) }
        fn empty_query(&mut self, fact: IntegrityReturn) -> Result<(), ProtocolFault> { self.loan.observe_empty_query(fact) }
        fn step(&mut self, code: i32) -> Result<(), ProtocolFault> { self.loan.observe_step(code) }
        fn column_type(&mut self, kind: i32) -> Result<(), ProtocolFault> { self.loan.observe_column_type(kind) }
        fn text<'bytes, 'short>(&'short mut self, bytes: &'bytes [u8]) -> Result<OriginalIntegrityTextLoan<'bytes, 'short, 'a>, ProtocolFault> { self.loan.text(bytes) }
        fn row_return(&mut self) -> Result<(), ProtocolFault> { self.loan.observe_row_return() }
        fn owned_raw(&mut self, error: rusqlite::Error) -> Result<(), rusqlite::Error> { self.loan.retain_existing_raw(error) }
        fn reset(&mut self, code: i32) -> Result<(), ProtocolFault> { self.loan.observe_reset(code) }
        fn finalize(&mut self, code: i32) -> Result<(), ProtocolFault> { self.loan.observe_finalize(code) }
        fn owned_cleanup(&mut self, error: rusqlite::Error) -> Result<(), rusqlite::Error> { self.loan.retain_existing_cleanup(error) }
        fn whole_collection(&mut self, rows: Vec<String>) -> Result<(), Vec<String>> { self.loan.retain_owned_whole_collection(rows) }
        fn whole_error(&mut self, fact: IntegrityReturn) -> Result<(), ProtocolFault> { self.loan.retain_whole_error_return(fact) }
        fn foreign_eof(&mut self) -> Result<(), ProtocolFault> { self.loan.retain_foreign_eof_return() }
        fn partial_owner_return(&mut self) -> Result<(), ProtocolFault> { self.loan.observe_partial_owner_return() }
    }
    #[cfg(test)]
    impl OriginalIntegrityReadLoan<'_> {
        fn test_code_barrier(&mut self) {
            let before = self.fields.work.test_code_observation();
            let expected = self.next().expect("fixed integrity scope remains pending");
            assert_eq!(self.fields.native.transaction_action(&self.fields.work, self.fields.physical), Some(expected));
            assert!(!self.fields.native.transaction_release_ready(&self.fields.work, self.fields.physical));
            let mut acquire = self.fields.reborrow().original_acquisition();
            assert_eq!(acquire.constructor().next(), expected);
            assert_eq!(acquire.a00_epilogue().next(), expected);
            assert_eq!(acquire.constructor().observe(ConstructorObservation::Close(rusqlite::ffi::SQLITE_OK)), Err(ProtocolFault::UnexpectedObservation));
            assert!(matches!(acquire.settle(), AcquisitionSettlement::Held(_)));
            assert_eq!(self.fields.work.test_code_observation(), before);
        }
        fn test_code_prepare_only(&mut self) {
            let query = self.fields.native.integrity_read.query;
            assert_eq!(query.sql(), if query == IntegrityQuery::Check { "PRAGMA integrity_check" } else { "PRAGMA foreign_key_check" });
            assert_eq!(self.observe_prepare(rusqlite::ffi::SQLITE_OK), Err(ProtocolFault::ResourceNotInstalled));
            self.fields.native.statements[query.index()] = StmtSlot::ProtocolHeld(StmtState { role: Role::Reference, action: query.action(), ..protocol_stmt_state() });
            assert_eq!(self.observe_prepare(rusqlite::ffi::SQLITE_OK), Err(ProtocolFault::UnexpectedObservation));
            self.fields.native.statements[query.index()] = StmtSlot::ProtocolHeld(StmtState { action: FixedAction::SourceId, ..protocol_stmt_state() });
            assert_eq!(self.observe_prepare(rusqlite::ffi::SQLITE_OK), Err(ProtocolFault::UnexpectedObservation));
            self.fields.native.statements[query.index()] = StmtSlot::ProtocolHeld(StmtState { action: query.action(), ..protocol_stmt_state() });
            self.observe_prepare(rusqlite::ffi::SQLITE_OK).unwrap();
            assert_eq!(self.observe_prepare(rusqlite::ffi::SQLITE_OK), Err(ProtocolFault::UnexpectedObservation));
        }
        fn test_code_prepare_query(&mut self) {
            self.test_code_prepare_only(); self.observe_empty_query(IntegrityReturn::Ok).unwrap();
        }
        fn test_code_check_row(&mut self, text: &str) {
            self.observe_step(rusqlite::ffi::SQLITE_ROW).unwrap();
            assert_eq!(self.observe_step(rusqlite::ffi::SQLITE_DONE), Err(ProtocolFault::UnexpectedObservation));
            self.observe_column_type(rusqlite::ffi::SQLITE_TEXT).unwrap();
            { let _short = self.text(text.as_bytes()).unwrap(); }
            assert_eq!(self.next(), Some(LifecycleAction::IntegrityText));
            self.text(text.as_bytes()).unwrap().observe_owned_string_return().unwrap();
            assert_eq!(self.next(), Some(LifecycleAction::AwaitIntegrityRowReturn));
            self.observe_row_return().unwrap();
        }
        pub(in crate::database::global_schema_v1) fn test_code_whole_check(&mut self, rows: Vec<String>) -> bool {
            let before = self.fields.work.test_code_observation();
            self.test_code_barrier(); self.test_code_prepare_query();
            for text in &rows { self.test_code_check_row(text); }
            let premature = Vec::new();
            assert!(self.retain_owned_whole_collection(premature).is_err());
            self.observe_step(rusqlite::ffi::SQLITE_DONE).unwrap(); self.observe_reset(rusqlite::ffi::SQLITE_OK).unwrap();
            assert_eq!(self.next(), Some(LifecycleAction::AwaitIntegrityWholeReturn));
            assert!(self.prefix.integrity_rows.is_none()); self.test_code_barrier();
            assert_eq!(self.retain_whole_error_return(IntegrityReturn::Error), Err(ProtocolFault::UnexpectedObservation));
            let vector = rows.as_ptr(); let child = rows.first().map(|s| s.as_ptr());
            self.retain_owned_whole_collection(rows).unwrap_or_else(|_| panic!("actual supplied whole Vec moves once"));
            assert_eq!(self.prefix.integrity_rows.as_ref().unwrap().as_ptr(), vector);
            assert_eq!(self.prefix.integrity_rows.as_ref().unwrap().first().map(|s| s.as_ptr()), child);
            assert!(self.retain_owned_whole_collection(Vec::new()).is_err());
            assert!(self.fields.native.statements[0].live().is_some());
            self.check_semantic().unwrap(); assert_eq!(self.fields.work.test_code_observation(), before);
            self.next() == Some(LifecycleAction::PrepareIntegrityRead)
        }
        pub(in crate::database::global_schema_v1) fn test_code_foreign_rows(&mut self, rows: usize, overflow: bool) {
            let vector = self.prefix.integrity_rows.as_ref().unwrap().as_ptr();
            assert!(self.fields.native.statements[0].live().is_some()); self.test_code_prepare_query();
            assert!(self.fields.native.statements[0].live().is_some());
            if overflow { self.prefix.foreign_key_violations = i64::MAX; } // Fixed scalar boundary protocol, not 2^63 actual SQL rows.
            for _ in 0..rows {
                self.observe_step(rusqlite::ffi::SQLITE_ROW).unwrap();
                assert_eq!(self.observe_column_type(rusqlite::ffi::SQLITE_TEXT), Err(ProtocolFault::UnexpectedObservation));
                self.observe_row_return().unwrap();
                if overflow { assert_eq!(self.next(), Some(LifecycleAction::RetainIntegrityDetail)); break; }
            }
            if !overflow {
                self.observe_step(rusqlite::ffi::SQLITE_DONE).unwrap(); self.observe_reset(rusqlite::ffi::SQLITE_OK).unwrap();
                assert_eq!(self.next(), Some(LifecycleAction::AwaitIntegrityWholeReturn)); self.test_code_barrier();
                assert_eq!(self.retain_capture_return(IntegrityReturn::Ok), Err(ProtocolFault::UnexpectedObservation));
                self.retain_foreign_eof_return().unwrap(); self.check_semantic().unwrap();
            }
            assert_eq!(self.prefix.integrity_rows.as_ref().unwrap().as_ptr(), vector);
            assert!(self.fields.native.statements[0].live().is_some());
        }
        pub(in crate::database::global_schema_v1) fn test_code_paid_semantic_error(&mut self, detail: String) {
            let pointer = detail.as_ptr();
            self.detail_port().unwrap().retain(detail);
            self.wrap_paid_detail().unwrap(); assert!(self.wrap_paid_detail().is_err());
            let Some(SourceOperationError::Global(super::super::super::GlobalSchemaV1Error::SelectionSnapshotChanged { detail })) = &self.fields.physical.primary else { panic!("fixed semantic category"); };
            assert_eq!(detail.as_ptr(), pointer);
            assert!(self.fields.native.statements[0].live().is_some());
            assert!(self.check_semantic().is_err()); self.test_code_barrier();
        }
        pub(in crate::database::global_schema_v1) fn test_code_driver_error(&mut self, case: super::super::super::FinancialIntegrityErrorCase,
            raw: rusqlite::Error, cleanup: rusqlite::Error) {
            use super::super::super::FinancialIntegrityErrorCase as Case;
            let foreign = matches!(case, Case::ForeignPrepare | Case::ForeignQuery | Case::ForeignStep);
            if foreign { assert!(self.test_code_whole_check(vec![String::from("ok")])); }
            if matches!(case, Case::Prepare | Case::ForeignPrepare) {
                self.observe_prepare(rusqlite::ffi::SQLITE_ERROR).unwrap();
            } else {
                self.test_code_prepare_only();
                if matches!(case, Case::Query | Case::ForeignQuery) {
                    // This is the first empty-query Result, never a rewind of
                    // an already-observed successful call or active cursor.
                    self.observe_empty_query(IntegrityReturn::Error).unwrap();
                } else {
                    self.observe_empty_query(IntegrityReturn::Ok).unwrap();
                    if matches!(case, Case::WrongType | Case::WrongNull | Case::WrongReal | Case::WrongBlob | Case::InvalidUtf8) {
                        self.observe_step(rusqlite::ffi::SQLITE_ROW).unwrap();
                        if matches!(case, Case::InvalidUtf8) {
                            self.observe_column_type(rusqlite::ffi::SQLITE_TEXT).unwrap(); assert!(self.text(&[0xff]).is_err());
                        } else {
                            let kind = match case { Case::WrongNull => rusqlite::ffi::SQLITE_NULL,
                                Case::WrongReal => rusqlite::ffi::SQLITE_FLOAT, Case::WrongBlob => rusqlite::ffi::SQLITE_BLOB,
                                _ => rusqlite::ffi::SQLITE_INTEGER };
                            self.observe_column_type(kind).unwrap();
                        }
                    } else if matches!(case, Case::DoneReset) {
                        self.observe_step(rusqlite::ffi::SQLITE_DONE).unwrap(); self.observe_reset(rusqlite::ffi::SQLITE_ERROR).unwrap();
                    } else { self.observe_step(rusqlite::ffi::SQLITE_ERROR).unwrap(); }
                }
            }
            self.retain_existing_raw(raw).unwrap_or_else(|_| panic!("actual owned driver child before remaining scopes"));
            if self.next() == Some(LifecycleAction::ResetIntegrityRead) {
                self.observe_reset(rusqlite::ffi::SQLITE_ERROR).unwrap();
                assert_eq!(self.next(), Some(LifecycleAction::RetainIntegrityOwnedCleanup));
                self.retain_existing_cleanup(cleanup).unwrap_or_else(|_| panic!("actual ignored reset Result owned once"));
                { let _short = self.fields.reborrow(); }
                self.test_code_barrier(); self.discard_owned_cleanup().unwrap();
                assert!(self.discard_owned_cleanup().is_err());
            } else { drop(cleanup); }
            assert_eq!(self.next(), Some(LifecycleAction::AwaitIntegrityWholeReturn));
            self.test_code_barrier(); assert!(self.fields.physical.primary.is_none());
            assert!(self.retain_owned_whole_collection(Vec::new()).is_err());
            self.retain_whole_error_return(IntegrityReturn::Error).unwrap();
            assert!(self.retain_whole_error_return(IntegrityReturn::Error).is_err());
            self.wrap_existing_raw().unwrap();
            assert!(self.wrap_existing_raw().is_err());
            if !matches!(case, Case::Prepare) { assert!(self.fields.native.statements[0].live().is_some()); }
            self.test_code_barrier();
        }
        pub(in crate::database::global_schema_v1) fn test_code_finish_failed_scope(&mut self, cleanup: rusqlite::Error) {
            let before = self.fields.work.test_code_observation();
            let mut cleanup = Some(cleanup);
            if self.next() == Some(LifecycleAction::ResetIntegrityRead) { self.observe_reset(rusqlite::ffi::SQLITE_OK).unwrap(); }
            if self.fields.native.statements[1].live().is_some() {
                assert_eq!(self.next(), Some(LifecycleAction::FinalizeIntegrityRead));
                let vector = self.prefix.integrity_rows.as_ref().map(|v| v.as_ptr());
                self.observe_finalize(rusqlite::ffi::SQLITE_OK).unwrap();
                assert_eq!(self.prefix.integrity_rows.as_ref().map(|v| v.as_ptr()), vector);
                assert!(self.fields.native.statements[0].live().is_some());
            }
            if self.prefix.integrity_rows.is_some() {
                assert_eq!(self.next(), Some(LifecycleAction::DiscardIntegrityOwnedVector));
                assert!(self.fields.native.statements[0].live().is_some());
                self.discard_vector().unwrap(); assert!(self.prefix.integrity_rows.is_none());
                assert!(self.discard_vector().is_err());
            }
            if self.fields.native.statements[0].live().is_some() {
                assert_eq!(self.next(), Some(LifecycleAction::FinalizeIntegrityRead));
                self.observe_finalize(rusqlite::ffi::SQLITE_ERROR).unwrap();
                self.test_code_barrier();
                self.retain_existing_cleanup(cleanup.take().unwrap()).unwrap_or_else(|_| panic!("owned finalize Result persists after VM consumption"));
                self.test_code_barrier(); self.discard_owned_cleanup().unwrap();
            }
            drop(cleanup);
            assert!(self.fields.native.statements.iter().all(|s| s.live().is_none()));
            assert_eq!(self.next(), Some(LifecycleAction::AwaitIntegrityCaptureReturn)); self.test_code_barrier();
            assert_eq!(self.retain_capture_return(IntegrityReturn::Ok), Err(ProtocolFault::UnexpectedObservation));
            self.retain_capture_return(IntegrityReturn::Error).unwrap();
            assert!(self.retain_capture_return(IntegrityReturn::Error).is_err());
            self.stop_or_reach().unwrap(); assert!(self.stop_or_reach().is_err());
            assert_eq!(self.fields.native.integrity_read.stage, IntegrityStage::Stopped);
            assert_eq!(self.fields.work.test_code_observation(), before);
        }
        pub(in crate::database::global_schema_v1) fn test_code_finish_success(&mut self) {
            let vector = self.prefix.integrity_rows.as_ref().unwrap().as_ptr();
            self.observe_finalize(rusqlite::ffi::SQLITE_OK).unwrap();
            assert!(self.fields.native.statements[0].live().is_some());
            assert_eq!(self.prefix.integrity_rows.as_ref().unwrap().as_ptr(), vector);
            self.observe_finalize(rusqlite::ffi::SQLITE_OK).unwrap();
            assert_eq!(self.next(), Some(LifecycleAction::AwaitIntegrityCaptureReturn)); self.test_code_barrier();
            assert_eq!(self.prefix.integrity_rows.as_ref().unwrap().as_ptr(), vector);
            assert!(self.discard_vector().is_err());
            assert_eq!(self.retain_capture_return(IntegrityReturn::Error), Err(ProtocolFault::UnexpectedObservation));
            self.retain_capture_return(IntegrityReturn::Ok).unwrap(); self.stop_or_reach().unwrap();
            assert_eq!(self.next(), Some(LifecycleAction::IntegrityPrefixReached)); self.test_code_barrier();
            assert_eq!(self.prefix.integrity_rows.as_ref().unwrap().as_ptr(), vector);
            assert!(self.fields.native.integrity_read.consumed.iter().all(Option::is_some));
        }
        pub(in crate::database::global_schema_v1) fn test_code_interruption_cut(&mut self, cut: super::super::super::FinancialIntegrityTerminalCut,
            raw: rusqlite::Error, cleanup: rusqlite::Error) {
            use super::super::super::FinancialIntegrityTerminalCut as Cut;
            if matches!(cut, Cut::BeforePrepare) { drop(raw); drop(cleanup); return; }
            if matches!(cut, Cut::CompletedBeforeWhole) {
                self.test_code_prepare_query(); self.test_code_check_row("ok");
                self.observe_step(rusqlite::ffi::SQLITE_DONE).unwrap(); self.observe_reset(rusqlite::ffi::SQLITE_OK).unwrap();
                assert_eq!(self.next(), Some(LifecycleAction::AwaitIntegrityWholeReturn));
                assert!(self.prefix.integrity_rows.is_none()); drop(raw); drop(cleanup); return;
            }
            if matches!(cut, Cut::PaidDetailPending | Cut::OwnedPaidDetail) {
                assert!(!self.test_code_whole_check(vec![String::from("bad")]));
                if matches!(cut, Cut::PaidDetailPending) { let _short = self.detail_port().unwrap(); }
                drop(raw); drop(cleanup); return;
            }
            if matches!(cut, Cut::SemanticDetail) {
                assert!(!self.test_code_whole_check(vec![String::from("bad")]));
                assert_eq!(self.next(), Some(LifecycleAction::RetainIntegrityDetail)); drop(raw); drop(cleanup); return;
            }
            if matches!(cut, Cut::ForeignEofPending) {
                assert!(self.test_code_whole_check(vec![String::from("ok")])); self.test_code_prepare_query();
                self.observe_step(rusqlite::ffi::SQLITE_DONE).unwrap(); self.observe_reset(rusqlite::ffi::SQLITE_OK).unwrap();
                assert_eq!(self.next(), Some(LifecycleAction::AwaitIntegrityWholeReturn)); drop(raw); drop(cleanup); return;
            }
            if matches!(cut, Cut::AfterVector | Cut::ForeignRow) {
                assert!(self.test_code_whole_check(vec![String::from("ok")]));
                if matches!(cut, Cut::ForeignRow) { self.test_code_prepare_query(); self.observe_step(rusqlite::ffi::SQLITE_ROW).unwrap(); }
                drop(raw); drop(cleanup); return;
            }
            self.test_code_prepare_query();
            if matches!(cut, Cut::RowWithPartialOwner) { self.observe_step(rusqlite::ffi::SQLITE_ROW).unwrap(); drop(raw); drop(cleanup); }
            else {
                self.observe_step(rusqlite::ffi::SQLITE_ERROR).unwrap();
                self.retain_existing_raw(raw).unwrap_or_else(|_| panic!("actual raw child retained before terminal"));
                if matches!(cut, Cut::OwnedIgnoredReset) {
                    self.observe_reset(rusqlite::ffi::SQLITE_ERROR).unwrap();
                    self.retain_existing_cleanup(cleanup).unwrap_or_else(|_| panic!("actual owned ignored Result before terminal"));
                } else { drop(cleanup); }
            }
        }
        pub(in crate::database::global_schema_v1) fn test_code_retain_detail_before_interruption(&mut self, detail: String) {
            let pointer = detail.as_ptr(); self.detail_port().unwrap().retain(detail);
            assert_eq!(self.prefix.paid_detail.as_ref().unwrap().as_ptr(), pointer);
            assert_eq!(self.next(), Some(LifecycleAction::WrapIntegrityDetail)); self.test_code_barrier();
        }
        pub(in crate::database::global_schema_v1) fn test_code_detail_at_interruption(&mut self, detail: String) {
            let pointer = detail.as_ptr(); assert_eq!(self.next(), Some(LifecycleAction::AwaitIntegrityDetailReturn)); self.test_code_barrier();
            self.retain_existing_detail(detail).unwrap_or_else(|_| panic!("already-owned real formatter return after barrier"));
            assert_eq!(self.prefix.paid_detail.as_ref().unwrap().as_ptr(), pointer);
            assert_eq!(self.next(), Some(LifecycleAction::DiscardIntegrityOwnedDetail));
            assert!(self.wrap_paid_detail().is_err()); self.test_code_barrier();
        }
        pub(in crate::database::global_schema_v1) fn test_code_completed_whole_at_interruption(&mut self, rows: Vec<String>) {
            assert!(self.fields.work.terminal().is_some() || self.fields.physical.primary.is_some());
            assert_eq!(self.next(), Some(LifecycleAction::AwaitIntegrityWholeReturn)); self.test_code_barrier();
            assert_eq!(self.retain_whole_error_return(IntegrityReturn::Interrupted), Err(ProtocolFault::UnexpectedObservation));
            let pointer = rows.as_ptr(); self.retain_owned_whole_collection(rows).unwrap_or_else(|_| panic!("independent actual whole ownership"));
            assert_eq!(self.prefix.integrity_rows.as_ref().unwrap().as_ptr(), pointer);
            assert!(self.check_semantic().is_err()); // first-primary stops fresh FK prepare.
        }
        pub(in crate::database::global_schema_v1) fn test_code_foreign_eof_at_interruption(&mut self) {
            assert!(self.fields.work.terminal().is_some() || self.fields.physical.primary.is_some());
            assert_eq!(self.next(), Some(LifecycleAction::AwaitIntegrityWholeReturn)); self.test_code_barrier();
            assert_eq!(self.retain_whole_error_return(IntegrityReturn::Interrupted), Err(ProtocolFault::UnexpectedObservation));
            self.retain_foreign_eof_return().unwrap(); assert!(self.check_semantic().is_err());
            assert_eq!(self.fields.native.integrity_read.returned, [IntegrityReturn::Ok; 2]);
        }
        pub(in crate::database::global_schema_v1) fn test_code_partial_owner(&mut self, row: String, rows: Vec<String>) {
            let row_pointer = row.as_ptr(); let vector_pointer = rows.as_ptr();
            self.retain_partial_owned(Some(row), Some(rows)).unwrap_or_else(|_| panic!("actual supplied partial children remain parent owned"));
            assert_eq!(self.prefix.partial_row.as_ref().unwrap().as_ptr(), row_pointer);
            assert_eq!(self.prefix.partial_rows.as_ref().unwrap().as_ptr(), vector_pointer);
            assert_eq!(self.next(), Some(LifecycleAction::AwaitIntegrityPartialReturn)); self.test_code_barrier();
            assert!(self.observe_step(rusqlite::ffi::SQLITE_DONE).is_err());
            self.observe_partial_owner_return().unwrap(); assert!(self.observe_partial_owner_return().is_err());
        }
        pub(in crate::database::global_schema_v1) fn test_code_primary_reset_cleanup(&mut self, cleanup: rusqlite::Error) -> usize {
            assert!(self.fields.work.terminal().is_none() && self.fields.physical.primary.is_some());
            let before = self.fields.work.test_code_observation();
            let returned = self.fields.native.integrity_read.returned;
            assert_eq!(self.next(), Some(LifecycleAction::ResetIntegrityRead));
            self.observe_reset(rusqlite::ffi::SQLITE_ERROR).unwrap();
            assert_eq!(self.next(), Some(LifecycleAction::RetainIntegrityOwnedCleanup));
            assert!(self.observe_finalize(rusqlite::ffi::SQLITE_OK).is_err());
            assert!(self.stop_or_reach().is_err()); self.test_code_barrier();
            let rusqlite::Error::InvalidColumnName(name) = &cleanup else { panic!("actual fixed owned reset Result"); };
            let allocation = name.as_ptr() as usize;
            self.retain_existing_cleanup(cleanup).unwrap_or_else(|_| panic!("normal primary retains actual ignored Result"));
            assert_eq!(self.next(), Some(LifecycleAction::DiscardIntegrityOwnedCleanup));
            assert!(self.observe_finalize(rusqlite::ffi::SQLITE_OK).is_err());
            assert!(self.stop_or_reach().is_err()); self.test_code_barrier();
            assert_eq!(self.fields.native.integrity_read.returned, returned);
            assert_eq!(self.fields.work.test_code_observation(), before); allocation
        }
        pub(in crate::database::global_schema_v1) fn test_code_primary_cleanup_once(&mut self, allocation: usize) {
            let before = self.fields.work.test_code_observation();
            let returned = self.fields.native.integrity_read.returned;
            let Some(rusqlite::Error::InvalidColumnName(name)) = &self.fields.native.integrity_read.ignored else { panic!("same-frame owned cleanup after short loan and move"); };
            assert_eq!(name.as_ptr() as usize, allocation);
            assert_eq!(self.next(), Some(LifecycleAction::DiscardIntegrityOwnedCleanup));
            assert!(self.observe_step(rusqlite::ffi::SQLITE_DONE).is_err());
            assert!(self.observe_finalize(rusqlite::ffi::SQLITE_OK).is_err());
            assert!(self.stop_or_reach().is_err()); self.test_code_barrier();
            self.discard_owned_cleanup().unwrap();
            assert!(self.fields.native.integrity_read.ignored.is_none() && self.fields.native.integrity_read.need_cleanup.is_none());
            assert_eq!(self.fields.native.integrity_read.phase, IntegrityPhase::Drain);
            assert_eq!(self.next(), Some(LifecycleAction::FinalizeIntegrityRead));
            assert!(self.discard_owned_cleanup().is_err());
            assert_eq!(self.fields.native.integrity_read.returned, returned);
            assert_eq!(self.fields.work.test_code_observation(), before);
        }
        pub(in crate::database::global_schema_v1) fn test_code_drain_interrupted(&mut self) {
            let before = self.fields.work.test_code_observation();
            assert!(self.fields.work.terminal().is_some() || self.fields.physical.primary.is_some());
            assert!(self.observe_empty_query(IntegrityReturn::Ok).is_err());
            if self.next() == Some(LifecycleAction::DiscardIntegrityOwnedCleanup) { self.discard_owned_cleanup().unwrap(); }
            if self.next() == Some(LifecycleAction::DiscardIntegrityOwnedDetail) {
                self.discard_paid_detail().unwrap(); assert!(self.prefix.paid_detail.is_none()); assert!(self.discard_paid_detail().is_err());
            }
            if self.next() == Some(LifecycleAction::AwaitIntegrityWholeReturn) {
                self.retain_whole_error_return(if self.fields.native.integrity_read.raw.is_some() { IntegrityReturn::Error } else { IntegrityReturn::Interrupted }).unwrap();
            }
            if self.next() == Some(LifecycleAction::DiscardIntegrityRaw) { self.discard_raw().unwrap(); }
            for _ in 0..2 {
                if self.next() == Some(LifecycleAction::ResetIntegrityRead) { self.observe_reset(rusqlite::ffi::SQLITE_ERROR).unwrap(); }
                if self.next() == Some(LifecycleAction::FinalizeIntegrityRead) { self.observe_finalize(rusqlite::ffi::SQLITE_OK).unwrap(); }
                if self.next() == Some(LifecycleAction::DiscardIntegrityOwnedVector) {
                    assert!(self.fields.native.statements[1].live().is_none()); self.discard_vector().unwrap();
                }
            }
            if self.next() == Some(LifecycleAction::DiscardIntegrityPartial) { self.discard_partial().unwrap(); }
            if self.next() == Some(LifecycleAction::AwaitIntegrityWholeReturn) {
                self.retain_whole_error_return(if self.fields.native.integrity_read.raw.is_some() { IntegrityReturn::Error } else { IntegrityReturn::Interrupted }).unwrap();
            }
            if self.next() == Some(LifecycleAction::DiscardIntegrityRaw) { self.discard_raw().unwrap(); }
            if self.next() == Some(LifecycleAction::AwaitIntegrityCaptureReturn) { self.test_code_barrier(); self.retain_capture_return(IntegrityReturn::Interrupted).unwrap(); }
            assert_eq!(self.next(), Some(LifecycleAction::StopIntegrityRead)); self.stop_or_reach().unwrap();
            assert!(self.fields.native.integrity_read.stopped_clear());
            assert!(self.prefix.integrity_rows.is_none() && self.prefix.partial_row.is_none() && self.prefix.partial_rows.is_none() && self.prefix.paid_detail.is_none());
            assert_eq!(self.fields.work.test_code_observation(), before);
        }
    }
    // C2082/C3136 fixed three-query prefix, before compile_options.
    // The private rusqlite pragma::Sql stays in its real callee scope. These
    // named lease obligations neither own that Sql nor prove its payment.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum CaptureQuery { ApplicationId, UserVersion, SourceId }
    impl CaptureQuery {
        fn index(self) -> usize { match self { Self::ApplicationId => 0, Self::UserVersion => 1, Self::SourceId => 2 } }
        fn action(self) -> FixedAction { if self == Self::SourceId { FixedAction::SourceId } else { FixedAction::Pragmas } }
        fn stage(self) -> &'static str { match self { Self::ApplicationId => "capture-application-id", Self::UserVersion => "capture-user-version", Self::SourceId => "capture-source-id" } }
        fn pragma(self) -> bool { self != Self::SourceId }
    }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum CaptureStage { Dormant, Reading, Ready, Stopped }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum CapturePhase { Scope, Prepare, Tail, Bind, Step, Type, Integer, Text, ValueReturn, NeedRaw, Reset, Drain }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum CaptureReturn { Unknown, Ok, Error, Interrupted }
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum CaptureOutcome { Unknown, Row, Value, Error, NoRows }
    #[derive(Clone, Copy)]
    struct CaptureCall {
        prepare: CodeSlot, rows_started: bool, outcome: CaptureOutcome,
        query_return: CaptureReturn, pragma_started: bool, pragma_scope_ended: bool,
        pragma_return: CaptureReturn, consumed: Option<StmtState>,
    }
    impl CaptureCall {
        fn empty() -> Self { Self { prepare: CodeSlot::NotCalled, rows_started: false, outcome: CaptureOutcome::Unknown,
            query_return: CaptureReturn::Unknown, pragma_started: false, pragma_scope_ended: false,
            pragma_return: CaptureReturn::Unknown, consumed: None } }
        fn scopes_complete(&self, query: CaptureQuery) -> bool {
            self.query_return != CaptureReturn::Unknown
                && (!query.pragma() || (self.pragma_scope_ended && self.pragma_return != CaptureReturn::Unknown))
        }
    }
    struct CapturePrefixRecord {
        stage: CaptureStage, phase: CapturePhase, query: CaptureQuery, calls: [CaptureCall; 3],
        raw: Option<rusqlite::Error>, ignored: Option<rusqlite::Error>, need_cleanup: bool,
        value_started: bool, value_returned: bool, value_live: bool, value_matches: bool,
        detail_started: bool, detail_returned: bool, catalog_live: bool,
        runtime_error_return: CaptureReturn, catalog_return: CaptureReturn, capture_started: bool,
    }
    impl CapturePrefixRecord {
        fn empty() -> Self { Self { stage: CaptureStage::Dormant, phase: CapturePhase::Scope,
            query: CaptureQuery::ApplicationId, calls: [CaptureCall::empty(); 3], raw: None, ignored: None,
            need_cleanup: false, value_started: false, value_returned: false, value_live: false, value_matches: false,
            detail_started: false, detail_returned: false, catalog_live: false,
            runtime_error_return: CaptureReturn::Unknown, catalog_return: CaptureReturn::Unknown, capture_started: false } }
        fn blocks_early_primary(&self) -> bool {
            self.raw.is_some() || self.ignored.is_some() || self.need_cleanup || self.detail_started
                || self.catalog_live || self.phase == CapturePhase::NeedRaw
                || (self.value_started && !self.value_returned)
        }
        fn stopped_clear(&self) -> bool {
            matches!(self.stage, CaptureStage::Dormant | CaptureStage::Stopped)
                && self.raw.is_none() && self.ignored.is_none() && !self.need_cleanup
                && !self.detail_started && !self.catalog_live && !self.value_live
                && (!self.value_started || self.value_returned)
        }
    }
    impl NativeOriginalOwner {
        fn capture_prefix_action(&self, work: &OriginalSourceWork<'_>, physical: &super::super::super::FinancialPhysical) -> Option<LifecycleAction> {
            if self.compile_options.stage != CompileStage::Dormant {
                return self.compile_options_action(work, physical);
            }
            let r = &self.capture_prefix;
            if matches!(r.stage, CaptureStage::Dormant | CaptureStage::Stopped) { return None; }
            if r.ignored.is_some() { return Some(LifecycleAction::DiscardCaptureCleanup); }
            if r.need_cleanup { return Some(LifecycleAction::RetainCaptureCleanup); }
            if r.phase == CapturePhase::NeedRaw { return Some(LifecycleAction::RetainCaptureRaw); }
            if r.value_started && !r.value_returned { return Some(LifecycleAction::AwaitCaptureString); }
            if r.detail_started && !r.detail_returned { return Some(LifecycleAction::AwaitCaptureDetail); }
            let interrupted = work.terminal().is_some() || physical.primary.is_some();
            let call = &r.calls[r.query.index()];
            let draining = interrupted || r.phase == CapturePhase::Reset || r.phase == CapturePhase::Drain || r.raw.is_some();
            if draining {
                if let Some(state) = self.statements[0].live() {
                    return Some(if state.cursor != CursorPhase::NoCursor && matches!(state.reset, CodeSlot::NotCalled) {
                        LifecycleAction::ResetCaptureRead
                    } else { LifecycleAction::FinalizeCaptureRead });
                }
                if !matches!(call.prepare, CodeSlot::NotCalled) && call.query_return == CaptureReturn::Unknown {
                    return Some(LifecycleAction::AwaitCaptureQueryReturn);
                }
            }
            if call.pragma_started && (call.query_return != CaptureReturn::Unknown || (interrupted && matches!(call.prepare, CodeSlot::NotCalled))) {
                if !call.pragma_scope_ended { return Some(LifecycleAction::AwaitCapturePragmaScopeEnd); }
                if call.pragma_return == CaptureReturn::Unknown { return Some(LifecycleAction::AwaitCapturePragmaReturn); }
            }
            if r.detail_started && r.detail_returned {
                return Some(if interrupted { LifecycleAction::DiscardCaptureDetail } else { LifecycleAction::BuildCaptureCatalogError });
            }
            if r.catalog_live {
                if r.query == CaptureQuery::SourceId && r.runtime_error_return == CaptureReturn::Unknown { return Some(LifecycleAction::AwaitCaptureRuntimeErrorReturn); }
                if r.catalog_return == CaptureReturn::Unknown { return Some(LifecycleAction::AwaitCaptureCatalogReturn); }
                return Some(if interrupted { LifecycleAction::DiscardCaptureCatalogError } else { LifecycleAction::WrapCaptureCatalogError });
            }
            if r.raw.is_some() { return Some(if interrupted { LifecycleAction::DiscardCaptureRaw } else { LifecycleAction::FormatCaptureDetail }); }
            if interrupted {
                if r.value_live { return Some(LifecycleAction::DiscardCaptureString); }
                if r.capture_started && r.catalog_return == CaptureReturn::Unknown { return Some(LifecycleAction::AwaitCaptureCatalogReturn); }
                return Some(LifecycleAction::StopCapturePrefix);
            }
            if r.stage == CaptureStage::Ready { return Some(LifecycleAction::CapturePrefixReached); }
            if call.scopes_complete(r.query) { return Some(LifecycleAction::AdvanceCaptureQuery); }
            Some(match r.phase {
                CapturePhase::Scope => LifecycleAction::BeginCapturePragmaScope,
                CapturePhase::Prepare => LifecycleAction::PrepareCaptureRead,
                CapturePhase::Tail => LifecycleAction::CheckCaptureNoTail,
                CapturePhase::Bind => LifecycleAction::BindCaptureEmpty,
                CapturePhase::Step => LifecycleAction::StepCaptureRead,
                CapturePhase::Type => LifecycleAction::CaptureColumnType,
                CapturePhase::Integer => LifecycleAction::CaptureInteger,
                CapturePhase::Text => LifecycleAction::CaptureText,
                CapturePhase::ValueReturn => LifecycleAction::AwaitCaptureString,
                CapturePhase::NeedRaw => LifecycleAction::RetainCaptureRaw,
                CapturePhase::Reset => LifecycleAction::ResetCaptureRead,
                CapturePhase::Drain => LifecycleAction::AwaitCaptureQueryReturn,
            })
        }
    }
    pub(in crate::database::global_schema_v1) struct OriginalCapturePrefixLoan<'a> {
        fields: OriginalOwnerFields<'a>, prefix: &'a mut super::super::super::FinancialCapturePrefixState,
    }
    struct OriginalCapturePort<'short, 'a, 'rules> {
        loan: &'short mut OriginalCapturePrefixLoan<'a>, query: CaptureQuery, _rules: &'rules SelectedOriginalNativeRules,
    }
    struct OriginalCaptureStringReturnPort<'bytes, 'short, 'a> {
        loan: &'short mut OriginalCapturePrefixLoan<'a>, text: &'bytes str,
    }
    struct OriginalCaptureDetailReturnPort<'short, 'a> { loan: &'short mut OriginalCapturePrefixLoan<'a> }
    impl OriginalOwnerFields<'_> {
        pub(in crate::database::global_schema_v1) fn begin_catalog_prefix(&mut self) -> bool {
            let n = &mut self.native;
            let r = &n.integrity_read;
            if self.work.terminal().is_some() || self.physical.primary.is_some()
                || !matches!(n.tx.phase, TxPhase::Active) || n.capture_prefix.stage != CaptureStage::Dormant
                || r.stage != IntegrityStage::Ready || r.returned != [IntegrityReturn::Ok; 2]
                || r.capture_returned != IntegrityReturn::Ok || r.failed || r.raw.is_some()
                || r.ignored.is_some() || r.need_cleanup.is_some() || r.partial_live || r.detail_started
                || r.consumed.iter().any(Option::is_none) || n.statements.iter().any(|s| s.live().is_some()) { return false; }
            n.capture_prefix.stage = CaptureStage::Reading;
            // Both A03 consumed ledgers and its owning Vec stay retained.
            // One existing slot is reused only after their scopes are complete.
            n.statements[0] = StmtSlot::Vacant;
            true
        }
    }
    impl<'a> OriginalOwnerFields<'a> {
        pub(in crate::database::global_schema_v1) fn capture_prefix(self, prefix: &'a mut super::super::super::FinancialCapturePrefixState) -> OriginalCapturePrefixLoan<'a> {
            OriginalCapturePrefixLoan { fields: self, prefix }
        }
    }
    impl<'a> OriginalCapturePrefixLoan<'a> {
        fn next(&self) -> Option<LifecycleAction> { self.fields.native.capture_prefix_action(&self.fields.work, self.fields.physical) }
        fn fixed_port<'short, 'rules>(&'short mut self, query: CaptureQuery, rules: &'rules SelectedOriginalNativeRules) -> Result<OriginalCapturePort<'short, 'a, 'rules>, ProtocolFault> {
            if self.fields.native.capture_prefix.query != query || self.fields.native.capture_prefix.stage != CaptureStage::Reading { return Err(ProtocolFault::UnexpectedObservation); }
            Ok(OriginalCapturePort { loan: self, query, _rules: rules })
        }
        fn application_id<'short, 'rules>(&'short mut self, rules: &'rules SelectedOriginalNativeRules) -> Result<OriginalCapturePort<'short, 'a, 'rules>, ProtocolFault> { self.fixed_port(CaptureQuery::ApplicationId, rules) }
        fn user_version<'short, 'rules>(&'short mut self, rules: &'rules SelectedOriginalNativeRules) -> Result<OriginalCapturePort<'short, 'a, 'rules>, ProtocolFault> { self.fixed_port(CaptureQuery::UserVersion, rules) }
        fn source_id<'short, 'rules>(&'short mut self, rules: &'rules SelectedOriginalNativeRules) -> Result<OriginalCapturePort<'short, 'a, 'rules>, ProtocolFault> { self.fixed_port(CaptureQuery::SourceId, rules) }
        fn begin_pragma_scope(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::BeginCapturePragmaScope) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.capture_prefix; let call = &mut r.calls[r.query.index()];
            if !r.query.pragma() || call.pragma_started { return Err(ProtocolFault::RepeatedObservation); }
            call.pragma_started = true; r.capture_started = true; r.phase = CapturePhase::Prepare; Ok(())
        }
        fn prepare_slot(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::PrepareCaptureRead) || self.fields.native.statements[0].live().is_some() { return Err(ProtocolFault::UnexpectedObservation); }
            self.fields.native.statements[0] = StmtSlot::Vacant; Ok(())
        }
        fn adverse(&mut self, code: i32) {
            if code == rusqlite::ffi::SQLITE_OK || code == rusqlite::ffi::SQLITE_ROW || code == rusqlite::ffi::SQLITE_DONE { return; }
            let query = self.fields.native.capture_prefix.query;
            let make = || FixedAdverse { role: Role::Original, action: query.action(), ordinal: query.index(), code };
            if self.fields.native.secondary.is_none() { self.fields.native.secondary = Some(make()); }
            if self.fields.release.first_secondary.is_none() { self.fields.release.first_secondary = Some(make()); }
        }
        fn observe_prepare(&mut self, code: i32) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::PrepareCaptureRead) { return Err(ProtocolFault::UnexpectedObservation); }
            let query = self.fields.native.capture_prefix.query;
            if code == rusqlite::ffi::SQLITE_OK {
                let state = self.fields.native.statements[0].live().ok_or(ProtocolFault::ResourceNotInstalled)?;
                if state.role != Role::Original || state.action != query.action() || state.cursor != CursorPhase::NoCursor { return Err(ProtocolFault::UnexpectedObservation); }
            } else if self.fields.native.statements[0].live().is_some() { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.capture_prefix; record_once(&mut r.calls[query.index()].prepare, code)?;
            r.capture_started = true;
            r.phase = if code == rusqlite::ffi::SQLITE_OK { CapturePhase::Tail } else { r.calls[query.index()].outcome = CaptureOutcome::Error; CapturePhase::NeedRaw };
            self.adverse(code); Ok(())
        }
        fn observe_tail(&mut self, fact: CaptureReturn) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::CheckCaptureNoTail) || !matches!(fact, CaptureReturn::Ok | CaptureReturn::Error) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.capture_prefix;
            r.phase = if fact == CaptureReturn::Ok { CapturePhase::Bind } else { r.calls[r.query.index()].outcome = CaptureOutcome::Error; CapturePhase::NeedRaw }; Ok(())
        }
        fn observe_bind(&mut self, fact: CaptureReturn) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::BindCaptureEmpty) || !matches!(fact, CaptureReturn::Ok | CaptureReturn::Error) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.capture_prefix;
            if fact == CaptureReturn::Ok { r.calls[r.query.index()].rows_started = true; r.phase = CapturePhase::Step;
                self.fields.native.statements[0].live_mut().ok_or(ProtocolFault::ResourceNotInstalled)?.cursor = CursorPhase::Active;
            } else { r.calls[r.query.index()].outcome = CaptureOutcome::Error; r.phase = CapturePhase::NeedRaw; } Ok(())
        }
        fn observe_step(&mut self, code: i32) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::StepCaptureRead) { return Err(ProtocolFault::UnexpectedObservation); }
            let state = self.fields.native.statements[0].live_mut().ok_or(ProtocolFault::ResourceNotInstalled)?;
            record_once(&mut state.step, code)?;
            let r = &mut self.fields.native.capture_prefix;
            if code == rusqlite::ffi::SQLITE_ROW { r.calls[r.query.index()].outcome = CaptureOutcome::Row; r.phase = CapturePhase::Type; }
            else if code == rusqlite::ffi::SQLITE_DONE { r.calls[r.query.index()].outcome = CaptureOutcome::NoRows; r.phase = CapturePhase::Reset; }
            else { r.calls[r.query.index()].outcome = CaptureOutcome::Error; r.phase = CapturePhase::NeedRaw; } self.adverse(code); Ok(())
        }
        fn observe_type(&mut self, kind: i32) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::CaptureColumnType) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.capture_prefix;
            let expected = if r.query == CaptureQuery::SourceId { rusqlite::ffi::SQLITE_TEXT } else { rusqlite::ffi::SQLITE_INTEGER };
            r.phase = if kind != expected { r.calls[r.query.index()].outcome = CaptureOutcome::Error; CapturePhase::NeedRaw }
                else if r.query == CaptureQuery::SourceId { CapturePhase::Text } else { CapturePhase::Integer }; Ok(())
        }
        fn observe_integer(&mut self, value: i64) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::CaptureInteger) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.capture_prefix;
            if r.query == CaptureQuery::SourceId || self.prefix.identity[r.query.index()].is_some() { return Err(ProtocolFault::RepeatedObservation); }
            self.prefix.identity[r.query.index()] = Some(value); r.calls[r.query.index()].outcome = CaptureOutcome::Value; r.phase = CapturePhase::Reset; Ok(())
        }
        fn text_return_port<'bytes, 'short>(&'short mut self, bytes: &'bytes [u8]) -> Result<OriginalCaptureStringReturnPort<'bytes, 'short, 'a>, ProtocolFault> {
            if self.next() != Some(LifecycleAction::CaptureText) { return Err(ProtocolFault::UnexpectedObservation); }
            let text = match std::str::from_utf8(bytes) { Ok(text) => text, Err(_) => {
                let r = &mut self.fields.native.capture_prefix; r.calls[r.query.index()].outcome = CaptureOutcome::Error; r.phase = CapturePhase::NeedRaw;
                return Err(ProtocolFault::UnexpectedObservation);
            } };
            let r = &mut self.fields.native.capture_prefix; r.value_started = true; r.phase = CapturePhase::ValueReturn;
            Ok(OriginalCaptureStringReturnPort { loan: self, text })
        }
        fn retain_raw(&mut self, error: rusqlite::Error) -> Result<(), rusqlite::Error> {
            if self.next() != Some(LifecycleAction::RetainCaptureRaw) || self.fields.native.capture_prefix.raw.is_some() { return Err(error); }
            self.fields.native.capture_prefix.raw = Some(error); self.fields.native.capture_prefix.phase = CapturePhase::Drain; Ok(())
        }
        fn observe_reset(&mut self, code: i32) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::ResetCaptureRead) { return Err(ProtocolFault::UnexpectedObservation); }
            let terminal = self.fields.work.terminal().is_some();
            let state = self.fields.native.statements[0].live_mut().ok_or(ProtocolFault::ResourceNotInstalled)?;
            record_once(&mut state.reset, code)?; state.cursor = CursorPhase::NoCursor;
            let r = &mut self.fields.native.capture_prefix; let i = r.query.index();
            if r.calls[i].outcome == CaptureOutcome::NoRows && r.raw.is_none() {
                // The reached DONE/reset callee result still needs owned custody
                // after T: actual reset Err or actual QueryReturnedNoRows.
                // This does not observe a whole return or construct that error.
                if code != rusqlite::ffi::SQLITE_OK { r.calls[i].outcome = CaptureOutcome::Error; }
                r.phase = CapturePhase::NeedRaw;
            } else { r.phase = CapturePhase::Drain;
                if !terminal && code != rusqlite::ffi::SQLITE_OK { r.need_cleanup = true; }
            } self.adverse(code); Ok(())
        }
        fn observe_finalize(&mut self, code: i32) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::FinalizeCaptureRead) { return Err(ProtocolFault::UnexpectedObservation); }
            let mut state = *self.fields.native.statements[0].live().ok_or(ProtocolFault::ResourceNotInstalled)?;
            if state.cursor != CursorPhase::NoCursor { return Err(ProtocolFault::UnexpectedObservation); }
            record_once(&mut state.finalize, code)?;
            self.fields.native.statements[0] = StmtSlot::Finalized(state);
            let r = &mut self.fields.native.capture_prefix; r.calls[r.query.index()].consumed = Some(state); r.phase = CapturePhase::Drain;
            if self.fields.work.terminal().is_none() && code != rusqlite::ffi::SQLITE_OK { r.need_cleanup = true; } self.adverse(code); Ok(())
        }
        fn retain_cleanup(&mut self, error: rusqlite::Error) -> Result<(), rusqlite::Error> {
            if self.next() != Some(LifecycleAction::RetainCaptureCleanup) || self.fields.native.capture_prefix.ignored.is_some() { return Err(error); }
            self.fields.native.capture_prefix.ignored = Some(error); Ok(())
        }
        fn discard_cleanup(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::DiscardCaptureCleanup) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.capture_prefix; drop(r.ignored.take()); r.need_cleanup = false;
            r.phase = CapturePhase::Drain; Ok(())
        }
        fn query_return(&mut self, fact: CaptureReturn) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitCaptureQueryReturn) { return Err(ProtocolFault::UnexpectedObservation); }
            let interrupted = self.fields.work.terminal().is_some() || self.fields.physical.primary.is_some();
            let r = &mut self.fields.native.capture_prefix; let call = &mut r.calls[r.query.index()];
            if call.query_return != CaptureReturn::Unknown { return Err(ProtocolFault::RepeatedObservation); }
            match fact {
                CaptureReturn::Ok if call.outcome == CaptureOutcome::Value && r.raw.is_none()
                    && (r.query != CaptureQuery::SourceId || (r.value_returned && r.value_matches && self.prefix.source_id.is_some())) => {},
                CaptureReturn::Error if matches!(call.outcome, CaptureOutcome::Error | CaptureOutcome::NoRows) && r.raw.is_some() => {},
                CaptureReturn::Interrupted if interrupted && !matches!(call.outcome, CaptureOutcome::Value | CaptureOutcome::Error | CaptureOutcome::NoRows) => {},
                _ => return Err(ProtocolFault::UnexpectedObservation),
            }
            call.query_return = fact; Ok(())
        }
        fn pragma_scope_end(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitCapturePragmaScopeEnd) { return Err(ProtocolFault::UnexpectedObservation); }
            // Independent real callee lexical return only, not scalar proof of
            // Sql.buf ownership, allocation, payment or a selectable adapter.
            let r = &mut self.fields.native.capture_prefix; r.calls[r.query.index()].pragma_scope_ended = true; Ok(())
        }
        fn pragma_return(&mut self, fact: CaptureReturn) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitCapturePragmaReturn) { return Err(ProtocolFault::UnexpectedObservation); }
            let interrupted = self.fields.work.terminal().is_some() || self.fields.physical.primary.is_some();
            let r = &mut self.fields.native.capture_prefix; let call = &mut r.calls[r.query.index()];
            if fact == CaptureReturn::Unknown || (fact != call.query_return
                && !(interrupted && matches!(call.prepare, CodeSlot::NotCalled) && fact == CaptureReturn::Interrupted)) { return Err(ProtocolFault::UnexpectedObservation); }
            call.pragma_return = fact; Ok(())
        }
        fn detail_return_port<'short>(&'short mut self) -> Result<OriginalCaptureDetailReturnPort<'short, 'a>, ProtocolFault> {
            if self.next() != Some(LifecycleAction::FormatCaptureDetail) || self.prefix.detail.is_some() { return Err(ProtocolFault::UnexpectedObservation); }
            self.fields.native.capture_prefix.detail_started = true;
            Ok(OriginalCaptureDetailReturnPort { loan: self })
        }
        fn build_catalog_error(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::BuildCaptureCatalogError) { return Err(ProtocolFault::UnexpectedObservation); }
            let detail = self.prefix.detail.take().ok_or(ProtocolFault::ResourceNotInstalled)?;
            let r = &mut self.fields.native.capture_prefix;
            self.prefix.catalog_error = Some(super::super::super::GlobalSchemaCatalogError::SqliteReferenceBuildFailure { stage: r.query.stage(), ddl_id: None, detail });
            // C's Display argument is consumed after formatting and before
            // its owned enum result returns. No raw child is cloned/wrapped early.
            drop(r.raw.take()); r.detail_started = false; r.catalog_live = true; Ok(())
        }
        fn runtime_error_return(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitCaptureRuntimeErrorReturn) { return Err(ProtocolFault::UnexpectedObservation); }
            self.fields.native.capture_prefix.runtime_error_return = CaptureReturn::Error; Ok(())
        }
        fn catalog_return(&mut self, fact: CaptureReturn) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitCaptureCatalogReturn) { return Err(ProtocolFault::UnexpectedObservation); }
            let interrupted = self.fields.work.terminal().is_some() || self.fields.physical.primary.is_some();
            let r = &mut self.fields.native.capture_prefix;
            if (r.catalog_live && fact != CaptureReturn::Error) || (!r.catalog_live && (!interrupted || fact != CaptureReturn::Interrupted)) { return Err(ProtocolFault::UnexpectedObservation); }
            r.catalog_return = fact; Ok(())
        }
        fn wrap_catalog_error(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::WrapCaptureCatalogError) { return Err(ProtocolFault::UnexpectedObservation); }
            let error = self.prefix.catalog_error.take().ok_or(ProtocolFault::ResourceNotInstalled)?;
            self.fields.physical.primary = Some(super::super::super::retain_capture_catalog_error(error));
            self.fields.physical.audit_phase = super::super::super::FinancialAuditPhase::Failed;
            self.fields.native.capture_prefix.catalog_live = false; Ok(())
        }
        fn discard_owned(&mut self) -> Result<(), ProtocolFault> {
            match self.fields.native.capture_prefix_action(&self.fields.work, self.fields.physical) {
                Some(LifecycleAction::DiscardCaptureRaw) => { drop(self.fields.native.capture_prefix.raw.take()); },
                Some(LifecycleAction::DiscardCaptureDetail) => { drop(self.prefix.detail.take()); self.fields.native.capture_prefix.detail_started = false; },
                Some(LifecycleAction::DiscardCaptureCatalogError) => { drop(self.prefix.catalog_error.take()); self.fields.native.capture_prefix.catalog_live = false; },
                Some(LifecycleAction::DiscardCaptureString) => { drop(self.prefix.source_id.take()); self.fields.native.capture_prefix.value_live = false; },
                _ => return Err(ProtocolFault::UnexpectedObservation),
            } Ok(())
        }
        fn advance(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AdvanceCaptureQuery) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.capture_prefix;
            if r.calls[r.query.index()].query_return != CaptureReturn::Ok { return Err(ProtocolFault::UnexpectedObservation); }
            match r.query {
                CaptureQuery::ApplicationId => r.query = CaptureQuery::UserVersion,
                CaptureQuery::UserVersion => r.query = CaptureQuery::SourceId,
                CaptureQuery::SourceId => { r.stage = CaptureStage::Ready; return Ok(()); },
            }
            r.phase = if r.query.pragma() { CapturePhase::Scope } else { CapturePhase::Prepare }; Ok(())
        }
        fn stop(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::StopCapturePrefix) || self.fields.native.statements[0].live().is_some() { return Err(ProtocolFault::UnexpectedObservation); }
            self.fields.native.capture_prefix.stage = CaptureStage::Stopped; Ok(())
        }
    }
    impl OriginalCaptureStringReturnPort<'_, '_, '_> {
        fn retain(self, text: String) {
            let matches = text.as_str() == self.text;
            // Exclusive preflight: actual acquired String moves into the same
            // owning frame before any later primary/terminal observation gate.
            self.loan.prefix.source_id = Some(text);
            let r = &mut self.loan.fields.native.capture_prefix;
            r.value_returned = true; r.value_live = true; r.value_matches = matches;
            r.calls[r.query.index()].outcome = CaptureOutcome::Value; r.phase = CapturePhase::Reset;
        }
    }
    impl OriginalCaptureDetailReturnPort<'_, '_> {
        fn retain(self, detail: String) {
            self.loan.prefix.detail = Some(detail); self.loan.fields.native.capture_prefix.detail_returned = true;
        }
    }
    impl<'a> OriginalCapturePort<'_, 'a, '_> {
        fn begin_pragma_scope(&mut self) -> Result<(), ProtocolFault> { self.loan.begin_pragma_scope() }
        fn prepare_slot(&mut self) -> Result<(), ProtocolFault> { self.loan.prepare_slot() }
        fn prepare(&mut self, code: i32) -> Result<(), ProtocolFault> { self.loan.observe_prepare(code) }
        fn no_tail(&mut self, fact: CaptureReturn) -> Result<(), ProtocolFault> { self.loan.observe_tail(fact) }
        fn empty_query(&mut self, fact: CaptureReturn) -> Result<(), ProtocolFault> { self.loan.observe_bind(fact) }
        fn step(&mut self, code: i32) -> Result<(), ProtocolFault> { self.loan.observe_step(code) }
        fn column_type(&mut self, kind: i32) -> Result<(), ProtocolFault> { self.loan.observe_type(kind) }
        fn integer(&mut self, value: i64) -> Result<(), ProtocolFault> { self.loan.observe_integer(value) }
        fn text<'bytes, 'short>(&'short mut self, bytes: &'bytes [u8]) -> Result<OriginalCaptureStringReturnPort<'bytes, 'short, 'a>, ProtocolFault> { self.loan.text_return_port(bytes) }
        fn owned_raw(&mut self, error: rusqlite::Error) -> Result<(), rusqlite::Error> { self.loan.retain_raw(error) }
        fn reset(&mut self, code: i32) -> Result<(), ProtocolFault> { self.loan.observe_reset(code) }
        fn finalize(&mut self, code: i32) -> Result<(), ProtocolFault> { self.loan.observe_finalize(code) }
        fn owned_cleanup(&mut self, error: rusqlite::Error) -> Result<(), rusqlite::Error> { self.loan.retain_cleanup(error) }
        fn query_return(&mut self, fact: CaptureReturn) -> Result<(), ProtocolFault> { self.loan.query_return(fact) }
        fn pragma_scope_end(&mut self) -> Result<(), ProtocolFault> { self.loan.pragma_scope_end() }
        fn pragma_return(&mut self, fact: CaptureReturn) -> Result<(), ProtocolFault> { self.loan.pragma_return(fact) }
        fn detail_return_port<'short>(&'short mut self) -> Result<OriginalCaptureDetailReturnPort<'short, 'a>, ProtocolFault> { self.loan.detail_return_port() }
        fn build_catalog_error(&mut self) -> Result<(), ProtocolFault> { self.loan.build_catalog_error() }
        fn runtime_error_return(&mut self) -> Result<(), ProtocolFault> { self.loan.runtime_error_return() }
        fn catalog_return(&mut self, fact: CaptureReturn) -> Result<(), ProtocolFault> { self.loan.catalog_return(fact) }
        fn wrap_catalog_error(&mut self) -> Result<(), ProtocolFault> { self.loan.wrap_catalog_error() }
    }

    #[cfg(test)]
    #[derive(Clone, Copy, Debug)]
    pub(in crate::database::global_schema_v1) enum CaptureErrorCase { Prepare, Tail, Bind, Step, NoRows, DoneReset, Type, Utf8 }
    #[cfg(test)]
    #[derive(Clone, Copy, Debug)]
    pub(in crate::database::global_schema_v1) enum CaptureTerminalCut { BeforeQuery, Prepared, Row, OwnedString, StringPending, WholePending, PragmaScopePending, OwnedRaw, CleanupOwed, DetailPending, CatalogPending }
    #[cfg(test)]
    fn capture_test_allocation(error: &rusqlite::Error) -> usize {
        match error {
            rusqlite::Error::SqliteFailure(_, Some(text)) | rusqlite::Error::InvalidColumnName(text)
                | rusqlite::Error::InvalidColumnType(_, text, _) => text.as_ptr() as usize,
            rusqlite::Error::FromSqlConversionFailure(_, _, child) => child.as_ref() as *const _ as *const () as usize,
            rusqlite::Error::QueryReturnedNoRows => 0,
            _ => panic!("fixed supplied capture driver child"),
        }
    }
    #[cfg(test)]
    impl OriginalCapturePrefixLoan<'_> {
        pub(in crate::database::global_schema_v1) fn test_code_barrier(&mut self) {
            let before = self.fields.work.test_code_observation();
            let expected = self.next().expect("fixed capture scope pending");
            assert_eq!(self.fields.native.transaction_action(&self.fields.work, self.fields.physical), Some(expected));
            assert_eq!(self.fields.native.integrity_read_action(&self.fields.work, self.fields.physical), Some(expected));
            assert!(!self.fields.native.transaction_release_ready(&self.fields.work, self.fields.physical));
            let mut acquire = self.fields.reborrow().original_acquisition();
            assert_eq!(acquire.constructor().next(), expected); assert_eq!(acquire.a00_epilogue().next(), expected);
            assert_eq!(acquire.constructor().observe(ConstructorObservation::Close(rusqlite::ffi::SQLITE_OK)), Err(ProtocolFault::UnexpectedObservation));
            assert!(matches!(acquire.settle(), AcquisitionSettlement::Held(_)));
            assert_eq!(self.fields.work.test_code_observation(), before);
        }
        fn test_code_prepare(&mut self) {
            if self.next() == Some(LifecycleAction::BeginCapturePragmaScope) {
                self.begin_pragma_scope().unwrap(); assert!(self.begin_pragma_scope().is_err());
            }
            self.prepare_slot().unwrap();
            assert_eq!(self.observe_prepare(rusqlite::ffi::SQLITE_OK), Err(ProtocolFault::ResourceNotInstalled));
            let action = self.fields.native.capture_prefix.query.action();
            self.fields.native.statements[0] = StmtSlot::ProtocolHeld(StmtState { action, ..protocol_stmt_state() });
            self.observe_prepare(rusqlite::ffi::SQLITE_OK).unwrap(); assert!(self.observe_prepare(rusqlite::ffi::SQLITE_OK).is_err());
        }
        fn test_code_row(&mut self) {
            self.test_code_prepare(); self.observe_tail(CaptureReturn::Ok).unwrap(); self.observe_bind(CaptureReturn::Ok).unwrap();
            self.observe_step(rusqlite::ffi::SQLITE_ROW).unwrap();
            assert!(self.observe_step(rusqlite::ffi::SQLITE_DONE).is_err());
            let kind = if self.fields.native.capture_prefix.query == CaptureQuery::SourceId { rusqlite::ffi::SQLITE_TEXT } else { rusqlite::ffi::SQLITE_INTEGER };
            self.observe_type(kind).unwrap();
        }
        fn test_code_ok_scope(&mut self) {
            assert!(self.query_return(CaptureReturn::Ok).is_err());
            self.observe_reset(rusqlite::ffi::SQLITE_OK).unwrap(); self.test_code_barrier();
            self.observe_finalize(rusqlite::ffi::SQLITE_OK).unwrap(); assert!(self.observe_finalize(rusqlite::ffi::SQLITE_OK).is_err());
            self.test_code_barrier(); assert!(self.query_return(CaptureReturn::Error).is_err());
            self.query_return(CaptureReturn::Ok).unwrap(); assert!(self.query_return(CaptureReturn::Ok).is_err());
            if self.fields.native.capture_prefix.query.pragma() {
                self.test_code_barrier(); assert!(self.pragma_return(CaptureReturn::Ok).is_err());
                self.pragma_scope_end().unwrap(); self.test_code_barrier();
                assert!(self.pragma_return(CaptureReturn::Error).is_err()); self.pragma_return(CaptureReturn::Ok).unwrap();
                assert!(self.pragma_return(CaptureReturn::Ok).is_err());
            }
            self.advance().unwrap(); assert!(self.advance().is_err());
        }
        pub(in crate::database::global_schema_v1) fn test_code_integer_query(&mut self, value: i64) {
            assert!(self.fields.native.capture_prefix.query.pragma());
            self.test_code_row(); self.observe_integer(value).unwrap(); assert!(self.observe_integer(value).is_err());
            self.test_code_ok_scope();
        }
        pub(in crate::database::global_schema_v1) fn test_code_success_with_ignored_children(&mut self, reset: rusqlite::Error, finalize: rusqlite::Error) {
            self.test_code_row(); self.observe_integer(i64::MIN).unwrap();
            self.observe_reset(rusqlite::ffi::SQLITE_ERROR).unwrap(); self.test_code_barrier();
            self.retain_cleanup(reset).unwrap_or_else(|_| panic!("successful mapper's actual ignored reset Err"));
            self.discard_cleanup().unwrap(); assert!(self.discard_cleanup().is_err());
            self.observe_finalize(rusqlite::ffi::SQLITE_ERROR).unwrap(); self.test_code_barrier();
            assert!(self.fields.native.statements[0].live().is_none());
            self.retain_cleanup(finalize).unwrap_or_else(|_| panic!("successful query's consumed VM finalize Err"));
            self.discard_cleanup().unwrap(); assert!(self.discard_cleanup().is_err());
            assert!(self.fields.physical.primary.is_none()); self.query_return(CaptureReturn::Ok).unwrap();
            self.pragma_scope_end().unwrap(); self.pragma_return(CaptureReturn::Ok).unwrap(); self.advance().unwrap();
        }
        pub(in crate::database::global_schema_v1) fn test_code_source_query(&mut self, value: String) {
            assert_eq!(self.fields.native.capture_prefix.query, CaptureQuery::SourceId);
            self.test_code_row(); let pointer = value.as_ptr(); let bytes = value.as_bytes().to_vec();
            let port = self.text_return_port(&bytes).unwrap(); port.retain(value);
            assert_eq!(self.prefix.source_id.as_ref().unwrap().as_ptr(), pointer);
            self.test_code_ok_scope(); assert_eq!(self.next(), Some(LifecycleAction::CapturePrefixReached));
            assert!(self.fields.native.capture_prefix.calls.iter().all(|call| call.consumed.is_some() && call.query_return == CaptureReturn::Ok));
            assert_eq!(self.fields.native.capture_prefix.catalog_return, CaptureReturn::Unknown);
        }
        pub(in crate::database::global_schema_v1) fn test_code_error(&mut self, case: CaptureErrorCase, raw: rusqlite::Error, reset: rusqlite::Error, finalize: rusqlite::Error) -> usize {
            let raw_pointer = capture_test_allocation(&raw);
            let query = self.fields.native.capture_prefix.query;
            if matches!(case, CaptureErrorCase::Prepare) {
                if query.pragma() { self.begin_pragma_scope().unwrap(); }
                self.prepare_slot().unwrap(); self.observe_prepare(rusqlite::ffi::SQLITE_ERROR).unwrap();
            } else {
                self.test_code_prepare();
                if matches!(case, CaptureErrorCase::Tail) { self.observe_tail(CaptureReturn::Error).unwrap(); }
                else {
                    self.observe_tail(CaptureReturn::Ok).unwrap();
                    if matches!(case, CaptureErrorCase::Bind) { self.observe_bind(CaptureReturn::Error).unwrap(); }
                    else {
                        self.observe_bind(CaptureReturn::Ok).unwrap();
                        match case {
                            CaptureErrorCase::Step => self.observe_step(rusqlite::ffi::SQLITE_ERROR).unwrap(),
                            CaptureErrorCase::NoRows | CaptureErrorCase::DoneReset => {
                                self.observe_step(rusqlite::ffi::SQLITE_DONE).unwrap(); assert!(self.retain_raw(rusqlite::Error::QueryReturnedNoRows).is_err());
                                self.observe_reset(if matches!(case, CaptureErrorCase::DoneReset) { rusqlite::ffi::SQLITE_ERROR } else { rusqlite::ffi::SQLITE_OK }).unwrap();
                            },
                            CaptureErrorCase::Type | CaptureErrorCase::Utf8 => {
                                self.observe_step(rusqlite::ffi::SQLITE_ROW).unwrap();
                                if matches!(case, CaptureErrorCase::Type) { self.observe_type(rusqlite::ffi::SQLITE_BLOB).unwrap(); }
                                else { self.observe_type(rusqlite::ffi::SQLITE_TEXT).unwrap(); assert!(self.text_return_port(&[0xff]).is_err()); }
                            },
                            _ => unreachable!(),
                        }
                    }
                }
            }
            self.retain_raw(raw).unwrap_or_else(|_| panic!("already owned driver error before Rows/Stmt scopes"));
            assert_eq!(capture_test_allocation(self.fields.native.capture_prefix.raw.as_ref().unwrap()), raw_pointer);
            assert!(self.build_catalog_error().is_err()); self.test_code_barrier();
            let mut reset = Some(reset); let mut finalize = Some(finalize);
            if self.next() == Some(LifecycleAction::ResetCaptureRead) {
                self.observe_reset(rusqlite::ffi::SQLITE_ERROR).unwrap(); self.test_code_barrier();
                let child = reset.take().unwrap(); let pointer = capture_test_allocation(&child);
                self.retain_cleanup(child).unwrap_or_else(|_| panic!("actual ignored reset child"));
                assert_eq!(capture_test_allocation(self.fields.native.capture_prefix.ignored.as_ref().unwrap()), pointer);
                self.test_code_barrier(); self.discard_cleanup().unwrap(); assert!(self.discard_cleanup().is_err());
            }
            if self.next() == Some(LifecycleAction::FinalizeCaptureRead) {
                self.observe_finalize(rusqlite::ffi::SQLITE_ERROR).unwrap(); self.test_code_barrier();
                assert!(self.fields.native.statements[0].live().is_none()); assert!(self.observe_finalize(rusqlite::ffi::SQLITE_OK).is_err());
                self.retain_cleanup(finalize.take().unwrap()).unwrap_or_else(|_| panic!("consumed VM's actual ignored finalize child"));
                self.discard_cleanup().unwrap(); assert!(self.discard_cleanup().is_err());
            }
            drop(reset); drop(finalize);
            assert_eq!(capture_test_allocation(self.fields.native.capture_prefix.raw.as_ref().unwrap()), raw_pointer);
            assert!(self.build_catalog_error().is_err()); self.test_code_barrier();
            assert!(self.query_return(CaptureReturn::Ok).is_err()); self.query_return(CaptureReturn::Error).unwrap();
            if query.pragma() { self.test_code_barrier(); assert!(self.detail_return_port().is_err());
                self.pragma_scope_end().unwrap(); self.test_code_barrier(); self.pragma_return(CaptureReturn::Error).unwrap(); }
            // Actual test Display return is an owned String; these supplied
            // test resources do not prove producer/formatter payment authority.
            let detail = self.fields.native.capture_prefix.raw.as_ref().unwrap().to_string(); let pointer = detail.as_ptr() as usize;
            self.detail_return_port().unwrap().retain(detail); self.test_code_barrier();
            self.build_catalog_error().unwrap(); assert!(self.fields.native.capture_prefix.raw.is_none());
            if query == CaptureQuery::SourceId { self.test_code_barrier(); assert!(self.catalog_return(CaptureReturn::Error).is_err()); self.runtime_error_return().unwrap(); }
            self.test_code_barrier(); assert!(self.wrap_catalog_error().is_err());
            self.catalog_return(CaptureReturn::Error).unwrap(); assert!(self.catalog_return(CaptureReturn::Error).is_err());
            self.wrap_catalog_error().unwrap(); assert!(self.wrap_catalog_error().is_err());
            self.test_code_barrier(); self.stop().unwrap(); assert!(self.stop().is_err()); pointer
        }
        pub(in crate::database::global_schema_v1) fn test_code_interruption_cut(&mut self, cut: CaptureTerminalCut, raw: rusqlite::Error, cleanup: rusqlite::Error) -> Option<String> {
            let mut raw = Some(raw); let mut cleanup = Some(cleanup); let mut pending = None;
            match cut {
                CaptureTerminalCut::BeforeQuery => {},
                CaptureTerminalCut::Prepared => self.test_code_prepare(),
                CaptureTerminalCut::Row => self.test_code_row(),
                CaptureTerminalCut::OwnedString | CaptureTerminalCut::StringPending => {
                    self.test_code_row(); assert_eq!(self.fields.native.capture_prefix.query, CaptureQuery::SourceId);
                    let text = String::from("TEST_CODE source-id");
                    if matches!(cut, CaptureTerminalCut::OwnedString) { self.text_return_port(b"TEST_CODE source-id").unwrap().retain(text); }
                    else { { let _short = self.text_return_port(b"TEST_CODE source-id").unwrap(); } pending = Some(text); }
                },
                CaptureTerminalCut::WholePending | CaptureTerminalCut::PragmaScopePending => {
                    self.test_code_row();
                    if self.fields.native.capture_prefix.query == CaptureQuery::SourceId { self.text_return_port(b"TEST_CODE source-id").unwrap().retain(String::from("TEST_CODE source-id")); }
                    else { self.observe_integer(i64::MIN).unwrap(); }
                    self.observe_reset(rusqlite::ffi::SQLITE_OK).unwrap(); self.observe_finalize(rusqlite::ffi::SQLITE_OK).unwrap();
                    if matches!(cut, CaptureTerminalCut::PragmaScopePending) { assert!(self.fields.native.capture_prefix.query.pragma()); self.query_return(CaptureReturn::Ok).unwrap(); }
                },
                CaptureTerminalCut::OwnedRaw | CaptureTerminalCut::CleanupOwed | CaptureTerminalCut::DetailPending | CaptureTerminalCut::CatalogPending => {
                    self.test_code_prepare(); self.observe_tail(CaptureReturn::Error).unwrap();
                    self.retain_raw(raw.take().unwrap()).unwrap_or_else(|_| panic!("same already-owned raw cut"));
                    if !matches!(cut, CaptureTerminalCut::OwnedRaw) {
                        self.observe_finalize(if matches!(cut, CaptureTerminalCut::CleanupOwed) { rusqlite::ffi::SQLITE_ERROR } else { rusqlite::ffi::SQLITE_OK }).unwrap();
                        if matches!(cut, CaptureTerminalCut::CleanupOwed) { self.retain_cleanup(cleanup.take().unwrap()).unwrap_or_else(|_| panic!("pre-existing cleanup owner")); }
                        else {
                            self.query_return(CaptureReturn::Error).unwrap();
                            if self.fields.native.capture_prefix.query.pragma() { self.pragma_scope_end().unwrap(); self.pragma_return(CaptureReturn::Error).unwrap(); }
                            let detail = self.fields.native.capture_prefix.raw.as_ref().unwrap().to_string();
                            if matches!(cut, CaptureTerminalCut::DetailPending) { { let _port = self.detail_return_port().unwrap(); } pending = Some(detail); }
                            else { self.detail_return_port().unwrap().retain(detail); self.build_catalog_error().unwrap(); }
                        }
                    }
                },
            }
            drop(raw); drop(cleanup); self.test_code_barrier(); pending
        }
        pub(in crate::database::global_schema_v1) fn test_code_reached_done(&mut self) {
            self.test_code_prepare(); self.observe_tail(CaptureReturn::Ok).unwrap(); self.observe_bind(CaptureReturn::Ok).unwrap();
            self.observe_step(rusqlite::ffi::SQLITE_DONE).unwrap(); self.test_code_barrier();
        }
        pub(in crate::database::global_schema_v1) fn test_code_done_result_after_terminal(&mut self, case: CaptureErrorCase, raw: rusqlite::Error, duplicate: rusqlite::Error) {
            assert!(self.fields.work.terminal().is_some());
            let before = self.fields.work.test_code_observation();
            let code = match case { CaptureErrorCase::NoRows => rusqlite::ffi::SQLITE_OK,
                CaptureErrorCase::DoneReset => rusqlite::ffi::SQLITE_ERROR, _ => unreachable!() };
            let pointer = capture_test_allocation(&raw); let duplicate_pointer = capture_test_allocation(&duplicate);
            self.observe_reset(code).unwrap(); assert!(self.observe_reset(code).is_err());
            let r = &self.fields.native.capture_prefix; let call = r.calls[r.query.index()];
            assert_eq!(call.outcome, if code == rusqlite::ffi::SQLITE_OK { CaptureOutcome::NoRows } else { CaptureOutcome::Error });
            assert_eq!(call.query_return, CaptureReturn::Unknown); assert!(!r.need_cleanup && r.raw.is_none());
            assert_eq!(self.next(), Some(LifecycleAction::RetainCaptureRaw)); self.test_code_barrier();
            assert!(self.observe_finalize(rusqlite::ffi::SQLITE_OK).is_err());
            assert!(self.query_return(CaptureReturn::Error).is_err()); assert!(self.query_return(CaptureReturn::Interrupted).is_err());
            assert!(self.stop().is_err()); assert!(self.pragma_scope_end().is_err()); assert!(self.catalog_return(CaptureReturn::Interrupted).is_err());
            self.retain_raw(raw).unwrap_or_else(|_| panic!("already returned DONE/reset child retained after T"));
            assert_eq!(capture_test_allocation(self.fields.native.capture_prefix.raw.as_ref().unwrap()), pointer);
            let rejected = self.retain_raw(duplicate).unwrap_err(); assert_eq!(capture_test_allocation(&rejected), duplicate_pointer); drop(rejected);
            self.test_code_barrier(); self.observe_finalize(rusqlite::ffi::SQLITE_OK).unwrap();
            assert!(self.observe_finalize(rusqlite::ffi::SQLITE_OK).is_err()); self.test_code_barrier();
            assert_eq!(self.next(), Some(LifecycleAction::AwaitCaptureQueryReturn));
            assert!(self.query_return(CaptureReturn::Unknown).is_err()); assert!(self.query_return(CaptureReturn::Ok).is_err());
            assert!(self.query_return(CaptureReturn::Interrupted).is_err()); assert!(self.stop().is_err());
            assert_eq!(self.fields.native.capture_prefix.calls[self.fields.native.capture_prefix.query.index()].query_return, CaptureReturn::Unknown);
            self.query_return(CaptureReturn::Error).unwrap(); assert!(self.query_return(CaptureReturn::Error).is_err());
            if self.fields.native.capture_prefix.query.pragma() {
                self.test_code_barrier(); assert!(self.pragma_return(CaptureReturn::Error).is_err()); self.pragma_scope_end().unwrap();
                self.test_code_barrier(); assert!(self.pragma_return(CaptureReturn::Unknown).is_err());
                assert!(self.pragma_return(CaptureReturn::Interrupted).is_err()); assert!(self.pragma_return(CaptureReturn::Ok).is_err());
                assert_eq!(self.fields.native.capture_prefix.calls[self.fields.native.capture_prefix.query.index()].pragma_return, CaptureReturn::Unknown);
                self.pragma_return(CaptureReturn::Error).unwrap(); assert!(self.pragma_return(CaptureReturn::Error).is_err());
            }
            assert_eq!(capture_test_allocation(self.fields.native.capture_prefix.raw.as_ref().unwrap()), pointer);
            assert!(self.detail_return_port().is_err()); self.discard_owned().unwrap(); assert!(self.discard_owned().is_err());
            assert_eq!(self.next(), Some(LifecycleAction::AwaitCaptureCatalogReturn)); self.test_code_barrier();
            assert!(self.catalog_return(CaptureReturn::Unknown).is_err()); assert!(self.catalog_return(CaptureReturn::Error).is_err());
            assert_eq!(self.fields.native.capture_prefix.catalog_return, CaptureReturn::Unknown); assert!(self.stop().is_err());
            assert_eq!(self.fields.work.test_code_observation(), before);
        }
        pub(in crate::database::global_schema_v1) fn test_code_drain_interrupted(&mut self, pending: Option<String>, reset_child: rusqlite::Error) {
            assert!(self.fields.work.terminal().is_some() || self.fields.physical.primary.is_some());
            let before = self.fields.work.test_code_observation();
            assert!(self.observe_prepare(rusqlite::ffi::SQLITE_OK).is_err()); assert!(self.observe_bind(CaptureReturn::Ok).is_err());
            assert!(self.observe_step(rusqlite::ffi::SQLITE_ROW).is_err()); assert!(self.detail_return_port().is_err());
            let pending_pointer = pending.as_ref().map(|s| s.as_ptr());
            let r = &self.fields.native.capture_prefix;
            if r.value_started && !r.value_returned {
                self.test_code_barrier(); assert!(self.query_return(CaptureReturn::Ok).is_err());
                OriginalCaptureStringReturnPort { loan: self, text: "TEST_CODE source-id" }.retain(pending.unwrap());
                assert_eq!(self.prefix.source_id.as_ref().map(|s| s.as_ptr()), pending_pointer);
            } else if r.detail_started && !r.detail_returned {
                self.test_code_barrier(); assert!(self.stop().is_err()); OriginalCaptureDetailReturnPort { loan: self }.retain(pending.unwrap());
                assert_eq!(self.prefix.detail.as_ref().map(|s| s.as_ptr()), pending_pointer);
            } else { assert!(pending.is_none()); }
            let mut reset_child = Some(reset_child);
            // Bounded fixed cleanup script. It supplies independent return
            // facts only after asserting their separate pending boundaries.
            for _ in 0..16 {
                self.test_code_barrier();
                match self.next().unwrap() {
                    LifecycleAction::DiscardCaptureCleanup => self.discard_cleanup().unwrap(),
                    LifecycleAction::RetainCaptureCleanup => self.retain_cleanup(reset_child.take().unwrap()).unwrap_or_else(|_| panic!("normal primary's actual ignored child")),
                    LifecycleAction::ResetCaptureRead => self.observe_reset(rusqlite::ffi::SQLITE_ERROR).unwrap(),
                    LifecycleAction::FinalizeCaptureRead => self.observe_finalize(rusqlite::ffi::SQLITE_OK).unwrap(),
                    LifecycleAction::AwaitCaptureQueryReturn => {
                        assert!(self.stop().is_err());
                        let r = &self.fields.native.capture_prefix; let call = r.calls[r.query.index()];
                        let fact = if r.raw.is_some() { CaptureReturn::Error } else if call.outcome == CaptureOutcome::Value { CaptureReturn::Ok } else { CaptureReturn::Interrupted };
                        let contrary = if fact == CaptureReturn::Ok { CaptureReturn::Error } else { CaptureReturn::Ok };
                        assert!(self.query_return(contrary).is_err()); self.query_return(fact).unwrap();
                    },
                    LifecycleAction::AwaitCapturePragmaScopeEnd => { assert!(self.stop().is_err()); self.pragma_scope_end().unwrap(); },
                    LifecycleAction::AwaitCapturePragmaReturn => {
                        let call = self.fields.native.capture_prefix.calls[self.fields.native.capture_prefix.query.index()];
                        self.pragma_return(if call.query_return == CaptureReturn::Unknown { CaptureReturn::Interrupted } else { call.query_return }).unwrap();
                    },
                    LifecycleAction::AwaitCaptureRuntimeErrorReturn => { assert!(self.stop().is_err()); self.runtime_error_return().unwrap(); },
                    LifecycleAction::AwaitCaptureCatalogReturn => {
                        assert!(self.stop().is_err()); self.catalog_return(if self.fields.native.capture_prefix.catalog_live { CaptureReturn::Error } else { CaptureReturn::Interrupted }).unwrap();
                    },
                    LifecycleAction::DiscardCaptureRaw | LifecycleAction::DiscardCaptureDetail | LifecycleAction::DiscardCaptureCatalogError | LifecycleAction::DiscardCaptureString => self.discard_owned().unwrap(),
                    LifecycleAction::StopCapturePrefix => { self.stop().unwrap(); break; },
                    action => panic!("new read work after interruption: {action:?}"),
                }
            }
            drop(reset_child);
            assert!(self.fields.native.capture_prefix.stopped_clear());
            assert!(self.prefix.source_id.is_none() && self.prefix.detail.is_none() && self.prefix.catalog_error.is_none());
            assert_eq!(self.fields.work.test_code_observation(), before); assert!(self.discard_owned().is_err());
        }
    }

    // Fixed compile_options callee protocol, before sort. These records do not
    // own the private MappedRows/partial Vec/mapper String or ignored Drop
    // Results. A real controlled adapter must end those lexical obligations;
    // the uninhabited Rules port cannot issue their allocation/payment facts.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum CompileStage { Dormant, Running, Ready, Stopped }
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum CompilePhase { Prepare, Query, Step, Type, Mapper, NeedRaw, Reset, Exit }
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum CompileReturn { Unknown, Ok, Error, Interrupted }
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum CompileOutcome { Unknown, Eof, Error }
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum CompileErrorStage { Prepare, Query, Read }
    impl CompileErrorStage {
        fn label(self) -> &'static str { match self {
            Self::Prepare => "prepare-compile-options", Self::Query => "query-compile-options",
            Self::Read => "read-compile-options",
        } }
    }
    struct CompileOptionsRecord {
        stage: CompileStage, phase: CompilePhase, error_stage: CompileErrorStage,
        prepare_started: bool, prepare: CodeSlot, prepare_return: CompileReturn,
        query_started: bool, query_observed: bool, query_return: CompileReturn, step_pending: bool,
        collect_started: bool, rows_live: bool, callee_scope_ended: bool,
        mapper_pending: bool, outcome: CompileOutcome, collect_return: CompileReturn,
        raw: Option<rusqlite::Error>, vector_pending: bool, vector_live: bool,
        detail_started: bool, detail_returned: bool, catalog_live: bool,
        statement_drop_owed: bool, statement_drop_ended: bool, consumed: Option<StmtState>,
        runtime_return: CompileReturn, catalog_return: CompileReturn,
    }
    impl CompileOptionsRecord {
        fn empty() -> Self { Self {
            stage: CompileStage::Dormant, phase: CompilePhase::Prepare, error_stage: CompileErrorStage::Prepare,
            prepare_started: false, prepare: CodeSlot::NotCalled, prepare_return: CompileReturn::Unknown,
            query_started: false, query_observed: false, query_return: CompileReturn::Unknown, step_pending: false,
            collect_started: false, rows_live: false, callee_scope_ended: false,
            mapper_pending: false, outcome: CompileOutcome::Unknown, collect_return: CompileReturn::Unknown,
            raw: None, vector_pending: false, vector_live: false, detail_started: false,
            detail_returned: false, catalog_live: false, statement_drop_owed: false,
            statement_drop_ended: false, consumed: None,
            runtime_return: CompileReturn::Unknown, catalog_return: CompileReturn::Unknown,
        } }
        fn blocks_early_primary(&self) -> bool {
            self.raw.is_some() || self.mapper_pending || self.vector_pending || self.detail_started
                || (self.prepare_started && self.prepare_return == CompileReturn::Unknown)
                || (self.query_started && self.query_return == CompileReturn::Unknown) || self.step_pending
                || self.catalog_live || self.statement_drop_owed || self.phase == CompilePhase::NeedRaw
                || (self.collect_started && self.collect_return == CompileReturn::Unknown)
        }
        fn stopped_clear(&self) -> bool {
            matches!(self.stage, CompileStage::Dormant | CompileStage::Stopped)
                && !self.blocks_early_primary() && !self.vector_live && !self.rows_live
        }
    }
    impl NativeOriginalOwner {
        fn compile_options_action(&self, work: &OriginalSourceWork<'_>, physical: &super::super::super::FinancialPhysical) -> Option<LifecycleAction> {
            if let Some(action) = self.compile_sort_action(work, physical) { return Some(action); }
            let r = &self.compile_options;
            if matches!(r.stage, CompileStage::Dormant | CompileStage::Stopped) { return None; }
            if r.prepare_started && matches!(r.prepare, CodeSlot::NotCalled) { return Some(LifecycleAction::AwaitCompilePrepareObservation); }
            if r.query_started && !r.query_observed { return Some(LifecycleAction::AwaitCompileQueryObservation); }
            if r.step_pending { return Some(LifecycleAction::AwaitCompileStepObservation); }
            if r.phase == CompilePhase::NeedRaw { return Some(LifecycleAction::RetainCompileRaw); }
            if r.mapper_pending { return Some(LifecycleAction::AwaitCompileMapperReturn); }
            if r.vector_pending { return Some(LifecycleAction::AwaitCompileVector); }
            if r.detail_started && !r.detail_returned { return Some(LifecycleAction::AwaitCompileDetail); }
            if !matches!(r.prepare, CodeSlot::NotCalled) && r.prepare_return == CompileReturn::Unknown {
                return Some(LifecycleAction::AwaitCompilePrepareReturn);
            }
            if r.query_started && r.query_return == CompileReturn::Unknown { return Some(LifecycleAction::AwaitCompileQueryReturn); }
            let interrupted = work.terminal().is_some() || physical.primary.is_some();
            let ending = interrupted || r.outcome != CompileOutcome::Unknown;
            if r.collect_started && r.collect_return == CompileReturn::Unknown {
                if !ending { return Some(match r.phase {
                    CompilePhase::Step => LifecycleAction::StepCompileRead,
                    CompilePhase::Type => LifecycleAction::CompileColumnType,
                    CompilePhase::Mapper => LifecycleAction::CompileText,
                    _ => LifecycleAction::AwaitCompileCollectReturn,
                }); }
                if r.rows_live {
                    if self.statements[0].live().is_some_and(|s| s.cursor != CursorPhase::NoCursor) {
                        return Some(LifecycleAction::ResetCompileRows);
                    }
                    return Some(LifecycleAction::AwaitCompileRowsDrop);
                }
                if !r.callee_scope_ended { return Some(LifecycleAction::AwaitCompileCalleeScope); }
                return Some(LifecycleAction::AwaitCompileCollectReturn);
            }
            if r.detail_started && r.detail_returned { return Some(if interrupted { LifecycleAction::DiscardCompileDetail } else { LifecycleAction::BuildCompileCatalogError }); }
            if r.raw.is_some() { return Some(if interrupted { LifecycleAction::DiscardCompileRaw } else { LifecycleAction::FormatCompileDetail }); }
            if r.vector_live && interrupted { return Some(LifecycleAction::DiscardCompileVector); }
            if r.statement_drop_owed { return Some(LifecycleAction::AwaitCompileStatementDrop); }
            if interrupted || r.catalog_live {
                if self.statements[0].live().is_some() { return Some(LifecycleAction::FinalizeCompileStatement); }
                if !r.statement_drop_ended && matches!(r.prepare, CodeSlot::Called(rusqlite::ffi::SQLITE_OK)) {
                    return Some(LifecycleAction::AwaitCompileStatementDrop);
                }
                if self.capture_prefix.value_live { return Some(LifecycleAction::DiscardCompileSourceId); }
                if r.runtime_return == CompileReturn::Unknown { return Some(LifecycleAction::AwaitCompileRuntimeReturn); }
                if r.catalog_return == CompileReturn::Unknown { return Some(LifecycleAction::AwaitCompileCatalogReturn); }
                if r.catalog_live { return Some(if interrupted { LifecycleAction::DiscardCompileCatalogError } else { LifecycleAction::WrapCompileCatalogError }); }
                return Some(LifecycleAction::StopCompileOptions);
            }
            if r.stage == CompileStage::Ready { return Some(LifecycleAction::CompileOptionsBeforeSort); }
            Some(match r.phase {
                CompilePhase::Prepare => LifecycleAction::PrepareCompileStatement,
                CompilePhase::Query => LifecycleAction::QueryCompileEmpty,
                _ => LifecycleAction::AwaitCompileCollectReturn,
            })
        }
    }
    pub(in crate::database::global_schema_v1) struct OriginalCompileOptionsLoan<'a> {
        fields: OriginalOwnerFields<'a>, options: &'a mut super::super::super::FinancialCompileOptionsState,
        source_id: &'a mut Option<String>,
    }
    struct OriginalCompileOptionsPort<'short, 'a, 'rules> {
        loan: &'short mut OriginalCompileOptionsLoan<'a>, _rules: &'rules SelectedOriginalNativeRules,
    }
    struct OriginalCompileVectorReturnPort<'short, 'a> { loan: &'short mut OriginalCompileOptionsLoan<'a> }
    struct OriginalCompileDetailReturnPort<'short, 'a> { loan: &'short mut OriginalCompileOptionsLoan<'a> }
    impl OriginalOwnerFields<'_> {
        pub(in crate::database::global_schema_v1) fn begin_compile_options(&mut self) -> bool {
            let n = &mut self.native; let p = &n.capture_prefix;
            if self.work.terminal().is_some() || self.physical.primary.is_some() || !matches!(n.tx.phase, TxPhase::Active)
                || n.compile_options.stage != CompileStage::Dormant || p.stage != CaptureStage::Ready
                || p.calls.iter().any(|c| c.query_return != CaptureReturn::Ok || c.consumed.is_none())
                || p.calls[..2].iter().any(|c| c.pragma_return != CaptureReturn::Ok || !c.pragma_scope_ended)
                || !p.value_live || !p.value_matches || p.blocks_early_primary()
                || n.statements.iter().any(|s| s.live().is_some()) { return false; }
            // Parent ledgers stay in p. Only its completed vacant VM place is
            // reused; no second Work take, request, meter or limit is created.
            n.statements[0] = StmtSlot::Vacant; n.compile_options.stage = CompileStage::Running; true
        }
    }
    impl<'a> OriginalOwnerFields<'a> {
        pub(in crate::database::global_schema_v1) fn compile_options(self, options: &'a mut super::super::super::FinancialCompileOptionsState,
            source_id: &'a mut Option<String>) -> OriginalCompileOptionsLoan<'a> {
            OriginalCompileOptionsLoan { fields: self, options, source_id }
        }
    }
    impl<'a> OriginalCompileOptionsLoan<'a> {
        fn next(&self) -> Option<LifecycleAction> { self.fields.native.compile_options_action(&self.fields.work, self.fields.physical) }
        fn fixed_port<'short, 'rules>(&'short mut self, rules: &'rules SelectedOriginalNativeRules) -> OriginalCompileOptionsPort<'short, 'a, 'rules> {
            OriginalCompileOptionsPort { loan: self, _rules: rules }
        }
        fn begin_prepare(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::PrepareCompileStatement) { return Err(ProtocolFault::UnexpectedObservation); }
            self.fields.native.compile_options.prepare_started = true; Ok(())
        }
        fn adverse(&mut self, code: i32) {
            if code == rusqlite::ffi::SQLITE_OK || code == rusqlite::ffi::SQLITE_ROW || code == rusqlite::ffi::SQLITE_DONE { return; }
            let make = || FixedAdverse { role: Role::Original, action: FixedAction::CompileOptions, ordinal: 0, code };
            if self.fields.native.secondary.is_none() { self.fields.native.secondary = Some(make()); }
            if self.fields.release.first_secondary.is_none() { self.fields.release.first_secondary = Some(make()); }
        }
        fn observe_prepare(&mut self, code: i32) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitCompilePrepareObservation) { return Err(ProtocolFault::UnexpectedObservation); }
            if code == rusqlite::ffi::SQLITE_OK {
                let s = self.fields.native.statements[0].live().ok_or(ProtocolFault::ResourceNotInstalled)?;
                if s.role != Role::Original || s.action != FixedAction::CompileOptions || s.cursor != CursorPhase::NoCursor { return Err(ProtocolFault::UnexpectedObservation); }
            } else if self.fields.native.statements[0].live().is_some() { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.compile_options; record_once(&mut r.prepare, code)?;
            if code != rusqlite::ffi::SQLITE_OK { r.outcome = CompileOutcome::Error; r.phase = CompilePhase::NeedRaw; }
            self.adverse(code); Ok(())
        }
        fn prepare_return(&mut self, fact: CompileReturn) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitCompilePrepareReturn) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.compile_options;
            let expected = if matches!(r.prepare, CodeSlot::Called(rusqlite::ffi::SQLITE_OK)) { CompileReturn::Ok } else { CompileReturn::Error };
            if fact != expected { return Err(ProtocolFault::UnexpectedObservation); }
            r.prepare_return = fact; r.phase = if fact == CompileReturn::Ok { CompilePhase::Query } else { CompilePhase::Exit }; Ok(())
        }
        fn begin_query(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::QueryCompileEmpty) { return Err(ProtocolFault::UnexpectedObservation); }
            self.fields.native.compile_options.query_started = true; Ok(())
        }
        fn observe_query(&mut self, fact: CompileReturn) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitCompileQueryObservation) || !matches!(fact, CompileReturn::Ok | CompileReturn::Error) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.compile_options; r.query_observed = true;
            if fact == CompileReturn::Ok {
                r.collect_started = true; r.rows_live = true; r.phase = CompilePhase::Step;
                self.fields.native.statements[0].live_mut().ok_or(ProtocolFault::ResourceNotInstalled)?.cursor = CursorPhase::Active;
            } else { r.error_stage = CompileErrorStage::Query; r.outcome = CompileOutcome::Error; r.phase = CompilePhase::NeedRaw; }
            Ok(())
        }
        fn query_return(&mut self, fact: CompileReturn) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitCompileQueryReturn) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.compile_options;
            if fact != if r.collect_started { CompileReturn::Ok } else { CompileReturn::Error } { return Err(ProtocolFault::UnexpectedObservation); }
            r.query_return = fact; Ok(())
        }
        fn begin_step(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::StepCompileRead) { return Err(ProtocolFault::UnexpectedObservation); }
            self.fields.native.compile_options.step_pending = true; Ok(())
        }
        fn observe_step(&mut self, code: i32) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitCompileStepObservation) { return Err(ProtocolFault::UnexpectedObservation); }
            let s = self.fields.native.statements[0].live_mut().ok_or(ProtocolFault::ResourceNotInstalled)?;
            s.step = CodeSlot::Called(code);
            let r = &mut self.fields.native.compile_options; r.step_pending = false; r.error_stage = CompileErrorStage::Read;
            if code == rusqlite::ffi::SQLITE_ROW { r.phase = CompilePhase::Type; }
            else if code == rusqlite::ffi::SQLITE_DONE { r.outcome = CompileOutcome::Eof; r.phase = CompilePhase::Reset; }
            else { r.outcome = CompileOutcome::Error; r.phase = CompilePhase::NeedRaw; }
            self.adverse(code); Ok(())
        }
        fn observe_type(&mut self, kind: i32) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::CompileColumnType) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.compile_options;
            if kind == rusqlite::ffi::SQLITE_TEXT { r.phase = CompilePhase::Mapper; }
            else { r.outcome = CompileOutcome::Error; r.phase = CompilePhase::NeedRaw; } Ok(())
        }
        fn text(&mut self, bytes: &[u8]) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::CompileText) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.compile_options;
            if std::str::from_utf8(bytes).is_err() { r.outcome = CompileOutcome::Error; r.phase = CompilePhase::NeedRaw; return Err(ProtocolFault::UnexpectedObservation); }
            // This is only a borrowed type/UTF8 preflight, not the actual
            // allocated mapper String or its insertion in the private Vec.
            r.mapper_pending = true; Ok(())
        }
        fn mapper_return(&mut self, fact: CompileReturn) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitCompileMapperReturn) || fact != CompileReturn::Ok { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.compile_options; r.mapper_pending = false; r.phase = CompilePhase::Step; Ok(())
        }
        fn retain_raw(&mut self, raw: rusqlite::Error) -> Result<(), rusqlite::Error> {
            if self.next() != Some(LifecycleAction::RetainCompileRaw) || self.fields.native.compile_options.raw.is_some() { return Err(raw); }
            let r = &mut self.fields.native.compile_options; r.raw = Some(raw); r.phase = CompilePhase::Exit; Ok(())
        }
        fn observe_reset(&mut self, code: i32) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::ResetCompileRows) { return Err(ProtocolFault::UnexpectedObservation); }
            let s = self.fields.native.statements[0].live_mut().ok_or(ProtocolFault::ResourceNotInstalled)?;
            record_once(&mut s.reset, code)?; s.cursor = CursorPhase::NoCursor;
            let r = &mut self.fields.native.compile_options;
            if r.outcome == CompileOutcome::Eof && code != rusqlite::ffi::SQLITE_OK {
                // DONE's nonignored reset Err is an actual reached raw result,
                // including after T. Never infer it from collect-return labels.
                r.outcome = CompileOutcome::Error; r.phase = CompilePhase::NeedRaw;
            }
            self.adverse(code); Ok(())
        }
        fn rows_drop_return(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitCompileRowsDrop) { return Err(ProtocolFault::UnexpectedObservation); }
            // Independently ended real Rows lexical scope; its internally
            // ignored reset Result has no G transport/payment issuer here.
            self.fields.native.compile_options.rows_live = false; Ok(())
        }
        fn callee_scope_end(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitCompileCalleeScope) { return Err(ProtocolFault::UnexpectedObservation); }
            self.fields.native.compile_options.callee_scope_ended = true; Ok(())
        }
        fn vector_return_port<'short>(&'short mut self) -> Result<OriginalCompileVectorReturnPort<'short, 'a>, ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitCompileCollectReturn)
                || self.fields.native.compile_options.outcome != CompileOutcome::Eof || self.options.rows.is_some() { return Err(ProtocolFault::UnexpectedObservation); }
            self.fields.native.compile_options.vector_pending = true; Ok(OriginalCompileVectorReturnPort { loan: self })
        }
        fn collect_return(&mut self, fact: CompileReturn) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitCompileCollectReturn) { return Err(ProtocolFault::UnexpectedObservation); }
            let interrupted = self.fields.work.terminal().is_some() || self.fields.physical.primary.is_some();
            let r = &mut self.fields.native.compile_options;
            if !((fact == CompileReturn::Error && r.outcome == CompileOutcome::Error && r.raw.is_some())
                || (fact == CompileReturn::Interrupted && interrupted && r.outcome == CompileOutcome::Unknown)) { return Err(ProtocolFault::UnexpectedObservation); }
            r.collect_return = fact; r.phase = CompilePhase::Exit; Ok(())
        }
        fn detail_return_port<'short>(&'short mut self) -> Result<OriginalCompileDetailReturnPort<'short, 'a>, ProtocolFault> {
            if self.next() != Some(LifecycleAction::FormatCompileDetail) || self.options.detail.is_some() { return Err(ProtocolFault::UnexpectedObservation); }
            self.fields.native.compile_options.detail_started = true; Ok(OriginalCompileDetailReturnPort { loan: self })
        }
        fn build_catalog_error(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::BuildCompileCatalogError) { return Err(ProtocolFault::UnexpectedObservation); }
            let detail = self.options.detail.take().ok_or(ProtocolFault::ResourceNotInstalled)?;
            let r = &mut self.fields.native.compile_options;
            self.options.catalog_error = Some(super::super::super::GlobalSchemaCatalogError::SqliteReferenceBuildFailure {
                stage: r.error_stage.label(), ddl_id: None, detail,
            });
            drop(r.raw.take()); r.detail_started = false; r.catalog_live = true; Ok(())
        }
        fn observe_finalize(&mut self, code: i32) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::FinalizeCompileStatement) { return Err(ProtocolFault::UnexpectedObservation); }
            let mut s = *self.fields.native.statements[0].live().ok_or(ProtocolFault::ResourceNotInstalled)?;
            if s.action != FixedAction::CompileOptions || s.cursor != CursorPhase::NoCursor { return Err(ProtocolFault::UnexpectedObservation); }
            record_once(&mut s.finalize, code)?;
            self.fields.native.statements[0] = StmtSlot::Finalized(s);
            let r = &mut self.fields.native.compile_options; r.consumed = Some(s); r.statement_drop_owed = true; self.adverse(code); Ok(())
        }
        fn statement_drop_return(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitCompileStatementDrop) || self.fields.native.statements[0].live().is_some() { return Err(ProtocolFault::UnexpectedObservation); }
            // VM consumed before the callee decodes/drops its ignored owned
            // finalize Result. Only independent lexical completion ends debt.
            let r = &mut self.fields.native.compile_options; r.statement_drop_owed = false; r.statement_drop_ended = true; Ok(())
        }
        fn runtime_return(&mut self, fact: CompileReturn) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitCompileRuntimeReturn) { return Err(ProtocolFault::UnexpectedObservation); }
            let r = &mut self.fields.native.compile_options;
            let expected = if r.catalog_live { CompileReturn::Error } else { CompileReturn::Interrupted };
            if fact != expected { return Err(ProtocolFault::UnexpectedObservation); }
            r.runtime_return = fact; Ok(())
        }
        fn catalog_return(&mut self, fact: CompileReturn) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitCompileCatalogReturn) || fact != self.fields.native.compile_options.runtime_return { return Err(ProtocolFault::UnexpectedObservation); }
            self.fields.native.compile_options.catalog_return = fact; Ok(())
        }
        fn wrap_catalog_error(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::WrapCompileCatalogError) { return Err(ProtocolFault::UnexpectedObservation); }
            let error = self.options.catalog_error.take().ok_or(ProtocolFault::ResourceNotInstalled)?;
            self.fields.physical.primary = Some(super::super::super::retain_capture_catalog_error(error));
            self.fields.physical.audit_phase = super::super::super::FinancialAuditPhase::Failed;
            self.fields.native.compile_options.catalog_live = false; Ok(())
        }
        fn discard_owned(&mut self) -> Result<(), ProtocolFault> {
            match self.next() {
                Some(LifecycleAction::DiscardCompileRaw) => drop(self.fields.native.compile_options.raw.take()),
                Some(LifecycleAction::DiscardCompileDetail) => { drop(self.options.detail.take()); self.fields.native.compile_options.detail_started = false; },
                Some(LifecycleAction::DiscardCompileVector) => {
                    drop(self.options.rows.take()); self.fields.native.compile_options.vector_live = false;
                    if !matches!(self.fields.native.compile_sort.phase, CompileSortPhase::Dormant | CompileSortPhase::Stopped) {
                        self.fields.native.compile_sort.phase = CompileSortPhase::Draining;
                    }
                },
                Some(LifecycleAction::DiscardCompileCatalogError) => { drop(self.options.catalog_error.take()); self.fields.native.compile_options.catalog_live = false; },
                Some(LifecycleAction::DiscardCompileSourceId) => { drop(self.source_id.take()); self.fields.native.capture_prefix.value_live = false; },
                _ => return Err(ProtocolFault::UnexpectedObservation),
            } Ok(())
        }
        fn stop(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::StopCompileOptions) { return Err(ProtocolFault::UnexpectedObservation); }
            self.fields.native.compile_options.stage = CompileStage::Stopped;
            self.fields.native.capture_prefix.stage = CaptureStage::Stopped;
            if self.fields.native.compile_sort.phase == CompileSortPhase::Draining {
                self.fields.native.compile_sort.phase = CompileSortPhase::Stopped;
            }
            Ok(())
        }
    }
    impl OriginalCompileVectorReturnPort<'_, '_> {
        fn retain(self, rows: Vec<String>) {
            // Exclusive preflight owns the only empty destination. Actual
            // returned Vec moves first, without a post-acquisition refusal.
            self.loan.options.rows = Some(rows);
            let r = &mut self.loan.fields.native.compile_options;
            r.vector_pending = false; r.vector_live = true; r.collect_return = CompileReturn::Ok; r.stage = CompileStage::Ready;
        }
    }
    impl OriginalCompileDetailReturnPort<'_, '_> {
        fn retain(self, detail: String) {
            self.loan.options.detail = Some(detail); self.loan.fields.native.compile_options.detail_returned = true;
        }
    }
    impl<'a> OriginalCompileOptionsPort<'_, 'a, '_> {
        fn sql(&self) -> &'static str { "PRAGMA compile_options" }
        fn begin_prepare(&mut self) -> Result<(), ProtocolFault> { self.loan.begin_prepare() }
        fn prepare(&mut self, code: i32) -> Result<(), ProtocolFault> { self.loan.observe_prepare(code) }
        fn prepare_return(&mut self, fact: CompileReturn) -> Result<(), ProtocolFault> { self.loan.prepare_return(fact) }
        fn begin_query(&mut self) -> Result<(), ProtocolFault> { self.loan.begin_query() }
        fn query_empty(&mut self, fact: CompileReturn) -> Result<(), ProtocolFault> { self.loan.observe_query(fact) }
        fn query_return(&mut self, fact: CompileReturn) -> Result<(), ProtocolFault> { self.loan.query_return(fact) }
        fn begin_step(&mut self) -> Result<(), ProtocolFault> { self.loan.begin_step() }
        fn step(&mut self, code: i32) -> Result<(), ProtocolFault> { self.loan.observe_step(code) }
        fn column_type(&mut self, kind: i32) -> Result<(), ProtocolFault> { self.loan.observe_type(kind) }
        fn text(&mut self, bytes: &[u8]) -> Result<(), ProtocolFault> { self.loan.text(bytes) }
        fn mapper_return(&mut self, fact: CompileReturn) -> Result<(), ProtocolFault> { self.loan.mapper_return(fact) }
        fn owned_raw(&mut self, raw: rusqlite::Error) -> Result<(), rusqlite::Error> { self.loan.retain_raw(raw) }
        fn reset(&mut self, code: i32) -> Result<(), ProtocolFault> { self.loan.observe_reset(code) }
        fn rows_drop_return(&mut self) -> Result<(), ProtocolFault> { self.loan.rows_drop_return() }
        fn callee_scope_end(&mut self) -> Result<(), ProtocolFault> { self.loan.callee_scope_end() }
        fn vector_return<'short>(&'short mut self) -> Result<OriginalCompileVectorReturnPort<'short, 'a>, ProtocolFault> { self.loan.vector_return_port() }
        fn collect_return(&mut self, fact: CompileReturn) -> Result<(), ProtocolFault> { self.loan.collect_return(fact) }
        fn detail_return<'short>(&'short mut self) -> Result<OriginalCompileDetailReturnPort<'short, 'a>, ProtocolFault> { self.loan.detail_return_port() }
        fn build_catalog_error(&mut self) -> Result<(), ProtocolFault> { self.loan.build_catalog_error() }
        fn finalize(&mut self, code: i32) -> Result<(), ProtocolFault> { self.loan.observe_finalize(code) }
        fn statement_drop_return(&mut self) -> Result<(), ProtocolFault> { self.loan.statement_drop_return() }
        fn runtime_return(&mut self, fact: CompileReturn) -> Result<(), ProtocolFault> { self.loan.runtime_return(fact) }
        fn catalog_return(&mut self, fact: CompileReturn) -> Result<(), ProtocolFault> { self.loan.catalog_return(fact) }
        fn wrap_catalog_error(&mut self) -> Result<(), ProtocolFault> { self.loan.wrap_catalog_error() }
        fn discard_owned(&mut self) -> Result<(), ProtocolFault> { self.loan.discard_owned() }
        fn stop(&mut self) -> Result<(), ProtocolFault> { self.loan.stop() }
    }

    #[cfg(test)]
    impl OriginalCompileOptionsLoan<'_> {
        pub(in crate::database::global_schema_v1) fn test_code_barrier(&mut self) {
            let before = self.fields.work.test_code_observation();
            let expected = self.next().expect("fixed compile_options obligation");
            assert_eq!(self.fields.native.transaction_action(&self.fields.work, self.fields.physical), Some(expected));
            assert!(!self.fields.native.transaction_release_ready(&self.fields.work, self.fields.physical));
            let mut acquire = self.fields.reborrow().original_acquisition();
            assert_eq!(acquire.constructor().next(), expected); assert_eq!(acquire.a00_epilogue().next(), expected);
            assert!(acquire.constructor().observe(ConstructorObservation::Close(rusqlite::ffi::SQLITE_OK)).is_err());
            assert!(matches!(acquire.settle(), AcquisitionSettlement::Held(_)));
            assert_eq!(self.fields.work.test_code_observation(), before);
        }
        fn test_code_prepare(&mut self) {
            assert!(self.prepare_return(CompileReturn::Ok).is_err()); self.begin_prepare().unwrap(); assert!(self.begin_prepare().is_err());
            assert_eq!(self.observe_prepare(rusqlite::ffi::SQLITE_OK), Err(ProtocolFault::ResourceNotInstalled));
            self.fields.native.statements[0] = StmtSlot::ProtocolHeld(StmtState { action: FixedAction::CompileOptions, ..protocol_stmt_state() });
            self.observe_prepare(rusqlite::ffi::SQLITE_OK).unwrap(); assert!(self.observe_prepare(rusqlite::ffi::SQLITE_OK).is_err());
            self.test_code_barrier(); assert!(self.observe_query(CompileReturn::Ok).is_err());
            assert!(self.prepare_return(CompileReturn::Error).is_err()); self.prepare_return(CompileReturn::Ok).unwrap();
            assert!(self.prepare_return(CompileReturn::Ok).is_err());
        }
        fn test_code_query(&mut self) {
            self.test_code_prepare(); self.begin_query().unwrap(); self.observe_query(CompileReturn::Ok).unwrap(); self.test_code_barrier();
            assert!(self.test_code_step(rusqlite::ffi::SQLITE_ROW).is_err());
            assert!(self.query_return(CompileReturn::Error).is_err()); self.query_return(CompileReturn::Ok).unwrap();
            assert!(self.query_return(CompileReturn::Ok).is_err());
        }
        fn test_code_step(&mut self, code: i32) -> Result<(), ProtocolFault> {
            self.begin_step()?; self.observe_step(code)
        }
        fn test_code_eof(&mut self) {
            self.test_code_step(rusqlite::ffi::SQLITE_DONE).unwrap();
            assert!(self.vector_return_port().is_err()); self.observe_reset(rusqlite::ffi::SQLITE_OK).unwrap();
            assert!(self.observe_reset(rusqlite::ffi::SQLITE_OK).is_err()); self.test_code_scope_end();
        }
        fn test_code_scope_end(&mut self) {
            self.test_code_barrier(); assert!(self.collect_return(CompileReturn::Interrupted).is_err());
            self.rows_drop_return().unwrap(); assert!(self.rows_drop_return().is_err());
            self.test_code_barrier(); assert!(self.vector_return_port().is_err());
            self.callee_scope_end().unwrap(); assert!(self.callee_scope_end().is_err());
        }
        pub(in crate::database::global_schema_v1) fn test_code_collect_success(&mut self, rows: Vec<String>) {
            let vector = rows.as_ptr(); let capacity = rows.capacity();
            self.test_code_query();
            for text in &rows {
                self.test_code_step(rusqlite::ffi::SQLITE_ROW).unwrap(); self.observe_type(rusqlite::ffi::SQLITE_TEXT).unwrap();
                self.text(text.as_bytes()).unwrap(); self.test_code_barrier();
                assert!(self.test_code_step(rusqlite::ffi::SQLITE_ROW).is_err()); self.mapper_return(CompileReturn::Ok).unwrap();
                assert!(self.mapper_return(CompileReturn::Ok).is_err());
            }
            self.test_code_eof(); self.vector_return_port().unwrap().retain(rows);
            assert_eq!(self.options.rows.as_ref().unwrap().as_ptr(), vector); assert_eq!(self.options.rows.as_ref().unwrap().capacity(), capacity);
            assert!(self.vector_return_port().is_err()); assert!(self.collect_return(CompileReturn::Error).is_err());
            assert_eq!(self.next(), Some(LifecycleAction::CompileOptionsBeforeSort));
            assert!(self.fields.native.statements[0].live().is_some()); assert!(self.observe_finalize(rusqlite::ffi::SQLITE_OK).is_err());
            assert!(self.fields.native.compile_options.consumed.is_none());
            assert!(self.fields.native.compile_options.runtime_return == CompileReturn::Unknown);
            assert!(self.fields.native.compile_options.catalog_return == CompileReturn::Unknown); self.test_code_barrier();
        }
        pub(in crate::database::global_schema_v1) fn test_code_error(&mut self, case: super::super::super::FinancialCompileErrorCase, raw: rusqlite::Error) {
            use super::super::super::FinancialCompileErrorCase as Case;
            let pointer = compile_test_allocation(&raw);
            match case {
                Case::Prepare => { self.begin_prepare().unwrap(); self.observe_prepare(rusqlite::ffi::SQLITE_ERROR).unwrap(); },
                Case::Query => { self.test_code_prepare(); self.begin_query().unwrap(); self.observe_query(CompileReturn::Error).unwrap(); },
                Case::Step => { self.test_code_query(); self.test_code_step(rusqlite::ffi::SQLITE_ERROR).unwrap(); },
                Case::Type | Case::Utf8 => {
                    self.test_code_query(); self.test_code_step(rusqlite::ffi::SQLITE_ROW).unwrap();
                    self.observe_type(if case == Case::Type { rusqlite::ffi::SQLITE_BLOB } else { rusqlite::ffi::SQLITE_TEXT }).unwrap();
                    if case == Case::Utf8 { assert!(self.text(&[0xff]).is_err()); }
                },
                Case::DoneReset => { self.test_code_query(); self.test_code_step(rusqlite::ffi::SQLITE_DONE).unwrap(); self.observe_reset(rusqlite::ffi::SQLITE_ERROR).unwrap(); },
            }
            assert_eq!(self.next(), Some(LifecycleAction::RetainCompileRaw)); self.test_code_barrier();
            self.retain_raw(raw).unwrap_or_else(|_| panic!("actual reached raw enters same frame"));
            assert_eq!(compile_test_allocation(self.fields.native.compile_options.raw.as_ref().unwrap()), pointer);
            if case == Case::Prepare { self.prepare_return(CompileReturn::Error).unwrap(); }
            else if case == Case::Query { self.query_return(CompileReturn::Error).unwrap(); }
            else {
                if self.next() == Some(LifecycleAction::ResetCompileRows) { self.observe_reset(rusqlite::ffi::SQLITE_ERROR).unwrap(); }
                self.test_code_scope_end(); assert!(self.collect_return(CompileReturn::Ok).is_err());
                self.collect_return(CompileReturn::Error).unwrap(); assert!(self.collect_return(CompileReturn::Error).is_err());
            }
            assert_eq!(self.next(), Some(LifecycleAction::FormatCompileDetail));
            if case != Case::Prepare { assert!(self.fields.native.statements[0].live().is_some()); }
            assert!(self.observe_finalize(rusqlite::ffi::SQLITE_OK).is_err()); assert!(self.runtime_return(CompileReturn::Error).is_err());
        }
        pub(in crate::database::global_schema_v1) fn test_code_wrap_before_statement_exit(&mut self) -> usize {
            let detail = self.fields.native.compile_options.raw.as_ref().unwrap().to_string(); let pointer = detail.as_ptr() as usize;
            self.detail_return_port().unwrap().retain(detail); self.test_code_barrier();
            self.build_catalog_error().unwrap(); assert!(self.build_catalog_error().is_err());
            assert!(self.fields.native.compile_options.raw.is_none()); assert!(self.options.catalog_error.is_some());
            assert!(self.runtime_return(CompileReturn::Error).is_err());
            if self.next() == Some(LifecycleAction::FinalizeCompileStatement) {
                self.observe_finalize(rusqlite::ffi::SQLITE_ERROR).unwrap();
                assert!(self.fields.native.statements[0].live().is_none()); self.test_code_barrier();
                assert!(self.runtime_return(CompileReturn::Error).is_err()); assert!(self.observe_finalize(rusqlite::ffi::SQLITE_OK).is_err());
                self.statement_drop_return().unwrap(); assert!(self.statement_drop_return().is_err());
            }
            assert_eq!(self.next(), Some(LifecycleAction::DiscardCompileSourceId)); self.discard_owned().unwrap();
            assert!(self.source_id.is_none()); self.test_code_barrier(); assert!(self.catalog_return(CompileReturn::Error).is_err());
            self.runtime_return(CompileReturn::Error).unwrap(); assert!(self.runtime_return(CompileReturn::Error).is_err());
            assert!(self.catalog_return(CompileReturn::Ok).is_err()); self.catalog_return(CompileReturn::Error).unwrap();
            assert!(self.catalog_return(CompileReturn::Error).is_err()); self.wrap_catalog_error().unwrap();
            assert!(self.wrap_catalog_error().is_err()); self.stop().unwrap(); assert!(self.stop().is_err()); pointer
        }
        pub(in crate::database::global_schema_v1) fn test_code_interruption_cut(&mut self, cut: super::super::super::FinancialCompileTerminalCut,
            raw: rusqlite::Error) -> (Option<Vec<String>>, Option<String>, Option<rusqlite::Error>) {
            use super::super::super::FinancialCompileTerminalCut as Cut;
            let mut raw = Some(raw);
            if cut == Cut::BeforePrepare { return (None, None, raw); }
            if cut == Cut::PreparePending {
                self.begin_prepare().unwrap();
                self.fields.native.statements[0] = StmtSlot::ProtocolHeld(StmtState { action: FixedAction::CompileOptions, ..protocol_stmt_state() });
                return (None, None, raw);
            }
            if cut == Cut::QueryPending { self.test_code_prepare(); self.begin_query().unwrap(); return (None, None, raw); }
            self.test_code_query();
            if cut == Cut::StepPending { self.begin_step().unwrap(); return (None, None, raw); }
            if cut == Cut::DoneBeforeReset { self.test_code_step(rusqlite::ffi::SQLITE_DONE).unwrap(); return (None, None, raw); }
            if cut == Cut::MapperPending {
                self.test_code_step(rusqlite::ffi::SQLITE_ROW).unwrap(); self.observe_type(rusqlite::ffi::SQLITE_TEXT).unwrap();
                self.text(b"TEST_CODE private mapper obligation").unwrap(); return (None, None, raw);
            }
            if cut == Cut::RawPending { self.test_code_step(rusqlite::ffi::SQLITE_ERROR).unwrap(); return (None, None, raw); }
            if matches!(cut, Cut::DetailPending | Cut::CatalogOwned | Cut::StatementDropPending) {
                self.test_code_step(rusqlite::ffi::SQLITE_ERROR).unwrap(); self.retain_raw(raw.take().unwrap()).unwrap_or_else(|_| panic!("fixed raw"));
                self.observe_reset(rusqlite::ffi::SQLITE_OK).unwrap(); self.test_code_scope_end(); self.collect_return(CompileReturn::Error).unwrap();
                let detail = self.fields.native.compile_options.raw.as_ref().unwrap().to_string();
                let port = self.detail_return_port().unwrap();
                if cut == Cut::DetailPending { drop(port); return (None, Some(detail), None); }
                port.retain(detail); self.build_catalog_error().unwrap();
                if cut == Cut::StatementDropPending { self.observe_finalize(rusqlite::ffi::SQLITE_ERROR).unwrap(); }
                return (None, None, None);
            }
            self.test_code_eof();
            if cut == Cut::CollectPending { return (Some(Vec::new()), None, raw); }
            let rows = Vec::new(); let port = self.vector_return_port().unwrap();
            if cut == Cut::VectorPending { drop(port); return (Some(rows), None, raw); }
            assert_eq!(cut, Cut::VectorOwned); port.retain(rows); (None, None, raw)
        }
        pub(in crate::database::global_schema_v1) fn test_code_drain_interrupted(&mut self, pending_vector: Option<Vec<String>>,
            pending_detail: Option<String>, raw: Option<rusqlite::Error>) {
            let before = self.fields.work.test_code_observation();
            assert!(self.fields.work.terminal().is_some() || self.fields.physical.primary.is_some());
            assert!(self.begin_query().is_err()); assert!(self.begin_step().is_err());
            let mut raw = raw; let mut pending_vector = pending_vector; let mut pending_detail = pending_detail;
            for _ in 0..24 {
                self.test_code_barrier();
                match self.next().unwrap() {
                    LifecycleAction::AwaitCompilePrepareObservation => self.observe_prepare(rusqlite::ffi::SQLITE_OK).unwrap(),
                    LifecycleAction::AwaitCompilePrepareReturn => self.prepare_return(CompileReturn::Ok).unwrap(),
                    LifecycleAction::AwaitCompileQueryObservation => self.observe_query(CompileReturn::Error).unwrap(),
                    LifecycleAction::AwaitCompileQueryReturn => self.query_return(CompileReturn::Error).unwrap(),
                    LifecycleAction::AwaitCompileStepObservation => self.observe_step(rusqlite::ffi::SQLITE_DONE).unwrap(),
                    LifecycleAction::RetainCompileRaw => { self.retain_raw(raw.take().unwrap()).unwrap_or_else(|_| panic!("late actual raw custody")); },
                    LifecycleAction::AwaitCompileMapperReturn => { assert!(self.mapper_return(CompileReturn::Interrupted).is_err()); self.mapper_return(CompileReturn::Ok).unwrap(); },
                    LifecycleAction::AwaitCompileVector => {
                        let rows = pending_vector.take().unwrap(); let pointer = rows.as_ptr();
                        OriginalCompileVectorReturnPort { loan: self }.retain(rows);
                        assert_eq!(self.options.rows.as_ref().unwrap().as_ptr(), pointer);
                    },
                    LifecycleAction::AwaitCompileDetail => {
                        let detail = pending_detail.take().unwrap(); let pointer = detail.as_ptr();
                        OriginalCompileDetailReturnPort { loan: self }.retain(detail);
                        assert_eq!(self.options.detail.as_ref().unwrap().as_ptr(), pointer);
                    },
                    LifecycleAction::ResetCompileRows => self.observe_reset(rusqlite::ffi::SQLITE_ERROR).unwrap(),
                    LifecycleAction::AwaitCompileRowsDrop => { assert!(self.callee_scope_end().is_err()); self.rows_drop_return().unwrap(); },
                    LifecycleAction::AwaitCompileCalleeScope => { assert!(self.collect_return(CompileReturn::Interrupted).is_err()); self.callee_scope_end().unwrap(); },
                    LifecycleAction::AwaitCompileCollectReturn => {
                        let outcome = self.fields.native.compile_options.outcome; assert!(self.stop().is_err());
                        if outcome == CompileOutcome::Eof {
                            let rows = pending_vector.take().unwrap(); self.vector_return_port().unwrap().retain(rows);
                        } else {
                            let fact = if outcome == CompileOutcome::Error { CompileReturn::Error } else { CompileReturn::Interrupted };
                            assert!(self.collect_return(CompileReturn::Ok).is_err()); self.collect_return(fact).unwrap();
                            assert!(self.collect_return(fact).is_err());
                        }
                    },
                    LifecycleAction::DiscardCompileRaw | LifecycleAction::DiscardCompileDetail | LifecycleAction::DiscardCompileVector
                        | LifecycleAction::DiscardCompileCatalogError | LifecycleAction::DiscardCompileSourceId => self.discard_owned().unwrap(),
                    LifecycleAction::FinalizeCompileStatement => { self.observe_finalize(rusqlite::ffi::SQLITE_ERROR).unwrap(); assert!(self.observe_finalize(rusqlite::ffi::SQLITE_OK).is_err()); },
                    LifecycleAction::AwaitCompileStatementDrop => { assert!(self.runtime_return(CompileReturn::Interrupted).is_err()); self.statement_drop_return().unwrap(); },
                    LifecycleAction::AwaitCompileRuntimeReturn => {
                        let fact = if self.fields.native.compile_options.catalog_live { CompileReturn::Error } else { CompileReturn::Interrupted };
                        assert!(self.runtime_return(CompileReturn::Ok).is_err()); self.runtime_return(fact).unwrap();
                    },
                    LifecycleAction::AwaitCompileCatalogReturn => { let fact = self.fields.native.compile_options.runtime_return; self.catalog_return(fact).unwrap(); },
                    LifecycleAction::StopCompileOptions => { self.stop().unwrap(); break; },
                    action => panic!("new compile read/format after first stop: {action:?}"),
                }
            }
            assert!(self.fields.native.compile_options.stopped_clear()); assert!(self.fields.native.capture_prefix.stopped_clear());
            assert!(self.options.rows.is_none() && self.options.detail.is_none() && self.options.catalog_error.is_none() && self.source_id.is_none());
            assert!(pending_vector.is_none() && pending_detail.is_none()); drop(raw);
            assert_eq!(self.fields.work.test_code_observation(), before);
        }
    }
    #[cfg(test)]
    fn compile_test_allocation(raw: &rusqlite::Error) -> usize {
        match raw {
            rusqlite::Error::SqliteFailure(_, Some(detail)) | rusqlite::Error::InvalidColumnType(_, detail, _) => detail.as_ptr() as usize,
            rusqlite::Error::FromSqlConversionFailure(_, _, child) => child.as_ref() as *const _ as *const () as usize,
            rusqlite::Error::InvalidParameterCount(_, _) => 0,
            _ => panic!("fixed supplied compile-options raw child"),
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum CompileSortPhase { Dormant, BeforeSort, SortReturn, LexicalReturn, Check, NeedError, ErrorOwner, ErrorReturn, DigestHeld, Draining, Stopped }
    struct CompileSortRecord {
        phase: CompileSortPhase, body_returned: bool,
    }
    impl CompileSortRecord {
        fn empty() -> Self { Self { phase: CompileSortPhase::Dormant, body_returned: false } }
        fn blocks_early_primary(&self) -> bool {
            matches!(self.phase, CompileSortPhase::SortReturn | CompileSortPhase::LexicalReturn
                | CompileSortPhase::ErrorOwner | CompileSortPhase::ErrorReturn)
        }
        fn stopped_clear(&self) -> bool { matches!(self.phase, CompileSortPhase::Dormant | CompileSortPhase::Stopped) }
    }
    impl NativeOriginalOwner {
        fn compile_sort_action(&self, work: &OriginalSourceWork<'_>, physical: &super::super::super::FinancialPhysical) -> Option<LifecycleAction> {
            use CompileSortPhase as Phase;
            let stopped = work.terminal().is_some() || physical.primary.is_some();
            Some(match self.compile_sort.phase {
                Phase::Dormant | Phase::Stopped => return None,
                // Reached call/lexical/owned-return obligations precede both
                // first-primary and terminal cleanup. Neither supplies a fact.
                Phase::SortReturn => LifecycleAction::AwaitCompileSortReturn,
                Phase::LexicalReturn => LifecycleAction::AwaitCompileSortLexicalReturn,
                Phase::ErrorOwner => LifecycleAction::AwaitCompileDuplicateOwner,
                Phase::ErrorReturn => LifecycleAction::AwaitCompileDuplicateReturn,
                Phase::BeforeSort | Phase::Check | Phase::NeedError | Phase::DigestHeld if stopped => LifecycleAction::DiscardCompileVector,
                Phase::BeforeSort => LifecycleAction::SortCompileOptions,
                Phase::Check => LifecycleAction::CheckCompileDuplicates,
                Phase::NeedError => LifecycleAction::AcquireCompileDuplicateError,
                Phase::DigestHeld => LifecycleAction::AwaitCompileDigestSuccessor,
                Phase::Draining => {
                    if self.compile_options.vector_live { LifecycleAction::DiscardCompileVector }
                    else { return None; } // parent's fixed Stmt -> source-id -> C/G cleanup
                },
            })
        }
    }
    impl OriginalOwnerFields<'_> {
        pub(in crate::database::global_schema_v1) fn begin_compile_sort_duplicate(&mut self) -> bool {
            let n = &mut self.native; let r = &n.compile_options;
            if self.work.terminal().is_some() || self.physical.primary.is_some() || !matches!(n.tx.phase, TxPhase::Active)
                || n.compile_sort.phase != CompileSortPhase::Dormant || r.stage != CompileStage::Ready
                || r.collect_return != CompileReturn::Ok || !r.vector_live || r.blocks_early_primary()
                || r.runtime_return != CompileReturn::Unknown || r.catalog_return != CompileReturn::Unknown
                || !n.statements[0].live().is_some_and(|s| s.action == FixedAction::CompileOptions && s.cursor == CursorPhase::NoCursor)
                || !n.capture_prefix.value_live { return false; }
            n.compile_sort.phase = CompileSortPhase::BeforeSort; true
        }
    }
    pub(in crate::database::global_schema_v1) struct OriginalCompileSortLoan<'a> {
        fields: OriginalOwnerFields<'a>, options: &'a mut super::super::super::FinancialCompileOptionsState,
        source_id: &'a mut Option<String>,
    }
    struct OriginalCompileSortPort<'short, 'a, 'rules> {
        loan: &'short mut OriginalCompileSortLoan<'a>, _rules: &'rules SelectedOriginalNativeRules,
    }
    struct OriginalCompileSortBodyPort<'short> { rows: &'short mut Vec<String>, record: &'short mut CompileSortRecord }
    struct OriginalCompileDuplicateReturnPort<'short, 'a> { loan: &'short mut OriginalCompileSortLoan<'a> }
    impl<'a> OriginalOwnerFields<'a> {
        pub(in crate::database::global_schema_v1) fn compile_sort_duplicate(self,
            options: &'a mut super::super::super::FinancialCompileOptionsState, source_id: &'a mut Option<String>) -> OriginalCompileSortLoan<'a> {
            OriginalCompileSortLoan { fields: self, options, source_id }
        }
    }
    impl<'a> OriginalCompileSortLoan<'a> {
        fn fixed_port<'short, 'rules>(&'short mut self, rules: &'rules SelectedOriginalNativeRules) -> OriginalCompileSortPort<'short, 'a, 'rules> {
            OriginalCompileSortPort { loan: self, _rules: rules }
        }
        fn next(&self) -> Option<LifecycleAction> { self.fields.native.compile_options_action(&self.fields.work, self.fields.physical) }
        fn parent_loan(&mut self) -> OriginalCompileOptionsLoan<'_> {
            OriginalCompileOptionsLoan { fields: self.fields.reborrow(), options: &mut *self.options, source_id: &mut *self.source_id }
        }
        fn body_port(&mut self) -> Result<OriginalCompileSortBodyPort<'_>, ProtocolFault> {
            if self.next() != Some(LifecycleAction::SortCompileOptions) { return Err(ProtocolFault::UnexpectedObservation); }
            let rows = self.options.rows.as_mut().ok_or(ProtocolFault::ResourceNotInstalled)?;
            self.fields.native.compile_sort.phase = CompileSortPhase::SortReturn;
            Ok(OriginalCompileSortBodyPort { rows, record: &mut self.fields.native.compile_sort })
        }
        fn sort_return(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitCompileSortReturn) || !self.fields.native.compile_sort.body_returned {
                return Err(ProtocolFault::UnexpectedObservation);
            }
            self.fields.native.compile_sort.phase = CompileSortPhase::LexicalReturn; Ok(())
        }
        fn sort_lexical_return(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitCompileSortLexicalReturn) { return Err(ProtocolFault::UnexpectedObservation); }
            // Independent opaque callee/drop obligation, not scratch ownership,
            // allocation accounting, allocator authority or a payment receipt.
            self.fields.native.compile_sort.phase = CompileSortPhase::Check; Ok(())
        }
        fn check_duplicates(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::CheckCompileDuplicates) { return Err(ProtocolFault::UnexpectedObservation); }
            let rows = self.options.rows.as_ref().ok_or(ProtocolFault::ResourceNotInstalled)?;
            let duplicate = rows.windows(2).any(|pair| pair[0] == pair[1]);
            self.fields.native.compile_sort.phase = if duplicate { CompileSortPhase::NeedError } else { CompileSortPhase::DigestHeld }; Ok(())
        }
        fn duplicate_return_port(&mut self) -> Result<OriginalCompileDuplicateReturnPort<'_, 'a>, ProtocolFault> {
            if self.next() != Some(LifecycleAction::AcquireCompileDuplicateError) || self.options.catalog_error.is_some() {
                return Err(ProtocolFault::UnexpectedObservation);
            }
            self.fields.native.compile_sort.phase = CompileSortPhase::ErrorOwner;
            Ok(OriginalCompileDuplicateReturnPort { loan: self })
        }
        fn duplicate_return(&mut self) -> Result<(), ProtocolFault> {
            if self.next() != Some(LifecycleAction::AwaitCompileDuplicateReturn) || !self.fields.native.compile_options.catalog_live {
                return Err(ProtocolFault::UnexpectedObservation);
            }
            self.fields.native.compile_sort.phase = CompileSortPhase::Draining; Ok(())
        }
        fn discard_owned(&mut self) -> Result<(), ProtocolFault> {
            if self.next() == Some(LifecycleAction::DiscardCompileVector) {
                self.parent_loan().discard_owned()?;
                self.fields.native.compile_sort.phase = CompileSortPhase::Draining; return Ok(());
            }
            self.parent_loan().discard_owned()
        }
        fn stop(&mut self) -> Result<(), ProtocolFault> {
            self.parent_loan().stop()?; self.fields.native.compile_sort.phase = CompileSortPhase::Stopped; Ok(())
        }
    }
    impl OriginalCompileSortBodyPort<'_> {
        fn run(self) {
            // Only the unissued fixed Rules port reaches this production body.
            // Stable sort's actual private scratch remains a qualification gap.
            self.rows.sort(); self.record.body_returned = true;
        }
    }
    impl OriginalCompileDuplicateReturnPort<'_, '_> {
        fn retain(self, error: super::super::super::GlobalSchemaCatalogError) {
            // Exclusive preflight has the sole empty destination. A genuine
            // already-owned C error moves before any later refusal; no format,
            // clone, owned detail factory, new Work or payment issuer is added.
            self.loan.options.catalog_error = Some(error);
            self.loan.fields.native.compile_options.catalog_live = true;
            self.loan.fields.native.compile_sort.phase = CompileSortPhase::ErrorReturn;
        }
    }
    impl<'a> OriginalCompileSortPort<'_, 'a, '_> {
        fn sort_body(&mut self) -> Result<OriginalCompileSortBodyPort<'_>, ProtocolFault> { self.loan.body_port() }
        fn sort_return(&mut self) -> Result<(), ProtocolFault> { self.loan.sort_return() }
        fn lexical_return(&mut self) -> Result<(), ProtocolFault> { self.loan.sort_lexical_return() }
        fn check_duplicates(&mut self) -> Result<(), ProtocolFault> { self.loan.check_duplicates() }
        fn duplicate_owner<'short>(&'short mut self) -> Result<OriginalCompileDuplicateReturnPort<'short, 'a>, ProtocolFault> { self.loan.duplicate_return_port() }
        fn duplicate_return(&mut self) -> Result<(), ProtocolFault> { self.loan.duplicate_return() }
        fn finalize(&mut self, code: i32) -> Result<(), ProtocolFault> { self.loan.parent_loan().observe_finalize(code) }
        fn statement_drop_return(&mut self) -> Result<(), ProtocolFault> { self.loan.parent_loan().statement_drop_return() }
        fn runtime_return(&mut self, fact: CompileReturn) -> Result<(), ProtocolFault> { self.loan.parent_loan().runtime_return(fact) }
        fn catalog_return(&mut self, fact: CompileReturn) -> Result<(), ProtocolFault> { self.loan.parent_loan().catalog_return(fact) }
        fn wrap_catalog_error(&mut self) -> Result<(), ProtocolFault> { self.loan.parent_loan().wrap_catalog_error() }
        fn discard_owned(&mut self) -> Result<(), ProtocolFault> { self.loan.discard_owned() }
        fn stop(&mut self) -> Result<(), ProtocolFault> { self.loan.stop() }
    }

    #[cfg(test)]
    impl OriginalCompileSortLoan<'_> {
        pub(in crate::database::global_schema_v1) fn test_code_barrier(&mut self) {
            self.parent_loan().test_code_barrier();
            assert!(!self.fields.native.compile_sort.stopped_clear());
        }
        pub(in crate::database::global_schema_v1) fn test_code_sort_body(&mut self) {
            assert!(self.sort_return().is_err()); assert!(self.sort_lexical_return().is_err());
            self.body_port().unwrap().run();
            assert!(self.body_port().is_err()); assert!(self.check_duplicates().is_err());
            self.test_code_barrier();
            assert!(self.parent_loan().observe_finalize(rusqlite::ffi::SQLITE_OK).is_err());
        }
        pub(in crate::database::global_schema_v1) fn test_code_sort_return(&mut self) {
            self.sort_return().unwrap(); assert!(self.sort_return().is_err());
            self.test_code_barrier(); assert!(self.check_duplicates().is_err());
            self.sort_lexical_return().unwrap(); assert!(self.sort_lexical_return().is_err());
        }
        pub(in crate::database::global_schema_v1) fn test_code_check_unique(&mut self) {
            self.check_duplicates().unwrap(); assert!(self.check_duplicates().is_err());
            assert_eq!(self.next(), Some(LifecycleAction::AwaitCompileDigestSuccessor));
            assert!(self.duplicate_return_port().is_err()); assert!(self.stop().is_err());
            assert!(self.parent_loan().runtime_return(CompileReturn::Ok).is_err());
            assert!(self.parent_loan().observe_finalize(rusqlite::ffi::SQLITE_OK).is_err()); self.test_code_barrier();
        }
        pub(in crate::database::global_schema_v1) fn test_code_check_duplicate(&mut self) {
            self.check_duplicates().unwrap(); assert!(self.check_duplicates().is_err());
            assert_eq!(self.next(), Some(LifecycleAction::AcquireCompileDuplicateError));
        }
        pub(in crate::database::global_schema_v1) fn test_code_retain_duplicate(&mut self, error: super::super::super::GlobalSchemaCatalogError) {
            self.duplicate_return_port().unwrap().retain(error);
            assert!(self.duplicate_return_port().is_err()); self.test_code_barrier();
            assert!(self.discard_owned().is_err()); assert!(self.parent_loan().observe_finalize(rusqlite::ffi::SQLITE_OK).is_err());
            assert!(self.parent_loan().runtime_return(CompileReturn::Error).is_err());
        }
        pub(in crate::database::global_schema_v1) fn test_code_duplicate_return(&mut self) {
            self.duplicate_return().unwrap(); assert!(self.duplicate_return().is_err());
        }
        pub(in crate::database::global_schema_v1) fn test_code_vector_before_statement(&mut self) {
            assert_eq!(self.next(), Some(LifecycleAction::DiscardCompileVector));
            assert!(self.fields.native.statements[0].live().is_some());
            assert!(self.source_id.is_some()); assert!(self.parent_loan().observe_finalize(rusqlite::ffi::SQLITE_OK).is_err());
            self.discard_owned().unwrap(); assert!(self.options.rows.is_none());
            assert!(!self.fields.native.compile_options.vector_live); assert!(self.fields.native.statements[0].live().is_some());
            assert_eq!(self.next(), Some(LifecycleAction::FinalizeCompileStatement)); assert!(self.discard_owned().is_err()); self.test_code_barrier();
        }
        pub(in crate::database::global_schema_v1) fn test_code_statement_before_source_id(&mut self) {
            self.parent_loan().observe_finalize(rusqlite::ffi::SQLITE_ERROR).unwrap();
            assert!(self.fields.native.statements[0].live().is_none()); assert!(self.source_id.is_some());
            assert!(self.parent_loan().observe_finalize(rusqlite::ffi::SQLITE_OK).is_err());
            assert!(self.parent_loan().runtime_return(CompileReturn::Error).is_err()); self.test_code_barrier();
            // The driver's internally ignored owned finalize Result has no
            // public transport. This fixed lexical return proves no payment.
            self.parent_loan().statement_drop_return().unwrap();
            assert!(self.parent_loan().statement_drop_return().is_err());
            assert_eq!(self.next(), Some(LifecycleAction::DiscardCompileSourceId)); self.discard_owned().unwrap();
            assert!(self.source_id.is_none()); self.test_code_barrier();
        }
        pub(in crate::database::global_schema_v1) fn test_code_finish_error_returns(&mut self) {
            let expected = if self.fields.native.compile_options.catalog_live { CompileReturn::Error } else { CompileReturn::Interrupted };
            assert!(self.parent_loan().catalog_return(expected).is_err());
            assert!(self.parent_loan().runtime_return(CompileReturn::Ok).is_err()); self.test_code_barrier();
            self.parent_loan().runtime_return(expected).unwrap(); assert!(self.parent_loan().runtime_return(expected).is_err());
            assert!(self.stop().is_err()); self.parent_loan().catalog_return(expected).unwrap();
            assert!(self.parent_loan().catalog_return(expected).is_err());
            if self.fields.native.compile_options.catalog_live {
                if self.fields.work.terminal().is_some() || self.fields.physical.primary.is_some() { self.discard_owned().unwrap(); }
                else { self.parent_loan().wrap_catalog_error().unwrap(); assert!(self.parent_loan().wrap_catalog_error().is_err()); }
            }
            self.stop().unwrap(); assert!(self.stop().is_err());
            assert!(self.fields.native.compile_sort.stopped_clear()); assert!(self.fields.native.compile_options.stopped_clear());
        }
        pub(in crate::database::global_schema_v1) fn test_code_drop_body_unknown(&mut self) {
            drop(self.body_port().unwrap());
            assert!(!self.fields.native.compile_sort.body_returned); assert!(self.sort_return().is_err());
            assert!(self.sort_lexical_return().is_err()); assert!(self.body_port().is_err()); self.test_code_barrier();
        }
        pub(in crate::database::global_schema_v1) fn test_code_begin_duplicate_unknown(&mut self) {
            drop(self.duplicate_return_port().unwrap());
            assert_eq!(self.next(), Some(LifecycleAction::AwaitCompileDuplicateOwner));
            assert!(self.duplicate_return().is_err()); self.test_code_barrier();
        }
        pub(in crate::database::global_schema_v1) fn test_code_late_duplicate(&mut self, error: super::super::super::GlobalSchemaCatalogError) {
            assert_eq!(self.next(), Some(LifecycleAction::AwaitCompileDuplicateOwner));
            OriginalCompileDuplicateReturnPort { loan: self }.retain(error);
            assert_eq!(self.next(), Some(LifecycleAction::AwaitCompileDuplicateReturn)); self.test_code_barrier();
            assert!(self.discard_owned().is_err()); assert!(self.parent_loan().runtime_return(CompileReturn::Error).is_err());
        }
        pub(in crate::database::global_schema_v1) fn test_code_no_successor_after_stop(&mut self) {
            assert!(self.body_port().is_err()); assert!(self.check_duplicates().is_err()); assert!(self.duplicate_return_port().is_err());
            assert!(self.options.rows.is_some()); assert!(self.source_id.is_some()); self.test_code_barrier();
            assert!(self.parent_loan().observe_finalize(rusqlite::ffi::SQLITE_OK).is_err());
            assert!(self.parent_loan().runtime_return(CompileReturn::Ok).is_err());
        }
    }

}
