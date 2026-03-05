use anyhow::Result;
use std::collections::HashMap;

/// 因子类型
#[derive(Debug, Clone)]
pub enum Factor {
    /// 市值（总市值）
    MarketCap,
    /// 净资产收益率
    ROE,
    /// 市盈率
    PE,
    /// 市净率
    PB,
    /// 换手率
    TurnoverRate,
}

/// 因子方向：1表示越小越好，-1表示越大越好
#[derive(Debug, Clone, Copy)]
pub enum FactorDirection {
    Ascending = 1,   // 越小越好（如市值、PE）
    Descending = -1, // 越大越好（如ROE）
}

/// 股票因子数据
#[derive(Debug, Clone)]
pub struct StockFactors {
    pub code: String,
    pub name: String,
    pub market_cap: Option<f64>,
    pub roe: Option<f64>,
    pub pe: Option<f64>,
    pub pb: Option<f64>,
    pub turnover_rate: Option<f64>,
}

/// 股票排名得分
#[derive(Debug, Clone)]
pub struct StockScore {
    pub code: String,
    pub name: String,
    pub total_score: f64,
    pub factor_ranks: HashMap<String, usize>, // 各因子的排名
}

/// 多因子策略配置
#[derive(Debug, Clone)]
pub struct MultiFactorConfig {
    /// 因子列表及其权重
    pub factors: Vec<(Factor, FactorDirection, f64)>, // (因子, 方向, 权重)
    /// 选股数量
    pub top_n: usize,
}

impl Default for MultiFactorConfig {
    fn default() -> Self {
        Self {
            factors: vec![
                (Factor::MarketCap, FactorDirection::Ascending, 1.0),  // 小盘股
                (Factor::ROE, FactorDirection::Descending, 1.0),       // 高ROE
                (Factor::PE, FactorDirection::Ascending, 0.5),         // 低PE
                (Factor::PB, FactorDirection::Ascending, 0.5),         // 低PB
            ],
            top_n: 20,
        }
    }
}

/// 多因子选股引擎
pub struct MultiFactorEngine {
    config: MultiFactorConfig,
}

impl MultiFactorEngine {
    pub fn new(config: MultiFactorConfig) -> Self {
        Self { config }
    }

    pub fn with_default() -> Self {
        Self::new(MultiFactorConfig::default())
    }

    /// 计算所有股票的因子得分
    pub fn calculate_scores(&self, stocks: &[StockFactors]) -> Result<Vec<StockScore>> {
        if stocks.is_empty() {
            return Ok(Vec::new());
        }

        // 1. 提取各因子的原始数据
        let mut factor_data: HashMap<String, Vec<(usize, f64)>> = HashMap::new();
        
        for (factor, _, _) in &self.config.factors {
            let factor_name = self.get_factor_name(factor);
            let mut data = Vec::new();
            
            for (idx, stock) in stocks.iter().enumerate() {
                if let Some(value) = self.get_factor_value(stock, factor) {
                    if value.is_finite() && value > 0.0 {
                        data.push((idx, value));
                    }
                }
            }
            
            factor_data.insert(factor_name, data);
        }

        // 2. 对每个因子进行排名
        let mut factor_ranks: HashMap<String, HashMap<usize, usize>> = HashMap::new();
        
        for (factor, direction, _) in &self.config.factors {
            let factor_name = self.get_factor_name(factor);
            
            if let Some(data) = factor_data.get_mut(&factor_name) {
                // 根据方向排序
                match direction {
                    FactorDirection::Ascending => {
                        // 越小越好：升序排列
                        data.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                    }
                    FactorDirection::Descending => {
                        // 越大越好：降序排列
                        data.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
                    }
                }
                
                // 记录排名（排名从1开始）
                let mut ranks = HashMap::new();
                for (rank, (idx, _)) in data.iter().enumerate() {
                    ranks.insert(*idx, rank + 1);
                }
                
                factor_ranks.insert(factor_name, ranks);
            }
        }

        // 3. 计算综合得分
        let mut scores = Vec::new();
        
        for (idx, stock) in stocks.iter().enumerate() {
            let mut total_score = 0.0;
            let mut stock_factor_ranks = HashMap::new();
            let mut valid_factors = 0;
            
            for (factor, _, weight) in &self.config.factors {
                let factor_name = self.get_factor_name(factor);
                
                if let Some(ranks) = factor_ranks.get(&factor_name) {
                    if let Some(&rank) = ranks.get(&idx) {
                        // 使用排名计算得分（排名越小得分越高）
                        total_score += (rank as f64) * weight;
                        stock_factor_ranks.insert(factor_name.clone(), rank);
                        valid_factors += 1;
                    }
                }
            }
            
            // 只有至少有一半因子有效的股票才参与排名
            if valid_factors >= self.config.factors.len() / 2 {
                scores.push(StockScore {
                    code: stock.code.clone(),
                    name: stock.name.clone(),
                    total_score,
                    factor_ranks: stock_factor_ranks,
                });
            }
        }

        // 4. 按总分排序（分数越低越好）
        scores.sort_by(|a, b| a.total_score.partial_cmp(&b.total_score).unwrap());

        Ok(scores)
    }

