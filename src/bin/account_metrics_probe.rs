//! 2026-09-21 评估 #12: 账户模式 metrics 装配探针 (只读).
//!
//! 复用用户确认汇总和持仓，明确区分缺失、时间绑定和时效拒绝，再经原
//! verified effective-fill 路径检查账本指标。读取显式文件的私有只读副本，
//! 不初始化全局数据库，不迁移、估值、seed、批准资金或触发任何 provider。

use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;
use stock_analysis::database::attribution_reports::{
    AttributionDatabaseAccess, AttributionDatabaseSession,
};
use stock_analysis::database::{user_account_summary, user_position_snapshot, DatabaseManager};

fn current_day_pnl_pct(
    snapshot_at: chrono::DateTime<chrono::FixedOffset>,
    evaluated_at: chrono::DateTime<chrono::FixedOffset>,
    daily_pnl: f64,
    total_assets: f64,
) -> Result<f64, String> {
    if total_assets <= 0.0 {
        return Err("total assets must be positive for account mode".to_owned());
    }
    let pnl_pct = daily_pnl / total_assets * 100.0;
    if !pnl_pct.is_finite() {
        return Err("daily PnL ratio is non-finite".to_owned());
    }
    let china_offset = chrono::FixedOffset::east_opt(8 * 60 * 60).expect("China offset");
    if snapshot_at.with_timezone(&china_offset).date_naive()
        != evaluated_at.with_timezone(&china_offset).date_naive()
    {
        return Err("account summary does not contain today's PnL".to_owned());
    }
    Ok(pnl_pct)
}

fn available_net_pnl(
    net: &stock_analysis::performance::economic_position::NetMetrics,
) -> Result<f64, String> {
    use stock_analysis::performance::economic_position::NetMetrics;
    match net {
        NetMetrics::Available { net_pnl, .. } => Ok(*net_pnl),
        NetMetrics::Unavailable { .. } => Err("closed position net PnL is unavailable".to_owned()),
    }
}

#[derive(Parser, Debug)]
#[command(about = "Inspect confirmed account and holdings in a private read-only snapshot")]
struct Args {
    /// Existing database or stable copy. Never creates a database or migrates it.
    #[arg(long)]
    db: PathBuf,
}

struct AccountInputs {
    summary: user_account_summary::UserAccountSummary,
    positions: Option<user_position_snapshot::UserPositionSnapshot>,
}

fn load_account_inputs(database: &DatabaseManager) -> Result<AccountInputs, String> {
    let summary = user_account_summary::latest_from_database(database)
        .map_err(|error| format!("latest user account summary: {error}"))?
        .ok_or_else(|| "user account summary is missing".to_owned())?;
    let positions = user_position_snapshot::latest_user_position_snapshot_from_database(database)
        .map_err(|error| format!("confirmed position snapshot read failed: {error}"))?;
    Ok(AccountInputs { summary, positions })
}

fn validate_account_inputs(
    inputs: &AccountInputs,
    observed_at: chrono::DateTime<chrono::FixedOffset>,
) -> Result<(f64, u8), String> {
    let summary = &inputs.summary;
    let effective_at = chrono::DateTime::parse_from_rfc3339(&summary.effective_at)
        .map_err(|error| format!("effective_at unparseable: {error}"))?;
    let age = observed_at.signed_duration_since(effective_at);
    if age < chrono::Duration::zero() {
        return Err(format!("summary from the future: age={age}"));
    }
    if age > chrono::Duration::hours(96) {
        return Err(format!("summary present but stale: age={age} max_age=96h"));
    }
    if summary.source.trim().is_empty() {
        return Err("account summary source is empty".to_owned());
    }
    if !summary.total_assets.is_finite()
        || summary.total_assets <= 0.0
        || !summary.available_cash.is_finite()
        || summary.available_cash < 0.0
        || !summary.securities_market_value.is_finite()
        || summary.securities_market_value < 0.0
        || !summary.position_ratio_pct.is_finite()
        || !(0.0..=100.0).contains(&summary.position_ratio_pct)
        || !summary.daily_pnl.is_finite()
    {
        return Err("confirmed account amounts are invalid".to_owned());
    }
    // Match the existing valuation component check; retain other assets rather
    // than silently replacing total assets with cash plus securities.
    if summary.total_assets - summary.available_cash - summary.securities_market_value < -0.01 {
        return Err("confirmed account total is below cash plus securities".to_owned());
    }
    let positions = inputs.positions.as_ref().ok_or_else(|| {
        "confirmed position snapshot missing; account summary is present".to_owned()
    })?;
    if positions.effective_at != effective_at {
        return Err("account/position effective_at mismatch".to_owned());
    }
    if positions.confirmed_at > observed_at
        || positions.confirmed_at < positions.effective_at
        || positions.source.trim().is_empty()
        || positions.confirm_empty != positions.items.is_empty()
        || positions.items.iter().any(|item| {
            item.code.trim().is_empty()
                || item.name.trim().is_empty()
                || item.quantity == 0
                || item.quantity > i64::MAX as u64
                || !item.cost_price.is_finite()
                || item.cost_price <= 0.0
        })
    {
        return Err("confirmed position snapshot is invalid".to_owned());
    }
    if positions.confirm_empty && summary.securities_market_value > 0.0 {
        return Err("confirmed empty holdings conflict with securities market value".to_owned());
    }
    let today_pnl_pct = current_day_pnl_pct(
        effective_at,
        observed_at,
        summary.daily_pnl,
        summary.total_assets,
    )?;
    let total_pos_cheng = (summary.position_ratio_pct / 10.0).round().clamp(0.0, 10.0) as u8;
    Ok((today_pnl_pct, total_pos_cheng))
}

