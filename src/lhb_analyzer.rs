//! Registered business rules: BR-127.
//! 龙虎榜数据分析模块
//!
//! 功能：
//! 1. 获取个股龙虎榜数据
//! 2. 获取每日龙虎榜汇总
//! 3. 分析机构和游资动向
//! 4. 作为选股指标

use crate::database::DatabaseManager;
use crate::models::{LhbDaily, NewLhbDaily};
use anyhow::{Context, Result};
use chrono::Local;
use serde::{Deserialize, Serialize};

/// 龙虎榜记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LhbRecord {
    /// 股票代码
    pub code: String,
    /// 股票名称
    pub name: String,
    /// 上榜日期
    pub trade_date: String,
    /// 上榜原因
    pub reason: String,
    /// 涨跌幅(%)
    pub pct_change: f64,
    /// 收盘价
    pub close_price: f64,
    /// 龙虎榜买入额(万元)
    pub buy_amount: f64,
    /// 龙虎榜卖出额(万元)
    pub sell_amount: f64,
    /// 龙虎榜净买额(万元)
    pub net_amount: f64,
    /// 市场总成交额(万元)
    pub total_amount: f64,
    /// 龙虎榜成交占比(%)
    pub lhb_ratio: f64,
    /// 机构买入席位数
    pub inst_buy_seats: i32,
    /// 机构卖出席位数
    pub inst_sell_seats: i32,
    /// 机构净买入额(万元)
    pub inst_net_amount: f64,
}

/// 龙虎榜席位明细
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LhbSeat {
    /// 席位名称
    pub seat_name: String,
    /// 买入额(万元)
    pub buy_amount: f64,
    /// 卖出额(万元)
    pub sell_amount: f64,
    /// 净额(万元)
    pub net_amount: f64,
    /// 席位类型（机构/游资）
    pub seat_type: String,
}

/// 龙虎榜分析结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LhbAnalysis {
    /// 股票代码
    pub code: String,
    /// 股票名称
    pub name: String,
    /// 最近上榜次数（30天内）
    pub recent_count: i32,
    /// 机构参与度评分 (0-100)
    pub inst_score: i32,
    /// 游资活跃度评分 (0-100)
    pub hot_money_score: i32,
    /// 综合评分 (0-100)
    pub total_score: i32,
    /// 推荐理由
    pub reason: String,
    /// 风险提示
    pub risk_warning: String,
}

/// 龙虎榜数据获取器
pub struct LhbDataFetcher {
    client: reqwest::Client,
}

impl LhbDataFetcher {
    /// 创建新实例
    /// review #14: 改用 SHARED_HTTP_CLIENT 共享 client, 避免每次 new Client
    /// 触发 TLS handshake. screen_lhb_stocks 循环 N 次调 new() 浪费数百 ms.
    pub fn new() -> Result<Self> {
        let client = crate::http_client::SHARED_HTTP_CLIENT.clone();
        Ok(Self { client })
    }

    /// 获取今日龙虎榜数据（优先从数据库缓存读取）
    pub async fn get_today_lhb(&self) -> Result<Vec<LhbRecord>> {
        let today = Local::now().format("%Y%m%d").to_string();
        self.get_lhb_by_date(&today).await
    }

    /// 获取指定日期的龙虎榜数据（优先从数据库缓存读取）
    pub async fn get_lhb_by_date(&self, date: &str) -> Result<Vec<LhbRecord>> {
        let date_normalized = Self::normalize_date(date)?;

        log::info!(
            "[龙虎榜] 正在查询 {} 的数据（标准化后：{}）",
            date,
            date_normalized
        );

        // 1. 先尝试从数据库读取缓存（支持模糊匹配日期）
        if let Some(db) = DatabaseManager::try_get() {
            let cached_records = db
                .get_lhb_by_date(&date_normalized)
                .map_err(|error| anyhow::anyhow!("龙虎榜缓存读取失败: {error}"))?;
            if !cached_records.is_empty() {
                log::info!(
                    "[龙虎榜] 从数据库缓存读取 {} 的数据 ({} 条记录)",
                    date,
                    cached_records.len()
                );
                return Ok(Self::convert_db_to_records(cached_records));
            }
            log::info!("[龙虎榜] 数据库中没有 {} 的缓存数据", date);
        }

        // 2. 如果缓存不存在，从API获取
        log::info!("[龙虎榜] 从API获取 {} 的数据", date);
        log::info!("[龙虎榜] 从API获取{}的数据...", date);
        let records = self.fetch_lhb_from_api(&date_normalized).await?;

        // 3. 保存到数据库缓存
        let db = DatabaseManager::try_get()
            .ok_or_else(|| anyhow::anyhow!("龙虎榜缓存数据库未初始化"))?;
        let new_records: Vec<NewLhbDaily> =
            records.iter().map(Self::convert_record_to_db).collect();
        let saved = db
            .save_lhb_records(&new_records)
            .map_err(|error| anyhow::anyhow!("龙虎榜缓存写入失败: {error}"))?;
        log::info!("[龙虎榜] 已缓存 {} 条记录到数据库", saved);

        Ok(records)
    }

