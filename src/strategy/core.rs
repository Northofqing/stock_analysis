use anyhow::Result;
use chrono::{DateTime, Local};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use serde::{Serialize, Deserialize};
use plotters::prelude::*;

/// 持仓记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub code: String,
    pub name: String,
    pub shares: f64,           // 持有数量
    pub avg_price: f64,        // 平均成本
    pub current_price: f64,    // 当前价格
    pub market_value: f64,     // 市值
}

impl Position {
    /// 计算持仓盈亏
    pub fn profit(&self) -> f64 {
        (self.current_price - self.avg_price) * self.shares
    }

    /// 计算持仓收益率
    pub fn return_rate(&self) -> f64 {
        if self.avg_price > 0.0 {
            (self.current_price - self.avg_price) / self.avg_price
        } else {
            0.0
        }
    }
}

/// 交易记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub date: DateTime<Local>,
    pub code: String,
    pub name: String,
    pub action: TradeAction,
    pub shares: f64,
    pub price: f64,
    pub amount: f64,
    pub commission: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TradeAction {
    Buy,
    Sell,
}

/// 回测配置
#[derive(Debug, Clone)]
pub struct BacktestConfig {
    /// 初始资金
    pub initial_capital: f64,
    /// 调仓频率（天）
    pub rebalance_days: usize,
    /// 持仓数量
    pub position_count: usize,
    /// 手续费率
    pub commission_rate: f64,
    /// 滑点率
    pub slippage_rate: f64,
    /// 印花税率（A 股仅卖方征收，现行 0.001）
    pub stamp_tax_rate: f64,
}

impl Default for BacktestConfig {
    fn default() -> Self {
        Self {
            initial_capital: 100_000.0,    // 10万初始资金
            rebalance_days: 15,             // 15天调仓一次
            position_count: 20,             // 持仓20只(扩展from 3)
            commission_rate: 0.0003,        // 万三手续费
            slippage_rate: 0.001,           // 千一滑点
            stamp_tax_rate: 0.001,          // 千一印花税（仅卖出）
        }
    }
}

/// 回测状态
#[derive(Debug, Clone)]
pub struct BacktestState {
    pub cash: f64,                                    // 现金
    pub positions: HashMap<String, Position>,         // 持仓
    pub trades: Vec<Trade>,                           // 交易记录
    pub daily_values: Vec<(DateTime<Local>, f64)>,   // 每日市值
    pub last_rebalance: Option<DateTime<Local>>,     // 上次调仓时间
}

impl BacktestState {
    pub fn new(initial_capital: f64) -> Self {
        Self {
            cash: initial_capital,
            positions: HashMap::new(),
            trades: Vec::new(),
            daily_values: vec![(Local::now(), initial_capital)],
            last_rebalance: None,
        }
    }

    /// 计算总资产
    pub fn total_value(&self) -> f64 {
        let position_value: f64 = self.positions.values().map(|p| p.market_value).sum();
        self.cash + position_value
    }

    /// 计算总收益率
    pub fn total_return(&self) -> f64 {
        if self.daily_values.len() < 2 {
            return 0.0;
        }
        
        let (_, initial_value) = self.daily_values.first().unwrap();
        let (_, final_value) = self.daily_values.last().unwrap();
        
        if *initial_value > 0.0 {
            (final_value - initial_value) / initial_value
        } else {
            0.0
        }
    }

    /// 计算最大回撤
    pub fn max_drawdown(&self) -> f64 {
        if self.daily_values.len() < 2 {
            return 0.0;
        }

        let mut max_value = 0.0;
        let mut max_dd = 0.0;

        for (_, value) in &self.daily_values {
            if *value > max_value {
                max_value = *value;
            }
            let dd = (max_value - value) / max_value;
            if dd > max_dd {
                max_dd = dd;
            }
        }

        max_dd
    }

    /// 计算年化收益率
    pub fn annual_return(&self) -> f64 {
        if self.daily_values.len() < 2 {
            return 0.0;
        }

        let (first_date, first_value) = &self.daily_values[0];
        let (last_date, last_value) = self.daily_values.last().unwrap();
        
        let days = (*last_date - *first_date).num_days() as f64;
        if days <= 0.0 || *first_value <= 0.0 {
            return 0.0;
        }

        let years = days / 365.0;
        (last_value / first_value).powf(1.0 / years) - 1.0
    }

    /// 计算夏普比率（修正版，扣除无风险利率）
    /// risk_free_rate: 年化无风险利率，默认2.5%
    pub fn sharpe_ratio(&self, risk_free_rate: f64) -> f64 {
        if self.daily_values.len() < 2 {
            return 0.0;
        }

        // 计算每日收益率
        let mut daily_returns = Vec::new();
        for i in 1..self.daily_values.len() {
            let prev_value = self.daily_values[i - 1].1;
            let curr_value = self.daily_values[i].1;
            if prev_value > 0.0 {
                daily_returns.push((curr_value - prev_value) / prev_value);
            }
        }

        if daily_returns.is_empty() {
            return 0.0;
        }

        // 计算平均收益率和标准差
        let mean: f64 = daily_returns.iter().sum::<f64>() / daily_returns.len() as f64;
        let variance: f64 = daily_returns
            .iter()
            .map(|r| (r - mean).powi(2))
            .sum::<f64>()
            / daily_returns.len() as f64;
        let std_dev = variance.sqrt();

        // 日化无风险利率
        let daily_rf = risk_free_rate / 252.0;

        if std_dev > 0.0 {
            // 年化夏普比率 = (平均日收益 - 日化无风险率) / 日标准差 * sqrt(252)
            (mean - daily_rf) / std_dev * (252.0_f64).sqrt()
        } else {
            0.0
        }
    }

    /// 计算Sortino比率（只惩罚向下波动）
    pub fn sortino_ratio(&self, risk_free_rate: f64) -> f64 {
        if self.daily_values.len() < 2 {
            return 0.0;
        }

        // 计算每日收益率
        let mut daily_returns = Vec::new();
        for i in 1..self.daily_values.len() {
            let prev_value = self.daily_values[i - 1].1;
            let curr_value = self.daily_values[i].1;
            if prev_value > 0.0 {
                daily_returns.push((curr_value - prev_value) / prev_value);
            }
        }

        if daily_returns.is_empty() {
            return 0.0;
        }

        let mean: f64 = daily_returns.iter().sum::<f64>() / daily_returns.len() as f64;
        let daily_rf = risk_free_rate / 252.0;

        // 只计算向下的偏差
        let downside_variance: f64 = daily_returns
            .iter()
            .map(|r| {
                let excess = r - daily_rf;
                if excess < 0.0 { excess.powi(2) } else { 0.0 }
            })
            .sum::<f64>()
            / daily_returns.len() as f64;
        let downside_std = downside_variance.sqrt();

        if downside_std > 0.0 {
            (mean - daily_rf) / downside_std * (252.0_f64).sqrt()
        } else {
            0.0
        }
    }

