//! RSI（相对强弱指数）策略集
//!
//! # 策略一：通用 RSI 超买超卖 v2（`RsiBacktest`）
//!
//! 1. 用 Wilder 指数平滑计算 N 日 RSI（比 SMA 更灵敏精准）
//! 2. RSI < 超卖阈值（默认 30）+ 跌势减缓过滤 + 冷却期 → **买入**
//! 3. RSI > 超买阈值（默认 70）时 → **卖出**（止盈）
//! 4. RSI 回升至均衡区（默认 65）且持仓盈利 → **平仓**（锁利）
//! 5. RSI 分档加仓：RSI 越低仓位越重（<20: 70%, <15: 100%）
//! 6. 可选 60 日均线趋势过滤，只在上升趋势中做多
//!
//! # 策略二：精准 RSI 深度超卖均值回归（`PrecisionRsiBacktest`）
//!
//! **买入条件（全部满足）**
//! 1. 5 日 RSI < 30 — 深度超卖
//! 2. 5 日 RSI 连续第三天走低（`rsi[i] < rsi[i-1] < rsi[i-2]`）— 超卖动能充分释放
//! 3. 三个交易日前 5 日 RSI < 60（`rsi[i-3] < 60`）— 前期压力已消化，排除假性超卖
//! 4. 收盘价 > 200 日均线 — 长期上升趋势，避免下跌通道诱多
//!
//! **卖出条件**
//! - 5 日 RSI 向上突破 50（前一日 < 50，当日 >= 50）— 超卖格局修复，均值回归完成

mod common;
pub mod precision;
pub mod standard;

pub use precision::{
    PrecisionRsiBacktest, PrecisionRsiConfig, PrecisionRsiResult, PrecisionRsiStrategy,
    SinglePrecisionRsiResult, compute_precision_indicators,
};
pub use standard::{
    RsiBacktest, RsiConfig, RsiResult, RsiStrategy, SingleRsiResult, compute_rsi_indicators,
};
