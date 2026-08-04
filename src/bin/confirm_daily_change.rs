//! Explicit operator entry point for BR-171 daily-change confirmation.

use std::io::{self, Write};
use std::path::PathBuf;

use chrono::{Local, NaiveDate};
use clap::Parser;
use serde::Serialize;
use stock_analysis::data_gateway::historical_bars::HistoricalBarsGateway;
use stock_analysis::database::daily_change_confirmation::{
    daily_change_review_token_v2, DailyChangeConfirmationInput, DailyChangeConfirmationQuery,
};
use stock_analysis::database::DatabaseManager;

#[derive(Debug, Parser)]
#[command(
    name = "confirm_daily_change",
    about = "Review exact BR-171 daily-change evidence; append only with explicit confirmation"
)]
struct Args {
    #[arg(long)]
    code: String,
    #[arg(long, default_value_t = 60)]
    days: usize,
    #[arg(long)]
    confirm: bool,
    #[arg(long)]
    previous_date: Option<NaiveDate>,
    #[arg(long)]
    current_date: Option<NaiveDate>,
    #[arg(long)]
    evidence_token: Option<String>,
    #[arg(long)]
    database: Option<PathBuf>,
    #[arg(long)]
    operator: Option<String>,
    #[arg(long)]
    reason: Option<String>,
}

#[derive(Debug)]
enum ValidatedAction {
    Review,
    Confirm(ConfirmationRequest),
}

#[derive(Debug)]
struct ConfirmationRequest {
    previous_date: NaiveDate,
    current_date: NaiveDate,
    evidence_token: String,
    operator: String,
    reason: String,
}

fn required_exact_text<'a>(flag: &str, value: Option<&'a str>) -> Result<&'a str, String> {
    let value = value.ok_or_else(|| format!("{flag} is required with --confirm"))?;
    if value.is_empty() || value.trim() != value {
        return Err(format!(
            "{flag} must be non-empty and contain no surrounding whitespace"
        ));
    }
    Ok(value)
}

fn validated_action(args: &Args) -> Result<ValidatedAction, String> {
    if args.code.trim() != args.code
        || args.code.len() != 6
        || !args.code.bytes().all(|byte| byte.is_ascii_digit())
        || !matches!(
            args.code.as_bytes().first(),
            Some(b'0' | b'2' | b'3' | b'4' | b'6' | b'8' | b'9')
        )
    {
        return Err(format!(
            "--code must be one canonical six-digit A-share code, got {:?}",
            args.code
        ));
    }
    if !(2..=usize::from(u16::MAX)).contains(&args.days) {
        return Err(format!(
            "--days must be in 2..={}, got {}",
            u16::MAX,
            args.days
        ));
    }
    if !args.confirm {
        if args.previous_date.is_some()
            || args.current_date.is_some()
            || args.evidence_token.is_some()
            || args.operator.is_some()
            || args.reason.is_some()
        {
            return Err(
                "confirmation-only fields require explicit --confirm; review mode does not append a confirmation"
                    .to_string(),
            );
        }
        return Ok(ValidatedAction::Review);
    }
    let operator = required_exact_text("--operator", args.operator.as_deref())?.to_string();
    let reason = required_exact_text("--reason", args.reason.as_deref())?.to_string();
    let evidence_token =
        required_exact_text("--evidence-token", args.evidence_token.as_deref())?.to_string();
    if evidence_token.len() != 64
        || !evidence_token
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("--evidence-token must be one lowercase 64-character SHA-256 hex value".into());
    }
    args.database
        .as_ref()
        .ok_or_else(|| "--database is required with --confirm".to_string())?;
    let previous_date = args
        .previous_date
        .ok_or_else(|| "--previous-date is required with --confirm".to_string())?;
    let current_date = args
        .current_date
        .ok_or_else(|| "--current-date is required with --confirm".to_string())?;
    if previous_date >= current_date {
        return Err("--previous-date must precede --current-date".to_string());
    }
    Ok(ValidatedAction::Confirm(ConfirmationRequest {
        previous_date,
        current_date,
        evidence_token,
        operator,
        reason,
    }))
}

