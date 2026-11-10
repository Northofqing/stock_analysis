//! 建议执行跟踪 (2026-10-16).
//!
//! 设计: 每次"操作建议"推送成功 → 落一行 (outcome 待回填);
//!       回填 = 读 stock_daily 的 T+1/T+3/T+5 收盘 + 区间 high/low,
//!       算 change_t1/t3/t5 + mfe/mae (语义对齐
//!       `market_analyzer::performance_feedback::ExecutionRow`).
//!
//! 背景: 旧 execution_tracking 表 schema 硬绑 paper_trade_id NOT NULL (V1 遗产),
//!       V1 于 2026-09-24 退役后结构性无法写新行 —— 本表为独立替代.
//!       写入为 best-effort: 失败只告警, 不影响推送主流程.
//!
//! 口径: 回填算的是"假设执行"的建议质量 (以推送日收盘为基准, 看后续 N 个
//!       交易日的收盘/最高/最低), **不是用户真实收益**. 用户是否照做由
//!       executed/execution_source 另行标注 (当前留空).

use diesel::prelude::*;
use diesel::sql_types::{BigInt, Double, Nullable, Text};

/// 建表 (幂等) + 回填列迁移 (幂等). 由 init_with_scope 调用.
pub fn create_schema(conn: &mut SqliteConnection) -> Result<(), diesel::result::Error> {
    diesel::sql_query(
        "CREATE TABLE IF NOT EXISTS advice_outcome_tracking (\
         id INTEGER PRIMARY KEY AUTOINCREMENT,\
         occurrence_identity TEXT NOT NULL,\
         business_date TEXT NOT NULL,\
         push_kind TEXT NOT NULL,\
         code TEXT NOT NULL,\
         pushed_at TEXT NOT NULL,\
         executed INTEGER,\
         execution_source TEXT,\
         outcome_pct REAL,\
         note TEXT,\
         expected_price REAL,\
         change_t1 REAL,\
         change_t3 REAL,\
         change_t5 REAL,\
         mfe REAL,\
         mae REAL,\
         outcome_filled_at TIMESTAMP,\
         created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,\
         updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,\
         UNIQUE(occurrence_identity))",
    )
    .execute(conn)?;
    // 迁移: 表可能建于"回填列"之前 (2026-10-16 早), 逐列补齐.
    for column in [
        "expected_price REAL",
        "change_t1 REAL",
        "change_t3 REAL",
        "change_t5 REAL",
        "mfe REAL",
        "mae REAL",
        "outcome_filled_at TIMESTAMP",
    ] {
        add_column_if_missing(conn, column);
    }
    diesel::sql_query(
        "CREATE INDEX IF NOT EXISTS idx_advice_outcome_business_date ON advice_outcome_tracking(business_date)",
    )
    .execute(conn)
    .ok();
    diesel::sql_query(
        "CREATE INDEX IF NOT EXISTS idx_advice_outcome_code ON advice_outcome_tracking(code)",
    )
    .execute(conn)
    .ok();
    diesel::sql_query(
        "CREATE INDEX IF NOT EXISTS idx_advice_outcome_pending ON advice_outcome_tracking(outcome_filled_at)",
    )
    .execute(conn)
    .ok();
    Ok(())
}

/// SQLite 无 `ADD COLUMN IF NOT EXISTS` —— 查 pragma 后按需加.
fn add_column_if_missing(conn: &mut SqliteConnection, definition: &str) {
    let Some(name) = definition.split_whitespace().next() else {
        return;
    };
    #[derive(QueryableByName)]
    struct Count {
        #[diesel(sql_type = BigInt)]
        n: i64,
    }
    let exists = diesel::sql_query(
        "SELECT COUNT(*) AS n FROM pragma_table_info('advice_outcome_tracking') WHERE name = ?",
    )
    .bind::<Text, _>(name)
    .get_result::<Count>(conn)
    .map(|row| row.n > 0)
    .unwrap_or(true); // 查询失败时不冒险改结构
    if !exists {
        let _ = diesel::sql_query(format!(
            "ALTER TABLE advice_outcome_tracking ADD COLUMN {definition}"
        ))
        .execute(conn);
    }
}

