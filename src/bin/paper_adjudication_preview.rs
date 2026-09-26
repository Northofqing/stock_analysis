//! Explicit read-only operator preview. Applying a ruling is a separate Task10 action.
use std::path::PathBuf;
use std::process::ExitCode;
use stock_analysis::database::attribution_reports::{
    AttributionDatabaseAccess, AttributionDatabaseSession,
};
use stock_analysis::trading::paper_ledger::{Adjudication, PaperLedger};

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
