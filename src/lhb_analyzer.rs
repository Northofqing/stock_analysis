//! 龙虎榜数据分析模块
//!
//! 功能：
//! 1. 获取个股龙虎榜数据
//! 2. 获取每日龙虎榜汇总
//! 3. 分析机构和游资动向
//! 4. 作为选股指标

use anyhow::{Result, Context};
use chrono::Local;
use serde::{Deserialize, Serialize};
use crate::database::DatabaseManager;
use crate::models::{NewLhbDaily, LhbDaily};

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
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
            .build()?;
        
        Ok(Self { client })
    }

    /// 获取今日龙虎榜数据（优先从数据库缓存读取）
    pub async fn get_today_lhb(&self) -> Result<Vec<LhbRecord>> {
        let today = Local::now().format("%Y%m%d").to_string();
        self.get_lhb_by_date(&today).await
    }

    /// 获取指定日期的龙虎榜数据（优先从数据库缓存读取）
    pub async fn get_lhb_by_date(&self, date: &str) -> Result<Vec<LhbRecord>> {
        // 标准化日期格式：20260128 -> 2026-01-28
        let date_normalized = if date.len() == 8 {
            format!("{}-{}-{}", &date[0..4], &date[4..6], &date[6..8])
        } else {
            date.to_string()
        };
        
        log::info!("[龙虎榜] 正在查询 {} 的数据（标准化后：{}）", date, date_normalized);
        
        // 1. 先尝试从数据库读取缓存（支持模糊匹配日期）
        if let Ok(db) = std::panic::catch_unwind(|| DatabaseManager::get()) {
            if let Ok(cached_records) = db.get_lhb_by_date(&date_normalized) {
                if !cached_records.is_empty() {
                    log::info!("[龙虎榜] 从数据库缓存读取 {} 的数据 ({} 条记录)", date, cached_records.len());
                    return Ok(Self::convert_db_to_records(cached_records));
                } else {
                    log::info!("[龙虎榜] 数据库中没有 {} 的缓存数据", date);
                }
            }
        }

        // 2. 如果缓存不存在，从API获取
        log::info!("[龙虎榜] 从API获取 {} 的数据", date);
        log::info!("[龙虎榜] 从API获取{}的数据...", date);
        let records = self.fetch_lhb_from_api(date).await?;

        // 3. 保存到数据库缓存
        if let Ok(db) = std::panic::catch_unwind(|| DatabaseManager::get()) {
            let new_records: Vec<NewLhbDaily> = records
                .iter()
                .map(|r| Self::convert_record_to_db(r))
                .collect();
            
            if let Ok(saved) = db.save_lhb_records(&new_records) {
                log::info!("[龙虎榜] 已缓存 {} 条记录到数据库", saved);
            }
        }

        Ok(records)
    }

    /// 从API获取龙虎榜数据
    async fn fetch_lhb_from_api(&self, date: &str) -> Result<Vec<LhbRecord>> {
        // 转换日期格式：20260128 -> 2026-01-28
        let date_formatted = if date.len() == 8 {
            format!("{}-{}-{}", &date[0..4], &date[4..6], &date[6..8])
        } else {
            date.to_string()
        };
        
        let url = format!(
            "http://datacenter-web.eastmoney.com/api/data/v1/get?\
            reportName=RPT_DAILYBILLBOARD_DETAILS\
            &columns=SECURITY_CODE,SECUCODE,SECURITY_NAME_ABBR,TRADE_DATE,EXPLAIN,CLOSE_PRICE,CHANGE_RATE,\
            BILLBOARD_BUY_AMT,BILLBOARD_SELL_AMT,BILLBOARD_NET_AMT,ACCUM_AMOUNT,DEAL_NET_RATIO\
            &filter=(TRADE_DATE='{}')\
            &pageNumber=1&pageSize=500&sortTypes=-1&sortColumns=BILLBOARD_NET_AMT",
            date_formatted
        );

        let response = self.client
            .get(&url)
            .send()
            .await
            .context("请求龙虎榜数据失败")?;

        let text = response.text().await?;
        
        // 解析JSON
        let json: serde_json::Value = serde_json::from_str(&text)
            .context("解析龙虎榜JSON失败")?;

        let mut records = Vec::new();

        if let Some(data) = json["result"]["data"].as_array() {
            for item in data {
                if let Some(record) = self.parse_lhb_record(item) {
                    records.push(record);
                }
            }
        }

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

        let response = self.client
            .get(&url)
            .send()
            .await
            .context("请求个股龙虎榜数据失败")?;

        let text = response.text().await?;
        let json: serde_json::Value = serde_json::from_str(&text)
            .context("解析个股龙虎榜JSON失败")?;

        let mut records = Vec::new();

        if let Some(data) = json["result"]["data"].as_array() {
            for item in data {
                if let Some(record) = self.parse_lhb_record(item) {
                    records.push(record);
                }
            }
        }

        log::info!("[龙虎榜] {} 最近{}天上榜 {} 次", code, days, records.len());
        Ok(records)
    }

    /// 解析单条龙虎榜记录
    fn parse_lhb_record(&self, item: &serde_json::Value) -> Option<LhbRecord> {
        Some(LhbRecord {
            code: item["SECURITY_CODE"].as_str()?.to_string(),
            name: item["SECURITY_NAME_ABBR"].as_str()?.to_string(),
            trade_date: item["TRADE_DATE"].as_str()?.to_string(),
            reason: item["EXPLAIN"].as_str().unwrap_or("").to_string(),
            pct_change: item["CHANGE_RATE"].as_f64().unwrap_or(0.0),
            close_price: item["CLOSE_PRICE"].as_f64().unwrap_or(0.0),
            buy_amount: item["BILLBOARD_BUY_AMT"].as_f64().unwrap_or(0.0),
            sell_amount: item["BILLBOARD_SELL_AMT"].as_f64().unwrap_or(0.0),
            net_amount: item["BILLBOARD_NET_AMT"].as_f64().unwrap_or(0.0),
            total_amount: item["ACCUM_AMOUNT"].as_f64().unwrap_or(0.0),
            lhb_ratio: item["DEAL_NET_RATIO"].as_f64().unwrap_or(0.0),
            inst_buy_seats: 0,  // 需要额外接口获取
            inst_sell_seats: 0,
            inst_net_amount: 0.0,
        })
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
        
        let name = records.first()
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
        
        // 统计净买入为正的次数
        let positive_count = records.iter()
            .filter(|r| r.net_amount > 0.0)
            .count();
        
        // 计算平均净买入额
        let avg_net_amount: f64 = records.iter()
            .map(|r| r.net_amount)
            .sum::<f64>() / records.len() as f64;
        
        // 评分逻辑
        let mut score = 0;
        
        // 上榜频率加分
        score += (records.len() * 10).min(30) as i32;
        
        // 净买入为正占比加分
        let positive_ratio = positive_count as f64 / records.len() as f64;
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
        
        let mut score = 0;
        
        // 上榜频率
        score += (records.len() * 15).min(40) as i32;
        
        // 龙虎榜成交占比
        let avg_ratio: f64 = records.iter()
            .map(|r| r.lhb_ratio)
            .sum::<f64>() / records.len() as f64;
        
        if avg_ratio > 20.0 {
            score += 40;
        } else if avg_ratio > 10.0 {
            score += 30;
        } else if avg_ratio > 5.0 {
            score += 20;
        }
        
        // 涨跌幅
        let avg_pct: f64 = records.iter()
            .map(|r| r.pct_change.abs())
            .sum::<f64>() / records.len() as f64;
        
        if avg_pct > 8.0 {
            score += 20;
        } else if avg_pct > 5.0 {
            score += 10;
        }
        
        score.min(100)
    }

    /// 生成推荐理由
    fn generate_recommendation(&self, records: &[LhbRecord], inst_score: i32, hot_money_score: i32) -> String {
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
        let net_buy_count = records.iter()
            .filter(|r| r.net_amount > 0.0)
            .count();
        
        if net_buy_count > records.len() / 2 {
            reasons.push(format!("最近{}次上榜中{}次为净买入", records.len(), net_buy_count));
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
        let net_sell_count = records.iter()
            .filter(|r| r.net_amount < -1000.0)
            .count();
        
        if net_sell_count > records.len() / 2 {
            warnings.push("频繁出现大额净卖出，资金流出风险");
        }
        
        // 检查涨跌幅波动
        let avg_pct: f64 = records.iter()
            .map(|r| r.pct_change.abs())
            .sum::<f64>() / records.len() as f64;
        
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
        results.sort_by(|a, b| b.total_score.cmp(&a.total_score));
        
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

    #[tokio::test]
    async fn test_get_today_lhb() {
        let fetcher = LhbDataFetcher::new().unwrap();
        let result = fetcher.get_today_lhb().await;
        
        match result {
            Ok(records) => {
                println!("今日龙虎榜: {} 条记录", records.len());
                for (i, record) in records.iter().take(5).enumerate() {
                    println!("{}. {} {} 净买入: {:.0}万", 
                        i+1, record.code, record.name, record.net_amount);
                }
            }
            Err(e) => {
                println!("获取失败: {}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_analyze_stock() {
        let fetcher = LhbDataFetcher::new().unwrap();
        
        // 测试一个股票代码
        let code = "600519";
        let result = fetcher.analyze_stock_lhb(code).await;
        
        match result {
            Ok(analysis) => {
                println!("\n龙虎榜分析结果:");
                println!("股票: {} {}", analysis.code, analysis.name);
                println!("上榜次数: {}", analysis.recent_count);
                println!("机构评分: {}", analysis.inst_score);
                println!("游资评分: {}", analysis.hot_money_score);
                println!("综合评分: {}", analysis.total_score);
                println!("推荐理由: {}", analysis.reason);
                if !analysis.risk_warning.is_empty() {
                    println!("风险提示: {}", analysis.risk_warning);
                }
            }
            Err(e) => {
                println!("分析失败: {}", e);
            }
        }
    }
}
