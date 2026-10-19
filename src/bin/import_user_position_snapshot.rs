//! BR-146: one-shot importer for a user-confirmed complete position snapshot.
//! BR-215: after a confirmed snapshot lands, the local `stock_position`
//! projection is reconciled from it so the two stop drifting apart.
use clap::Parser;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use stock_analysis::database::{self, DatabaseManager};
use stock_analysis::portfolio::user_position_snapshot::user_position_snapshot_input_from_json;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    database: PathBuf,
    /// Confirmed snapshot JSON. Omit together with `--reconcile-only` to
    /// reconcile from the snapshot already stored in the database.
    #[arg(long, required_unless_present = "reconcile_only")]
    snapshot: Option<PathBuf>,
    /// BR-215: skip the import and only reconcile `stock_position` from the
    /// latest already-confirmed snapshot. Re-importing an existing snapshot
    /// with a fresh `confirmed_at` would fabricate a new confirmation, so this
    /// is the only supported way to reconcile after the fact.
    #[arg(long)]
    reconcile_only: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if args.reconcile_only {
        if args.snapshot.is_some() {
            return Err("--reconcile-only must not be combined with --snapshot".into());
        }
        DatabaseManager::init(Some(args.database))?;
        let reconciliation = database::reconcile_stock_position_from_confirmed_snapshot()?;
        println!(
            "reconcile_only snapshot_id={} updated={} inserted={} unchanged={} unconfirmed_open={:?}",
            reconciliation.snapshot_id,
            reconciliation.updated,
            reconciliation.inserted,
            reconciliation.unchanged,
            reconciliation.unconfirmed_open
        );
        return Ok(());
    }

    let snapshot_path = args.snapshot.expect("clap enforces snapshot presence");
    let metadata = std::fs::metadata(&snapshot_path)?;
    if !metadata.is_file() || metadata.len() > 1_048_576 {
        return Err("snapshot must be a regular UTF-8 JSON file no larger than 1 MiB".into());
    }
    let json = std::fs::read_to_string(&snapshot_path)?;
    let confirmed_at = chrono::Local::now().fixed_offset();
    let input = user_position_snapshot_input_from_json(&json, confirmed_at)?;
    DatabaseManager::init(Some(args.database))?;
    let receipt = database::user_position_snapshot::save_user_position_snapshot(&input)?;
    let mut receipt_hash = Sha256::new();
    receipt_hash.update(b"stock_analysis.import_user_position_snapshot.receipt.v1\0");
    receipt_hash.update(input.evidence_sha256.as_bytes());
    println!(
        "snapshot_id_hash={:x} inserted={} item_count={}",
        receipt_hash.finalize(),
        receipt.inserted,
        input.items.len()
    );
    // BR-215: a confirmed snapshot is the only authority for the projection.
    // Failure stays explicit — the import receipt above is already printed, so
    // the operator can see exactly which half succeeded.
    let reconciliation = database::reconcile_stock_position_from_confirmed_snapshot()?;
    println!(
        "reconciled snapshot_id={} updated={} inserted={} unchanged={} unconfirmed_open={:?}",
        reconciliation.snapshot_id,
        reconciliation.updated,
        reconciliation.inserted,
        reconciliation.unchanged,
        reconciliation.unconfirmed_open
    );

    // 交易流水：把这份快照相对上一份的买入增量落 `trades`（幂等）。
    //
    // 只推断买入——快照里没有卖出价，用成本价冒充会让盈亏归因全错（BR-222）。
    // 卖出的票列出来，由操作者用 `trade_record` 补录真实成交价。
    let effective = input.effective_at.format("%Y-%m-%d %H:%M:%S").to_string();
    match stock_analysis::monitor::trade_ledger::record_snapshot_buys(
        DatabaseManager::get(),
        &effective,
    ) {
        Ok(report) => {
            report.log();
            println!(
                "trade_ledger inferred_buys={} sells_needing_manual_price={} already_recorded={}",
                report.inferred_buys,
                report.sells_needing_manual_price.len(),
                report.already_recorded
            );
        }
        Err(error) => {
            // 流水推断失败不该让「快照已落库」这件事看起来失败，但必须出声。
            eprintln!("trade_ledger 推断失败（快照与持仓投影已成功写入）: {error}");
        }
    }
    Ok(())
}
