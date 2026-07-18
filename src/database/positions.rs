//! positions（从 database.rs 拆分）

use diesel::prelude::*;
use log::info;

use crate::models::{NewStockPosition, StockPosition};
use crate::schema::stock_position;

use super::DatabaseManager;
use super::DbConnection;

fn env_reject_error(msg: String) -> Box<dyn std::error::Error> {
    Box::new(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        msg,
    ))
}

impl DatabaseManager {
    pub fn save_position(
        &self,
        position: &NewStockPosition,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Err(reason) = crate::risk::env_guard::validate_symbol_for_current_env(&position.code)
        {
            log::warn!(
                "[ENV_GUARD] rule_id=AGENTS-2.5 code={} env={:?} action=reject reason={} timestamp={}",
                position.code,
                crate::risk::env_guard::current_env(),
                reason,
                chrono::Utc::now().timestamp()
            );
            return Err(env_reject_error(reason));
        }

        use diesel::dsl::sql;
        use diesel::upsert::excluded;

        let mut conn = self.get_conn()?;
        // BR-123: normalize legacy missing sentinels before Diesel binds the row.
        let mut normalized = position.clone();
        normalized.chain_name = position
            .chain_name
            .as_deref()
            .map(str::trim)
            .filter(|chain| !chain.is_empty() && *chain != "其他")
            .map(str::to_string);

        diesel::insert_into(stock_position::table)
            .values(&normalized)
            .on_conflict((stock_position::code, stock_position::buy_date))
            .do_update()
            .set((
                stock_position::name.eq(excluded(stock_position::name)),
                stock_position::buy_price.eq(excluded(stock_position::buy_price)),
                stock_position::quantity.eq(excluded(stock_position::quantity)),
                stock_position::status.eq(excluded(stock_position::status)),
                // v14.1 F7 fix: COALESCE 保 NULL 时不覆盖 backfilled / broker-pushed 值
                // trading::open_position 总是传 None, 之前会清掉 backfill 写好的 *ST
                stock_position::st_type.eq(sql::<
                    diesel::sql_types::Nullable<diesel::sql_types::Text>,
                >(
                    "COALESCE(excluded.st_type, stock_position.st_type)"
                )),
                // BR-123: 缺失/旧“其他”哨兵不覆盖既有明确产业链。
                stock_position::chain_name.eq(sql::<
                    diesel::sql_types::Nullable<diesel::sql_types::Text>,
                >(
                    "COALESCE(NULLIF(NULLIF(trim(excluded.chain_name), ''), '其他'), stock_position.chain_name)",
                )),
            ))
            .execute(&mut conn)?;

        info!(
            "[{}] 模拟买入记录已保存（价格: {:.2}, 数量: {}）",
            normalized.code, normalized.buy_price, normalized.quantity
        );
        Ok(())
    }

    /// 获取指定股票的最新一条持仓中(open)记录
    pub fn get_open_position(
        &self,
        code: &str,
    ) -> Result<Option<StockPosition>, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        let result = stock_position::table
            .filter(stock_position::code.eq(code))
            .filter(stock_position::status.eq("open"))
            .order(stock_position::buy_date.desc())
            .first::<StockPosition>(&mut conn)
            .optional()?;

