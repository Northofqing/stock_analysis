//! 情绪票竞价买价（批 1.3，2026-10-23）+ 09:25 板块热度（批 2.1）。
//!
//! ## 口径（Q2 = 两者都给；Q4 = 上浮 0.5%）
//!
//! 情绪型票不走 ma 回踩 —— 回测（2026-10-22）显示涨停次日盘中触及 ma10(±1%) 只有 15.4%，
//! 给回踩价等于给一个统计上不会成交的价。改为**竞价价直给 + 分档**：
//!
//! | 竞价涨幅 | 给什么价 | 标签 |
//! |---|---|---|
//! | > +6% | 不给价 | 高开过多不追（该子集均值 -1.97%、胜率 28.4%，n=338） |
//! | +2% ~ +6% | 竞价价 | 轻仓 |
//! | -2% ~ +2% | 竞价价 × 1.005 | 保成交（Q4 默认 0.5%） |
//! | < -2% | 竞价价 | 低开博弈区（不设低开放弃线：低开 >2% 是唯一正期望分组） |
//!
//! 文案必须带「竞价买入当日 T+1 不可卖」—— 这不是免责声明，是交易规则。
//!
//! ## 边界
//!
//! - 只有 `regime=emotion` 的票进这条路径；趋势型仍走 `buy_zone` 回踩区间；
//! - 竞价价缺失（停牌/未撮合）的票不出现，不填 0（BR-222）；
//! - 09:25 前 `auction_volume_ratio` 只有首段有值，所以量比只作为参考列，不做筛选。

use std::collections::HashMap;

use diesel::prelude::*;
use diesel::sql_types::Text;

use crate::database::{ticket_lifecycle, DatabaseManager};
use crate::monitor::auction_watch::AuctionRow;
use crate::monitor::ticket_regime::{self, Regime};

/// Q4：保成交上浮比例（operator 可改）。
pub const ENSURE_FILL_MARKUP: f64 = 0.005;
/// 高开帽：竞价涨幅超过它不给买价。
pub const MAX_AUCTION_PCT: f64 = 6.0;
/// 板块热度取前几名。
const BOARD_HEAT_TOP: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputKind {
    /// WP5 目标语义：只输出参考价，不声称可执行/可成交。
    #[default]
    ReferencePrice,
    /// 旧文案；仅用于 shadow 期保持推送不变。
    LegacyAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// 高开过多，不给价
    SkipHigh,
    /// +2~+6%，轻仓
    Light,
    /// -2~+2%，上浮保成交
    EnsureFill,
    /// <-2%，低开博弈区
    Bargain,
}

impl Tier {
    pub fn label(self) -> &'static str {
        match self {
            Tier::SkipHigh => "高开过多不追",
            Tier::Light => "轻仓",
            Tier::EnsureFill => "保成交",
            Tier::Bargain => "低开博弈区",
        }
    }

    /// WP5 的参考语义：保留分档信息，但不把档位写成下单动作。
    pub fn reference_label(self) -> &'static str {
        match self {
            Tier::SkipHigh => "高开过多不追",
            Tier::Light => "轻仓参考",
            Tier::EnsureFill => "中间区间参考",
            Tier::Bargain => "低开参考",
        }
    }
}

pub fn tier_for(auction_pct: f64) -> Tier {
    if auction_pct > MAX_AUCTION_PCT {
        Tier::SkipHigh
    } else if auction_pct >= 2.0 {
        Tier::Light
    } else if auction_pct >= -2.0 {
        Tier::EnsureFill
    } else {
        Tier::Bargain
    }
}

/// 给什么价：`None` = 不给价。
pub fn quote_price(auction_price: f64, tier: Tier) -> Option<f64> {
    if auction_price <= 0.0 {
        return None;
    }
    match tier {
        Tier::SkipHigh => None,
        Tier::Light | Tier::Bargain => Some(auction_price),
        Tier::EnsureFill => Some(auction_price * (1.0 + ENSURE_FILL_MARKUP)),
    }
}

#[derive(Debug, Clone)]
pub struct PriceLine {
    pub code: String,
    pub name: String,
    pub auction_price: f64,
    pub auction_pct: f64,
    /// 给出来的价（含上浮），`None` = 不给
    pub price: Option<f64>,
    pub tier: Tier,
    /// 默认只输出参考价；旧文案要显式降级。
    pub output_kind: OutputKind,
}

