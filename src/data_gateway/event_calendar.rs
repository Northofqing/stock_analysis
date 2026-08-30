//! BR-161 evidence-preserving R-08 event-calendar acquisition.

use super::review::{acquisition_request_hash, audit_gateway_result};

use super::GatewayBatch;
use super::GatewayError;
use crate::market_domain::ProviderId;

use chrono::{DateTime, NaiveDate, Utc};

const CAPABILITY: &str = "R-08-announcements";
const SOURCE: &str = "cninfo-market";

/// One validated whole-market announcement fact for R-08 rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventAnnouncement {
    pub announcement_id: String,
    pub code: String,
    pub category: Option<String>,
    pub title: String,
    pub published_at: String,
    pub canonical_url: String,
}

/// BR-161 production seam for bounded, whole-market CNInfo announcements.
#[derive(Debug, Clone, Copy, Default)]
pub struct EventCalendarGateway;

impl EventCalendarGateway {
    pub const fn new() -> Self {
        Self
    }

    pub async fn market_announcements(
        &self,
        trading_date: NaiveDate,
        limit: u32,
    ) -> Result<GatewayBatch<EventAnnouncement>, GatewayError> {
        let request_hash = acquisition_request_hash(CAPABILITY, format!("{trading_date}:{limit}"));
        // P4 M3 钩子: remote gRPC → gRPC 通道 (fail-closed, audit 对等)。
        match super::grpc_source::bridge_for("Announcements") {
            Ok(bridge) => {
                let result = bridge.announcements_async().await;
                let audit_provider = result
                    .as_ref()
                    .map(|b| b.evidence().provider)
                    .unwrap_or(ProviderId::Cninfo);
                return audit_gateway_result(CAPABILITY, audit_provider, &request_hash, result);
            }
            Err(error) => {
                return audit_gateway_result(
                    CAPABILITY,
                    ProviderId::Cninfo,
                    &request_hash,
                    Err(error),
                );
            }
        }
        // no-feature (monitor 零 magic): library transport 不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
    }
}

fn parse_observed_at(value: &str) -> Result<DateTime<Utc>, GatewayError> {
    let (seconds, nanos) = value.split_once('.').ok_or_else(|| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Cninfo),
            format!("invalid CNInfo observation time {value:?}"),
        )
    })?;
    if nanos.len() != 9 || !nanos.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Cninfo),
            format!("invalid CNInfo observation nanos {value:?}"),
        ));
    }
    let seconds = seconds.parse::<i64>().map_err(|error| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Cninfo),
            format!("invalid CNInfo observation seconds {value:?}: {error}"),
        )
    })?;
    let nanos = nanos.parse::<u32>().map_err(|error| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Cninfo),
            format!("invalid CNInfo observation nanos {value:?}: {error}"),
        )
    })?;
    DateTime::from_timestamp(seconds, nanos).ok_or_else(|| {
        GatewayError::invalid_evidence(
            CAPABILITY,
            Some(ProviderId::Cninfo),
            format!("CNInfo observation time is out of range {value:?}"),
        )
    })
}