        Ok(result)
    }

    /// 获取所有持仓中(open)的记录
    pub fn get_all_open_positions(&self) -> Result<Vec<StockPosition>, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        let results = stock_position::table
            .filter(stock_position::status.eq("open"))
            .order(stock_position::buy_date.desc())
            .load::<StockPosition>(&mut conn)?;

        Ok(results)
    }

    /// 统计持仓中(open)的记录数 (v19.11 用于 --test 路径判断 DB 是否已被真实持仓填充)
    pub fn count_open_positions(&self) -> Result<usize, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;
        let count: i64 = stock_position::table
            .filter(stock_position::status.eq("open"))
            .count()
            .get_result(&mut conn)?;
        Ok(count as usize)
    }

    /// 更新持仓收益率
    pub fn update_position_return(
        &self,
        id: i32,
        _current_price: f64,
        return_rate: f64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        diesel::update(stock_position::table.filter(stock_position::id.eq(id)))
            .set((
                stock_position::return_rate.eq(return_rate),
                stock_position::updated_at.eq(diesel::dsl::now),
            ))
            .execute(&mut conn)?;

        Ok(())
    }

    /// 平仓（将状态改为 closed）
    pub fn close_position(
        &self,
        id: i32,
        sell_price: f64,
        sell_date: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;
        let code = self.get_position_code(&mut conn, id)?;
        if let Err(reason) = crate::risk::env_guard::validate_symbol_for_current_env(&code) {
            log::warn!(
                "[ENV_GUARD] rule_id=AGENTS-2.5 code={} env={:?} action=reject reason={} timestamp={}",
                code,
                crate::risk::env_guard::current_env(),
                reason,
                chrono::Utc::now().timestamp()
            );
            return Err(env_reject_error(reason));
        }

        let return_rate = (sell_price / self.get_position_buy_price(&mut conn, id)? - 1.0) * 100.0;

        diesel::update(stock_position::table.filter(stock_position::id.eq(id)))
            .set((
                stock_position::status.eq("closed"),
                stock_position::sell_date.eq(sell_date),
                stock_position::sell_price.eq(sell_price),
                stock_position::return_rate.eq(return_rate),
                stock_position::updated_at.eq(diesel::dsl::now),
            ))
            .execute(&mut conn)?;

        Ok(())
    }

    fn get_position_buy_price(
        &self,
        conn: &mut DbConnection,
        id: i32,
    ) -> Result<f64, Box<dyn std::error::Error>> {
        let price: f64 = stock_position::table
            .filter(stock_position::id.eq(id))
            .select(stock_position::buy_price)
            .first(conn)?;
        Ok(price)
    }

    fn get_position_code(
        &self,
        conn: &mut DbConnection,
        id: i32,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let code: String = stock_position::table
            .filter(stock_position::id.eq(id))
            .select(stock_position::code)
            .first(conn)?;
        Ok(code)
    }

    /// v14.1 F7: 回填 stock_position.st_type 列 (从 name 字段 LIKE 推断)
    ///   - name 含 "*ST" → "*ST"
    ///   - name 以 "ST" / "SST" / "S*ST" 开头 → "ST"
    ///   - 其他保持 NULL
    ///
    /// 返回更新的行数. 只在 st_type IS NULL 时更新, 重复跑幂等.
    pub fn backfill_st_type(&self) -> Result<usize, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;
        // v14.1 review fix: 前缀锚定 ('ST%' / '*ST%' / 'SST%' / 'S*ST%') 避免子串误判
        // 之前 '%ST%' 会把 'BEST' / 'GST' / 'VST' 误判成 ST 类
        // 顺序: 先标 *ST, 再标 ST, 避免 ST 把 *ST 覆盖
        let star_updated = diesel::sql_query(
            "UPDATE stock_position
             SET st_type = '*ST'
             WHERE st_type IS NULL AND (name LIKE '*ST%' OR name LIKE 'S*ST%')",
        )
        .execute(&mut conn)?;
        let st_updated = diesel::sql_query(
            "UPDATE stock_position
             SET st_type = 'ST'
             WHERE st_type IS NULL
               AND (name LIKE 'ST%' OR name LIKE 'SST%')",
        )
        .execute(&mut conn)?;
        Ok(star_updated + st_updated)
    }

    /// v14.1 BR-015: 回填 stock_position.chain_name
    ///   1. 优先查 stock_concepts 缓存 (东财/同花顺拉过)
    ///   2. 回退到 chain_registry 静态映射 (80+ 龙头股)
    ///   3. 都查不到保持 NULL/其他 (不强行填)
    ///
    ///   只在 chain_name IS NULL OR '' OR '其他' 时更新, 重复跑幂等.
    ///   返回 (updated, missing_after) — 更新行数 + 仍缺失数.
    pub fn backfill_chain_name(&self) -> Result<(usize, i64), Box<dyn std::error::Error>> {
        use crate::data_provider::chain_registry;
        let mut conn = self.get_conn()?;

        // 1. 拉所有缺失 chain_name 的 (code, name) 列表
        #[derive(diesel::QueryableByName)]
        struct PosRow {
            #[diesel(sql_type = diesel::sql_types::Text)]
            code: String,
        }
        let rows: Vec<PosRow> = diesel::sql_query(
            "SELECT code FROM stock_position
             WHERE status = 'open'
               AND (chain_name IS NULL OR chain_name = '' OR chain_name = '其他')",
        )
        .load(&mut conn)?;

        // 2. 查 registry, 找到的 UPDATE
        let mut updated = 0;
        for row in &rows {
            if let Some(chain) = chain_registry::lookup(&row.code) {
                let n = diesel::sql_query(
                    "UPDATE stock_position SET chain_name = ?1 \
                     WHERE code = ?2 AND status = 'open' \
                       AND (chain_name IS NULL OR chain_name = '' OR chain_name = '其他')",
                )
                .bind::<diesel::sql_types::Text, _>(chain)
                .bind::<diesel::sql_types::Text, _>(&row.code)
                .execute(&mut conn)?;
                updated += n;
            }
        }

        // 3. 统计仍缺失数
        let missing_after: i64 = diesel::sql_query(
            "SELECT COUNT(*) AS cnt FROM stock_position
             WHERE status = 'open'
               AND (chain_name IS NULL OR chain_name = '' OR chain_name = '其他')",
        )
        .get_result::<CountRow>(&mut conn)
        .map(|r| r.cnt)?;

        Ok((updated, missing_after))
    }
}

