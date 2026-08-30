//! 交易日志 — 每笔买卖的复盘追踪。
//!
//! 从 portfolio 读取已平仓交易，计算持有时长、盈亏、卖出后走势。

use chrono::{NaiveDate, NaiveDateTime};

use crate::data_gateway::{AdmittedDailyBars, BatchEvidence, HistoricalBarsGateway};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostExitEnrichmentState {
    AwaitingFiveDay { available_sessions: usize },
    FiveDayOnly { available_sessions: usize },
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostExitEnrichmentAwaiting {
    pub code: String,
    pub sell_date: NaiveDate,
    pub available_sessions: usize,
    pub required_sessions: usize,
    pub batch_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostExitEnrichmentFailure {
    pub code: String,
    pub sell_date: NaiveDate,
    pub reason: String,
    pub batch_id: Option<String>,
}

#[derive(Debug, Default)]
pub struct PostExitEnrichmentSummary {
    pub attempted: usize,
    pub complete: usize,
    pub five_day_only: usize,
    pub awaiting_five_day: usize,
    pub failed: usize,
    pub batches: Vec<BatchEvidence>,
    pub awaiting: Vec<PostExitEnrichmentAwaiting>,
    pub failures: Vec<PostExitEnrichmentFailure>,
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

fn apply_post_exit_batch(
    review: &mut TradeReview,
    batch: &AdmittedDailyBars,
) -> Result<PostExitEnrichmentState, String> {
    review.post_exit_chg_5d = None;
    review.post_exit_chg_20d = None;

    let exit_close = batch
        .records()
        .iter()
        .find(|bar| bar.date == review.sell_date)
        .map(|bar| bar.close)
        .ok_or_else(|| {
            format!(
                "{} 卖出日 {} 的已接纳日线缺失",
                review.code, review.sell_date
            )
        })?;
    if !exit_close.is_finite() || exit_close <= 0.0 {
        return Err(format!(
            "{} 卖出日 {} 收盘价非法: {}",
            review.code, review.sell_date, exit_close
        ));
    }

    // HistoricalBarsGateway guarantees newest-first records. Reversing the
    // admitted slice yields ascending trading sessions without re-sorting or
    // crossing into pre-exit history.
    let future_sessions = batch
        .records()
        .iter()
        .rev()
        .filter(|bar| bar.date > review.sell_date)
        .collect::<Vec<_>>();
    let return_from_exit = |close: f64| -> Result<f64, String> {
        if !close.is_finite() || close <= 0.0 {
            return Err(format!("{} 卖出后收盘价非法: {}", review.code, close));
        }
        Ok((close - exit_close) / exit_close * 100.0)
    };

    let Some(day_five) = future_sessions.get(4) else {
        return Ok(PostExitEnrichmentState::AwaitingFiveDay {
            available_sessions: future_sessions.len(),
        });
    };
    review.post_exit_chg_5d = Some(return_from_exit(day_five.close)?);

    let Some(day_twenty) = future_sessions.get(19) else {
        return Ok(PostExitEnrichmentState::FiveDayOnly {
            available_sessions: future_sessions.len(),
        });
    };
    review.post_exit_chg_20d = Some(return_from_exit(day_twenty.close)?);
    Ok(PostExitEnrichmentState::Complete)
}

/// Enrich closed trades with admitted daily bars while retaining every batch's
/// BR-159 evidence. One failed review is isolated and logged; it never changes
/// another review and is never converted into fabricated performance.
pub fn enrich_post_exit(reviews: &mut [TradeReview]) -> PostExitEnrichmentSummary {
    let gateway = HistoricalBarsGateway::new();
    let mut summary = PostExitEnrichmentSummary {
        attempted: reviews.len(),
        ..PostExitEnrichmentSummary::default()
    };

    for review in reviews.iter_mut() {
        let batch = match gateway.required_daily_bars(&review.code, 60) {
            Ok(batch) => batch,
            Err(error) => {
                summary.failed += 1;
                summary.failures.push(PostExitEnrichmentFailure {
                    code: review.code.clone(),
                    sell_date: review.sell_date,
                    reason: error.to_string(),
                    batch_id: None,
                });
                log::warn!(
                    "[review::journal][BR-164] {} sell_date={} 日线批次不可用: {}",
                    review.code,
                    review.sell_date,
                    error
                );
                continue;
            }
        };
        summary.batches.push(batch.evidence().clone());
        match apply_post_exit_batch(review, &batch) {
            Ok(PostExitEnrichmentState::Complete) => summary.complete += 1,
            Ok(PostExitEnrichmentState::FiveDayOnly { available_sessions }) => {
                summary.five_day_only += 1;
                summary.awaiting.push(PostExitEnrichmentAwaiting {
                    code: review.code.clone(),
                    sell_date: review.sell_date,
                    available_sessions,
                    required_sessions: 20,
                    batch_id: batch.evidence().batch_id.clone(),
                });
            }
            Ok(PostExitEnrichmentState::AwaitingFiveDay { available_sessions }) => {
                summary.awaiting_five_day += 1;
                summary.awaiting.push(PostExitEnrichmentAwaiting {
                    code: review.code.clone(),
                    sell_date: review.sell_date,
                    available_sessions,
                    required_sessions: 5,
                    batch_id: batch.evidence().batch_id.clone(),
                });
            }
            Err(error) => {
                summary.failed += 1;
                summary.failures.push(PostExitEnrichmentFailure {
                    code: review.code.clone(),
                    sell_date: review.sell_date,
                    reason: error.clone(),
                    batch_id: Some(batch.evidence().batch_id.clone()),
                });
                log::warn!(
                    "[review::journal][BR-164] {} sell_date={} enrichment rejected: {} source={} provider={:?} batch_id={}",
                    review.code,
                    review.sell_date,
                    error,
                    batch.evidence().source,
                    batch.evidence().provider,
                    batch.evidence().batch_id,
                );
                continue;
            }
        }
        log::info!(
            "[review::journal][BR-164] {} sell_date={} source={} provider={:?} batch_id={}",
            review.code,
            review.sell_date,
            batch.evidence().source,
            batch.evidence().provider,
            batch.evidence().batch_id,
        );
    }
    summary
}

/// Audit and govern one complete post-exit enrichment attempt.
///
/// Awaiting D+5/D+20 windows are expected states and remain eligible for a
/// later run. Failed acquisitions or rejected batches are blocking because a
/// partial report must not be presented as a complete review.
pub fn govern_post_exit_enrichment(
    context: &str,
    summary: &PostExitEnrichmentSummary,
) -> Result<(), String> {
    if context.trim().is_empty() {
        return Err("post-exit enrichment governance context is blank".to_string());
    }
    let classified =
        summary.complete + summary.five_day_only + summary.awaiting_five_day + summary.failed;
    if classified != summary.attempted {
        return Err(format!(
            "{context} post-exit summary count mismatch: attempted={} classified={classified}",
            summary.attempted
        ));
    }
    if summary.awaiting.len() != summary.five_day_only + summary.awaiting_five_day {
        return Err(format!(
            "{context} post-exit awaiting evidence mismatch: awaiting={} classified={}",
            summary.awaiting.len(),
            summary.five_day_only + summary.awaiting_five_day
        ));
    }
    if summary.failures.len() != summary.failed {
        return Err(format!(
            "{context} post-exit failure evidence mismatch: failures={} classified={}",
            summary.failures.len(),
            summary.failed
        ));
    }

    log::info!(
        "[review::journal][BR-164][governance] context={} attempted={} complete={} five_day_only={} awaiting_five_day={} failed={} admitted_batches={}",
        context,
        summary.attempted,
        summary.complete,
        summary.five_day_only,
        summary.awaiting_five_day,
        summary.failed,
        summary.batches.len()
    );
    for evidence in &summary.batches {
        log::info!(
            "[review::journal][BR-164][evidence] context={} provider={:?} source={} source_at={:?} observed_at={} batch_id={}",
            context,
            evidence.provider,
            evidence.source,
            evidence.source_at,
            evidence.observed_at,
            evidence.batch_id
        );
    }
    for awaiting in &summary.awaiting {
        log::warn!(
            "[review::journal][BR-164][awaiting] context={} code={} sell_date={} available_sessions={} required_sessions={} batch_id={}",
            context,
            awaiting.code,
            awaiting.sell_date,
            awaiting.available_sessions,
            awaiting.required_sessions,
            awaiting.batch_id
        );
    }
    for failure in &summary.failures {
        log::error!(
            "[review::journal][BR-164][failed] context={} code={} sell_date={} batch_id={:?} reason={}",
            context,
            failure.code,
            failure.sell_date,
            failure.batch_id,
            failure.reason
        );
    }

    if summary.failed > 0 {
        let identities = summary
            .failures
            .iter()
            .map(|failure| format!("{}@{}", failure.code, failure.sell_date))
            .collect::<Vec<_>>()
            .join(",");
        return Err(format!(
            "{context} post-exit enrichment failed={} identities=[{}]",
            summary.failed, identities
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_gateway::{AdmittedDailyBars, BatchEvidence};
    use crate::data_provider::{AdjustType, KlineData};
    use crate::market_domain::ProviderId;

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

    fn kline(date: NaiveDate, close: f64) -> KlineData {
        KlineData {
            date,
            open: close,
            high: close,
            low: close,
            close,
            volume: 100.0,
            amount: close * 100.0,
            pct_chg: 0.0,
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
            adjust: AdjustType::None,
        }
    }

    #[test]
    fn post_exit_enrichment_uses_future_trading_sessions_and_keeps_evidence() {
        let trades = vec![
            make_trade("TEST_CODE_000547", TradeDirection::Buy, 9.0, "2026-06-01"),
            make_trade("TEST_CODE_000547", TradeDirection::Sell, 10.0, "2026-06-10"),
        ];
        let mut review = review_closed_trades(&trades).unwrap().remove(0);
        let mut records = vec![kline(review.sell_date, 10.0)];
        for offset in 1..=20 {
            let close = match offset {
                5 => 12.0,
                20 => 15.0,
                _ => 10.0,
            };
            records.push(kline(
                review.sell_date + chrono::Duration::days(offset),
                close,
            ));
        }
        records.reverse();
        let batch_id = "TEST_CODE_post_exit_batch";
        let batch = AdmittedDailyBars::from_test_fixture(
            "TEST_CODE_000547",
            records,
            BatchEvidence {
                provider: ProviderId::Tdx,
                source: "TEST_CODE_magic_tdx".to_string(),
                source_at: Some("2026-06-30".to_string()),
                observed_at: "2026-06-30T15:01:00+08:00".to_string(),
                batch_id: batch_id.to_string(),
            },
        )
        .unwrap();

        let state = apply_post_exit_batch(&mut review, &batch).unwrap();

        assert_eq!(state, PostExitEnrichmentState::Complete);
        assert!((review.post_exit_chg_5d.unwrap() - 20.0).abs() < 0.001);
        assert!((review.post_exit_chg_20d.unwrap() - 50.0).abs() < 0.001);
        assert_eq!(batch.evidence().batch_id, batch_id);
    }

    #[test]
    fn post_exit_enrichment_reports_awaiting_five_trading_sessions() {
        let trades = vec![
            make_trade("TEST_CODE_000547", TradeDirection::Buy, 9.0, "2026-06-01"),
            make_trade("TEST_CODE_000547", TradeDirection::Sell, 10.0, "2026-06-10"),
        ];
        let mut review = review_closed_trades(&trades).unwrap().remove(0);
        let mut records = vec![kline(review.sell_date, 10.0)];
        for offset in 1..=4 {
            records.push(kline(
                review.sell_date + chrono::Duration::days(offset),
                11.0,
            ));
        }
        records.reverse();
        let batch = AdmittedDailyBars::from_test_fixture(
            "TEST_CODE_000547",
            records,
            BatchEvidence {
                provider: ProviderId::Tdx,
                source: "TEST_CODE_magic_tdx".to_string(),
                source_at: Some("2026-06-14".to_string()),
                observed_at: "2026-06-14T15:01:00+08:00".to_string(),
                batch_id: "TEST_CODE_awaiting_post_exit".to_string(),
            },
        )
        .unwrap();

        let state = apply_post_exit_batch(&mut review, &batch).unwrap();

        assert_eq!(
            state,
            PostExitEnrichmentState::AwaitingFiveDay {
                available_sessions: 4
            }
        );
        assert_eq!(review.post_exit_chg_5d, None);
        assert_eq!(review.post_exit_chg_20d, None);
    }

    #[test]
    fn test_code_post_exit_governance_accepts_awaiting_and_audits_evidence() {
        let summary = PostExitEnrichmentSummary {
            attempted: 1,
            complete: 0,
            five_day_only: 0,
            awaiting_five_day: 1,
            failed: 0,
            batches: vec![BatchEvidence {
                provider: ProviderId::Tdx,
                source: "TEST_CODE_magic_tdx".to_string(),
                source_at: Some("2026-06-14".to_string()),
                observed_at: "2026-06-14T15:01:00+08:00".to_string(),
                batch_id: "TEST_CODE_awaiting_batch".to_string(),
            }],
            awaiting: vec![PostExitEnrichmentAwaiting {
                code: "TEST_CODE_000547".to_string(),
                sell_date: NaiveDate::from_ymd_opt(2026, 6, 10).unwrap(),
                available_sessions: 4,
                required_sessions: 5,
                batch_id: "TEST_CODE_awaiting_batch".to_string(),
            }],
            failures: Vec::new(),
        };

        govern_post_exit_enrichment("TEST_CODE_close_review", &summary).unwrap();
    }

    #[test]
    fn test_code_post_exit_governance_rejects_failed_enrichment() {
        let summary = PostExitEnrichmentSummary {
            attempted: 1,
            failed: 1,
            failures: vec![PostExitEnrichmentFailure {
                code: "TEST_CODE_000547".to_string(),
                sell_date: NaiveDate::from_ymd_opt(2026, 6, 10).unwrap(),
                reason: "TEST_CODE gateway unavailable".to_string(),
                batch_id: None,
            }],
            ..PostExitEnrichmentSummary::default()
        };

        let error = govern_post_exit_enrichment("TEST_CODE_close_review", &summary).unwrap_err();
        assert!(error.contains("failed=1"), "{error}");
        assert!(error.contains("TEST_CODE_000547"), "{error}");
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
