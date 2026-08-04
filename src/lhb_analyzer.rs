//! BR-162 pure auxiliary scoring for admitted dragon-tiger reviews.
//!
//! Acquisition, validation, aggregation, ordering, and immutable audit belong
//! to [`crate::data_gateway::DragonTigerGateway`]. This module owns no client,
//! cache, or database and does not infer institutions, hot-money style,
//! security names, price changes, or recommendation ratings.

use crate::data_gateway::DragonTigerStockReview;
use anyhow::{ensure, Result};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// Deterministic score derived only from fields present in one admitted review.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LhbAnalysis {
    pub code: String,
    pub disclosure_count: usize,
    pub explicit_net_count: usize,
    pub positive_net_count: usize,
    pub average_net_amount_yuan: f64,
    pub total_score: i32,
}

/// Accept the two documented CLI date forms and reject all other input.
pub fn parse_dragon_tiger_date(input: &str) -> Result<NaiveDate> {
    let format = match input.len() {
        8 if input.bytes().all(|byte| byte.is_ascii_digit()) => "%Y%m%d",
        10 => "%Y-%m-%d",
        _ => anyhow::bail!("龙虎榜日期格式非法: {input:?}，仅支持 YYYYMMDD 或 YYYY-MM-DD"),
    };
    NaiveDate::parse_from_str(input, format)
        .map_err(|error| anyhow::anyhow!("龙虎榜日期非法 {input:?}: {error}"))
}

