//! kline（从 database.rs 拆分）

use chrono::{Local, NaiveDate};
use diesel::prelude::*;
use log::{info, warn};

use crate::models::{NewStockDaily, StockDaily, NewAnalysisResult, AnalysisResultRecord};
use crate::schema::{stock_daily, analysis_result};

use super::DatabaseManager;
use super::{AnalysisContext, DbConnection, StockDailyRecord};

impl DatabaseManager {
    pub fn has_data_for_date(
        &self,
        code: &str,
        target_date: NaiveDate,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        let count: i64 = stock_daily::table
            .filter(stock_daily::code.eq(code))
            .filter(stock_daily::date.eq(target_date))
            .count()
            .get_result(&mut conn)?;

        Ok(count > 0)
    }

    /// 检查是否有今天的数据
    pub fn has_today_data(&self, code: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let today = Local::now().date_naive();
        self.has_data_for_date(code, today)
    }

    /// 获取最近 N 天的数据
    ///
    /// 用于计算"相比昨日"的变化
    pub fn get_latest_data(
        &self,
        code: &str,
        days: i64,
    ) -> Result<Vec<StockDaily>, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        let results = stock_daily::table
            .filter(stock_daily::code.eq(code))
            .order(stock_daily::date.desc())
            .limit(days)
            .load::<StockDaily>(&mut conn)?;

