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
    pub(in crate::database::global_schema_v1) struct NativeOriginalOwner {
        original: OriginalPlace,
        aux: AuxPlace,
        statements: [StmtSlot; 3],
        statement_phase: StatementPhase,
        a00: A00Record,
        tx: TxRecord,
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
        Quiescent, FatalBorrow,
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
            if self.terminal().is_some() || self.primary.is_some()
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
        fn retain_paid_primary(&mut self, error: SourceOperationError) -> Result<(), SourceOperationError> {
            if self.terminal().is_some() || self.primary.is_some() { return Err(error); }
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
            if (self.terminal().is_none() && self.pending_normal_action().is_some())
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
            if self.next() != LifecycleAction::ReadA00Integer || self.loan.primary.is_some() { return Err(error); }
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
    }
    impl<'a> OriginalOwnerFields<'a> {
        pub(in crate::database::global_schema_v1) fn lend(
            native: &'a mut NativeOriginalOwner,
            work: OriginalSourceWork<'a>,
            release: &'a mut FixedDrainLedger,
        ) -> Self {
            Self { native, work, release }
        }
        pub(in crate::database::global_schema_v1) fn reborrow(&mut self) -> OriginalOwnerFields<'_> {
            OriginalOwnerFields {
                native: &mut *self.native,
                work: self.work.reborrow(),
                release: &mut *self.release,
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
