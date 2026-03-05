// -*- coding: utf-8 -*-
//! ===================================
//! A股自选股智能分析系统 - 数据模型
//! ===================================
//!
//! 定义数据库表结构和ORM模型

use chrono::{NaiveDate, NaiveDateTime};
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::schema::{stock_daily, lhb_daily, analysis_result};

// ============================================================================
// 数据模型
// ============================================================================

/// 股票日线数据模型
///
/// 存储每日行情数据和计算的技术指标
/// 支持多股票、多日期的唯一约束
#[derive(Debug, Clone, Queryable, Selectable, Serialize, Deserialize)]
#[diesel(table_name = stock_daily)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct StockDaily {
    pub id: i32,
    pub code: String,
    pub date: NaiveDate,
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub close: Option<f64>,
    pub volume: Option<f64>,
    pub amount: Option<f64>,
    pub pct_chg: Option<f64>,
    pub ma5: Option<f64>,
    pub ma10: Option<f64>,
    pub ma20: Option<f64>,
    pub volume_ratio: Option<f64>,
    pub data_source: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// 插入新的股票日线数据
#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = stock_daily)]
pub struct NewStockDaily {
    pub code: String,
    pub date: NaiveDate,
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub close: Option<f64>,
    pub volume: Option<f64>,
    pub amount: Option<f64>,
    pub pct_chg: Option<f64>,
    pub ma5: Option<f64>,
    pub ma10: Option<f64>,
    pub ma20: Option<f64>,
    pub volume_ratio: Option<f64>,
    pub data_source: Option<String>,
}

impl NewStockDaily {
    pub fn new(code: String, date: NaiveDate) -> Self {
        Self {
            code,
            date,
            open: None,
            high: None,
            low: None,
            close: None,
            volume: None,
            amount: None,
            pct_chg: None,
            ma5: None,
            ma10: None,
            ma20: None,
            volume_ratio: None,
            data_source: None,
        }
    }
}

/// 更新股票日线数据
#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = stock_daily)]
pub struct UpdateStockDaily {
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub close: Option<f64>,
    pub volume: Option<f64>,
    pub amount: Option<f64>,
    pub pct_chg: Option<f64>,
    pub ma5: Option<f64>,
    pub ma10: Option<f64>,
    pub ma20: Option<f64>,
    pub volume_ratio: Option<f64>,
    pub data_source: Option<String>,
    pub updated_at: NaiveDateTime,
}

impl StockDaily {
    /// 转换为字典（HashMap）
    pub fn to_dict(&self) -> HashMap<String, serde_json::Value> {
        let mut map = HashMap::new();
        map.insert("code".to_string(), serde_json::json!(self.code));
        map.insert("date".to_string(), serde_json::json!(self.date.to_string()));
        
        if let Some(v) = self.open {
            map.insert("open".to_string(), serde_json::json!(v));
        }
        if let Some(v) = self.high {
            map.insert("high".to_string(), serde_json::json!(v));
        }
        if let Some(v) = self.low {
            map.insert("low".to_string(), serde_json::json!(v));
        }
        if let Some(v) = self.close {
            map.insert("close".to_string(), serde_json::json!(v));
        }
        if let Some(v) = self.volume {
            map.insert("volume".to_string(), serde_json::json!(v));
        }
        if let Some(v) = self.amount {
            map.insert("amount".to_string(), serde_json::json!(v));
        }
        if let Some(v) = self.pct_chg {
            map.insert("pct_chg".to_string(), serde_json::json!(v));
        }
        if let Some(v) = self.ma5 {
            map.insert("ma5".to_string(), serde_json::json!(v));
        }
        if let Some(v) = self.ma10 {
            map.insert("ma10".to_string(), serde_json::json!(v));
        }
        if let Some(v) = self.ma20 {
            map.insert("ma20".to_string(), serde_json::json!(v));
        }
        if let Some(v) = self.volume_ratio {
            map.insert("volume_ratio".to_string(), serde_json::json!(v));
        }
        if let Some(ref v) = self.data_source {
            map.insert("data_source".to_string(), serde_json::json!(v));
        }
        
        map
    }
}

