//! Same-invocation source observations for the legacy chain preparation.
//! These facts cover two acquisitions only; they do not grant Foundation admission.

use std::future::Future;

use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, NaiveDate};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use stock_analysis::data_gateway::{
    BatchEvidence, GatewayBatch, GatewayError, GlobalNewsProvider, GlobalNewsRecord,
};
use stock_analysis::database::data_acquisition_audit::DataAcquisitionAuditReceipt;
use stock_analysis::market_analyzer::{LimitUpObservation, LimitUpObservationStatus};
use stock_analysis::market_data::TopStock;
use stock_analysis::market_domain::{EvidenceTimestamp, ProviderId};
use stock_analysis::pipeline::chain_analysis::preparation::PreparedChainAnalysis;

#[derive(Clone)]
struct RetainedBatch {
    evidence: BatchEvidence,
    request_hash: String,
    receipt: DataAcquisitionAuditReceipt,
    record_count: usize,
}

#[derive(Clone)]
struct RetainedNameShard {
    requested_codes: Vec<String>,
    batch: RetainedBatch,
}

#[derive(Clone)]
pub(super) struct ChainLimitEvidence {
    status: LimitUpObservationStatus,
    pool: RetainedBatch,
    name_shards: Vec<RetainedNameShard>,
    composition_receipt: Option<DataAcquisitionAuditReceipt>,
}

pub(super) struct ChainLimitAcquisition {
    stocks: Vec<TopStock>,
    evidence: ChainLimitEvidence,
}

impl ChainLimitAcquisition {
    pub(super) fn from_observation(observation: LimitUpObservation) -> Self {
        let pool = RetainedBatch {
            evidence: observation.limit_pool_batch().evidence().clone(),
            request_hash: observation.limit_pool_request_hash().to_owned(),
            receipt: observation.limit_pool_receipt().clone(),
            record_count: observation.limit_pool_batch().records().len(),
        };
        let name_shards = observation
            .name_shards()
            .iter()
            .map(|shard| RetainedNameShard {
                requested_codes: shard.requested_codes().to_vec(),
                batch: RetainedBatch {
                    evidence: shard.batch().evidence().clone(),
                    request_hash: shard.request_hash().to_owned(),
                    receipt: shard.receipt().clone(),
                    record_count: shard.batch().records().len(),
                },
            })
            .collect();
        Self {
            stocks: observation.stocks().to_vec(),
            evidence: ChainLimitEvidence {
                status: observation.status(),
                pool,
                name_shards,
                composition_receipt: observation.composition_receipt().cloned(),
            },
        }
    }
}

#[derive(Clone)]
pub(super) enum ChainNewsEvidence {
    Available {
        evidence: BatchEvidence,
        record_count: usize,
        selected_count: usize,
        selected_input_sha256: String,
        selected_input_bytes: usize,
    },
    VerifiedEmpty(BatchEvidence),
    InvalidAvailableEmpty(BatchEvidence),
    Unavailable {
        provider: Option<stock_analysis::market_domain::ProviderId>,
        capability: &'static str,
        audit_outcome: &'static str,
        reason_code: &'static str,
        retryable: bool,
    },
}

pub(super) struct ChainAcquisitionEvidence {
    pub(super) observed_at: DateTime<FixedOffset>,
    pub(super) business_date: NaiveDate,
    pub(super) limit_up: ChainLimitEvidence,
    pub(super) news: ChainNewsEvidence,
}

/// Diagnostic reference to the selected UTF-8 titles saved as `macro_input`.
/// Its content digest is not a hash of the provider's raw batch and grants no
/// Foundation source admission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ChainSelectedNewsSourceRefV1 {
    pub(super) ref_sha256: String,
    pub(super) provider: ProviderId,
    pub(super) source: String,
    pub(super) batch_id_sha256: String,
    pub(super) provider_observed_at: String,
    pub(super) source_at: String,
    pub(super) content_sha256: String,
    pub(super) content_bytes: usize,
}

#[derive(Debug, thiserror::Error)]
#[error("产业链新闻来源时间缺失")]
pub(super) struct MissingSelectedNewsSourceTime;