    /// 计算Calmar比率 = 年化收益 / 最大回撤
    ///
    /// 修正：移除「回撤<1%直接置0」的断层，只要存在有效回撤（>1e-6）即正常计算，
    /// 避免低波动样本下 Calmar 被人为抹平为 0 造成误读。
    pub fn calmar_ratio(&self) -> f64 {
        let annual = self.annual_return();
        let mdd = self.max_drawdown();

        if mdd > 1e-6 {
            annual / mdd
        } else {
            0.0
        }
    }

    /// 计算最长回撤恢复期（净值处于水下、未创新高的最长连续自然日数）。
    ///
    /// 返回值为日历日（按 daily_values 的首尾日期跨度估算），用于衡量
    /// 策略在最差情形下"被套"多久才回本，是机构评估资金占用质量的重要指标。
    pub fn max_drawdown_duration_days(&self) -> i64 {
        if self.daily_values.len() < 2 {
            return 0;
        }

        let mut peak_value = self.daily_values[0].1;
        let mut peak_date = self.daily_values[0].0;
        let mut max_days: i64 = 0;

        for (date, value) in &self.daily_values {
            if *value >= peak_value {
                // 创新高：水下区间结束，重置峰值
                peak_value = *value;
                peak_date = *date;
            } else {
                let days = (*date - peak_date).num_days();
                if days > max_days {
                    max_days = days;
                }
            }
        }

        max_days
    }

    /// 计算平均仓位(暴露率) - 从daily_values推算
    pub fn average_exposure(&self, _initial_capital: f64) -> (f64, Vec<f64>) {
        let mut daily_exposure = Vec::new();
        let mut total_exposure = 0.0;

        // 从每日净值推算仓位(净值越高说明仓位越重)
        if self.daily_values.is_empty() {
            return (0.0, daily_exposure);
        }

        let initial_value = self.daily_values[0].1;

        for (_, value) in &self.daily_values {
            // 简化估算: 用当前净值与初值的比率乘以假设的平均持有周期
            // 实际应该从持仓明细推算，这里暂用保守估计
            let exposure = (*value / initial_value).min(1.0);
            daily_exposure.push(exposure);
            total_exposure += exposure;
        }

        let avg = if !daily_exposure.is_empty() {
            total_exposure / daily_exposure.len() as f64
        } else {
            0.0
        };

        (avg, daily_exposure)
    }

    /// 基于真实基准日线序列，计算同期基准收益、Beta、CAPM Alpha 与信息比率。
    ///
    /// 仅使用策略净值日期与基准日期的交集对齐，缺失数据自动跳过；
    /// 当有效对齐样本不足时返回 None（上层据此显示"基准数据缺失"，绝不伪造）。
    pub(crate) fn benchmark_metrics(
        &self,
        bench: &BenchmarkSeries,
        strat_annual: f64,
        risk_free_rate: f64,
    ) -> Option<BenchmarkMetrics> {
        if self.daily_values.len() < 3 {
            return None;
        }

        let mut strat_rets: Vec<f64> = Vec::new();
        let mut bench_rets: Vec<f64> = Vec::new();
        let mut first_bench: Option<(f64, DateTime<Local>)> = None;
        let mut last_bench: Option<(f64, DateTime<Local>)> = None;

        for i in 1..self.daily_values.len() {
            let (d_prev, v_prev) = &self.daily_values[i - 1];
            let (d_cur, v_cur) = &self.daily_values[i];
            if let (Some(&bp), Some(&bc)) = (bench.closes.get(&d_prev.date_naive()), bench.closes.get(&d_cur.date_naive())) {
                if *v_prev > 0.0 && bp > 0.0 {
                    strat_rets.push((*v_cur - *v_prev) / *v_prev);
                    bench_rets.push((bc - bp) / bp);
                    if first_bench.is_none() {
                        first_bench = Some((bp, *d_prev));
                    }
                    last_bench = Some((bc, *d_cur));
                }
            }
        }

        if strat_rets.len() < 2 {
            return None;
        }

        let (first_close, first_date) = first_bench?;
        let (last_close, last_date) = last_bench?;

        let bench_total = if first_close > 0.0 {
            (last_close - first_close) / first_close
        } else {
            return None;
        };

        let days = (last_date - first_date).num_days() as f64;
        let bench_annual = if days > 0.0 {
            (1.0 + bench_total).powf(365.0 / days) - 1.0
        } else {
            0.0
        };

        let n = strat_rets.len() as f64;
        let mean_s = strat_rets.iter().sum::<f64>() / n;
        let mean_b = bench_rets.iter().sum::<f64>() / n;

        let mut cov = 0.0;
        let mut var_b = 0.0;
        for i in 0..strat_rets.len() {
            cov += (strat_rets[i] - mean_s) * (bench_rets[i] - mean_b);
            var_b += (bench_rets[i] - mean_b).powi(2);
        }
        cov /= n;
        var_b /= n;
        let beta = if var_b > 1e-12 { cov / var_b } else { 0.0 };

        // CAPM Alpha（年化）：策略年化 − [无风险 + beta×(基准年化 − 无风险)]
        let alpha = strat_annual - (risk_free_rate + beta * (bench_annual - risk_free_rate));

        // 信息比率：年化超额 / 跟踪误差
        let mean_ex = mean_s - mean_b;
        let var_ex = strat_rets
            .iter()
            .zip(&bench_rets)
            .map(|(s, b)| ((s - b) - mean_ex).powi(2))
            .sum::<f64>()
            / n;
        let std_ex = var_ex.sqrt();
        let information_ratio = if std_ex > 1e-12 {
            mean_ex / std_ex * (252.0_f64).sqrt()
        } else {
            0.0
        };

        Some(BenchmarkMetrics {
            name: bench.name.clone(),
            total_return: bench_total,
            annual_return: bench_annual,
            alpha,
            beta,
            information_ratio,
        })
    }
}

/// 回测引擎
pub struct BacktestEngine {
    config: BacktestConfig,
    state: BacktestState,
}

impl BacktestEngine {
    pub fn new(config: BacktestConfig) -> Self {
        let state = BacktestState::new(config.initial_capital);
        Self { config, state }
    }

    /// 买入股票
    pub fn buy(&mut self, code: &str, name: &str, price: f64, shares: f64, date: DateTime<Local>) -> Result<()> {
        let actual_price = price * (1.0 + self.config.slippage_rate); // 买入滑点
        let amount = actual_price * shares;
        let commission = amount * self.config.commission_rate;
        let total_cost = amount + commission;

        if total_cost > self.state.cash {
            return Err(anyhow::anyhow!("资金不足"));
        }

        self.state.cash -= total_cost;

        // 更新持仓
        let position = self.state.positions.entry(code.to_string()).or_insert(Position {
            code: code.to_string(),
            name: name.to_string(),
            shares: 0.0,
            avg_price: 0.0,
            current_price: price,
            market_value: 0.0,
        });

        // 更新平均成本
        let old_value = position.avg_price * position.shares;
        position.shares += shares;
        position.avg_price = (old_value + amount) / position.shares;
        position.current_price = price;
        position.market_value = position.shares * position.current_price;

        // 记录交易
        self.state.trades.push(Trade {
            date,
            code: code.to_string(),
            name: name.to_string(),
            action: TradeAction::Buy,
            shares,
            price: actual_price,
            amount,
            commission,
        });

        Ok(())
    }

