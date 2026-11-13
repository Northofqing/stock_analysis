//! 交易流水（2026-10-19）。
//!
//! ## 为什么要有这个模块
//!
//! `trades` 表一直存在（方向/价格/股数/金额/手续费/印花税/已实现盈亏/策略标签），
//! 但生产上数据停在 2026-07-13——没有任何代码往里面写。于是「今天买了多少」只能
//! 靠前后两份持仓快照的差额反推，T+1 判断、成本核算、复盘归因都在猜。
//!
//! ## 两条来源，可信度不同
//!
//! - `source='manual'`：用户手工录入的真实成交，价格/时间/手续费都准
//! - `source='snapshot_inferred'`：导入快照时用差额反推的买入。数量准，
//!   时间=导入时刻，价格是按加权成本反推的均价（除权/送股会算歪）
//!
//! ## 卖出不自动写
//!
//! 快照里没有卖出价。任何填法都是编造（BR-222）——用成本价冒充卖出价会让
//! 盈亏归因全错。卖出只能手工补录，或由用户在导入时明确给出成交价。
//!
//! ## 幂等
//!
//! 同一 `snapshot_id` 只推断一次；重复导入快照不会重复记流水。

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Nullable, Text};

use crate::database::DatabaseManager;

/// 一条待写入的交易。
#[derive(Debug, Clone)]
pub struct NewTrade {
    pub code: String,
    pub name: String,
    /// `buy` / `sell`
    pub direction: String,
    pub price: f64,
    pub shares: u64,
    /// RFC3339 或 `YYYY-MM-DD HH:MM:SS`
    pub traded_at: String,
    pub reason: String,
    /// `manual` / `snapshot_inferred`
    pub source: String,
    pub snapshot_id: Option<String>,
}

/// 快照差分推断出的一笔买入。
#[derive(Debug, Clone)]
pub struct InferredBuy {
    pub code: String,
    pub name: String,
    pub price: f64,
    pub shares: u64,
    pub note: String,
}

#[derive(Debug, Default, Clone)]
pub struct SnapshotTradeReport {
    pub snapshot_id: String,
    pub already_recorded: bool,
    pub inferred_buys: usize,
    /// 减仓/清仓的票：卖出价未知，不写流水，只列出来提醒手工补录。
    pub sells_needing_manual_price: Vec<(String, String, i64)>,
    pub errors: Vec<String>,
}

impl SnapshotTradeReport {
    pub fn log(&self) {
        if self.already_recorded {
            log::info!(
                "[交易流水] snapshot={} 已推断过，跳过",
                self.snapshot_id
            );
            return;
        }
        log::info!(
            "[交易流水] snapshot={} 推断买入 {} 笔, 待手工补录卖出 {} 笔",
            self.snapshot_id,
            self.inferred_buys,
            self.sells_needing_manual_price.len()
        );
        for (code, name, delta) in &self.sells_needing_manual_price {
            log::warn!(
                "[交易流水] {code} {name} 减少 {} 股：快照无卖出价，未写流水，请用 trade_record 补录",
                -delta
            );
        }
        for error in &self.errors {
            log::warn!("[交易流水] {error}");
        }
    }
}

/// 补齐 `trades` 的溯源列（幂等）。
///
/// 用 `ALTER TABLE` 而不是 migration 目录：这两列只服务本模块，
/// 列已存在时报错即可忽略。
pub fn ensure_schema(conn: &mut SqliteConnection) -> Result<(), String> {
    let _ = diesel::sql_query(
        "ALTER TABLE trades ADD COLUMN source TEXT NOT NULL DEFAULT 'manual'",
    )
    .execute(conn);
    let _ = diesel::sql_query("ALTER TABLE trades ADD COLUMN snapshot_id TEXT")
        .execute(conn);
    Ok(())
}

