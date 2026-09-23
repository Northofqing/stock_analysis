//! 复权服务层 — 统一复权逻辑入口
//!
//! 封装复权相关的业务逻辑，供各客户端 (TdxHqClient / TdxDirectClient / AsyncTdxHqClient) 调用。
//! 解耦客户端代码与复权算法细节。
//!
//! ## 功能
//!
//! - **自动档位检测** — 根据 XDXR 历史自动选择复权上下文档位
//! - **统一复权入口** — 供客户端调用的复权应用方法
//! - **因子计算** — 封装 adjuster 的因子计算接口
//!
//! ## 设计原则
//!
//! - 无网络依赖 — 所有方法都是纯函数，不涉及网络请求
//! - 向后兼容 — 复用 adjuster 模块的算法，不修改底层逻辑
//! - 单一职责 — 只负责业务编排，不负责具体算法

use crate::net::utils::FqContextTier;
use crate::protocol::adjuster::{adjust_security_bars, calc_fq_factors, FqFactorResult, FqType};
use crate::protocol::types::{SecurityBar, XdXrInfo};

/// 复权服务 — 统一复权逻辑入口
pub struct FqService;

impl FqService {
    /// 自动检测复权上下文档位 (O(1))
    ///
    /// 根据 XDXR 历史中最早的记录自动选择合适的档位：
    /// - Low (约 10 年): 股票上市不足 10 年
    /// - Mid (约 20 年): 股票上市 10-20 年 (默认)
    /// - High (约 30 年): 股票上市超过 20 年
    ///
    /// # 参数
    ///
    /// - `xdxr_list`: 该股票的除权除息历史 (由 get_xdxr_info 获取)
    /// - `current_year`: 当前年份 (通常取最新 K 线的年份)
    ///
    /// # 返回
    ///
    /// 推荐的复权上下文档位
    ///
    /// # 示例
    ///
    /// ```ignore
    /// let xdxr = client.get_xdxr_info(market, code)?;
    /// let current_year = bars.last().map(|b| b.year as u32).unwrap_or(2026);
    /// let tier = FqService::auto_detect_tier(&xdxr, current_year);
    /// ```
    pub fn auto_detect_tier(xdxr_list: &[XdXrInfo], current_year: u32) -> FqContextTier {
        if xdxr_list.is_empty() {
            return FqContextTier::Mid;
        }

        // XDXR 已按日期升序，第一个即最早记录 (O(1))
        let earliest_year = xdxr_list[0].year;
        let years_back = current_year.saturating_sub(earliest_year);

        match years_back {
            0..=10 => FqContextTier::Low,
            11..=20 => FqContextTier::Mid,
            _ => FqContextTier::High,
        }
    }

    /// 统一复权入口 — 对 K 线数据执行复权调整
    ///
    /// 供客户端在获取 K 线数据后调用，自动完成：
    /// 1. 调用 adjust_security_bars 进行价格调整
    ///
    /// # 参数
    ///
    /// - `bars`: 未复权 K 线数据 (按日期升序)，将被原地修改
    /// - `context_bars`: 额外的历史 K 线数据 (按日期升序)，仅用于因子计算
    /// - `xdxr_list`: 该股票的除权除息历史
    /// - `fq_type`: 复权类型 (None/Qfq/Hfq)
    ///
    /// # 注意
    ///
    /// 此方法是对 adjuster::adjust_security_bars 的直接封装，
    /// 保持接口一致性，便于后续扩展。
    pub fn apply_fq(
        bars: &mut [SecurityBar],
        context_bars: &[SecurityBar],
        xdxr_list: &[XdXrInfo],
        fq_type: FqType,
    ) {
        adjust_security_bars(bars, context_bars, xdxr_list, fq_type);
    }

    /// 计算复权因子 (不修改 K 线数据)
    ///
    /// 根据 XDXR 历史和上下文 K 线数据，计算并返回每个除权事件的复权因子。
    /// 可用于：
    /// - 验证复权精度
    /// - 手动应用复权调整
    /// - 导出因子表供外部使用
    ///
    /// # 参数
    ///
    /// - `xdxr_list`: 该股票的除权除息历史
    /// - `bars`: 请求的 K 线数据 (按日期升序)
    /// - `context_bars`: 额外的历史 K 线数据 (按日期升序)
    ///
    /// # 返回
    ///
    /// `FqFactorResult` 包含每个事件的因子和累计因子
    pub fn calc_factors(
        xdxr_list: &[XdXrInfo],
        bars: &[SecurityBar],
        context_bars: &[SecurityBar],
    ) -> FqFactorResult {
        calc_fq_factors(xdxr_list, bars, context_bars)
    }

    /// 将 fq 参数转换为 FqType 枚举
    ///
    /// - 0 → None (未复权)
    /// - 1 → Qfq (前复权)
    /// - 2 → Hfq (后复权)
    /// - _ → Qfq (默认前复权)
    pub fn fq_type_from_u8(fq: u8) -> FqType {
        match fq {
            0 => FqType::None,
            2 => FqType::Hfq,
            _ => FqType::Qfq,
        }
    }
}

#[cfg(test)]
#[path = "../../tests/internal/protocol_fq_service.rs"]
mod tests;
