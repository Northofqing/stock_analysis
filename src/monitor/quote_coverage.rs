//! 实时行情覆盖率：把「请求了多少、拿到多少、缺了谁、哪批失败」变成可核对的数字。
//!
//! 2026-10-27 WP3.5：此前观察池和盘中选票只记录「有行情 N 只」，失败时整批跳过，
//! 验收也只能数「查询失败」文本。这里统一成可合并的 coverage，避免把数据缺失
//! 当成「今天没有候选」。

use std::collections::HashSet;

use crate::data_gateway::market_data::RealtimeMarketQuote;
use crate::monitor::ticket_decision::SourceHealth;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct QuoteCoverage {
    pub requested: usize,
    pub accepted: usize,
    pub missing: Vec<String>,
    /// 被行情源整批拒绝/隔离的票；与 missing 分开，避免把业务态当故障。
    pub rejected: Vec<String>,
    pub batches: usize,
    pub failed_batches: usize,
}

impl QuoteCoverage {
    /// 正常响应：按请求代码计算缺失，不把未请求的响应算进 accepted。
    pub fn observed(requested: &[String], quotes: &[RealtimeMarketQuote]) -> Self {
        let requested_codes = unique_codes(requested);
        let requested_set: HashSet<&str> =
            requested_codes.iter().map(String::as_str).collect();
        let accepted_codes: HashSet<&str> = quotes
            .iter()
            .map(|quote| quote.code.trim())
            .filter(|code| requested_set.contains(code))
            .collect();
        let missing: Vec<String> = requested_codes
            .iter()
            .filter(|code| !accepted_codes.contains(code.as_str()))
            .cloned()
            .collect();
        Self {
            requested: requested_codes.len(),
            accepted: accepted_codes.len(),
            missing,
            rejected: Vec::new(),
            batches: 1,
            failed_batches: 0,
        }
    }

    /// 整批请求失败：所有请求代码都进入 rejected，不能当成空结果。
    pub fn failed(requested: &[String]) -> Self {
        let requested_codes = unique_codes(requested);
        Self {
            requested: requested_codes.len(),
            accepted: 0,
            missing: Vec::new(),
            rejected: requested_codes,
            batches: 1,
            failed_batches: 1,
        }
    }

    pub fn merge(&mut self, other: Self) {
        self.requested += other.requested;
        self.accepted += other.accepted;
        self.missing.extend(other.missing);
        self.rejected.extend(other.rejected);
        self.batches += other.batches;
        self.failed_batches += other.failed_batches;
    }

    pub fn health(&self) -> SourceHealth {
        if self.requested == 0 {
            SourceHealth::Available
        } else if self.accepted == 0 {
            SourceHealth::Unavailable
        } else if !self.missing.is_empty() || !self.rejected.is_empty() {
            SourceHealth::Partial
        } else {
            SourceHealth::Available
        }
    }

    pub fn coverage_pct(&self) -> Option<f64> {
        if self.requested == 0 {
            None
        } else {
            Some(self.accepted as f64 / self.requested as f64 * 100.0)
        }
    }

    pub fn summary(&self) -> String {
        let coverage = self
            .coverage_pct()
            .map(|value| format!("{value:.1}%"))
            .unwrap_or_else(|| "n/a".to_owned());
        format!(
            "requested={} accepted={} missing={} rejected={} coverage={} health={} batches={} failed_batches={}",
            self.requested,
            self.accepted,
            self.missing.len(),
            self.rejected.len(),
            coverage,
            self.health().as_str(),
            self.batches,
            self.failed_batches
        )
    }

    pub fn log(&self, scope: &str) {
        log::info!("[观察池][coverage] scope={} {}", scope, self.summary());
    }
}

fn unique_codes(codes: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    codes
        .iter()
        .filter_map(|code| {
            let code = code.trim();
            if code.is_empty() || !seen.insert(code.to_owned()) {
                None
            } else {
                Some(code.to_owned())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market_domain::ProviderId;
    use chrono::Utc;

    fn quote(code: &str, price: f64) -> RealtimeMarketQuote {
        let now = Utc::now();
        RealtimeMarketQuote {
            code: code.to_owned(),
            name: code.to_owned(),
            price,
            previous_close: price,
            change_percent: 0.0,
            source_at: now,
            observed_at: now,
            provider: ProviderId::Tencent,
            batch_id: "test".to_owned(),
        }
    }

    #[test]
    fn observed_marks_missing_and_keeps_accepted() {
        let requested = vec!["600396".to_owned(), "001216".to_owned(), "  ".to_owned()];
        let coverage = QuoteCoverage::observed(&requested, &[quote("600396", 10.0)]);
        assert_eq!(coverage.requested, 2);
        assert_eq!(coverage.accepted, 1);
        assert_eq!(coverage.missing, vec!["001216".to_string()]);
        assert_eq!(coverage.health(), SourceHealth::Partial);
        assert_eq!(coverage.coverage_pct(), Some(50.0));
    }

    #[test]
    fn empty_response_is_unavailable_not_zero_candidates() {
        let requested = vec!["600396".to_owned()];
        let coverage = QuoteCoverage::observed(&requested, &[]);
        assert_eq!(coverage.health(), SourceHealth::Unavailable);
        assert_eq!(coverage.missing, requested);
    }

    #[test]
    fn failed_batch_is_rejected_and_merge_accumulates() {
        let mut coverage = QuoteCoverage::failed(&["600396".to_owned()]);
        coverage.merge(QuoteCoverage::observed(
            &["001216".to_owned()],
            &[quote("001216", 10.0)],
        ));
        assert_eq!(coverage.requested, 2);
        assert_eq!(coverage.accepted, 1);
        assert_eq!(coverage.rejected, vec!["600396".to_string()]);
        assert_eq!(coverage.failed_batches, 1);
        assert_eq!(coverage.health(), SourceHealth::Partial);
    }

    #[test]
    fn empty_request_is_available_with_no_percentage() {
        let coverage = QuoteCoverage::default();
        assert_eq!(coverage.health(), SourceHealth::Available);
        assert_eq!(coverage.coverage_pct(), None);
        assert!(coverage.summary().contains("coverage=n/a"));
    }
}
