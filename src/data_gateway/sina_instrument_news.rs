//! BR-066/BR-164/BR-172 evidence-preserving Sina instrument-news gateway.

use crate::market_domain::{ProviderId, SourceEvidence};
use chrono::{DateTime, Utc};

use super::review::{acquisition_request_hash, audit_gateway_result, GatewayBatch, GatewayError};

use crate::data_provider::news_item::NewsItem;

const CAPABILITY: &str = "SinaInstrumentNews";
const SOURCE: &str = "sina-company-news";
const REQUEST_LIMIT: u32 = 100;

/// One admitted Sina company-news row with the legacy persistence projection
/// and its immutable upstream evidence.
#[derive(Debug, Clone)]
pub struct SinaInstrumentNewsRecord {
    persistence_item: NewsItem,
    evidence: SourceEvidence,
}

impl SinaInstrumentNewsRecord {
    /// P4 M3: gRPC convert 构造用 (视图字段已全, 无静默填充)。
    pub fn new(persistence_item: NewsItem, evidence: SourceEvidence) -> Self {
        Self {
            persistence_item,
            evidence,
        }
    }

    pub fn persistence_item(&self) -> &NewsItem {
        &self.persistence_item
    }

    pub fn evidence(&self) -> &SourceEvidence {
        &self.evidence
    }
}

/// BR-163 production seam for bounded Sina company-news history.
#[derive(Debug, Clone, Copy, Default)]
pub struct SinaInstrumentNewsGateway;

impl SinaInstrumentNewsGateway {
    pub const fn new() -> Self {
        Self
    }

    pub async fn instrument_news_in_range(
        &self,
        code: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<GatewayBatch<SinaInstrumentNewsRecord>, GatewayError> {
        let code = code.to_owned();
        let request_hash = acquisition_request_hash(
            CAPABILITY,
            format!(
                "{code}:{}:{}:{REQUEST_LIMIT}",
                from.to_rfc3339(),
                to.to_rfc3339()
            ),
        );
        // P4 M3: gRPC 桥 (remote gRPC 时替换 transport; audit 留客户端)。
        // from_days 契约: 生产调用方传 now-30d..now, 等价服务端"近 N 日"语义。
        match super::grpc_source::bridge_for("InstrumentNews") {
            Ok(bridge) => {
                let from_days = (to - from).num_days().clamp(1, 30) as u32;
                let result = bridge
                    .instrument_news_async(std::slice::from_ref(&code), from_days)
                    .await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Sina);
                return audit_gateway_result(CAPABILITY, audit_provider, &request_hash, result);
            }
            Err(error) => {
                return audit_gateway_result(
                    CAPABILITY,
                    ProviderId::Sina,
                    &request_hash,
                    Err(error),
                );
            }
        }
        // no-feature (monitor 零 magic): library transport 不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
    }
}