/// 写一条交易流水。
pub fn record(db: &DatabaseManager, trade: &NewTrade) -> Result<(), String> {
    if trade.price <= 0.0 || !trade.price.is_finite() {
        return Err(format!("{} 成交价非法: {}", trade.code, trade.price));
    }
    if trade.shares == 0 {
        return Err(format!("{} 成交股数必须为正", trade.code));
    }
    if trade.direction != "buy" && trade.direction != "sell" {
        return Err(format!("方向只能是 buy/sell，收到 {}", trade.direction));
    }
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    ensure_schema(&mut conn)?;
    let amount = trade.price * trade.shares as f64;
    diesel::sql_query(
        "INSERT INTO trades \
          (code, name, direction, price, shares, amount, reason, traded_at, source, snapshot_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind::<Text, _>(trade.code.clone())
    .bind::<Text, _>(trade.name.clone())
    .bind::<Text, _>(trade.direction.clone())
    .bind::<Double, _>(trade.price)
    .bind::<BigInt, _>(trade.shares as i64)
    .bind::<Double, _>(amount)
    .bind::<Text, _>(trade.reason.clone())
    .bind::<Text, _>(trade.traded_at.clone())
    .bind::<Text, _>(trade.source.clone())
    .bind::<Nullable<Text>, _>(trade.snapshot_id.clone())
    .execute(&mut conn)
    .map_err(|e| format!("写流水 {} 失败: {e}", trade.code))?;
    Ok(())
}

/// 当前快照相对上一份快照的**买入**增量。
///
/// 买入价用加权成本反推：
/// `买入价 = (新数量×新成本 − 旧数量×旧成本) / 买入股数`
/// 新建仓时直接等于成本价。
///
/// 反推失效的情形（算出来 <= 0 或非有限）不写，记进 `errors`。
pub fn snapshot_buy_deltas(
    db: &DatabaseManager,
) -> Result<(String, Vec<InferredBuy>, Vec<(String, String, i64)>, Vec<String>), String> {
    #[derive(QueryableByName)]
    struct SnapRow {
        #[diesel(sql_type = Text)]
        snapshot_id: String,
    }
    #[derive(QueryableByName)]
    struct ItemRow {
        #[diesel(sql_type = Text)]
        code: String,
        #[diesel(sql_type = Text)]
        name: String,
        #[diesel(sql_type = BigInt)]
        quantity: i64,
        #[diesel(sql_type = Double)]
        cost_price: f64,
    }

    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    let snaps: Vec<SnapRow> = diesel::sql_query(
        "SELECT snapshot_id FROM user_position_snapshot \
         ORDER BY effective_at DESC, rowid DESC LIMIT 2",
    )
    .load(&mut conn)
    .map_err(|e| e.to_string())?;
    if snaps.len() < 2 {
        return Ok((String::new(), Vec::new(), Vec::new(), Vec::new()));
    }
    let current_id = snaps[0].snapshot_id.clone();

    let load = |conn: &mut SqliteConnection, id: &str| -> Result<Vec<ItemRow>, String> {
        diesel::sql_query(
            "SELECT code, name, quantity, cost_price FROM user_position_snapshot_item \
             WHERE snapshot_id = ?",
        )
        .bind::<Text, _>(id.to_owned())
        .load::<ItemRow>(conn)
        .map_err(|e| format!("读快照明细失败: {e}"))
    };

    let current = load(&mut conn, &current_id)?;
    let previous = load(&mut conn, &snaps[1].snapshot_id)?;
    let prev_map: std::collections::HashMap<&str, &ItemRow> =
        previous.iter().map(|row| (row.code.as_str(), row)).collect();

    let mut buys = Vec::new();
    let mut sells = Vec::new();
    let mut errors = Vec::new();

    for item in &current {
        let old = prev_map.get(item.code.as_str()).copied();
        let old_qty = old.map(|row| row.quantity).unwrap_or(0);
        let old_cost = old.map(|row| row.cost_price).unwrap_or(0.0);
        let delta = item.quantity - old_qty;
        if delta == 0 {
            continue;
        }
        if delta < 0 {
            sells.push((item.code.clone(), item.name.clone(), delta));
            continue;
        }
        let price = if old_qty == 0 {
            item.cost_price
        } else {
            (item.quantity as f64 * item.cost_price - old_qty as f64 * old_cost) / delta as f64
        };
        if !price.is_finite() || price <= 0.0 {
            errors.push(format!(
                "{} 买入价反推失败 (new={}×{}, old={}×{}), 该笔未记流水",
                item.code, item.quantity, item.cost_price, old_qty, old_cost
            ));
            continue;
        }
        let note = if old_qty == 0 {
            format!("新建仓 {} 股，价格=快照成本价（非成交价）", delta)
        } else {
            format!(
                "加仓 {} → {} 股，价格由加权成本反推（非成交价）",
                old_qty, item.quantity
            )
        };
        buys.push(InferredBuy {
            code: item.code.clone(),
            name: item.name.clone(),
            price,
            shares: delta as u64,
            note,
        });
    }

    Ok((current_id, buys, sells, errors))
}

/// 为最近一份快照推断买入并落 `trades`（幂等，同一 snapshot 只记一次）。
pub fn record_snapshot_buys(db: &DatabaseManager, snapshot_effective_at: &str) -> Result<SnapshotTradeReport, String> {
    let (snapshot_id, buys, sells, errors) = snapshot_buy_deltas(db)?;
    let mut report = SnapshotTradeReport {
        snapshot_id: snapshot_id.clone(),
        sells_needing_manual_price: sells,
        errors,
        ..Default::default()
    };
    if snapshot_id.is_empty() {
        return Ok(report);
    }

    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    ensure_schema(&mut conn)?;

    #[derive(QueryableByName)]
    struct CountRow {
        #[diesel(sql_type = BigInt)]
        n: i64,
    }
    let existing: i64 = diesel::sql_query(
        "SELECT COUNT(*) AS n FROM trades WHERE snapshot_id = ? AND source = 'snapshot_inferred'",
    )
    .bind::<Text, _>(snapshot_id.clone())
    .load::<CountRow>(&mut conn)
    .map_err(|e| format!("查已推断流水失败: {e}"))?
    .into_iter()
    .next()
    .map(|row| row.n)
    .unwrap_or(0);
    if existing > 0 {
        report.already_recorded = true;
        return Ok(report);
    }
    drop(conn);

    for buy in &buys {
        let trade = NewTrade {
            code: buy.code.clone(),
            name: buy.name.clone(),
            direction: "buy".to_string(),
            price: buy.price,
            shares: buy.shares,
            traded_at: snapshot_effective_at.to_string(),
            reason: format!("快照差分推断：{}", buy.note),
            source: "snapshot_inferred".to_string(),
            snapshot_id: Some(snapshot_id.clone()),
        };
        match record(db, &trade) {
            Ok(()) => report.inferred_buys += 1,
            Err(error) => report.errors.push(error),
        }
    }
    Ok(report)
}

/// 今天（本地时区）的买入股数，按票汇总。
///
/// 优先读 `trades`（真实成交）；没有任何手工/推断流水时才退回快照差分。
/// 这是 T+1 禁卖判断的数据源。
pub fn today_bought_quantities(
    db: &DatabaseManager,
) -> Result<std::collections::HashMap<String, u64>, String> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        code: String,
        #[diesel(sql_type = BigInt)]
        shares: i64,
    }

    let today = chrono::Local::now().date_naive().format("%Y-%m-%d").to_string();
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    let rows: Vec<Row> = diesel::sql_query(
        "SELECT code, SUM(shares) AS shares FROM trades \
          WHERE direction = 'buy' AND traded_at >= ? \
          GROUP BY code",
    )
    .bind::<Text, _>(today)
    .load(&mut conn)
    .map_err(|e| format!("查今日买入失败: {e}"))?;

    let map: std::collections::HashMap<String, u64> = rows
        .into_iter()
        .filter(|row| row.shares > 0)
        .map(|row| (row.code, row.shares as u64))
        .collect();
    if !map.is_empty() {
        return Ok(map);
    }
    // 没有流水：退回快照差分（历史行为，偏保守）。
    crate::monitor::position_change_detect::t1_locked_quantities(db)
}

/// 指定日期（`traded_at` 的本地日期）的买入股数，按票汇总。
/// 只读 `trades`，供决策卡/历史日期做 T+1 判断，不退快照差分。
pub fn bought_quantities_on(
    conn: &mut SqliteConnection,
    date: &str,
) -> Result<std::collections::HashMap<String, u64>, String> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        code: String,
        #[diesel(sql_type = BigInt)]
        shares: i64,
    }
    let rows: Vec<Row> = diesel::sql_query(
        "SELECT code, SUM(shares) AS shares FROM trades \
         WHERE direction = 'buy' AND date(traded_at) = ?1 GROUP BY code",
    )
    .bind::<Text, _>(date)
    .load(conn)
    .map_err(|error| format!("查 {date} 买入失败: {error}"))?;
    Ok(rows
        .into_iter()
        .filter(|row| row.shares > 0)
        .map(|row| (row.code, row.shares as u64))
        .collect())
}
