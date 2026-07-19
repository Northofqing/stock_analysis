//! Registered business rules: BR-127.
//! lhb（从 database.rs 拆分）

use chrono::Local;
use diesel::prelude::*;

use crate::models::{LhbDaily, NewLhbDaily};
use crate::schema::lhb_daily;

use super::DatabaseManager;

pub(crate) fn validate_lhb_records(
    records: &[NewLhbDaily],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut identities = std::collections::HashSet::with_capacity(records.len());
    for (index, record) in records.iter().enumerate() {
        crate::risk::env_guard::validate_symbol_for_current_env(&record.code).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("龙虎榜第 {} 行环境隔离失败: {error}", index + 1),
            )
        })?;
        if record.name.trim().is_empty() || record.reason.trim().is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("龙虎榜第 {} 行 name/reason 缺失", index + 1),
            )
            .into());
        }
        let date_text = record.trade_date.get(..10).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("龙虎榜第 {} 行 trade_date 字段不足", index + 1),
            )
        })?;
        let trade_date =
            chrono::NaiveDate::parse_from_str(date_text, "%Y-%m-%d").map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("龙虎榜第 {} 行 trade_date 非法: {error}", index + 1),
                )
            })?;
        if !crate::calendar::is_trading_day(trade_date) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("龙虎榜第 {} 行不是交易日: {trade_date}", index + 1),
            )
            .into());
        }
        if !identities.insert((record.code.as_str(), date_text)) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("龙虎榜批次重复 code/date: {}/{}", record.code, date_text),
            )
            .into());
        }

        let values = [
            ("pct_change", record.pct_change),
            ("close_price", record.close_price),
            ("buy_amount", record.buy_amount),
            ("sell_amount", record.sell_amount),
            ("net_amount", record.net_amount),
            ("total_amount", record.total_amount),
            ("lhb_ratio", record.lhb_ratio),
        ];
        if let Some((field, _)) = values.iter().find(|(_, value)| !value.is_finite()) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("龙虎榜第 {} 行 {field} 非有限", index + 1),
            )
            .into());
        }
        if record.close_price <= 0.0
            || record.pct_change.abs() > 20.0
            || record.buy_amount < 0.0
            || record.sell_amount < 0.0
            || record.total_amount < 0.0
            || record.lhb_ratio.abs() > 100.0
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("龙虎榜第 {} 行数值越界", index + 1),
            )
            .into());
        }
        let expected_net = record.buy_amount - record.sell_amount;
        let tolerance = 0.01_f64.max((record.buy_amount + record.sell_amount) * 1e-9);
        if (record.net_amount - expected_net).abs() > tolerance {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "龙虎榜第 {} 行净额不一致: net={} buy-sell={expected_net}",
                    index + 1,
                    record.net_amount
                ),
            )
            .into());
        }
    }
    Ok(())
}

impl DatabaseManager {
    pub fn save_lhb_records(
        &self,
        records: &[NewLhbDaily],
    ) -> Result<usize, Box<dyn std::error::Error>> {
        if records.is_empty() {
            return Ok(0);
        }
        validate_lhb_records(records)?;

        let mut conn = self.get_conn()?;
        // 注：diesel 的 SQLite 后端不支持「多行批量 INSERT + ON CONFLICT」组合，
        // 故保持单事务内逐行 upsert——事务包裹已消除逐行 fsync，是此场景下的最优写法。
        let saved = conn.transaction::<usize, Box<dyn std::error::Error>, _>(|conn| {
            let mut count = 0;
            for record in records {
                count += diesel::insert_into(lhb_daily::table)
                    .values(record)
                    .on_conflict((lhb_daily::code, lhb_daily::trade_date))
                    .do_nothing()
                    .execute(conn)
                    .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
            }
            Ok(count)
        })?;

        Ok(saved)
    }

    /// 检查指定日期的龙虎榜数据是否已缓存
    pub fn has_lhb_data_for_date(
        &self,
        trade_date: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        let count: i64 = lhb_daily::table
            .filter(lhb_daily::trade_date.eq(trade_date))
            .count()
            .get_result(&mut conn)?;

        Ok(count > 0)
    }

    /// 从数据库获取指定日期的龙虎榜数据（支持模糊匹配）
    pub fn get_lhb_by_date(
        &self,
        trade_date: &str,
    ) -> Result<Vec<LhbDaily>, Box<dyn std::error::Error>> {
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

        let deleted =
            diesel::delete(lhb_daily::table.filter(lhb_daily::trade_date.lt(cutoff_date)))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn init_test_db() -> &'static DatabaseManager {
        DatabaseManager::init(None).expect("test database init");
        DatabaseManager::get()
    }