impl ChainAcquisitionEvidence {
    pub(super) fn selected_news_source_ref(
        &self,
        prepared: &PreparedChainAnalysis,
    ) -> Result<Option<ChainSelectedNewsSourceRefV1>> {
        let ChainNewsEvidence::Available {
            evidence,
            record_count,
            selected_count,
            selected_input_sha256,
            selected_input_bytes,
        } = &self.news
        else {
            return Ok(None);
        };
        anyhow::ensure!(
            prepared.business_date() == self.business_date,
            "产业链新闻来源业务日期与准备对象不一致"
        );
        anyhow::ensure!(
            *selected_count > 0 && *selected_count <= 15 && *selected_count <= *record_count,
            "产业链新闻已选条数无效"
        );
        let saved = prepared
            .macro_input()
            .context("产业链新闻已选输入未保存在准备对象中")?;
        anyhow::ensure!(!saved.trim().is_empty(), "产业链新闻已选输入为空");
        anyhow::ensure!(
            saved.len() == *selected_input_bytes
                && format!("{:x}", Sha256::digest(saved.as_bytes())) == *selected_input_sha256,
            "产业链新闻已选输入与同次采集不一致"
        );
        anyhow::ensure!(
            evidence.provider == GlobalNewsProvider::Cailianpress.provider_id()
                && evidence.source == GlobalNewsProvider::Cailianpress.source()
                && !evidence.batch_id.trim().is_empty()
                && !evidence.observed_at.trim().is_empty(),
            "产业链新闻批次身份或时间不完整"
        );
        let source_at = evidence
            .source_at
            .as_deref()
            .filter(|at| !at.trim().is_empty())
            .ok_or(MissingSelectedNewsSourceTime)?;
        let source_time =
            EvidenceTimestamp::parse_instant(source_at).context("产业链新闻来源时间无效")?;
        let observed_time = EvidenceTimestamp::parse_instant(&evidence.observed_at)
            .context("产业链新闻观察时间无效")?;
        anyhow::ensure!(
            source_time <= observed_time,
            "产业链新闻来源时间晚于观察时间"
        );
        let ref_bytes = serde_json::to_vec(&json!({
            "schema": "chain-selected-news-source-ref-v1",
            "content_scope": "selected_titles_utf8",
            "provider": evidence.provider,
            "source": evidence.source,
            "batch_id": evidence.batch_id,
            "source_at": evidence.source_at,
            "provider_observed_at": evidence.observed_at,
            "content_sha256": selected_input_sha256,
            "content_bytes": selected_input_bytes,
        }))?;
        Ok(Some(ChainSelectedNewsSourceRefV1 {
            ref_sha256: format!("{:x}", Sha256::digest(ref_bytes)),
            provider: evidence.provider,
            source: evidence.source.clone(),
            batch_id_sha256: format!("{:x}", Sha256::digest(evidence.batch_id.as_bytes())),
            provider_observed_at: evidence.observed_at.clone(),
            source_at: source_at.to_owned(),
            content_sha256: selected_input_sha256.clone(),
            content_bytes: *selected_input_bytes,
        }))
    }