        Ok(results)
    }

    /// 获取指定日期范围的数据
    pub fn get_data_range(
        &self,
        code: &str,
        start_date: NaiveDate,
        end_date: NaiveDate,
    ) -> Result<Vec<StockDaily>, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        let results = stock_daily::table
            .filter(stock_daily::code.eq(code))
            .filter(stock_daily::date.ge(start_date))
            .filter(stock_daily::date.le(end_date))
            .order(stock_daily::date.asc())
            .load::<StockDaily>(&mut conn)?;

        Ok(results)
    }

    /// 保存单条日线数据
    ///
    /// 策略：使用 ON CONFLICT DO UPDATE（单条 SQL 完成 UPSERT）
    pub fn save_daily_record(
        &self,
        code: &str,
        date: NaiveDate,
        open: Option<f64>,
        high: Option<f64>,
        low: Option<f64>,
        close: Option<f64>,
        volume: Option<f64>,
        amount: Option<f64>,
        pct_chg: Option<f64>,
        ma5: Option<f64>,
        ma10: Option<f64>,
        ma20: Option<f64>,
        volume_ratio: Option<f64>,
        data_source: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;
        Self::upsert_daily_record(&mut conn, code, date, open, high, low, close, volume, amount, pct_chg, ma5, ma10, ma20, volume_ratio, data_source)
    }

    /// 内部 UPSERT 方法，接受已有连接（避免批量操作时重复获取连接）
    fn upsert_daily_record(
        conn: &mut DbConnection,
        code: &str,
        date: NaiveDate,
        open: Option<f64>,
        high: Option<f64>,
        low: Option<f64>,
        close: Option<f64>,
        volume: Option<f64>,
        amount: Option<f64>,
        pct_chg: Option<f64>,
        ma5: Option<f64>,
        ma10: Option<f64>,
        ma20: Option<f64>,
        volume_ratio: Option<f64>,
        data_source: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use diesel::upsert::excluded;

        let new_record = NewStockDaily {
            code: code.to_string(),
            date,
            open,
            high,
            low,
            close,
            volume,
            amount,
            pct_chg,
            ma5,
            ma10,
            ma20,
            volume_ratio,
            data_source: data_source.map(|s| s.to_string()),
        };

        diesel::insert_into(stock_daily::table)
            .values(&new_record)
            .on_conflict((stock_daily::code, stock_daily::date))
            .do_update()
            .set((
                stock_daily::open.eq(excluded(stock_daily::open)),
                stock_daily::high.eq(excluded(stock_daily::high)),
                stock_daily::low.eq(excluded(stock_daily::low)),
                stock_daily::close.eq(excluded(stock_daily::close)),
                stock_daily::volume.eq(excluded(stock_daily::volume)),
                stock_daily::amount.eq(excluded(stock_daily::amount)),
                stock_daily::pct_chg.eq(excluded(stock_daily::pct_chg)),
                stock_daily::ma5.eq(excluded(stock_daily::ma5)),
                stock_daily::ma10.eq(excluded(stock_daily::ma10)),
                stock_daily::ma20.eq(excluded(stock_daily::ma20)),
                stock_daily::volume_ratio.eq(excluded(stock_daily::volume_ratio)),
                stock_daily::data_source.eq(excluded(stock_daily::data_source)),
                stock_daily::updated_at.eq(Local::now().naive_local()),
            ))
            .execute(conn)?;

        Ok(())
    }

    /// 批量保存日线数据
    ///
    /// 使用单连接 + 事务，返回新增/更新的记录数
    pub fn save_daily_batch(
        &self,
        records: &[StockDailyRecord],
    ) -> Result<usize, Box<dyn std::error::Error>> {
        if records.is_empty() {
            return Ok(0);
        }

        let mut conn = self.get_conn()?;
        let saved_count = conn.transaction::<usize, Box<dyn std::error::Error>, _>(|conn| {
            for record in records {
                Self::upsert_daily_record(
                    conn,
                    &record.code,
                    record.date,
                    record.open,
                    record.high,
                    record.low,
                    record.close,
                    record.volume,
                    record.amount,
                    record.pct_chg,
                    record.ma5,
                    record.ma10,
                    record.ma20,
                    record.volume_ratio,
                    record.data_source.as_deref(),
                )?;
            }
            Ok(records.len())
        })?;

        info!("批量保存完成，新增/更新 {} 条记录", saved_count);
        Ok(saved_count)
    }

    /// 获取分析所需的上下文数据
    ///
    /// 返回今日数据 + 昨日数据的对比信息
    pub fn get_analysis_context(
        &self,
        code: &str,
        target_date: Option<NaiveDate>,
    ) -> Result<Option<AnalysisContext>, Box<dyn std::error::Error>> {
        let _target = target_date.unwrap_or_else(|| Local::now().date_naive());

        // 获取最近2天数据
        let recent_data = self.get_latest_data(code, 2)?;

        if recent_data.is_empty() {
            warn!("未找到 {} 的数据", code);
            return Ok(None);
        }

        let today_data = &recent_data[0];
        let yesterday_data = recent_data.get(1);

        let mut context = AnalysisContext {
            code: code.to_string(),
            date: today_data.date,
            today: today_data.to_dict(),
            yesterday: None,
            volume_change_ratio: None,
            price_change_ratio: None,
            ma_status: today_data.analyze_ma_status(),
        };

        if let Some(yesterday) = yesterday_data {
            context.yesterday = Some(yesterday.to_dict());

            // 计算成交量变化
            if let (Some(today_vol), Some(yesterday_vol)) = (today_data.volume, yesterday.volume) {
                if yesterday_vol > 0.0 {
                    context.volume_change_ratio = Some((today_vol / yesterday_vol * 100.0).round() / 100.0);
                }
            }

            // 计算价格变化
            if let (Some(today_close), Some(yesterday_close)) =
                (today_data.close, yesterday.close)
            {
                if yesterday_close > 0.0 {
                    context.price_change_ratio = Some(
                        ((today_close - yesterday_close) / yesterday_close * 100.0 * 100.0).round()
                            / 100.0,
                    );
                }
            }
        }

        Ok(Some(context))
    }

    /// 保存 KlineData 列表到数据库
    ///
    /// 使用单连接 + 事务批量 UPSERT
    pub fn save_kline_data(
        &self,
        code: &str,
        data: &[crate::data_provider::KlineData],
        source: &str,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        if data.is_empty() {
            return Ok(0);
        }

        let mut conn = self.get_conn()?;
        let saved = conn.transaction::<usize, Box<dyn std::error::Error>, _>(|conn| {
            for kline in data {
                Self::upsert_daily_record(
                    conn,
                    code,
                    kline.date,
                    Some(kline.open),
                    Some(kline.high),
                    Some(kline.low),
                    Some(kline.close),
                    Some(kline.volume),
                    Some(kline.amount),
                    Some(kline.pct_chg),
                    None, // ma5 由趋势分析模块计算
                    None, // ma10
                    None, // ma20
                    None, // volume_ratio
                    Some(source),
                )?;
            }
            Ok(data.len())
        })?;

        info!("[{}] 已保存 {} 条K线数据到数据库（数据源: {}）", code, saved, source);
        Ok(saved)
    }

    /// 保存分析结果到数据库（使用 ON CONFLICT DO UPDATE，单条 SQL）
    pub fn save_analysis_result(
        &self,
        result: &NewAnalysisResult,
    ) -> Result<(), Box<dyn std::error::Error>> {
        use diesel::upsert::excluded;

        let mut conn = self.get_conn()?;

        diesel::insert_into(analysis_result::table)
            .values(result)
            .on_conflict((analysis_result::code, analysis_result::date))
            .do_update()
            .set((
                analysis_result::name.eq(excluded(analysis_result::name)),
                analysis_result::sentiment_score.eq(excluded(analysis_result::sentiment_score)),
                analysis_result::operation_advice.eq(excluded(analysis_result::operation_advice)),
                analysis_result::trend_prediction.eq(excluded(analysis_result::trend_prediction)),
                analysis_result::pe_ratio.eq(excluded(analysis_result::pe_ratio)),
                analysis_result::pb_ratio.eq(excluded(analysis_result::pb_ratio)),
                analysis_result::turnover_rate.eq(excluded(analysis_result::turnover_rate)),
                analysis_result::market_cap.eq(excluded(analysis_result::market_cap)),
                analysis_result::circulating_cap.eq(excluded(analysis_result::circulating_cap)),
                analysis_result::close_price.eq(excluded(analysis_result::close_price)),
                analysis_result::pct_chg.eq(excluded(analysis_result::pct_chg)),
                analysis_result::data_source.eq(excluded(analysis_result::data_source)),
            ))
            .execute(&mut conn)?;

        info!("[{}] 保存/更新分析结果（评分: {}）", result.code, result.sentiment_score);
        Ok(())
    }

    /// 获取指定日期的所有分析结果
    #[allow(dead_code)]
    pub fn get_analysis_results_by_date(
        &self,
        date: NaiveDate,
    ) -> Result<Vec<AnalysisResultRecord>, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        let results = analysis_result::table
            .filter(analysis_result::date.eq(date))
            .order(analysis_result::sentiment_score.desc())
            .load::<AnalysisResultRecord>(&mut conn)?;

        Ok(results)
    }

    /// 获取指定股票最近N次分析结果
    #[allow(dead_code)]
    pub fn get_latest_analysis_results(
        &self,
        code: &str,
        limit: i64,
    ) -> Result<Vec<AnalysisResultRecord>, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        let results = analysis_result::table
            .filter(analysis_result::code.eq(code))
            .order(analysis_result::date.desc())
            .limit(limit)
            .load::<AnalysisResultRecord>(&mut conn)?;

        Ok(results)
    }

    /// 删除指定股票的所有数据（用于测试）
    #[allow(dead_code)]
    pub fn delete_stock_data(&self, code: &str) -> Result<usize, Box<dyn std::error::Error>> {
        let mut conn = self.get_conn()?;

        let deleted = diesel::delete(stock_daily::table.filter(stock_daily::code.eq(code)))
            .execute(&mut conn)?;

        Ok(deleted)
    }
}
