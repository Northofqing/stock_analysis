//! BR-153 Gateway for the validated Magic TDX T0 evidence batch.
//!
//! This provider is deliberately strict: transport/protocol failures are
//! returned to the caller and no synthetic quote, bar, or order-book evidence
//! is made. General realtime quotes are owned by `MarketDataGateway`; this
//! module only keeps the T0-specific evidence contract.
//!
//! Business rules: BR-092 (strict K-line validation), BR-147 (settled close
//! evidence).

#[cfg(feature = "magic-gateway")]
use super::magic_tdx_t0::fetch_magic_tdx_t0_batch;
use super::magic_tdx_t0::MagicTdxT0Batch;
use anyhow::Result;
use chrono::{DateTime, Utc};

#[derive(Debug, Default, Clone, Copy)]
pub struct MagicTdxGateway;

impl MagicTdxGateway {
    pub fn new() -> Self {
        Self
    }

    pub fn get_t0_evidence_batch(
        &self,
        codes: &[String],
        observed_at: DateTime<Utc>,
    ) -> Result<MagicTdxT0Batch> {
        // P4 M3: gRPC 桥 (DATA_GATEWAY_GRPC=1 时替换 transport; 本地无 audit,
        // 桥路径亦不 audit — MagicTdxT0Batch 自带 requested_at/source_at/observed_at)。
        match super::grpc_source::bridge_for("T0Evidence") {
            Ok(Some(bridge)) => {
                let batch = bridge.t0_evidence_batch(codes).map_err(|error| {
                    anyhow::anyhow!("T0 证据批 gRPC 桥失败 ({} codes): {error}", codes.len())
                })?;
                return Ok(batch);
            }
            Ok(None) => {}
            Err(error) => return Err(anyhow::anyhow!("T0 证据批 gRPC 桥不可用: {error}")),
        }
        // no-feature (monitor 零 magic): library transport 不存在。
        // 无 bridge 时显式失败 (fail-closed), 绝不静默回退。
        #[cfg(not(feature = "magic-gateway"))]
        {
            return Err(anyhow::anyhow!(
                "library transport disabled: DATA_GATEWAY_GRPC=1 required"
            ));
        }
        #[cfg(feature = "magic-gateway")]
        {
            fetch_magic_tdx_t0_batch(codes, observed_at)
        }
    }
}
