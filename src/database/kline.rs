//! kline（从 database.rs 拆分）
//! Registered business rule: BR-092.

use chrono::{Local, NaiveDate};
use diesel::prelude::*;
use log::{info, warn};

use crate::data_gateway::historical_bars::AdmittedDailyBars;
use crate::models::{AnalysisResultRecord, NewAnalysisResult, NewStockDaily, StockDaily};
use crate::schema::{analysis_result, stock_daily};

use super::DatabaseManager;
use super::{AnalysisContext, DbConnection, StockDailyRecord};

impl DatabaseManager {
    fn persist_validated_kline_data(
        &self,
        code: &str,
        data: &[crate::data_provider::KlineData],
        source: &str,
    ) -> Result<usize, Box<dyn std::error::Error>> {
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
                    None,
                    None,
                    None,
                    None,
                    Some(source),
                )?;
            }
            Ok(data.len())
        })?;

        info!(
            "[{}] 已保存 {} 条K线数据到数据库（数据源: {}）",
            code, saved, source
        );
        Ok(saved)
    }

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
    #[allow(
        clippy::too_many_arguments,
        reason = "stable database boundary mirrors the stock_daily row schema"
    )]
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
        conn.transaction::<(), Box<dyn std::error::Error>, _>(|conn| {
            Self::upsert_daily_record(
                conn,
                code,
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
                data_source,
            )
        })
    }

    /// 内部 UPSERT 方法，接受已有连接（避免批量操作时重复获取连接）
    #[allow(
        clippy::too_many_arguments,
        reason = "internal UPSERT boundary mirrors the stock_daily row schema"
    )]
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

        // Ordinary daily writes do not carry the authority that qualified the
        // previous projection. Even identical prices cannot prove its original
        // provenance binding survived. All callers hold the encompassing
        // transaction, so an invalidation error also rolls back this UPSERT.
        diesel::sql_query(
            "DELETE FROM qualified_daily_trading_status WHERE code = ?1 AND date = ?2",
        )
        .bind::<diesel::sql_types::Text, _>(code)
        .bind::<diesel::sql_types::Text, _>(date.to_string())
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
                    context.volume_change_ratio =
                        Some((today_vol / yesterday_vol * 100.0).round() / 100.0);
                }
            }

            // 计算价格变化
            if let (Some(today_close), Some(yesterday_close)) = (today_data.close, yesterday.close)
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
        if code.trim().is_empty() || source.trim().is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("BR-092 K 线批次代码/来源不能为空: code={code:?} source={source:?}"),
            )
            .into());
        }
        let mut checked = data.to_vec();
        crate::monitor::data_quality::validate_daily_kline_quality(&mut checked, code).map_err(
            |error| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("BR-092 K 线批次拒绝: {error}"),
                )
            },
        )?;

        self.persist_validated_kline_data(code, &checked, source)
    }

    /// Persist an immutable daily-bar capability after the Gateway has
    /// completed structural, lifecycle, confirmation, freshness and audit
    /// admission. The private fields of `AdmittedDailyBars` prevent raw
    /// records from entering this path without that authority.
    pub fn save_admitted_kline_data(
        &self,
        batch: &AdmittedDailyBars,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        let code = batch.target_code();
        let evidence = batch.evidence();
        if code.trim().is_empty()
            || evidence.source.trim().is_empty()
            || evidence.batch_id.trim().is_empty()
            || evidence.observed_at.trim().is_empty()
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "BR-171 admitted K-line batch has incomplete target/source/batch evidence",
            )
            .into());
        }
        if batch.records().is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "BR-171 admitted K-line batch is unexpectedly empty",
            )
            .into());
        }
        self.persist_validated_kline_data(code, batch.records(), &evidence.source)
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
                analysis_result::score_breakdown_json
                    .eq(excluded(analysis_result::score_breakdown_json)),
                analysis_result::original_advice.eq(excluded(analysis_result::original_advice)),
                analysis_result::veto_flags_json.eq(excluded(analysis_result::veto_flags_json)),
            ))
            .execute(&mut conn)?;

        info!(
            "[{}] 保存/更新分析结果（评分: {}）",
            result.code, result.sentiment_score
        );
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

        conn.transaction::<usize, Box<dyn std::error::Error>, _>(|conn| {
            let deleted = diesel::delete(stock_daily::table.filter(stock_daily::code.eq(code)))
                .execute(conn)?;
            // Also clear marker-only orphans. Preserve the daily-row count and
            // propagate either deletion error without leaving a partial delete.
            diesel::sql_query("DELETE FROM qualified_daily_trading_status WHERE code = ?1")
                .bind::<diesel::sql_types::Text, _>(code)
                .execute(conn)?;
            Ok(deleted)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_gateway::{AdmittedDailyBars, BatchEvidence};
    use crate::data_provider::{AdjustType, KlineData};
    use crate::market_domain::ProviderId;

    fn unique_code(label: &str) -> String {
        format!(
            "TEST_CODE_KLINE_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time")
                .as_nanos()
        )
    }

    fn kline(date: NaiveDate, close: f64, pct_chg: f64) -> KlineData {
        KlineData {
            date,
            open: close,
            high: close,
            low: close,
            close,
            volume: 1_000.0,
            amount: close * 1_000.0,
            pct_chg,
            intraday_price: None,
            settled: true,
            pe_ratio: None,
            pb_ratio: None,
            turnover_rate: None,
            market_cap: None,
            circulating_cap: None,
            eps: None,
            roe: None,
            revenue_yoy: None,
            net_profit_yoy: None,
            gross_margin: None,
            net_margin: None,
            sharpe_ratio: None,
            financials_history: None,
            valuation_history: None,
            consensus: None,
            industry: None,
            is_limit_up: false,
            is_limit_down: false,
            is_suspended: false,
            adjust: AdjustType::Qfq,
        }
    }

    struct KlineGuard(Vec<String>);

    impl Drop for KlineGuard {
        fn drop(&mut self) {
            if let Ok(mut conn) = DatabaseManager::get().get_conn() {
                for code in &self.0 {
                    let _ = diesel::delete(stock_daily::table.filter(stock_daily::code.eq(code)))
                        .execute(&mut conn);
                    let _ = diesel::delete(
                        analysis_result::table.filter(analysis_result::code.eq(code)),
                    )
                    .execute(&mut conn);
                }
            }
        }
    }

    #[test]
    #[serial_test::serial]
    fn br092_database_persistence_requires_confirmation_for_tick_rounding_over_twenty_percent() {
        DatabaseManager::init(None).expect("test database init");
        let code = "TEST_CODE_688548".to_string();
        let _guard = KlineGuard(vec![code.clone()]);
        let db = DatabaseManager::get();
        let day1 = NaiveDate::from_ymd_opt(2026, 7, 20).unwrap();
        let day2 = NaiveDate::from_ymd_opt(2026, 7, 21).unwrap();
        let day3 = NaiveDate::from_ymd_opt(2026, 7, 22).unwrap();
        let day1_close = 10.0;
        let day2_close = 12.000_52;
        let day3_close = day2_close * 1.200_917;
        let bars = vec![
            kline(day1, day1_close, 0.0),
            kline(day2, day2_close, 20.0052),
            kline(day3, day3_close, 20.0917),
        ];

        let error = db
            .save_kline_data(&code, &bars, "TEST_PROVIDER")
            .expect_err("20.0052% still requires BR-171 confirmation");
        assert!(error.to_string().contains("manual_confirmation_required"));
    }

    #[test]
    #[serial_test::serial]
    fn br092_database_persistence_rejects_unconfirmed_large_main_board_move() {
        DatabaseManager::init(None).expect("test database init");
        let code = "TEST_CODE_600548".to_string();
        let _guard = KlineGuard(vec![code.clone()]);
        let db = DatabaseManager::get();
        let day1 = NaiveDate::from_ymd_opt(2026, 7, 20).unwrap();
        let day2 = NaiveDate::from_ymd_opt(2026, 7, 21).unwrap();
        let bars = vec![kline(day1, 10.0, 0.0), kline(day2, 12.000_52, 20.0052)];

        let error = db
            .save_kline_data(&code, &bars, "TEST_PROVIDER")
            .expect_err("结构完整的大幅行情仍需 BR-171 人工确认");
        assert!(error.to_string().contains("manual_confirmation_required"));
    }

    #[test]
    #[serial_test::serial]
    fn br092_database_persistence_rejects_unconfirmed_large_star_market_move() {
        DatabaseManager::init(None).expect("test database init");
        let code = "TEST_CODE_688690".to_string();
        let _guard = KlineGuard(vec![code.clone()]);
        let db = DatabaseManager::get();
        let day1 = NaiveDate::from_ymd_opt(2026, 7, 20).unwrap();
        let day2 = NaiveDate::from_ymd_opt(2026, 7, 21).unwrap();
        let bars = vec![kline(day1, 10.0, 0.0), kline(day2, 12.051, 20.51)];

        let error = db
            .save_kline_data(&code, &bars, "TEST_PROVIDER")
            .expect_err("科创板大幅变化也需 BR-171 证据绑定确认");
        assert!(error.to_string().contains("manual_confirmation_required"));
    }

    #[test]
    #[serial_test::serial]
    fn br171_admitted_daily_batch_preserves_gateway_authority_through_persistence() {
        DatabaseManager::init(None).expect("test database init");
        let code = unique_code("ADMITTED");
        let _guard = KlineGuard(vec![code.clone()]);
        let day1 = NaiveDate::from_ymd_opt(2026, 7, 20).unwrap();
        let day2 = NaiveDate::from_ymd_opt(2026, 7, 21).unwrap();
        let bars = vec![kline(day1, 10.0, 0.0), kline(day2, 12.5, 25.0)];
        let admitted = AdmittedDailyBars::from_test_fixture(
            &code,
            bars,
            BatchEvidence {
                provider: ProviderId::Tdx,
                source: "TEST_CODE_magic_tdx".to_string(),
                source_at: Some("2026-07-21".to_string()),
                observed_at: "2026-07-21T15:01:00+08:00".to_string(),
                batch_id: "TEST_CODE_admitted_daily_batch".to_string(),
            },
        )
        .expect("test-only admitted capability");

        assert_eq!(
            DatabaseManager::get()
                .save_admitted_kline_data(&admitted)
                .expect("persist admitted batch"),
            2
        );
        assert!(DatabaseManager::get()
            .has_data_for_date(&code, day2)
            .expect("persisted latest day"));
    }

    #[test]
    #[serial_test::serial]
    fn br092_kline_repository_roundtrip_and_analysis_context() {
        DatabaseManager::init(None).expect("test database init");
        let code = unique_code("DAILY");
        let provider_code = unique_code("PROVIDER");
        let _guard = KlineGuard(vec![code.clone(), provider_code.clone()]);
        let db = DatabaseManager::get();
        let day1 = NaiveDate::from_ymd_opt(2026, 7, 15).unwrap();
        let day2 = NaiveDate::from_ymd_opt(2026, 7, 16).unwrap();

        assert_eq!(db.save_daily_batch(&[]).unwrap(), 0);
        assert_eq!(
            db.save_kline_data(&provider_code, &[], "TEST_SOURCE")
                .unwrap(),
            0
        );
        assert!(!db.has_data_for_date(&code, day1).unwrap());
        assert!(db.get_analysis_context(&code, None).unwrap().is_none());

        db.save_daily_record(
            &code,
            day1,
            Some(10.0),
            Some(10.2),
            Some(9.8),
            Some(10.0),
            Some(100.0),
            Some(1_000.0),
            Some(0.0),
            Some(9.9),
            Some(9.8),
            Some(9.7),
            Some(1.0),
            Some("TEST_SOURCE"),
        )
        .expect("save daily row");
        db.save_daily_record(
            &code,
            day1,
            Some(10.0),
            Some(10.3),
            Some(9.8),
            Some(10.0),
            Some(100.0),
            Some(1_100.0),
            Some(0.0),
            Some(9.9),
            Some(9.8),
            Some(9.7),
            Some(1.1),
            Some("TEST_SOURCE_V2"),
        )
        .expect("upsert daily row");
        assert!(db.has_data_for_date(&code, day1).unwrap());
        let record = StockDailyRecord {
            code: code.clone(),
            date: day2,
            open: Some(10.0),
            high: Some(11.2),
            low: Some(9.9),
            close: Some(11.0),
            volume: Some(200.0),
            amount: Some(2_200.0),
            pct_chg: Some(10.0),
            ma5: Some(10.5),
            ma10: Some(10.0),
            ma20: Some(9.5),
            volume_ratio: Some(2.0),
            data_source: Some("TEST_BATCH".to_string()),
        };
        assert_eq!(db.save_daily_batch(&[record]).unwrap(), 1);
        let latest = db.get_latest_data(&code, 1).unwrap();
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].date, day2);
        assert_eq!(latest[0].close, Some(11.0));
        let range = db.get_data_range(&code, day1, day2).unwrap();
        assert_eq!(range.len(), 2);
        assert_eq!(range[0].date, day1);
        let context = db
            .get_analysis_context(&code, None)
            .unwrap()
            .expect("analysis context");
        assert_eq!(context.code, code);
        assert_eq!(context.date, day2);
        assert_eq!(context.volume_change_ratio, Some(2.0));
        assert_eq!(context.price_change_ratio, Some(10.0));
        assert!(context.yesterday.is_some());

        let valid = vec![kline(day1, 10.0, 0.0), kline(day2, 11.0, 10.0)];
        assert!(db.save_kline_data("", &valid, "TEST_PROVIDER").is_err());
        assert!(db.save_kline_data(&provider_code, &valid, " ").is_err());
        assert_eq!(
            db.save_kline_data(&provider_code, &valid, "TEST_PROVIDER")
                .expect("save validated provider batch"),
            2
        );
        let mut invalid = kline(day2, 11.0, 0.0);
        invalid.low = -1.0;
        assert!(db
            .save_kline_data(&provider_code, &[invalid], "TEST_PROVIDER")
            .is_err());

        let mut result = NewAnalysisResult {
            code: code.clone(),
            name: "K线测试".to_string(),
            date: day2,
            sentiment_score: 70,
            operation_advice: "观望".to_string(),
            trend_prediction: "震荡".to_string(),
            pe_ratio: Some(10.0),
            pb_ratio: None,
            turnover_rate: None,
            market_cap: None,
            circulating_cap: None,
            close_price: Some(11.0),
            pct_chg: Some(10.0),
            data_source: Some("TEST_SOURCE".to_string()),
            score_breakdown_json: None,
            original_advice: None,
            veto_flags_json: None,
        };
        db.save_analysis_result(&result)
            .expect("save analysis result");
        result.sentiment_score = 80;
        result.operation_advice = "持有".to_string();
        db.save_analysis_result(&result)
            .expect("upsert analysis result");
        let by_date = db.get_analysis_results_by_date(day2).unwrap();
        let stored = by_date.iter().find(|row| row.code == code).unwrap();
        assert_eq!(stored.sentiment_score, 80);
        assert_eq!(stored.operation_advice, "持有");
        let latest_results = db.get_latest_analysis_results(&code, 1).unwrap();
        assert_eq!(latest_results.len(), 1);
        assert_eq!(db.delete_stock_data(&code).unwrap(), 2);
        assert!(db.get_latest_data(&code, 1).unwrap().is_empty());
    }

    fn qualified_daily_private_db() -> (tempfile::TempDir, DatabaseManager) {
        let dir = tempfile::tempdir().unwrap();
        let db = DatabaseManager::open_isolated_for_test(
            dir.path().join("TEST_CODE_qualified_daily_invalidation.db"),
        )
        .unwrap();
        (dir, db)
    }

    fn qualified_daily_record(code: &str, day: NaiveDate, close: f64) -> StockDailyRecord {
        assert!(code.starts_with("TEST_CODE_"));
        StockDailyRecord {
            code: code.to_owned(),
            date: day,
            open: Some(close),
            high: Some(close),
            low: Some(close),
            close: Some(close),
            volume: Some(1_000.0),
            amount: Some(close * 1_000.0),
            pct_chg: Some(0.0),
            ma5: None,
            ma10: None,
            ma20: None,
            volume_ratio: None,
            data_source: Some("TEST_CODE_ordinary_daily".to_owned()),
        }
    }

    fn qualified_daily_save_single(
        db: &DatabaseManager,
        record: &StockDailyRecord,
    ) -> Result<(), Box<dyn std::error::Error>> {
        db.save_daily_record(
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
        )
    }

    // Synthetic markers are confined to an actually isolated Test database.
    // They never construct a Gateway authority or production status writer.
    fn qualified_daily_test_marker(db: &DatabaseManager, code: &str, day: NaiveDate, status: &str) {
        assert!(code.starts_with("TEST_CODE_"));
        let mut conn = db.get_conn().unwrap();
        diesel::sql_query(
            "INSERT INTO qualified_daily_trading_status \
             (code,date,status,contract_version,source,source_at,observed_at,batch_id) \
             VALUES (?1,?2,?3,'TEST_CODE_AUTHORITY_V1','TEST_CODE_AUTHORITY', \
                     '2026-09-30T07:00:00Z','2026-09-30T07:00:01Z','TEST_CODE_BATCH')",
        )
        .bind::<diesel::sql_types::Text, _>(code)
        .bind::<diesel::sql_types::Text, _>(day.to_string())
        .bind::<diesel::sql_types::Text, _>(status)
        .execute(&mut conn)
        .unwrap();
    }

    fn qualified_daily_seed(
        db: &DatabaseManager,
        code: &str,
        day: NaiveDate,
        close: f64,
        status: &str,
    ) {
        qualified_daily_save_single(db, &qualified_daily_record(code, day, close)).unwrap();
        qualified_daily_test_marker(db, code, day, status);
    }

    #[derive(Debug, PartialEq, Eq, diesel::QueryableByName)]
    struct QualifiedDailyMarkerRow {
        #[diesel(sql_type = diesel::sql_types::Text)]
        code: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        date: String,
        #[diesel(sql_type = diesel::sql_types::Text)]
        status: String,
    }

    fn qualified_daily_markers(db: &DatabaseManager) -> Vec<QualifiedDailyMarkerRow> {
        let mut conn = db.get_conn().unwrap();
        diesel::sql_query(
            "SELECT code,date,status FROM qualified_daily_trading_status ORDER BY code,date",
        )
        .load(&mut conn)
        .unwrap()
    }

    #[derive(diesel::QueryableByName)]
    struct QualifiedDailySnapshotRow {
        #[diesel(sql_type = diesel::sql_types::Text)]
        serialized: String,
    }

    fn qualified_daily_snapshot(db: &DatabaseManager) -> Vec<String> {
        let mut conn = db.get_conn().unwrap();
        let daily = diesel::sql_query(
            "SELECT 'daily:' || quote(id) || '|' || quote(code) || '|' || quote(date) || \
             '|' || quote(open) || '|' || quote(high) || '|' || quote(low) || '|' || quote(close) || \
             '|' || quote(volume) || '|' || quote(amount) || '|' || quote(pct_chg) || \
             '|' || quote(ma5) || '|' || quote(ma10) || '|' || quote(ma20) || \
             '|' || quote(volume_ratio) || '|' || quote(data_source) || \
             '|' || quote(created_at) || '|' || quote(updated_at) || '|' || quote(is_limit_up) || \
             '|' || quote(is_limit_down) || '|' || quote(is_suspended) AS serialized \
             FROM stock_daily ORDER BY code,date",
        )
        .load::<QualifiedDailySnapshotRow>(&mut conn)
        .unwrap();
        let markers = diesel::sql_query(
            "SELECT 'marker:' || quote(rowid) || '|' || quote(code) || '|' || quote(date) || \
             '|' || quote(status) || '|' || quote(contract_version) || '|' || quote(source) || \
             '|' || quote(source_at) || '|' || quote(observed_at) || '|' || quote(batch_id) \
             AS serialized FROM qualified_daily_trading_status ORDER BY code,date",
        )
        .load::<QualifiedDailySnapshotRow>(&mut conn)
        .unwrap();
        daily
            .into_iter()
            .chain(markers)
            .map(|row| row.serialized)
            .collect()
    }

    fn qualified_daily_admitted(code: &str, bars: Vec<KlineData>) -> AdmittedDailyBars {
        AdmittedDailyBars::from_test_fixture(
            code,
            bars,
            BatchEvidence {
                provider: ProviderId::Tdx,
                source: "TEST_CODE_ordinary_daily".to_owned(),
                source_at: Some("2026-09-29".to_owned()),
                observed_at: "2026-09-29T15:01:00+08:00".to_owned(),
                batch_id: "TEST_CODE_ordinary_batch".to_owned(),
            },
        )
        .unwrap()
    }

    #[tokio::test]
    async fn qualified_daily_ordinary_savers_invalidate_only_exact_key() {
        use crate::database::repository::StockRepository;

        let day1 = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let day2 = NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();
        for saver in ["single", "batch", "raw_bars", "admitted", "repository"] {
            let (_dir, db) = qualified_daily_private_db();
            let code = "TEST_CODE_qualified_target";
            let other = "TEST_CODE_qualified_other";
            qualified_daily_seed(&db, code, day1, 10.0, "trading");
            qualified_daily_seed(&db, code, day2, 11.0, "suspended");
            qualified_daily_seed(&db, other, day1, 20.0, "trading");
            let record = qualified_daily_record(code, day1, 10.5);
            let bars = vec![kline(day1, 10.5, 0.0)];
            match saver {
                "single" => qualified_daily_save_single(&db, &record).unwrap(),
                "batch" => assert_eq!(db.save_daily_batch(&[record]).unwrap(), 1),
                "raw_bars" => {
                    assert_eq!(
                        db.save_kline_data(code, &bars, "TEST_CODE_daily").unwrap(),
                        1
                    )
                }
                "admitted" => assert_eq!(
                    db.save_admitted_kline_data(&qualified_daily_admitted(code, bars))
                        .unwrap(),
                    1
                ),
                "repository" => {
                    assert_eq!(
                        StockRepository::save_kline(&db, code, &bars).await.unwrap(),
                        1
                    )
                }
                _ => unreachable!(),
            }
            assert_eq!(
                qualified_daily_markers(&db),
                vec![
                    QualifiedDailyMarkerRow {
                        code: other.to_owned(),
                        date: day1.to_string(),
                        status: "trading".to_owned(),
                    },
                    QualifiedDailyMarkerRow {
                        code: code.to_owned(),
                        date: day2.to_string(),
                        status: "suspended".to_owned(),
                    },
                ],
                "{saver} must invalidate only the exact written key"
            );
            let rows = db.get_data_range(code, day1, day2).unwrap();
            assert_eq!((rows[0].close, rows[1].close), (Some(10.5), Some(11.0)));
            assert_eq!(db.get_latest_data(other, 1).unwrap()[0].close, Some(20.0));
        }
    }

    #[test]
    fn qualified_daily_identical_prices_and_both_statuses_lose_qualification() {
        let day = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        for status in ["trading", "suspended"] {
            let (_dir, db) = qualified_daily_private_db();
            let code = "TEST_CODE_qualified_identical";
            qualified_daily_seed(&db, code, day, 10.0, status);
            qualified_daily_save_single(&db, &qualified_daily_record(code, day, 10.0)).unwrap();
            assert!(qualified_daily_markers(&db).is_empty());
            assert_eq!(db.get_latest_data(code, 1).unwrap()[0].close, Some(10.0));
        }
    }

    #[test]
    fn qualified_daily_single_upsert_invalidation_failure_rolls_back_daily_and_marker() {
        let (_dir, db) = qualified_daily_private_db();
        let day = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let code = "TEST_CODE_qualified_single_rollback";
        qualified_daily_seed(&db, code, day, 10.0, "trading");
        diesel::sql_query(
            "CREATE TRIGGER TEST_CODE_reject_marker_delete \
             BEFORE DELETE ON qualified_daily_trading_status \
             BEGIN SELECT RAISE(ABORT,'TEST_CODE_marker_delete_blocked'); END",
        )
        .execute(&mut db.get_conn().unwrap())
        .unwrap();
        let before = qualified_daily_snapshot(&db);
        let error = qualified_daily_save_single(&db, &qualified_daily_record(code, day, 12.0))
            .expect_err("invalidation failure must fail the whole ordinary write");
        assert!(error
            .to_string()
            .contains("TEST_CODE_marker_delete_blocked"));
        assert_eq!(qualified_daily_snapshot(&db), before);
    }

    #[test]
    fn qualified_daily_batch_and_bar_later_invalidation_failure_rolls_back_all_rows() {
        let day1 = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let day2 = NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();
        for saver in ["batch", "raw_bars", "admitted"] {
            let (_dir, db) = qualified_daily_private_db();
            let code = "TEST_CODE_qualified_later_rollback";
            qualified_daily_seed(&db, code, day1, 10.0, "trading");
            qualified_daily_seed(&db, code, day2, 11.0, "suspended");
            diesel::sql_query(
                "CREATE TRIGGER TEST_CODE_reject_later_marker_delete \
                 BEFORE DELETE ON qualified_daily_trading_status WHEN OLD.date='2026-09-29' \
                 BEGIN SELECT RAISE(ABORT,'TEST_CODE_later_marker_delete_blocked'); END",
            )
            .execute(&mut db.get_conn().unwrap())
            .unwrap();
            let before = qualified_daily_snapshot(&db);
            let bars = vec![kline(day1, 10.5, 0.0), kline(day2, 11.55, 10.0)];
            let error = match saver {
                "batch" => db.save_daily_batch(&[
                    qualified_daily_record(code, day1, 10.5),
                    qualified_daily_record(code, day2, 11.55),
                ]),
                "raw_bars" => db.save_kline_data(code, &bars, "TEST_CODE_daily"),
                "admitted" => db.save_admitted_kline_data(&qualified_daily_admitted(code, bars)),
                _ => unreachable!(),
            }
            .expect_err("later invalidation failure must roll back every earlier row");
            assert!(error
                .to_string()
                .contains("TEST_CODE_later_marker_delete_blocked"));
            assert_eq!(qualified_daily_snapshot(&db), before, "{saver}");
        }
    }

    #[test]
    fn qualified_daily_empty_and_rejected_batches_preserve_rows_and_markers() {
        let (_dir, db) = qualified_daily_private_db();
        let day = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let code = "TEST_CODE_qualified_rejected";
        qualified_daily_seed(&db, code, day, 10.0, "trading");
        let before = qualified_daily_snapshot(&db);
        assert_eq!(db.save_daily_batch(&[]).unwrap(), 0);
        assert_eq!(db.save_kline_data(code, &[], "TEST_CODE_daily").unwrap(), 0);
        let mut invalid = kline(day, 10.0, 0.0);
        invalid.amount = 0.0;
        assert!(db
            .save_kline_data(code, &[invalid], "TEST_CODE_daily")
            .is_err());
        assert_eq!(qualified_daily_snapshot(&db), before);
    }

    #[test]
    fn qualified_daily_delete_clears_only_requested_code_and_orphans_with_daily_count() {
        let (_dir, db) = qualified_daily_private_db();
        let day1 = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let day2 = NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();
        let day3 = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        let code = "TEST_CODE_qualified_delete";
        let other = "TEST_CODE_qualified_other";
        let orphan_only = "TEST_CODE_qualified_orphan_only";
        qualified_daily_seed(&db, code, day1, 10.0, "trading");
        qualified_daily_seed(&db, code, day2, 11.0, "suspended");
        qualified_daily_test_marker(&db, code, day3, "trading");
        qualified_daily_seed(&db, other, day1, 20.0, "trading");
        qualified_daily_test_marker(&db, orphan_only, day1, "suspended");
        assert_eq!(db.delete_stock_data(code).unwrap(), 2);
        assert!(db.get_latest_data(code, 10).unwrap().is_empty());
        assert_eq!(db.delete_stock_data(orphan_only).unwrap(), 0);
        assert_eq!(
            qualified_daily_markers(&db),
            vec![QualifiedDailyMarkerRow {
                code: other.to_owned(),
                date: day1.to_string(),
                status: "trading".to_owned(),
            }]
        );
        assert_eq!(db.get_latest_data(other, 1).unwrap()[0].close, Some(20.0));
    }

    #[test]
    fn qualified_daily_delete_invalidation_failure_rolls_back_rows_and_markers() {
        let (_dir, db) = qualified_daily_private_db();
        let day1 = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let day2 = NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();
        let code = "TEST_CODE_qualified_delete_rollback";
        qualified_daily_seed(&db, code, day1, 10.0, "trading");
        qualified_daily_seed(&db, code, day2, 11.0, "suspended");
        diesel::sql_query(
            "CREATE TRIGGER TEST_CODE_reject_delete_invalidation \
             BEFORE DELETE ON qualified_daily_trading_status \
             BEGIN SELECT RAISE(ABORT,'TEST_CODE_delete_invalidation_blocked'); END",
        )
        .execute(&mut db.get_conn().unwrap())
        .unwrap();
        let before = qualified_daily_snapshot(&db);
        let error = db.delete_stock_data(code).unwrap_err();
        assert!(error
            .to_string()
            .contains("TEST_CODE_delete_invalidation_blocked"));
        assert_eq!(qualified_daily_snapshot(&db), before);
    }

    #[tokio::test]
    async fn qualified_daily_ordinary_overwrite_defers_actual_prediction_verifier_without_mutation()
    {
        let (_dir, db) = qualified_daily_private_db();
        let day1 = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let day2 = NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();
        let code = "TEST_CODE_qualified_prediction";
        db.save_prediction_legacy(
            &day1.to_string(),
            &day2.to_string(),
            None,
            Some(code),
            "up",
            80.0,
            None,
        )
        .unwrap();
        qualified_daily_seed(&db, code, day1, 10.0, "trading");
        qualified_daily_seed(&db, code, day2, 11.0, "trading");
        let before = serde_json::to_value(
            db.get_prediction_by_code_date(code, &day1.to_string())
                .unwrap(),
        )
        .unwrap();
        let original = crate::monitor::prediction::verify_one(
            &db,
            code,
            &day1.to_string(),
            &day2.to_string(),
            "up",
        )
        .await
        .expect("synthetic qualified exact closes permit the original read-only observation");
        assert!(original.hit);
        assert!((original.actual_change - 10.0).abs() < 1e-9);

        qualified_daily_save_single(&db, &qualified_daily_record(code, day2, 11.0)).unwrap();
        assert!(crate::monitor::prediction::verify_one(
            &db,
            code,
            &day1.to_string(),
            &day2.to_string(),
            "up",
        )
        .await
        .is_none());
        let after = db
            .get_prediction_by_code_date(code, &day1.to_string())
            .unwrap();
        assert!(after.actual_change.is_none());
        assert!(after.hit.is_none());
        assert!(after.actual_result.is_none());
        assert_eq!(serde_json::to_value(after).unwrap(), before);
    }
}
