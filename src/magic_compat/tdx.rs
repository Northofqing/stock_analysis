//! SecurityBar 本地镜像 (M5, Task #76, feature 关时使用)。
//! 与上游 magic-tdx-rs protocol/types.rs rev 75ee2a2 同构:
//! pub 字段集/serde 表示一致 (wire 是 JSON)。

#[cfg(not(feature = "magic-gateway"))]
use serde::Serialize;

/// K线 (K-line) bar. 上游仅 derive Debug/Clone/Serialize, 无 Deserialize。
#[cfg(not(feature = "magic-gateway"))]
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