impl PriceLine {
    pub fn render(&self) -> String {
        let kind = if crate::monitor::output_contract::reference_language_enabled() {
            self.output_kind
        } else {
            OutputKind::LegacyAction
        };
        self.render_as(kind)
    }

    pub fn render_as(&self, kind: OutputKind) -> String {
        match (kind, self.price) {
            (OutputKind::ReferencePrice, Some(price)) => format!(
                "{} {} 竞价 {:.2}（{:+.2}%）→ 竞价参考价 {:.2}〔{}〕",
                self.code,
                self.name,
                self.auction_price,
                self.auction_pct,
                price,
                self.tier.reference_label()
            ),
            (OutputKind::ReferencePrice, None) => format!(
                "{} {} 竞价 {:.2}（{:+.2}%）→ 不给参考价〔{}〕",
                self.code,
                self.name,
                self.auction_price,
                self.auction_pct,
                self.tier.reference_label()
            ),
            (OutputKind::LegacyAction, Some(price)) => format!(
                "{} {} 竞价 {:.2}（{:+.2}%）→ 买价 {:.2}〔{}〕",
                self.code,
                self.name,
                self.auction_price,
                self.auction_pct,
                price,
                self.tier.label()
            ),
            (OutputKind::LegacyAction, None) => format!(
                "{} {} 竞价 {:.2}（{:+.2}%）→ 不给价〔{}〕",
                self.code,
                self.name,
                self.auction_price,
                self.auction_pct,
                self.tier.label()
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BoardHeat {
    pub board_name: String,
    pub mean_pct: f64,
    pub median_volume_ratio: Option<f64>,
    pub members: usize,
}

impl BoardHeat {
    pub fn render(&self) -> String {
        let ratio = self
            .median_volume_ratio
            .map(|value| format!(" 量比中位 {value:.2}"))
            .unwrap_or_else(|| " 量比 —".to_string());
        format!(
            "{} 竞价均 {:+.2}%（{} 只）{}",
            self.board_name, self.mean_pct, self.members, ratio
        )
    }
}

#[derive(Debug, Default)]
pub struct AuctionPriceReport {
    pub emotion: usize,
    pub quoted: Vec<PriceLine>,
    pub skipped_high: Vec<PriceLine>,
    pub no_price: usize,
    /// 情绪高潮期被压掉的连板票（2026-10-24）
    pub skipped_phase: Vec<PriceLine>,
    pub errors: Vec<String>,
    pub board_heat: Vec<BoardHeat>,
}

impl AuctionPriceReport {
    pub fn summary_line(&self) -> String {
        let label = if crate::monitor::output_contract::reference_language_enabled() {
            "竞价参考价"
        } else {
            "竞价买价"
        };
        format!(
            "[{}] emotion={} quoted={} skipped_high={} skipped_phase={} no_price={} errors={}",
            label,
            self.emotion,
            self.quoted.len(),
            self.skipped_high.len(),
            self.skipped_phase.len(),
            self.no_price,
            self.errors.len()
        )
    }

    /// 推送正文；没有情绪型票时返回 `None`（不推空消息）。
    pub fn render(&self, as_of: &str) -> Option<String> {
        if self.emotion == 0 {
            return None;
        }
        let reference = crate::monitor::output_contract::reference_language_enabled();
        let mut out = String::new();
        if reference {
            out.push_str(&format!("【{as_of} 09:25 竞价参考价】\n"));
        } else {
            out.push_str(&format!("【{as_of} 09:25 竞价买价】\n"));
        }
        out.push_str(&format!(
            "情绪型 {} 只（趋势型走 ma 回踩，见明日计划）\n",
            self.emotion
        ));
        for line in &self.quoted {
            out.push_str(&format!("\n{}\n", line.render()));
        }
        for line in &self.skipped_high {
            out.push_str(&format!("\n{}\n", line.render()));
        }
        if !self.skipped_phase.is_empty() {
            out.push_str(&format!(
                "\n情绪高潮 · 连板≥{} 不追（{} 只）\n",
                crate::monitor::market_regime::EMOTION_STREAK,
                self.skipped_phase.len()
            ));
            for line in &self.skipped_phase {
                out.push_str(&format!(
                    "· {} {} 竞价{:+.2}%\n",
                    line.code, line.name, line.auction_pct
                ));
            }
        }
        if self.no_price > 0 {
            let no_price_label = if reference { "不给参考价" } else { "不给价" };
            out.push_str(&format!(
                "\n{} 只竞价价缺失（停牌/未撮合），{}\n",
                self.no_price, no_price_label
            ));
        }
        if !self.board_heat.is_empty() {
            out.push_str("\n📊 板块热度（竞价均涨幅）\n");
            for heat in &self.board_heat {
                out.push_str(&format!("· {}\n", heat.render()));
            }
        }
        if reference {
            out.push_str("\n⚠ 参考价，不是下单指令；未核验成交，T+1 不可卖。");
        } else {
            out.push_str("\n⚠ 竞价买入当日 T+1 不可卖。给的是参考价，不是成交承诺。");
        }
        Some(out)
    }
}

/// 板块热度（2.1）：竞价 rows 按 `board_member` 聚合，取均涨幅前 5。
fn board_heat(conn: &mut SqliteConnection, rows: &[AuctionRow]) -> Result<Vec<BoardHeat>, String> {
    #[derive(QueryableByName)]
    struct MemberRow {
        #[diesel(sql_type = Text)]
        board_name: String,
        #[diesel(sql_type = Text)]
        instrument_code: String,
    }
    let members: Vec<MemberRow> =
        diesel::sql_query("SELECT board_name, instrument_code FROM board_member")
            .load(conn)
            .map_err(|error| format!("板块成分查询失败: {error}"))?;
    if members.is_empty() {
        return Ok(Vec::new());
    }

    let by_code: HashMap<&str, &AuctionRow> = rows
        .iter()
        .filter_map(|row| row.auction_pct.map(|_| (row.code.as_str(), row)))
        .collect();

    let mut buckets: HashMap<&str, (Vec<f64>, Vec<f64>)> = HashMap::new();
    for member in &members {
        let Some(row) = by_code.get(member.instrument_code.as_str()) else {
            continue;
        };
        let Some(pct) = row.auction_pct else {
            continue;
        };
        let entry = buckets.entry(member.board_name.as_str()).or_default();
        entry.0.push(pct);
        if let Some(ratio) = row.auction_volume_ratio {
            entry.1.push(ratio);
        }
    }

    let mut heats: Vec<BoardHeat> = buckets
        .into_iter()
        .filter(|(_, (pcts, _))| pcts.len() >= 2)
        .map(|(board_name, (pcts, ratios))| {
            let mean = pcts.iter().sum::<f64>() / pcts.len() as f64;
            BoardHeat {
                board_name: board_name.to_string(),
                mean_pct: mean,
                median_volume_ratio: median(&ratios),
                members: pcts.len(),
            }
        })
        .collect();
    heats.sort_by(|a, b| {
        b.mean_pct
            .partial_cmp(&a.mean_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    heats.truncate(BOARD_HEAT_TOP);
    Ok(heats)
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    Some(if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    })
}

/// 给情绪型活跃票算竞价买价。
///
/// `rows` 由 `auction_watch::observe` 提供（活跃票 ∪ 持仓的竞价终态）。
pub fn build(
    db: &DatabaseManager,
    as_of: &str,
    rows: &[AuctionRow],
) -> Result<AuctionPriceReport, String> {
    let mut conn = db.get_conn().map_err(|error| error.to_string())?;
    let mut report = AuctionPriceReport::default();

    for row in rows {
        let ticket = match ticket_lifecycle::find_active_ticket(&mut conn, &row.code) {
            Ok(Some(ticket)) => ticket,
            Ok(None) => continue,
            Err(error) => {
                report.errors.push(format!("{} {error}", row.code));
                continue;
            }
        };
        let decision =
            match ticket_regime::resolve(&mut conn, ticket.id, &row.code, &ticket.source, as_of) {
                Ok(decision) => decision,
                Err(error) => {
                    report
                        .errors
                        .push(format!("{} 分型失败: {error}", row.code));
                    continue;
                }
            };
        if decision.regime != Regime::Emotion {
            continue;
        }
        report.emotion += 1;

        let (Some(auction_price), Some(auction_pct)) = (row.auction_price, row.auction_pct) else {
            report.no_price += 1;
            continue;
        };
        // 2026-10-24：情绪高潮期不给连板 ≥4 的票定价 —— 文章说的「情绪高潮不追高」，
        // 我们这边就用市场级情绪档来压，而不是逐票拍脑袋。
        let streak = crate::monitor::market_regime::streak_on(&mut conn, as_of, &row.code);
        let phase = crate::monitor::market_regime::phase_on(&mut conn, as_of);
        if streak.is_none() {
            log::debug!(
                "[竞价买价] {} 高位状态未核实（无 <= {} 的涨停池快照）",
                row.code,
                as_of
            );
        }
        let line_for_skip = || PriceLine {
            code: row.code.clone(),
            name: row.name.clone(),
            auction_price,
            auction_pct,
            price: None,
            tier: Tier::SkipHigh,
            output_kind: OutputKind::ReferencePrice,
        };
        if streak.is_some_and(|value| value >= crate::monitor::market_regime::EMOTION_STREAK)
            && phase == Some(crate::monitor::market_regime::MarketPhase::Climax)
        {
            report.skipped_phase.push(line_for_skip());
            continue;
        }
        let tier = tier_for(auction_pct);
        let price = quote_price(auction_price, tier);
        let line = PriceLine {
            code: row.code.clone(),
            name: row.name.clone(),
            auction_price,
            auction_pct,
            price,
            tier,
            output_kind: OutputKind::ReferencePrice,
        };
        match tier {
            Tier::SkipHigh => report.skipped_high.push(line),
            _ => report.quoted.push(line),
        }
    }

    report.board_heat = board_heat(&mut conn, rows)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_boundaries_match_backtest() {
        assert_eq!(tier_for(6.01), Tier::SkipHigh);
        assert_eq!(tier_for(6.0), Tier::Light);
        assert_eq!(tier_for(2.0), Tier::Light);
        assert_eq!(tier_for(1.99), Tier::EnsureFill);
        assert_eq!(tier_for(-2.0), Tier::EnsureFill);
        assert_eq!(tier_for(-2.01), Tier::Bargain);
    }

    #[test]
    fn ensure_fill_markup_is_half_a_percent() {
        let price = quote_price(10.0, Tier::EnsureFill).expect("quoted");
        assert!((price - 10.05).abs() < 1e-9);
        // 轻仓与低开不给上浮
        assert_eq!(quote_price(10.0, Tier::Light), Some(10.0));
        assert_eq!(quote_price(10.0, Tier::Bargain), Some(10.0));
        // 高开帽：不给价
        assert_eq!(quote_price(10.0, Tier::SkipHigh), None);
        // 缺价不补 0
        assert_eq!(quote_price(0.0, Tier::Light), None);
    }

    #[test]
    fn render_always_carries_t1_warning() {
        let report = AuctionPriceReport {
            emotion: 1,
            quoted: vec![PriceLine {
                code: "002696".to_string(),
                name: "测试".to_string(),
                auction_price: 10.0,
                auction_pct: 3.0,
                price: Some(10.0),
                tier: Tier::Light,
                output_kind: OutputKind::ReferencePrice,
            }],
            ..Default::default()
        };
        let text = report.render("2026-10-23").expect("non-empty");
        assert!(text.contains("T+1 不可卖"));
        assert!(text.contains("002696"));
        assert!(text.contains("轻仓"));
    }

    #[test]
    fn reference_output_never_claims_buy_or_fill() {
        let line = PriceLine {
            code: "001216".to_string(),
            name: "华瓷股份".to_string(),
            auction_price: 22.78,
            auction_pct: -5.41,
            price: Some(22.78),
            tier: Tier::Bargain,
            output_kind: OutputKind::ReferencePrice,
        };
        let text = line.render_as(OutputKind::ReferencePrice);
        assert!(text.contains("竞价参考价"));
        assert!(!text.contains("买价"));
        assert!(!text.contains("保成交"));
        assert!(!text.contains("保证成交"));
        assert!(!text.contains("可执行"));
    }

    #[test]
    fn no_emotion_tickets_means_no_push() {
        let report = AuctionPriceReport::default();
        assert!(report.render("2026-10-23").is_none());
    }

    #[test]
    fn median_handles_even_and_odd() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[1.0, 2.0, 3.0, 4.0]), Some(2.5));
        assert_eq!(median(&[]), None);
    }
}