    /// 从API获取龙虎榜数据
    async fn fetch_lhb_from_api(&self, date: &str) -> Result<Vec<LhbRecord>> {
        let date_formatted = Self::normalize_date(date)?;

        let url = format!(
            "http://datacenter-web.eastmoney.com/api/data/v1/get?\
            reportName=RPT_DAILYBILLBOARD_DETAILS\
            &columns=SECURITY_CODE,SECUCODE,SECURITY_NAME_ABBR,TRADE_DATE,EXPLAIN,CLOSE_PRICE,CHANGE_RATE,\
            BILLBOARD_BUY_AMT,BILLBOARD_SELL_AMT,BILLBOARD_NET_AMT,ACCUM_AMOUNT,DEAL_NET_RATIO\
            &filter=(TRADE_DATE='{}')\
            &pageNumber=1&pageSize=500&sortTypes=-1&sortColumns=BILLBOARD_NET_AMT",
            date_formatted
        );

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("请求龙虎榜数据失败")?
            .error_for_status()
            .context("龙虎榜 HTTP 状态失败")?;

        let text = response.text().await?;

        // 解析JSON
        let json: serde_json::Value = serde_json::from_str(&text).context("解析龙虎榜JSON失败")?;

        let data = json
            .pointer("/result/data")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("龙虎榜响应缺少 result.data 数组"))?;
        let records = self.parse_lhb_batch(data)?;