    /// 买入股票（带涨跌停合规检查）
    ///
    /// 与 `buy` 的区别：要求传入 `name` 和 `prev_close`，会先用
    /// `data_provider::limit_status::validate_limit_price` 检查价格是否
    /// 在涨跌停范围内。如果超出，则返回 `Err(LimitPriceError)`，不下单。
    ///
    /// 修复：QUANT_ANALYST_REVIEW §1.1
    pub fn try_buy_validated(
        &mut self,
        code: &str,
        name: &str,
        prev_close: f64,
        price: f64,
        shares: f64,
        date: DateTime<Local>,
    ) -> Result<()> {
        crate::data_provider::limit_status::validate_limit_price(code, name, prev_close, price)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        self.buy(code, name, price, shares, date)
    }

    /// 卖出股票（带涨跌停合规检查）
    pub fn try_sell_validated(
        &mut self,
        code: &str,
        name: &str,
        prev_close: f64,
        shares: f64,
        price: f64,
        date: DateTime<Local>,
    ) -> Result<()> {
        crate::data_provider::limit_status::validate_limit_price(code, name, prev_close, price)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        self.sell(code, shares, price, date)
    }

    /// 买入股票（A 股真实约束：涨跌停 + 整百股 + 最小佣金 5 元）
    ///
    /// 修复：QUANT_ANALYST_REVIEW §1.2, §2.4
    /// 行为：
    ///   1. 涨跌停价格检查
    ///   2. shares 向下取整到 100 的倍数，< 100 报错
    ///   3. 佣金 = max(amount * 0.0003, 5.0)
    pub fn try_buy_realistic(
        &mut self,
        code: &str,
        name: &str,
        prev_close: f64,
        price: f64,
        shares: f64,
        date: DateTime<Local>,
    ) -> Result<u64> {
        use crate::strategy::lot::{min_commission, round_lot};
        // 1. 涨跌停
        crate::data_provider::limit_status::validate_limit_price(code, name, prev_close, price)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        // 2. 整百股
        let rounded = round_lot(shares);
        if rounded == 0 {
            return Err(anyhow::anyhow!(
                "买入股数 {shares} 取整后为 0，不足 1 手 (100 股)"
            ));
        }
        // 3. 实际买入（用整百股后的数量）
        self.buy(code, name, price, rounded as f64, date)?;
        // 4. 强制覆盖佣金为含 5 元保底（A 股标准）
        //    引擎内部的 buy 已经按 commission_rate 算过，但没保底
        //    我们在最后一次 trade 上把 commission 调整
        if let Some(last_trade) = self.state.trades.last_mut() {
            let amount = last_trade.amount;
            let min_c = min_commission(amount);
            let diff = min_c - last_trade.commission;
            if diff > 0.0 {
                last_trade.commission = min_c;
                self.state.cash -= diff; // 补扣差额
            }
        }
        Ok(rounded)
    }

    /// 卖出股票（A 股真实约束：涨跌停 + T+1 + 整百股 + 最小佣金 + 印花税）
    pub fn try_sell_realistic(
        &mut self,
        code: &str,
        name: &str,
        prev_close: f64,
        shares: f64,
        price: f64,
        date: DateTime<Local>,
    ) -> Result<u64> {
        use crate::strategy::lot::{min_commission, round_lot, stamp_tax};
        // 1. 涨跌停
        crate::data_provider::limit_status::validate_limit_price(code, name, prev_close, price)
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        // 2. T+1：取该 code 的最后买入日期
        let last_buy_date = self
            .state
            .trades
            .iter()
            .rev()
            .find(|t| t.code == code && matches!(t.action, TradeAction::Buy))
            .map(|t| t.date.date_naive());
        if let Some(buy_date) = last_buy_date {
            if date.date_naive() <= buy_date {
                return Err(anyhow::anyhow!(
                    "T+1 违规: {code} 最后买入 {buy_date}, 不能在 {sell_date} 卖出",
                    sell_date = date.date_naive()
                ));
            }
        }
        // 3. 整百股
        let rounded = round_lot(shares);
        if rounded == 0 {
            return Err(anyhow::anyhow!(
                "卖出股数 {shares} 取整后为 0，不足 1 手 (100 股)"
            ));
        }
        // 4. 卖出
        self.sell(code, rounded as f64, price, date)?;
        // 5. 调整佣金 + 印花税
        if let Some(last_trade) = self.state.trades.last_mut() {
            let amount = last_trade.amount;
            let min_c = min_commission(amount);
            let commission_diff = min_c - last_trade.commission;
            if commission_diff > 0.0 {
                last_trade.commission = min_c;
                self.state.cash -= commission_diff;
            }
            // 印花税：sell() 已按 stamp_tax_rate 算过，这里校验
            // 实际已是引擎内 calc, 不重复加
            let _ = stamp_tax(amount); // 抑制 unused 警告
        }
        Ok(rounded)
    }

    /// 卖出股票
    pub fn sell(&mut self, code: &str, shares: f64, price: f64, date: DateTime<Local>) -> Result<()> {
        let position = self.state.positions.get_mut(code)
            .ok_or_else(|| anyhow::anyhow!("没有持仓"))?;

        if shares > position.shares {
            return Err(anyhow::anyhow!("持仓不足"));
        }

        let actual_price = price * (1.0 - self.config.slippage_rate); // 卖出滑点
        let amount = actual_price * shares;
        let commission = amount * self.config.commission_rate;
        let stamp_tax = amount * self.config.stamp_tax_rate;
        let proceeds = amount - commission - stamp_tax;

        self.state.cash += proceeds;
        position.shares -= shares;
        
        // 记录交易
        self.state.trades.push(Trade {
            date,
            code: code.to_string(),
            name: position.name.clone(),
            action: TradeAction::Sell,
            shares,
            price: actual_price,
            amount,
            commission,
        });

        // 如果清仓，移除持仓
        if position.shares < 0.01 {
            self.state.positions.remove(code);
        } else {
            position.market_value = position.shares * position.current_price;
        }

        Ok(())
    }

    /// 更新持仓价格
    pub fn update_prices(&mut self, prices: &HashMap<String, f64>) {
        for (code, position) in self.state.positions.iter_mut() {
            if let Some(&price) = prices.get(code) {
                position.current_price = price;
                position.market_value = position.shares * price;
            }
        }
    }

    /// 记录每日市值
    pub fn record_daily_value(&mut self, date: DateTime<Local>) {
        let total_value = self.state.total_value();
        self.state.daily_values.push((date, total_value));
    }

