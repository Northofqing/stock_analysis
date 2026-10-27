//! WP6 单票证据包 CLI（只读生产库）。
use std::path::PathBuf;

use clap::Parser;
use stock_analysis::monitor::ticket_evidence::{build_evidence_pack, EvidenceRequest};

#[derive(Debug, Parser)]
#[command(about = "Build a read-only, hash-manifested evidence pack for one ticket")]
struct Args {
    #[arg(long)]
    db: PathBuf,
    #[arg(long = "delivery-db")]
    delivery_db: PathBuf,
    #[arg(long)]
    code: String,
    #[arg(long = "as-of")]
    as_of: String,
    #[arg(long)]
    out: PathBuf,
    /// 默认是主库同级的 push_log 目录。
    #[arg(long = "push-log-root")]
    push_log_root: Option<PathBuf>,
}

fn run() -> Result<(), String> {
    let args = Args::parse();
    let push_log_root = args.push_log_root.clone().unwrap_or_else(|| {
        args.db
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("push_log")
    });
    let summary = build_evidence_pack(&EvidenceRequest {
        db: args.db,
        delivery_db: args.delivery_db,
        code: args.code,
        as_of: args.as_of,
        out: args.out,
        push_log_root,
    })?;
    println!(
        "{}",
        serde_json::to_string_pretty(&summary).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("ticket_audit: {error}");
        std::process::exit(1);
    }
}