        log::info!("[龙虎榜] API返回 {} 条记录", records.len());
        Ok(records)
    }

    /// 将数据库记录转换为LhbRecord
    fn convert_db_to_records(db_records: Vec<LhbDaily>) -> Vec<LhbRecord> {
        db_records
            .into_iter()
            .map(|db| LhbRecord {
                code: db.code,
                name: db.name,
                trade_date: db.trade_date,
                reason: db.reason,
                pct_change: db.pct_change,
                close_price: db.close_price,
                buy_amount: db.buy_amount,
                sell_amount: db.sell_amount,
                net_amount: db.net_amount,
                total_amount: db.total_amount,
                lhb_ratio: db.lhb_ratio,
                inst_buy_seats: 0,
                inst_sell_seats: 0,
                inst_net_amount: 0.0,
            })
            .collect()
    }

    /// 将LhbRecord转换为数据库记录
    fn convert_record_to_db(record: &LhbRecord) -> NewLhbDaily {
        NewLhbDaily {
            code: record.code.clone(),
            name: record.name.clone(),
            trade_date: record.trade_date.clone(),
            reason: record.reason.clone(),
            pct_change: record.pct_change,
            close_price: record.close_price,
            buy_amount: record.buy_amount,
            sell_amount: record.sell_amount,
            net_amount: record.net_amount,
            total_amount: record.total_amount,
            lhb_ratio: record.lhb_ratio,
        }
    }

    /// 获取个股龙虎榜历史（最近N天）
    pub async fn get_stock_lhb_history(&self, code: &str, days: i32) -> Result<Vec<LhbRecord>> {
        // 计算日期范围
        let end_date = Local::now();
        let start_date = end_date - chrono::Duration::days(days as i64);

        let url = format!(
            "http://datacenter-web.eastmoney.com/api/data/v1/get?\
            reportName=RPT_DAILYBILLBOARD_DETAILS\
            &columns=SECURITY_CODE,SECUCODE,SECURITY_NAME_ABBR,TRADE_DATE,EXPLAIN,CLOSE_PRICE,CHANGE_RATE,\
            BILLBOARD_BUY_AMT,BILLBOARD_SELL_AMT,BILLBOARD_NET_AMT,ACCUM_AMOUNT,DEAL_NET_RATIO\
            &filter=(SECURITY_CODE=\"{}\")(TRADE_DATE>='{}')(TRADE_DATE<='{}')\
            &pageNumber=1&pageSize=100&sortTypes=-1&sortColumns=TRADE_DATE",
            code,
            start_date.format("%Y-%m-%d"),
            end_date.format("%Y-%m-%d")
        );

        log::info!("[龙虎榜] 获取{}最近{}天的数据...", code, days);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("请求个股龙虎榜数据失败")?
            .error_for_status()
            .context("个股龙虎榜 HTTP 状态失败")?;

        let text = response.text().await?;
        let json: serde_json::Value =
            serde_json::from_str(&text).context("解析个股龙虎榜JSON失败")?;

        let data = json
            .pointer("/result/data")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("个股龙虎榜响应缺少 result.data 数组"))?;
        let records = self.parse_lhb_batch(data)?;

        log::info!("[龙虎榜] {} 最近{}天上榜 {} 次", code, days, records.len());
        Ok(records)
    }

    /// 解析单条龙虎榜记录
    fn parse_lhb_batch(&self, data: &[serde_json::Value]) -> Result<Vec<LhbRecord>> {
        let records = data
            .iter()
            .enumerate()
            .map(|(index, item)| {
                self.parse_lhb_record(item)
                    .with_context(|| format!("龙虎榜第 {} 行解析失败", index + 1))
            })
            .collect::<Result<Vec<_>>>()?;
        let db_records = records
            .iter()
            .map(Self::convert_record_to_db)
            .collect::<Vec<_>>();
        crate::database::validate_lhb_records(&db_records)
            .map_err(|error| anyhow::anyhow!("龙虎榜批次校验失败: {error}"))?;
        Ok(records)
    }

    fn parse_lhb_record(&self, item: &serde_json::Value) -> Result<LhbRecord> {
        let required_text = |field: &str| -> Result<String> {
            item.get(field)
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
                .ok_or_else(|| anyhow::anyhow!("{field} 缺失或为空"))
        };
        let required_number = |field: &str| -> Result<f64> {
            let value = item
                .get(field)
                .ok_or_else(|| anyhow::anyhow!("{field} 缺失"))?;
            let parsed = match value {
                serde_json::Value::Number(number) => number.as_f64(),
                serde_json::Value::String(text) => text.trim().parse::<f64>().ok(),
                _ => None,
            }
            .ok_or_else(|| anyhow::anyhow!("{field} 不是有效数值"))?;
            if !parsed.is_finite() {
                anyhow::bail!("{field} 非有限");
            }
            Ok(parsed)
        };
        Ok(LhbRecord {
            code: required_text("SECURITY_CODE")?,
            name: required_text("SECURITY_NAME_ABBR")?,
            trade_date: required_text("TRADE_DATE")?,
            reason: required_text("EXPLAIN")?,
            pct_change: required_number("CHANGE_RATE")?,
            close_price: required_number("CLOSE_PRICE")?,
            buy_amount: required_number("BILLBOARD_BUY_AMT")?,
            sell_amount: required_number("BILLBOARD_SELL_AMT")?,
            net_amount: required_number("BILLBOARD_NET_AMT")?,
            total_amount: required_number("ACCUM_AMOUNT")?,
            lhb_ratio: required_number("DEAL_NET_RATIO")?,
            inst_buy_seats: 0, // 需要额外接口获取
            inst_sell_seats: 0,
            inst_net_amount: 0.0,
        })
    }

    fn normalize_date(date: &str) -> Result<String> {
        let normalized = if date.len() == 8 && date.bytes().all(|byte| byte.is_ascii_digit()) {
            format!("{}-{}-{}", &date[0..4], &date[4..6], &date[6..8])
        } else {
            date.to_string()
        };
        chrono::NaiveDate::parse_from_str(&normalized, "%Y-%m-%d")
            .map_err(|error| anyhow::anyhow!("龙虎榜日期非法 {date:?}: {error}"))?;
        Ok(normalized)
    }

    /// 分析个股龙虎榜数据，生成选股指标
    pub async fn analyze_stock_lhb(&self, code: &str) -> Result<LhbAnalysis> {
        let records = self.get_stock_lhb_history(code, 30).await?;

        let recent_count = records.len() as i32;

        // 计算机构参与度
        let inst_score = self.calculate_inst_score(&records);

        // 计算游资活跃度
        let hot_money_score = self.calculate_hot_money_score(&records);

        // 综合评分
        let total_score = (inst_score * 6 + hot_money_score * 4) / 10;

        // 生成推荐理由
        let reason = self.generate_recommendation(&records, inst_score, hot_money_score);

        // 风险提示
        let risk_warning = self.generate_risk_warning(&records);

        let name = records
            .first()
            .map(|r| r.name.clone())
            .unwrap_or_else(|| "未知".to_string());

        Ok(LhbAnalysis {
            code: code.to_string(),
            name,
            recent_count,
            inst_score,
            hot_money_score,
            total_score,
            reason,
            risk_warning,
        })
    }

    /// 计算机构参与度评分
    fn calculate_inst_score(&self, records: &[LhbRecord]) -> i32 {
        if records.is_empty() {
            return 0;
        }

        let len = records.len();
        let len_f = len as f64;

        // 统计净买入为正的次数
        let positive_count = records.iter().filter(|r| r.net_amount > 0.0).count();

        // 计算平均净买入额
        let avg_net_amount: f64 = records.iter().map(|r| r.net_amount).sum::<f64>() / len_f;

        // 评分逻辑
        let mut score = 0;

        // 上榜频率加分
        score += (len * 10).min(30) as i32;

        // 净买入为正占比加分
        let positive_ratio = positive_count as f64 / len_f;
        score += (positive_ratio * 40.0) as i32;

        // 平均净买入额加分
        if avg_net_amount > 5000.0 {
            score += 30;
        } else if avg_net_amount > 1000.0 {
            score += 20;
        } else if avg_net_amount > 0.0 {
            score += 10;
        }

        score.min(100)
    }

    /// 计算游资活跃度评分
    fn calculate_hot_money_score(&self, records: &[LhbRecord]) -> i32 {
        if records.is_empty() {
            return 0;
        }

        let len = records.len();
        let len_f = len as f64;

        let mut score = 0;

        // 上榜频率
        score += (len * 15).min(40) as i32;

        // 龙虎榜成交占比
        let avg_ratio: f64 = records.iter().map(|r| r.lhb_ratio).sum::<f64>() / len_f;

        if avg_ratio > 20.0 {
            score += 40;
        } else if avg_ratio > 10.0 {
            score += 30;
        } else if avg_ratio > 5.0 {
            score += 20;
        }

        // 涨跌幅
        let avg_pct: f64 = records.iter().map(|r| r.pct_change.abs()).sum::<f64>() / len_f;

        if avg_pct > 8.0 {
            score += 20;
        } else if avg_pct > 5.0 {
            score += 10;
        }

        score.min(100)
    }

    /// 生成推荐理由
    fn generate_recommendation(
        &self,
        records: &[LhbRecord],
        inst_score: i32,
        hot_money_score: i32,
    ) -> String {
        if records.is_empty() {
            return "近期未上榜龙虎榜".to_string();
        }

        let mut reasons = Vec::new();

        if inst_score >= 70 {
            reasons.push("机构高度参与，资金实力雄厚".to_string());
        } else if inst_score >= 50 {
            reasons.push("机构适度关注".to_string());
        }

        if hot_money_score >= 70 {
            reasons.push("游资高度活跃，题材热度高".to_string());
        } else if hot_money_score >= 50 {
            reasons.push("游资参与度较高".to_string());
        }

        // 统计净买入情况
        let net_buy_count = records.iter().filter(|r| r.net_amount > 0.0).count();

        if net_buy_count > records.len() / 2 {
            reasons.push(format!(
                "最近{}次上榜中{}次为净买入",
                records.len(),
                net_buy_count
            ));
        }

        // 最近一次上榜
        if let Some(latest) = records.first() {
            if latest.net_amount > 1000.0 {
                reasons.push(format!("最近上榜净买入{:.0}万元", latest.net_amount));
            }
        }

        if reasons.is_empty() {
            "龙虎榜数据一般，建议谨慎".to_string()
        } else {
            reasons.join("；")
        }
    }

    /// 生成风险提示
    fn generate_risk_warning(&self, records: &[LhbRecord]) -> String {
        if records.is_empty() {
            return "".to_string();
        }

        let mut warnings = Vec::new();

        // 检查是否频繁上榜但净卖出
        let net_sell_count = records.iter().filter(|r| r.net_amount < -1000.0).count();

        if net_sell_count > records.len() / 2 {
            warnings.push("频繁出现大额净卖出，资金流出风险");
        }

        // 检查涨跌幅波动
        let avg_pct: f64 =
            records.iter().map(|r| r.pct_change.abs()).sum::<f64>() / records.len() as f64;

        if avg_pct > 9.0 {
            warnings.push("波动剧烈，短线炒作风险高");
        }

        // 检查最近是否大幅下跌上榜
        if let Some(latest) = records.first() {
            if latest.pct_change < -7.0 {
                warnings.push("最近因大跌上榜，注意止损");
            }
        }

        warnings.join("；")
    }

    /// 筛选优质龙虎榜股票
    pub async fn screen_lhb_stocks(&self, min_score: i32) -> Result<Vec<LhbAnalysis>> {
        // 获取今日龙虎榜
        let today_lhb = self.get_today_lhb().await?;

        let mut results = Vec::new();

        for record in today_lhb {
            // 分析每只股票
            if let Ok(analysis) = self.analyze_stock_lhb(&record.code).await {
                if analysis.total_score >= min_score {
                    results.push(analysis);
                }
            }

            // 避免请求过快
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        }

        // 按评分排序
        results.sort_by_key(|item| std::cmp::Reverse(item.total_score));

        Ok(results)
    }
}

