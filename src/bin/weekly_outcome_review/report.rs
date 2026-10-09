use chrono::{DateTime, Duration, FixedOffset, NaiveDate, NaiveDateTime};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Nullable, Text};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use stock_analysis::calendar::{verified_a_share_trading_day, verified_next_a_share_trading_day};
use stock_analysis::database::DatabaseManager;
use stock_analysis::performance::economic_position::{
    query_effective_fills_through_from_database_typed, report_from_effective, NetMetrics,
    NetSummary,
};

const MAX_ROWS: i64 = 100_000;
const MAX_PREDICTIONS: i64 = 4096;

#[derive(Debug, Serialize)]
pub struct ReadState<T> {
    pub status: &'static str,
    pub value: Option<T>,
    pub reason: Option<String>,
}
impl<T> ReadState<T> {
    fn available(value: T) -> Self {
        Self {
            status: "available",
            value: Some(value),
            reason: None,
        }
    }
    fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            status: "unavailable",
            value: None,
            reason: Some(reason.into()),
        }
    }
    fn from_result(result: Result<T, String>) -> Self {
        match result {
            Ok(value) => Self::available(value),
            Err(reason) => Self::unavailable(reason),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Period {
    pub requested_from: NaiveDate,
    pub requested_to: NaiveDate,
    pub observed_at: DateTime<FixedOffset>,
    pub latest_completed_session: NaiveDate,
    pub period_completed_through: NaiveDate,
    pub completed_sessions: Vec<NaiveDate>,
}
impl Period {
    pub fn new(from: NaiveDate, to: NaiveDate, now: DateTime<FixedOffset>) -> Result<Self, String> {
        if to < from || (to - from).num_days() > 6 {
            return Err("weekly range must be 1..=7 natural days".into());
        }
        let latest = stock_analysis::monitor::prediction::completed_session_as_of_at(now)?;
        let through = latest.min(to);
        let mut sessions = Vec::new();
        let mut date = from;
        while date <= to {
            let trading = verified_a_share_trading_day(date)?;
            if date <= through && trading {
                sessions.push(date);
            }
            date = date.succ_opt().ok_or("weekly date overflow")?;
        }
        // The boundary is the last actual completed session, including when the
        // requested calendar week ends on a closure. Never label to as a session.
        let completed = if verified_a_share_trading_day(through)? {
            through
        } else {
            stock_analysis::calendar::verified_prev_a_share_trading_day(through)?
        };
        Ok(Self {
            requested_from: from,
            requested_to: to,
            observed_at: now,
            latest_completed_session: latest,
            period_completed_through: completed,
            completed_sessions: sessions,
        })
    }
    fn includes(&self, date: NaiveDate) -> bool {
        self.completed_sessions.contains(&date)
    }
}

#[derive(Debug, Serialize, QueryableByName)]
pub struct SourceExtent {
    #[diesel(sql_type = BigInt)]
    pub rows: i64,
    #[diesel(sql_type = BigInt)]
    pub codes: i64,
    #[diesel(sql_type = Nullable<Text>)]
    pub earliest: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub latest: Option<String>,
}
#[derive(Debug, Serialize, Default)]
pub struct Counts {
    pub rows: usize,
    pub revalidated_observations: usize,
    pub recorded_pairs: usize,
    pub pending_pairs: usize,
    pub not_mature: usize,
    pub missing_qualification: usize,
    pub suspended: usize,
    pub missing_price: usize,
    pub invalid: usize,
    pub ready_unrecorded: usize,
    pub hits: usize,
    pub observation_hit_rate: Option<f64>,
    pub observation_mean_change_pct: Option<f64>,
    #[serde(skip)]
    change_sum: f64,
}
impl Counts {
    fn add(&mut self, window: &Window) {
        self.rows += 1;
        if window.recorded_pair {
            self.recorded_pairs += 1;
        } else {
            self.pending_pairs += 1;
        }
        match window.status.as_str() {
            "revalidated_observation" => {
                self.revalidated_observations += 1;
                self.hits += usize::from(window.hit == Some(true));
                self.change_sum += window.change_pct.unwrap();
                self.observation_hit_rate =
                    Some(self.hits as f64 / self.revalidated_observations as f64);
                self.observation_mean_change_pct =
                    Some(self.change_sum / self.revalidated_observations as f64);
            }
            "not_mature" => self.not_mature += 1,
            "missing_qualification" => self.missing_qualification += 1,
            "suspended" => self.suspended += 1,
            "missing_price" => self.missing_price += 1,
            "ready_unrecorded" => self.ready_unrecorded += 1,
            _ => self.invalid += 1,
        }
    }
}
#[derive(Debug, Serialize)]
pub struct Horizon {
    pub trading_days: usize,
    /// Rows whose ORIGINAL pred_date is in the completed requested week.
    pub weekly_origins: Counts,
    /// Rows whose calendar-derived horizon matures in the requested week.
    pub maturing_this_week: Counts,
    /// All original rows at/before the period boundary, including old backlog.
    pub history_through_period: Counts,
}
#[derive(Debug, Serialize)]
pub struct Window {
    pub prediction_row_id: i64,
    pub code: Option<String>,
    pub original_pred_date: String,
    pub original_target_date: String,
    pub trading_days: usize,
    pub calendar_maturity: Option<NaiveDate>,
    pub weekly_origin: bool,
    pub matures_this_week: bool,
    pub status: String,
    pub recorded_pair: bool,
    pub change_pct: Option<f64>,
    pub hit: Option<bool>,
    pub gaps: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct Predictions {
    pub original_rows: usize,
    pub horizons: Vec<Horizon>,
    pub windows: Vec<Window>,
    pub schema_gaps: Vec<String>,
    pub reliable_prediction_samples: ReadState<usize>,
}

#[derive(QueryableByName)]
struct Prediction {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Text)]
    pred_date: String,
    #[diesel(sql_type = Text)]
    target_date: String,
    #[diesel(sql_type = Nullable<Text>)]
    code: Option<String>,
    #[diesel(sql_type = Text)]
    direction: String,
    #[diesel(sql_type = Nullable<Double>)]
    change1: Option<f64>,
    #[diesel(sql_type = Nullable<Double>)]
    change3: Option<f64>,
    #[diesel(sql_type = Nullable<Double>)]
    change5: Option<f64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    hit1: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    hit3: Option<i64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    hit5: Option<i64>,
}
#[derive(QueryableByName)]
struct Column {
    #[diesel(sql_type = Text)]
    name: String,
}
fn columns(conn: &mut SqliteConnection, table: &str) -> Result<BTreeSet<String>, String> {
    // Only fixed internal table names enter this helper.
    diesel::sql_query(format!("PRAGMA table_info({table})"))
        .load::<Column>(conn)
        .map(|rows| rows.into_iter().map(|row| row.name).collect())
        .map_err(|e| e.to_string())
}
#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = BigInt)]
    count: i64,
}
fn bounded(conn: &mut SqliteConnection, table: &str, max: i64) -> Result<(), String> {
    let count = diesel::sql_query(format!("SELECT COUNT(*) AS count FROM {table}"))
        .get_result::<Count>(conn)
        .map_err(|e| e.to_string())?
        .count;
    if count > max {
        return Err(format!(
            "{table} exceeds bounded report limit {max}; no partial denominator"
        ));
    }
    Ok(())
}
fn extent(db: &DatabaseManager, table: &str, date: &str) -> Result<SourceExtent, String> {
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    diesel::sql_query(format!("SELECT COUNT(*) AS rows,COUNT(DISTINCT code) AS codes,MIN({date}) AS earliest,MAX({date}) AS latest FROM {table}"))
        .get_result(&mut conn).map_err(|e| e.to_string())
}
fn canonical_date(raw: &str) -> Result<NaiveDate, String> {
    let date = NaiveDate::parse_from_str(raw, "%Y-%m-%d").map_err(|_| "invalid original date")?;
    if date.to_string() != raw || !verified_a_share_trading_day(date)? {
        return Err("original date is not a canonical verified session".into());
    }
    Ok(date)
}
fn expected_hit(direction: &str, change: f64) -> Result<bool, String> {
    match direction.trim().to_lowercase().as_str() {
        "up" | "bullish" | "long" | "看多" | "上涨" => Ok(change > 0.5),
        "down" | "bearish" | "short" | "看空" | "下跌" => Ok(change < -0.5),
        "neutral" | "flat" | "sideways" | "中性" | "震荡" | "观望" => Ok(change.abs() <= 0.5),
        _ => Err("unsupported original prediction direction".into()),
    }
}
#[derive(QueryableByName)]
struct State {
    #[diesel(sql_type = Text)]
    status: String,
    #[diesel(sql_type = Text)]
    contract_version: String,
    #[diesel(sql_type = Text)]
    source: String,
    #[diesel(sql_type = Text)]
    source_at: String,
    #[diesel(sql_type = Text)]
    observed_at: String,
    #[diesel(sql_type = Text)]
    batch_id: String,
}
#[derive(QueryableByName)]
struct Close {
    #[diesel(sql_type = Nullable<Double>)]
    close: Option<f64>,
    #[diesel(sql_type = Nullable<BigInt>)]
    is_suspended: Option<i64>,
}
fn classify(
    conn: &mut SqliteConnection,
    row: &Prediction,
    n: usize,
    period: &Period,
    change: Option<f64>,
    hit: Option<i64>,
    qualified_schema: bool,
    price_schema: bool,
) -> Window {
    let recorded =
        change.is_some_and(|v| v.is_finite() && v >= -100.) && matches!(hit, Some(0 | 1));
    let mut window = Window {
        prediction_row_id: row.id,
        code: row.code.clone(),
        original_pred_date: row.pred_date.clone(),
        original_target_date: row.target_date.clone(),
        trading_days: n,
        calendar_maturity: None,
        weekly_origin: false,
        matures_this_week: false,
        status: "invalid".into(),
        recorded_pair: recorded,
        change_pct: None,
        hit: None,
        gaps: Vec::new(),
    };
    let result = (|| -> Result<(String, Option<f64>, Option<bool>), String> {
        let start = canonical_date(&row.pred_date)?;
        // Attribution follows the valid original prediction date even when
        // another original field is bad. Such rows remain weekly invalids.
        window.weekly_origin = period.includes(start);
        let frozen = canonical_date(&row.target_date)?;
        if row.id <= 0 || frozen < start {
            return Err("invalid original row identity/target".into());
        }
        expected_hit(&row.direction, 0.)?;
        let mut dates = vec![start];
        for _ in 0..n {
            dates.push(verified_next_a_share_trading_day(*dates.last().unwrap())?);
        }
        let target = *dates.last().unwrap();
        window.calendar_maturity = Some(target);
        window.matures_this_week = period.includes(target);
        if target > period.period_completed_through {
            if change.is_some() || hit.is_some() {
                window
                    .gaps
                    .push("stored result precedes report-period maturity; excluded".into());
            }
            return Ok(("not_mature".into(), None, None));
        }
        if (change.is_some() || hit.is_some())
            && (!recorded
                || hit
                    != Some(i64::from(expected_hit(
                        &row.direction,
                        change.unwrap_or(0.),
                    )?)))
        {
            return Err(
                "stored result pair incomplete/invalid or contradicts original direction".into(),
            );
        }
        let code = row
            .code
            .as_deref()
            .filter(|v| !v.trim().is_empty())
            .ok_or("missing original stock code")?;
        if !qualified_schema {
            window
                .gaps
                .push("independent daily trading qualification table/fields absent".into());
            return Ok(("missing_qualification".into(), None, None));
        }
        // Every expected session needs an explicit independent status. Endpoint
        // OHLC or a missing bar never manufactures a trading/suspension fact.
        let mut missing = false;
        let mut suspended = false;
        for date in &dates {
            let states = diesel::sql_query("SELECT status,contract_version,source,source_at,observed_at,batch_id FROM qualified_daily_trading_status WHERE code=?1 AND date=?2")
                .bind::<Text,_>(code).bind::<Text,_>(date.to_string()).load::<State>(conn).map_err(|e| e.to_string())?;
            let state = if states.len() == 1 {
                &states[0]
            } else {
                missing = true;
                window.gaps.push(format!(
                    "{code}/{date}: independent status absent or duplicated"
                ));
                continue;
            };
            let times = DateTime::parse_from_rfc3339(&state.source_at)
                .ok()
                .zip(DateTime::parse_from_rfc3339(&state.observed_at).ok());
            if [&state.contract_version,&state.source,&state.batch_id].iter().any(|v| v.trim().is_empty())
                || state.contract_version == stock_analysis::data_gateway::qualified_trading_facts::QUALIFIED_TRADING_FACTS_CONTRACT_V1
                || !times.is_some_and(|(source, observed)| source <= observed && observed <= period.observed_at) {
                missing = true;
                window.gaps.push(format!("{code}/{date}: independent status evidence invalid/future"));
            } else if state.status == "suspended" {
                suspended = true;
                window.gaps.push(format!("{code}/{date}: explicit suspended fact; no zero return"));
            } else if state.status != "trading" { return Err("invalid independent status".into()); }
        }
        if missing {
            return Ok(("missing_qualification".into(), None, None));
        }
        if suspended {
            return Ok(("suspended".into(), None, None));
        }
        if !price_schema {
            window.gaps.push("daily close table/fields absent".into());
            return Ok(("missing_price".into(), None, None));
        }
        let mut closes = Vec::new();
        for date in [start, target] {
            let values = diesel::sql_query("SELECT close,CAST(is_suspended AS BIGINT) AS is_suspended FROM stock_daily WHERE code=?1 AND date=?2")
                .bind::<Text,_>(code).bind::<Text,_>(date.to_string()).load::<Close>(conn).map_err(|e| e.to_string())?;
            if values.len() != 1
                || values[0].is_suspended != Some(0)
                || values[0].close.is_none_or(|v| !v.is_finite() || v <= 0.)
            {
                window.gaps.push(format!(
                    "{code}/{date}: qualified endpoint close absent/invalid/conflicting"
                ));
                return Ok(("missing_price".into(), None, None));
            }
            closes.push(values[0].close.unwrap());
        }
        let observed = (closes[1] - closes[0]) / closes[0] * 100.;
        if !observed.is_finite() || observed < -100. {
            return Err("invalid close-to-close observation".into());
        }
        if change.is_none() && hit.is_none() {
            window.gaps.push("qualified snapshot endpoints available; original result not recorded, no backfill performed".into());
            return Ok(("ready_unrecorded".into(), None, None));
        }
        if !recorded
            || (change.unwrap() - observed).abs() > 1e-8
            || hit != Some(i64::from(expected_hit(&row.direction, observed)?))
        {
            return Err("stored result incomplete/invalid or differs from current qualified endpoint observation".into());
        }
        Ok((
            "revalidated_observation".into(),
            Some(observed),
            Some(hit == Some(1)),
        ))
    })();
    match result {
        Ok((status, change, hit)) => {
            window.status = status;
            window.change_pct = change;
            window.hit = hit;
        }
        Err(reason) => window.gaps.push(reason),
    }
    window
}
fn predictions(db: &DatabaseManager, period: &Period) -> Result<Predictions, String> {
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    bounded(&mut conn, "prediction_tracker", MAX_PREDICTIONS)?;
    let fields = columns(&mut conn, "prediction_tracker")?;
    for required in [
        "id",
        "pred_date",
        "target_date",
        "stock_code",
        "pred_direction",
    ] {
        if !fields.contains(required) {
            return Err(format!("prediction_tracker missing {required}"));
        }
    }
    let mut schema_gaps = Vec::new();
    let mut projection = Vec::new();
    for n in [1, 3, 5] {
        for (prefix, alias) in [("actual_change_t", "change"), ("hit_t", "hit")] {
            let column = format!("{prefix}{n}");
            if fields.contains(&column) {
                projection.push(format!("{column} AS {alias}{n}"));
            } else {
                projection.push(format!("NULL AS {alias}{n}"));
                schema_gaps.push(format!("legacy schema missing {column}"));
            }
        }
    }
    let rows = diesel::sql_query(format!("SELECT id,pred_date,target_date,stock_code AS code,pred_direction AS direction,{} FROM prediction_tracker ORDER BY id", projection.join(",")))
        .load::<Prediction>(&mut conn).map_err(|e| e.to_string())?;
    let qualified = columns(&mut conn, "qualified_daily_trading_status")?;
    let qualified_schema = [
        "code",
        "date",
        "status",
        "contract_version",
        "source",
        "source_at",
        "observed_at",
        "batch_id",
    ]
    .iter()
    .all(|v| qualified.contains(*v));
    let daily = columns(&mut conn, "stock_daily")?;
    let price_schema = ["code", "date", "close", "is_suspended"]
        .iter()
        .all(|v| daily.contains(*v));
    let mut horizons = [1, 3, 5]
        .map(|n| Horizon {
            trading_days: n,
            weekly_origins: Counts::default(),
            maturing_this_week: Counts::default(),
            history_through_period: Counts::default(),
        })
        .into_iter()
        .collect::<Vec<_>>();
    let mut windows = Vec::new();
    for row in &rows {
        // Keep malformed original dates visible. Future originating rows are
        // outside this historical report; do not read them as matured evidence.
        if canonical_date(&row.pred_date).is_ok_and(|date| date > period.period_completed_through) {
            continue;
        }
        for (index, n, change, hit) in [
            (0, 1, row.change1, row.hit1),
            (1, 3, row.change3, row.hit3),
            (2, 5, row.change5, row.hit5),
        ] {
            let window = classify(
                &mut conn,
                row,
                n,
                period,
                change,
                hit,
                qualified_schema,
                price_schema,
            );
            horizons[index].history_through_period.add(&window);
            if window.weekly_origin {
                horizons[index].weekly_origins.add(&window);
            }
            if window.matures_this_week {
                horizons[index].maturing_this_week.add(&window);
            }
            windows.push(window);
        }
    }
    Ok(Predictions { original_rows:rows.len(), horizons, windows, schema_gaps,
        reliable_prediction_samples:ReadState::unavailable("full prediction/window qualification and historical available_at/PIT evidence are not represented by these legacy rows; snapshot revalidation is descriptive only") })
}

