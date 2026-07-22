//! BR-148 capability probe seam.
//!
//! This module deliberately does not read account/position state. The runtime
//! integration can record each provider attempt independently of governance
//! `DataMode`, so a stale position snapshot cannot poison Quote freshness.

use stock_analysis::monitor::data_mode::{Capability, CapabilityTracker};

#[allow(dead_code)]
pub fn register_provider_capabilities(tracker: &CapabilityTracker) -> Result<(), String> {
    for capability in Capability::ALL {
        if capability == Capability::OrderBook {
            tracker.register_unsupported(capability)?;
        } else {
            tracker.register_supported(capability)?;
        }
    }
    Ok(())
}
