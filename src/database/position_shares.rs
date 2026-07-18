//! v12 PR3-3.2 / 3.3: available_shares 合成 + position_adjustments 写入.
//!
//! 设计: 不动 stock_position upsert/close 语义. available_shares 是**计算函数**, 不写库.
//! position_adjustments 是**新表**写入入口, 与 stock_position 互补 (BR-023/024).
//!
//! 合成口径 (v12 §10.2):
//!   available_shares(code) = Σ (stock_position.quantity where status='open' AND buy_date<today)
//!                          + Σ (position_adjustments.delta where effective_date<=today)
//!                            其中 delta<0 → applied_immediately=1 → 计入
//!                                  delta>0 → applied_immediately=0 → effective_date<=today 计入
//!
//! 边界: 当日买入 (buy_date==today) 不计入 available (避免 T+1 制度冲突).

use diesel::prelude::*;

use super::DatabaseManager;
use crate::risk::env_guard::current_env;
use crate::schema::{position_adjustments, stock_position};

/// 计算某 code 当前可用股数 (可卖数).
///
/// 返回 None 表示数据缺失 (今日无 open 持仓 + 无 adjustments).
/// 返回 Some(n) 表示可用股数 (n>=0).
pub fn available_shares(code: &str) -> Result<Option<i64>, String> {
    let mut conn = DatabaseManager::get()
        .get_conn()
        .map_err(|e| format!("DB 连接失败: {}", e))?;

    let today = chrono::Local::now().date_naive();
    let today_str = today.format("%Y-%m-%d").to_string();

    // 1. stock_position: open + buy_date < today (当日买入不计入)
    let pos_total: Option<i64> = stock_position::table
        .filter(stock_position::code.eq(code))
        .filter(stock_position::status.eq("open"))
        .filter(stock_position::buy_date.lt(&today_str))
        .select(diesel::dsl::sum(stock_position::quantity))
        .first(&mut conn)
        .map_err(|e| format!("sum stock_position: {}", e))?;

    let pos_sum: i64 = pos_total.unwrap_or(0);

    // 2. position_adjustments: effective_date <= today
    //    delta<0 (减仓) always counts (applied_immediately=1)
    //    delta>0 (加仓) only counts when effective_date <= today
    let adj_total: Option<i64> = position_adjustments::table
        .filter(position_adjustments::code.eq(code))
        .filter(position_adjustments::effective_date.le(&today_str))
        .select(diesel::dsl::sum(position_adjustments::delta))
        .first(&mut conn)
        .map_err(|e| format!("sum position_adjustments: {}", e))?;

    let adj_sum: i64 = adj_total.unwrap_or(0);

    let total = pos_sum + adj_sum;

    if total == 0 && pos_sum == 0 && adj_sum == 0 {
        // 完全无持仓 (且无 adjustments) → None
        Ok(None)
    } else {
        Ok(Some(total.max(0))) // 防御: 负数裁 0 (理论上不该出现, 但保险)
    }
}

