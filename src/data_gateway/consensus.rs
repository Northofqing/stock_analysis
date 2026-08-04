//! Registered business rules: BR-119, BR-159, BR-164.
//! Typed seller-consensus acquisition and normalization.

use crate::data_gateway::review::{
    acquisition_request_hash, audit_blocking_join_failure, audit_gateway_result, BatchEvidence,
    GatewayBatch, GatewayError,
};
use crate::data_provider::consensus::{ConsensusData, RecentReport};
use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime};
use magic_eastmoney_rs::{EastmoneyClient, EastmoneyError};
use magic_market_core::{
    InstrumentId, PositiveU32, ProviderId, ReportScope, ResearchReport, ResearchReports,
    ResearchRequest,
};
use std::collections::{HashMap, HashSet};

const CAPABILITY: &str = "consensus";
const REPORT_LIMIT: u32 = 50;
const REPORT_WINDOW_DAYS: i64 = 180;

/// Production seller-consensus seam.
///
/// The blocking typed provider is constructed, used and dropped inside a
/// `spawn_blocking` worker so it cannot drop its HTTP runtime in Tokio's async
/// execution context.
#[derive(Debug, Clone, Copy, Default)]
pub struct ConsensusDataGateway;

impl ConsensusDataGateway {
    pub const fn new() -> Self {
        Self
    }

    pub async fn fetch(&self, code: &str) -> Result<GatewayBatch<ConsensusData>, GatewayError> {
        let code = code.to_owned();
        let request_hash = acquisition_request_hash(
            CAPABILITY,
            &format!("{code}:{REPORT_WINDOW_DAYS}:{REPORT_LIMIT}"),
        );
        let worker_hash = request_hash.clone();
        let joined = tokio::task::spawn_blocking(move || {
            let result = a_share_instrument(&code).and_then(fetch_consensus_batch);
            audit_gateway_result(CAPABILITY, ProviderId::Eastmoney, &worker_hash, result)
        })
        .await;
        match joined {
            Ok(result) => result,
            Err(error) => {
                audit_blocking_join_failure(
                    CAPABILITY,
                    ProviderId::Eastmoney,
                    request_hash,
                    error.to_string(),
                )
                .await
            }
        }
    }
}

fn a_share_instrument(code: &str) -> Result<InstrumentId, GatewayError> {
    #[cfg(test)]
    let resolved = super::instrument_identity::resolve_test_equity(code, None);
    #[cfg(not(test))]
    let resolved = super::instrument_identity::resolve_production_equity(code, None);
    let identity =
        resolved.map_err(|error| GatewayError::invalid_request(CAPABILITY, error.to_string()))?;
    identity
        .require_a_share()
        .map_err(|error| GatewayError::invalid_request(CAPABILITY, error.to_string()))?;
    Ok(identity.instrument().clone())
}

fn fetch_consensus_batch(
    instrument: InstrumentId,
) -> Result<GatewayBatch<ConsensusData>, GatewayError> {
    let client = EastmoneyClient::new().map_err(map_provider_error)?;
    let request = ResearchRequest::new(
        ReportScope::Instrument(instrument.clone()),
        PositiveU32::new(1)
            .map_err(|error| GatewayError::invalid_request(CAPABILITY, error.to_string()))?,
        PositiveU32::new(REPORT_LIMIT)
            .map_err(|error| GatewayError::invalid_request(CAPABILITY, error.to_string()))?,
    )
    .map_err(|error| GatewayError::invalid_request(CAPABILITY, error.to_string()))?;
    let batch = client
        .research_reports(&request)
        .map_err(map_provider_error)?;
    if !batch.quality().is_complete() {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            format!(
                "typed research batch is not complete: {:?}",
                batch.quality()
            ),
        ));
    }
    let evidence = BatchEvidence::from_provenance(ProviderId::Eastmoney, batch.provenance())?;
    let consensus = normalize_reports(
        batch.records(),
        &instrument,
        &evidence.batch_id,
        Local::now().date_naive(),
    )?;
    Ok(GatewayBatch::Available {
        records: vec![consensus],
        evidence,
    })
}

