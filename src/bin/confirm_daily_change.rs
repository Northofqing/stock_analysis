//! BR-171 operator actions bind persisted evidence, never reacquired RPC data.
use chrono::{NaiveDate, Utc};
use clap::Parser;
use diesel::{connection::SimpleConnection, Connection, SqliteConnection};
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};
use stock_analysis::{
    data_gateway::historical_bars::HistoricalBarsGateway,
    database::daily_change_review::{
        run_review_action_on_conn, ReviewAction, ReviewDecision, ReviewSelection,
    },
};

#[derive(Debug, Parser)]
#[command(
    name = "confirm_daily_change",
    about = "离线审阅 BR-171 持久候选；确认/拒绝只绑定原候选与 token"
)]
struct Args {
    /// 持久候选 ID；省略时只能尝试发现（当前普通 transport 未交付发现合同）。
    #[arg(long)]
    candidate_id: Option<String>,
    /// 已存在的数据库；候选操作必填，不创建/迁移数据库。
    #[arg(long)]
    database: Option<PathBuf>,
    #[arg(long)]
    code: Option<String>,
    #[arg(long)]
    days: Option<usize>,
    #[arg(long, conflicts_with_all=["candidate_id","confirm","reject","renew","days","previous_date","current_date","evidence_token","operator","reason"])]
    prepare_window: bool,
    #[arg(long, requires = "prepare_window")]
    exchange: Option<String>,
    #[arg(long, requires = "prepare_window")]
    from: Option<NaiveDate>,
    #[arg(long, requires = "prepare_window")]
    to: Option<NaiveDate>,
    #[arg(long, requires = "prepare_window")]
    as_of: Option<chrono::DateTime<Utc>>,
    #[arg(long, requires = "prepare_window")]
    source_profile: Option<String>,
    #[arg(long, requires = "prepare_window")]
    client_bundle: Option<PathBuf>,
    #[arg(long,conflicts_with_all=["reject","renew"])]
    confirm: bool,
    #[arg(long,conflicts_with_all=["confirm","renew"])]
    reject: bool,
    #[arg(long,conflicts_with_all=["confirm","reject"])]
    renew: bool,
    #[arg(long)]
    previous_date: Option<NaiveDate>,
    #[arg(long)]
    current_date: Option<NaiveDate>,
    #[arg(long)]
    evidence_token: Option<String>,
    #[arg(long)]
    operator: Option<String>,
    #[arg(long)]
    reason: Option<String>,
}

fn exact<'a>(flag: &str, value: Option<&'a str>) -> anyhow::Result<&'a str> {
    let value = value.ok_or_else(|| anyhow::anyhow!("{flag} is required"))?;
    anyhow::ensure!(
        !value.is_empty() && value.trim() == value,
        "{flag} must be nonempty without surrounding whitespace"
    );
    Ok(value)
}
fn validate(args: &Args) -> anyhow::Result<()> {
    if args.prepare_window {
        exact("--code", args.code.as_deref())?;
        anyhow::ensure!(
            matches!(args.exchange.as_deref(), Some("Shanghai" | "Shenzhen")),
            "--exchange must be Shanghai or Shenzhen"
        );
        anyhow::ensure!(
            args.from.is_some()
                && args.to.is_some()
                && args.as_of.is_some()
                && args.database.is_some()
                && args.client_bundle.is_some(),
            "prepare requires --from --to --as-of --client-bundle --database"
        );
        let profile = exact("--source-profile", args.source_profile.as_deref())?;
        let (id, version) = profile
            .rsplit_once('@')
            .ok_or_else(|| anyhow::anyhow!("--source-profile requires id@version"))?;
        anyhow::ensure!(
            !id.is_empty() && version.parse::<u32>().is_ok_and(|v| v > 0),
            "profile version must be positive"
        );
    }
    if let Some(code) = &args.code {
        anyhow::ensure!(
            code.len() == 6
                && code.bytes().all(|b| b.is_ascii_digit())
                && matches!(
                    code.as_bytes()[0],
                    b'0' | b'2' | b'3' | b'4' | b'6' | b'8' | b'9'
                ),
            "--code must be one canonical six-digit A-share code"
        );
    }
    anyhow::ensure!(
        (2..=usize::from(u16::MAX)).contains(&args.days.unwrap_or(60)),
        "--days outside 2..65535"
    );
    if args.candidate_id.is_some() {
        exact("--candidate-id", args.candidate_id.as_deref())?;
        anyhow::ensure!(
            args.database.is_some(),
            "--database is required for persisted candidate actions"
        );
    } else {
        exact("--code", args.code.as_deref())?;
        anyhow::ensure!(
            !args.confirm
                && !args.reject
                && !args.renew
                && args.evidence_token.is_none()
                && args.operator.is_none()
                && args.reason.is_none()
                && args.previous_date.is_none()
                && args.current_date.is_none(),
            "decisions/renewal/scope assertions require --candidate-id"
        );
    }
    if args.confirm || args.reject {
        exact("--evidence-token", args.evidence_token.as_deref())?;
        exact("--operator", args.operator.as_deref())?;
        exact("--reason", args.reason.as_deref())?;
    } else if args.renew {
        exact("--evidence-token", args.evidence_token.as_deref())?;
        anyhow::ensure!(
            args.operator.is_none() && args.reason.is_none(),
            "renewal is not a decision"
        );
    } else {
        anyhow::ensure!(
            args.evidence_token.is_none() && args.operator.is_none() && args.reason.is_none(),
            "decision fields need explicit --confirm/--reject/--renew"
        );
    }
    if let (Some(start), Some(end)) = (args.previous_date, args.current_date) {
        anyhow::ensure!(start < end, "previous date must precede current date");
    }
    Ok(())
}