impl Default for LhbDataFetcher {
    fn default() -> Self {
        Self::new().expect("创建LhbDataFetcher失败")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diesel::prelude::*;

    fn fetcher() -> LhbDataFetcher {
        LhbDataFetcher::new().expect("shared client must build")
    }

    fn protocol_row(code: &str) -> serde_json::Value {
        serde_json::json!({
            "SECURITY_CODE": code,
            "SECURITY_NAME_ABBR": "测试龙虎榜",
            "TRADE_DATE": "2099-07-16 00:00:00",
            "EXPLAIN": "测试完整协议",
            "CHANGE_RATE": "9.0",
            "CLOSE_PRICE": 10.0,
            "BILLBOARD_BUY_AMT": 10_000.0,
            "BILLBOARD_SELL_AMT": 4_000.0,
            "BILLBOARD_NET_AMT": 6_000.0,
            "ACCUM_AMOUNT": 20_000.0,
            "DEAL_NET_RATIO": 25.0
        })
    }

    fn record(net_amount: f64, pct_change: f64, ratio: f64) -> LhbRecord {
        LhbRecord {
            code: "TEST_CODE_LHB_SCORE".to_string(),
            name: "测试评分".to_string(),
            trade_date: "2099-07-16".to_string(),
            reason: "测试完整事实".to_string(),
            pct_change,
            close_price: 10.0,
            buy_amount: 10_000.0,
            sell_amount: 10_000.0 - net_amount,
            net_amount,
            total_amount: 20_000.0,
            lhb_ratio: ratio,
            inst_buy_seats: 0,
            inst_sell_seats: 0,
            inst_net_amount: 0.0,
        }
    }

    #[tokio::test]
    async fn br127_cached_lhb_query_uses_validated_database_facts_without_network() {
        DatabaseManager::init(None).expect("test database init");
        let db = DatabaseManager::get();
        let code = format!(
            "TEST_CODE_LHB_CACHE_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        );
        let parsed = fetcher()
            .parse_lhb_batch(&[protocol_row(&code)])
            .expect("complete local provider row");
        let db_rows = parsed
            .iter()
            .map(LhbDataFetcher::convert_record_to_db)
            .collect::<Vec<_>>();
        db.save_lhb_records(&db_rows).expect("cache local row");

        let loaded = fetcher()
            .get_lhb_by_date("20990716")
            .await
            .expect("cache hit must not use network");
        let ours = loaded
            .iter()
            .find(|row| row.code == code)
            .expect("cached row is returned");
        assert_eq!(ours.name, "测试龙虎榜");
        assert_eq!(ours.trade_date, "2099-07-16 00:00:00");
        assert_eq!(ours.net_amount, 6_000.0);
        assert_eq!(ours.inst_buy_seats, 0);

        let mut conn = db.get_conn().expect("test database connection");
        diesel::delete(
            crate::schema::lhb_daily::table.filter(crate::schema::lhb_daily::code.eq(&code)),
        )
        .execute(&mut conn)
        .expect("clean cached LHB fixture");
    }

    #[test]
    fn br127_protocol_parser_rejects_missing_or_bad_rows_as_a_complete_batch() {
        let fetcher = fetcher();
        let complete = fetcher
            .parse_lhb_batch(&[protocol_row("TEST_CODE_LHB_PROTOCOL")])
            .expect("complete row parses");
        assert_eq!(complete.len(), 1);
        assert_eq!(complete[0].pct_change, 9.0);
        assert_eq!(complete[0].close_price, 10.0);

        for bad in [
            serde_json::json!({}),
            serde_json::json!({"SECURITY_CODE": "TEST_CODE_BAD"}),
            {
                let mut row = protocol_row("TEST_CODE_BAD_TEXT");
                row["EXPLAIN"] = serde_json::Value::String(" ".to_string());
                row
            },
            {
                let mut row = protocol_row("TEST_CODE_BAD_NUMBER");
                row["CHANGE_RATE"] = serde_json::Value::String("bad".to_string());
                row
            },
            {
                let mut row = protocol_row("TEST_CODE_MISSING_NUMBER");
                row.as_object_mut().unwrap().remove("CLOSE_PRICE");
                row
            },
        ] {
            assert!(fetcher.parse_lhb_batch(&[bad]).is_err());
        }
        assert!(LhbDataFetcher::normalize_date("20260230").is_err());
        assert!(LhbDataFetcher::normalize_date("not-a-date").is_err());
        assert_eq!(
            LhbDataFetcher::normalize_date("20990716").unwrap(),
            "2099-07-16"
        );
    }

    #[test]
    fn lhb_scoring_and_text_cover_positive_neutral_and_risk_bands() {
        let fetcher = fetcher();
        assert_eq!(fetcher.calculate_inst_score(&[]), 0);
        assert_eq!(fetcher.calculate_hot_money_score(&[]), 0);
        assert_eq!(
            fetcher.generate_recommendation(&[], 0, 0),
            "近期未上榜龙虎榜"
        );
        assert!(fetcher.generate_risk_warning(&[]).is_empty());

        let strong = vec![record(6_000.0, 9.5, 25.0); 3];
        assert_eq!(fetcher.calculate_inst_score(&strong), 100);
        assert_eq!(fetcher.calculate_hot_money_score(&strong), 100);
        let recommendation = fetcher.generate_recommendation(&strong, 100, 100);
        assert!(recommendation.contains("机构高度参与"));
        assert!(recommendation.contains("游资高度活跃"));
        assert!(recommendation.contains("3次为净买入"));
        assert!(recommendation.contains("净买入6000万元"));

        let moderate = vec![record(1_500.0, 6.0, 15.0); 2];
        assert!(fetcher.calculate_inst_score(&moderate) >= 50);
        assert!(fetcher.calculate_hot_money_score(&moderate) >= 50);
        let moderate_text = fetcher.generate_recommendation(&moderate, 50, 50);
        assert!(moderate_text.contains("机构适度关注"));
        assert!(moderate_text.contains("游资参与度较高"));

        let neutral = vec![record(0.0, 1.0, 1.0)];
        assert_eq!(
            fetcher.generate_recommendation(&neutral, 0, 0),
            "龙虎榜数据一般，建议谨慎"
        );
        let risk = vec![record(-2_000.0, -10.0, 5.0); 3];
        let warning = fetcher.generate_risk_warning(&risk);
        assert!(warning.contains("大额净卖出"));
        assert!(warning.contains("波动剧烈"));
        assert!(warning.contains("大跌上榜"));
    }
}