    /// 调仓：卖出不在目标列表的股票，买入目标股票
    pub fn rebalance(
        &mut self,
        target_stocks: &[(String, String, f64)], // (code, name, price)
        date: DateTime<Local>,
    ) -> Result<()> {
        // 1. 卖出不在目标列表的股票
        let target_codes: HashSet<&str> = target_stocks.iter().map(|(c, _, _)| c.as_str()).collect();
        let to_sell: Vec<String> = self.state.positions.keys()
            .filter(|code| !target_codes.contains(code.as_str()))
            .cloned()
            .collect();

        for code in to_sell {
            if let Some(position) = self.state.positions.get(&code) {
                let shares = position.shares;
                let price = position.current_price;
                self.sell(&code, shares, price, date)?;
            }
        }

        // 2. 计算每只股票应该买入的金额（等权重）
        let total_value = self.state.total_value();
        let per_stock_value = total_value / self.config.position_count as f64;

        // 3. 买入目标股票
        for (code, name, price) in target_stocks {
            // 如果已有持仓，计算需要补足的金额
            let current_value = self.state.positions.get(code)
                .map(|p| p.market_value)
                .unwrap_or(0.0);
            
            let target_value = per_stock_value;
            let diff_value = target_value - current_value;

            if diff_value > 100.0 {  // 差额大于100元才交易
                let shares = (diff_value / price).floor();
                if shares > 0.0 {
                    let _ = self.buy(code, name, *price, shares, date);
                }
            }
        }

        self.state.last_rebalance = Some(date);
        Ok(())
    }

    /// 获取回测状态
    pub fn get_state(&self) -> &BacktestState {
        &self.state
    }

    /// 获取现金
    pub fn get_cash(&self) -> f64 {
        self.state.cash
    }

    /// 获取持仓
    pub fn get_positions(&self) -> &HashMap<String, Position> {
        &self.state.positions
    }
}

/// 回测结果摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestSummary {
    pub initial_capital: f64,
    pub final_value: f64,
    pub total_return: f64,
    pub annual_return: f64,
    pub max_drawdown: f64,
    pub sharpe_ratio: f64,
    pub sortino_ratio: f64,            // NEW: Sortino比率
    pub calmar_ratio: f64,             // NEW: Calmar比率
    pub average_exposure: f64,         // NEW: 平均仓位
    pub total_trades: usize,
    pub win_rate: f64,
    pub chart_path: Option<String>,  // 图表路径
    pub benchmark_annual_return: Option<f64>,  // NEW: 基准年化收益(真实指数, 缺失则 None)
    pub alpha: Option<f64>,            // NEW: CAPM Alpha(年化, 相对真实基准)
    pub benchmark_name: Option<String>,   // NEW: 基准名称(如 沪深300)
    pub benchmark_total_return: Option<f64>, // NEW: 同期基准区间收益
    pub excess_return: Option<f64>,    // NEW: 年化超额(策略年化-基准年化)
    pub beta: Option<f64>,             // NEW: 相对基准的 beta
    pub information_ratio: Option<f64>, // NEW: 信息比率(年化超额/跟踪误差)
    pub max_dd_duration_days: i64,     // NEW: 最长回撤恢复期(自然日)
}

/// 真实基准日线序列（用于计算超额收益 / Alpha / Beta / 信息比率）。
///
/// `closes` 以 "YYYY-MM-DD" 为键映射到当日收盘价；缺失日期会被跳过，
/// 因此当指数数据抓取失败时上层应传 `None`，而非伪造固定收益。
#[derive(Debug, Clone)]
pub struct BenchmarkSeries {
    pub name: String,
    pub closes: HashMap<chrono::NaiveDate, f64>,
}

impl BenchmarkSeries {
    pub fn new(name: impl Into<String>, closes: HashMap<chrono::NaiveDate, f64>) -> Self {
        Self { name: name.into(), closes }
    }
}

/// 修复 P2.9: 常用基准指数代码常量
/// 量化分析师建议: 不同策略用不同基准
/// - 大盘股策略: 沪深300 (sh000300)
/// - 中盘股策略: 中证500 (sh000905)
/// - 小盘股策略: 中证1000 (sh000852) / 国证2000 (sz399303)
/// - 创业板策略: 创业板指 (sz399006)
/// - 科创板策略: 科创50 (sh000688)
pub mod benchmark_codes {
    pub const HS300: &str = "sh000300";      // 沪深300
    pub const ZZ500: &str = "sh000905";      // 中证500
    pub const ZZ1000: &str = "sh000852";     // 中证1000
    pub const GZ2000: &str = "sz399303";     // 国证2000
    pub const CHINEXT: &str = "sz399006";    // 创业板指
    pub const STAR50: &str = "sh000688";     // 科创50
    pub const SH_COMP: &str = "sh000001";    // 上证指数
    pub const SZ_COMP: &str = "sz399001";    // 深证成指

    /// 根据策略类型推荐基准
    pub fn recommend_for_strategy(strategy_kind: &str) -> &'static str {
        match strategy_kind {
            "large_cap" => HS300,
            "mid_cap" => ZZ500,
            "small_cap" => ZZ1000,
            "chinext" => CHINEXT,
            "star" => STAR50,
            "broad_market" => HS300,
            _ => HS300, // 默认
        }
    }
}

/// 市场状态分类（基于基准指数的趋势）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegimeKind {
    Bull,
    Sideways,
    Bear,
}

impl RegimeKind {
    pub fn label(&self) -> &'static str {
        match self {
            RegimeKind::Bull => "牛市/上行",
            RegimeKind::Sideways => "震荡/横盘",
            RegimeKind::Bear => "熊市/下行",
        }
    }
}

/// 单个市场状态下的策略表现统计。
#[derive(Debug, Clone)]
pub struct RegimeStats {
    pub kind: RegimeKind,
    pub days: usize,
    pub strat_return: f64, // 该状态下策略累计收益（复利）
    pub bench_return: f64, // 该状态下基准累计收益（复利）
    pub up_day_rate: f64,  // 策略上涨日占比
}

/// 分市场状态回测拆解报告。
#[derive(Debug, Clone)]
pub struct RegimeReport {
    pub window: usize,        // 趋势判定窗口（交易日）
    pub bull_threshold: f64,  // 牛市阈值（窗口涨幅）
    pub bear_threshold: f64,  // 熊市阈值（窗口跌幅）
    pub stats: Vec<RegimeStats>,
}

