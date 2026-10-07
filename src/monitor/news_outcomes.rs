//! Original physically accepted D-01 cards and subsequent session prices.
//! All calculations are read-only close-to-close observations, not entry fills,
//! news causation, benchmark alpha, paper P&L or automatic strategy weights.
use crate::database::DatabaseManager;
use crate::durable_delivery::{DurableDeliveryCoordinator, NewsToIdeaTerminal};
use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use diesel::Connection;

#[derive(Debug, serde::Serialize)]
pub struct NewsOutcomeWindow {
    pub horizon: usize,
    pub target: Option<NaiveDate>,
    pub change_pct: Option<f64>,
    pub unavailable: Option<String>,
}
#[derive(Debug, serde::Serialize)]
pub struct NewsOutcomeRow {
    pub code: String,
    pub decision_identity: String,
    pub occurrence_identity: String,
    pub source_sha256: String,
    pub content_sha256: String,
    pub terminal: String,
    pub accepted_at: Option<DateTime<Utc>>,
    pub terminal_evidence_sha256: Option<String>,
    pub original_publication_time_available: bool,
    pub original_push_price_available: bool,
    pub windows: Vec<NewsOutcomeWindow>,
}
#[derive(Debug, serde::Serialize)]
pub struct NewsOutcomeReport {
    pub business_date: NaiveDate,
    pub completed_as_of: NaiveDate,
    pub original_cards: usize,
    pub physically_accepted_cards: usize,
    pub rows: Vec<NewsOutcomeRow>,
    pub scope: &'static str,
}

pub fn read_news_outcomes(
    db: &DatabaseManager,
    coordinator: &DurableDeliveryCoordinator,
    business_date: NaiveDate,
    now: DateTime<FixedOffset>,
) -> Result<NewsOutcomeReport, String> {
    let as_of = super::prediction::completed_session_as_of_at(now)?;
    let cards = coordinator
        .read_news_to_idea_cards(&business_date.to_string())
        .map_err(|e| e.to_string())?;
    let mut conn = db.get_conn().map_err(|e| e.to_string())?;
    let rows = conn
        .transaction::<Vec<_>, Box<dyn std::error::Error>, _>(|conn| {
            cards
                .iter()
                .map(|card| {
                    let physical = card.terminal() == NewsToIdeaTerminal::Accepted;
                    let delivery_clock_valid = card.accepted_at().is_some_and(|accepted| {
                        accepted <= now.with_timezone(&chrono::Utc)
                            && accepted.with_timezone(now.offset()).date_naive() == business_date
                    });
                    let mut windows = Vec::with_capacity(3);
                    for horizon in [1, 3, 5] {
                        let mut window = NewsOutcomeWindow {
                            horizon,
                            target: None,
                            change_pct: None,
                            unavailable: None,
                        };
                        if !physical {
                            window.unavailable =
                                Some("original_card_not_physically_accepted".into());
                        } else if !delivery_clock_valid {
                            window.unavailable =
                                Some("accepted_time_not_bound_to_business_date".into());
                        } else if !crate::calendar::verified_a_share_trading_day(business_date)? {
                            window.unavailable =
                                Some("business_date_is_not_verified_trading_session".into());
                        } else {
                            let mut target = business_date;
                            for _ in 0..horizon {
                                if target >= as_of {
                                    break;
                                }
                                target =
                                    crate::calendar::verified_next_a_share_trading_day(target)?;
                            }
                            // Count again only across known sessions; unknown future
                            // calendar coverage is not needed for an immature window.
                            let mut count = 0;
                            let mut cursor = business_date;
                            while cursor < target {
                                cursor =
                                    crate::calendar::verified_next_a_share_trading_day(cursor)?;
                                count += 1;
                            }
                            if count != horizon || target > as_of {
                                window.unavailable = Some("window_not_mature".into());
                            } else {
                                window.target = Some(target);
                                let start = super::prediction::read_exact_close_on(
                                    conn,
                                    card.code(),
                                    &business_date.to_string(),
                                )?;
                                let close = super::prediction::read_exact_close_on(
                                    conn,
                                    card.code(),
                                    &target.to_string(),
                                )?;
                                match start.zip(close) {
                                    Some((start, close))
                                        if start.is_finite()
                                            && start > 0.
                                            && close.is_finite()
                                            && close > 0. =>
                                    {
                                        let change = (close - start) / start * 100.;
                                        if change.is_finite() && change >= -100. {
                                            window.change_pct = Some(change);
                                        } else {
                                            window.unavailable =
                                                Some("nonfinite_or_impossible_return".into());
                                        }
                                    }
                                    _ => {
                                        window.unavailable = Some(
                                            "exact_close_or_trading_authority_unavailable".into(),
                                        )
                                    }
                                }
                            }
                        }
                        windows.push(window);
                    }
                    Ok(NewsOutcomeRow {
                        code: card.code().into(),
                        decision_identity: card.decision_identity().into(),
                        occurrence_identity: card.occurrence_identity().into(),
                        source_sha256: card.source_sha256().into(),
                        content_sha256: card.content_sha256().into(),
                        terminal: format!("{:?}", card.terminal()),
                        accepted_at: card.accepted_at(),
                        terminal_evidence_sha256: card
                            .terminal_evidence_sha256()
                            .map(str::to_owned),
                        original_publication_time_available: false,
                        original_push_price_available: false,
                        windows,
                    })
                })
                .collect()
        })
        .map_err(|e| e.to_string())?;
    Ok(NewsOutcomeReport {business_date,completed_as_of:as_of,original_cards:cards.len(),
        physically_accepted_cards:cards.iter().filter(|card|card.terminal()==NewsToIdeaTerminal::Accepted).count(),rows,
        scope:"original counted cards; business-date close to T+1/3/5 close; not pushed_stocks membership, push-price returns, execution, costs or news alpha"})
}

impl NewsOutcomeReport {
    /// Expose missing observations and denominators beside the original card
    /// identities. A price movement is not a fill or a causal news return.
    pub fn render_markdown(&self) -> Result<String, String> {
        let mut text = format!(
            "# 新闻后续价格观察 {}\n\n截至已完成交易日 {}。原 counted 卡片 {} 条，其中物理接纳 {} 条。\n\n收益口径：业务日收盘到后续交易日收盘；原新闻发布时间和原推送价格未提供。每个窗口只统计有独立交易资格和两端收盘价的记录。不能据此认定实际成交、成本后盈利或新闻因果效果。\n\n| 窗口 | 具备观察值 | 原因待定或未成熟 |\n|---|---:|---:|\n",
            self.business_date, self.completed_as_of, self.original_cards,
            self.physically_accepted_cards,
        );
        for horizon in [1, 3, 5] {
            let available = self
                .rows
                .iter()
                .filter(|row| {
                    row.windows
                        .iter()
                        .any(|window| window.horizon == horizon && window.change_pct.is_some())
                })
                .count();
            text.push_str(&format!(
                "| T+{horizon} | {available} | {} |\n",
                self.rows.len() - available
            ));
        }
        text.push_str("\n## 原卡片与逐窗口缺失原因\n\n```json\n");
        text.push_str(&serde_json::to_string_pretty(self).map_err(|error| error.to_string())?);
        text.push_str("\n```\n");
        Ok(text)
    }
}
