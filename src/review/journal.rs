//! 交易日志 — 每笔买卖的复盘追踪。
//!
//! 从 portfolio 读取已平仓交易，计算持有时长、盈亏、卖出后走势。

use chrono::{NaiveDate, NaiveDateTime};

use crate::portfolio::{Trade, TradeDirection};

#[derive(Debug, Clone)]
pub struct TradeReview {
    pub code: String,
    pub name: String,
    pub buy_date: NaiveDate,
    pub sell_date: NaiveDate,
    pub buy_datetime: NaiveDateTime,
    pub sell_datetime: NaiveDateTime,
    pub buy_price: f64,
    pub sell_price: f64,
    pub holding_days: u32,
    pub pnl_pct: f64,
    pub post_exit_chg_5d: Option<f64>,
    pub post_exit_chg_20d: Option<f64>,
    pub self_rating: Option<u8>,
    pub lesson: Option<String>,
}

/// 从交易历史生成复盘记录。
/// 每笔 sell 对应一个 TradeReview，通过买入卖出配对计算。
pub fn review_closed_trades(trades: &[Trade]) -> Vec<TradeReview> {
    let mut reviews = Vec::new();
    let mut pending_buys: std::collections::VecDeque<&Trade> = std::collections::VecDeque::new();

    // 按时间排序
    let mut sorted: Vec<&Trade> = trades.iter().collect();
    sorted.sort_by_key(|t| t.traded_at);

    for trade in &sorted {
        match trade.direction {
            TradeDirection::Buy => {
                pending_buys.push_back(trade);
            }
            TradeDirection::Sell => {
                // 先进先出匹配
                if let Some(buy) = pending_buys.front().cloned() {
                    let holding_days = (trade.traded_at - buy.traded_at).num_days().max(0) as u32;
                    let pnl_pct = if buy.price > 0.0 {
                        (trade.price - buy.price) / buy.price * 100.0
                    } else { 0.0 };

                    reviews.push(TradeReview {
                        code: trade.code.clone(),
                        name: trade.name.clone(),
                        buy_date: buy.traded_at.date(),
                        sell_date: trade.traded_at.date(),
                        buy_datetime: buy.traded_at,
                        sell_datetime: trade.traded_at,
                        buy_price: buy.price,
                        sell_price: trade.price,
                        holding_days,
                        pnl_pct,
                        post_exit_chg_5d: None,  // 需要 K 线数据，后续补充
                        post_exit_chg_20d: None,
                        self_rating: None,
                        lesson: None,
                    });
                    pending_buys.pop_front();
                }
            }
        }
    }

    reviews
}

/// 尝试补充卖出后走势（需要 data_provider 拉 K 线）。
/// 失败静默降级 — post_exit_chg 保持 None。
pub fn enrich_post_exit(reviews: &mut [TradeReview]) {
    let fetcher = match crate::data_provider::DataFetcherManager::new() {
        Ok(f) => f,
        Err(_) => return,
    };

    for r in reviews.iter_mut() {
        // 只补充已卖出 5 天以上的（否则数据不足）
        let days_since_sell = (chrono::Local::now().date_naive() - r.sell_date).num_days();
        if days_since_sell < 5 { continue; }

        match fetcher.get_daily_data(&r.code, 60) {
            Ok((kline, _)) => {
                // 找到卖出日的价格作为基准
                let exit_idx = kline.iter().position(|k| k.date >= r.sell_date);
                if let Some(idx) = exit_idx {
                    let exit_close = kline[idx].close;
                    // 5日后
                    if idx + 5 < kline.len() {
                        let c = kline[idx + 5].close;
                        if exit_close > 0.0 {
                            r.post_exit_chg_5d = Some((c - exit_close) / exit_close * 100.0);
                        }
                    }
                    // 20日后
                    if idx + 20 < kline.len() {
                        let c = kline[idx + 20].close;
                        if exit_close > 0.0 {
                            r.post_exit_chg_20d = Some((c - exit_close) / exit_close * 100.0);
                        }
                    }
                }
            }
            Err(_) => {} // 静默降级
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(d: &str) -> NaiveDate { NaiveDate::parse_from_str(d, "%Y-%m-%d").unwrap() }
    fn dt(d: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(&format!("{} 10:00:00", d), "%Y-%m-%d %H:%M:%S").unwrap()
    }

    fn make_trade(code: &str, dir: TradeDirection, price: f64, date_str: &str) -> Trade {
        Trade {
            id: None,
            code: code.into(), name: format!("股票{}", code),
            direction: dir, price, shares: 100, amount: price * 100.0,
            reason: String::new(), traded_at: dt(date_str),
        }
    }

    #[test]
    fn test_review_basic() {
        let trades = vec![
            make_trade("000547", TradeDirection::Buy, 10.0, "2026-06-01"),
            make_trade("000547", TradeDirection::Sell, 12.0, "2026-06-10"),
        ];
        let reviews = review_closed_trades(&trades);
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].holding_days, 9);
        assert!((reviews[0].pnl_pct - 20.0).abs() < 0.01);
    }

    #[test]
    fn test_review_fifo() {
        let trades = vec![
            make_trade("000547", TradeDirection::Buy, 10.0, "2026-06-01"),
            make_trade("000547", TradeDirection::Buy, 11.0, "2026-06-05"),
            make_trade("000547", TradeDirection::Sell, 12.0, "2026-06-10"),
        ];
        let reviews = review_closed_trades(&trades);
        assert_eq!(reviews.len(), 1);
        // FIFO: matches first buy at 10.0
        assert!((reviews[0].pnl_pct - 20.0).abs() < 0.01);
    }

    #[test]
    fn test_no_review_for_holding() {
        let trades = vec![
            make_trade("000547", TradeDirection::Buy, 10.0, "2026-06-01"),
        ];
        let reviews = review_closed_trades(&trades);
        assert_eq!(reviews.len(), 0);
    }
}
