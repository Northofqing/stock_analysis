//! Locally owned security-bar wire type.
//! Its public fields and serde representation are stable transport contracts.

use serde::Serialize;

/// K-line bar used by the remote transport contract.
#[derive(Debug, Clone, Serialize)]
pub struct SecurityBar {
    pub open: f64,
    pub close: f64,
    pub high: f64,
    pub low: f64,
    pub vol: f64,
    pub amount: f64,
    pub year: u32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub datetime: String,
}