/// 均线形态枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaStatus {
    /// 多头排列 📈
    BullAlignment,
    /// 空头排列 📉
    BearAlignment,
    /// 短期向好 🔼
    ShortTermUp,
    /// 短期走弱 🔽
    ShortTermDown,
    /// 震荡整理 ↔️
    Consolidation,
}

impl std::fmt::Display for MaStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let s = match self {
            Self::BullAlignment => "多头排列 📈",
            Self::BearAlignment => "空头排列 📉",
            Self::ShortTermUp => "短期向好 🔼",
            Self::ShortTermDown => "短期走弱 🔽",
            Self::Consolidation => "震荡整理 ↔️",
        };
        write!(f, "{}", s)
    }
}

impl StockDaily {
    /// 分析均线形态
    ///
    /// 判断条件：
    /// - 多头排列：close > ma5 > ma10 > ma20
    /// - 空头排列：close < ma5 < ma10 < ma20
    /// - 震荡整理：其他情况
    pub fn analyze_ma_status(&self) -> MaStatus {
        let close = self.close.unwrap_or(0.0);
        let ma5 = self.ma5.unwrap_or(0.0);
        let ma10 = self.ma10.unwrap_or(0.0);
        let ma20 = self.ma20.unwrap_or(0.0);

        if close > ma5 && ma5 > ma10 && ma10 > ma20 && ma20 > 0.0 {
            MaStatus::BullAlignment
        } else if close < ma5 && ma5 < ma10 && ma10 < ma20 && ma20 > 0.0 {
            MaStatus::BearAlignment
        } else if close > ma5 && ma5 > ma10 {
            MaStatus::ShortTermUp
        } else if close < ma5 && ma5 < ma10 {
            MaStatus::ShortTermDown
        } else {
            MaStatus::Consolidation
        }
    }
}

// ============================================================================
// 龙虎榜数据模型
// ============================================================================

/// 龙虎榜日线数据模型
#[derive(Debug, Clone, Queryable, Selectable, Serialize, Deserialize)]
#[diesel(table_name = lhb_daily)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct LhbDaily {
    pub id: i32,
    pub code: String,
    pub name: String,
    pub trade_date: String,
    pub reason: String,
    pub pct_change: f64,
    pub close_price: f64,
    pub buy_amount: f64,
    pub sell_amount: f64,
    pub net_amount: f64,
    pub total_amount: f64,
    pub lhb_ratio: f64,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

// ============================================================================
// 分析结果数据模型
// ============================================================================

/// 分析结果查询模型
#[derive(Debug, Clone, Queryable, Selectable, Serialize, Deserialize)]
#[diesel(table_name = analysis_result)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct AnalysisResultRecord {
    pub id: i32,
    pub code: String,
    pub name: String,
    pub date: NaiveDate,
    pub sentiment_score: i32,
    pub operation_advice: String,
    pub trend_prediction: String,
    pub pe_ratio: Option<f64>,
    pub pb_ratio: Option<f64>,
    pub turnover_rate: Option<f64>,
    pub market_cap: Option<f64>,
    pub circulating_cap: Option<f64>,
    pub close_price: Option<f64>,
    pub pct_chg: Option<f64>,
    pub data_source: Option<String>,
    pub created_at: NaiveDateTime,
}

/// 插入新的分析结果
#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = analysis_result)]
pub struct NewAnalysisResult {
    pub code: String,
    pub name: String,
    pub date: NaiveDate,
    pub sentiment_score: i32,
    pub operation_advice: String,
    pub trend_prediction: String,
    pub pe_ratio: Option<f64>,
    pub pb_ratio: Option<f64>,
    pub turnover_rate: Option<f64>,
    pub market_cap: Option<f64>,
    pub circulating_cap: Option<f64>,
    pub close_price: Option<f64>,
    pub pct_chg: Option<f64>,
    pub data_source: Option<String>,
}

/// 插入新的龙虎榜数据
#[derive(Debug, Clone, Insertable)]
#[diesel(table_name = lhb_daily)]
pub struct NewLhbDaily {
    pub code: String,
    pub name: String,
    pub trade_date: String,
    pub reason: String,
    pub pct_change: f64,
    pub close_price: f64,
    pub buy_amount: f64,
    pub sell_amount: f64,
    pub net_amount: f64,
    pub total_amount: f64,
    pub lhb_ratio: f64,
}
