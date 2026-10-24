//! 逻辑档案录入（0.17）：给活跃 ticket 记一句话逻辑 / 证伪条件 / 兑现节点。
//!
//!   STOCK_DB=<db> ticket_logic --code 600000 --date 2026-10-24 \
//!       --logic "固态电池硫化物路线定点" --falsify "客户验证未通过" --due 2026-11-15
//!
//! 直接开 diesel 连接（不走 DatabaseManager::init 的审计链），只写 `ticket_event`。
//! 逻辑强度打分**有意不做**——没有历史分布校准，分数只会变成自我说服。

use diesel::Connection as _;
use diesel::SqliteConnection;

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "ticket_logic --code <6位> --date YYYY-MM-DD --logic <一句话> [--falsify <证伪条件>] [--due YYYY-MM-DD]"
        );
        return Ok(());
    }
    let code = arg_value(&args, "--code").ok_or("缺少 --code")?;
    let date = arg_value(&args, "--date")
        .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());
    let logic = arg_value(&args, "--logic").ok_or("缺少 --logic")?;
    let falsify = arg_value(&args, "--falsify").unwrap_or_default();
    let due = arg_value(&args, "--due");
    let db_path =
        std::env::var("STOCK_DB").unwrap_or_else(|_| "data/stock_analysis.db".to_string());
    let mut conn = SqliteConnection::establish(&format!("sqlite://{db_path}"))?;
    let ticket_id = stock_analysis::database::ticket_lifecycle::record_logic(
        &mut conn,
        &code,
        &date,
        &logic,
        &falsify,
        due.as_deref(),
    )?;
    println!("已记录逻辑档案: ticket#{ticket_id} {code} {date}");
    Ok(())
}