#[derive(Serialize)]
struct ReviewEvidence<'a> {
    schema_version: u8,
    code: &'a str,
    previous_date: String,
    current_date: String,
    previous_close: &'a str,
    current_close: &'a str,
    calculated_pct: &'a str,
    daily_provider: &'a str,
    daily_source: &'a str,
    daily_batch_id: &'a str,
    lifecycle_provider: &'a str,
    lifecycle_batch_id: &'a str,
    listing_date: Option<String>,
    corporate_action_identity: Option<&'a str>,
}

fn review_evidence(query: &DailyChangeConfirmationQuery) -> ReviewEvidence<'_> {
    ReviewEvidence {
        schema_version: 2,
        code: &query.code,
        previous_date: query.previous_date.to_string(),
        current_date: query.current_date.to_string(),
        previous_close: &query.previous_close,
        current_close: &query.current_close,
        calculated_pct: &query.calculated_pct,
        daily_provider: &query.daily_provider,
        daily_source: &query.daily_source,
        daily_batch_id: &query.daily_batch_id,
        lifecycle_provider: &query.lifecycle_provider,
        lifecycle_batch_id: &query.lifecycle_batch_id,
        listing_date: query.listing_date.map(|date| date.to_string()),
        corporate_action_identity: query.corporate_action_identity.as_deref(),
    }
}

fn evidence_token(query: &DailyChangeConfirmationQuery) -> Result<String, String> {
    daily_change_review_token_v2(query)
}

#[derive(Serialize)]
struct ReviewOutput<'a> {
    #[serde(flatten)]
    evidence: ReviewEvidence<'a>,
    evidence_token: String,
}

fn render_review_query(query: &DailyChangeConfirmationQuery) -> Result<String, String> {
    let output = ReviewOutput {
        evidence: review_evidence(query),
        evidence_token: evidence_token(query)?,
    };
    serde_json::to_string(&output)
        .map_err(|error| format!("cannot render BR-171 review evidence: {error}"))
}

fn select_confirmation_query<'a>(
    action: &ValidatedAction,
    queries: &'a [DailyChangeConfirmationQuery],
) -> Result<&'a DailyChangeConfirmationQuery, String> {
    let ValidatedAction::Confirm(request) = action else {
        return Err("BR-171 review mode cannot select a confirmation".to_string());
    };
    let mut matches = queries.iter().filter(|query| {
        query.previous_date == request.previous_date && query.current_date == request.current_date
    });
    let query = matches.next().ok_or_else(|| {
        format!(
            "no pending BR-171 change matches {} -> {}",
            request.previous_date, request.current_date
        )
    })?;
    if matches.next().is_some() {
        return Err(format!(
            "multiple pending BR-171 changes match {} -> {}",
            request.previous_date, request.current_date
        ));
    }
    let current_token = evidence_token(query)?;
    if current_token != request.evidence_token {
        return Err(format!(
            "BR-171 evidence changed; reviewed token={} current token={}; run review again",
            request.evidence_token, current_token
        ));
    }
    Ok(query)
}

fn write_json_line(value: &str) -> anyhow::Result<()> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    output.write_all(value.as_bytes())?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}