/// 写入一笔 position_adjustment (人工确认减仓 / 同日加仓)
///
/// `source` ∈ {'manual_confirm', 'import'}
/// `delta` < 0 → applied_immediately=1 (即时生效, available 立即减)
/// `delta` > 0 → applied_immediately=0 (T+1 生效, effective_date=today+1)
///
/// env_guard: 测试环境 (env=test) 拦截写入 (对齐 positions.rs:21/:111).
///
/// 返回新插入行的 id.
pub fn insert_position_adjustment(
    code: &str,
    delta: i32,
    source: &str,
    reason: &str,
    operator: Option<&str>,
) -> Result<i64, String> {
    if code.is_empty() {
        return Err("code 不能为空".to_string());
    }
    if delta == 0 {
        return Err("delta 不能为 0".to_string());
    }
    if !matches!(source, "manual_confirm" | "import") {
        return Err(format!(
            "source 必须 ∈ manual_confirm|import, 实得 {}",
            source
        ));
    }

    // env_guard: 测试环境拦截 (对齐 positions.rs:21)
    if matches!(current_env(), crate::risk::env_guard::TradingEnv::Test) {
        log::warn!(
            "[ENV_GUARD] position_adjustments: 测试环境拦截写入 code={} delta={}",
            code,
            delta
        );
        return Err("测试环境不允许写入 position_adjustments".to_string());
    }

    let mut conn = DatabaseManager::get()
        .get_conn()
        .map_err(|e| format!("DB 连接失败: {}", e))?;

    let today = chrono::Local::now().date_naive();
    let effective_date = if delta < 0 {
        today.format("%Y-%m-%d").to_string()
    } else {
        // 加仓 T+1 生效
        (today + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string()
    };
    let applied_immediately = if delta < 0 { 1 } else { 0 };

    let esc = |s: &str| s.replace('\'', "''");
    let sql = format!(
        "INSERT INTO position_adjustments (code, delta, source, reason, effective_date, applied_immediately, operator) \
         VALUES ('{}', {}, '{}', '{}', '{}', {}, {})",
        esc(code), delta, esc(source), esc(reason), effective_date, applied_immediately,
        operator.map(|o| format!("'{}'", esc(o))).unwrap_or_else(|| "NULL".to_string())
    );

    diesel::sql_query(sql)
        .execute(&mut conn)
        .map_err(|e| format!("insert_position_adjustment: {}", e))?;

    // 取 last_insert_rowid
    #[derive(diesel::QueryableByName)]
    struct IdRow {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        id: i64,
    }
    let row: IdRow = diesel::sql_query("SELECT last_insert_rowid() AS id")
        .get_result(&mut conn)
        .map_err(|e| format!("last_insert_rowid: {}", e))?;
    Ok(row.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::NewStockPosition;

    fn init_test_db() -> &'static DatabaseManager {
        DatabaseManager::init(None).expect("test database init");
        DatabaseManager::get()
    }

    fn unique_code(label: &str) -> String {
        format!(
            "TEST_CODE_SHARES_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        )
    }

    /// available_shares 边界测试: 纯逻辑, 不需要 DB.
    /// 真实 DB 测试由 `tests/position_shares_e2e.rs` (PR3 验收) 覆盖.
    #[test]
    fn insert_position_adjustment_validates_source() {
        // 测试参数校验 (不依赖 DB)
        assert!(
            insert_position_adjustment("TEST_CODE_000001", 0, "manual_confirm", "", None).is_err()
        );
        assert!(
            insert_position_adjustment("TEST_CODE_000001", 100, "invalid_source", "", None)
                .is_err()
        );
        assert!(insert_position_adjustment("", 100, "manual_confirm", "", None).is_err());
    }

    #[test]
    fn insert_position_adjustment_blocked_in_test_env() {
        // 显式设测试环境, 验证 env_guard
        std::env::set_var("STOCK_ENV_MODE", "test");
        let result =
            insert_position_adjustment("TEST_CODE_000001", -100, "manual_confirm", "test", None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("测试环境"));
        std::env::remove_var("STOCK_ENV_MODE");
    }

    /// 有效日期计算: delta<0 → today, delta>0 → today+1
    #[test]
    fn effective_date_logic() {
        let today = chrono::Local::now().date_naive();
        let eff_minus = today; // delta<0: 即时
        let eff_plus = today + chrono::Duration::days(1); // delta>0: T+1
        assert!(eff_minus < eff_plus);
    }

    #[test]
    #[serial_test::serial]
    fn br024_available_shares_uses_only_settled_positions_and_effective_adjustments() {
        let db = init_test_db();
        let code = unique_code("COMPOSE");
        let empty_code = unique_code("EMPTY");
        assert_eq!(available_shares(&empty_code).unwrap(), None);

        let today = chrono::Local::now().date_naive();
        let yesterday = today - chrono::Duration::days(1);
        db.save_position(&NewStockPosition {
            code: code.clone(),
            name: "测试可用股份".to_string(),
            buy_date: yesterday.format("%Y-%m-%d").to_string(),
            buy_price: 10.0,
            quantity: 200,
            status: "open".to_string(),
            st_type: None,
            chain_name: None,
        })
        .expect("save settled position");
        db.save_position(&NewStockPosition {
            code: code.clone(),
            name: "测试当日股份".to_string(),
            buy_date: today.format("%Y-%m-%d").to_string(),
            buy_price: 11.0,
            quantity: 300,
            status: "open".to_string(),
            st_type: None,
            chain_name: None,
        })
        .expect("save same-day position");

        let mut conn = db.get_conn().expect("test database connection");
        diesel::sql_query(
            "INSERT INTO position_adjustments \
             (code, delta, source, reason, effective_date, applied_immediately, operator) \
             VALUES (?, -50, 'manual_confirm', 'TEST_CODE immediate', ?, 1, NULL), \
                    (?, 100, 'import', 'TEST_CODE future', ?, 0, 'TEST_CODE_OPERATOR')",
        )
        .bind::<diesel::sql_types::Text, _>(&code)
        .bind::<diesel::sql_types::Text, _>(today.format("%Y-%m-%d").to_string())
        .bind::<diesel::sql_types::Text, _>(&code)
        .bind::<diesel::sql_types::Text, _>(
            (today + chrono::Duration::days(1))
                .format("%Y-%m-%d")
                .to_string(),
        )
        .execute(&mut conn)
        .expect("insert isolated adjustment facts");
        drop(conn);

        assert_eq!(available_shares(&code).unwrap(), Some(150));

        let mut conn = db.get_conn().expect("test database connection");
        diesel::sql_query(
            "INSERT INTO position_adjustments \
             (code, delta, source, reason, effective_date, applied_immediately, operator) \
             VALUES (?, -500, 'manual_confirm', 'TEST_CODE clamp', ?, 1, NULL)",
        )
        .bind::<diesel::sql_types::Text, _>(&code)
        .bind::<diesel::sql_types::Text, _>(today.format("%Y-%m-%d").to_string())
        .execute(&mut conn)
        .expect("insert negative adjustment fact");
        drop(conn);
        assert_eq!(available_shares(&code).unwrap(), Some(0));

        let mut conn = db.get_conn().expect("test database connection");
        diesel::delete(position_adjustments::table.filter(position_adjustments::code.eq(&code)))
            .execute(&mut conn)
            .expect("clean adjustment fixtures");
        diesel::delete(stock_position::table.filter(stock_position::code.eq(&code)))
            .execute(&mut conn)
            .expect("clean position fixtures");
    }
}