#[derive(QueryableByName)]
struct PaperRow {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = Text)]
    direction: String,
    #[diesel(sql_type = Text)]
    status: String,
    #[diesel(sql_type = Nullable<Text>)]
    not_fill_reason: Option<String>,
    #[diesel(sql_type = Text)]
    virtual_reason: String,
    #[diesel(sql_type = Text)]
    ts: String,
    #[diesel(sql_type = Text)]
    updated_at: String,
}
fn persisted_utc(raw: &str) -> Result<NaiveDateTime, String> {
    let at = NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f")
        .map_err(|_| "invalid original UTC timestamp")?;
    let whole = at.format("%Y-%m-%d %H:%M:%S").to_string();
    if raw != whole
        && !raw.strip_prefix(&(whole + ".")).is_some_and(|tail| {
            !tail.is_empty() && tail.len() <= 9 && tail.bytes().all(|v| v.is_ascii_digit())
        })
    {
        return Err("noncanonical original UTC timestamp".into());
    }
    at.checked_add_signed(Duration::hours(8))
        .ok_or("UTC timestamp overflow".into())
}
#[derive(Debug, Serialize)]
pub struct RawExit {
    pub row_id: i64,
    pub code: String,
    pub status: String,
    pub reason: String,
    pub terminal_at_shanghai: NaiveDateTime,
}
#[derive(Debug, Serialize)]
pub struct FutureTimestamp {
    pub row_id: i64,
    pub recorded_at_shanghai: NaiveDateTime,
}
#[derive(Debug, Serialize)]
pub struct RawPaper {
    pub historical_filled_rows: usize,
    pub latest_filled_utc: Option<String>,
    pub weekly_states: BTreeMap<String, usize>,
    pub weekly_not_filled_reasons: BTreeMap<String, usize>,
    pub weekly_sell_rows: Vec<RawExit>,
    pub malformed_timestamp_rows: usize,
    pub future_timestamp_rows: usize,
    pub future_timestamps: Vec<FutureTimestamp>,
    pub meaning: &'static str,
}
fn raw_paper(db: &DatabaseManager, period: &Period) -> Result<RawPaper, String> {
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    bounded(&mut conn, "paper_trades", MAX_ROWS)?;
    let rows=diesel::sql_query("SELECT id,code,direction,status,not_fill_reason,virtual_reason,CAST(ts AS TEXT) AS ts,CAST(updated_at AS TEXT) AS updated_at FROM paper_trades ORDER BY id")
        .load::<PaperRow>(&mut conn).map_err(|e|e.to_string())?;
    let mut result=RawPaper { historical_filled_rows:0,latest_filled_utc:None,weekly_states:BTreeMap::new(),weekly_not_filled_reasons:BTreeMap::new(),weekly_sell_rows:Vec::new(),malformed_timestamp_rows:0,future_timestamp_rows:0,future_timestamps:Vec::new(),
        meaning:"whole-input-snapshot raw Filled/latest diagnostics include future rows and are not as of observed_at; weekly facts exclude future timestamps; Filled is not verified execution, net return, physical delivery, or a win-rate denominator" };
    for row in rows {
        if row.status == "Filled" {
            result.historical_filled_rows += 1;
            if persisted_utc(&row.ts).is_ok()
                && result
                    .latest_filled_utc
                    .as_ref()
                    .is_none_or(|v| v < &row.ts)
            {
                result.latest_filled_utc = Some(row.ts.clone());
            }
        }
        // Filled ts is the existing economic owner's fact time. Rejections use
        // their recorded terminal updated_at, never the trigger's date alone.
        let raw = if matches!(row.status.as_str(), "NotFilled" | "Invalidated") {
            &row.updated_at
        } else {
            &row.ts
        };
        let at = match persisted_utc(raw) {
            Ok(at) => at,
            Err(_) => {
                result.malformed_timestamp_rows += 1;
                continue;
            }
        };
        if at > period.observed_at.naive_local() {
            result.future_timestamp_rows += 1;
            result.future_timestamps.push(FutureTimestamp {
                row_id: row.id,
                recorded_at_shanghai: at,
            });
            continue;
        }
        if !period.includes(at.date()) {
            continue;
        }
        *result.weekly_states.entry(row.status.clone()).or_default() += 1;
        if matches!(row.status.as_str(), "NotFilled" | "Invalidated") {
            *result
                .weekly_not_filled_reasons
                .entry(
                    row.not_fill_reason
                        .clone()
                        .filter(|v| !v.trim().is_empty())
                        .unwrap_or("missing recorded reason".into()),
                )
                .or_default() += 1;
        }
        if row.direction == "sell" {
            result.weekly_sell_rows.push(RawExit {
                row_id: row.id,
                code: row.code,
                status: row.status,
                reason: row.not_fill_reason.unwrap_or(row.virtual_reason),
                terminal_at_shanghai: at,
            });
        }
    }
    Ok(result)
}
#[derive(QueryableByName)]
struct AuditRow {
    #[diesel(sql_type=BigInt)]
    id: i64,
    #[diesel(sql_type=Text)]
    source: String,
    #[diesel(sql_type=Text)]
    side: String,
    #[diesel(sql_type=Text)]
    outcome: String,
    #[diesel(sql_type=Nullable<Text>)]
    failure_reason: Option<String>,
    #[diesel(sql_type=Text)]
    created_at: String,
}
#[derive(Debug, Serialize, Default)]
pub struct OrderAttempts {
    pub source_side_outcomes: BTreeMap<String, usize>,
    pub failure_reasons: BTreeMap<String, usize>,
    pub malformed_timestamp_rows: usize,
    pub future_timestamp_rows: usize,
    pub future_timestamps: Vec<FutureTimestamp>,
}
fn attempts(db: &DatabaseManager, period: &Period) -> Result<OrderAttempts, String> {
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    bounded(&mut conn, "order_audit", MAX_ROWS)?;
    let rows=diesel::sql_query("SELECT id,source,side,outcome,failure_reason,CAST(created_at AS TEXT) AS created_at FROM order_audit ORDER BY id")
        .load::<AuditRow>(&mut conn).map_err(|e|e.to_string())?;
    let mut result = OrderAttempts::default();
    for row in rows {
        let at = match persisted_utc(&row.created_at) {
            Ok(at) => at,
            Err(_) => {
                result.malformed_timestamp_rows += 1;
                continue;
            }
        };
        if at > period.observed_at.naive_local() {
            result.future_timestamp_rows += 1;
            result.future_timestamps.push(FutureTimestamp {
                row_id: row.id,
                recorded_at_shanghai: at,
            });
            continue;
        }
        if !period.includes(at.date()) {
            continue;
        }
        *result
            .source_side_outcomes
            .entry(format!("{} / {} / {}", row.source, row.side, row.outcome))
            .or_default() += 1;
        if row.outcome != "Filled" {
            *result
                .failure_reasons
                .entry(
                    row.failure_reason
                        .filter(|v| !v.trim().is_empty())
                        .unwrap_or("missing recorded reason".into()),
                )
                .or_default() += 1;
        }
    }
    Ok(result)
}
#[derive(Debug, Serialize)]
pub struct VerifiedPaper {
    pub projection_sha256: String,
    pub rule_version: String,
    pub source_fill_rows: usize,
    pub legacy_without_terminal_rows: usize,
    pub period_fill_rows: usize,
    pub period_scenario_fill_cost_cny: Option<f64>,
    pub closed_cycles_in_week: usize,
    pub open_cycles_at_period_end: usize,
    pub closed_cycle_scenario_cost_cny: Option<f64>,
    pub closed_cycle_scenario_net_pnl_cny: Option<f64>,
    pub exits: Vec<VerifiedExit>,
    pub scenario_amount_unavailable_reason: Option<String>,
    pub actual_settlement_costs: ReadState<f64>,
    pub executable_net_return: ReadState<f64>,
}
#[derive(Debug, Serialize)]
pub struct VerifiedExit {
    pub code: String,
    pub closed_at_shanghai: NaiveDateTime,
    pub fill_ids: Vec<i64>,
    pub exit_reasons: Vec<String>,
    pub scenario_cost_cny: Option<f64>,
    pub scenario_net_pnl_cny: Option<f64>,
    pub scenario_amount_unavailable_reason: Option<String>,
}
fn verified_paper(db: &DatabaseManager, period: &Period) -> Result<VerifiedPaper, String> {
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    bounded(&mut conn, "paper_trades", MAX_ROWS)?;
    drop(conn);
    let effective =
        query_effective_fills_through_from_database_typed(db, period.period_completed_through)
            .map_err(|e| e.to_string())?;
    let rows = effective.rows().map_err(|e| e.to_string())?;
    let mut period_ids = BTreeSet::new();
    for row in rows {
        let at = NaiveDateTime::parse_from_str(&row.occurred_at, "%Y-%m-%d %H:%M:%S%.f")
            .map_err(|e| e.to_string())?;
        // The existing effective capability is day-granularity and verifies the
        // full ledger. Never manufacture an intraday subset of economic facts.
        if at > period.observed_at.naive_local() {
            return Err(format!("future_effective_fill row={} occurred_at_shanghai={} exceeds observed_at={}; reliable paper unavailable, day-granularity effective ledger is not truncated", row.id, at, period.observed_at));
        }
        if period.includes(at.date()) {
            period_ids.insert(row.id);
        }
    }
    let costs = effective.costs().map_err(|e| e.to_string())?;
    let economics = report_from_effective(&effective)?;
    let mut exits = Vec::new();
    for position in economics
        .closed_positions
        .iter()
        .filter(|p| period.includes(p.closed_at.date()))
    {
        let (cost, net) = match &position.net {
            NetMetrics::Available {
                total_adverse_cost,
                net_pnl,
                ..
            } => (Some(*total_adverse_cost), Some(*net_pnl)),
            NetMetrics::Unavailable { .. } => (None, None),
        };
        exits.push(VerifiedExit {
            code: position.code.clone(),
            closed_at_shanghai: position.closed_at,
            fill_ids: position.source_fill_ids.clone(),
            exit_reasons: position.exit_reasons.clone(),
            scenario_cost_cny: cost,
            scenario_net_pnl_cny: net,
            scenario_amount_unavailable_reason: match &position.net {
                NetMetrics::Unavailable { reason } => Some(reason.clone()),
                _ => None,
            },
        });
    }
    let (cycle_cost, cycle_net, scenario_amount_unavailable_reason) =
        closed_scenario_amounts(&exits, &economics.net_summary);
    let aggregate_amounts_available = matches!(economics.net_summary, NetSummary::Available { .. });
    Ok(VerifiedPaper { projection_sha256:effective.receipt().projection_hash.clone(),rule_version:effective.receipt().rule_version.clone(),source_fill_rows:rows.len(),
        legacy_without_terminal_rows:effective.lineage().iter().filter(|v|matches!(v.authority,stock_analysis::trading::paper_ledger::FillAuthority::LegacyNoTerminal)).count(),
        period_fill_rows:period_ids.len(),period_scenario_fill_cost_cny:(!period_ids.is_empty() && aggregate_amounts_available).then(||costs.costs.iter().filter(|v|period_ids.contains(&v.fill_id)).map(|v|v.adverse_cost).sum()),
        closed_cycles_in_week:exits.len(),open_cycles_at_period_end:economics.open_positions.len(),closed_cycle_scenario_cost_cny:cycle_cost,closed_cycle_scenario_net_pnl_cny:cycle_net,exits,scenario_amount_unavailable_reason,
        actual_settlement_costs:ReadState::unavailable("no observed settlement fee evidence is read by this local paper review; lot-rates-v1 costs are scenario estimates"),
        executable_net_return:ReadState::unavailable("verified paper arithmetic and modeled fees do not establish executable fills, brokerage settlement or historical price/PIT qualification") })
}

