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
mod original_native {
    use super::super::super::rows::original_source::{OriginalSourceWork, SourceTerminal};
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
        MaterializeCount, ProspectiveExtent, Pragmas, Integrity, ForeignKeyCheck,
        SourceId, CompileOptions, Catalog, ForeignKeys, IndexList, IndexXinfo,
        AttachedNames, TableCount, ReferenceDdl, Encoding, TempCheck,
        RowsExtent, TableShape, TableColumns, RowsPreflight, RowsStream,
        CopiedQueryOnly, SelectionReconciliation, Begin, Commit, Rollback,
        ConstructorClose, OriginalClose, ReferenceClose, CopiedClose,
    }
    #[repr(C, u8)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum CodeSlot { NotCalled, Called(i32) }
    struct ConnectionStatus {
        open: CodeSlot,
        extended_result: CodeSlot,
        busy_timeout: CodeSlot,
        close_first: CodeSlot,
        close_second: CodeSlot,
    }
    #[repr(u8)]
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
    enum CursorPhase { NoCursor, Active, Ended }
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
    struct TxRecord {
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
    struct NativeOriginalOwner {
        original: OriginalPlace,
        aux: AuxPlace,
        statements: [StmtSlot; 3],
        statement_phase: StatementPhase,
        tx: TxRecord,
        secondary: Option<FixedAdverse>,
        _thread: PhantomData<Rc<()>>,
    }
    // No Connection/Statement/Rows/Transaction overlap, raw getter, from_raw,
    // live-handle constructor or default Drop is introduced. Real acquisition
    // and explicit qualified release remain the next behavior slice.
    impl NativeOriginalOwner {
        fn empty() -> Self {
            Self {
                original: OriginalPlace::Empty,
                aux: AuxPlace::Empty,
                statements: [StmtSlot::Vacant, StmtSlot::Vacant, StmtSlot::Vacant],
                statement_phase: StatementPhase::Empty,
                tx: TxRecord {
                    phase: TxPhase::NotCreated,
                    exit: TxExit::NotSelected,
                    autocommit: AutocommitObservation::NotObserved,
                    rollback: RollbackObservation::NotReached,
                },
                secondary: None,
                _thread: PhantomData,
            }
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
    struct FixedDrainLedger {
        first_secondary: Option<FixedAdverse>,
        filesystem: FsObservation,
        audit: AuditReleaseStatus,
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
}
