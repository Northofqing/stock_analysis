//! Bounded collection of candidate outcome observations. This does not turn
//! pushed_stocks into Delivered membership or daily bars into trading facts.
use crate::data_gateway::{
    AdmittedDailyBars, HistoricalBarsGateway, QualifiedListingStatus, QualifiedTradingFactsGateway,
    QualifiedTradingFactsRequest,
};
use crate::database::DatabaseManager;
use chrono::NaiveDate;
use diesel::RunQueryDsl;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy)]
pub enum OutcomeDataScope {
    PendingHistory,
    RecentSessions,
}

#[derive(Debug, Default)]
pub struct OutcomeDataRefreshReport {
    pub requested_instruments: usize,
    pub saved_bars: usize,
    pub authority_bound_bars: usize,
    pub authority_deferred_bars: usize,
    pub missing_dates: Vec<(String, NaiveDate)>,
    pub errors: Vec<String>,
    pub next_code: Option<String>,
}

#[derive(diesel::QueryableByName)]
struct Window {
    #[diesel(sql_type=diesel::sql_types::Text)]
    code: String,
    #[diesel(sql_type=diesel::sql_types::Text)]
    start: String,
}

fn expected_dates(mut start: NaiveDate, through: NaiveDate) -> Result<Vec<NaiveDate>, String> {
    if !crate::calendar::verified_a_share_trading_day(through)? || start > through {
        return Err("outcome collection requires a completed verified session range".into());
    }
    if !crate::calendar::verified_a_share_trading_day(start)? {
        // Broaden a raw candidate's collection range; no observation is
        // normalized or qualified as a prediction on this preceding session.
        start = crate::calendar::verified_prev_a_share_trading_day(start)?;
    }
    let mut dates = vec![start];
    while *dates.last().unwrap() < through {
        if dates.len() >= 800 {
            return Err("outcome collection window exceeds 800 sessions".into());
        }
        dates.push(crate::calendar::verified_next_a_share_trading_day(
            *dates.last().unwrap(),
        )?);
    }
    Ok(dates)
}

fn load_windows(
    db: &DatabaseManager,
    through: NaiveDate,
    scope: OutcomeDataScope,
    after_code: Option<&str>,
    limit: i64,
) -> Result<Vec<Window>, String> {
    if !(1..=50).contains(&limit) {
        return Err("outcome collection page must be 1..=50 instruments".into());
    }
    let since = match scope {
        OutcomeDataScope::PendingHistory => "0001-01-01".to_owned(),
        OutcomeDataScope::RecentSessions => {
            let mut since = through;
            for _ in 0..6 {
                since = crate::calendar::verified_prev_a_share_trading_day(since)?;
            }
            since.to_string()
        }
    };
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    diesel::sql_query("SELECT code,min(start) AS start FROM (SELECT code,substr(push_time,1,10) AS start FROM pushed_stocks WHERE substr(push_time,1,10)>=?1 AND substr(push_time,1,10)<=?2 UNION ALL SELECT stock_code AS code,pred_date AS start FROM prediction_tracker WHERE stock_code IS NOT NULL AND pred_date<=?2 AND (actual_change_t1 IS NULL OR hit_t1 IS NULL OR actual_change_t3 IS NULL OR hit_t3 IS NULL OR actual_change_t5 IS NULL OR hit_t5 IS NULL)) WHERE code>?3 GROUP BY code ORDER BY code LIMIT ?4")
        .bind::<diesel::sql_types::Text,_>(since).bind::<diesel::sql_types::Text,_>(through.to_string())
        .bind::<diesel::sql_types::Text,_>(after_code.unwrap_or(""))
        .bind::<diesel::sql_types::BigInt,_>(limit).load(&mut conn).map_err(|e|e.to_string())
}

/// Range is derived from original rows and the immutable calendar, never a
/// fixed recent 90 days. The transport is a latest-window observation API:
/// explicit older through dates are not asserted as native historical queries.
pub async fn refresh_outcome_data_page(
    db: &DatabaseManager,
    through: NaiveDate,
    scope: OutcomeDataScope,
    after_code: Option<&str>,
    limit: i64,
) -> Result<OutcomeDataRefreshReport, String> {
    if !crate::calendar::verified_a_share_trading_day(through)? {
        return Err("outcome refresh through is not a verified session".into());
    }
    let windows = load_windows(db, through, scope, after_code, limit)?;
    let mut report = OutcomeDataRefreshReport::default();
    for window in windows {
        report.requested_instruments += 1;
        report.next_code = Some(window.code.clone());
        let result = async {
            let identity = crate::data_gateway::instrument_identity::resolve_production_equity(
                &window.code,
                None,
            )
            .map_err(|e| e.to_string())?;
            identity.require_a_share().map_err(|e| e.to_string())?;
            let dates = expected_dates(
                NaiveDate::parse_from_str(&window.start, "%Y-%m-%d").map_err(|e| e.to_string())?,
                through,
            )?;
            let batch = HistoricalBarsGateway::new()
                .required_daily_bars_async(&window.code, dates.len())
                .await
                .map_err(|e| e.to_string())?;
            persist_observed_batch(db, &window.code, &dates, &batch, &mut report)?;
            Ok::<(), String>(())
        }
        .await;
        if let Err(error) = result {
            report.errors.push(format!("{}: {error}", window.code));
        }
    }
    Ok(report)
}