/// v14.1 BR-015: count helper (Query 复用)
#[derive(diesel::QueryableByName)]
struct CountRow {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    cnt: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_test_db() -> &'static DatabaseManager {
        DatabaseManager::init(None).expect("test database init");
        DatabaseManager::get()
    }

    fn unique_code(label: &str) -> String {
        format!(
            "TEST_CODE_POS_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        )
    }

    #[test]
    #[serial_test::serial]
    fn position_repository_round_trip_preserves_metadata_and_closes() {
        let db = init_test_db();
        let code = unique_code("ROUND_TRIP");
        let buy_date = "2026-07-01";
        db.save_position(&NewStockPosition {
            code: code.clone(),
            name: "*ST测试持仓".to_string(),
            buy_date: buy_date.to_string(),
            buy_price: 8.0,
            quantity: 200,
            status: "open".to_string(),
            st_type: Some("*ST".to_string()),
            chain_name: Some("TEST_CODE_CHAIN".to_string()),
        })
        .expect("save position");

        db.save_position(&NewStockPosition {
            code: code.clone(),
            name: "测试持仓改名".to_string(),
            buy_date: buy_date.to_string(),
            buy_price: 10.0,
            quantity: 300,
            status: "open".to_string(),
            st_type: None,
            chain_name: None,
        })
        .expect("upsert position");

        let row = db
            .get_open_position(&code)
            .expect("query position")
            .expect("open position exists");
        assert_eq!(row.name, "测试持仓改名");
        assert_eq!(row.buy_price, 10.0);
        assert_eq!(row.quantity, 300);
        assert_eq!(row.st_type.as_deref(), Some("*ST"));
        assert_eq!(row.chain_name.as_deref(), Some("TEST_CODE_CHAIN"));
        assert!(db
            .get_all_open_positions()
            .expect("list positions")
            .iter()
            .any(|position| position.code == code));
        assert!(db.count_open_positions().expect("count positions") >= 1);

        db.update_position_return(row.id, 11.0, 10.0)
            .expect("update return");
        let updated = db
            .get_open_position(&code)
            .expect("query updated position")
            .expect("updated position exists");
        assert_eq!(updated.return_rate, Some(10.0));

        db.close_position(row.id, 12.0, "2026-07-18")
            .expect("close position");
        assert!(db
            .get_open_position(&code)
            .expect("query closed position")
            .is_none());

        let mut conn = db.get_conn().expect("test database connection");
        let closed = stock_position::table
            .filter(stock_position::id.eq(row.id))
            .first::<StockPosition>(&mut conn)
            .expect("closed row remains auditable");
        assert_eq!(closed.status, "closed");
        assert_eq!(closed.sell_price, Some(12.0));
        assert_eq!(closed.sell_date.as_deref(), Some("2026-07-18"));
        let closed_return = closed.return_rate.expect("closed return is stored");
        assert!((closed_return - 20.0).abs() < 1e-9);
    }

    #[test]
    #[serial_test::serial]
    fn position_repository_rejects_cross_environment_and_missing_rows() {
        let db = init_test_db();
        let rejected = db.save_position(&NewStockPosition {
            code: "000001".to_string(),
            name: "真实代码不得进入测试持仓".to_string(),
            buy_date: "2026-07-01".to_string(),
            buy_price: 10.0,
            quantity: 100,
            status: "open".to_string(),
            st_type: None,
            chain_name: None,
        });
        assert!(rejected
            .expect_err("test environment rejects real symbols")
            .to_string()
            .contains("测试环境拒绝真实标的"));
        assert!(db.close_position(i32::MAX, 10.0, "2026-07-18").is_err());
    }

    #[test]
    #[serial_test::serial]
    fn position_backfills_only_supported_evidence() {
        let db = init_test_db();
        let star_code = unique_code("STAR_ST");
        let ordinary_code = unique_code("ORDINARY");
        for (code, name) in [(&star_code, "S*ST测试"), (&ordinary_code, "BEST测试")] {
            db.save_position(&NewStockPosition {
                code: code.clone(),
                name: name.to_string(),
                buy_date: "2026-07-02".to_string(),
                buy_price: 10.0,
                quantity: 100,
                status: "open".to_string(),
                st_type: None,
                chain_name: None,
            })
            .expect("save backfill fixture");
        }

        assert!(db.backfill_st_type().expect("backfill ST type") >= 1);
        let (_updated, missing_after) = db.backfill_chain_name().expect("backfill chain name");
        assert!(missing_after >= 2);

        let star = db
            .get_open_position(&star_code)
            .expect("query star position")
            .expect("star position exists");
        let ordinary = db
            .get_open_position(&ordinary_code)
            .expect("query ordinary position")
            .expect("ordinary position exists");
        assert_eq!(star.st_type.as_deref(), Some("*ST"));
        assert_eq!(ordinary.st_type, None);
        assert_eq!(star.chain_name, None);
        assert_eq!(ordinary.chain_name, None);
    }
}