async fn run(args: Args) -> anyhow::Result<()> {
    let action = validated_action(&args).map_err(anyhow::Error::msg)?;
    DatabaseManager::init(args.database.clone())
        .map_err(|error| anyhow::anyhow!("BR-171 review database init: {error}"))?;
    let queries = HistoricalBarsGateway::new()
        .pending_daily_change_confirmations_async(&args.code, args.days)
        .await?;

    match &action {
        ValidatedAction::Review => {
            if queries.is_empty() {
                write_json_line(
                    &serde_json::json!({
                        "status": "no_pending_confirmation",
                        "code": args.code,
                    })
                    .to_string(),
                )?;
            } else {
                for query in &queries {
                    write_json_line(&render_review_query(query).map_err(anyhow::Error::msg)?)?;
                }
            }
            Ok(())
        }
        ValidatedAction::Confirm(request) => {
            let query = select_confirmation_query(&action, &queries)
                .map_err(anyhow::Error::msg)?
                .clone();
            // The exact source/lifecycle evidence is printed before any
            // durable write. Broken stdout therefore blocks confirmation.
            write_json_line(&render_review_query(&query).map_err(anyhow::Error::msg)?)?;

            let receipt = DatabaseManager::get()
                .append_daily_change_confirmation(&DailyChangeConfirmationInput {
                    query,
                    operator_identity: request.operator.clone(),
                    reason: request.reason.clone(),
                    confirmed_at: Local::now().fixed_offset(),
                })
                .map_err(anyhow::Error::msg)?;
            write_json_line(
                &serde_json::json!({
                    "status": if receipt.inserted {
                        "confirmation_appended"
                    } else {
                        "confirmation_already_present"
                    },
                    "confirmation_id": receipt.confirmation_id,
                    "query_identity_hash": receipt.query_identity_hash,
                    "record_hash": receipt.record_hash,
                    "inserted": receipt.inserted,
                })
                .to_string(),
            )
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run(Args::parse()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    type ConfirmationMutation = Box<dyn Fn(&mut DailyChangeConfirmationQuery)>;
    fn query() -> DailyChangeConfirmationQuery {
        DailyChangeConfirmationQuery {
            // The production validator requires the canonical provider identity.
            // This unit performs no provider/database/account/order/sink I/O.
            code: "600396".to_string(),
            previous_date: NaiveDate::from_ymd_opt(2026, 7, 23).unwrap(),
            current_date: NaiveDate::from_ymd_opt(2026, 7, 24).unwrap(),
            previous_close: "10".to_string(),
            current_close: "13".to_string(),
            calculated_pct: "30".to_string(),
            daily_provider: "TEST_CODE_magic_tdx".to_string(),
            daily_source: "TEST_CODE_tdx-smart".to_string(),
            daily_batch_id: "TEST_CODE_daily_batch".to_string(),
            lifecycle_provider: "TEST_CODE_magic_tdx".to_string(),
            lifecycle_batch_id: "TEST_CODE_lifecycle_batch".to_string(),
            listing_date: Some(NaiveDate::from_ymd_opt(2010, 1, 1).unwrap()),
            corporate_action_identity: Some("TEST_CODE_action_identity".to_string()),
        }
    }

    #[test]
    fn cli_defaults_to_read_only_and_rejects_incomplete_confirmation() {
        let read_only =
            Args::try_parse_from(["confirm_daily_change", "--code", "600396"]).expect("parse");
        assert!(matches!(
            validated_action(&read_only).expect("read-only action"),
            ValidatedAction::Review
        ));
        let read_only_with_database = Args::try_parse_from([
            "confirm_daily_change",
            "--code",
            "600396",
            "--database",
            "/tmp/TEST_CODE_br171_review.db",
        ])
        .expect("review database parses");
        assert!(matches!(
            validated_action(&read_only_with_database).expect("read-only database action"),
            ValidatedAction::Review
        ));

        let incomplete =
            Args::try_parse_from(["confirm_daily_change", "--code", "600396", "--confirm"])
                .expect("shape parses");
        assert!(validated_action(&incomplete)
            .expect_err("confirmation fields are mandatory")
            .contains("--operator"));
    }

    #[test]
    fn evidence_token_v2_binds_stable_fact_but_not_acquisition_batch_ids() {
        let original = query();
        let original_token = evidence_token(&original).expect("evidence token");
        assert_eq!(original_token.len(), 64);

        let mut rotated_batches = original.clone();
        rotated_batches.daily_batch_id = "TEST_CODE_daily_batch_reacquired".to_string();
        rotated_batches.lifecycle_batch_id = "TEST_CODE_lifecycle_batch_reacquired".to_string();
        assert_eq!(
            evidence_token(&rotated_batches).expect("rotated batch token"),
            original_token,
            "acquisition-time batch rotation must not change the reviewed fact"
        );

        let mut changes: Vec<ConfirmationMutation> = vec![
            Box::new(|query| query.code = "600397".to_string()),
            Box::new(|query| query.previous_date = query.previous_date.pred_opt().unwrap()),
            Box::new(|query| query.current_date = query.current_date.succ_opt().unwrap()),
            Box::new(|query| {
                query.previous_close = "10.4".to_string();
                query.calculated_pct = "25".to_string();
            }),
            Box::new(|query| {
                query.current_close = "13.1".to_string();
                query.calculated_pct = "31".to_string();
            }),
            Box::new(|query| query.daily_provider.push_str("_changed")),
            Box::new(|query| query.daily_source.push_str("_changed")),
            Box::new(|query| query.lifecycle_provider.push_str("_changed")),
            Box::new(|query| {
                query.listing_date = Some(query.listing_date.unwrap().succ_opt().unwrap())
            }),
            Box::new(|query| query.corporate_action_identity = None),
        ];
        for change in changes.drain(..) {
            let mut changed = original.clone();
            change(&mut changed);
            assert_ne!(
                evidence_token(&changed).expect("changed evidence token"),
                original_token
            );
        }
    }

    #[test]
    fn explicit_confirmation_selects_only_the_reviewed_exact_evidence() {
        let query = query();
        let token = evidence_token(&query).expect("evidence token");
        let args = Args::try_parse_from([
            "confirm_daily_change",
            "--code",
            "600396",
            "--confirm",
            "--previous-date",
            "2026-07-23",
            "--current-date",
            "2026-07-24",
            "--evidence-token",
            &token,
            "--database",
            "/tmp/TEST_CODE_br171.db",
            "--operator",
            "TEST_CODE_operator",
            "--reason",
            "TEST_CODE reviewed provider evidence",
        ])
        .expect("explicit confirmation CLI");
        let action = validated_action(&args).expect("validated confirmation");
        let queries = vec![query.clone()];
        assert_eq!(
            select_confirmation_query(&action, &queries)
                .expect("exact reviewed query")
                .daily_batch_id,
            query.daily_batch_id
        );

        let replacement = if token.starts_with('a') { "b" } else { "a" };
        let changed_args = Args {
            evidence_token: Some(format!("{replacement}{}", &token[1..])),
            ..args
        };
        let changed_action = validated_action(&changed_args).expect("shape remains valid");
        assert!(select_confirmation_query(&changed_action, &queries)
            .expect_err("changed token must fail closed")
            .contains("evidence changed"));
    }

    #[test]
    fn cli_rejects_invalid_scope_and_confirmation_fields_in_review_mode() {
        for argv in [
            vec!["confirm_daily_change", "--code", "TEST_CODE_600396"],
            vec!["confirm_daily_change", "--code", "600396", "--days", "1"],
            vec![
                "confirm_daily_change",
                "--code",
                "600396",
                "--operator",
                "TEST_CODE_operator",
            ],
        ] {
            let args = Args::try_parse_from(argv).expect("CLI shape");
            assert!(
                validated_action(&args).is_err(),
                "invalid or ambiguous CLI must fail closed: {args:?}"
            );
        }
    }

    #[test]
    fn review_output_is_machine_readable_and_contains_all_exact_evidence() {
        let query = query();
        let rendered = render_review_query(&query).expect("review JSON");
        let value: serde_json::Value = serde_json::from_str(&rendered).expect("valid JSON");
        assert_eq!(value["code"], "600396");
        assert_eq!(value["previous_date"], "2026-07-23");
        assert_eq!(value["current_date"], "2026-07-24");
        assert_eq!(value["previous_close"], "10");
        assert_eq!(value["current_close"], "13");
        assert_eq!(value["calculated_pct"], "30");
        assert_eq!(value["daily_provider"], "TEST_CODE_magic_tdx");
        assert_eq!(value["daily_source"], "TEST_CODE_tdx-smart");
        assert_eq!(value["daily_batch_id"], "TEST_CODE_daily_batch");
        assert_eq!(value["lifecycle_provider"], "TEST_CODE_magic_tdx");
        assert_eq!(value["lifecycle_batch_id"], "TEST_CODE_lifecycle_batch");
        assert_eq!(value["listing_date"], "2010-01-01");
        assert_eq!(
            value["corporate_action_identity"],
            "TEST_CODE_action_identity"
        );
        assert_eq!(
            value["evidence_token"],
            evidence_token(&query).expect("token")
        );
    }
}