/// 记录一次建议推送 (幂等: occurrence_identity 唯一). 返回 true = 新插入.
/// 简化落行（2026-10-24）：不走 presentation binding 的推送路径用这个。
/// occurrence_identity 由 `(push_kind, business_date, code)` 派生，天然幂等。
pub fn record_advice_push_simple(
    conn: &mut SqliteConnection,
    push_kind: &str,
    business_date: &str,
    code: &str,
    pushed_at: &str,
) -> Result<(), String> {
    let occurrence = format!("{push_kind}:{business_date}:{code}");
    record_advice_push(conn, &occurrence, business_date, push_kind, code, pushed_at).map(|_| ())
}

pub fn record_advice_push(
    conn: &mut SqliteConnection,
    occurrence_identity: &str,
    business_date: &str,
    push_kind: &str,
    code: &str,
    pushed_at: &str,
) -> Result<bool, String> {
    let rows = diesel::sql_query(
        "INSERT OR IGNORE INTO advice_outcome_tracking \
         (occurrence_identity, business_date, push_kind, code, pushed_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind::<Text, _>(occurrence_identity)
    .bind::<Text, _>(business_date)
    .bind::<Text, _>(push_kind)
    .bind::<Text, _>(code)
    .bind::<Text, _>(pushed_at)
    .execute(conn)
    .map_err(|e| format!("advice_outcome insert failed: {e}"))?;
    Ok(rows > 0)
}

/// 待回填的一行.
#[derive(Debug, Clone)]
pub struct PendingAdviceRow {
    pub id: i64,
    pub code: String,
    pub business_date: String,
    pub push_kind: String,
}

/// 取未回填的行 (按日期升序, 最早的先算).
pub fn load_pending_outcomes(
    conn: &mut SqliteConnection,
    limit: i64,
) -> Result<Vec<PendingAdviceRow>, String> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = BigInt)]
        id: i64,
        #[diesel(sql_type = Text)]
        code: String,
        #[diesel(sql_type = Text)]
        business_date: String,
        #[diesel(sql_type = Text)]
        push_kind: String,
    }
    let rows: Vec<Row> = diesel::sql_query(
        "SELECT id, code, business_date, push_kind FROM advice_outcome_tracking \
         WHERE outcome_filled_at IS NULL \
         ORDER BY business_date ASC, id ASC LIMIT ?",
    )
    .bind::<BigInt, _>(limit)
    .load(conn)
    .map_err(|e| format!("advice pending load failed: {e}"))?;
    Ok(rows
        .into_iter()
        .map(|r| PendingAdviceRow {
            id: r.id,
            code: r.code,
            business_date: r.business_date,
            push_kind: r.push_kind,
        })
        .collect())
}

/// 已回填的一行 (供消费段转 ExecutionRow).
#[derive(Debug, Clone)]
pub struct FilledAdviceRow {
    pub code: String,
    pub push_kind: String,
    pub business_date: String,
    pub expected_price: Option<f64>,
    pub change_t1: Option<f64>,
    pub change_t3: Option<f64>,
    pub change_t5: Option<f64>,
    pub mfe: Option<f64>,
    pub mae: Option<f64>,
}

/// 取已回填的行 (消费段用).
pub fn load_filled_outcomes(
    conn: &mut SqliteConnection,
    since_business_date: &str,
    limit: i64,
) -> Result<Vec<FilledAdviceRow>, String> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Text)]
        code: String,
        #[diesel(sql_type = Text)]
        push_kind: String,
        #[diesel(sql_type = Text)]
        business_date: String,
        #[diesel(sql_type = Nullable<Double>)]
        expected_price: Option<f64>,
        #[diesel(sql_type = Nullable<Double>)]
        change_t1: Option<f64>,
        #[diesel(sql_type = Nullable<Double>)]
        change_t3: Option<f64>,
        #[diesel(sql_type = Nullable<Double>)]
        change_t5: Option<f64>,
        #[diesel(sql_type = Nullable<Double>)]
        mfe: Option<f64>,
        #[diesel(sql_type = Nullable<Double>)]
        mae: Option<f64>,
    }
    let rows: Vec<Row> = diesel::sql_query(
        "SELECT code, push_kind, business_date, expected_price, change_t1, change_t3, \
         change_t5, mfe, mae FROM advice_outcome_tracking \
         WHERE outcome_filled_at IS NOT NULL AND business_date >= ? \
         ORDER BY business_date ASC, id ASC LIMIT ?",
    )
    .bind::<Text, _>(since_business_date)
    .bind::<BigInt, _>(limit)
    .load(conn)
    .map_err(|e| format!("advice filled load failed: {e}"))?;
    Ok(rows
        .into_iter()
        .map(|r| FilledAdviceRow {
            code: r.code,
            push_kind: r.push_kind,
            business_date: r.business_date,
            expected_price: r.expected_price,
            change_t1: r.change_t1,
            change_t3: r.change_t3,
            change_t5: r.change_t5,
            mfe: r.mfe,
            mae: r.mae,
        })
        .collect())
}