fn open_existing(path: &Path, read_only: bool) -> anyhow::Result<SqliteConnection> {
    let path = path.canonicalize()?;
    anyhow::ensure!(
        path.is_file(),
        "--database must be an existing regular file"
    );
    let mut uri =
        url::Url::from_file_path(path).map_err(|_| anyhow::anyhow!("invalid database path"))?;
    // mode=rw/ro prevents SQLite's default CREATE even if the file vanishes
    // after canonicalize. No DatabaseManager startup/migration is invoked.
    uri.set_query(Some(if read_only { "mode=ro" } else { "mode=rw" }));
    let mut conn = SqliteConnection::establish(uri.as_str())?;
    conn.batch_execute("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
    Ok(conn)
}

fn window_request(
    args: &Args,
) -> anyhow::Result<
    stock_analysis::data_gateway::ordinary_daily_change_window::OrdinaryDailyChangeWindowRequest,
> {
    use stock_analysis::market_domain::{AssetClass, Exchange, InstrumentId};
    Ok(stock_analysis::data_gateway::ordinary_daily_change_window::OrdinaryDailyChangeWindowRequest {
        instrument:InstrumentId::new(if args.exchange.as_deref()==Some("Shanghai"){Exchange::Shanghai}else{Exchange::Shenzhen},exact("--code",args.code.as_deref())?,AssetClass::Equity).map_err(|e|anyhow::anyhow!(e.to_string()))?,
        from:args.from.ok_or_else(||anyhow::anyhow!("--from required"))?,to:args.to.ok_or_else(||anyhow::anyhow!("--to required"))?,as_of:args.as_of.ok_or_else(||anyhow::anyhow!("--as-of required"))?,source_profile:exact("--source-profile",args.source_profile.as_deref())?.into(),
    })
}

async fn run(args: Args, output: &mut impl Write) -> anyhow::Result<()> {
    validate(&args)?;
    if args.prepare_window {
        let request = window_request(&args)?;
        let receipt = stock_analysis::data_gateway::ordinary_daily_change_window::prepare_window(
            request,
            args.client_bundle.as_deref().unwrap(),
            args.database.as_deref().unwrap(),
        )
        .await?;
        serde_json::to_writer(&mut *output, &receipt)?;
        writeln!(output)?;
        output.flush()?;
        return Ok(());
    }
    let Some(candidate_id) = args.candidate_id.as_deref() else {
        HistoricalBarsGateway::new()
            .pending_daily_change_confirmations_async(
                args.code.as_deref().unwrap(),
                args.days.unwrap_or(60),
            )
            .await?;
        anyhow::bail!(
            "daily_change_discovery_unavailable_v1: no persisted candidate discovery contract"
        );
    };
    let action = if args.confirm || args.reject {
        ReviewAction::Decide {
            token: args.evidence_token.as_deref().unwrap(),
            decision: if args.confirm {
                ReviewDecision::Confirm
            } else {
                ReviewDecision::Reject
            },
            operator: args.operator.as_deref().unwrap(),
            reason: args.reason.as_deref().unwrap(),
        }
    } else if args.renew {
        ReviewAction::Renew {
            token: args.evidence_token.as_deref().unwrap(),
        }
    } else {
        ReviewAction::Review
    };
    let mut conn = open_existing(
        args.database.as_deref().unwrap(),
        !args.confirm && !args.reject && !args.renew,
    )?;
    run_review_action_on_conn(
        &mut conn,
        ReviewSelection {
            candidate_id,
            code: args.code.as_deref(),
            previous_date: args.previous_date,
            current_date: args.current_date,
        },
        action,
        Utc::now(),
        output,
    )?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run(Args::parse(), &mut io::stdout().lock()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persisted_actions_require_explicit_id_database_token_and_decision() {
        let args = Args::try_parse_from([
            "confirm_daily_change",
            "--candidate-id",
            "br171_review_TEST_CODE",
            "--database",
            "TEST_CODE.sqlite",
            "--confirm",
            "--evidence-token",
            "TEST_CODE_token",
            "--operator",
            "operator",
            "--reason",
            "reviewed",
        ])
        .unwrap();
        validate(&args).unwrap();
        assert!(Args::try_parse_from(["confirm_daily_change", "--confirm", "--reject"]).is_err());
        let args =
            Args::try_parse_from(["confirm_daily_change", "--candidate-id", "TEST_CODE"]).unwrap();
        assert!(validate(&args).is_err());
        let args = Args::try_parse_from(["confirm_daily_change", "--code", "300005", "--confirm"])
            .unwrap();
        assert!(validate(&args).is_err());
        let args = Args::try_parse_from(["confirm_daily_change", "--code", "sz300005"]).unwrap();
        assert!(validate(&args).is_err());
    }
    #[test]
    fn existing_connection_never_creates_and_review_connection_is_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TEST_CODE ?# review.sqlite");
        assert!(open_existing(&path, false).is_err());
        assert!(!path.exists());
        let mut create = SqliteConnection::establish(path.to_str().unwrap()).unwrap();
        create
            .batch_execute("CREATE TABLE TEST_CODE_guard(value INTEGER)")
            .unwrap();
        drop(create);
        let mut reader = open_existing(&path, true).unwrap();
        assert!(reader
            .batch_execute("INSERT INTO TEST_CODE_guard VALUES(1)")
            .is_err());
        let mut writer = open_existing(&path, false).unwrap();
        writer
            .batch_execute("INSERT INTO TEST_CODE_guard VALUES(1)")
            .unwrap();
    }
    #[tokio::test]
    async fn discover_without_raw_contract_is_unavailable_and_creates_no_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("TEST_CODE_absent.sqlite");
        let args = Args::try_parse_from([
            "confirm_daily_change",
            "--code",
            "300005",
            "--database",
            path.to_str().unwrap(),
        ])
        .unwrap();
        assert!(run(args, &mut Vec::new())
            .await
            .unwrap_err()
            .to_string()
            .contains("daily_change_discovery_unavailable_v1"));
        assert!(!path.exists());
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;
    fn args() -> Vec<&'static str> {
        vec![
            "confirm_daily_change",
            "--prepare-window",
            "--exchange",
            "Shanghai",
            "--code",
            "600519",
            "--from",
            "2026-09-11",
            "--to",
            "2026-09-15",
            "--as-of",
            "2026-09-16T08:00:00Z",
            "--source-profile",
            "TEST_CODE_SYNTHETIC@1",
            "--client-bundle",
            "TEST_CODE.bundle",
            "--database",
            "TEST_CODE.sqlite",
        ]
    }
    #[test]
    fn wg07_cli_explicit_window_fields_and_legacy_days_default() {
        let a = Args::try_parse_from(args()).unwrap();
        validate(&a).unwrap();
        let r = window_request(&a).unwrap();
        assert_eq!(r.instrument.code(), "600519");
        assert_eq!(r.from.to_string(), "2026-09-11");
        assert_eq!(r.source_profile, "TEST_CODE_SYNTHETIC@1");
        assert!(a.days.is_none());
        let a = Args::try_parse_from(["confirm_daily_change", "--code", "300005"]).unwrap();
        assert_eq!(a.days.unwrap_or(60), 60);
        for extra in [
            vec!["--days", "60"],
            vec!["--confirm"],
            vec!["--candidate-id", "x"],
            vec!["--renew"],
        ] {
            let mut input = args();
            input.extend(extra);
            assert!(Args::try_parse_from(input).is_err());
        }
    }
    #[tokio::test]
    async fn wg07_cli_calls_public_prepare_owner_without_creating_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.sqlite");
        let mut a = Args::try_parse_from(args()).unwrap();
        a.database = Some(path.clone());
        let error = run(a, &mut Vec::new()).await.unwrap_err().to_string();
        assert!(error.contains("UnsupportedProfileOrVersion"));
        assert!(!path.exists());
    }
}