fn normalize_reports(
    reports: &[ResearchReport],
    expected_instrument: &InstrumentId,
    expected_batch_id: &str,
    today: NaiveDate,
) -> Result<ConsensusData, GatewayError> {
    let begin = today - Duration::days(REPORT_WINDOW_DAYS);
    let mut admitted = reports
        .iter()
        .map(|report| {
            let date = parse_provider_date(report.published_at.as_str())?;
            Ok((date, report))
        })
        .collect::<Result<Vec<_>, GatewayError>>()?;
    admitted.retain(|(date, _)| *date >= begin && *date <= today);
    admitted.sort_by_key(|item| std::cmp::Reverse(item.0));

    if admitted.is_empty() {
        return Err(GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            "unavailable",
            "no_current_reports",
            false,
            format!("typed provider returned no reports in admitted window {begin}..={today}"),
        ));
    }

    let mut report_ids = HashSet::with_capacity(admitted.len());
    let mut brokers = HashSet::with_capacity(admitted.len());
    let mut rating_distribution = HashMap::new();
    let mut eps_this = Vec::new();
    let mut eps_next = Vec::new();
    let mut eps_next2 = Vec::new();
    let mut target_price_high = Vec::new();
    let mut target_price_low = Vec::new();
    let mut recent_reports = Vec::with_capacity(3);

    for (publish_date, report) in &admitted {
        let ReportScope::Instrument(source_instrument) = &report.scope else {
            return Err(invalid_evidence(
                "typed research report has non-instrument scope",
            ));
        };
        if source_instrument != expected_instrument {
            return Err(invalid_evidence(format!(
                "typed research report instrument {:?}.{} differs from requested {:?}.{}",
                source_instrument.exchange(),
                source_instrument.code(),
                expected_instrument.exchange(),
                expected_instrument.code()
            )));
        }
        if report.evidence.provider() != ProviderId::Eastmoney {
            return Err(invalid_evidence(format!(
                "typed research report provider is {:?}",
                report.evidence.provider()
            )));
        }
        if report.evidence.batch_id() != expected_batch_id {
            return Err(invalid_evidence(format!(
                "typed research report batch ID {} differs from admitted batch {}",
                report.evidence.batch_id(),
                expected_batch_id
            )));
        }
        let source_at = report
            .evidence
            .source_at()
            .ok_or_else(|| invalid_evidence("research report source_at is absent"))?;
        if parse_provider_date(source_at)? != *publish_date {
            return Err(invalid_evidence(format!(
                "research report source_at {source_at:?} differs from published_at {:?}",
                report.published_at.as_str()
            )));
        }
        if !report_ids.insert(report.report_id.as_str()) {
            return Err(invalid_evidence(format!(
                "duplicate research report ID {}",
                report.report_id.as_str()
            )));
        }
        if let (Some(high), Some(low)) = (
            report.source_indv_aim_price_t,
            report.source_indv_aim_price_l,
        ) {
            if low.get() > high.get() {
                return Err(invalid_evidence(format!(
                    "research report {} target-price lower bound {} exceeds upper bound {}",
                    report.report_id.as_str(),
                    low.get(),
                    high.get()
                )));
            }
        }
        if let Some(high) = report.source_indv_aim_price_t {
            target_price_high.push(high.get());
        }
        if let Some(low) = report.source_indv_aim_price_l {
            target_price_low.push(low.get());
        }
        let rating = report
            .rating
            .as_ref()
            .ok_or_else(|| invalid_evidence("research report rating is absent"))?
            .as_str()
            .to_owned();
        brokers.insert(report.organization.as_str().to_owned());
        *rating_distribution.entry(rating.clone()).or_insert(0_u32) += 1;

        let report_year = u32::try_from(publish_date.year())
            .map_err(|_| invalid_evidence("research report year is negative"))?;
        for estimate in &report.estimates {
            let Some(eps) = estimate.eps().map(|value| value.get()) else {
                continue;
            };
            match estimate.fiscal_year().get().checked_sub(report_year) {
                Some(0) => eps_this.push(eps),
                Some(1) => eps_next.push(eps),
                Some(2) => eps_next2.push(eps),
                _ => {}
            }
        }

        if recent_reports.len() < 3 {
            recent_reports.push(RecentReport {
                title: report.title.as_str().to_owned(),
                org_name: report.organization.as_str().to_owned(),
                publish_date: publish_date.to_string(),
                rating,
            });
        }
    }

    if eps_this.is_empty() && eps_next.is_empty() && eps_next2.is_empty() {
        return Err(invalid_evidence(
            "admitted research batch contains no EPS estimates",
        ));
    }

    Ok(ConsensusData {
        report_count: admitted.len(),
        broker_count: brokers.len(),
        eps_this_year_avg: average(&eps_this),
        eps_next_year_avg: average(&eps_next),
        eps_next2_year_avg: average(&eps_next2),
        rating_distribution,
        target_price_high_avg: average(&target_price_high),
        target_price_low_avg: average(&target_price_low),
        latest_report_date: admitted.first().map(|(date, _)| date.to_string()),
        recent_reports,
    })
}

