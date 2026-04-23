//! positions（从 database.rs 拆分）

use diesel::prelude::*;
use log::info;

use crate::models::{NewStockPosition, StockPosition};
use crate::schema::stock_position;

use super::DatabaseManager;
use super::DbConnection;

impl DatabaseManager {
    pub fn save_position(
        &self,
        position: &NewStockPosition,
    ) -> Result<(), Box<dyn std::error::Error>> {
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
}
