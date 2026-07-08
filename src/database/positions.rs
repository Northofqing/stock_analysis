//! positions（从 database.rs 拆分）

use diesel::prelude::*;
use log::info;

use crate::models::{NewStockPosition, StockPosition};
use crate::schema::stock_position;

use super::DatabaseManager;
use super::DbConnection;

fn env_reject_error(msg: String) -> Box<dyn std::error::Error> {
    Box::new(std::io::Error::new(std::io::ErrorKind::PermissionDenied, msg))
}

impl DatabaseManager {
    pub fn save_position(
        &self,
        position: &NewStockPosition,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Err(reason) = crate::risk::env_guard::validate_symbol_for_current_env(&position.code) {
            log::warn!(
                "[ENV_GUARD] rule_id=AGENTS-2.5 code={} env={:?} action=reject reason={} timestamp={}",
                position.code,
                crate::risk::env_guard::current_env(),
                reason,
                chrono::Utc::now().timestamp()
            );
            return Err(env_reject_error(reason));
        }

        use diesel::upsert::excluded;

        let mut conn = self.get_conn()?;

        diesel::insert_into(stock_position::table)
            .values(position)
            .on_conflict((stock_position::code, stock_position::buy_date))
            .do_update()
            .set((
                stock_position::name.eq(excluded(stock_position::name)),
                stock_position::buy_price.eq(excluded(stock_position::buy_price)),
                stock_position::quantity.eq(excluded(stock_position::quantity)),
                stock_position::status.eq(excluded(stock_position::status)),
                // v14.1 F7: 同步 st_type (broker 推送更新时同步写)
                stock_position::st_type.eq(excluded(stock_position::st_type)),
                // v14.1 BR-015: 同步 chain_name (板块集中度数据源)
                stock_position::chain_name.eq(excluded(stock_position::chain_name)),
            ))
            .execute(&mut conn)?;

        info!("[{}] 模拟买入记录已保存（价格: {:.2}, 数量: {}）", position.code, position.buy_price, position.quantity);
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
    pub fn get_all_open_positions(
        &self,
    ) -> Result<Vec<StockPosition>, Box<dyn std::error::Error>> {
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
    ///   - name 含 "ST" / "SST" / "S*ST" → "ST"
    ///   - 其他保持 NULL
    /// 返回更新的行数. 只在 st_type IS NULL 时更新, 重复跑幂等.
    pub fn backfill_st_type(&self) -> Result<usize, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;
        // 顺序: 先标 *ST, 再标 ST, 避免 ST 把 *ST 覆盖
        let star_updated = diesel::sql_query(
            "UPDATE stock_position
             SET st_type = '*ST'
             WHERE st_type IS NULL AND name LIKE '%*ST%'",
        )
        .execute(&mut conn)?;
        let st_updated = diesel::sql_query(
            "UPDATE stock_position
             SET st_type = 'ST'
             WHERE st_type IS NULL
               AND (name LIKE '%SST%' OR name LIKE '%ST%')",
        )
        .execute(&mut conn)?;
        Ok(star_updated + st_updated)
    }

    /// v14.1 BR-015: 回填 stock_position.chain_name
    ///   1. 优先查 stock_concepts 缓存 (东财/同花顺拉过)
    ///   2. 回退到 chain_registry 静态映射 (80+ 龙头股)
    ///   3. 都查不到保持 NULL/其他 (不强行填)
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
