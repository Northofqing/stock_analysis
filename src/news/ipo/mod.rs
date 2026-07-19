//! news::ipo — IPO 监测模块 (v15.1 Phase B)
//!
//! 包含:
//! - supply_chain: 静态 pre-IPO 公司 → A 股供应链标的映射
//! - monitor: PreIpoMonitor::tick 每 30min 扫一次 cninfo 待发行清单

pub mod supply_chain;