fn run_probe(
    database: &DatabaseManager,
    observed_at: chrono::DateTime<chrono::FixedOffset>,
) -> Result<(), String> {
    let inputs = load_account_inputs(database)?;
    let summary = &inputs.summary;
    println!(
        "summary: present effective_at={} total={:.2} cash={:.2} market_value={:.2} pos={:.1}% pnl={:.2} source={}",
        summary.effective_at,
        summary.total_assets,
        summary.available_cash,
        summary.securities_market_value,
        summary.position_ratio_pct,
        summary.daily_pnl,
        summary.source,
    );
    match &inputs.positions {
        Some(positions) => println!(
            "holdings: present count={} confirmed_empty={} effective_at={} confirmed_at={} snapshot_id={} source={}",
            positions.items.len(),
            positions.confirm_empty,
            positions.effective_at,
            positions.confirmed_at,
            positions.snapshot_id,
            positions.source,
        ),
        None => println!("holdings: missing (account summary is present)"),
    }
    let (today_pnl_pct, total_pos_cheng) = validate_account_inputs(&inputs, observed_at)?;
    let china_offset = chrono::FixedOffset::east_opt(8 * 60 * 60).expect("China offset");
    let as_of = observed_at.with_timezone(&china_offset).date_naive();
    // Use the same opaque verified source and epoch selection as production;
    // a raw table/count is never substituted for a qualified ledger anchor.
    let effective = stock_analysis::performance::economic_position::query_effective_fills_through_from_database(database, as_of)
        .map_err(|error| format!("paper ledger anchor: {error}"))?;
    let report = stock_analysis::performance::economic_position::report_from_effective(&effective)
        .map_err(|error| format!("paper ledger anchor: {error}"))?;
    println!(
        "ledger: closed_positions={} open_positions={} (费率逐笔成本净口径)",
        report.closed_positions.len(),
        report.open_positions.len()
    );
    if let Some(opening) = &report.opening_inventory {
        println!("opening inventory: remaining_lots={} excluded_exit_parts={} projection={} seed={:?} (期初份额不计策略连续止损)",opening.remaining_opening_lots.len(),opening.excluded_exits.len(),opening.projection_hash,opening.seed_binding);
    }
    let mut realized: Vec<(chrono::NaiveDateTime, String, f64)> = report
        .closed_positions
        .iter()
        .map(|position| {
            Ok((
                position.closed_at,
                format!("economic-cycle-{}", position.cycle_open_fill_id),
                available_net_pnl(&position.net)?,
            ))
        })
        .collect::<Result<_, String>>()?;
    realized.sort_by(|left, right| right.0.cmp(&left.0));
    println!("recent closed cycles (newest first):");
    for (closed_at, identity, pnl) in realized.iter().take(8) {
        println!("  {closed_at}  {identity}  pnl={pnl:+.2}");
    }
    let consecutive = realized
        .iter()
        .take(5)
        .take_while(|(_, _, pnl)| *pnl < 0.0)
        .count();
    println!(
        "metrics: today_pnl_pct={today_pnl_pct:+.3} total_pos_cheng={total_pos_cheng} consecutive_stop_loss_n={consecutive}"
    );
    Ok(())
}