fn parse_provider_date(raw: &str) -> Result<NaiveDate, GatewayError> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .or_else(|_| {
            NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S")
                .map(|timestamp| timestamp.date())
        })
        .or_else(|_| {
            NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f")
                .map(|timestamp| timestamp.date())
        })
        .map_err(|error| {
            invalid_evidence(format!(
                "research report published_at is invalid {raw:?}: {error}"
            ))
        })
}

fn average(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn invalid_evidence(message: impl Into<String>) -> GatewayError {
    GatewayError::invalid_evidence(CAPABILITY, Some(ProviderId::Eastmoney), message)
}

fn map_provider_error(error: EastmoneyError) -> GatewayError {
    let message = error.to_string();
    match error {
        EastmoneyError::InvalidRequest(_) => GatewayError::invalid_request(CAPABILITY, message),
        EastmoneyError::Unsupported(_) => GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            "unsupported",
            "provider_unsupported",
            false,
            message,
        ),
        EastmoneyError::Transport(_) | EastmoneyError::ResponseTooLarge { .. } => {
            GatewayError::unavailable(CAPABILITY, Some(ProviderId::Eastmoney), true, message)
        }
        EastmoneyError::VerifiedEmpty(_) => GatewayError::classified(
            CAPABILITY,
            Some(ProviderId::Eastmoney),
            "verified_empty",
            "verified_empty",
            false,
            message,
        ),
        EastmoneyError::Decode(_) | EastmoneyError::Protocol(_) | EastmoneyError::Core(_) => {
            invalid_evidence(message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use magic_market_core::{
        AssetClass, EarningsEstimate, Exchange, FiniteNumber, HttpsUrl, NonEmptyText, Price,
        SourceEvidence,
    };

    fn instrument() -> InstrumentId {
        InstrumentId::new(Exchange::Shanghai, "TEST_CODE_600396", AssetClass::Equity)
            .expect("test instrument")
    }

    fn report(
        id: &str,
        date: &str,
        broker: &str,
        rating: Option<&str>,
        eps: Option<f64>,
    ) -> ResearchReport {
        let batch_id = "TEST_CODE_consensus_batch";
        let evidence =
            SourceEvidence::new(ProviderId::Eastmoney, "2099-07-18T09:30:00+08:00", batch_id)
                .expect("evidence")
                .with_source_at(date)
                .expect("source at");
        ResearchReport {
            report_id: NonEmptyText::new(id).expect("report ID"),
            scope: ReportScope::Instrument(instrument()),
            title: NonEmptyText::new(format!("{broker}研报")).expect("title"),
            organization: NonEmptyText::new(broker).expect("broker"),
            organization_id: None,
            author: None,
            rating: rating.map(NonEmptyText::new).transpose().expect("rating"),
            industry_code: None,
            industry_name: None,
            published_at: NonEmptyText::new(date).expect("date"),
            canonical_url: HttpsUrl::new(format!("https://example.com/{id}")).expect("URL"),
            pdf_url: None,
            estimates: eps
                .map(|eps| {
                    EarningsEstimate::new(
                        PositiveU32::new(2099).expect("year"),
                        Some(FiniteNumber::new(eps).expect("EPS")),
                        None,
                        None,
                        None,
                        None,
                        None,
                    )
                    .expect("estimate")
                })
                .into_iter()
                .collect(),
            source_indv_aim_price_t: None,
            source_indv_aim_price_l: None,
            evidence,
        }
    }

    #[test]
    fn br119_normalizes_typed_reports_and_keeps_unavailable_targets_blank() {
        let today = NaiveDate::from_ymd_opt(2099, 7, 18).expect("date");
        let records = vec![
            report(
                "TEST_CODE_R2",
                "2099-07-17",
                "乙券商",
                Some("增持"),
                Some(1.5),
            ),
            report(
                "TEST_CODE_R1",
                "2099-07-18",
                "甲券商",
                Some("买入"),
                Some(1.0),
            ),
            report(
                "TEST_CODE_R3",
                "2099-07-16",
                "甲券商",
                Some("中性"),
                Some(2.0),
            ),
        ];
        let consensus =
            normalize_reports(&records, &instrument(), "TEST_CODE_consensus_batch", today)
                .expect("complete batch");

        assert_eq!(consensus.report_count, 3);
        assert_eq!(consensus.broker_count, 2);
        assert_eq!(consensus.eps_this_year_avg, Some(1.5));
        assert_eq!(consensus.bullish_ratio(), Some(2.0 / 3.0 * 100.0));
        assert_eq!(consensus.latest_report_date.as_deref(), Some("2099-07-18"));
        assert_eq!(consensus.recent_reports.len(), 3);
        assert_eq!(consensus.target_price_high_avg, None);
        assert_eq!(consensus.target_price_low_avg, None);
    }

    #[test]
    fn br119_aggregates_only_source_proven_target_price_sides() {
        let today = NaiveDate::from_ymd_opt(2099, 7, 18).expect("date");
        let mut first = report(
            "TEST_CODE_R1",
            "2099-07-18",
            "甲券商",
            Some("买入"),
            Some(1.0),
        );
        first.source_indv_aim_price_t = Some(Price::new(14.0).expect("upper"));
        first.source_indv_aim_price_l = Some(Price::new(10.0).expect("lower"));
        let mut second = report(
            "TEST_CODE_R2",
            "2099-07-17",
            "乙券商",
            Some("增持"),
            Some(2.0),
        );
        second.source_indv_aim_price_t = Some(Price::new(16.0).expect("upper"));

        let consensus = normalize_reports(
            &[first, second],
            &instrument(),
            "TEST_CODE_consensus_batch",
            today,
        )
        .expect("same-batch source target prices");
        assert_eq!(consensus.target_price_high_avg, Some(15.0));
        assert_eq!(consensus.target_price_low_avg, Some(10.0));
    }

    #[test]
    fn br119_rejects_inverted_source_target_price_bounds() {
        let today = NaiveDate::from_ymd_opt(2099, 7, 18).expect("date");
        let mut inverted = report(
            "TEST_CODE_R1",
            "2099-07-18",
            "甲券商",
            Some("买入"),
            Some(1.0),
        );
        inverted.source_indv_aim_price_t = Some(Price::new(8.0).expect("upper"));
        inverted.source_indv_aim_price_l = Some(Price::new(9.0).expect("lower"));

        let error = normalize_reports(
            &[inverted],
            &instrument(),
            "TEST_CODE_consensus_batch",
            today,
        )
        .expect_err("inverted source bounds must reject the batch");
        assert_eq!(error.reason_code(), "invalid_evidence");
    }

    #[test]
    fn br119_rejects_missing_rating_eps_duplicates_and_stale_batches() {
        let today = NaiveDate::from_ymd_opt(2099, 7, 18).expect("date");
        let no_rating = vec![report(
            "TEST_CODE_R1",
            "2099-07-18",
            "甲券商",
            None,
            Some(1.0),
        )];
        assert!(normalize_reports(
            &no_rating,
            &instrument(),
            "TEST_CODE_consensus_batch",
            today
        )
        .is_err());

        let no_eps = vec![report(
            "TEST_CODE_R1",
            "2099-07-18",
            "甲券商",
            Some("买入"),
            None,
        )];
        assert!(
            normalize_reports(&no_eps, &instrument(), "TEST_CODE_consensus_batch", today).is_err()
        );

        let duplicate = vec![
            report(
                "TEST_CODE_R1",
                "2099-07-18",
                "甲券商",
                Some("买入"),
                Some(1.0),
            ),
            report(
                "TEST_CODE_R1",
                "2099-07-17",
                "乙券商",
                Some("增持"),
                Some(1.1),
            ),
        ];
        assert!(normalize_reports(
            &duplicate,
            &instrument(),
            "TEST_CODE_consensus_batch",
            today
        )
        .is_err());

        let stale = vec![report(
            "TEST_CODE_R1",
            "2098-01-01",
            "甲券商",
            Some("买入"),
            Some(1.0),
        )];
        assert!(
            normalize_reports(&stale, &instrument(), "TEST_CODE_consensus_batch", today).is_err()
        );
    }

    #[test]
    fn br119_a_share_identity_mapping_is_bounded_and_explicit() {
        let cases = [
            ("TEST_CODE_600396", Exchange::Shanghai),
            ("TEST_CODE_000001", Exchange::Shenzhen),
            ("TEST_CODE_300001", Exchange::Shenzhen),
            ("TEST_CODE_920001", Exchange::Beijing),
        ];
        for (code, exchange) in cases {
            let instrument = a_share_instrument(code).unwrap();
            assert_eq!(instrument.exchange(), exchange);
            assert_eq!(instrument.asset_class(), AssetClass::Equity);
            assert_eq!(instrument.code(), code);
        }
        for code in [
            "TEST_CODE_430001",
            "TEST_CODE_830001",
            "TEST_CODE_200001",
            "TEST_CODE_900901",
            "TEST_CODE_500001",
            "TEST_CODE_A00396",
        ] {
            assert!(a_share_instrument(code).is_err());
        }
    }

    #[test]
    fn br119_scope_provider_batch_and_source_evidence_must_match() {
        let today = NaiveDate::from_ymd_opt(2099, 7, 18).unwrap();
        let base = report(
            "TEST_CODE_R1",
            "2099-07-18",
            "甲券商",
            Some("买入"),
            Some(1.0),
        );

        let mut industry = base.clone();
        industry.scope = ReportScope::Industry(NonEmptyText::new("TEST_CODE industry").unwrap());
        assert!(normalize_reports(
            &[industry],
            &instrument(),
            "TEST_CODE_consensus_batch",
            today
        )
        .is_err());

        let mut wrong_instrument = base.clone();
        wrong_instrument.scope = ReportScope::Instrument(
            InstrumentId::new(Exchange::Shenzhen, "TEST_CODE_000001", AssetClass::Equity).unwrap(),
        );
        assert!(normalize_reports(
            &[wrong_instrument],
            &instrument(),
            "TEST_CODE_consensus_batch",
            today
        )
        .is_err());

        let mut wrong_provider = base.clone();
        wrong_provider.evidence = SourceEvidence::new(
            ProviderId::Cninfo,
            "2099-07-18T09:30:00+08:00",
            "TEST_CODE_consensus_batch",
        )
        .unwrap()
        .with_source_at("2099-07-18")
        .unwrap();
        assert!(normalize_reports(
            &[wrong_provider],
            &instrument(),
            "TEST_CODE_consensus_batch",
            today
        )
        .is_err());

        let mut wrong_batch = base.clone();
        wrong_batch.evidence = SourceEvidence::new(
            ProviderId::Eastmoney,
            "2099-07-18T09:30:00+08:00",
            "TEST_CODE_other_batch",
        )
        .unwrap()
        .with_source_at("2099-07-18")
        .unwrap();
        assert!(normalize_reports(
            &[wrong_batch],
            &instrument(),
            "TEST_CODE_consensus_batch",
            today
        )
        .is_err());

        let mut missing_source_at = base.clone();
        missing_source_at.evidence = SourceEvidence::new(
            ProviderId::Eastmoney,
            "2099-07-18T09:30:00+08:00",
            "TEST_CODE_consensus_batch",
        )
        .unwrap();
        assert!(normalize_reports(
            &[missing_source_at],
            &instrument(),
            "TEST_CODE_consensus_batch",
            today
        )
        .is_err());

        let mut mismatched_source_at = base;
        mismatched_source_at.evidence = SourceEvidence::new(
            ProviderId::Eastmoney,
            "2099-07-18T09:30:00+08:00",
            "TEST_CODE_consensus_batch",
        )
        .unwrap()
        .with_source_at("2099-07-17")
        .unwrap();
        assert!(normalize_reports(
            &[mismatched_source_at],
            &instrument(),
            "TEST_CODE_consensus_batch",
            today
        )
        .is_err());
    }

    #[test]
    fn br119_provider_dates_eps_horizons_and_averages_are_normalized() {
        assert_eq!(
            parse_provider_date("2099-07-18 12:34:56").unwrap(),
            NaiveDate::from_ymd_opt(2099, 7, 18).unwrap()
        );
        assert_eq!(
            parse_provider_date("2099-07-18 12:34:56.123").unwrap(),
            NaiveDate::from_ymd_opt(2099, 7, 18).unwrap()
        );
        assert!(parse_provider_date("TEST_CODE_bad_date").is_err());
        assert_eq!(average(&[]), None);
        assert_eq!(average(&[1.0, 3.0]), Some(2.0));

        let mut row = report(
            "TEST_CODE_R1",
            "2099-07-18",
            "甲券商",
            Some("买入"),
            Some(1.0),
        );
        row.estimates.extend([
            EarningsEstimate::new(
                PositiveU32::new(2100).unwrap(),
                Some(FiniteNumber::new(2.0).unwrap()),
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap(),
            EarningsEstimate::new(
                PositiveU32::new(2101).unwrap(),
                Some(FiniteNumber::new(3.0).unwrap()),
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap(),
            EarningsEstimate::new(
                PositiveU32::new(2102).unwrap(),
                Some(FiniteNumber::new(99.0).unwrap()),
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap(),
        ]);
        let normalized =
            normalize_reports(&[row], &instrument(), "TEST_CODE_consensus_batch", today()).unwrap();
        assert_eq!(normalized.eps_this_year_avg, Some(1.0));
        assert_eq!(normalized.eps_next_year_avg, Some(2.0));
        assert_eq!(normalized.eps_next2_year_avg, Some(3.0));
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2099, 7, 18).unwrap()
    }

    #[test]
    fn br119_provider_errors_keep_retryability_semantics() {
        let cases = [
            map_provider_error(EastmoneyError::InvalidRequest("TEST_CODE".into())),
            map_provider_error(EastmoneyError::Unsupported("TEST_CODE".into())),
            map_provider_error(EastmoneyError::Transport("TEST_CODE".into())),
            map_provider_error(EastmoneyError::ResponseTooLarge { limit: 1 }),
            map_provider_error(EastmoneyError::Decode("TEST_CODE".into())),
            map_provider_error(EastmoneyError::Protocol("TEST_CODE".into())),
        ];
        assert_eq!(cases[0].audit_outcome(), "invalid_request");
        assert_eq!(cases[1].audit_outcome(), "unsupported");
        for error in &cases[2..4] {
            assert_eq!(error.audit_outcome(), "unavailable");
            assert!(error.retryable());
        }
        for error in &cases[4..] {
            assert_eq!(error.audit_outcome(), "partial");
            assert!(!error.retryable());
        }
    }
}
