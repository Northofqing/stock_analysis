use anyhow::Result;
use chrono::{DateTime, Local};
use std::collections::HashMap;
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
    pub fn calmar_ratio(&self) -> f64 {
        let annual = self.annual_return();
        let mdd = self.max_drawdown();

        if mdd > 0.01 {  // 回撤大于1%才计算
            annual / mdd
        } else {
            0.0
        }
    }

    /// 计算平均仓位(暴露率) - 从daily_values推算
    pub fn average_exposure(&self, initial_capital: f64) -> (f64, Vec<f64>) {
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
        let target_codes: Vec<String> = target_stocks.iter().map(|(c, _, _)| c.clone()).collect();
        let to_sell: Vec<String> = self.state.positions.keys()
            .filter(|code| !target_codes.contains(code))
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
    pub benchmark_annual_return: Option<f64>,  // NEW: 基准年化收益(如沪深300)
    pub alpha: Option<f64>,            // NEW: Alpha值
}

impl BacktestSummary {
    pub fn from_state(state: &BacktestState, initial_capital: f64) -> Self {
        let final_value = state.total_value();
        let total_return = state.total_return();
        let annual_return = state.annual_return();
        let max_drawdown = state.max_drawdown();

        // 使用2.5% 1Y国债作为无风险利率
        let risk_free_rate = 0.025;
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
            benchmark_annual_return: None,
            alpha: None,
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
}
