//! 持仓桥单次同步：拉 Windows 侧导出快照 → 写本地确认快照 → 对齐投影。
//!
//! 调用方：[`crate::market_loop`] 的 15:05 / 16:05 调度块。
//! 逻辑与 CLI `holdings_bridge_sync` 一致，区别只是错误用 `Result` 返回
//! 而不是打印。
//!
//! ## 三条边界
//!
//! 1. 数据源是 TDX「资金股份」导出文件，不是券商 API；桥只读、不刷新。
//! 2. `effective_at` 用桥的 `snapshot_at`（导出时刻），不是现在——
//!    否则一份两小时前的导出会冒充「现在的持仓」。
//! 3. 过期（`age_seconds` 超阈值）或空持仓一律拒写：写入一份陈旧快照
//!    会顶掉当天真实的确认快照，比不写更糟。

use stock_analysis::database;
use stock_analysis::monitor::holdings_bridge;
use stock_analysis::portfolio::user_position_snapshot::user_position_snapshot_input_from_json;

pub struct SyncReport {
    pub snapshot_at: String,
    pub age_seconds: f64,
    pub positions: usize,
    pub inserted: bool,
}

/// 拉一次并落库。任何一步失败都原样返回原因，由调用方决定是否重试。
///
/// 异步：HTTP 走 `SHARED_HTTP_CLIENT`。SQLite 部分仍是同步调用——写入只有
/// 几行，阻塞时间在毫秒级，不需要再套一层 `spawn_blocking`。
pub async fn sync_once(base_url: &str, token_path: &str) -> Result<SyncReport, String> {
    let token = std::fs::read_to_string(token_path)
        .map_err(|error| format!("读取 token {token_path} 失败: {error}"))?
        .trim()
        .to_string();
    if token.is_empty() {
        return Err(format!("token 文件 {token_path} 为空"));
    }

    let holdings = holdings_bridge::fetch_async(base_url, &token).await?;
    // 空持仓或过期快照直接拒写。
    holdings_bridge::accept(&holdings)?;

    let json = holdings_bridge::to_snapshot_json(&holdings);
    let confirmed_at = chrono::Local::now().fixed_offset();
    let input = user_position_snapshot_input_from_json(&json, confirmed_at)
        .map_err(|error| format!("快照 JSON 不合法: {error}"))?;

    let receipt = database::user_position_snapshot::save_user_position_snapshot(&input)
        .map_err(|error| format!("保存快照失败: {error}"))?;

    database::reconcile_stock_position_from_confirmed_snapshot()
        .map_err(|error| format!("对齐 stock_position 失败: {error}"))?;

    // 买入增量落 trades（幂等）。失败不回滚快照——快照与投影已经写入，
    // 流水缺一条可以补，把已成功的两半撤销反而更糟。
    let effective = input.effective_at.format("%Y-%m-%d %H:%M:%S").to_string();
    if let Err(error) =
        stock_analysis::monitor::trade_ledger::record_snapshot_buys(
            database::DatabaseManager::get(),
            &effective,
        )
    {
        log::warn!("[持仓桥] trade_ledger 推断失败（快照与投影已写入）: {error}");
    }

    Ok(SyncReport {
        snapshot_at: holdings.snapshot_at,
        age_seconds: holdings.age_seconds,
        positions: holdings.positions.len(),
        inserted: receipt.inserted,
    })
}