/// 按基准指数趋势把回测期切分为 牛/震荡/熊 三种状态，分别统计策略表现。
///
/// 趋势判定：基准在 `window` 个交易日内的累计涨幅 > `bull_threshold` 记为牛市，
/// < `bear_threshold` 记为熊市，其余为震荡。基准数据不足或无法对齐时返回 None。
pub fn regime_breakdown(
    daily_values: &[(DateTime<Local>, f64)],
    bench: &BenchmarkSeries,
) -> Option<RegimeReport> {
    if daily_values.len() < 5 {
        return None;
    }

    // 基准按日期升序排列，便于按窗口取趋势
    let mut bench_sorted: Vec<(chrono::NaiveDate, f64)> =
        bench.closes.iter().map(|(k, v)| (*k, *v)).collect();
    bench_sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let date_to_idx: HashMap<chrono::NaiveDate, usize> = bench_sorted
        .iter()
        .enumerate()
        .map(|(i, (d, _))| (*d, i))
        .collect();

    let window = 20usize;
    let bull_threshold = 0.03;
    let bear_threshold = -0.03;

    // 每种状态累加：(天数, 策略累计因子, 基准累计因子, 上涨日数)
    let mut acc: [(usize, f64, f64, usize); 3] = [(0, 1.0, 1.0, 0); 3];

    for i in 1..daily_values.len() {
        let (d_prev, v_prev) = &daily_values[i - 1];
        let (d_cur, v_cur) = &daily_values[i];
        if *v_prev <= 0.0 {
            continue;
        }
        let (Some(&idx_cur), Some(&idx_prev)) = (
            date_to_idx.get(&d_cur.date_naive()),
            date_to_idx.get(&d_prev.date_naive()),
        ) else {
            continue;
        };
        if idx_cur < window {
            continue;
        }

        let base = bench_sorted[idx_cur - window].1;
        if base <= 0.0 {
            continue;
        }
        let trail = (bench_sorted[idx_cur].1 - base) / base;
        let slot = if trail > bull_threshold {
            0 // Bull
        } else if trail < bear_threshold {
            2 // Bear
        } else {
            1 // Sideways
        };

        let s_ret = (*v_cur - *v_prev) / *v_prev;
        let prev_close = bench_sorted[idx_prev].1;
        let b_ret = if prev_close > 0.0 {
            (bench_sorted[idx_cur].1 - prev_close) / prev_close
        } else {
            0.0
        };

        acc[slot].0 += 1;
        acc[slot].1 *= 1.0 + s_ret;
        acc[slot].2 *= 1.0 + b_ret;
        if s_ret > 0.0 {
            acc[slot].3 += 1;
        }
    }

    let kinds = [RegimeKind::Bull, RegimeKind::Sideways, RegimeKind::Bear];
    let stats: Vec<RegimeStats> = acc
        .iter()
        .zip(kinds.iter())
        .filter(|((days, ..), _)| *days > 0)
        .map(|((days, sf, bf, ups), kind)| RegimeStats {
            kind: *kind,
            days: *days,
            strat_return: sf - 1.0,
            bench_return: bf - 1.0,
            up_day_rate: *ups as f64 / *days as f64,
        })
        .collect();

    if stats.is_empty() {
        return None;
    }

    Some(RegimeReport {
        window,
        bull_threshold,
        bear_threshold,
        stats,
    })
}

/// 基准对标计算结果（内部使用）。
#[derive(Debug, Clone)]
pub struct BenchmarkMetrics {
    pub name: String,
    pub total_return: f64,
    pub annual_return: f64,
    pub alpha: f64,
    pub beta: f64,
    pub information_ratio: f64,
}

/// 单个 walk-forward 折（fold）的样本外结果。
#[derive(Debug, Clone)]
pub struct WalkForwardFold {
    pub fold: usize,
    pub train_label: String,   // 训练区间（样本内）
    pub test_label: String,    // 测试区间（样本外）
    pub chosen_param: String,  // 训练段选出的最优参数标签
    pub test_return: f64,      // 该参数在测试段的总收益
    pub test_sharpe: f64,
    pub test_trades: usize,
}

/// Walk-forward 滚动优化汇总。
#[derive(Debug, Clone)]
pub struct WalkForwardReport {
    pub folds: Vec<WalkForwardFold>,
    pub avg_test_return: f64,    // 各折样本外总收益的算术平均
    pub compounded_return: f64,  // 各折样本外收益串联复利
    pub positive_fold_rate: f64, // 样本外为正的折数占比
}

impl BacktestSummary {
    /// 不带基准的构造（基准字段全部为 None，**不再伪造 7%**）。
    pub fn from_state(state: &BacktestState, initial_capital: f64) -> Self {
        Self::from_state_with_benchmark(state, initial_capital, None)
    }

    /// 带真实基准的构造。`benchmark` 为 None 时基准相关字段保持 None。
    pub fn from_state_with_benchmark(
        state: &BacktestState,
        initial_capital: f64,
        benchmark: Option<&BenchmarkSeries>,
    ) -> Self {
        let final_value = state.total_value();
        let total_return = state.total_return();
        let annual_return = state.annual_return();
        let max_drawdown = state.max_drawdown();
        let max_dd_duration_days = state.max_drawdown_duration_days();

        // 修复 P1.2: 统一无风险利率常量
        // 之前 core.rs 用 2.5% (1Y 国债), sharpe_calculator.rs 默认 3%
        // 量化分析师要求: 同一系统内 rf 必须一致, 跨报告不可比问题
        // 改用 sharpe_calculator 模块的统一常量
        let risk_free_rate = super::super::sharpe_calculator::DEFAULT_RISK_FREE_RATE;
        let sharpe_ratio = state.sharpe_ratio(risk_free_rate);
        let sortino_ratio = state.sortino_ratio(risk_free_rate);

        let calmar_ratio = state.calmar_ratio();
        let (average_exposure, _) = state.average_exposure(initial_capital);

        let total_trades = state.trades.len();

        // 计算胜率
        let mut wins = 0;
        let mut total_closed = 0;
        let mut positions_closed: HashMap<String, (f64, f64)> = HashMap::new(); // (avg_cost, total_shares)

        for trade in &state.trades {
            match trade.action {
                TradeAction::Buy => {
                    let entry = positions_closed.entry(trade.code.clone()).or_insert((0.0, 0.0));
                    let old_value = entry.0 * entry.1;
                    entry.1 += trade.shares;
                    entry.0 = (old_value + trade.amount) / entry.1;
                }
                TradeAction::Sell => {
                    if let Some((avg_cost, total_shares)) = positions_closed.get_mut(&trade.code) {
                        if trade.price > *avg_cost {
                            wins += 1;
                        }
                        total_closed += 1;

                        *total_shares -= trade.shares;
                        if *total_shares < 0.01 {
                            positions_closed.remove(&trade.code);
                        }
                    }
                }
            }
        }

        let win_rate = if total_closed > 0 {
            wins as f64 / total_closed as f64
        } else {
            0.0
        };

        // ── 真实基准对标（缺失时全部 None，不伪造） ──
        let bench = benchmark.and_then(|b| state.benchmark_metrics(b, annual_return, risk_free_rate));
        let (
            benchmark_name,
            benchmark_annual_return,
            benchmark_total_return,
            excess_return,
            alpha,
            beta,
            information_ratio,
        ) = match bench {
            Some(m) => (
                Some(m.name),
                Some(m.annual_return),
                Some(m.total_return),
                Some(annual_return - m.annual_return),
                Some(m.alpha),
                Some(m.beta),
                Some(m.information_ratio),
            ),
            None => (None, None, None, None, None, None, None),
        };

        Self {
            initial_capital,
            final_value,
            total_return,
            annual_return,
            max_drawdown,
            sharpe_ratio,
            sortino_ratio,
            calmar_ratio,
            average_exposure,
            total_trades,
            win_rate,
            chart_path: None,
            benchmark_annual_return,
            alpha,
            benchmark_name,
            benchmark_total_return,
            excess_return,
            beta,
            information_ratio,
            max_dd_duration_days,
        }
    }

