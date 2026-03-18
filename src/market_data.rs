// -*- coding: utf-8 -*-
//! 市场数据结构模块
//!
//! 定义大盘指数和市场概览相关的数据结构

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 大盘指数数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketIndex {
    /// 指数代码
    pub code: String,
    /// 指数名称
    pub name: String,
    /// 当前点位
    pub current: f64,
    /// 涨跌点数
    pub change: f64,
    /// 涨跌幅(%)
    pub change_pct: f64,
    /// 开盘点位
    pub open: f64,
    /// 最高点位
    pub high: f64,
    /// 最低点位
    pub low: f64,
    /// 昨收点位
    pub prev_close: f64,
    /// 成交量（手）
    pub volume: f64,
    /// 成交额（元）
    pub amount: f64,
    /// 振幅(%)
    pub amplitude: f64,
}

impl MarketIndex {
    /// 创建新的指数数据
    pub fn new(code: String, name: String) -> Self {
        Self {
            code,
            name,
            current: 0.0,
            change: 0.0,
            change_pct: 0.0,
            open: 0.0,
            high: 0.0,
            low: 0.0,
            prev_close: 0.0,
            volume: 0.0,
            amount: 0.0,
            amplitude: 0.0,
        }
    }

    /// 转换为字典格式
    pub fn to_dict(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        map.insert("code".to_string(), self.code.clone());
        map.insert("name".to_string(), self.name.clone());
        map.insert("current".to_string(), self.current.to_string());
        map.insert("change".to_string(), self.change.to_string());
        map.insert("change_pct".to_string(), self.change_pct.to_string());
        map.insert("open".to_string(), self.open.to_string());
        map.insert("high".to_string(), self.high.to_string());
        map.insert("low".to_string(), self.low.to_string());
        map.insert("volume".to_string(), self.volume.to_string());
        map.insert("amount".to_string(), self.amount.to_string());
        map.insert("amplitude".to_string(), self.amplitude.to_string());
        map
    }

    /// 计算振幅
    pub fn calculate_amplitude(&mut self) {
        if self.prev_close > 0.0 {
            self.amplitude = (self.high - self.low) / self.prev_close * 100.0;
        }
    }
}

/// 板块信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectorInfo {
    /// 板块名称
    pub name: String,
    /// 涨跌幅(%)
    pub change_pct: f64,
}

/// 市场概览数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketOverview {
    /// 日期
    pub date: String,
    /// 主要指数
    pub indices: Vec<MarketIndex>,
    /// 上涨家数
    pub up_count: i32,
    /// 下跌家数
    pub down_count: i32,
    /// 平盘家数
    pub flat_count: i32,
    /// 涨停家数
    pub limit_up_count: i32,
    /// 跌停家数
    pub limit_down_count: i32,
    /// 两市成交额（亿元）
    pub total_amount: f64,
    /// 北向资金净流入（亿元）
    pub north_flow: f64,
    /// 涨幅前5板块
    pub top_sectors: Vec<SectorInfo>,
    /// 跌幅前5板块
    pub bottom_sectors: Vec<SectorInfo>,
    /// 涨幅前10个股
    pub top_stocks: Vec<TopStock>,
    /// 当日涨停股票列表
    pub limit_up_stocks: Vec<TopStock>,
}

/// 表现突出的个股
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopStock {
    /// 股票代码
    pub code: String,
    /// 股票名称
    pub name: String,
    /// 涨跌幅(%)
    pub change_pct: f64,
    /// 现价
    pub price: f64,
}

impl MarketOverview {
    /// 创建新的市场概览
    pub fn new(date: String) -> Self {
        Self {
            date,
            indices: Vec::new(),
            up_count: 0,
            down_count: 0,
            flat_count: 0,
            limit_up_count: 0,
            limit_down_count: 0,
            total_amount: 0.0,
            north_flow: 0.0,
            top_sectors: Vec::new(),
            bottom_sectors: Vec::new(),
            top_stocks: Vec::new(),
            limit_up_stocks: Vec::new(),
        }
    }

    /// 获取上证指数
    pub fn get_sh_index(&self) -> Option<&MarketIndex> {
        self.indices.iter().find(|idx| idx.code.contains("000001"))
    }

    /// 判断市场走势
    pub fn market_mood(&self) -> &str {
        if let Some(sh_index) = self.get_sh_index() {
            if sh_index.change_pct > 1.0 {
                "强势上涨"
            } else if sh_index.change_pct > 0.0 {
                "小幅上涨"
            } else if sh_index.change_pct > -1.0 {
                "小幅下跌"
            } else {
                "明显下跌"
            }
        } else {
            "震荡整理"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_market_index_amplitude() {
        let mut index = MarketIndex::new("sh000001".to_string(), "上证指数".to_string());
        index.prev_close = 3000.0;
        index.high = 3050.0;
        index.low = 2980.0;
        index.calculate_amplitude();
        
        // 振幅 = (3050 - 2980) / 3000 * 100 = 2.33%
        assert!((index.amplitude - 2.33).abs() < 0.01);
    }

    #[test]
    fn test_market_mood() {
        let mut overview = MarketOverview::new("2026-01-22".to_string());
        
        let mut index = MarketIndex::new("sh000001".to_string(), "上证指数".to_string());
        index.change_pct = 1.5;
        overview.indices.push(index);
        
        assert_eq!(overview.market_mood(), "强势上涨");
    }
}
