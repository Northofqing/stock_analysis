//! 交易日志 — 每笔买卖的复盘追踪。
//!
//! 从 portfolio 读取已平仓交易，计算持有时长、盈亏、卖出后走势。

use chrono::{NaiveDate, NaiveDateTime};

use crate::portfolio::{Trade, TradeDirection};

#[derive(Debug, Clone)]
pub struct TradeReview {
    pub buy_trade_id: String,
    pub sell_trade_id: String,
    pub code: String,
    pub name: String,
    pub buy_date: NaiveDate,
    pub sell_date: NaiveDate,
    pub buy_datetime: NaiveDateTime,
    pub sell_datetime: NaiveDateTime,
    pub buy_price: f64,
    pub sell_price: f64,
    pub shares: u64,
    pub holding_days: u32,
    pub pnl_pct: f64,
    pub post_exit_chg_5d: Option<f64>,
    pub post_exit_chg_20d: Option<f64>,
    pub self_rating: Option<u8>,
    pub lesson: Option<String>,
}

/// 从交易历史生成复盘记录。
/// 每笔 sell 对应一个 TradeReview，通过买入卖出配对计算。
///
/// 以持久化交易 ID 去重并按 `(traded_at, id)` 稳定排序；按股票做数量感知 FIFO。
/// 坏交易、重复 ID、未匹配卖出或超卖会拒绝整批，不能生成部分复盘事实。
pub fn review_closed_trades(trades: &[Trade]) -> Result<Vec<TradeReview>, String> {
    use std::collections::{HashMap, HashSet, VecDeque};

    struct OpenLot<'a> {
        trade: &'a Trade,
        remaining: u64,
    }

    let mut seen_ids = HashSet::new();
    let mut sorted = Vec::with_capacity(trades.len());
    for trade in trades {
        let id = trade
            .id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| format!("交易 {} 缺少持久化 ID", trade.code))?;
        if !seen_ids.insert(id) {
            return Err(format!("交易 ID 重复: {id}"));
        }
        if trade.code.trim().is_empty() || trade.name.trim().is_empty() {
            return Err(format!("交易 {id} code/name 缺失"));
        }
        if !trade.price.is_finite() || trade.price <= 0.0 || trade.shares == 0 {
            return Err(format!(
                "交易 {id} price/shares 非法: price={} shares={}",
                trade.price, trade.shares
            ));
        }
        if !trade.amount.is_finite() {
            return Err(format!("交易 {id} amount 非法: {}", trade.amount));
        }
        sorted.push(trade);
    }
    sorted.sort_by(|left, right| {
        left.traded_at
            .cmp(&right.traded_at)
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut reviews = Vec::new();
    let mut pending_buys: HashMap<&str, VecDeque<OpenLot<'_>>> = HashMap::new();
    for trade in sorted {
        match trade.direction {
            TradeDirection::Buy => pending_buys
                .entry(trade.code.as_str())
                .or_default()
                .push_back(OpenLot {
                    trade,
                    remaining: trade.shares,
                }),
            TradeDirection::Sell => {
                let mut remaining_sell = trade.shares;
                while remaining_sell > 0 {
                    let queue = pending_buys.get_mut(trade.code.as_str()).ok_or_else(|| {
                        format!(
                            "卖出交易 {}({}) 无匹配买入",
                            trade.id.as_deref().unwrap_or("missing"),
                            trade.code
                        )
                    })?;
                    let lot = queue.front_mut().ok_or_else(|| {
                        format!(
                            "卖出交易 {}({}) 超卖 {} 股",
                            trade.id.as_deref().unwrap_or("missing"),
                            trade.code,
                            remaining_sell
                        )
                    })?;
                    let matched = remaining_sell.min(lot.remaining);
                    let buy = lot.trade;
                    let holding_days =
                        u32::try_from((trade.traded_at - buy.traded_at).num_days().max(0))
                            .map_err(|_| format!("交易 {} 持有天数溢出", trade.code))?;
                    let pnl_pct = (trade.price - buy.price) / buy.price * 100.0;
                    if !pnl_pct.is_finite() {
                        return Err(format!("交易 {} 收益率非有限值", trade.code));
                    }
                    reviews.push(TradeReview {
                        buy_trade_id: buy.id.clone().expect("validated trade ID"),
                        sell_trade_id: trade.id.clone().expect("validated trade ID"),
                        code: trade.code.clone(),
                        name: trade.name.clone(),
                        buy_date: buy.traded_at.date(),
                        sell_date: trade.traded_at.date(),
                        buy_datetime: buy.traded_at,
                        sell_datetime: trade.traded_at,
                        buy_price: buy.price,
                        sell_price: trade.price,
                        shares: matched,
                        holding_days,
                        pnl_pct,
                        post_exit_chg_5d: None,
                        post_exit_chg_20d: None,
                        self_rating: None,
                        lesson: None,
                    });
                    remaining_sell -= matched;
                    lot.remaining -= matched;
                    if lot.remaining == 0 {
                        queue.pop_front();
                    }
                }
            }
        }
    }

    Ok(reviews)
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
        if days_since_sell < 5 {
            continue;
        }

        if let Ok((kline, _)) = fetcher.get_daily_data(&r.code, 60) {
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
        } // 复盘补充失败不阻断其他记录
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(d: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(&format!("{} 10:00:00", d), "%Y-%m-%d %H:%M:%S").unwrap()
    }

    fn make_trade(code: &str, dir: TradeDirection, price: f64, date_str: &str) -> Trade {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Trade {
            id: Some(format!("trade-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed))),
            code: code.into(),
            name: format!("股票{}", code),
            direction: dir,
            price,
            shares: 100,
            amount: price * 100.0,
            reason: String::new(),
            traded_at: dt(date_str),
        }
    }

    #[test]
    fn test_review_basic() {
        let trades = vec![
            make_trade("TEST_CODE_000547", TradeDirection::Buy, 10.0, "2026-06-01"),
            make_trade("TEST_CODE_000547", TradeDirection::Sell, 12.0, "2026-06-10"),
        ];
        let reviews = review_closed_trades(&trades).unwrap();
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].holding_days, 9);
        assert!((reviews[0].pnl_pct - 20.0).abs() < 0.01);
    }

    #[test]
    fn test_review_fifo() {
        let trades = vec![
            make_trade("TEST_CODE_000547", TradeDirection::Buy, 10.0, "2026-06-01"),
            make_trade("TEST_CODE_000547", TradeDirection::Buy, 11.0, "2026-06-05"),
            make_trade("TEST_CODE_000547", TradeDirection::Sell, 12.0, "2026-06-10"),
        ];
        let reviews = review_closed_trades(&trades).unwrap();
        assert_eq!(reviews.len(), 1);
        // FIFO: matches first buy at 10.0
        assert!((reviews[0].pnl_pct - 20.0).abs() < 0.01);
    }

    #[test]
    fn test_no_review_for_holding() {
        let trades = vec![make_trade(
            "TEST_CODE_000547",
            TradeDirection::Buy,
            10.0,
            "2026-06-01",
        )];
        let reviews = review_closed_trades(&trades).unwrap();
        assert_eq!(reviews.len(), 0);
    }

    #[test]
    fn br103_fifo_never_pairs_across_codes_and_preserves_partial_quantities() {
        let mut buy_a = make_trade("TEST_CODE_000001", TradeDirection::Buy, 10.0, "2026-06-01");
        buy_a.shares = 200;
        let buy_b = make_trade("TEST_CODE_600000", TradeDirection::Buy, 20.0, "2026-06-02");
        let sell_b = make_trade("TEST_CODE_600000", TradeDirection::Sell, 22.0, "2026-06-03");
        let sell_a_1 = make_trade("TEST_CODE_000001", TradeDirection::Sell, 11.0, "2026-06-04");
        let sell_a_2 = make_trade("TEST_CODE_000001", TradeDirection::Sell, 12.0, "2026-06-05");
        let reviews = review_closed_trades(&[buy_a, buy_b, sell_b, sell_a_1, sell_a_2]).unwrap();
        assert_eq!(reviews.len(), 3);
        assert_eq!(reviews[0].code, "TEST_CODE_600000");
        assert_eq!(reviews[1].code, "TEST_CODE_000001");
        assert_eq!(reviews[1].shares, 100);
        assert_eq!(reviews[2].shares, 100);
    }

    #[test]
    fn br103_duplicate_ids_and_oversells_reject_complete_batch() {
        let buy = make_trade("TEST_CODE_000001", TradeDirection::Buy, 10.0, "2026-06-01");
        let mut duplicate =
            make_trade("TEST_CODE_000001", TradeDirection::Sell, 11.0, "2026-06-02");
        duplicate.id = buy.id.clone();
        assert!(review_closed_trades(&[buy.clone(), duplicate]).is_err());

        let mut oversell = make_trade("TEST_CODE_000001", TradeDirection::Sell, 11.0, "2026-06-02");
        oversell.shares = 200;
        assert!(review_closed_trades(&[buy, oversell]).is_err());
    }
}