    fn unique_code(label: &str) -> String {
        format!(
            "TEST_CODE_LHB_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        )
    }

    fn record(code: &str, trade_date: &str, net_amount: f64) -> NewLhbDaily {
        NewLhbDaily {
            code: code.to_string(),
            name: "测试龙虎榜".to_string(),
            trade_date: trade_date.to_string(),
            reason: "测试席位事实".to_string(),
            pct_change: 5.0,
            close_price: 10.0,
            buy_amount: 30.0,
            sell_amount: 30.0 - net_amount,
            net_amount,
            total_amount: 100.0,
            lhb_ratio: 10.0,
        }
    }

    #[test]
    #[serial_test::serial]
    fn br127_lhb_write_rejects_bad_or_duplicate_complete_batches() {
        let db = init_test_db();
        let code = unique_code("INVALID");
        let mut invalid_rows = Vec::new();
        let mut real_code = record("000001", "2026-07-16", 10.0);
        real_code.name = "测试环境中的真实代码".to_string();
        invalid_rows.push(real_code);
        for mutate in [
            |row: &mut NewLhbDaily| row.name = " ".to_string(),
            |row: &mut NewLhbDaily| row.reason = "".to_string(),
            |row: &mut NewLhbDaily| row.trade_date = "2026".to_string(),
            |row: &mut NewLhbDaily| row.trade_date = "2026-02-30".to_string(),
            |row: &mut NewLhbDaily| row.pct_change = f64::NAN,
            |row: &mut NewLhbDaily| row.close_price = 0.0,
            |row: &mut NewLhbDaily| row.pct_change = 20.1,
            |row: &mut NewLhbDaily| row.buy_amount = -1.0,
            |row: &mut NewLhbDaily| row.sell_amount = -1.0,
            |row: &mut NewLhbDaily| row.total_amount = -1.0,
            |row: &mut NewLhbDaily| row.lhb_ratio = 100.1,
            |row: &mut NewLhbDaily| row.net_amount = 9.0,
        ] {
            let mut invalid = record(&code, "2026-07-16", 10.0);
            mutate(&mut invalid);
            invalid_rows.push(invalid);
        }
        for invalid in invalid_rows {
            assert!(
                db.save_lhb_records(&[invalid]).is_err(),
                "bad LHB row must reject the complete batch"
            );
        }

        let duplicated = record(&code, "2026-07-16", 10.0);
        assert!(db
            .save_lhb_records(&[duplicated.clone(), duplicated])
            .is_err());
        let partial_code = unique_code("PARTIAL");
        let valid = record(&partial_code, "2026-07-17", 10.0);
        let mut bad = record(&unique_code("BAD_TAIL"), "2026-07-17", 10.0);
        bad.close_price = -1.0;
        assert!(db.save_lhb_records(&[valid, bad]).is_err());
        assert_eq!(
            db.get_lhb_count_by_code(&partial_code, "2026-01-01", "2026-12-31")
                .expect("query partial-batch sentinel"),
            0
        );
        assert!(!db
            .has_lhb_data_for_date("2026-07-16")
            .expect("query rejected fixture date"));
    }

    #[test]
    #[serial_test::serial]
    fn br127_lhb_repository_is_idempotent_queryable_and_cleanable() {
        let db = init_test_db();
        let first_code = unique_code("FIRST");
        let second_code = unique_code("SECOND");
        let future_date = "2099-07-16";
        let rows = [
            record(&first_code, future_date, 10.0),
            record(&second_code, future_date, -5.0),
        ];
        assert_eq!(db.save_lhb_records(&rows).expect("save valid batch"), 2);
        assert_eq!(db.save_lhb_records(&rows).expect("idempotent replay"), 0);
        assert!(db
            .has_lhb_data_for_date(future_date)
            .expect("date cache exists"));
        let loaded = db
            .get_lhb_by_date("2099-07")
            .expect("fuzzy date query succeeds");
        let ours = loaded
            .iter()
            .filter(|row| row.code == first_code || row.code == second_code)
            .collect::<Vec<_>>();
        assert_eq!(ours.len(), 2);
        assert_eq!(ours[0].code, first_code);
        assert!(ours[0].net_amount >= ours[1].net_amount);
        assert_eq!(
            db.get_lhb_count_by_code(&first_code, "2099-01-01", "2099-12-31")
                .expect("count code rows"),
            1
        );
        assert_eq!(db.dedupe_lhb_data().expect("dedupe valid data"), 0);

        let old_code = unique_code("OLD");
        db.save_lhb_records(&[record(&old_code, "2000-01-03", 0.0)])
            .expect("save old row");
        assert!(db.clean_old_lhb_data(30).expect("clean old rows") >= 1);
        assert_eq!(
            db.get_lhb_count_by_code(&old_code, "1900-01-01", "2100-01-01")
                .expect("old row count"),
            0
        );

        let mut conn = db.get_conn().expect("test database connection");
        diesel::delete(lhb_daily::table.filter(lhb_daily::code.eq_any([first_code, second_code])))
            .execute(&mut conn)
            .expect("clean lhb fixtures");
    }
}