/// 回填结果 (语义对齐 performance_feedback::ExecutionRow).
#[derive(Debug, Clone, Copy)]
pub struct FilledOutcome {
    pub expected_price: f64,
    pub change_t1: Option<f64>,
    pub change_t3: Option<f64>,
    pub change_t5: Option<f64>,
    pub mfe: f64,
    pub mae: f64,
}

/// 写回一行的回填结果.
/// `outcome_pct` 取 T+1 (短期方向); 完整区间看 change_t3/change_t5.
/// `execution_source` 不被覆盖 —— 它留给"用户真实执行"的确认来源.
/// `note` 记录口径补充 (如"仅部分期数有数据").
pub fn apply_outcome(
    conn: &mut SqliteConnection,
    id: i64,
    filled: &FilledOutcome,
    note: Option<&str>,
) -> Result<(), String> {
    diesel::sql_query(
        "UPDATE advice_outcome_tracking SET \
         expected_price = ?, change_t1 = ?, change_t3 = ?, change_t5 = ?, \
         mfe = ?, mae = ?, outcome_pct = ?, note = ?, \
         outcome_filled_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP \
         WHERE id = ?",
    )
    .bind::<Double, _>(filled.expected_price)
    .bind::<Nullable<Double>, _>(filled.change_t1)
    .bind::<Nullable<Double>, _>(filled.change_t3)
    .bind::<Nullable<Double>, _>(filled.change_t5)
    .bind::<Double, _>(filled.mfe)
    .bind::<Double, _>(filled.mae)
    .bind::<Nullable<Double>, _>(filled.change_t1)
    .bind::<Nullable<Text>, _>(note)
    .bind::<BigInt, _>(id)
    .execute(conn)
    .map_err(|e| format!("advice outcome update failed: {e}"))?;
    Ok(())
}

/// 写回已成熟但尚未走完 T+5 观察窗的部分结果。
///
/// 只填 `change_t1/change_t3`，**不写 `outcome_filled_at`**，让该行继续留在
/// pending 集合；等 T+5 走完后由 `apply_outcome` 写最终值。这样周初/上周的
/// T+1、T+3 不必等 T+5 才能看到，同时不会把 T+5 误标成已完成。
pub fn apply_partial_outcome(
    conn: &mut SqliteConnection,
    id: i64,
    expected_price: f64,
    change_t1: Option<f64>,
    change_t3: Option<f64>,
) -> Result<(), String> {
    diesel::sql_query(
        "UPDATE advice_outcome_tracking SET \
         expected_price = ?, change_t1 = COALESCE(?, change_t1), \
         change_t3 = COALESCE(?, change_t3), outcome_pct = COALESCE(?, outcome_pct), \
         note = 'partial_t1_t3', updated_at = CURRENT_TIMESTAMP \
         WHERE id = ? AND outcome_filled_at IS NULL",
    )
    .bind::<Double, _>(expected_price)
    .bind::<Nullable<Double>, _>(change_t1)
    .bind::<Nullable<Double>, _>(change_t3)
    .bind::<Nullable<Double>, _>(change_t1)
    .bind::<BigInt, _>(id)
    .execute(conn)
    .map_err(|e| format!("advice partial outcome update failed: {e}"))?;
    Ok(())
}

/// 标记该行无法回填 (基准日缺收盘, 或观察窗超期仍无数据).
///
/// 只写 `note` + `outcome_filled_at`, **数值列保持 NULL** —— 不补 0
/// (BR-222: 数据缺失一律 fail-closed, 不用推导值填充).
/// 写 `outcome_filled_at` 同时让它退出 pending 扫描, 避免每轮重复搜库.
pub fn mark_outcome_void(conn: &mut SqliteConnection, id: i64, reason: &str) -> Result<(), String> {
    diesel::sql_query(
        "UPDATE advice_outcome_tracking SET note = ?, \
         outcome_filled_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP \
         WHERE id = ? AND outcome_filled_at IS NULL",
    )
    .bind::<Text, _>(reason)
    .bind::<BigInt, _>(id)
    .execute(conn)
    .map_err(|e| format!("advice outcome void mark failed: {e}"))?;
    Ok(())
}