    /// 设置图表路径
    pub fn set_chart_path(&mut self, path: String) {
        self.chart_path = Some(path);
    }

    /// 生成回测净值曲线图表（三 panel：净值+买卖点 / Drawdown / 指标）
    pub fn generate_chart(&self, state: &BacktestState, output_path: &str) -> Result<PathBuf> {
        let path_buf = PathBuf::from(output_path);
        
        if state.daily_values.is_empty() {
            return Err(anyhow::anyhow!("No daily values to plot"));
        }

        // 计算净值曲线（归一化为1开始）
        let initial_value = state.daily_values[0].1;
        let net_values: Vec<_> = state.daily_values.iter()
            .map(|(date, value)| (*date, *value / initial_value))
            .collect();

        // 计算 drawdown 序列：dd[i] = (cur - peak_so_far) / peak_so_far，单位为百分数
        let mut peak = f64::NEG_INFINITY;
        let drawdowns: Vec<(DateTime<Local>, f64)> = net_values.iter()
            .map(|(d, v)| {
                if *v > peak { peak = *v; }
                let dd = if peak > 0.0 { (*v - peak) / peak * 100.0 } else { 0.0 };
                (*d, dd)
            })
            .collect();

        // 找出净值最大最小值
        let min_value = net_values.iter().map(|(_, v)| *v).fold(f64::INFINITY, f64::min);
        let max_value = net_values.iter().map(|(_, v)| *v).fold(f64::NEG_INFINITY, f64::max);

        {
            let root = BitMapBackend::new(output_path, (1400, 1100)).into_drawing_area();
            root.fill(&WHITE)?;

            // 上 50%（净值+买卖点） / 中 25%（drawdown） / 下 25%（指标）
            let (top, rest) = root.split_vertically(550);
            let (middle, bottom) = rest.split_vertically(275);

            Self::draw_net_value_curve_with_trades(&top, &net_values, min_value, max_value, &state.trades, initial_value)?;
            Self::draw_drawdown_curve(&middle, &drawdowns, self.max_drawdown)?;
            Self::draw_backtest_metrics(&bottom, self)?;

            root.present()?;
        }
        
        Ok(path_buf)
    }

    /// 绘制净值曲线（含买卖点散点）
    fn draw_net_value_curve_with_trades(
        area: &DrawingArea<BitMapBackend, plotters::coord::Shift>,
        net_values: &[(DateTime<Local>, f64)],
        min_value: f64,
        max_value: f64,
        trades: &[Trade],
        initial_value: f64,
    ) -> Result<()> {
        if net_values.is_empty() {
            return Ok(());
        }

        let first_date = net_values[0].0;
        let last_date = net_values.last().unwrap().0;

        let y_min = (min_value * 0.95).max(0.0);
        let y_max = max_value * 1.05;

        let mut chart = ChartBuilder::on(area)
            .caption("净值曲线 & 买卖点", ("sans-serif", 32).into_font().color(&BLACK))
            .margin(15)
            .x_label_area_size(40)
            .y_label_area_size(70)
            .build_cartesian_2d(first_date..last_date, y_min..y_max)?;

        chart
            .configure_mesh()
            .x_desc("日期")
            .y_desc("净值（归一化）")
            .x_labels(10)
            .y_labels(8)
            .x_label_formatter(&|date| date.format("%y-%m-%d").to_string())
            .y_label_formatter(&|y| format!("{:.2}", y))
            .draw()?;

        // 净值线
        chart.draw_series(LineSeries::new(
            net_values.iter().map(|(date, value)| (*date, *value)),
            BLUE.mix(0.85).stroke_width(2),
        ))?
        .label("净值")
        .legend(|(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], &BLUE));