    /// 选出得分最高的N只股票
    pub fn select_top_stocks(&self, stocks: &[StockFactors]) -> Result<Vec<String>> {
        let scores = self.calculate_scores(stocks)?;
        
        Ok(scores
            .iter()
            .take(self.config.top_n)
            .map(|s| s.code.clone())
            .collect())
    }

    /// 获取因子名称
    fn get_factor_name(&self, factor: &Factor) -> String {
        match factor {
            Factor::MarketCap => "market_cap".to_string(),
            Factor::ROE => "roe".to_string(),
            Factor::PE => "pe".to_string(),
            Factor::PB => "pb".to_string(),
            Factor::TurnoverRate => "turnover_rate".to_string(),
        }
    }

    /// 获取股票的因子值
    fn get_factor_value(&self, stock: &StockFactors, factor: &Factor) -> Option<f64> {
        match factor {
            Factor::MarketCap => stock.market_cap,
            Factor::ROE => stock.roe,
            Factor::PE => stock.pe,
            Factor::PB => stock.pb,
            Factor::TurnoverRate => stock.turnover_rate,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_multi_factor_scoring() {
        let stocks = vec![
            StockFactors {
                code: "000001".to_string(),
                name: "平安银行".to_string(),
                market_cap: Some(100.0),
                roe: Some(0.15),
                pe: Some(10.0),
                pb: Some(1.5),
                turnover_rate: Some(2.0),
            },
            StockFactors {
                code: "000002".to_string(),
                name: "万科A".to_string(),
                market_cap: Some(200.0),
                roe: Some(0.20),
                pe: Some(8.0),
                pb: Some(1.2),
                turnover_rate: Some(3.0),
            },
            StockFactors {
                code: "600000".to_string(),
                name: "浦发银行".to_string(),
                market_cap: Some(50.0),
                roe: Some(0.10),
                pe: Some(12.0),
                pb: Some(1.8),
                turnover_rate: Some(1.5),
            },
        ];

        let engine = MultiFactorEngine::with_default();
        let scores = engine.calculate_scores(&stocks).unwrap();

        assert_eq!(scores.len(), 3);
        // 第一名应该是综合得分最低的
        assert!(scores[0].total_score <= scores[1].total_score);
    }

    #[test]
    fn test_select_top_stocks() {
        let stocks = vec![
            StockFactors {
                code: "000001".to_string(),
                name: "平安银行".to_string(),
                market_cap: Some(100.0),
                roe: Some(0.15),
                pe: Some(10.0),
                pb: Some(1.5),
                turnover_rate: Some(2.0),
            },
            StockFactors {
                code: "000002".to_string(),
                name: "万科A".to_string(),
                market_cap: Some(200.0),
                roe: Some(0.20),
                pe: Some(8.0),
                pb: Some(1.2),
                turnover_rate: Some(3.0),
            },
        ];

        let mut config = MultiFactorConfig::default();
        config.top_n = 1;
        
        let engine = MultiFactorEngine::new(config);
        let selected = engine.select_top_stocks(&stocks).unwrap();

        assert_eq!(selected.len(), 1);
    }
}
