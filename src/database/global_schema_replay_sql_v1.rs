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
                || self.fields.native.initial_read.phase == InitialPhase::Primary || self.fields.native.initial_read.driver_error.is_some() { return Err(error); }
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
}