        // 基准线
        chart.draw_series(LineSeries::new(
            vec![(first_date, 1.0), (last_date, 1.0)],
            RED.mix(0.5).stroke_width(1),
        ))?
        .label("基准 (1.0)")
        .legend(|(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], &RED));

        // 买卖点：将 trade.date 上当日的净值作为标记 y
        // 用 daily_values 做 O(N+M) 的双指针查找
        let mut buy_pts: Vec<(DateTime<Local>, f64)> = Vec::new();
        let mut sell_pts: Vec<(DateTime<Local>, f64)> = Vec::new();
        let mut idx = 0usize;
        for t in trades {
            // 找到 net_values 中 >= t.date 的第一个点（trades 假定已排序，但 portfolio 聚合可能未严格排序）
            let target = t.date;
            // 当未排序时退化为线性扫描
            if idx >= net_values.len() || net_values[idx].0 > target {
                idx = 0;
            }
            while idx + 1 < net_values.len() && net_values[idx].0 < target {
                idx += 1;
            }
            let nv = net_values[idx].1;
            // 净值 y 已归一化；trade 用此点画散点
            match t.action {
                TradeAction::Buy => buy_pts.push((target, nv)),
                TradeAction::Sell => sell_pts.push((target, nv)),
            }
            let _ = initial_value; // 抑制未使用警告（保留参数以备扩展）
        }

        if !buy_pts.is_empty() {
            chart.draw_series(buy_pts.iter().map(|(d, v)| {
                TriangleMarker::new((*d, *v), 6, GREEN.filled())
            }))?
            .label(format!("买入 ({})", buy_pts.len()))
            .legend(|(x, y)| TriangleMarker::new((x + 10, y), 6, GREEN.filled()));
        }
        if !sell_pts.is_empty() {
            chart.draw_series(sell_pts.iter().map(|(d, v)| {
                Circle::new((*d, *v), 5, RED.filled())
            }))?
            .label(format!("卖出 ({})", sell_pts.len()))
            .legend(|(x, y)| Circle::new((x + 10, y), 5, RED.filled()));
        }

        chart.configure_series_labels()
            .position(SeriesLabelPosition::UpperLeft)
            .background_style(WHITE.mix(0.85))
            .border_style(BLACK)
            .draw()?;

        Ok(())
    }

    /// 绘制 Drawdown 区域填充
    fn draw_drawdown_curve(
        area: &DrawingArea<BitMapBackend, plotters::coord::Shift>,
        drawdowns: &[(DateTime<Local>, f64)],
        max_drawdown: f64,
    ) -> Result<()> {
        if drawdowns.is_empty() {
            return Ok(());
        }

        let first_date = drawdowns[0].0;
        let last_date = drawdowns.last().unwrap().0;

        // y 轴范围：dd 永远 <= 0；下界取 max_drawdown 与序列最小值的更深者，再留 5% 余量
        let series_min = drawdowns.iter().map(|(_, d)| *d).fold(0.0_f64, f64::min);
        let y_min = (series_min.min(-max_drawdown * 100.0)) * 1.10;
        let y_max = 1.0_f64;

        let mut chart = ChartBuilder::on(area)
            .caption(
                format!("Drawdown（最大回撤 {:.2}%）", max_drawdown * 100.0),
                ("sans-serif", 28).into_font().color(&BLACK),
            )
            .margin(15)
            .x_label_area_size(40)
            .y_label_area_size(70)
            .build_cartesian_2d(first_date..last_date, y_min..y_max)?;

        chart
            .configure_mesh()
            .x_desc("日期")
            .y_desc("Drawdown (%)")
            .x_labels(10)
            .y_labels(6)
            .x_label_formatter(&|d| d.format("%y-%m-%d").to_string())
            .y_label_formatter(&|y| format!("{:.1}%", y))
            .draw()?;

        // 区域填充：从 0 线到 dd 曲线
        chart.draw_series(AreaSeries::new(
            drawdowns.iter().map(|(d, v)| (*d, *v)),
            0.0,
            RED.mix(0.25),
        ).border_style(RED.stroke_width(1)))?
        .label("Drawdown")
        .legend(|(x, y)| PathElement::new(vec![(x, y), (x + 20, y)], &RED));

        // 0 基准线
        chart.draw_series(LineSeries::new(
            vec![(first_date, 0.0), (last_date, 0.0)],
            BLACK.mix(0.5).stroke_width(1),
        ))?;

        Ok(())
    }

    /// 绘制回测指标
    fn draw_backtest_metrics(
        area: &DrawingArea<BitMapBackend, plotters::coord::Shift>,
        summary: &BacktestSummary,
    ) -> Result<()> {
        area.fill(&WHITE)?;

        // 标题
        area.draw(&Text::new(
            "回测指标",
            (650, 40),
            ("sans-serif", 35).into_font().color(&BLACK),
        ))?;

        // 指标数据
        let metrics = vec![
            ("初始资金", format!("¥{:.2}", summary.initial_capital)),
            ("最终市值", format!("¥{:.2}", summary.final_value)),
            ("总收益率", format!("{:+.2}%", summary.total_return * 100.0)),
            ("年化收益率", format!("{:+.2}%", summary.annual_return * 100.0)),
            ("最大回撤", format!("{:.2}%", summary.max_drawdown * 100.0)),
            ("夏普比率", format!("{:.2}", summary.sharpe_ratio)),
            ("总交易次数", format!("{}", summary.total_trades)),
            ("胜率", format!("{:.2}%", summary.win_rate * 100.0)),
        ];

        let start_y = 120;
        let row_height = 50;
        let col1_x: i32 = 200;
        let col2_x: i32 = 450;
        let col3_x: i32 = 800;
        let col4_x: i32 = 1050;

        for (idx, (label, value)) in metrics.iter().enumerate() {
            let row = idx / 2;
            let col = idx % 2;
            let y: i32 = start_y + (row as i32) * row_height;
            let x: i32 = if col == 0 { col1_x } else { col3_x };
            let value_x: i32 = if col == 0 { col2_x } else { col4_x };

            // 标签
            let label_text = Text::new(
                label.to_string(),
                (x, y),
                ("sans-serif", 25).into_font().color(&BLACK),
            );
            area.draw(&label_text)?;

            // 值（根据正负显示不同颜色）
            let color = if value.contains('+') {
                &GREEN
            } else if value.contains('-') {
                &RED
            } else {
                &BLACK
            };

            let value_text = Text::new(
                value.clone(),
                (value_x, y),
                ("sans-serif", 25).into_font().color(color),
            );
            area.draw(&value_text)?;
        }

        Ok(())
    }

    /// 导出交易记录为CSV
    pub fn export_trades_csv(&self, state: &BacktestState, output_path: &str) -> Result<()> {
        use std::fs::File;
        use std::io::Write;

        let mut file = File::create(output_path)?;
        // CSV 头
        writeln!(file, "date,code,name,action,shares,price,amount,commission")?;

        for trade in &state.trades {
            let action = match trade.action {
                TradeAction::Buy => "BUY",
                TradeAction::Sell => "SELL",
            };
            writeln!(
                file,
                "{},{},{},{},{:.0},{:.4},{:.2},{:.2}",
                trade.date.format("%Y-%m-%d"),
                trade.code,
                trade.name,
                action,
                trade.shares,
                trade.price,
                trade.amount,
                trade.commission
            )?;
        }
        Ok(())
    }

    /// 导出每日净值为CSV
    pub fn export_daily_values_csv(&self, state: &BacktestState, output_path: &str, initial_capital: f64) -> Result<()> {
        use std::fs::File;
        use std::io::Write;

        let mut file = File::create(output_path)?;
        writeln!(file, "date,total_value,daily_return,cumulative_return")?;

        let mut prev_value = initial_capital;
        for (date, value) in &state.daily_values {
            let daily_ret = (*value - prev_value) / prev_value;
            let cum_ret = (*value - initial_capital) / initial_capital;
            writeln!(
                file,
                "{},{:.2},{:.4},{:.4}",
                date.format("%Y-%m-%d"),
                value,
                daily_ret,
                cum_ret
            )?;
            prev_value = *value;
        }
        Ok(())
    }
}

/// 导出交易明细 CSV（审计用，独立于回测引擎实例）
pub fn write_trades_csv(state: &BacktestState, output_path: &str) -> Result<()> {
    use std::fs::File;
    use std::io::Write;

    let mut file = File::create(output_path)?;
    writeln!(file, "date,code,name,action,shares,price,amount,commission")?;
    for trade in &state.trades {
        let action = match trade.action {
            TradeAction::Buy => "BUY",
            TradeAction::Sell => "SELL",
        };
        writeln!(
            file,
            "{},{},{},{},{:.0},{:.4},{:.2},{:.2}",
            trade.date.format("%Y-%m-%d"),
            trade.code,
            trade.name,
            action,
            trade.shares,
            trade.price,
            trade.amount,
            trade.commission
        )?;
    }
    Ok(())
}