    /// Versioned diagnostic identity of the retained source observations.
    /// Report/artifact identity is bound separately by the post-send observer.
    pub(super) fn sha256(&self) -> Result<String> {
        let name_shards: Vec<_> = self
            .limit_up
            .name_shards
            .iter()
            .map(|shard| {
                json!({
                    "requested_codes": shard.requested_codes,
                    "batch": batch_identity(&shard.batch),
                })
            })
            .collect();
        let news = match &self.news {
            ChainNewsEvidence::Available {
                evidence,
                record_count,
                selected_count,
                selected_input_sha256,
                selected_input_bytes,
            } => json!({
                "status": "available",
                "evidence": evidence_identity(evidence),
                "record_count": record_count,
                "selected_count": selected_count,
                "selected_input_sha256": selected_input_sha256,
                "selected_input_bytes": selected_input_bytes,
            }),
            ChainNewsEvidence::VerifiedEmpty(evidence) => json!({
                "status": "verified_empty",
                "evidence": evidence_identity(evidence),
            }),
            ChainNewsEvidence::InvalidAvailableEmpty(evidence) => json!({
                "status": "invalid_available_empty",
                "evidence": evidence_identity(evidence),
            }),
            ChainNewsEvidence::Unavailable {
                provider,
                capability,
                audit_outcome,
                reason_code,
                retryable,
            } => json!({
                "status": "unavailable",
                "provider": provider,
                "capability": capability,
                "audit_outcome": audit_outcome,
                "reason_code": reason_code,
                "retryable": retryable,
            }),
        };
        let bytes = serde_json::to_vec(&json!({
            "schema": "chain-acquisition-evidence-v2",
            "observed_at": self.observed_at,
            "business_date": self.business_date,
            "limit_up": {
                "status": match self.limit_up.status {
                    LimitUpObservationStatus::Available => "available",
                    LimitUpObservationStatus::VerifiedEmpty => "verified_empty",
                },
                "pool": batch_identity(&self.limit_up.pool),
                "name_shards": name_shards,
                "composition_receipt": self.limit_up.composition_receipt.as_ref().map(receipt_identity),
            },
            "news": news,
            "preparation": "succeeded",
        }))?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
}

fn batch_identity(batch: &RetainedBatch) -> Value {
    json!({
        "evidence": evidence_identity(&batch.evidence),
        "request_hash": batch.request_hash,
        "receipt": receipt_identity(&batch.receipt),
        "record_count": batch.record_count,
    })
}

fn evidence_identity(evidence: &BatchEvidence) -> Value {
    json!({
        "provider": evidence.provider,
        "source": evidence.source,
        "source_at": evidence.source_at,
        "observed_at": evidence.observed_at,
        "batch_id": evidence.batch_id,
    })
}

fn receipt_identity(receipt: &DataAcquisitionAuditReceipt) -> Value {
    json!({
        "audit_id": receipt.audit_id,
        "record_hash": receipt.record_hash,
        "previous_outcome": receipt.previous_outcome,
        "current_outcome": receipt.current_outcome,
    })
}

/// Retain the original batch and its state while selecting the unchanged first 15 titles.
fn macro_news_input(
    result: std::result::Result<GatewayBatch<GlobalNewsRecord>, GatewayError>,
) -> (Option<String>, ChainNewsEvidence) {
    match result {
        Ok(GatewayBatch::Available { records, evidence }) if !records.is_empty() => {
            log::info!("[产业链] 已收集 {} 条快讯进 LLM 背景", records.len());
            let selected_count = records.len().min(15);
            let input = records
                .iter()
                .take(15)
                .map(|record| record.title.clone())
                .collect::<Vec<_>>()
                .join("; ");
            let selected_input_sha256 = format!("{:x}", Sha256::digest(input.as_bytes()));
            let selected_input_bytes = input.len();
            (
                Some(input),
                ChainNewsEvidence::Available {
                    evidence,
                    record_count: records.len(),
                    selected_count,
                    selected_input_sha256,
                    selected_input_bytes,
                },
            )
        }
        Ok(GatewayBatch::Available { evidence, .. }) => {
            log::warn!("[产业链] 快讯批次为空, LLM 无新闻背景");
            (None, ChainNewsEvidence::InvalidAvailableEmpty(evidence))
        }
        Ok(GatewayBatch::VerifiedEmpty(evidence)) => {
            log::warn!("[产业链] 快讯已验证为空: {:?}", evidence.batch_id);
            (None, ChainNewsEvidence::VerifiedEmpty(evidence))
        }
        Err(error) => {
            log::warn!("[产业链] 快讯收集失败, LLM 无新闻背景: {error}");
            (
                None,
                ChainNewsEvidence::Unavailable {
                    provider: error.provider(),
                    capability: error.capability(),
                    audit_outcome: error.audit_outcome(),
                    reason_code: error.reason_code(),
                    retryable: error.retryable(),
                },
            )
        }
    }
}

/// The callbacks are the existing one-time acquisition and preparation effects.
/// This wrapper makes their order/count testable without a second live fetch.
pub(super) async fn prepare_with_acquisition<L, LF, N, NF, P, PF>(
    observed_at: DateTime<FixedOffset>,
    limit_up: L,
    news: N,
    prepare: P,
) -> Result<(PreparedChainAnalysis, ChainAcquisitionEvidence)>
where
    L: FnOnce(NaiveDate) -> LF,
    LF: Future<Output = Result<ChainLimitAcquisition>>,
    N: FnOnce() -> NF,
    NF: Future<Output = std::result::Result<GatewayBatch<GlobalNewsRecord>, GatewayError>>,
    P: FnOnce(NaiveDate, Vec<TopStock>, Option<String>) -> PF,
    PF: Future<Output = Result<PreparedChainAnalysis>>,
{
    let business_date =
        stock_analysis::calendar::latest_completed_trading_day_at(observed_at.naive_local());
    let acquired = limit_up(business_date).await?;
    log::info!("今日涨停池共 {} 只", acquired.stocks.len());
    let (macro_news, news) = macro_news_input(news().await);
    let prepared = prepare(business_date, acquired.stocks, macro_news).await?;
    let evidence = ChainAcquisitionEvidence {
        observed_at,
        business_date,
        limit_up: acquired.evidence,
        news,
    };
    Ok((prepared, evidence))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::chain_schedule::{finish_scheduled_delivery, ChainPhase, ChainScheduleStatus};
    use crate::app::chain_shadow_input::{
        observe, test_prepared, test_prepared_with_macro, ChainGateCapture,
        SelectedNewsSourceRefStatus, ACQUISITION_INPUTS,
    };
    use crate::app::modes::{ChainDeliveryEnvelope, ChainSendSuppression};
    use chrono::Utc;
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    use stock_analysis::market_domain::{ProviderId, SourceEvidence};
    use stock_analysis::pipeline::chain_analysis::preparation::SourceStatus;

    fn batch(provider: ProviderId, source: &str, batch_id: &str, time: &str) -> BatchEvidence {
        BatchEvidence {
            provider,
            source: source.to_owned(),
            source_at: Some(time.to_owned()),
            observed_at: time.to_owned(),
            batch_id: batch_id.to_owned(),
        }
    }

    fn receipt(id: i64) -> DataAcquisitionAuditReceipt {
        DataAcquisitionAuditReceipt {
            audit_id: id,
            record_hash: format!("TEST_CODE_receipt_{id}"),
            previous_outcome: None,
            current_outcome: "accepted".to_owned(),
        }
    }

    fn limit(revision: &str) -> ChainLimitAcquisition {
        ChainLimitAcquisition {
            stocks: vec![TopStock {
                code: "600000".to_owned(),
                name: "TEST_CODE_stock".to_owned(),
                ..TopStock::default()
            }],
            evidence: ChainLimitEvidence {
                status: LimitUpObservationStatus::Available,
                pool: RetainedBatch {
                    evidence: batch(
                        ProviderId::Eastmoney,
                        "limit-pool",
                        revision,
                        "2026-09-28T07:00:00Z",
                    ),
                    request_hash: "TEST_CODE_pool_request".to_owned(),
                    receipt: receipt(101),
                    record_count: 1,
                },
                name_shards: vec![RetainedNameShard {
                    requested_codes: vec!["600000".to_owned()],
                    batch: RetainedBatch {
                        evidence: batch(
                            ProviderId::Tencent,
                            "security-identity",
                            "TEST_CODE_names_v7",
                            "2026-09-28T07:00:01Z",
                        ),
                        request_hash: "TEST_CODE_names_request".to_owned(),
                        receipt: receipt(102),
                        record_count: 1,
                    },
                }],
                composition_receipt: Some(receipt(103)),
            },
        }
    }

    fn news_record(index: usize) -> GlobalNewsRecord {
        let time = DateTime::parse_from_rfc3339("2026-09-28T07:00:02Z")
            .unwrap()
            .with_timezone(&Utc);
        GlobalNewsRecord {
            item_id: format!("TEST_CODE_news_{index}"),
            title: format!("TEST_CODE_title_{index}"),
            summary: None,
            content: None,
            publisher: "TEST_CODE_publisher".to_owned(),
            canonical_url: format!("https://example.invalid/{index}"),
            published_at: time,
            observed_at: time,
            instruments: Vec::new(),
            topics: Vec::new(),
            language: "zh-CN".to_owned(),
            evidence: SourceEvidence::new(
                ProviderId::Cailianpress,
                "2026-09-28T07:00:02Z",
                "TEST_CODE_news_source",
            )
            .unwrap(),
        }
    }

    async fn scripted(
        revision: &str,
        news_result: std::result::Result<GatewayBatch<GlobalNewsRecord>, GatewayError>,
    ) -> (
        PreparedChainAnalysis,
        ChainAcquisitionEvidence,
        Option<String>,
    ) {
        let observed_at = DateTime::parse_from_rfc3339("2026-09-29T09:05:00+08:00").unwrap();
        let limit_calls = Rc::new(Cell::new(0));
        let news_calls = Rc::new(Cell::new(0));
        let prepare_calls = Rc::new(Cell::new(0));
        let prepared_macro = Rc::new(RefCell::new(None));
        let revision = revision.to_owned();
        let (prepared, retained) = prepare_with_acquisition(
            observed_at,
            {
                let limit_calls = limit_calls.clone();
                move |business_date| async move {
                    limit_calls.set(limit_calls.get() + 1);
                    assert_eq!(business_date, NaiveDate::from_ymd_opt(2026, 9, 28).unwrap());
                    Ok(limit(&revision))
                }
            },
            {
                let news_calls = news_calls.clone();
                move || async move {
                    news_calls.set(news_calls.get() + 1);
                    news_result
                }
            },
            {
                let prepare_calls = prepare_calls.clone();
                let prepared_macro = prepared_macro.clone();
                move |business_date, stocks, macro_news| async move {
                    prepare_calls.set(prepare_calls.get() + 1);
                    assert_eq!(stocks.len(), 1);
                    assert_eq!(stocks[0].code, "600000");
                    *prepared_macro.borrow_mut() = macro_news.clone();
                    // The real preparation's effects are exercised elsewhere. This
                    // scripted callback proves the acquisition path invokes it once.
                    Ok(
                        test_prepared_with_macro(business_date, Rc::new(Cell::new(0)), macro_news)
                            .await,
                    )
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(limit_calls.get(), 1);
        assert_eq!(news_calls.get(), 1);
        assert_eq!(prepare_calls.get(), 1);
        let selected = prepared_macro.borrow().clone();
        (prepared, retained, selected)
    }

    #[tokio::test]
    async fn retains_original_pool_name_and_news_revisions_after_title_selection() {
        let news_batch = batch(
            ProviderId::Cailianpress,
            "cls-v1",
            "TEST_CODE_news_v9",
            "2026-09-28T07:00:02Z",
        );
        let (prepared, retained, selected) = scripted(
            "TEST_CODE_pool_v3",
            Ok(GatewayBatch::Available {
                records: (0..17).map(news_record).collect(),
                evidence: news_batch.clone(),
            }),
        )
        .await;
        assert_eq!(
            retained.business_date,
            NaiveDate::from_ymd_opt(2026, 9, 28).unwrap()
        );
        assert_eq!(
            retained.observed_at.date_naive(),
            NaiveDate::from_ymd_opt(2026, 9, 29).unwrap()
        );
        assert_eq!(
            retained.limit_up.pool.evidence.batch_id,
            "TEST_CODE_pool_v3"
        );
        assert_eq!(
            retained.limit_up.pool.request_hash,
            "TEST_CODE_pool_request"
        );
        assert_eq!(retained.limit_up.pool.receipt.audit_id, 101);
        assert_eq!(retained.limit_up.name_shards.len(), 1);
        assert_eq!(
            retained.limit_up.name_shards[0].batch.evidence.batch_id,
            "TEST_CODE_names_v7"
        );
        assert_eq!(retained.limit_up.name_shards[0].batch.receipt.audit_id, 102);
        assert_eq!(
            retained
                .limit_up
                .composition_receipt
                .as_ref()
                .unwrap()
                .audit_id,
            103
        );
        match &retained.news {
            ChainNewsEvidence::Available {
                evidence,
                record_count,
                selected_count,
                ..
            } => {
                assert_eq!(evidence, &news_batch);
                assert_eq!((*record_count, *selected_count), (17, 15));
            }
            _ => panic!("expected available news"),
        }
        let selected = selected.unwrap();
        assert!(selected.contains("TEST_CODE_title_14"));
        assert!(!selected.contains("TEST_CODE_title_15"));
        let observed = observe(
            ChainPhase::Preopen,
            retained.observed_at.date_naive(),
            &prepared,
            prepared.report().as_bytes(),
            Some(&retained),
        )
        .unwrap();
        assert_eq!(observed.prepared_business_date, retained.business_date);
        assert_eq!(observed.schedule_date, retained.observed_at.date_naive());
        assert_eq!(observed.coverage, "incomplete");
        assert_eq!(observed.covered_inputs, ACQUISITION_INPUTS);
        assert_eq!(observed.acquisition_sha256.as_deref().unwrap().len(), 64);
        assert_eq!(
            observed
                .acquisition_report_binding_sha256
                .as_deref()
                .unwrap()
                .len(),
            64
        );
        assert_eq!(prepared.limit_up_source().status(), &SourceStatus::Unknown);
        assert_eq!(prepared.macro_input(), Some(selected.as_str()));
        let source_ref = observed.selected_news_source_ref.unwrap();
        assert_eq!(source_ref.provider_observed_at, news_batch.observed_at);
        assert_eq!(
            Some(source_ref.source_at.as_str()),
            news_batch.source_at.as_deref()
        );
        assert_eq!(source_ref.provider, ProviderId::Cailianpress);
        assert_eq!(source_ref.source, "cls-v1");
        assert_eq!(source_ref.batch_id_sha256.len(), 64);
        assert_eq!(source_ref.content_bytes, selected.len());
        assert_eq!(
            source_ref.content_sha256,
            format!("{:x}", Sha256::digest(selected.as_bytes()))
        );
        assert_eq!(source_ref.ref_sha256.len(), 64);
    }

    #[tokio::test]
    async fn suppressed_prepared_report_retains_source_and_exact_input_binding() {
        let (prepared, acquisition, _) = scripted(
            "TEST_CODE_pool_v3",
            Ok(GatewayBatch::Available {
                records: vec![news_record(0)],
                evidence: batch(
                    ProviderId::Cailianpress,
                    "cls-v1",
                    "TEST_CODE_news_v9",
                    "2026-09-28T07:00:02Z",
                ),
            }),
        )
        .await;
        let input = prepared.report().as_bytes().to_vec();
        let schedule_date = acquisition.observed_at.date_naive();
        let envelope = ChainDeliveryEnvelope {
            prepared,
            acquisition: Some(acquisition),
            send_attempted: false,
            report_input: input.clone(),
            notification_report: None,
            wechat_http_body: None,
            feishu_http_body: None,
            suppression: Some(ChainSendSuppression::NoConfiguredChannel),
            legacy_result: Err(anyhow::anyhow!("TEST_CODE_no_channel")),
        };
        let error = finish_scheduled_delivery(
            envelope,
            ChainPhase::Preopen,
            schedule_date,
            &ChainGateCapture::new(
                ChainPhase::Preopen,
                schedule_date,
                DateTime::parse_from_rfc3339("2026-09-29T09:05:00+08:00").unwrap(),
                true,
                ChainScheduleStatus::Ready,
            ),
            || panic!("suppressed report must not mark weak acceptance"),
            |phase, date, prepared, report_input, acquisition| {
                assert_eq!(phase, ChainPhase::Preopen);
                assert_eq!(date, schedule_date);
                assert_eq!(report_input, input);
                let observed = observe(phase, date, prepared, report_input, acquisition)?;
                assert!(observed.prepared_report_equals_input);
                assert_eq!(
                    observed.selected_news_source_ref_status,
                    SelectedNewsSourceRefStatus::Observed
                );
                assert!(observed.acquisition_sha256.is_some());
                assert!(observed.acquisition_report_binding_sha256.is_some());
                Ok(observed)
            },
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "TEST_CODE_no_channel");
    }

    #[tokio::test]
    async fn source_revision_changes_digest_without_changing_prepared_report() {
        let news_batch = || {
            GatewayBatch::VerifiedEmpty(batch(
                ProviderId::Cailianpress,
                "cls-v1",
                "TEST_CODE_news_empty",
                "2026-09-28T07:00:02Z",
            ))
        };
        let (first, first_retained, _) = scripted("TEST_CODE_pool_v3", Ok(news_batch())).await;
        let (second, second_retained, _) = scripted("TEST_CODE_pool_v4", Ok(news_batch())).await;
        assert_eq!(first.report(), second.report());
        assert_eq!(
            first.to_artifact_bytes().unwrap(),
            second.to_artifact_bytes().unwrap()
        );
        assert_ne!(
            first_retained.sha256().unwrap(),
            second_retained.sha256().unwrap()
        );
    }

    #[tokio::test]
    async fn selected_news_source_ref_replays_and_refuses_changed_or_missing_saved_input() {
        let evidence = batch(
            ProviderId::Cailianpress,
            "cls-v1",
            "TEST_CODE_news_v9",
            "2026-09-28T07:00:02Z",
        );
        let (prepared, mut retained, selected) = scripted(
            "TEST_CODE_pool_v3",
            Ok(GatewayBatch::Available {
                records: (0..2).map(news_record).collect(),
                evidence,
            }),
        )
        .await;
        let selected = selected.unwrap();
        let artifact = prepared.to_artifact_bytes().unwrap();
        let replayed = PreparedChainAnalysis::from_artifact_bytes(&artifact).unwrap();
        let original_ref = retained
            .selected_news_source_ref(&prepared)
            .unwrap()
            .unwrap();
        let replayed_ref = retained
            .selected_news_source_ref(&replayed)
            .unwrap()
            .unwrap();
        assert_eq!(original_ref, replayed_ref);
        assert_eq!(replayed.macro_input(), Some(selected.as_str()));

        let changed = test_prepared_with_macro(
            retained.business_date,
            Rc::new(Cell::new(0)),
            Some("TEST_CODE_changed_title".to_owned()),
        )
        .await;
        let changed =
            PreparedChainAnalysis::from_artifact_bytes(&changed.to_artifact_bytes().unwrap())
                .unwrap();
        assert!(retained.selected_news_source_ref(&changed).is_err());
        let changed_observation = observe(
            ChainPhase::Preopen,
            retained.business_date,
            &changed,
            changed.report().as_bytes(),
            Some(&retained),
        )
        .unwrap();
        assert_eq!(
            changed_observation.selected_news_source_ref_status,
            SelectedNewsSourceRefStatus::Rejected
        );
        assert!(changed_observation.selected_news_source_ref.is_none());
        assert!(changed_observation
            .acquisition_report_binding_sha256
            .is_some());

        let missing = test_prepared(retained.business_date, Rc::new(Cell::new(0))).await;
        let missing =
            PreparedChainAnalysis::from_artifact_bytes(&missing.to_artifact_bytes().unwrap())
                .unwrap();
        assert!(retained.selected_news_source_ref(&missing).is_err());

        if let ChainNewsEvidence::Available { evidence, .. } = &mut retained.news {
            evidence.source_at = None;
        }
        assert!(retained.selected_news_source_ref(&prepared).is_err());
    }

    #[tokio::test]
    async fn missing_news_source_time_keeps_existing_input_digests() {
        let mut evidence = batch(
            ProviderId::Cailianpress,
            "cls-v1",
            "TEST_CODE_news_without_source_time",
            "2026-09-28T07:00:02Z",
        );
        evidence.source_at = None;
        let (prepared, retained, _) = scripted(
            "TEST_CODE_pool_v3",
            Ok(GatewayBatch::Available {
                records: vec![news_record(0)],
                evidence,
            }),
        )
        .await;
        let observation = observe(
            ChainPhase::Preopen,
            retained.business_date,
            &prepared,
            prepared.report().as_bytes(),
            Some(&retained),
        )
        .unwrap();
        assert_eq!(
            observation.selected_news_source_ref_status,
            SelectedNewsSourceRefStatus::MissingSourceTime
        );
        assert_eq!(
            observation.selected_news_source_ref_status.status(),
            "unobserved"
        );
        assert_eq!(
            observation.selected_news_source_ref_status.reason(),
            "missing_source_at"
        );
        assert!(observation.selected_news_source_ref.is_none());
        assert_eq!(
            observation.acquisition_sha256,
            Some(retained.sha256().unwrap())
        );
        assert_eq!(observation.artifact_sha256.len(), 64);
        assert_eq!(observation.report_input_sha256.len(), 64);
        assert!(observation.acquisition_report_binding_sha256.is_some());
        assert!(retained.selected_news_source_ref(&prepared).is_err());
    }

    #[tokio::test]
    async fn selected_news_source_ref_accepts_gateway_instant_time_format() {
        let evidence = batch(
            ProviderId::Cailianpress,
            "cls-v1",
            "TEST_CODE_news_epoch_batch",
            "1785799979",
        );
        let (prepared, retained, _) = scripted(
            "TEST_CODE_pool_v3",
            Ok(GatewayBatch::Available {
                records: vec![news_record(0)],
                evidence,
            }),
        )
        .await;
        let source_ref = retained
            .selected_news_source_ref(&prepared)
            .unwrap()
            .unwrap();
        assert_eq!(source_ref.source_at, "1785799979");
        assert_eq!(source_ref.provider_observed_at, "1785799979");
    }

    #[tokio::test]
    async fn empty_invalid_and_failed_news_keep_distinct_status_without_macro_text() {
        let evidence = batch(
            ProviderId::Cailianpress,
            "cls-v1",
            "TEST_CODE_news_empty",
            "2026-09-28T07:00:02Z",
        );
        let observed_status = |prepared: &PreparedChainAnalysis,
                               retained: &ChainAcquisitionEvidence| {
            let observation = observe(
                ChainPhase::Preopen,
                retained.business_date,
                prepared,
                prepared.report().as_bytes(),
                Some(retained),
            )
            .unwrap();
            assert!(observation.selected_news_source_ref.is_none());
            assert!(observation.acquisition_sha256.is_some());
            observation.selected_news_source_ref_status
        };
        let (verified_prepared, verified, macro_news) = scripted(
            "TEST_CODE_pool_v3",
            Ok(GatewayBatch::VerifiedEmpty(evidence.clone())),
        )
        .await;
        assert!(macro_news.is_none());
        assert!(matches!(verified.news, ChainNewsEvidence::VerifiedEmpty(_)));
        assert!(verified
            .selected_news_source_ref(&verified_prepared)
            .unwrap()
            .is_none());
        assert_eq!(
            observed_status(&verified_prepared, &verified).reason(),
            "verified_empty"
        );

        let (invalid_prepared, invalid, macro_news) = scripted(
            "TEST_CODE_pool_v3",
            Ok(GatewayBatch::Available {
                records: Vec::new(),
                evidence,
            }),
        )
        .await;
        assert!(macro_news.is_none());
        assert!(matches!(
            invalid.news,
            ChainNewsEvidence::InvalidAvailableEmpty(_)
        ));
        assert!(invalid
            .selected_news_source_ref(&invalid_prepared)
            .unwrap()
            .is_none());
        assert_eq!(
            observed_status(&invalid_prepared, &invalid).reason(),
            "invalid_available_empty"
        );

        let (failed_prepared, failed, macro_news) = scripted(
            "TEST_CODE_pool_v3",
            Err(GatewayError::unavailable(
                "GlobalNews-CLS",
                Some(ProviderId::Cailianpress),
                true,
                "TEST_CODE_no_batch",
            )),
        )
        .await;
        assert!(macro_news.is_none());
        assert!(matches!(
            failed.news,
            ChainNewsEvidence::Unavailable {
                reason_code: "no_verified_batch",
                ..
            }
        ));
        assert!(failed
            .selected_news_source_ref(&failed_prepared)
            .unwrap()
            .is_none());
        assert_eq!(
            observed_status(&failed_prepared, &failed).reason(),
            "news_unavailable"
        );
        assert_ne!(verified.sha256().unwrap(), invalid.sha256().unwrap());
        assert_ne!(verified.sha256().unwrap(), failed.sha256().unwrap());
    }
}
