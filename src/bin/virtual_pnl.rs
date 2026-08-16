//! 虚拟持仓收益快照 — 当前市值 / 浮动盈亏 / 年化收益。
//!
//! 数据: paper_trades (Filled buy 聚合持仓) + TDX 5min 线最近一根 bar 收盘价
//! (非交易时段 = 上一交易日收盘)。年化 = 浮动盈亏 / 成本 / 持有天数 * 365。
//! 只读, 无写入、无推送。

#[cfg(feature = "magic-gateway")]
use magic_tdx_rs::protocol::constants::{fq_type, KLINE_5MIN};
#[cfg(feature = "magic-gateway")]
use magic_tdx_rs::TdxHqClient;

fn main() {
    stock_analysis::database::DatabaseManager::init(Some(
        "/Users/zhangzhen/Desktop/Quant/stock_analysis/data/stock_analysis.db".into(),
    ))
    .expect("production db init");

    use diesel::query_dsl::RunQueryDsl;
    #[derive(diesel::QueryableByName)]
    struct HoldingRow {
        #[diesel(sql_type = diesel::sql_types::Text)]
        code: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        name: String,
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        net_qty: i64,
        #[diesel(sql_type = diesel::sql_types::Double)]
        buy_cost: f64,
    }
    let db = stock_analysis::database::DatabaseManager::get();
    let mut conn = db.get_conn().expect("conn");
    // GROUP BY code 而非 (code,name): 7/14-16 Breakout 价格异常期 name 字段在
    // "沪电股份"/"002463" 间抖动, 按 name 分组会把同一持仓拆成两行。
    let rows: Vec<HoldingRow> = diesel::sql_query(
        "SELECT code, MAX(name) as name,
                SUM(CASE WHEN direction='buy' THEN quantity ELSE -quantity END) as net_qty,
                SUM(CASE WHEN direction='buy' THEN quantity*price ELSE 0 END) as buy_cost
         FROM paper_trades WHERE status='Filled'
         GROUP BY code HAVING net_qty > 0 ORDER BY buy_cost DESC",
    )
    .load::<HoldingRow>(&mut conn)
    .expect("paper holdings read");

    let client = TdxHqClient::new();
    client.connect_to_any(Some(5.0)).expect("tdx connect");

    let mut total_cost = 0.0f64;
    let mut total_mv = 0.0f64;
    let mut with_price = 0usize;
    // avg_cost < 1.0 元 = 7/14-16 Breakout 价格 feed 异常期的破损成交价 (真实
    // A 股价格不可能低于 1 元), 标注 ⚠️ 使污染持仓可见, 不静默计入汇总口径。
    let avg_cost = |cost: f64, qty: i64| if qty > 0 { cost / qty as f64 } else { 0.0 };
    println!("{:4} {:8} {:<10} {:>10} {:>10} {:>10} {:>8} {:>4}", "#", "代码", "名称", "持仓", "成本", "市值", "浮盈亏%", "注");
    for (i, row) in rows.iter().enumerate() {
        let market = if row.code.starts_with('6') { 1u8 } else { 0u8 };
        let close = client
            .get_security_bars(KLINE_5MIN, market, &row.code, 0, 400, fq_type::NONE)
            .ok()
            .and_then(|bars| bars.last().map(|b| b.close));
        let broken = avg_cost(row.buy_cost, row.net_qty) < 1.0;
        total_cost += row.buy_cost;
        match close {
            Some(price) => {
                let mv = price * row.net_qty as f64;
                total_mv += mv;
                with_price += 1;
                let pnl = (mv / row.buy_cost - 1.0) * 100.0;
                println!(
                    "{:4} {:8} {:<10} {:>10} {:>10.0} {:>10.0} {:>+7.2}%  {}",
                    i + 1,
                    row.code,
                    row.name,
                    row.net_qty,
                    row.buy_cost,
                    mv,
                    pnl,
                    if broken { "⚠️" } else { "" }
                );
            }
            None => {
                println!(
                    "{:4} {:8} {:<10} {:>10} {:>10.0} {:>10} {:>8}  {}",
                    i + 1,
                    row.code,
                    row.name,
                    row.net_qty,
                    row.buy_cost,
                    "-",
                    "无报价",
                    if broken { "⚠️" } else { "" }
                );
            }
        }
    }

    // 年化: 建仓期 2026-07-10 ~ 07-16, 以最早建仓日计持有天数至最近收盘日
    let first_date = chrono::NaiveDate::from_ymd_opt(2026, 7, 10).expect("date");
    let last_date = chrono::Local::now().date_naive();
    let days = (last_date - first_date).num_days().max(1) as f64;
    let float_pnl = total_mv - total_cost;
    let annualized = if total_cost > 0.0 {
        (float_pnl / total_cost) / days * 365.0 * 100.0
    } else {
        0.0
    };
    println!("\n══════════════════════════════════════");
    println!("持仓数: {} ({} 只有效报价)", rows.len(), with_price);
    println!("总成本: {:>12.0} 元", total_cost);
    println!("当前市值: {:>10.0} 元 (以最近交易日收盘计)", total_mv);
    println!(
        "浮动盈亏: {:>+10.0} 元 ({:+.2}%)",
        float_pnl,
        float_pnl / total_cost * 100.0
    );
    println!("持有天数: {} 天 (2026-07-10 起)", days as i64);
    println!("年化收益: {:+.2}%", annualized);
    println!("(无已实现盈亏: 虚拟盘 325 笔 Filled 全部为买入, 0 笔卖出)");
}