/// 导出每日净值 CSV（审计用，独立于回测引擎实例）
pub fn write_daily_values_csv(state: &BacktestState, output_path: &str, initial_capital: f64) -> Result<()> {
    use std::fs::File;
    use std::io::Write;

    let mut file = File::create(output_path)?;
    writeln!(file, "date,total_value,daily_return,cumulative_return")?;
    let mut prev_value = initial_capital;
    for (date, value) in &state.daily_values {
        let daily_ret = if prev_value.abs() > 1e-9 { (*value - prev_value) / prev_value } else { 0.0 };
        let cum_ret = (*value - initial_capital) / initial_capital;
        writeln!(
            file,
            "{},{:.2},{:.4},{:.4}",
            date.format("%Y-%m-%d"),
            value,
            daily_ret,
            cum_ret
        )?;
        prev_value = *value;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backtest_engine() {
        let config = BacktestConfig::default();
        let mut engine = BacktestEngine::new(config);
        let date = Local::now();

        // 测试买入
        engine.buy("000001", "平安银行", 10.0, 100.0, date).unwrap();
        assert!(engine.get_cash() < 1_000_000.0);
        assert_eq!(engine.get_positions().len(), 1);

        // 测试卖出
        engine.sell("000001", 50.0, 11.0, date).unwrap();
        assert_eq!(engine.get_positions().len(), 1);

        // 测试清仓
        engine.sell("000001", 50.0, 11.0, date).unwrap();
        assert_eq!(engine.get_positions().len(), 0);
    }

    #[test]
    fn test_try_buy_validated_rejects_above_limit() {
        // 修复：QUANT_ANALYST_REVIEW §1.1
        // 验证 try_buy_validated 在价格 > 涨停价时拒绝成交
        let config = BacktestConfig::default();
        let mut engine = BacktestEngine::new(config);
        let date = Local::now();
        // prev_close=10.0，主板 10%，涨停 11.0；试图以 11.5 买入应被拒
        let result = engine.try_buy_validated("600000", "浦发银行", 10.0, 11.5, 100.0, date);
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("涨停价") || msg.contains("高于"), "msg={msg}");
        assert_eq!(engine.get_positions().len(), 0, "被拒后不应建仓");
    }

    #[test]
    fn test_try_buy_validated_accepts_within_range() {
        let config = BacktestConfig::default();
        let mut engine = BacktestEngine::new(config);
        let date = Local::now();
        // 10.5 在 9.0~11.0 范围内，合规
        let result = engine.try_buy_validated("600000", "浦发银行", 10.0, 10.5, 100.0, date);
        assert!(result.is_ok(), "应当接受 10.5 买入, got: {:?}", result);
        assert_eq!(engine.get_positions().len(), 1);
    }

    #[test]
    fn test_try_sell_validated_rejects_below_limit() {
        let config = BacktestConfig::default();
        let mut engine = BacktestEngine::new(config);
        let date = Local::now();
        // 先买入 100 股 @10.0
        engine.buy("600000", "浦发银行", 10.0, 100.0, date).unwrap();
        // 跌停 9.0，尝试 8.5 卖出应被拒
        let result = engine.try_sell_validated("600000", "浦发银行", 10.0, 100.0, 8.5, date);
        assert!(result.is_err());
    }

    #[test]
    fn test_try_buy_realistic_rounds_to_lot() {
        // 修复：QUANT_ANALYST_REVIEW §1.2
        // 150 股 -> 100 股
        let config = BacktestConfig::default();
        let mut engine = BacktestEngine::new(config);
        let date = Local::now();
        let filled = engine
            .try_buy_realistic("600000", "浦发银行", 10.0, 10.5, 150.0, date)
            .unwrap();
        assert_eq!(filled, 100);
    }

    #[test]
    fn test_try_buy_realistic_rejects_under_lot() {
        // 50 股 -> 取整 0, 报错
        let config = BacktestConfig::default();
        let mut engine = BacktestEngine::new(config);
        let date = Local::now();
        let result = engine.try_buy_realistic("600000", "浦发银行", 10.0, 10.5, 50.0, date);
        assert!(result.is_err());
    }

    #[test]
    fn test_try_buy_realistic_min_commission_5() {
        // 100 股 @ 10.0 = 1000 元, 佣金理论 0.3 元, 应保底 5 元
        let config = BacktestConfig::default();
        let mut engine = BacktestEngine::new(config);
        let date = Local::now();
        engine
            .try_buy_realistic("600000", "浦发银行", 10.0, 10.0, 100.0, date)
            .unwrap();
        let last = engine.state.trades.last().unwrap();
        assert!((last.commission - 5.0).abs() < 1e-6, "commission={}", last.commission);
    }

    #[test]
    fn test_try_sell_realistic_t1_violation() {
        // T 日买入, T 日卖出应被拒
        let config = BacktestConfig::default();
        let mut engine = BacktestEngine::new(config);
        let buy_date = Local::now();
        engine
            .try_buy_realistic("600000", "浦发银行", 10.0, 10.0, 100.0, buy_date)
            .unwrap();
        let result = engine.try_sell_realistic(
            "600000",
            "浦发银行",
            10.0,
            100.0,
            10.5,
            buy_date, // 同一天
        );
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("T+1"), "msg={msg}");
    }

    #[test]
    fn test_try_sell_realistic_t1_allows_next_day() {
        // T 日买入, T+1 日卖出应通过
        let config = BacktestConfig::default();
        let mut engine = BacktestEngine::new(config);
        let buy_date = Local::now();
        engine
            .try_buy_realistic("600000", "浦发银行", 10.0, 10.0, 100.0, buy_date)
            .unwrap();
        let sell_date = buy_date + chrono::Duration::days(1);
        let result =
            engine.try_sell_realistic("600000", "浦发银行", 10.0, 100.0, 10.5, sell_date);
        assert!(result.is_ok(), "got: {:?}", result);
    }

    #[test]
    fn test_backtest_metrics() {
        let mut state = BacktestState::new(1_000_000.0);
        
        // 清空初始值，使用一个明确的时间序列
        state.daily_values.clear();
        
        let start = Local::now();
        state.daily_values.push((start, 1_000_000.0)); // Day 0: 初始资金
        state.daily_values.push((start + chrono::Days::new(1), 1_100_000.0)); // Day 1: +10%
        state.daily_values.push((start + chrono::Days::new(2), 1_050_000.0)); // Day 2: 回撤
        state.daily_values.push((start + chrono::Days::new(3), 1_200_000.0)); // Day 3: 新高

        // 最后一个值 1_200_000 > 初始值 1_000_000，所以收益率应该 > 0
        let ret = state.total_return();
        assert!(ret > 0.0, "total_return should be positive, got: {}", ret);
        
        // 应该有回撤（从1_100_000跌到1_050_000）
        let dd = state.max_drawdown();
        assert!(dd > 0.0, "max_drawdown should be positive, got: {}", dd);
    }

    #[test]
    fn test_regime_breakdown_buckets() {
        // 构造 60 天基准：前 30 天上行，后 30 天下行
        let base = Local::now();
        let mut closes = HashMap::new();
        let mut dates: Vec<DateTime<Local>> = Vec::new();
        for i in 0..60i64 {
            let dt = base + chrono::Duration::days(i);
            let close = if i < 30 {
                100.0 + i as f64 * 2.0 // 上行
            } else {
                160.0 - (i - 30) as f64 * 2.0 // 下行
            };
            closes.insert(dt.date_naive(), close);
            dates.push(dt);
        }
        let bench = BenchmarkSeries::new("测试指数", closes);

        // 策略每日净值取基准 d20..d59（确保 idx_cur >= window=20）
        let daily_values: Vec<(DateTime<Local>, f64)> = dates[20..]
            .iter()
            .enumerate()
            .map(|(k, d)| (*d, 1_000_000.0 + k as f64 * 1000.0))
            .collect();

        let rep = regime_breakdown(&daily_values, &bench).expect("应返回 regime 报告");
        // 同时存在牛市与熊市两种状态
        let has_bull = rep.stats.iter().any(|s| s.kind == RegimeKind::Bull && s.days > 0);
        let has_bear = rep.stats.iter().any(|s| s.kind == RegimeKind::Bear && s.days > 0);
        assert!(has_bull, "应识别出牛市区间");
        assert!(has_bear, "应识别出熊市区间");
    }
}