/// The owner's typed unavailable summary also guards aggregate amounts when a
/// disputed lifecycle remains open, or is outside this week's closed subset.
pub(super) fn closed_scenario_amounts(
    exits: &[VerifiedExit],
    summary: &NetSummary,
) -> (Option<f64>, Option<f64>, Option<String>) {
    if let NetSummary::Unavailable { reason } = summary {
        return (None, None, Some(reason.clone()));
    }
    if exits.is_empty() {
        return (
            None,
            None,
            Some(
                "no closed cycles in completed requested sessions; no zero-return denominator"
                    .into(),
            ),
        );
    }
    let cost = exits.iter().map(|v| v.scenario_cost_cny).sum();
    let net = exits.iter().map(|v| v.scenario_net_pnl_cny).sum();
    let reason = exits
        .iter()
        .find_map(|v| v.scenario_amount_unavailable_reason.clone());
    (cost, net, reason)
}

#[derive(Debug, Serialize)]
pub struct InputSource {
    pub database_path: String,
    /// Source main-file bytes only. The wrapper supplies a normalized backup;
    /// this is not a market qualification or a digest of a live WAL sequence.
    pub source_main_sha256: String,
    pub original_source_label: Option<String>,
    pub temporary_snapshot_deleted_after_run: bool,
    pub boundary: &'static str,
}
#[derive(Debug, Serialize)]
pub struct Review {
    pub scorecard: Option<super::scorecard::Scorecard>,
    pub evidence_manifest: Option<super::scorecard::EvidenceManifest>,
    pub report_version: &'static str,
    pub input_source: Option<InputSource>,
    pub period: Period,
    pub source_extent_boundary: &'static str,
    pub daily_bars: ReadState<SourceExtent>,
    pub independent_daily_status: ReadState<SourceExtent>,
    pub predictions: ReadState<Predictions>,
    pub raw_paper: ReadState<RawPaper>,
    pub original_order_attempts: ReadState<OrderAttempts>,
    pub verified_paper: ReadState<VerifiedPaper>,
    pub physical_delivery: ReadState<usize>,
    pub next_week_actions: Vec<String>,
    pub interpretation: &'static str,
}
pub fn read(db: &DatabaseManager, period: Period) -> Review {
    let daily_bars = ReadState::from_result(extent(db, "stock_daily", "date"));
    let independent_daily_status =
        ReadState::from_result(extent(db, "qualified_daily_trading_status", "date"));
    let predictions = ReadState::from_result(predictions(db, &period));
    let raw_paper = ReadState::from_result(raw_paper(db, &period));
    let original_order_attempts = ReadState::from_result(attempts(db, &period));
    let verified_paper = ReadState::from_result(verified_paper(db, &period));
    let mut actions = Vec::new();
    if predictions.value.as_ref().is_none_or(|p| {
        p.horizons
            .iter()
            .any(|h| h.history_through_period.missing_qualification > 0)
    }) {
        actions.push("H08：取得逐证券/逐交易日状态、生命周期、价格资格和修订证据；验收前保持 Unknown/deferred。".into());
    }
    if predictions.value.as_ref().is_some_and(|p| {
        p.horizons.iter().any(|h| {
            h.history_through_period.pending_pairs > 0 || h.history_through_period.missing_price > 0
        })
    }) {
        actions.push("Outcome：合格证据到位后，按原候选/预测日期有界回填；分别记录 T+1/3/5 缺数与未成熟窗口。".into());
    }
    if let Some(reason) = &verified_paper.reason {
        actions.push(format!("Paper：保留失败账本证据并核对原成交/审计：{reason}；不改量、删单、制造 seed 或自动重置。"));
    }
    actions.push("Monitor：在已完成交易日核对真实接纳/失败/恢复及原未成交/退出原因；休市无新增原行不能证明恢复。".into());
    actions.push("根据描述性原事实复核下周动作；不自动调整策略、预算、实验或投递。".into());
    Review {scorecard:None,evidence_manifest:None,report_version:"H16-descriptive-weekly-v1",input_source:None,period,
        source_extent_boundary:"Raw table row counts and readability do not establish independent qualification; kline-inferred status and OHLC cannot certify lifecycle, historical availability or PIT.",
        daily_bars,independent_daily_status,predictions,raw_paper,original_order_attempts,verified_paper,
        physical_delivery:ReadState::unavailable("no independent durable database/card-to-row receipt read supplied; candidate rows, raw outcomes and paper fills are not physical message delivery denominators"),
        next_week_actions:actions,interpretation:"Descriptive snapshot review, not preregistration, causal validation, strategy promotion, historical PIT certification, funding authority or a delivery completion receipt."}
}
pub(super) fn escape(value: &str) -> String {
    value.replace('|', "\\|").replace(['\n', '\r'], " ")
}
fn amount(value: Option<f64>) -> String {
    value.map(|v| format!("{v:.2}")).unwrap_or("不可用".into())
}
impl Review {
    pub fn markdown(&self) -> String {
        let p = &self.period;
        let mut text=format!("# 周报复盘 {}–{}\n\n观察时刻：{}。最近已完成交易日：{}；本报告截止已完成交易日：{}。\n\n完成交易日（{}）：{}。\n\n描述性快照复盘；不签发预注册、因果验证、策略晋级、历史 PIT、资金或物理送达资格。\n",p.requested_from,p.requested_to,p.observed_at,p.latest_completed_session,p.period_completed_through,p.completed_sessions.len(),if p.completed_sessions.is_empty(){"本范围无已完成交易日".into()}else{p.completed_sessions.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")});
        if let Some(source) = &self.input_source {
            text.push_str(&format!(
                "\n输入副本：`{}`；主文件 SHA-256：`{}`。这是快照来源定位，不签发市场资格。\n",
                escape(&source.database_path),
                source.source_main_sha256
            ));
            if source.temporary_snapshot_deleted_after_run {
                text.push_str("\n输入为运行后清理的临时 backup；上面的路径不会持久保留，SHA-256 定位当次完整副本。\n");
            }
            if let Some(label) = &source.original_source_label {
                text.push_str(&format!("\n原来源标签（wrapper/操作人提供）：`{}`；CLI 不另行打开原来源，不据此签发资格。\n",escape(label)));
            }
        }
        text.push_str("\n## 事实与资格\n\n以下仅为原表行数/可读性，不等于独立资格；从日线推断的状态与 OHLC 不能证实生命周期或历史可用时刻/PIT。\n\n| 项目 | 原表观察（未核验资格） |\n| --- | --- |\n");
        for (name, section) in [
            ("日线表原始行", &self.daily_bars),
            ("状态表原始行（未核验资格）", &self.independent_daily_status),
        ] {
            let value = section
                .value
                .as_ref()
                .map(|v| {
                    format!(
                        "{}行 / {}证券 / {}..{}",
                        v.rows,
                        v.codes,
                        v.earliest.as_deref().unwrap_or("无日期"),
                        v.latest.as_deref().unwrap_or("无日期")
                    )
                })
                .unwrap_or_else(|| {
                    format!("不可用：{}", section.reason.as_deref().unwrap_or("unknown"))
                });
            text.push_str(&format!("| {name} | {} |\n", escape(&value)));
        }
        text.push_str("\n## T+1/3/5\n\n原 `pred_date` 和 `target_date` 保持原件；T+n 仅用仓库核验日历计算观察成熟时点。所有预期交易日需要独立状态，价格端点需有效收盘价。后来缓存的状态只支持当前快照观察，不能授予历史可用时刻/PIT 或完整预测资格。\n");
        if let Some(pred) = &self.predictions.value {
            text.push_str(&format!("\n原预测总行数：{}。完整资格的可靠预测样本：不可用（{}）。物理送达分母：不可用。\n\n| T+n / 范围 | 原行 | 原已记录对 | 重新核对观察 | 未成熟 | 缺资格 | 停牌 | 缺价格 | 可核对但未记录 | 坏记录 |\n| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n",pred.original_rows,pred.reliable_prediction_samples.reason.as_deref().unwrap_or("qualification unavailable")));
            for h in &pred.horizons {
                for (label, c) in [
                    ("本周原预测", &h.weekly_origins),
                    ("本周成熟", &h.maturing_this_week),
                    ("截至本期历史", &h.history_through_period),
                ] {
                    text.push_str(&format!(
                        "| T+{} / {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
                        h.trading_days,
                        label,
                        c.rows,
                        c.recorded_pairs,
                        c.revalidated_observations,
                        c.not_mature,
                        c.missing_qualification,
                        c.suspended,
                        c.missing_price,
                        c.ready_unrecorded,
                        c.invalid
                    ));
                }
            }
            for h in &pred.horizons {
                let c = &h.maturing_this_week;
                if let (Some(rate), Some(change)) =
                    (c.observation_hit_rate, c.observation_mean_change_pct)
                {
                    text.push_str(&format!("\nT+{} 本周成熟描述性观察：命中 {}/{}（{:.2}%），平均收盘涨跌 {:+.4}%；非可成交净收益。\n",h.trading_days,c.hits,c.revalidated_observations,rate*100.,change));
                }
            }
            let gap_windows: Vec<_> = pred.windows.iter().filter(|w| !w.gaps.is_empty()).collect();
            if !gap_windows.is_empty() {
                text.push_str("\n缺口（全部逐项保留在 JSON；正文最多列 20 项）：\n\n");
                for w in gap_windows.iter().take(20) {
                    text.push_str(&format!(
                        "- row={} {} / T+{} / {}：{}\n",
                        w.prediction_row_id,
                        w.code.as_deref().unwrap_or("缺证券"),
                        w.trading_days,
                        w.status,
                        escape(&w.gaps.join("; "))
                    ));
                }
                if gap_windows.len() > 20 {
                    text.push_str(&format!(
                        "- 另有 {} 项，见 JSON windows。\n",
                        gap_windows.len() - 20
                    ));
                }
            }
            for gap in &pred.schema_gaps {
                text.push_str(&format!("\n旧库缺口：{}。\n", escape(gap)));
            }
        } else {
            text.push_str(&format!(
                "\n预测读端不可用：{}。\n",
                escape(self.predictions.reason.as_deref().unwrap_or("unknown"))
            ));
        }
        text.push_str("\n## Paper 原记录、未成交与退出\n");
        if let Some(raw) = &self.raw_paper.value {
            text.push_str(&format!("\n快照全量原 Filled：{}行；全量最后 UTC 时间 {}（含未来原行，非截至观察时刻）。原 Filled 总数不能作可靠胜率或可成交净收益分母。\n\n本期状态原行：{:?}；未成交原因：{:?}；无法归入日期的坏时间原行：{}；快照全量未来时间原行：{}（原 row ID/上海时间见 JSON，不进入本期状态或退出）。无本期记录时执行率/收益不可用，不归因于用户未执行。\n",raw.historical_filled_rows,raw.latest_filled_utc.as_deref().unwrap_or("无合格时间原行"),raw.weekly_states,raw.weekly_not_filled_reasons,raw.malformed_timestamp_rows,raw.future_timestamp_rows));
            for exit in &raw.weekly_sell_rows {
                text.push_str(&format!(
                    "\n卖出原行 {} / {} / {} / {} / {}（尚非结算证明）。\n",
                    exit.row_id,
                    escape(&exit.code),
                    escape(&exit.status),
                    exit.terminal_at_shanghai,
                    escape(&exit.reason)
                ));
            }
        } else {
            text.push_str(&format!(
                "\nPaper 原记录不可用：{}。\n",
                escape(self.raw_paper.reason.as_deref().unwrap_or("unknown"))
            ));
        }
        if let Some(attempts) = &self.original_order_attempts.value {
            text.push_str(&format!("\n原 order_audit 尝试（来源 / 方向 / 终态）：{:?}；原失败原因：{:?}；坏时间行：{}；快照全量未来时间原行：{}（原 row ID/上海时间见 JSON，不进入本期尝试）。仅报告原审计行，不替代账本/费用校验。\n",attempts.source_side_outcomes,attempts.failure_reasons,attempts.malformed_timestamp_rows,attempts.future_timestamp_rows));
        } else {
            text.push_str(&format!(
                "\n原尝试审计不可用：{}。\n",
                escape(
                    self.original_order_attempts
                        .reason
                        .as_deref()
                        .unwrap_or("unknown")
                )
            ));
        }
        text.push_str("\n## Paper 完整性与费用\n");
        if let Some(paper) = &self.verified_paper.value {
            text.push_str(&format!("\n现有 effective 读端通过：{}行，投影 `{}`，规则 `{}`；缺原 terminal 的 legacy 行 {}。本期有效 fill {}，本期闭合生命周期 {}，期末开放生命周期 {}（右删失）。\n\n费用类型为 lot-rates-v1 情景估算：本期 fill 费用 {} 元；本期闭合生命周期全周期费用 {} 元；闭合生命周期情景净盈亏 {} 元。无生命周期时指标不可用，不写零收益。\n\n实际结算费用：不可用。可成交净收益：不可用；paper 算术与情景费用不能替代真实价格、PIT、券商成交或费用结算资格。\n",paper.source_fill_rows,paper.projection_sha256,paper.rule_version,paper.legacy_without_terminal_rows,paper.period_fill_rows,paper.closed_cycles_in_week,paper.open_cycles_at_period_end,amount(paper.period_scenario_fill_cost_cny),amount(paper.closed_cycle_scenario_cost_cny),amount(paper.closed_cycle_scenario_net_pnl_cny)));
            for exit in &paper.exits {
                text.push_str(&format!("\n闭合退出 {} / {} / fills={:?} / 原因={:?} / 情景费用={}元 / 情景净盈亏={}元。\n",escape(&exit.code),exit.closed_at_shanghai,exit.fill_ids,exit.exit_reasons,amount(exit.scenario_cost_cny),amount(exit.scenario_net_pnl_cny)));
            }
        } else {
            text.push_str(&format!("\n现有 effective 读端失败/不可用：{}。可靠 paper 样本、费用和净收益保持不可用；原件不改量、不删单、不制造 seed。\n",escape(self.verified_paper.reason.as_deref().unwrap_or("unknown"))));
        }
        text.push_str(&format!(
            "\n物理消息送达：不可用（{}）。\n\n## 下一周动作\n\n",
            self.physical_delivery
                .reason
                .as_deref()
                .unwrap_or("unknown")
        ));
        for action in &self.next_week_actions {
            text.push_str(&format!("- {}\n", escape(action)));
        }
        if let Some(scorecard) = &self.scorecard {
            text.push_str(&scorecard.markdown());
        }
        if let Some(manifest) = &self.evidence_manifest {
            text.push_str("\n## 可复现证据 manifest\n\n以下 JSON 与旁存 manifest 相同；最长路径范围覆盖原周报所有下级指标，* 表示数组索引，评分卡指标另含逐项 evidence。\n\n```json\n");
            text.push_str(&serde_json::to_string_pretty(manifest).expect("serializable manifest"));
            text.push_str("\n```\n");
        }
        text
    }
}