fn persist_observed_batch(
    db: &DatabaseManager,
    code: &str,
    expected: &[NaiveDate],
    batch: &AdmittedDailyBars,
    report: &mut OutcomeDataRefreshReport,
) -> Result<(), String> {
    let through = *expected.last().ok_or("outcome expected window is empty")?;
    if batch.target_code() != code || batch.records().iter().any(|bar| bar.date > through) {
        return Err("outcome batch identity or completed-session boundary mismatch".into());
    }
    let present: BTreeSet<_> = batch.records().iter().map(|bar| bar.date).collect();
    let missing: Vec<_> = expected
        .iter()
        .copied()
        .filter(|date| !present.contains(date))
        .map(|date| (code.to_owned(), date))
        .collect();
    let identity = crate::data_gateway::instrument_identity::resolve_production_equity(code, None)
        .map_err(|e| e.to_string())?;
    let gateway = QualifiedTradingFactsGateway::new();
    let facts: Vec<_> = batch
        .records()
        .iter()
        .map(|bar| {
            gateway.acquire(QualifiedTradingFactsRequest::new(
                identity.instrument().clone(),
                bar.date,
            ))
        })
        .collect();
    let all_available = facts.iter().all(|fact| {
        fact.lifecycle().require() == Ok(&QualifiedListingStatus::Listed)
            && fact.suspension().require().is_ok()
    });
    let count = if all_available {
        db.save_admitted_kline_with_trading_facts(batch, &facts)
            .map_err(|e| e.to_string())?
    } else {
        // Keep actual observations and explicitly leave every status Unknown;
        // ordinary persistence invalidates any previous marker for these keys.
        db.save_admitted_kline_data(batch)
            .map_err(|e| e.to_string())?
    };
    report.saved_bars += count;
    if all_available {
        report.authority_bound_bars += count;
    } else {
        report.authority_deferred_bars += count;
    }
    report.missing_dates.extend(missing);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }
    #[test]
    fn outcome_collection_uses_full_original_range_and_verified_holidays() {
        let dates = expected_dates(day("2026-07-02"), day("2026-10-09")).unwrap();
        assert_eq!(dates.first(), Some(&day("2026-07-02")));
        assert_eq!(dates.last(), Some(&day("2026-10-09")));
        assert!(!dates.contains(&day("2026-09-25")));
        assert!(!dates.contains(&day("2026-10-07")));
        assert!(expected_dates(day("2026-10-07"), day("2026-10-09"))
            .unwrap()
            .contains(&day("2026-09-30")));
        assert!(expected_dates(day("2026-09-30"), day("2026-10-07")).is_err());
        assert!(expected_dates(day("2024-01-01"), day("2026-10-09")).is_err());
    }
    #[test]
    fn outcome_collection_pages_real_candidates_and_pending_predictions_independently_of_holdings()
    {
        let dir = tempfile::tempdir().unwrap();
        let db = DatabaseManager::open_isolated_for_test(dir.path().join("TEST_CODE_windows.db"))
            .unwrap();
        let mut conn = db.get_conn().unwrap();
        for (code, date) in [
            ("TEST_CODE_old", "2026-07-02"),
            ("TEST_CODE_recent", "2026-09-30"),
            ("TEST_CODE_recent", "2026-09-29"),
        ] {
            diesel::sql_query("INSERT INTO pushed_stocks(push_time,push_kind,code,name,push_price,metric_json,source) VALUES (?1,'D-01',?2,'TEST_CODE',10,'{}','TEST_CODE')")
                .bind::<diesel::sql_types::Text,_>(date).bind::<diesel::sql_types::Text,_>(code).execute(&mut conn).unwrap();
        }
        db.save_prediction_legacy(
            "2026-07-03",
            "2026-07-06",
            None,
            Some("TEST_CODE_prediction"),
            "up",
            60.,
            None,
        )
        .unwrap();
        let first = load_windows(
            &db,
            day("2026-09-30"),
            OutcomeDataScope::PendingHistory,
            None,
            1,
        )
        .unwrap();
        assert_eq!(first[0].code, "TEST_CODE_old");
        assert_eq!(first[0].start, "2026-07-02");
        let second = load_windows(
            &db,
            day("2026-09-30"),
            OutcomeDataScope::PendingHistory,
            Some(&first[0].code),
            1,
        )
        .unwrap();
        assert_eq!(second[0].code, "TEST_CODE_prediction");
        let recent = load_windows(
            &db,
            day("2026-09-30"),
            OutcomeDataScope::RecentSessions,
            None,
            50,
        )
        .unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[1].start, "2026-09-29");
        assert!(load_windows(
            &db,
            day("2026-09-30"),
            OutcomeDataScope::PendingHistory,
            None,
            51
        )
        .is_err());
    }
}
