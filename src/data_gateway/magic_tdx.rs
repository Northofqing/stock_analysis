//! BR-153 Gateway for the validated Magic TDX T0 evidence batch.
//!
//! This provider is deliberately strict: transport/protocol failures are
//! returned to the caller and no synthetic quote, bar, or order-book evidence
//! is made. General realtime quotes are owned by `MarketDataGateway`; this
//! module only keeps the T0-specific evidence contract.
//!
//! Business rules: BR-092 (strict K-line validation), BR-147 (settled close
//! evidence).

use super::magic_tdx_t0::{fetch_magic_tdx_t0_batch, MagicTdxT0Batch};
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
        fetch_magic_tdx_t0_batch(codes, observed_at)
    }
}
