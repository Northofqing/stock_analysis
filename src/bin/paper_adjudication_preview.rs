//! Explicit read-only operator preview. Applying a ruling is a separate Task10 action.
use std::path::PathBuf;
use std::process::ExitCode;
use stock_analysis::database::attribution_reports::{
    AttributionDatabaseAccess, AttributionDatabaseSession,
};
use stock_analysis::trading::paper_ledger::{Adjudication, AdjudicationPreview, PaperLedger};

#[derive(Debug, PartialEq, Eq)]
struct Args {
    database: PathBuf,
    manifest: PathBuf,
}
fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut args = args.into_iter();
    let mut database = None;
    let mut manifest = None;
    while let Some(arg) = args.next() {
        let slot = match arg.as_str() {
            "--db" => &mut database,
            "--manifest" => &mut manifest,
            _ => {
                return Err(format!(
                    "不支持参数 {arg}；本工具仅 preview，apply 需要独立运维授权"
                ))
            }
        };
        if slot.is_some() {
            return Err(format!("重复参数 {arg}"));
        }
        let value = args
            .next()
            .filter(|value| !value.trim().is_empty() && !value.starts_with("--"))
            .ok_or_else(|| format!("{arg} 必须指定路径"))?;
        *slot = Some(PathBuf::from(value));
    }
    Ok(Args {
        database: database.ok_or("必须显式指定 --db")?,
        manifest: manifest.ok_or("必须显式指定 --manifest")?,
    })
}
fn run(args: Args) -> Result<String, String> {
    let bytes =
        std::fs::read(&args.manifest).map_err(|error| format!("读取裁定清单失败: {error}"))?;
    let request: Adjudication = serde_json::from_slice(&bytes)
        .map_err(|error| format!("裁定清单不符合完整版本化合同: {error}"))?;
    let session =
        AttributionDatabaseSession::open(&args.database, AttributionDatabaseAccess::ReadOnly)
            .map_err(|error| format!("只读数据库资格失败: {error:?}"))?;
    let preview = PaperLedger::open(session.database(), &chrono::Utc::now)
        .preview_adjudication(&request)
        .map_err(|error| error.to_string())?;
    format_preview(&request, &preview)
}
fn format_preview(request: &Adjudication, preview: &AdjudicationPreview) -> Result<String, String> {
    serde_json::to_string_pretty(&serde_json::json!({"mode":"preview_only","applied":false,"money_model":stock_analysis::trading::paper_ledger::MONEY_MODEL,"account":request.binding,"original":request.original,"expected_version":request.expected_version,"expected_head":request.expected_head,"expected_predecessor":request.expected_predecessor,"preview":preview})).map_err(|error|error.to_string())
}
fn main() -> ExitCode {
    match parse_args(std::env::args().skip(1)).and_then(run) {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("裁定预览失败: {error}");
            ExitCode::FAILURE
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Result<Args, String> {
        parse_args(values.iter().map(|value| (*value).into()))
    }
    #[test]
    fn output_preserves_historical_dependency_failure_without_claiming_current_cash_failure() {
        let request: Adjudication = serde_json::from_value(serde_json::json!({
            "binding":{"account_id":"TEST_CODE_account","epoch_id":"TEST_CODE_epoch","manifest_hash":"a".repeat(64)},
            "request_id":"TEST_CODE_preview", "expected_version":1, "expected_head":"b".repeat(64),
            "expected_predecessor":null,
            "original":{"paper_trade_id":1,"plan_id":"TEST_CODE_legacy","event_hash":"", "raw_trade_hash":"c".repeat(64),"audit_hash":"","fact_at":"2026-07-10T02:00:00Z","legacy_before_cutover":true,"legacy_no_terminal":true},
            "action":"Quarantine","reason":"TEST_CODE_reason","evidence":"TEST_CODE_evidence",
            "operator":"TEST_CODE_operator","source":"TEST_CODE_manifest","decision_at":"2026-09-14T02:00:00Z"
        })).unwrap();
        let preview: AdjudicationPreview = serde_json::from_value(serde_json::json!({
            "current_account":{"changed":false,"projection_hash":"d".repeat(64),"cash":100000000000_i64,"fees":0,"unavailable":null},
            "historical_scope":{"scope":{"LegacyBeforeCutover":request.binding},"identity_version":"LegacyEconomicV1","before_hash":"e".repeat(64),"after_hash":"f".repeat(64),"unavailable":"TEST_CODE dependent sell exceeds inventory"}
        })).unwrap();
        let output: serde_json::Value =
            serde_json::from_str(&format_preview(&request, &preview).unwrap()).unwrap();
        assert_eq!(output["mode"], "preview_only");
        assert_eq!(output["applied"], false);
        assert_eq!(
            output["preview"]["current_account"]["cash"],
            100000000000_i64
        );
        assert!(output["preview"]["current_account"]["unavailable"].is_null());
        assert_eq!(
            output["preview"]["historical_scope"]["unavailable"],
            "TEST_CODE dependent sell exceeds inventory"
        );
        assert_eq!(
            output["preview"]["historical_scope"]["scope"]["LegacyBeforeCutover"],
            output["account"]
        );
        assert_ne!(
            output["preview"]["historical_scope"]["before_hash"],
            output["preview"]["historical_scope"]["after_hash"]
        );
    }
    #[test]
    fn explicit_paths_only_and_no_apply_authority() {
        assert_eq!(
            args(&["--db", "TEST_CODE.db", "--manifest", "TEST_CODE.json"]).unwrap(),
            Args {
                database: "TEST_CODE.db".into(),
                manifest: "TEST_CODE.json".into()
            }
        );
        for values in [
            vec![],
            vec!["--db", "TEST_CODE.db"],
            vec!["--db", "--manifest", "TEST_CODE.json"],
            vec![
                "--db",
                "TEST_CODE.db",
                "--db",
                "OTHER.db",
                "--manifest",
                "TEST_CODE.json",
            ],
            vec![
                "--db",
                "TEST_CODE.db",
                "--manifest",
                "TEST_CODE.json",
                "--apply",
            ],
        ] {
            assert!(args(&values).is_err());
        }
    }
}
