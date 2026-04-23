//! lhb（从 database.rs 拆分）

use chrono::Local;
use diesel::prelude::*;

use crate::models::{NewLhbDaily, LhbDaily};
use crate::schema::lhb_daily;

use super::DatabaseManager;

impl DatabaseManager {
    pub fn save_lhb_records(&self, records: &[NewLhbDaily]) -> Result<usize, Box<dyn std::error::Error>> {
        if records.is_empty() {
            return Ok(0);
        }

        let mut conn = self.get_conn()?;
        let saved = conn.transaction::<usize, Box<dyn std::error::Error>, _>(|conn| {
            let mut count = 0;
            for record in records {
                let result = diesel::insert_into(lhb_daily::table)
                    .values(record)
                    .on_conflict((lhb_daily::code, lhb_daily::trade_date))
                    .do_nothing()
                    .execute(conn);

                match result {
                    Ok(n) => count += n,
                    Err(e) => return Err(Box::new(e) as Box<dyn std::error::Error>),
                }
            }
            Ok(count)
        })?;

        Ok(saved)
    }

    /// 检查指定日期的龙虎榜数据是否已缓存
    pub fn has_lhb_data_for_date(&self, trade_date: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        let count: i64 = lhb_daily::table
            .filter(lhb_daily::trade_date.eq(trade_date))
            .count()
            .get_result(&mut conn)?;

        Ok(count > 0)
    }

    /// 从数据库获取指定日期的龙虎榜数据（支持模糊匹配）
    pub fn get_lhb_by_date(&self, trade_date: &str) -> Result<Vec<LhbDaily>, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        // 支持日期模糊匹配：2026-01-29 可以匹配 2026-01-29%
        let date_pattern = format!("{}%", trade_date);
        
        let records = lhb_daily::table
            .filter(lhb_daily::trade_date.like(date_pattern))
            .order(lhb_daily::net_amount.desc())
            .load::<LhbDaily>(&mut conn)?;

        Ok(records)
    }

    /// 获取指定股票在某段时间内的龙虎榜上榜次数
    pub fn get_lhb_count_by_code(
        &self,
        code: &str,
        start_date: &str,
        end_date: &str,
    ) -> Result<i64, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        let count = lhb_daily::table
            .filter(lhb_daily::code.eq(code))
            .filter(lhb_daily::trade_date.ge(start_date))
            .filter(lhb_daily::trade_date.le(end_date))
            .count()
            .get_result(&mut conn)?;

        Ok(count)
    }

    /// 清除过期的龙虎榜缓存数据（保留最近N天）
    pub fn clean_old_lhb_data(&self, keep_days: i64) -> Result<usize, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;
        
        let cutoff_date = Local::now()
            .date_naive()
            .checked_sub_signed(chrono::Duration::days(keep_days))
            .unwrap()
            .format("%Y%m%d")
            .to_string();

        let deleted = diesel::delete(
            lhb_daily::table.filter(lhb_daily::trade_date.lt(cutoff_date))
        )
        .execute(&mut conn)?;

        Ok(deleted)
    }

    /// 去重龙虎榜缓存（同一股票同一日期仅保留最新一条）
    pub fn dedupe_lhb_data(&self) -> Result<usize, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        let deleted = diesel::sql_query(
            r#"
            DELETE FROM lhb_daily
            WHERE id NOT IN (
                SELECT MAX(id)
                FROM lhb_daily
                GROUP BY code, trade_date
            )
            "#,
        )
        .execute(&mut conn)?;

        Ok(deleted)
    }

    // ========================================================================
    // 模拟持仓操作
    // ========================================================================

}