/// Score one Gateway-admitted stock review under BR-162.
pub fn analyze_dragon_tiger_review(review: &DragonTigerStockReview) -> Result<LhbAnalysis> {
    let source_code = review
        .code
        .strip_prefix("TEST_CODE_")
        .unwrap_or(&review.code);
    ensure!(
        source_code.len() == 6 && source_code.bytes().all(|byte| byte.is_ascii_digit()),
        "龙虎榜评分证券代码非法: {:?}",
        review.code
    );
    ensure!(
        review.ranking_net_amount_yuan.is_finite() && review.ranking_net_amount_yuan > 0.0,
        "龙虎榜评分排名净额必须为正且有限: code={} value={}",
        review.code,
        review.ranking_net_amount_yuan
    );
    ensure!(
        !review.disclosures.is_empty(),
        "龙虎榜评分缺少源披露: code={}",
        review.code
    );

    let explicit_nets = review
        .disclosures
        .iter()
        .filter_map(|disclosure| disclosure.net_amount_yuan)
        .map(|value| {
            ensure!(
                value.is_finite(),
                "龙虎榜评分源净额非有限值: code={} value={value}",
                review.code
            );
            Ok(value)
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        !explicit_nets.is_empty(),
        "龙虎榜评分没有显式源净额: code={}",
        review.code
    );

    let disclosure_count = review.disclosures.len();
    let explicit_net_count = explicit_nets.len();
    let positive_net_count = explicit_nets.iter().filter(|value| **value > 0.0).count();
    let average_net_amount_yuan =
        explicit_nets.iter().copied().sum::<f64>() / explicit_net_count as f64;
    ensure!(
        average_net_amount_yuan.is_finite(),
        "龙虎榜评分平均净额非有限值: code={}",
        review.code
    );

    let disclosure_score = (disclosure_count * 10).min(30) as i32;
    let positive_ratio_score =
        ((positive_net_count as f64 / explicit_net_count as f64) * 40.0).floor() as i32;
    let average_net_score = if average_net_amount_yuan > 50_000_000.0 {
        30
    } else if average_net_amount_yuan > 10_000_000.0 {
        20
    } else if average_net_amount_yuan > 0.0 {
        10
    } else {
        0
    };

    Ok(LhbAnalysis {
        code: review.code.clone(),
        disclosure_count,
        explicit_net_count,
        positive_net_count,
        average_net_amount_yuan,
        total_score: (disclosure_score + positive_ratio_score + average_net_score).min(100),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_gateway::DragonTigerSourceDisclosure;
    use magic_market_core::Exchange;

    fn disclosure(trade_id: &str, net_amount_yuan: Option<f64>) -> DragonTigerSourceDisclosure {
        DragonTigerSourceDisclosure {
            entry_id: format!("TEST_CODE_2026-07-24:{trade_id}"),
            trade_id: trade_id.to_string(),
            reason: Some("TEST_CODE_真实上榜原因".to_string()),
            buy_amount_yuan: net_amount_yuan.map(|value| value.max(0.0) + 10_000_000.0),
            sell_amount_yuan: Some(10_000_000.0),
            net_amount_yuan,
            turnover_rate_pct: Some(5.0),
            seats: Vec::new(),
        }
    }

    fn review(nets: &[Option<f64>]) -> DragonTigerStockReview {
        DragonTigerStockReview {
            exchange: Exchange::Shanghai,
            code: "TEST_CODE_600000".to_string(),
            ranking_net_amount_yuan: 60_000_000.0,
            disclosures: nets
                .iter()
                .enumerate()
                .map(|(index, value)| disclosure(&(index + 1).to_string(), *value))
                .collect(),
        }
    }

    #[test]
    fn br162_score_uses_only_explicit_disclosure_facts() {
        let analysis = analyze_dragon_tiger_review(&review(&[
            Some(60_000_000.0),
            Some(30_000_000.0),
            Some(90_000_000.0),
        ]))
        .expect("complete facts score");

        assert_eq!(analysis.code, "TEST_CODE_600000");
        assert_eq!(analysis.disclosure_count, 3);
        assert_eq!(analysis.explicit_net_count, 3);
        assert_eq!(analysis.positive_net_count, 3);
        assert_eq!(analysis.average_net_amount_yuan, 60_000_000.0);
        assert_eq!(analysis.total_score, 100);
    }

    #[test]
    fn br162_missing_optional_net_is_not_filled_or_counted() {
        let analysis =
            analyze_dragon_tiger_review(&review(&[Some(20_000_000.0), None, Some(-5_000_000.0)]))
                .expect("partial optional facts remain explicit");

        assert_eq!(analysis.disclosure_count, 3);
        assert_eq!(analysis.explicit_net_count, 2);
        assert_eq!(analysis.positive_net_count, 1);
        assert_eq!(analysis.average_net_amount_yuan, 7_500_000.0);
        assert_eq!(analysis.total_score, 60);
    }

    #[test]
    fn br162_rejects_invalid_or_fact_free_reviews() {
        let mut no_explicit_net = review(&[None]);
        assert!(analyze_dragon_tiger_review(&no_explicit_net).is_err());

        no_explicit_net.code = "TEST_CODE_BAD".to_string();
        assert!(analyze_dragon_tiger_review(&no_explicit_net).is_err());

        let mut bad_ranking = review(&[Some(1.0)]);
        bad_ranking.ranking_net_amount_yuan = f64::NAN;
        assert!(analyze_dragon_tiger_review(&bad_ranking).is_err());

        let mut empty = review(&[Some(1.0)]);
        empty.disclosures.clear();
        assert!(analyze_dragon_tiger_review(&empty).is_err());
    }

    #[test]
    fn br162_date_parser_accepts_only_documented_valid_forms() {
        let expected = NaiveDate::from_ymd_opt(2026, 7, 24).expect("valid date");
        assert_eq!(parse_dragon_tiger_date("20260724").unwrap(), expected);
        assert_eq!(parse_dragon_tiger_date("2026-07-24").unwrap(), expected);
        for invalid in ["20260230", "2026/07/24", "2026-7-24", "", "not-a-date"] {
            assert!(
                parse_dragon_tiger_date(invalid).is_err(),
                "unexpected valid date: {invalid}"
            );
        }
    }
}
