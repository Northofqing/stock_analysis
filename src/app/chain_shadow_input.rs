//! Read-only fingerprint of the scheduled chain report input, not Foundation shadow parity.

use anyhow::Result;
use chrono::NaiveDate;
use sha2::{Digest, Sha256};
use stock_analysis::pipeline::chain_analysis::preparation::PreparedChainAnalysis;

use super::chain_schedule::ChainPhase;

pub(super) const COVERAGE: &str = "prepared_report_utf8_only";

#[derive(Debug)]
pub(super) struct ChainReportInputObservation {
    pub phase: ChainPhase,
    pub schedule_date: NaiveDate,
    pub prepared_business_date: NaiveDate,
    pub artifact_sha256: String,
    pub artifact_bytes: usize,
    pub report_input_sha256: String,
    pub report_input_bytes: usize,
    pub prepared_report_equals_input: bool,
    pub coverage: &'static str,
}

/// `report_input` is the UTF-8 report supplied to the legacy sender, not a
/// rendered transport payload or channel wire bytes.
pub(super) fn observe(
    phase: ChainPhase,
    schedule_date: NaiveDate,
    prepared: &PreparedChainAnalysis,
    report_input: &[u8],
) -> Result<ChainReportInputObservation> {
    let artifact = prepared.to_artifact_bytes()?;
    Ok(ChainReportInputObservation {
        phase,
        schedule_date,
        prepared_business_date: prepared.business_date(),
        artifact_sha256: format!("{:x}", Sha256::digest(&artifact)),
        artifact_bytes: artifact.len(),
        report_input_sha256: format!("{:x}", Sha256::digest(report_input)),
        report_input_bytes: report_input.len(),
        prepared_report_equals_input: prepared.report().as_bytes() == report_input,
        coverage: COVERAGE,
    })
}

#[cfg(test)]
pub(super) async fn test_prepared(
    date: NaiveDate,
    preparations: std::rc::Rc<std::cell::Cell<usize>>,
) -> PreparedChainAnalysis {
    use stock_analysis::pipeline::chain_analysis::preparation::{
        prepare_chain_analysis_with_io, ChainPreparationIo,
    };
    struct ScriptedIo(std::rc::Rc<std::cell::Cell<usize>>);
    #[async_trait::async_trait(?Send)]
    impl ChainPreparationIo for ScriptedIo {
        fn validate_fixed_input(
            &mut self,
            _date: NaiveDate,
            _stocks: &[stock_analysis::market_data::TopStock],
            _macro_news: &Option<String>,
        ) -> Result<()> {
            self.0.set(self.0.get() + 1);
            Ok(())
        }
        async fn concepts(
            &mut self,
            _codes: &[String],
        ) -> Result<std::collections::HashMap<String, Vec<String>>> {
            panic!("empty scripted pool must not request concepts")
        }
    }
    prepare_chain_analysis_with_io(date, Vec::new(), None, &mut ScriptedIo(preparations))
        .await
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};

    #[tokio::test]
    async fn separates_schedule_and_business_dates_and_labels_digest_scope() {
        let business_date = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
        let schedule_date = NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();
        let preparations = Rc::new(Cell::new(0));
        let prepared = test_prepared(business_date, preparations.clone()).await;
        let input = prepared.report().as_bytes();
        let observed = observe(ChainPhase::Preopen, schedule_date, &prepared, input).unwrap();
        assert_eq!(preparations.get(), 1);
        assert_eq!(observed.schedule_date, schedule_date);
        assert_eq!(observed.prepared_business_date, business_date);
        assert!(observed.prepared_report_equals_input);
        assert_eq!(observed.coverage, "prepared_report_utf8_only");
        assert_eq!(observed.report_input_bytes, input.len());
        assert_eq!(
            observed.artifact_bytes,
            prepared.to_artifact_bytes().unwrap().len()
        );
        assert_eq!(observed.artifact_sha256.len(), 64);
        assert_eq!(observed.report_input_sha256.len(), 64);
    }

    #[tokio::test]
    async fn altered_report_input_is_a_visible_difference() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();
        let prepared = test_prepared(date, Rc::new(Cell::new(0))).await;
        let original = observe(
            ChainPhase::Postclose,
            date,
            &prepared,
            prepared.report().as_bytes(),
        )
        .unwrap();
        let altered = observe(ChainPhase::Postclose, date, &prepared, b"changed input").unwrap();
        assert!(!altered.prepared_report_equals_input);
        assert_ne!(original.report_input_sha256, altered.report_input_sha256);
        assert_eq!(original.artifact_sha256, altered.artifact_sha256);
    }
}