fn main() -> ExitCode {
    let args = Args::parse();
    let session =
        match AttributionDatabaseSession::open(&args.db, AttributionDatabaseAccess::ReadOnly) {
            Ok(session) => session,
            Err(error) => {
                eprintln!(
                    "account snapshot read failed: {}: {}",
                    error.reason_code(),
                    error.detail()
                );
                return ExitCode::FAILURE;
            }
        };
    match run_probe(session.database(), chrono::Local::now().fixed_offset()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("account metrics probe failed: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diesel::{Connection, RunQueryDsl};
    use stock_analysis::performance::economic_position::NetMetrics;

    fn at(value: &str) -> chrono::DateTime<chrono::FixedOffset> {
        chrono::DateTime::parse_from_rfc3339(value).unwrap()
    }

    fn fixture(with_positions: bool) -> (tempfile::TempDir, PathBuf, AttributionDatabaseSession) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TEST_CODE_confirmed_account.db");
        let mut writer = diesel::SqliteConnection::establish(path.to_str().unwrap()).unwrap();
        user_account_summary::create_schema(&mut writer).unwrap();
        user_position_snapshot::create_schema(&mut writer).unwrap();
        diesel::sql_query("INSERT INTO user_account_summary(effective_at,total_assets,securities_market_value,available_cash,position_ratio_pct,daily_pnl,source) VALUES ('2026-09-28T15:00:00+08:00',1000,600,400,60,-10,'TEST_CODE_user_confirmed')")
            .execute(&mut writer).unwrap();
        if with_positions {
            diesel::sql_query("INSERT INTO user_position_snapshot(snapshot_id,effective_at,confirmed_at,source,confirm_empty,evidence_sha256,item_count) VALUES ('TEST_CODE_snapshot','2026-09-28T15:00:00+08:00','2026-09-28T15:01:00+08:00','TEST_CODE_user_confirmed',0,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',1)")
                .execute(&mut writer).unwrap();
            diesel::sql_query("INSERT INTO user_position_snapshot_item(snapshot_id,code,name,quantity,cost_price) VALUES ('TEST_CODE_snapshot','TEST_CODE_000001','TEST_CODE_holding',100,6)")
                .execute(&mut writer).unwrap();
        }
        drop(writer);
        let session =
            AttributionDatabaseSession::open(&path, AttributionDatabaseAccess::ReadOnly).unwrap();
        (dir, path, session)
    }

    #[test]
    fn readonly_probe_reuses_cash_and_holdings_without_migrating_source() {
        let (_dir, path, session) = fixture(true);
        let before = std::fs::read(&path).unwrap();
        let inputs = load_account_inputs(session.database()).unwrap();
        assert_eq!(inputs.summary.available_cash, 400.0);
        assert_eq!(inputs.summary.total_assets, 1000.0);
        let positions = inputs.positions.as_ref().unwrap();
        assert_eq!(positions.items[0].quantity, 100);
        assert_eq!(positions.items[0].cost_price, 6.0);
        assert_eq!(
            validate_account_inputs(&inputs, at("2026-09-28T15:10:00+08:00")),
            Ok((-1.0, 6))
        );
        let mut connection = session.database().get_conn().unwrap();
        assert!(
            diesel::sql_query("CREATE TABLE TEST_CODE_forbidden(id INTEGER)")
                .execute(&mut connection)
                .is_err()
        );
        drop(connection);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        drop(session);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        for suffix in ["-wal", "-shm", "-journal"] {
            assert!(!PathBuf::from(format!("{}{suffix}", path.display())).exists());
        }
    }

    #[test]
    fn existing_stale_inputs_are_reported_as_present_before_ledger_check() {
        let (_dir, _path, session) = fixture(true);
        let inputs = load_account_inputs(session.database()).unwrap();
        assert!(inputs.positions.is_some());
        assert_eq!(inputs.summary.available_cash, 400.0);
        let error = run_probe(session.database(), at("2026-10-07T13:16:00+08:00")).unwrap_err();
        assert!(error.contains("summary present but stale"), "{error}");
        assert!(error.contains("max_age=96h"), "{error}");
        assert!(!error.contains("missing"), "{error}");
    }

    #[test]
    fn missing_holdings_do_not_erase_existing_cash_or_count_as_confirmed_empty() {
        let (_dir, _path, session) = fixture(false);
        let mut inputs = load_account_inputs(session.database()).unwrap();
        assert_eq!(inputs.summary.available_cash, 400.0);
        let now = at("2026-09-28T15:10:00+08:00");
        assert!(validate_account_inputs(&inputs, now)
            .unwrap_err()
            .contains("confirmed position snapshot missing; account summary is present"));
        inputs.positions = Some(user_position_snapshot::UserPositionSnapshot {
            snapshot_row_id: 1,
            snapshot_id: "TEST_CODE_confirmed_empty".into(),
            effective_at: at(&inputs.summary.effective_at),
            confirmed_at: at("2026-09-28T15:01:00+08:00"),
            source: "TEST_CODE_user_confirmed".into(),
            confirm_empty: true,
            evidence_sha256: "b".repeat(64),
            items: Vec::new(),
        });
        assert!(validate_account_inputs(&inputs, now)
            .unwrap_err()
            .contains("empty holdings conflict"));
        inputs.summary.total_assets = 400.0;
        inputs.summary.securities_market_value = 0.0;
        inputs.summary.position_ratio_pct = 0.0;
        assert_eq!(validate_account_inputs(&inputs, now), Ok((-2.5, 0)));
        inputs.positions.as_mut().unwrap().confirm_empty = false;
        assert!(validate_account_inputs(&inputs, now)
            .unwrap_err()
            .contains("position snapshot is invalid"));
    }

    #[test]
    fn account_position_binding_and_future_confirmation_are_checked() {
        let (_dir, _path, session) = fixture(true);
        let mut inputs = load_account_inputs(session.database()).unwrap();
        let now = at("2026-09-28T15:10:00+08:00");
        inputs.positions.as_mut().unwrap().effective_at = at("2026-09-27T15:00:00+08:00");
        assert!(validate_account_inputs(&inputs, now)
            .unwrap_err()
            .contains("account/position effective_at mismatch"));
        let positions = inputs.positions.as_mut().unwrap();
        positions.effective_at = at(&inputs.summary.effective_at);
        positions.confirmed_at = at("2026-09-28T16:00:00+08:00");
        assert!(validate_account_inputs(&inputs, now)
            .unwrap_err()
            .contains("position snapshot is invalid"));
        assert!(
            validate_account_inputs(&inputs, at("2026-09-28T14:59:00+08:00"))
                .unwrap_err()
                .contains("summary from the future")
        );
    }

    #[test]
    fn existing_fresh_account_sources_do_not_substitute_for_a_verified_ledger() {
        let (_dir, path, session) = fixture(true);
        let before = std::fs::read(&path).unwrap();
        let error = run_probe(session.database(), at("2026-09-28T15:10:00+08:00")).unwrap_err();
        assert!(error.starts_with("paper ledger anchor:"), "{error}");
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn explicit_path_is_required_and_a_missing_database_is_not_created() {
        assert!(Args::try_parse_from(["account_metrics_probe"]).is_err());
        assert!(Args::try_parse_from(["account_metrics_probe", "--db", "x", "--seed"]).is_err());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TEST_CODE_absent.db");
        assert!(
            AttributionDatabaseSession::open(&path, AttributionDatabaseAccess::ReadOnly).is_err()
        );
        assert!(!path.exists());
    }

    #[test]
    fn invalid_cash_or_exposure_is_not_accepted_as_complete_account_evidence() {
        let (_dir, _path, session) = fixture(true);
        let mut inputs = load_account_inputs(session.database()).unwrap();
        let now = at("2026-09-28T15:10:00+08:00");
        for cash in [-1.0, f64::INFINITY, f64::NAN] {
            inputs.summary.available_cash = cash;
            assert_eq!(
                validate_account_inputs(&inputs, now).unwrap_err(),
                "confirmed account amounts are invalid"
            );
        }
        inputs.summary.available_cash = 400.0;
        inputs.summary.position_ratio_pct = 101.0;
        assert!(validate_account_inputs(&inputs, now).is_err());
        inputs.summary.position_ratio_pct = 60.0;
        inputs.summary.available_cash = 500.0;
        assert!(validate_account_inputs(&inputs, now)
            .unwrap_err()
            .contains("total is below cash plus securities"));
        inputs.summary.available_cash = 399.9;
        assert_eq!(validate_account_inputs(&inputs, now), Ok((-1.0, 6)));
        inputs.positions.as_mut().unwrap().items[0].quantity = u64::MAX;
        assert!(validate_account_inputs(&inputs, now)
            .unwrap_err()
            .contains("position snapshot is invalid"));
    }

    #[test]
    fn previous_day_summary_cannot_be_reported_as_today_pnl() {
        let yesterday = chrono::DateTime::parse_from_rfc3339("2026-09-28T16:00:00+08:00").unwrap();
        let today = chrono::DateTime::parse_from_rfc3339("2026-09-29T10:00:00+08:00").unwrap();
        assert!(current_day_pnl_pct(yesterday, today, 10.0, 1000.0).is_err());
        assert_eq!(current_day_pnl_pct(today, today, 10.0, 1000.0), Ok(1.0));
    }

    #[test]
    fn unavailable_net_cost_cannot_fall_back_to_gross_pnl() {
        assert!(available_net_pnl(&NetMetrics::Unavailable {
            reason: "missing fee evidence".to_owned(),
        })
        .is_err());
    }
}
